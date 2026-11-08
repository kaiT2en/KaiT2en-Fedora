// SPDX-License-Identifier: GPL-3.0-or-later
//! GNOME Night Light writes the same gamma LUT. While t2-truetone controls the
//! white point, Night Light is turned off. Its previous value is kept and
//! restored when t2-truetone gives control back. Automatic brightness keeps
//! its own handling in the autobright module.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub const NIGHT_LIGHT: (&str, &str) = ("org.gnome.settings-daemon.plugins.color", "night-light-enabled");

pub fn get(key: (&str, &str)) -> Option<bool> {
    let out = Command::new("gsettings").args(["get", key.0, key.1]).output().ok()?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub fn set(key: (&str, &str), v: bool) -> bool {
    Command::new("gsettings")
        .args(["set", key.0, key.1, if v { "true" } else { "false" }])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn saved_path(key: (&str, &str)) -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(dir.join("t2-truetone").join(format!("saved-{}", key.1)))
}

fn log(msg: &str) {
    eprintln!("t2-truetone: {msg}");
}

/// Take the GNOME feature over: remember its value once, then turn it off.
/// Nothing is changed unless the original value is safely on disk.
pub fn take_over(key: (&str, &str)) {
    let Some(p) = saved_path(key) else {
        return log("no state directory, leaving GNOME settings alone");
    };
    if !p.exists() {
        let Some(v) = get(key) else {
            return log(&format!("could not read {} {}, leaving it alone", key.0, key.1));
        };
        let saved = p
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::write(&p, if v { "true" } else { "false" }));
        if let Err(e) = saved {
            return log(&format!("could not save {}: {e}, leaving {} alone", p.display(), key.1));
        }
    }
    if get(key) != Some(false) && !set(key, false) {
        log(&format!("could not turn off {} {}", key.0, key.1));
    }
}

/// Give the GNOME feature back with the value it had before.
pub fn give_back(key: (&str, &str)) {
    let Some(p) = saved_path(key) else { return };
    let Ok(v) = fs::read_to_string(&p) else { return };
    if !set(key, v.trim() == "true") {
        return log(&format!("could not restore {} {}, keeping {}", key.0, key.1, p.display()));
    }
    if let Err(e) = fs::remove_file(&p) {
        log(&format!("could not remove {}: {e}", p.display()));
    }
}
