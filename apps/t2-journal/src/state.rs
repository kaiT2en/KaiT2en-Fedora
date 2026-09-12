// SPDX-License-Identifier: LicenseRef-KAIT2EN-1.0
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

//! Where t2-journal keeps its parsed bridgeOS snapshot.

use std::path::PathBuf;

use anyhow::{Context, Result, ensure};

pub fn state_file(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        let state_home = PathBuf::from(state_home);
        ensure!(state_home.is_absolute(), "XDG_STATE_HOME must be absolute");
        return Ok(state_home.join("t2-journal/bridgeos.jsonl"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/state/t2-journal/bridgeos.jsonl"))
}
