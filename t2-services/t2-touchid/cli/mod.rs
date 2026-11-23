// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use anyhow::{Result, bail};

pub struct Config {
    pub socket: String,
    /// The macOS user id whose fingers to verify against; 0 means find it.
    pub user_id: u32,
    pub flags: u32,
    /// Linux account that gets the enrolled fingers bound to it automatically.
    pub bind_user: Option<String>,
}

pub fn config() -> Result<Config> {
    let mut socket = String::from("/run/t2-touchid/fprint.sock");
    let mut user_id = 501;
    let mut flags = 0;
    let mut bind_user = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        let mut value = || args.next().ok_or_else(|| anyhow::anyhow!("{argument} needs a value"));
        match argument.as_str() {
            "--socket" => socket = value()?,
            "--uid" => {
                let raw = value()?;
                user_id = if raw == "auto" { 0 } else { raw.parse()? };
            }
            "--flags" => {
                let raw = value()?;
                flags = raw
                    .strip_prefix("0x")
                    .map_or_else(|| raw.parse(), |hex| u32::from_str_radix(hex, 16))?;
            }
            "--bind-user" => {
                let name = value()?;
                bind_user = (!name.is_empty() && name != "none").then_some(name);
            }
            other => bail!("unknown argument {other}"),
        }
    }
    Ok(Config { socket, user_id, flags, bind_user })
}
