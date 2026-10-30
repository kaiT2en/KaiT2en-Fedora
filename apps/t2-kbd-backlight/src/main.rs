// SPDX-License-Identifier: GPL-3.0-or-later

//! Turns the keyboard backlight off after a while without keyboard, trackpad
//! or Touch Bar input, and back on with the next input.

mod config;
mod dimmer;
mod kbdlight;
mod sleep;
mod touchpad;

use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::fs::OpenOptionsExt,
    },
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result};
use input::{
    Device as InputDevice, DeviceCapability, Libinput, LibinputInterface, SendEventsMode,
    event::{DeviceEvent, Event, EventTrait, touch::TouchEvent},
};
use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY, pollfd};

use config::{Config, config_path};
use dimmer::Dimmer;
use kbdlight::KbdBacklight;
use touchpad::Touchpads;

/// The Touch Bar sits on its own seat, see the t2-touchbar udev rules.
const TOUCH_BAR_SEAT: &str = "seat-touchbar";

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn stop(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}

struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> std::result::Result<OwnedFd, i32> {
        let mode = flags & O_ACCMODE;
        OpenOptions::new()
            .custom_flags(flags)
            .read(mode == O_RDONLY || mode == O_RDWR)
            .write(mode == O_WRONLY || mode == O_RDWR)
            .open(path)
            .map(Into::into)
            .map_err(|error| error.raw_os_error().unwrap_or(libc::EIO))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(File::from(fd));
    }
}

fn main() -> Result<()> {
    let config = Config::load()?;
    if !config.enabled {
        println!(
            "t2-kbd-backlight: disabled in {}, leaving the keyboard backlight alone",
            config_path().display()
        );
        return Ok(());
    }

    unsafe {
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
    }

    let mut dimmer = Dimmer::new(KbdBacklight::connect()?, config.idle_timeout_ms);
    let mut main_input = Libinput::new_with_udev(Interface);
    main_input
        .udev_assign_seat("seat0")
        .map_err(|()| anyhow::anyhow!("assign main seat"))?;
    let mut touch_bar_input = Libinput::new_with_udev(Interface);
    touch_bar_input
        .udev_assign_seat(TOUCH_BAR_SEAT)
        .map_err(|()| anyhow::anyhow!("assign Touch Bar seat"))?;
    let mut kbd_wake = kbdlight::watch()?;
    kbd_wake.set_nonblocking(true)?;
    let (sleep_rx, mut sleep_wake) = sleep::watch()?;
    sleep_wake.set_nonblocking(true)?;
    let mut touchpads = Touchpads::default();
    // libinput queues the devices present at start without its fd becoming
    // readable, so the first round dispatches without waiting for input.
    let mut startup = true;

    while !STOP.load(Ordering::Relaxed) {
        let mut poll_fds: Vec<pollfd> = [
            main_input.as_fd().as_raw_fd(),
            touch_bar_input.as_fd().as_raw_fd(),
            kbd_wake.as_raw_fd(),
            sleep_wake.as_raw_fd(),
        ]
        .into_iter()
        .chain(touchpads.fds())
        .map(|fd| pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
        let result = unsafe {
            libc::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as _,
                if startup { 0 } else { dimmer.next_timeout() },
            )
        };
        if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(std::io::Error::last_os_error()).context("poll input devices");
        }

        if startup || poll_fds[0].revents & libc::POLLIN != 0 {
            main_input.dispatch()?;
            for event in &mut main_input {
                match event {
                    Event::Device(DeviceEvent::Added(added)) => {
                        let device = added.device();
                        if device.has_capability(DeviceCapability::Gesture) {
                            touchpads.add(device.sysname());
                        }
                        ignore_unless_input(&device);
                    }
                    Event::Device(DeviceEvent::Removed(removed)) => {
                        touchpads.remove(removed.device().sysname());
                    }
                    Event::Keyboard(_) | Event::Pointer(_) => dimmer.activity(),
                    _ => {}
                }
            }
        }

        if startup || poll_fds[1].revents & libc::POLLIN != 0 {
            touch_bar_input.dispatch()?;
            for event in &mut touch_bar_input {
                match event {
                    Event::Device(DeviceEvent::Added(added)) => {
                        ignore_unless_input(&added.device())
                    }
                    Event::Touch(TouchEvent::Down(_)) => dimmer.activity(),
                    _ => {}
                }
            }
        }

        if poll_fds[2].revents & libc::POLLIN != 0 {
            let mut discard = [0u8; 64];
            while kbd_wake.read(&mut discard).is_ok() {}
            dimmer.changed();
        }

        if poll_fds[3].revents & libc::POLLIN != 0 {
            let mut discard = [0u8; 64];
            while sleep_wake.read(&mut discard).is_ok() {}
            while let Ok(done) = sleep_rx.try_recv() {
                dimmer.prepare_for_sleep();
                let _ = done.send(());
            }
        }

        if poll_fds[4..].iter().any(|fd| fd.revents != 0) && touchpads.touched() {
            dimmer.activity();
        }

        dimmer.tick();
        startup = false;
    }
    dimmer.restore_on_exit();
    Ok(())
}

/// Devices that cannot report input, such as lid and power switches, are
/// not worth waking up for.
fn ignore_unless_input(device: &InputDevice) {
    let input = [
        DeviceCapability::Keyboard,
        DeviceCapability::Pointer,
        DeviceCapability::Touch,
    ]
    .into_iter()
    .any(|capability| device.has_capability(capability));
    if !input {
        let _ = device.config_send_events_set_mode(SendEventsMode::DISABLED);
    }
}
