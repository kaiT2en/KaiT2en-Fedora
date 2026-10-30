// SPDX-License-Identifier: GPL-3.0-or-later

use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

const MIN_IDLE_TIMEOUT_MS: u64 = 5_000;
const MAX_IDLE_TIMEOUT_MS: u64 = 3_600_000;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub idle_timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_timeout_ms: 30_000,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let config: Self = match fs::read_to_string(&path) {
            Ok(body) => toml::from_str(&body)
                .with_context(|| format!("invalid configuration {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        anyhow::ensure!(
            (MIN_IDLE_TIMEOUT_MS..=MAX_IDLE_TIMEOUT_MS).contains(&config.idle_timeout_ms),
            "idle_timeout_ms must be {MIN_IDLE_TIMEOUT_MS}..{MAX_IDLE_TIMEOUT_MS}"
        );
        Ok(config)
    }
}

pub fn config_path() -> PathBuf {
    env::var_os("T2_KBD_BACKLIGHT_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/kait2en/kbd-backlight.toml"))
}
