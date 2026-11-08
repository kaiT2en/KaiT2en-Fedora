// SPDX-License-Identifier: GPL-3.0-or-later
//! Live state published by the daemon in $XDG_RUNTIME_DIR/t2-truetone/status.json.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    pub unix_time: u64,
    pub sensor: bool,
    pub lux: f64,
    pub sensor_cct: f64,
    pub sensor_xy: [f64; 2],
    pub ambient_cct: f64,
    pub white_cct: f64,
    pub gains: [f64; 3],
    pub night_factor: f64,
    pub brightness: Option<f64>,
    pub error: Option<String>,
}

pub fn path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("t2-truetone")
        .join("status.json")
}

pub fn write(s: &Status) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(s).map_err(std::io::Error::other)?)?;
    fs::rename(tmp, p)
}

pub fn read() -> Option<Status> {
    serde_json::from_slice(&fs::read(path()).ok()?).ok()
}
