// SPDX-License-Identifier: MIT

use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::DateTime;

use crate::record::Record;

pub fn linux_boot(offset: i32) -> Result<Vec<Record>> {
    let output = Command::new("journalctl")
        .args([
            "-b",
            &offset.to_string(),
            "--utc",
            "--no-pager",
            "--quiet",
            "-o",
            "short-iso-precise",
        ])
        .output()
        .context("failed to execute journalctl")?;
    if !output.status.success() {
        bail!(
            "journalctl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut records = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((timestamp, message)) = line.split_once(' ') else {
            continue;
        };
        let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp) else {
            continue;
        };
        records.push(Record {
            timestamp_ns: timestamp.timestamp_nanos_opt().unwrap_or_default(),
            source: "LNX".into(),
            process: String::new(),
            subsystem: String::new(),
            category: String::new(),
            message: message.into(),
        });
    }
    Ok(records)
}

pub fn list_boots() -> Result<i32> {
    let status = Command::new("journalctl")
        .args(["--list-boots", "--no-pager"])
        .status()
        .context("failed to execute journalctl")?;
    Ok(status.code().unwrap_or(1))
}
