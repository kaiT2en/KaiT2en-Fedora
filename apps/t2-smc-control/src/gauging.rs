// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! Gauging mitigation like BatteryGaugingAccuracyManager in powerd (macOS
//! 15.4.1). A permanent charge limit keeps the gauge from learning Qmax and
//! DOD0. powerd lifts the limit for a full charge when DOD0 did not change
//! for 3 days or the last full charge is older than 21 days, and restores it
//! once none of that holds anymore.
//!
//! powerd also charges when Qmax did not change for 14 days, but only after a
//! qualified Qmax within the last 30 hours. It learns that from the battery
//! property Dod0AtQualifiedQmax, SMC key BDAQ. T2 SMCs do not have BDAQ
//! (bridgeOS 23.16 and 24.16), so powerd never charges for Qmax there and
//! neither does this.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gtk4::glib;

const STATE_FILE: &str = "/var/lib/t2-smc-control/gauging";
const BATTERY_STATUS: &str = "/sys/class/power_supply/BAT0/status";

const DOD0_MAX_AGE: u64 = 3 * 86_400;
const FULL_CHARGE_MAX_AGE: u64 = 21 * 86_400;
/// powerd gives up on a full charge attempt after 12 hours
const FULL_CHARGE_ATTEMPT: u64 = 12 * 3_600;
const POLL_SECONDS: u32 = 60;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

#[derive(Default, Debug, PartialEq)]
struct State {
    dod0: String,
    dod0_changed: u64,
    full_charge: u64,
    attempt_started: u64,
}

impl State {
    fn load() -> Self {
        let mut state = Self::default();
        let Ok(text) = fs::read_to_string(STATE_FILE) else { return state };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let number = || value.parse().unwrap_or(0);
            match key {
                "dod0" => state.dod0 = value.to_owned(),
                "dod0_changed" => state.dod0_changed = number(),
                "full_charge" => state.full_charge = number(),
                "attempt_started" => state.attempt_started = number(),
                _ => {}
            }
        }
        state
    }

    fn save(&self) {
        let text = format!(
            "dod0={}\ndod0_changed={}\nfull_charge={}\nattempt_started={}\n",
            self.dod0, self.dod0_changed, self.full_charge, self.attempt_started
        );
        let path = Path::new(STATE_FILE);
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(err) = fs::write(path, text) {
            eprintln!("cannot write {STATE_FILE}: {err}");
        }
    }

    /// disableSocLimitPolicy: true while a full charge is needed
    fn update(&mut self, now: u64, dod0: &str, fully_charged: bool) -> bool {
        if self.dod0 != dod0 {
            self.dod0 = dod0.to_owned();
            self.dod0_changed = now;
        }
        if fully_charged || self.full_charge == 0 {
            self.full_charge = now;
            self.attempt_started = 0;
        }

        let mut charge = now - self.dod0_changed >= DOD0_MAX_AGE;
        if now - self.full_charge > FULL_CHARGE_MAX_AGE {
            if self.attempt_started == 0 {
                self.attempt_started = now;
            } else if now - self.attempt_started > FULL_CHARGE_ATTEMPT {
                self.full_charge = now;
                self.attempt_started = 0;
            }
            charge |= self.attempt_started != 0;
        }
        charge
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|text| text.trim().to_owned())
}

fn set_suspended(hwmon: &Path, suspended: bool) {
    let path = hwmon.join("battery_charge_limit_suspended");
    let value = if suspended { "1" } else { "0" };
    if read_trimmed(&path).as_deref() == Some(value) {
        return;
    }
    match fs::write(&path, value) {
        Ok(()) => println!("charge limit {}", if suspended { "lifted for a calibration charge" } else { "restored" }),
        Err(err) => eprintln!("cannot write {}: {err}", path.display()),
    }
}

fn tick(hwmon: &Path, state: &mut State) {
    let Some(dod0) = read_trimmed(&hwmon.join("smc_battery_dod0")) else { return };
    // sbs reports Full while the limit holds, t2sbs only when the gauge flags
    // it, so the SMC SoC decides
    let fully_charged = matches!(read_trimmed(Path::new(BATTERY_STATUS)).as_deref(), Some("Full" | "Not charging"))
        && read_trimmed(&hwmon.join("smc_battery_capacity_percent"))
            .and_then(|soc| soc.parse::<u32>().ok())
            .is_some_and(|soc| soc >= 99);
    let before = format!("{state:?}");
    let charge = state.update(now(), &dod0, fully_charged);
    if format!("{state:?}") != before {
        state.save();
    }
    set_suspended(hwmon, charge);
}

pub fn run(hwmon: PathBuf) -> Result<(), String> {
    if !hwmon.join("battery_charge_limit_suspended").exists() {
        return Err("t2smc has no battery_charge_limit_suspended, nothing to do".into());
    }

    let state = Rc::new(RefCell::new(State::load()));
    tick(&hwmon, &mut state.borrow_mut());

    let main_loop = glib::MainLoop::new(None, false);
    {
        let (hwmon, state) = (hwmon.clone(), state.clone());
        glib::timeout_add_seconds_local(POLL_SECONDS, move || {
            tick(&hwmon, &mut state.borrow_mut());
            glib::ControlFlow::Continue
        });
    }
    for signal in [SIGTERM, SIGINT] {
        let main_loop = main_loop.clone();
        glib::unix_signal_add_local(signal, move || {
            main_loop.quit();
            glib::ControlFlow::Break
        });
    }
    main_loop.run();

    // The SMC keeps CHLS over reboots, leave it with the user limit
    set_suspended(&hwmon, false);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    fn settled(now: u64) -> State {
        State {
            dod0: "8736".into(),
            dod0_changed: now,
            full_charge: now,
            attempt_started: 0,
        }
    }

    #[test]
    fn dod0_unchanged_for_three_days_asks_for_a_full_charge() {
        let mut state = settled(1_000_000);
        assert!(!state.update(1_000_000 + 2 * DAY, "8736", false));
        assert!(state.update(1_000_000 + 3 * DAY, "8736", false));
        assert!(!state.update(1_000_000 + 3 * DAY + 1, "8700", false));
    }

    #[test]
    fn old_full_charge_is_attempted_for_twelve_hours() {
        let start = 1_000_000;
        let mut state = settled(start);
        let mut now = start;
        // Keep DOD0 moving so only the full charge age matters
        for day in 1..=22 {
            now = start + day * DAY;
            let charge = state.update(now, &format!("{}", 8000 + day), false);
            assert_eq!(charge, day == 22, "day {day}");
        }
        assert!(state.update(now + 6 * 3_600, "8100", false));
        assert!(!state.update(now + 13 * 3_600, "8101", false));
    }
}
