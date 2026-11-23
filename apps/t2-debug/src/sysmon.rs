// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! Decode sysmontap samples into structured system and per-process rows.

use crate::archive::Obj;
use crate::dtx::Payload;

/// One process in a sysmon sample.
#[derive(Clone)]
pub struct Proc {
    pub pid: i64,
    pub name: String,
    pub cpu: f64,
    pub power: f64,
    pub wakeups: i64,
    pub foot_mb: f64,
    pub disk_kb: f64,
}

/// The `setConfig:` dict for a 1 Hz sysmon stream.
pub fn config(proc_attrs: &[String], sys_attrs: &[String]) -> Obj {
    Obj::Dict(vec![
        ("ur".into(), Obj::Int(1000)),
        ("bm".into(), Obj::Int(0)),
        ("sampleInterval".into(), Obj::Int(1_000_000_000)),
        ("cpuUsage".into(), Obj::Bool(true)),
        ("physFootprint".into(), Obj::Bool(true)),
        ("procAttrs".into(), string_array(proc_attrs)),
        ("sysAttrs".into(), string_array(sys_attrs)),
    ])
}

/// The system counters of a sample, as (name, formatted value) pairs.
pub fn decode_system(values: &[Obj], attrs: &[String]) -> Vec<(String, String)> {
    attrs
        .iter()
        .zip(values)
        .map(|(name, value)| (name.clone(), scalar(value)))
        .collect()
}

/// The processes of a sample, sorted by CPU descending, optionally truncated.
pub fn decode_processes(processes: &Obj, attrs: &[String], top: Option<usize>) -> Vec<Proc> {
    let Obj::Dict(entries) = processes else {
        return Vec::new();
    };
    let index = |name: &str| attrs.iter().position(|a| a == name);
    let (pid, name) = (index("pid"), index("name"));
    let (cpu, power) = (index("cpuUsage"), index("powerScore"));
    let (wake, foot, disk) =
        (index("intWakeups"), index("physFootprint"), index("diskBytesWritten"));

    let mut rows: Vec<&Vec<Obj>> = entries
        .iter()
        .filter_map(|(_, value)| match value {
            Obj::Array(values) => Some(values),
            _ => None,
        })
        .collect();
    let num = |row: &Vec<Obj>, i: Option<usize>| i.and_then(|i| row.get(i)).and_then(Obj::as_f64);
    rows.sort_by(|a, b| {
        num(b, cpu)
            .unwrap_or(0.0)
            .partial_cmp(&num(a, cpu).unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(top) = top {
        rows.truncate(top);
    }
    rows.into_iter()
        .map(|row| Proc {
            pid: num(row, pid).unwrap_or_default() as i64,
            name: name
                .and_then(|i| row.get(i))
                .and_then(Obj::as_str)
                .unwrap_or("?")
                .to_owned(),
            cpu: num(row, cpu).unwrap_or_default(),
            power: num(row, power).unwrap_or_default(),
            wakeups: num(row, wake).unwrap_or_default() as i64,
            foot_mb: num(row, foot).unwrap_or_default() / 1024.0 / 1024.0,
            disk_kb: num(row, disk).unwrap_or_default() / 1024.0,
        })
        .collect()
}

pub fn scalar(value: &Obj) -> String {
    match value {
        Obj::Int(v) => v.to_string(),
        Obj::UInt(v) => v.to_string(),
        Obj::Real(v) => format!("{v:.2}"),
        other => format!("{other:?}"),
    }
}

/// The strings of an array reply (e.g. sysmon attribute lists).
pub fn string_list(payload: Payload) -> Vec<String> {
    match payload {
        Payload::Obj(Obj::Array(items)) => {
            items.into_iter().filter_map(|item| item.as_str().map(str::to_owned)).collect()
        }
        _ => Vec::new(),
    }
}

pub fn string_array(items: &[String]) -> Obj {
    Obj::Array(items.iter().map(|s| Obj::Str(s.clone())).collect())
}
