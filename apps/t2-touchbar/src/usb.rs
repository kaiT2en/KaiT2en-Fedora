// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

use std::{
    fs::{self, OpenOptions},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};

const USBDEVFS_RESET: libc::c_ulong = 0x5514;
const ATTACH_TIMEOUT: Duration = Duration::from_secs(45);

pub fn touchbar_drm_card() -> Option<PathBuf> {
    let entries = fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if !name_text.strip_prefix("card").is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            continue;
        }
        let Ok(driver) = fs::read_link(entry.path().join("device/driver")) else {
            continue;
        };
        if matches!(
            driver.file_name().and_then(|name| name.to_str()),
            Some("t2bdrm" | "appletbdrm" | "adp")
        ) {
            return Some(PathBuf::from("/dev/dri").join(name));
        }
    }
    None
}

pub fn ensure_display_configuration() -> Result<()> {
    if touchbar_drm_card().is_some() {
        return Ok(());
    }
    let deadline = Instant::now() + ATTACH_TIMEOUT;
    let mut next_reprobe = Instant::now();

    loop {
        let Some(device) = find_touchbar_usb() else {
            if Instant::now() >= deadline {
                return Err(anyhow!("Touch Bar USB device 05ac:8302 not found"));
            }
            thread::sleep(Duration::from_millis(250));
            continue;
        };
        let configuration = read_configuration(&device);

        if configuration != "2" || Instant::now() >= next_reprobe {
            if let Err(error) = switch_to_display(&device, &configuration) {
                eprintln!("kait2en-touchbar: attach attempt failed: {error:#}");
            }
            next_reprobe = Instant::now() + Duration::from_secs(2);
        }

        if touchbar_drm_card().is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = restore_firmware_configuration();
            return Err(anyhow!(
                "Touch Bar reached no usable t2bdrm device within 45 seconds"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

pub fn restore_firmware_configuration() -> Result<()> {
    let deadline = Instant::now() + ATTACH_TIMEOUT;
    loop {
        if let Some(device) = find_touchbar_usb() {
            let configuration = read_configuration(&device);
            if configuration == "1" {
                return Ok(());
            }
            let result = (|| {
                if configuration.is_empty() {
                    reset_usb_device(&device)?;
                }
                let path = device.join("bConfigurationValue");
                fs::write(&path, b"0")
                    .with_context(|| format!("unconfigure {}", device.display()))?;
                fs::write(&path, b"1").with_context(|| {
                    format!("restore firmware configuration on {}", device.display())
                })?;
                Ok::<_, anyhow::Error>(())
            })();
            if let Err(error) = result {
                eprintln!("kait2en-touchbar: detach attempt failed: {error:#}");
            }
        }
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "could not restore Touch Bar firmware configuration"
            ));
        }
        thread::sleep(Duration::from_secs(5));
    }
}

fn find_touchbar_usb() -> Option<PathBuf> {
    fs::read_dir("/sys/bus/usb/devices")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            read_trimmed(&path.join("idVendor")) == "05ac"
                && read_trimmed(&path.join("idProduct")) == "8302"
        })
}

fn read_configuration(device: &Path) -> String {
    read_trimmed(&device.join("bConfigurationValue"))
}

fn read_trimmed(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn switch_to_display(device: &Path, configuration: &str) -> Result<()> {
    if configuration.is_empty() {
        reset_usb_device(device)?;
    }
    let path = device.join("bConfigurationValue");
    fs::write(&path, b"0").with_context(|| format!("unconfigure {}", device.display()))?;
    fs::write(&path, b"2")
        .with_context(|| format!("select display configuration on {}", device.display()))?;
    Ok(())
}

fn reset_usb_device(device: &Path) -> Result<()> {
    let bus = read_trimmed(&device.join("busnum"))
        .parse::<u16>()
        .context("parse Touch Bar USB bus number")?;
    let number = read_trimmed(&device.join("devnum"))
        .parse::<u16>()
        .context("parse Touch Bar USB device number")?;
    let path = PathBuf::from(format!("/dev/bus/usb/{bus:03}/{number:03}"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    let result = unsafe { libc::ioctl(file.as_raw_fd(), USBDEVFS_RESET) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("reset Touch Bar USB device");
    }
    Ok(())
}
