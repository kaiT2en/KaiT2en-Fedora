// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::{io, num::ParseIntError, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FanControlError {
    #[error("no T2 fan endpoints found under sysfs")]
    NoFans,
    #[error("glob pattern failed: {0}")]
    Glob(#[from] glob::PatternError),
    #[error("glob iteration failed: {0}")]
    GlobWalk(#[from] glob::GlobError),
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid integer in {path}: {source}")]
    ParseInt {
        path: PathBuf,
        source: ParseIntError,
    },
    #[error("fan path {0} is missing a valid file name")]
    InvalidFanPath(PathBuf),
    #[error("failed to spawn process: {0}")]
    ProcessSpawn(io::Error),
    #[error("command failed: {command}: {stderr}")]
    CommandFailed { command: String, stderr: String },
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, FanControlError>;
