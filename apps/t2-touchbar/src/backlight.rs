// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow};

pub struct Backlight {
    file: File,
    maximum: u32,
    current: Option<u32>,
}

impl Backlight {
    pub fn open() -> Result<Self> {
        let path = find_backlight()?;
        let maximum = fs::read_to_string(path.join("max_brightness"))
            .context("read Touch Bar max_brightness")?
            .trim()
            .parse()
            .context("parse Touch Bar max_brightness")?;
        let file = OpenOptions::new()
            .write(true)
            .open(path.join("brightness"))
            .context("open Touch Bar brightness")?;
        Ok(Self {
            file,
            maximum,
            current: None,
        })
    }

    pub fn set(&mut self, value: u32) -> Result<()> {
        let value = value.min(self.maximum);
        if self.current == Some(value) {
            return Ok(());
        }
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(format!("{value}\n").as_bytes())?;
        self.file.flush()?;
        self.current = Some(value);
        Ok(())
    }
}

fn find_backlight() -> Result<PathBuf> {
    for name in ["t2tb_backlight", "appletb_backlight"] {
        let path = PathBuf::from("/sys/class/backlight").join(name);
        if path.join("brightness").exists() {
            return Ok(path);
        }
    }
    Err(anyhow!("Touch Bar backlight not found"))
}
