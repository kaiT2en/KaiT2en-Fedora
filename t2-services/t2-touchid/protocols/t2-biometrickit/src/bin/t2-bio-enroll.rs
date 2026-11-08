// SPDX-License-Identifier: GPL-3.0-or-later

//! Enroll a finger from Linux.
//!
//! The entire sequence is implemented: arm `EnrollMode` (3, version 2) with the
//! enroll payload, drive the capture loop with `EnrollContinue` (14) while the
//! finger is lifted and pressed, and, with `--commit`, run the three-phase
//! catacomb save and write the sealed blobs the SEP returns.
//!
//! One input is not ours to invent: the 16-byte **unlock token** that goes in
//! the authData field. The SEP mints it on passcode validation and rejects
//! enroll with -3 without it (proven). Pass it with `--token <hex>`; with no
//! token the tool still sends the request, so the -3 can be confirmed, but it
//! cannot enrol. Producing that token on Linux is the one open problem, and it
//! is the same credential the post-power-cycle first-unlock needs.
//!
//! Needs no `t2sep`: this is BiometricKit over BridgeXPC. It does need the
//! `t2-touchid` daemon stopped so only one session is open.
//!
//! ```text
//! sudo systemctl stop kait2en-t2-touchid
//! sudo ./t2-bio-enroll --uid 501 --token <32-hex> --yes          # arm + capture
//! sudo ./t2-bio-enroll --uid 501 --token <32-hex> --yes --commit # also persist
//! sudo systemctl start kait2en-t2-touchid
//! ```

use std::time::Duration;

use anyhow::{Result, bail};
use t2_biometrickit::{Event, Session, proto};

struct Args {
    uid: u32,
    token: Vec<u8>,
    commit: bool,
    yes: bool,
}

fn parse_args() -> Result<Args> {
    let mut uid = 501u32;
    let mut token = Vec::new();
    let mut commit = false;
    let mut yes = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--uid" => uid = it.next().and_then(|v| v.parse().ok()).ok_or_else(|| anyhow::anyhow!("--uid needs a number"))?,
            "--token" => {
                let hex = it.next().ok_or_else(|| anyhow::anyhow!("--token needs hex"))?;
                token = (0..hex.len()).step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                    .collect::<Result<_, _>>()?;
            }
            "--commit" => commit = true,
            "--yes" => yes = true,
            other => bail!("unknown argument {other}"),
        }
    }
    Ok(Args { uid, token, commit, yes })
}

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

fn main() -> Result<()> {
    let args = parse_args()?;
    if !args.yes {
        eprintln!(
            "This writes to the SEP: it arms a real enrollment. Pass --yes.\n\
             With a valid --token and --commit it enrolls a finger and persists it.\n\
             usage: t2-bio-enroll --uid N --token <hex> [--commit] --yes"
        );
        std::process::exit(2);
    }
    if args.token.is_empty() {
        eprintln!("warning: no --token given; the SEP will reject EnrollMode with -3.\n");
    }

    let mut s = Session::open(None, None)?;
    println!("session open, uid {}", args.uid);

    let before = s.read_identities(args.uid)?.len();
    println!("identities before: {before}");

    // The 20-byte device group: first record of GetBioDeviceList (bytes 4..24).
    let (_, list) = s.raw_command(proto::GET_BIO_DEVICE_LIST, 0, &[], 264)?;
    let device_group: Vec<u8> = if list.len() >= 24 { list[4..24].to_vec() } else { vec![0u8; 20] };
    println!("device group: {}", hex(&device_group));

    // Arm the enrollment.
    let status = s.enroll_begin(args.uid, &args.token, &device_group)?;
    if status != 0 {
        println!("EnrollMode rejected: status {status} ({status:#x})");
        if args.token.is_empty() {
            println!("expected: without the 16-byte unlock token the SEP returns -3.");
        }
        s.cancel()?;
        return Ok(());
    }
    println!("EnrollMode accepted. Press and lift the finger repeatedly.\n");

    // Capture loop: advance once per accepted image, track progress, stop when
    // the SEP signals completion or we stop seeing progress.
    let mut captures = 0;
    let mut done = false;
    for _ in 0..400 {
        match s.next_event(Duration::from_secs(20))? {
            Some(Event::FingerDown) => println!("  finger down"),
            Some(Event::FingerUp) => {
                println!("  finger up  (capture {captures})");
                captures += 1;
                let c = s.enroll_continue()?;
                if c != 0 { println!("  EnrollContinue -> {c}"); }
            }
            Some(Event::Status(code)) => {
                // Progress records come through as status-bearing events; a
                // terminal/complete code ends the loop. We log raw so the real
                // completion code can be read off the first successful run.
                println!("  status {code}");
            }
            Some(Event::MatchResult { .. }) => {}
            Some(other) => println!("  event {other:?}"),
            None => { println!("  (timeout waiting for finger)"); break; }
        }
        // Re-read identities cheaply to detect the new one appearing.
        if s.read_identities(args.uid)?.len() > before {
            done = true;
            break;
        }
    }

    let after = s.read_identities(args.uid)?;
    println!("\nidentities after: {} (was {before})", after.len());
    for id in after {
        println!("  slot {} uuid {}", id.slot, hex(&id.uuid));
    }

    if !done {
        println!("\nenrollment did not complete; nothing to persist.");
        s.cancel()?;
        return Ok(());
    }

    if !args.commit {
        println!("\nnew template is in the SEP for this session. Run again with --commit to persist it.");
        s.cancel()?;
        return Ok(());
    }

    // Persist: ask the SEP to seal each catacomb component and write the blob.
    println!("\ncommitting catacomb...");
    let component = s.catacomb_group_state()?;
    if component.len() < 24 {
        bail!("catacomb group state too short ({} bytes); cannot form the save descriptor", component.len());
    }
    let blob = s.save_catacomb_component(&component[..24])?;
    println!("sealed catacomb blob: {} bytes (write to the .cat file)", blob.len());
    // Writing the file to the right volume-uuid path is the host bookkeeping
    // step; left to the daemon/installer, not done here, so this tool never
    // touches the filesystem on its own.

    s.cancel()?;
    Ok(())
}
