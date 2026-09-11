// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

//! Bridges the Apple T2 Touch ID sensor to fprintd.
//!
//! libfprint's virtual storage device listens on a Unix socket while fprintd
//! has the device open, which is exactly the duration of one authentication
//! attempt. That connection is therefore the arming signal: the sensor is put
//! into match mode only while someone is actually standing in front of a
//! password prompt. Anything else would let a finger touched at an idle moment
//! satisfy a later sudo, because libfprint queues a scan that arrives while no
//! operation is pending.

mod fprint;
mod resume;
mod signal;

use std::sync::atomic::Ordering;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use signal::{Signal, State};
use t2_biometrickit::{Event, Identity, Session, proto};

const IDLE_POLL: Duration = Duration::from_millis(250);
const MATCH_WINDOW: Duration = Duration::from_secs(10);
const EVENT_WAIT: Duration = Duration::from_millis(500);
/// libfprint handles one connection at a time and drops the previous one, so a
/// probe fired straight after a scan would discard the scan itself.
const SETTLE: Duration = Duration::from_millis(750);
/// Between two commands; long enough that libfprint has read the first.
const BETWEEN: Duration = Duration::from_millis(150);
/// After resume the link needs a moment to carry traffic. While fprintd is
/// asking, keep trying to reach the sensor for this long before giving up to
/// the password, so the prompt waits the link out.
const REACH_GRACE: Duration = Duration::from_secs(15);

struct Config {
    socket: String,
    /// The macOS user id whose fingers to verify against; 0 means find it.
    user_id: u32,
    flags: u32,
    /// Linux account that gets the enrolled fingers bound to it automatically.
    bind_user: Option<String>,
}

/// Labels for automatically bound fingers. The SEP does not say which finger a
/// template is, so these are positional and only what fprintd-list shows.
const FINGER_LABELS: [&str; 10] = [
    "right-index-finger",
    "right-middle-finger",
    "right-thumb",
    "right-ring-finger",
    "right-little-finger",
    "left-index-finger",
    "left-middle-finger",
    "left-thumb",
    "left-ring-finger",
    "left-little-finger",
];

/// macOS hands out user ids from 501 upwards; the SEP answers with an empty
/// inventory for an id that has no finger, so the right one can be found.
const FIRST_MACOS_UID: u32 = 501;
const LAST_MACOS_UID: u32 = 520;

fn config() -> Result<Config> {
    let mut socket = String::from("/run/t2-touchid/fprint.sock");
    let mut user_id = 501;
    let mut flags = 0;
    let mut bind_user = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        let mut value = || args.next().ok_or_else(|| anyhow::anyhow!("{argument} needs a value"));
        match argument.as_str() {
            "--socket" => socket = value()?,
            "--uid" => {
                let raw = value()?;
                user_id = if raw == "auto" { 0 } else { raw.parse()? };
            }
            "--flags" => {
                let raw = value()?;
                flags = raw
                    .strip_prefix("0x")
                    .map_or_else(|| raw.parse(), |hex| u32::from_str_radix(hex, 16))?;
            }
            "--bind-user" => {
                let name = value()?;
                bind_user = (!name.is_empty() && name != "none").then_some(name);
            }
            other => bail!("unknown argument {other}"),
        }
    }
    Ok(Config { socket, user_id, flags, bind_user })
}

fn main() -> Result<()> {
    let config = config()?;
    let mut session: Option<Session> = None;
    let mut identities: Vec<Identity> = Vec::new();
    let mut stages_set = false;
    let mut user_id = config.user_id;
    let mut bind_pending = config.bind_user.is_some();
    let mut prompt = Signal::new();
    let resumed = resume::watch();
    let mut unreachable_since: Option<Instant> = None;
    log(&format!(
        "watching {} for uid {} with flags {:#x}",
        config.socket,
        if user_id == 0 { "auto".to_string() } else { user_id.to_string() },
        config.flags
    ));

    loop {
        // The suspend kills the held session silently; drop it on resume so the
        // next prompt opens a fresh one instead of blocking on a dead socket.
        if resumed.swap(false, Ordering::Relaxed) && session.take().is_some() {
            log("dropping the sensor session after resume");
        }

        // A session is needed for both binding and matching, so keep one open
        // and reopen it if it drops. Holding it does not arm the sensor; only
        // start_match does that.
        if session.is_none() {
            match Session::open(None, None) {
                Ok(mut opened) => {
                    log("BiometricKit session open");
                    (user_id, identities) = find_fingers(&mut opened, user_id);
                    session = Some(opened);
                }
                Err(error) => {
                    log(&format!("cannot reach the sensor: {error:#}"));
                    // Only fall the prompt back to the password once the sensor
                    // has stayed unreachable past the grace window; a link
                    // returning after resume settles well within it.
                    if fprint::device_is_open(&config.socket) {
                        let since = *unreachable_since.get_or_insert_with(Instant::now);
                        if since.elapsed() >= REACH_GRACE {
                            fprint::report_failure(&config.socket);
                            unreachable_since = None;
                        }
                    } else {
                        unreachable_since = None;
                    }
                    sleep(SETTLE);
                    continue;
                }
            }
            unreachable_since = None;
        }

        // Bind the enrolled fingers to the installing account. This drives
        // fprintd-enroll, which opens the device itself, so it must run
        // independently of an incoming authentication rather than behind it.
        if bind_pending {
            if let Some(user) = &config.bind_user {
                bind_pending = !bind_missing(&config, user, &identities);
            } else {
                bind_pending = false;
            }
            if bind_pending {
                sleep(SETTLE);
            }
            continue;
        }

        // libfprint only listens while fprintd holds the device open, which is
        // the duration of one authentication attempt.
        if !fprint::device_is_open(&config.socket) {
            stages_set = false;
            prompt.set(State::Idle);
            sleep(IDLE_POLL);
            continue;
        }
        log("fprintd is asking for a finger");
        prompt.set(State::Waiting);

        // One touch is enough: the finger is already enrolled in the SEP.
        // Sent once per opening; repeating it mid-enrolment resets its progress.
        if !stages_set {
            stages_set = fprint::send(&config.socket, "SET_ENROLL_STAGES 1").is_ok();
        }

        if let Err(error) = serve(session.as_mut().unwrap(), &config, &identities, &mut prompt) {
            log(&format!("attempt failed: {error:#}"));
            prompt.set(State::Failed);
            fprint::report_failure(&config.socket);
            session = None;
            sleep(SETTLE);
        }
    }
}

/// Read the enrolled fingers for the configured uid, or search for the uid
/// that has some when none was configured or the configured one has none.
fn find_fingers(session: &mut Session, configured: u32) -> (u32, Vec<Identity>) {
    let read = |session: &mut Session, uid: u32| -> Vec<Identity> {
        session.read_identities(uid).map(<[Identity]>::to_vec).unwrap_or_default()
    };
    if configured != 0 {
        let found = read(session, configured);
        if !found.is_empty() {
            log(&format!("macOS uid {configured} has {} enrolled finger(s)", found.len()));
            return (configured, found);
        }
        log(&format!("macOS uid {configured} has no enrolled finger, looking for one that has"));
    }
    for uid in FIRST_MACOS_UID..=LAST_MACOS_UID {
        let found = read(session, uid);
        if !found.is_empty() {
            log(&format!("macOS uid {uid} has {} enrolled finger(s), using it", found.len()));
            return (uid, found);
        }
    }
    log("no macOS user on this Mac has a finger enrolled");
    (configured, Vec::new())
}

/// Bind every enrolled finger fprintd does not know yet to the configured
/// Linux account. The binding is only the statement that this SEP identity
/// belongs to that account; the finger itself still has to be presented at
/// every login. fprintd-enroll is driven by answering its scan with the
/// identity straight away, so nobody has to sit at the sensor for it, which is
/// what makes unattended installations and updates work.
fn bind_missing(config: &Config, user: &str, identities: &[Identity]) -> bool {
    let store = std::path::Path::new(fprint::STORE);
    let mut all_bound = true;
    for (slot, identity) in identities.iter().enumerate() {
        let id = fprint::print_id(identity);
        if fprint::is_bound(store, &id) {
            continue;
        }
        let Some(label) = FINGER_LABELS.get(slot) else {
            break;
        };
        log(&format!("binding finger {slot} to {user} as {label}"));
        let mut child = match std::process::Command::new("fprintd-enroll")
            .arg("-f")
            .arg(label)
            .arg(user)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                log(&format!("cannot run fprintd-enroll: {error}"));
                return false;
            }
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut stages_set = false;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                if status.success() {
                    log(&format!("bound finger {slot} to {user}"));
                } else {
                    log(&format!("fprintd-enroll failed for {user}; bind it by hand"));
                    all_bound = false;
                }
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                log("fprintd did not finish the binding; bind it by hand with fprintd-enroll");
                all_bound = false;
                break;
            }
            if !fprint::device_is_open(&config.socket) {
                sleep(IDLE_POLL);
                continue;
            }
            if !stages_set {
                stages_set = fprint::send(&config.socket, "SET_ENROLL_STAGES 1").is_ok();
                sleep(BETWEEN);
            }
            if fprint::send(&config.socket, &format!("SCAN {id}")).is_ok() {
                sleep(SETTLE);
            }
        }
    }
    all_bound
}

fn log(message: &str) {
    eprintln!("t2-touchid: {message}");
}

/// One authentication attempt: arm the sensor and report the identity that
/// matched. The socket is deliberately not probed while the match runs,
/// because every connection is a command channel and probing would disturb the
/// operation libfprint has in flight.
fn serve(session: &mut Session, config: &Config, identities: &[Identity], prompt: &mut Signal) -> Result<()> {
    // First of all, before anything can take time: fprintd drops a stored
    // finger the moment it finds the device does not have it, so the bound
    // ones have to be back in place before it looks.
    for identity in identities {
        let id = fprint::print_id(identity);
        if fprint::is_bound(std::path::Path::new(fprint::STORE), &id) {
            fprint::send(&config.socket, &format!("INSERT {id}"))?;
            sleep(BETWEEN);
        }
    }

    if identities.is_empty() {
        log("nothing to match against; enroll the finger under macOS first");
        prompt.set(State::Failed);
        fprint::report_failure(&config.socket);
        sleep(SETTLE);
        return Ok(());
    }

    // Nothing else is sent before the match: every command reaches libfprint's
    // state machine, and changing the enrolment stage count part way through
    // an enrolment throws its progress away.
    session.start_match(config.flags, proto::NO_CREDENTIAL_SET)?;
    log("put your finger on the sensor");

    let deadline = Instant::now() + MATCH_WINDOW;
    let mut outcome = None;
    while Instant::now() < deadline {
        match session.next_event(EVENT_WAIT)? {
            Some(Event::MatchResult { slot, bytes }) => {
                log(&format!("match result: slot {slot:?}, {bytes} bytes"));
                outcome = Some(slot);
                break;
            }
            Some(Event::FingerDown) => {
                log("finger down");
                prompt.set(State::Scanning);
            }
            Some(Event::FingerUp) => log("finger up"),
            Some(Event::Status(code)) => log(&format!("status {code}")),
            Some(Event::Other { kind, bytes }) => {
                log(&format!("event {kind:#x}, {bytes} bytes"))
            }
            Some(Event::Statistics(_)) | None => {}
        }
    }
    session.cancel()?;

    match outcome {
        Some(Some(slot)) => {
            let id = fprint::print_id(&identities[slot]);
            fprint::send(&config.socket, &format!("SCAN {id}"))?;
            log(&format!("recognised finger {slot}, told fprintd"));
            prompt.set(State::Matched);
            sleep(SETTLE);
        }
        Some(None) => {
            // A retry keeps fprintd's prompt standing so the finger can simply
            // be placed again. Reporting a no-match instead would spend one of
            // pam_fprintd's few attempts on what is usually a bad read.
            log("not recognised, asking for the finger again");
            prompt.set(State::Retry);
            fprint::send(&config.socket, "RETRY 0")?;
            sleep(SETTLE);
        }
        None => log("nothing recognised, arming again"),
    }
    Ok(())
}
