// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::process::Command;

use crate::error::{FanControlError, Result};

const SERVICE_NAME: &str = "t2-fancontrol.service";

pub fn autostart_enabled() -> bool {
    Command::new("systemctl")
        .args(["is-enabled", "--quiet", SERVICE_NAME])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> Result<()> {
    if enabled {
        enable_service()?;
    } else {
        disable_service()?;
    }
    Ok(())
}

fn enable_service() -> Result<()> {
    run_systemctl(["enable", SERVICE_NAME])?;
    Ok(())
}

fn disable_service() -> Result<()> {
    run_systemctl(["disable", SERVICE_NAME])?;
    Ok(())
}

fn run_systemctl<const N: usize>(args: [&str; N]) -> Result<()> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(FanControlError::ProcessSpawn)?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(FanControlError::CommandFailed {
        command: format!("systemctl {}", args.join(" ")),
        stderr,
    })
}
