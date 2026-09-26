use std::{fs, path::PathBuf};

use evdev::Device;

use crate::error::{ForceClickError, Result};

const PARAM_DIR: &str = "/sys/module/t2_precision_trackpad/parameters";
const FORCE_CLICK_DEVICE_NAME: &str = "T2 Force Click Events";

pub fn module_loaded() -> bool {
    /* t2_precision_trackpad owns both configurable parameter files. */
    PathBuf::from(PARAM_DIR).is_dir()
}

pub fn read_click_strength() -> Option<u8> {
    read_param_u8("click_strength")
}

pub fn read_force_click_threshold_percent() -> Option<u32> {
    read_param_u32("force_click_threshold_percent")
}

pub fn write_click_strength(value: u8) -> Result<()> {
    write_param("click_strength", &value.to_string())
}

pub fn write_force_click_threshold_percent(value: u32) -> Result<()> {
    write_param("force_click_threshold_percent", &value.to_string())
}

pub fn write_force_click_enabled(value: bool) -> Result<()> {
    write_param("force_click_enabled", if value { "Y" } else { "N" })
}

fn read_param_u8(name: &str) -> Option<u8> {
    read_param(name)?.parse().ok()
}

fn read_param_u32(name: &str) -> Option<u32> {
    read_param(name)?.parse().ok()
}

fn read_param(name: &str) -> Option<String> {
    fs::read_to_string(PathBuf::from(PARAM_DIR).join(name))
        .ok()
        .map(|value| value.trim().to_owned())
}

fn write_param(name: &str, value: &str) -> Result<()> {
    let path = PathBuf::from(PARAM_DIR).join(name);
    fs::write(&path, value).map_err(|source| ForceClickError::Io { path, source })
}

/// Finds the separate event device created by t2_precision_trackpad.
pub fn find_trackpad_device() -> Result<Device> {
    find_trackpad_device_with_path().map(|(_, device)| device)
}

/// Return the evdev path as well as the device. The daemon uses the path to
/// notice re-enumeration after the kernel trackpad module is reloaded.
pub fn find_trackpad_device_with_path() -> Result<(PathBuf, Device)> {
    for (path, device) in evdev::enumerate() {
        if device.name() == Some(FORCE_CLICK_DEVICE_NAME) {
            return Ok((path, device));
        }
    }
    Err(ForceClickError::NoTrackpadDevice)
}
