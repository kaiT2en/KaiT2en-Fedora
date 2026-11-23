// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

use std::{ffi::c_char, fs::OpenOptions};

use anyhow::{Context, Result};
use input_linux::{EventKind, Key, SynchronizeKind, uinput::UInputHandle};
use input_linux_sys::{input_event, input_id, timeval, uinput_setup};

const ALLOWED_KEYS: [Key; 33] = [
    Key::Esc,
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
    Key::BrightnessDown,
    Key::BrightnessUp,
    Key::MicMute,
    Key::IllumDown,
    Key::IllumUp,
    Key::PreviousSong,
    Key::PlayPause,
    Key::NextSong,
    Key::Sysrq,
    Key::Insert,
    Key::Delete,
    Key::Home,
    Key::End,
    Key::PageUp,
    Key::PageDown,
    Key::Rewind,
    Key::FastForward,
    Key::Mute,
    Key::VolumeDown,
    Key::VolumeUp,
];

pub struct VirtualKeyboard {
    handle: UInputHandle<std::fs::File>,
    pressed: Option<Key>,
}

impl VirtualKeyboard {
    /// `extra` adds the keys of the personal keys file to the fixed set.
    pub fn open(extra: &[Key]) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .context("open /dev/uinput")?;
        let handle = UInputHandle::new(file);
        handle.set_evbit(EventKind::Key)?;
        for key in ALLOWED_KEYS.iter().chain(extra) {
            handle.set_keybit(*key)?;
        }
        let mut name = [0 as c_char; 80];
        for (target, byte) in name.iter_mut().zip(b"T2 Touch Bar".iter().copied()) {
            *target = byte as c_char;
        }
        handle.dev_setup(&uinput_setup {
            id: input_id {
                bustype: 0x19,
                vendor: 0x1209,
                product: 0x4b32,
                version: 1,
            },
            ff_effects_max: 0,
            name,
        })?;
        handle.dev_create()?;
        Ok(Self {
            handle,
            pressed: None,
        })
    }

    pub fn press(&mut self, key: Key) -> Result<()> {
        self.release()?;
        emit(&mut self.handle, key, 1)?;
        self.pressed = Some(key);
        Ok(())
    }

    /// Taps a combination: modifiers and key down in order, up in reverse.
    pub fn tap_combination(&mut self, keys: &[Key]) -> Result<()> {
        self.release()?;
        for key in keys {
            emit(&mut self.handle, *key, 1)?;
        }
        for key in keys.iter().rev() {
            emit(&mut self.handle, *key, 0)?;
        }
        Ok(())
    }

    pub fn release(&mut self) -> Result<()> {
        if let Some(key) = self.pressed.take() {
            emit(&mut self.handle, key, 0)?;
        }
        Ok(())
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn emit(handle: &mut UInputHandle<std::fs::File>, key: Key, value: i32) -> Result<()> {
    let event = |kind: EventKind, code: u16, value| input_event {
        value,
        type_: kind as u16,
        code,
        time: timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
    };
    handle.write(&[
        event(EventKind::Key, key as u16, value),
        event(EventKind::Synchronize, SynchronizeKind::Report as u16, 0),
    ])?;
    Ok(())
}
