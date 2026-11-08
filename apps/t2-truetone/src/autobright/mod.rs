// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

//! Automatic display brightness for T2 Intel Macs, following macOS.
//!
//! The panel follows a learned lux curve, but only when the target differs
//! from the current brightness by at least 20 %. Changes fade in steps of
//! 1/1024 on a perceptual scale, so they are never visible as steps. Moving
//! the brightness by hand pauses the automatics, and one second after the
//! last movement the curve learns the new point.
//!
//! On GNOME the service writes through mutter with gnome-shell's own
//! automatics off. Its shell extension moves the slider along, so the slider
//! and the brightness keys always start from the real brightness.
//!
//! This was the standalone t2-autobrightness service. It runs as a thread of
//! the t2-truetone daemon and gets its lux from the shared sensor poller.

pub mod backlight;
pub mod curve;
pub mod gnome;
pub mod shell;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use zbus::blocking::Connection;

use backlight::Backlight;
use curve::Curve;

const COMMIT_DELAY: Duration = Duration::from_secs(1);
const MIN_STEP: Duration = Duration::from_millis(10);
const WAKE: Duration = Duration::from_millis(50);
const DEAD_BAND: f32 = 0.2;
const STEPS_PER_UNIT: f32 = 1024.0;
pub const EXTENSION: &str = "kait2en-autobrightness@kait2en";

/// The built-in panel: written through mutter, read back from the kernel.
/// Brightness inside the service is linear, `raw / max`.
struct Panel<'a> {
    connection: &'a Connection,
    backlight: Backlight,
    actual: PathBuf,
}

impl<'a> Panel<'a> {
    fn find(connection: &'a Connection, backlight: Backlight) -> Result<Self> {
        let read = |path: &Path| fs::read_to_string(path).ok()?.trim().parse::<i32>().ok();
        let device = fs::read_dir("/sys/class/backlight")
            .context("reading /sys/class/backlight")?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|device| read(&device.join("max_brightness")) == Some(backlight.max))
            .ok_or_else(|| anyhow!("no backlight device with max {}", backlight.max))?;
        Ok(Panel { connection, backlight, actual: device.join("actual_brightness") })
    }

    fn linear(&self) -> Result<f32> {
        let text = fs::read_to_string(&self.actual).with_context(|| format!("reading {}", self.actual.display()))?;
        Ok(text.trim().parse::<f32>()? / self.backlight.max as f32)
    }

    fn min_linear(&self) -> f32 {
        self.backlight.min as f32 / self.backlight.max as f32
    }

    fn raw(&self, linear: f32) -> i32 {
        ((linear * self.backlight.max as f32).round() as i32).clamp(self.backlight.min, self.backlight.max)
    }

    /// mutter hands out a new serial when the monitor setup changes, so a
    /// refused write is retried once with fresh data.
    fn show(&mut self, linear: f32) -> Result<()> {
        let value = self.raw(linear);
        if backlight::set(self.connection, &self.backlight, value).is_ok() {
            return Ok(());
        }
        self.backlight = backlight::read(self.connection)?.ok_or_else(|| anyhow!("the built-in backlight is gone"))?;
        backlight::set(self.connection, &self.backlight, value)
    }
}

/// A fade that follows the clock. Each pass computes how far the fade should
/// be and moves there, so a late pass skips ahead instead of replaying the
/// missed steps in a burst.
struct Ramp {
    from: f32,
    delta: f32,
    steps: u32,
    done: u32,
    start: Instant,
    interval: Duration,
    target: f32,
}

impl Ramp {
    fn new(current: f32, target: f32, now: Instant) -> Self {
        let from = curve::perceptual(current);
        let delta = curve::perceptual(target) - from;
        let steps = ((delta.abs() * STEPS_PER_UNIT).floor() as u32).max(1);
        let interval = Duration::from_secs_f32(curve::ramp_seconds(delta) / steps as f32).max(MIN_STEP);
        Ramp { from, delta, steps, done: 0, start: now, interval, target }
    }

    /// The brightness to show now, if the fade has moved since the last call.
    fn advance(&mut self, now: Instant) -> Option<f32> {
        let due = (now.duration_since(self.start).as_secs_f32() / self.interval.as_secs_f32()) as u32;
        let step = due.min(self.steps);
        if step <= self.done {
            return None;
        }
        self.done = step;
        Some(if step == self.steps {
            self.target
        } else {
            curve::linear(self.from + self.delta * step as f32 / self.steps as f32)
        })
    }

    fn finished(&self) -> bool {
        self.done >= self.steps
    }

    fn next(&self) -> Instant {
        self.start + self.interval * (self.done + 1)
    }
}

/// Live state for the status file.
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub linear: Option<f32>,
    pub curve: Option<Curve>,
}

/// Run until `stop` is set. Lux arrives on `lux` from the shared sensor.
pub fn run(lux: Receiver<f32>, stop: Arc<AtomicBool>, live: Arc<Mutex<Live>>) -> Result<()> {
    let state = state_dir()?;
    fs::create_dir_all(&state).with_context(|| format!("creating {}", state.display()))?;
    migrate(&state);
    let curve_path = state.join("curve");
    let saved_settings = state.join("gnome-settings");

    // Without a controllable built-in panel there is nothing to do.
    let connection = Connection::session().context("connecting to the session bus")?;
    let Some(backlight) = backlight::read(&connection)? else {
        return unsupported("mutter reports no built-in backlight");
    };
    let mut panel = match Panel::find(&connection, backlight) {
        Ok(panel) => panel,
        Err(error) => return unsupported(&format!("{error:#}")),
    };
    let product = fs::read_to_string("/sys/class/dmi/id/product_name").unwrap_or_default();
    let product = product.trim();
    let curve = Curve::load(&curve_path).unwrap_or_else(|| Curve::factory(product));
    log(&format!(
        "{product}, {} range {}..{}, curve {curve:?}",
        panel.backlight.connector, panel.backlight.min, panel.backlight.max
    ));

    gnome::take_over(&saved_settings)?;
    if let Err(error) = gnome::enable_extension(EXTENSION) {
        log(&format!("enabling the shell extension failed: {error:#}"));
    }
    shell::disable_auto_target(&connection)?;
    let result = run_loop(lux, &stop, &live, &mut panel, curve, &curve_path);
    if let Err(error) = gnome::restore(&saved_settings) {
        log(&format!("restoring GNOME settings failed: {error:#}"));
    }
    *live.lock().unwrap() = Live::default();
    result
}

fn run_loop(
    readings: Receiver<f32>,
    stop: &AtomicBool,
    live: &Mutex<Live>,
    panel: &mut Panel,
    mut curve: Curve,
    curve_path: &Path,
) -> Result<()> {
    let (manual_sender, manual) = mpsc::channel();
    shell::watch(manual_sender);

    let mut ramp: Option<Ramp> = None;
    let mut commit_at: Option<Instant> = None;
    let mut lux: Option<f32> = None;
    let mut force = true;
    live.lock().unwrap().curve = Some(curve.clone());

    while !stop.load(Ordering::SeqCst) {
        let now = Instant::now();

        let changes = manual.try_iter().count();
        if changes > 0 {
            if commit_at.is_none() {
                log(&format!("manual brightness change ({changes} signals), automatics paused"));
            }
            ramp = None;
            commit_at = Some(now + COMMIT_DELAY);
        }

        let fresh = readings.try_iter().last().map(f32::floor);
        if fresh.is_some() {
            lux = fresh;
        }

        if commit_at.is_some_and(|at| now >= at) {
            commit_at = None;
            let linear = panel.linear()?;
            let at = lux.unwrap_or(0.0);
            curve.learn(at, linear);
            log(&format!("learned {linear:.4} at {at} lux: {curve:?}"));
            if let Err(error) = curve.save(curve_path) {
                log(&format!("{error:#}"));
            }
            live.lock().unwrap().curve = Some(curve.clone());
        }

        if commit_at.is_none()
            && let (Some(_), Some(at)) = (fresh, lux)
        {
            let target = curve.linear_for_lux(at, panel.min_linear(), 1.0);
            let current = panel.linear()?;
            live.lock().unwrap().linear = Some(current);
            // The T2 only reports real changes, so every macOS event retargets.
            // Polling sees every sample, so a running fade only restarts when
            // its own target is off by the dead band.
            let reference = ramp.as_ref().map_or(current, |active| active.target);
            let same_raw = panel.raw(target) == panel.raw(current);
            if !same_raw && (force || (target - reference).abs() / reference >= DEAD_BAND) {
                let next = Ramp::new(current, target, now);
                log(&format!(
                    "{at} lux: {current:.4} -> {target:.4} in {} steps of {:?}",
                    next.steps, next.interval
                ));
                ramp = Some(next);
            }
            force = false;
        }

        if let Some(active) = ramp.as_mut() {
            if let Some(linear) = active.advance(now) {
                panel.show(linear)?;
            }
            if active.finished() {
                ramp = None;
            }
        }

        let mut wait = WAKE;
        if let Some(active) = &ramp {
            wait = wait.min(active.next().saturating_duration_since(Instant::now()));
        }
        if let Some(at) = commit_at {
            wait = wait.min(at.saturating_duration_since(Instant::now()));
        }
        sleep(wait.max(Duration::from_millis(1)));
    }
    Ok(())
}

fn unsupported(reason: &str) -> Result<()> {
    log(&format!("automatic brightness has nothing to control: {reason}"));
    Ok(())
}

fn state_base() -> Result<PathBuf> {
    Ok(match env::var_os("XDG_STATE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".local/state"),
    })
}

fn state_dir() -> Result<PathBuf> {
    Ok(state_base()?.join("t2-truetone").join("brightness"))
}

/// Take over the learned curve and the saved GNOME settings of the former
/// t2-autobrightness service, once.
fn migrate(state: &Path) {
    let Ok(old) = state_base().map(|b| b.join("t2-autobrightness")) else { return };
    for name in ["curve", "gnome-settings"] {
        let (from, to) = (old.join(name), state.join(name));
        if !from.exists() || to.exists() {
            continue;
        }
        match fs::copy(&from, &to) {
            Ok(_) => {
                log(&format!("took over {}", from.display()));
                if let Err(e) = fs::remove_file(&from) {
                    log(&format!("could not remove {}: {e}", from.display()));
                }
            }
            Err(e) => log(&format!("could not take over {}: {e}", from.display())),
        }
    }
    // Only an empty directory goes, anything else in it is left in place.
    if fs::read_dir(&old).is_ok_and(|mut d| d.next().is_none()) {
        if let Err(e) = fs::remove_dir(&old) {
            log(&format!("could not remove {}: {e}", old.display()));
        }
    }
}

pub(crate) fn log(message: &str) {
    eprintln!("t2-truetone: brightness: {message}");
}
