// SPDX-License-Identifier: GPL-3.0-or-later

//! Finger contact on the trackpad. libinput reports a touchpad only once the
//! finger moves, its evdev node already reports the contact itself.

use std::{
    fs::{File, OpenOptions},
    io::{ErrorKind, Read},
    mem::size_of,
    os::{
        fd::{AsRawFd, RawFd},
        unix::fs::OpenOptionsExt,
    },
    slice,
};

use input_linux_sys::{BTN_TOUCH, EV_KEY, input_event};

#[derive(Default)]
pub struct Touchpads {
    /// Shared read-only next to libinput, never grabbed.
    devices: Vec<(String, File)>,
}

impl Touchpads {
    pub fn add(&mut self, sysname: &str) {
        self.remove(sysname);
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(format!("/dev/input/{sysname}"))
        {
            Ok(file) => self.devices.push((sysname.to_owned(), file)),
            Err(error) => eprintln!("t2-kbd-backlight: open trackpad {sysname}: {error}"),
        }
    }

    pub fn remove(&mut self, sysname: &str) {
        self.devices.retain(|(name, _)| name != sysname);
    }

    pub fn fds(&self) -> impl Iterator<Item = RawFd> + '_ {
        self.devices.iter().map(|(_, file)| file.as_raw_fd())
    }

    /// Drains every node and tells whether a finger came down. A node that
    /// went away with its device is dropped.
    pub fn touched(&mut self) -> bool {
        let mut touched = false;
        self.devices.retain_mut(|(_, file)| {
            let mut events = [input_event {
                time: input_linux_sys::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: 0,
                code: 0,
                value: 0,
            }; 64];
            loop {
                // SAFETY: input_event is plain old data, any byte pattern is valid.
                let bytes = unsafe {
                    slice::from_raw_parts_mut(
                        events.as_mut_ptr().cast::<u8>(),
                        size_of::<input_event>() * events.len(),
                    )
                };
                match file.read(bytes) {
                    Ok(0) => return false,
                    Ok(read) => {
                        touched |= events[..read / size_of::<input_event>()]
                            .iter()
                            .any(|event| {
                                i32::from(event.type_) == EV_KEY
                                    && i32::from(event.code) == BTN_TOUCH
                                    && event.value == 1
                            });
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => return true,
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(_) => return false,
                }
            }
        });
        touched
    }
}
