// SPDX-License-Identifier: GPL-3.0-or-later
//! User settings in $XDG_CONFIG_HOME/t2-truetone/config.toml, written by the
//! GUI and reloaded by the daemon when the file changes.

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub true_tone: bool,
    /// 0.0 leaves the panel at D65. The app offers 0.0 to 0.3.
    pub strength: f64,
    /// Seconds for the white point to settle after a change in ambient light.
    pub fade_s: f64,
    pub night: bool,
    /// Start and end of the evening mode as "HH:MM", local time.
    pub night_from: String,
    pub night_to: String,
    pub night_kelvin: f64,
    /// Minutes over which the evening mode fades in and out.
    pub night_ramp_min: f64,
    pub auto_brightness: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            true_tone: true,
            strength: crate::model::DEFAULT_STRENGTH,
            fade_s: 5.0,
            night: false,
            night_from: "21:00".into(),
            night_to: "07:00".into(),
            night_kelvin: 3400.0,
            night_ramp_min: 10.0,
            auto_brightness: true,
        }
    }
}

pub fn dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("t2-truetone")
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

pub fn mtime() -> Option<SystemTime> {
    fs::metadata(path()).and_then(|m| m.modified()).ok()
}

/// Missing file: defaults. Unreadable or broken file: defaults and the reason.
pub fn load_checked() -> (Config, Option<String>) {
    let p = path();
    match fs::read_to_string(&p) {
        Ok(t) => match toml::from_str(&t) {
            Ok(c) => (sanitize(c), None),
            Err(e) => (Config::default(), Some(format!("{}: {e}", p.display()))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
        Err(e) => (Config::default(), Some(format!("{}: {e}", p.display()))),
    }
}

pub fn load() -> Config {
    let (c, err) = load_checked();
    if let Some(e) = err {
        eprintln!("t2-truetone: using defaults, {e}");
    }
    c
}

fn sanitize(mut c: Config) -> Config {
    c.strength = c.strength.clamp(0.0, 0.3);
    c.fade_s = c.fade_s.clamp(0.0, 10.0);
    c.night_kelvin = c.night_kelvin.clamp(1900.0, 6500.0);
    c.night_ramp_min = c.night_ramp_min.clamp(0.0, 10.0);
    c
}

pub fn save(c: &Config) -> std::io::Result<()> {
    fs::create_dir_all(dir())?;
    let tmp = path().with_extension("toml.tmp");
    fs::write(&tmp, toml::to_string_pretty(c).map_err(std::io::Error::other)?)?;
    fs::rename(tmp, path())
}

/// "HH:MM" to minutes after midnight.
pub fn parse_hhmm(s: &str) -> Option<f64> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h * 60 + m) as f64)
}
