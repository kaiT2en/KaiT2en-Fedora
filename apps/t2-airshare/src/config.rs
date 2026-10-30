// SPDX-License-Identifier: GPL-3.0-or-later
//! User settings in `~/.config/t2-airshare/config.toml`, shared by the service
//! and the settings window. The service reads them for every transfer, so
//! changes apply without a restart. Settings of T2 Share, the former name,
//! in `~/.config/t2-share` are taken over on first use.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Where received files go, the XDG download directory when unset.
    pub download_dir: Option<PathBuf>,
    /// Device names whose transfers are accepted without asking.
    pub trusted_devices: Vec<String>,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        let config = dirs::config_dir()?;
        let dir = config.join("t2-airshare");
        let old = config.join("t2-share");
        if !dir.exists() && old.is_dir() {
            let _ = std::fs::rename(&old, &dir);
        }
        Some(dir.join("config.toml"))
    }

    /// The saved settings, defaults when there is no file yet.
    pub fn load() -> Result<Self> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("parse {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
        }
    }

    /// Writes the settings through a temporary file, so the service never
    /// reads a half written file.
    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("no configuration directory")?;
        let dir = path.parent().expect("path has a parent");
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)
            .with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replace {}", path.display()))?;
        Ok(())
    }

    pub fn download_dir(&self) -> Option<PathBuf> {
        self.download_dir
            .clone()
            .or_else(dirs::download_dir)
            .or_else(dirs::home_dir)
    }

    pub fn is_trusted(&self, name: &str) -> bool {
        self.trusted_devices.iter().any(|trusted| trusted == name)
    }

    pub fn trust(&mut self, name: &str) {
        if !self.is_trusted(name) {
            self.trusted_devices.push(name.to_string());
        }
    }
}
