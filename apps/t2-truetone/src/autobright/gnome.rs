// SPDX-License-Identifier: GPL-3.0-or-later
//! Keeps GNOME's own brightness automatics out of the way while the service
//! runs, and gives the user's settings back when it stops.
//!
//! The previous values are written to disk before anything is changed, and
//! only if no saved copy exists yet, so a crash and restart doesn't overwrite
//! the user's original settings with the service's own.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

const SCHEMA: &str = "org.gnome.settings-daemon.plugins.power";
/// Ambient light handling in gsd-power, and idle dimming, which would write
/// its own value back on wake.
const KEYS: [&str; 2] = ["ambient-enabled", "idle-dim"];

fn gsettings(args: &[&str]) -> Result<String> {
    let output = Command::new("gsettings").args(args).output().context("running gsettings")?;
    if !output.status.success() {
        bail!("gsettings {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn take_over(saved: &Path) -> Result<()> {
    if !saved.exists() {
        let mut text = String::new();
        for key in KEYS {
            text.push_str(&format!("{key}={}\n", gsettings(&["get", SCHEMA, key])?));
        }
        fs::write(saved, text).with_context(|| format!("writing {}", saved.display()))?;
    }
    for key in KEYS {
        gsettings(&["set", SCHEMA, key, "false"])?;
    }
    Ok(())
}

/// The extension keeps the shell's slider on the brightness the service sets.
/// It is turned on for every user the service runs for, unless the user
/// switched it off, which puts it on the disabled list.
pub fn enable_extension(uuid: &str) -> Result<()> {
    let quoted = format!("'{uuid}'");
    if gsettings(&["get", "org.gnome.shell", "disabled-extensions"])?.contains(&quoted) {
        return Ok(());
    }
    let enabled = gsettings(&["get", "org.gnome.shell", "enabled-extensions"])?;
    if enabled.contains(&quoted) {
        return Ok(());
    }
    let list = match enabled.trim_start_matches("@as ").trim_end_matches(']').trim() {
        "[" => format!("[{quoted}]"),
        start => format!("{start}, {quoted}]"),
    };
    gsettings(&["set", "org.gnome.shell", "enabled-extensions", &list])?;
    Ok(())
}

pub fn restore(saved: &Path) -> Result<()> {
    let Ok(text) = fs::read_to_string(saved) else {
        return Ok(());
    };
    for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
        if KEYS.contains(&key) {
            gsettings(&["set", SCHEMA, key, value])?;
        }
    }
    fs::remove_file(saved).with_context(|| format!("removing {}", saved.display()))
}
