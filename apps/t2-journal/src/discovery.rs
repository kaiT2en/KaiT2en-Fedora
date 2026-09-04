// SPDX-License-Identifier: MIT

use std::fs;
use std::net::Ipv6Addr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};

pub const DEFAULT_HOST: &str = "fe80::aede:48ff:fe33:4455";
pub const FIRST_DYNAMIC_PORT: u16 = 49152;
pub const LAST_DYNAMIC_PORT: u16 = 65535;

pub fn interface(explicit: Option<String>) -> Result<String> {
    if let Some(interface) = explicit {
        return Ok(interface);
    }
    let mut matches = Vec::new();
    for item in fs::read_dir("/sys/class/net")? {
        let item = item?;
        let driver = item.path().join("device/driver");
        if fs::canonicalize(driver)
            .ok()
            .and_then(|p| p.file_name().map(|n| n == "cdc_ncm"))
            != Some(true)
        {
            continue;
        }
        let device = fs::canonicalize(item.path().join("device"))?;
        if apple_ncm(&device) {
            matches.push(item.file_name().to_string_lossy().into_owned());
        }
    }
    match matches.as_slice() {
        [name] => Ok(name.clone()),
        [] => bail!("no Apple 05ac:8233 CDC-NCM interface found"),
        _ => bail!("multiple Apple CDC-NCM interfaces found; use --interface"),
    }
}

fn apple_ncm(path: &Path) -> bool {
    path.ancestors().any(|parent| {
        let vendor = fs::read_to_string(parent.join("idVendor")).ok();
        let product = fs::read_to_string(parent.join("idProduct")).ok();
        vendor
            .as_deref()
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("05ac"))
            && product
                .as_deref()
                .is_some_and(|p| p.trim().eq_ignore_ascii_case("8233"))
    })
}

pub fn host(interface: &str, explicit: Option<String>) -> Result<Ipv6Addr> {
    ensure_link_local(interface)?;
    if let Some(host) = explicit {
        return host.parse().context("invalid IPv6 host");
    }
    let _ = Command::new("ping")
        .args(["-6", "-c", "1", "-W", "1", &format!("ff02::1%{interface}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let output = Command::new("ip")
        .args(["-j", "-6", "neighbor", "show", "dev", interface])
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let entries: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            let mut hosts: Vec<Ipv6Addr> = entries
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| {
                    !matches!(
                        entry.get("state").and_then(|state| state.as_str()),
                        Some("FAILED" | "INCOMPLETE")
                    )
                })
                .filter_map(|entry| entry.get("dst")?.as_str()?.split('%').next()?.parse().ok())
                .filter(|address: &Ipv6Addr| (address.segments()[0] & 0xffc0) == 0xfe80)
                .collect();
            hosts.sort_unstable();
            hosts.dedup();
            if let [host] = hosts.as_slice() {
                return Ok(*host);
            }
        }
    }
    DEFAULT_HOST.parse().context("invalid built-in T2 address")
}

fn ensure_link_local(interface: &str) -> Result<()> {
    let addresses = fs::read_to_string("/proc/net/if_inet6")
        .context("IPv6 is unavailable; cannot reach the T2 link-local service")?;
    let found = addresses.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields.len() == 6 && fields[3] == "20" && fields[5] == interface
    });
    ensure!(
        found,
        "interface {interface} has no IPv6 link-local address; enable IPv6 on the CDC-NCM link"
    );
    Ok(())
}

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
