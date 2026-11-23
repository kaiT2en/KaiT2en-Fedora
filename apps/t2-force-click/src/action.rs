// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::process::Command;
use std::str::FromStr;

use evdev::{AttributeSet, EventType, InputEvent, KeyCode};
use evdev::uinput::VirtualDevice;

use crate::config::{ActionKind, AppConfig};
use crate::service::ActiveSession;

/// Owns the synthetic keyboard used for the key-combo action, and runs the
/// configured action when the force click event fires.
pub struct ActionRunner {
    virtual_keyboard: Option<VirtualDevice>,
    copy_paste_next_is_paste: bool,
}

impl ActionRunner {
    pub fn new() -> Self {
        Self {
            virtual_keyboard: build_virtual_keyboard().ok(),
            copy_paste_next_is_paste: false,
        }
    }

    pub fn run(&mut self, config: &AppConfig, session: Option<&ActiveSession>) {
        match config.action_kind {
            ActionKind::None => {}
            ActionKind::CopyPaste => self.run_copy_paste(),
            ActionKind::KeyCombo => self.run_key_combo(&config.key_combo),
            ActionKind::Command => run_command(&config.command, session),
        }
    }

    fn run_key_combo(&mut self, combo: &str) {
        self.emit_key_combo(&parse_key_combo(combo));
    }

    fn run_copy_paste(&mut self) {
        let key = if self.copy_paste_next_is_paste {
            KeyCode::KEY_V
        } else {
            KeyCode::KEY_C
        };
        self.emit_key_combo(&[KeyCode::KEY_LEFTCTRL, key]);
        self.copy_paste_next_is_paste = !self.copy_paste_next_is_paste;
    }

    fn emit_key_combo(&mut self, keys: &[KeyCode]) {
        let Some(device) = self.virtual_keyboard.as_mut() else {
            eprintln!("t2-force-click: no virtual keyboard available, skipping key combo");
            return;
        };
        if keys.is_empty() {
            return;
        }
        let press: Vec<InputEvent> = keys
            .iter()
            .map(|key| InputEvent::new(EventType::KEY.0, key.0, 1))
            .collect();
        let release: Vec<InputEvent> = keys
            .iter()
            .rev()
            .map(|key| InputEvent::new(EventType::KEY.0, key.0, 0))
            .collect();
        if let Err(error) = device.emit(&press) {
            eprintln!("t2-force-click: key combo press failed: {error}");
            return;
        }
        if let Err(error) = device.emit(&release) {
            eprintln!("t2-force-click: key combo release failed: {error}");
        }
    }

}

fn run_command(command: &str, session: Option<&ActiveSession>) {
    if command.trim().is_empty() {
        return;
    }
    let Some(session) = session else {
        eprintln!("t2-force-click: no active graphical session, skipping command");
        return;
    };
    let runtime_dir = format!("/run/user/{}", session.uid);
    if let Err(error) = Command::new("runuser")
        .args(["--preserve-environment", "-u", &session.user, "--", "sh", "-c", command])
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={runtime_dir}/bus"))
        .spawn()
    {
        eprintln!("t2-force-click: command spawn failed: {error}");
    }
}

/// Unknown names are skipped.

fn parse_key_combo(combo: &str) -> Vec<KeyCode> {
    combo
        .split('+')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter_map(|name| key_from_name(name))
        .collect()
}

fn key_from_name(name: &str) -> Option<KeyCode> {
    let upper = format!("KEY_{}", name.to_uppercase());
    KeyCode::from_str(&upper).ok()
}

/// Linux KEY_MAX is 0x2ff. The virtual keyboard registers every key code.
const KEY_MAX: u16 = 0x2ff;

fn build_virtual_keyboard() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for code in 0..=KEY_MAX {
        keys.insert(KeyCode(code));
    }
    VirtualDevice::builder()?
        .name("t2-force-click virtual keyboard")
        .with_keys(&keys)?
        .build()
}
