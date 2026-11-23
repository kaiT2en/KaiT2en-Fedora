// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::error::{Result, ZonePowerError};

pub fn read_string(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .map_err(|source| ZonePowerError::Io { path: path.to_owned(), source })
}

pub fn read_value<T: FromStr>(path: &Path) -> Result<T> {
    let value = read_string(path)?;
    value
        .parse()
        .map_err(|_| ZonePowerError::Parse { path: path.to_owned(), value })
}

pub fn write_value(path: &Path, value: impl ToString) -> Result<()> {
    fs::write(path, value.to_string())
        .map_err(|source| ZonePowerError::Io { path: path.to_owned(), source })
}

pub fn glob_paths(pattern: &str) -> Result<Vec<PathBuf>> {
    Ok(glob::glob(pattern)?.filter_map(|entry| entry.ok()).collect())
}

/// hwmon directory of the t2smc driver
pub fn find_t2smc() -> Result<PathBuf> {
    glob_paths("/sys/class/hwmon/hwmon*")?
        .into_iter()
        .find(|dir| read_string(&dir.join("name")).is_ok_and(|name| name == "t2smc"))
        .ok_or(ZonePowerError::NoSmc)
}
