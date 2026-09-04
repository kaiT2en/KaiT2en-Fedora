// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use regex::Regex;

use crate::archive;
use crate::record::Record;

pub fn parse(root: &Path) -> Result<Vec<Record>> {
    let timestamp = Regex::new(
        r"(\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:\.\d+)?)(?:Z|\s*[+-]\d{2}:?\d{2})?",
    )?;
    let mut records = Vec::new();

    for path in archive::walk(root)? {
        if !path.is_file()
            || path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("._"))
        {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name == "sysdiagnose.log" {
            parse_sysdiagnose(&path, &mut records)?;
        } else if wanted(&name) {
            parse_timestamped(&path, &name, &timestamp, &mut records)?;
        } else if name.starts_with("stacks-") && name.ends_with(".ips") {
            parse_stackshot(&path, &name, &mut records)?;
        }
    }
    Ok(records)
}

fn wanted(name: &str) -> bool {
    matches!(
        name,
        "IOPower.txt" | "BridgeActivation.log" | "watchdog.log" | "Panics.log"
    ) || name.starts_with("usermanagerd.log.")
        || name.starts_with("multiversed-")
        || name.starts_with("restoreserviced-")
        || name.starts_with("logd")
}

fn parse_sysdiagnose(path: &Path, records: &mut Vec<Record>) -> Result<()> {
    let data = fs::read(path).with_context(|| format!("open {}", path.display()))?;
    let text = String::from_utf8_lossy(&data);
    let first_record = records.len();
    for line in text.lines() {
        let Some((raw, message)) = line.split_once(": ") else {
            if records.len() > first_record
                && let Some(last) = records.last_mut()
            {
                last.message.push('\n');
                last.message.push_str(line);
            }
            continue;
        };
        let Ok(time) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S") else {
            continue;
        };
        records.push(record(
            time.and_utc().timestamp_nanos_opt().unwrap_or_default(),
            "sysdiagnose",
            message,
        ));
    }
    Ok(())
}

fn parse_timestamped(
    path: &Path,
    name: &str,
    timestamp: &Regex,
    records: &mut Vec<Record>,
) -> Result<()> {
    let data = fs::read(path).with_context(|| format!("open {}", path.display()))?;
    let text = String::from_utf8_lossy(&data);
    for line in text.lines() {
        let Some(raw) = timestamp.captures(&line).and_then(|capture| capture.get(1)) else {
            continue;
        };
        let Some(time) = parse_naive(raw.as_str()) else {
            continue;
        };
        if time.and_utc().year() >= 2000 {
            records.push(record(
                time.and_utc().timestamp_nanos_opt().unwrap_or_default(),
                name,
                &line,
            ));
        }
    }
    Ok(())
}

fn parse_stackshot(path: &Path, name: &str, records: &mut Vec<Record>) -> Result<()> {
    let data = fs::read(path).with_context(|| format!("open {}", path.display()))?;
    let text = String::from_utf8_lossy(&data);
    let Some(line) = text.lines().next() else {
        return Ok(());
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) else {
        return Ok(());
    };
    let Some(raw) = json.get("timestamp").and_then(|value| value.as_str()) else {
        return Ok(());
    };
    let Ok(time) = DateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f %z") else {
        return Ok(());
    };
    records.push(record(
        time.with_timezone(&Utc)
            .timestamp_nanos_opt()
            .unwrap_or_default(),
        name,
        "stackshot",
    ));
    Ok(())
}

fn parse_naive(raw: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f"))
        .ok()
}

fn record(timestamp_ns: i64, process: &str, message: &str) -> Record {
    Record {
        timestamp_ns,
        source: "T2".into(),
        process: process.into(),
        subsystem: String::new(),
        category: String::new(),
        message: message.into(),
    }
}
