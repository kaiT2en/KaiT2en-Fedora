// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! Turn decoded os-log rows into display lines, and filter them.
//!
//! Shared by the CLI and GUI so both render and filter log rows identically.

use anyhow::{Context, Result};

use crate::archive::Obj;
use crate::tap;

/// A formatted os-log line plus the fields the filters and colouring use.
#[derive(Clone)]
pub struct LogLine {
    pub pid: u64,
    pub subsystem: String,
    /// The message type (Default, Info, Debug, Error, Fault) for colouring.
    pub level: String,
    pub text: String,
}

/// Build the display line for an os-log row, or None for non-log rows
/// (signpost intervals and the tap's own argument rows).
pub fn format_log_row(row: &tap::Row, mapping: Option<&TimeMapping>) -> Option<LogLine> {
    if row.table != "os-log" {
        return None;
    }
    let pid = row.integer("process").unwrap_or_default();
    let subsystem = row.string("subsystem").unwrap_or_default();
    let category = row.string("category").unwrap_or_default();
    let message_type = row.string("message-type").unwrap_or_default();
    let image = row
        .string("sender-image-path")
        .and_then(|path| path.rsplit('/').next().map(str::to_owned))
        .unwrap_or_default();
    let line = row.message().unwrap_or_default();
    let shown_subsystem = if subsystem.is_empty() { "-" } else { &subsystem };
    let category = if category.is_empty() { "-".to_owned() } else { category };
    let time = mapping
        .zip(row.integer("time"))
        .map(|(m, mach)| m.utc(mach))
        .unwrap_or_else(|| "-".repeat(27));
    let text =
        format!("{time}  {pid:<5} {message_type:<7} {shown_subsystem}:{category}  {image}  {line}");
    Some(LogLine { pid, subsystem, level: message_type, text })
}

/// Client-side log filter. An all-lowercase `grep`/`exclude` pattern matches
/// case-insensitively, like t2journal and journalctl.
#[derive(Default)]
pub struct Filter {
    grep: Option<regex::Regex>,
    subsystem: Option<String>,
    pid: Option<u64>,
    exclude: Option<regex::Regex>,
    no_hid: bool,
}

impl Filter {
    pub fn new(
        grep: Option<&str>,
        subsystem: Option<&str>,
        pid: Option<u64>,
        exclude: Option<&str>,
        no_hid: bool,
    ) -> Result<Self> {
        let compile = |pattern: &str| {
            let insensitive = pattern.chars().all(|c| !c.is_ascii_uppercase());
            regex::RegexBuilder::new(pattern).case_insensitive(insensitive).build()
        };
        Ok(Self {
            grep: grep.map(compile).transpose().context("invalid grep regex")?,
            subsystem: subsystem.map(str::to_lowercase),
            pid,
            exclude: exclude.map(compile).transpose().context("invalid exclude regex")?,
            no_hid,
        })
    }

    pub fn passes(&self, line: &LogLine) -> bool {
        if let Some(pid) = self.pid {
            if line.pid != pid {
                return false;
            }
        }
        if self.no_hid {
            let subsystem = line.subsystem.to_lowercase();
            if subsystem.contains("hid") || subsystem.contains("inputdevice") {
                return false;
            }
        }
        if let Some(subsystem) = &self.subsystem {
            if !line.subsystem.to_lowercase().contains(subsystem) {
                return false;
            }
        }
        if let Some(grep) = &self.grep {
            if !grep.is_match(&line.text) {
                return false;
            }
        }
        if let Some(exclude) = &self.exclude {
            if exclude.is_match(&line.text) {
                return false;
            }
        }
        true
    }
}

/// The device's wall clock at one mach continuous time, from the tap's
/// "Time Mapping" notice.
pub struct TimeMapping {
    continuous: u64,
    since1970: f64,
    numerator: u64,
    denominator: u64,
}

impl TimeMapping {
    /// UTC string for a row's mach continuous time.
    pub fn utc(&self, mach: u64) -> String {
        let delta = mach as f64 - self.continuous as f64;
        let seconds = self.since1970 + delta * self.numerator as f64 / self.denominator as f64 / 1e9;
        format_utc(seconds)
    }
}

pub fn parse_time_mapping(obj: &Obj) -> Option<TimeMapping> {
    // The notice is nested in the tap message: {DTTapMessagePlist: {notice, info}}.
    let plist = obj.get("DTTapMessagePlist").unwrap_or(obj);
    if plist.get("notice").and_then(Obj::as_str) != Some("Time Mapping") {
        return None;
    }
    let info = plist.get("info")?;
    let (numerator, denominator) = (
        info.get("DTActivityTraceTapDeviceMachTimebaseNumeratorKey")?.as_u64()?,
        info.get("DTActivityTraceTapDeviceMachTimebaseDenominatorKey")?.as_u64()?,
    );
    if numerator == 0 || denominator == 0 {
        return None;
    }
    Some(TimeMapping {
        continuous: info.get("DTActivityTraceTapDeviceMachContinuousTimeKey")?.as_u64()?,
        since1970: info.get("DTActivityTraceTapDeviceTimeIntervalSince1970Key")?.as_f64()?,
        numerator,
        denominator,
    })
}

/// Format Unix seconds as `YYYY-MM-DDTHH:MM:SS.ffffffZ` (UTC), no dependencies.
pub fn format_utc(seconds: f64) -> String {
    let whole = seconds.floor() as i64;
    let micros = ((seconds - whole as f64) * 1e6).round() as u32;
    let days = whole.div_euclid(86_400);
    let rem = whole.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{micros:06}Z")
}

/// Gregorian date from days since the Unix epoch (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}
