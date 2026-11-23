// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::{io, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ZonePowerError {
    #[error("no t2smc hwmon device found")]
    NoSmc,
    #[error("glob pattern failed: {0}")]
    Glob(#[from] glob::PatternError),
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid value in {path}: {value:?}")]
    Parse { path: PathBuf, value: String },
}

pub type Result<T> = std::result::Result<T, ZonePowerError>;
