// SPDX-License-Identifier: MIT

use std::process::Command;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use chrono::DateTime;

use crate::record::Record;

#[derive(Clone, Copy)]
pub enum Boot {
    Offset(i32),
    All,
}

impl FromStr for Boot {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        value
            .parse()
            .map(Self::Offset)
            .map_err(|_| "boot must be a journalctl offset such as 0 or -1".into())
    }
}

impl Boot {
    fn argument(self) -> String {
        match self {
            Self::Offset(offset) => offset.to_string(),
            Self::All => "all".into(),
        }
    }
}

pub fn linux_boot(boot: Boot) -> Result<Vec<Record>> {
    let boot = boot.argument();
    let output = Command::new("journalctl")
        .args([
            "-b",
            &boot,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_boot_selectors() {
        assert!(matches!("-3".parse(), Ok(Boot::Offset(-3))));
        assert!("all".parse::<Boot>().is_err());
        assert!("previous".parse::<Boot>().is_err());
    }
}
