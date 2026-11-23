// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::error::{FanControlError, Result};

#[derive(Clone, Debug)]
pub struct FanEndpoint {
    pub name: String,
    pub base_path: PathBuf,
    pub min_speed: u32,
    pub max_speed: u32,
    pub current_speed: Option<u32>,
    pub app_controlled: Option<bool>,
    /// true = t2smc/macsmc hwmon (_target), false = applesmc (_output + _manual)
    uses_target_api: bool,
}

#[derive(Clone, Debug)]
pub struct TemperatureSource {
    pub key: String,
    pub name: String,
    pub path: PathBuf,
    pub last_temp_c: Option<u8>,
    pub role: TemperatureRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemperatureRole { CpuDie, CpuTelemetry, Gpu, System }

#[derive(Clone, Debug, Default)]
pub struct TemperatureSnapshot {
    pub cpu_temp_c: Option<u8>,
    pub gpu_temp_c: Option<u8>,
    pub hottest_temp_c: Option<u8>,
    pub hottest_sensor_name: Option<String>,
    pub optional_curve_temp_c: Option<u8>,
    pub overall_hottest_temp_c: Option<u8>,
    pub overall_hottest_sensor_name: Option<String>,
    pub system_temp_c: Option<u8>,
    pub system_sensor_count: usize,
    pub monitored_sensor_count: usize,
}

impl TemperatureSnapshot {
    pub fn read_from(sources: &mut [TemperatureSource]) -> Self {
        for source in sources.iter_mut() {
            source.last_temp_c = read_temperature(&source.path).ok();
        }
        Self::from_cached(sources)
    }

    pub fn read_for_control(
        sources: &mut [TemperatureSource],
        optional_key: Option<&str>,
        refresh_all: bool,
    ) -> Self {
        for source in sources.iter_mut() {
            let needed_for_fast_control = matches!(source.role, TemperatureRole::CpuDie | TemperatureRole::Gpu)
                || optional_key.is_some_and(|key| source.key == key);
            if refresh_all || needed_for_fast_control {
                source.last_temp_c = read_temperature(&source.path).ok();
            }
        }
        Self::from_cached(sources)
    }

    fn from_cached(sources: &[TemperatureSource]) -> Self {
        let mut snapshot = Self::default();
        let mut cpu_sum = 0_u32;
        let mut cpu_count = 0_usize;
        let mut system_sum = 0_u32;
        let mut system_count = 0_usize;
        for source in sources {
            let Some(temp) = source.last_temp_c.filter(|temp| *temp > 0) else { continue; };
            snapshot.monitored_sensor_count += 1;
            if snapshot.overall_hottest_temp_c.map_or(true, |hottest| temp > hottest) {
                snapshot.overall_hottest_temp_c = Some(temp);
                snapshot.overall_hottest_sensor_name = Some(source.name.clone());
            }
            match source.role {
                TemperatureRole::CpuDie => {
                    cpu_sum += temp as u32;
                    cpu_count += 1;
                }
                TemperatureRole::CpuTelemetry => {}
                TemperatureRole::Gpu => {
                    snapshot.gpu_temp_c = Some(snapshot.gpu_temp_c.map_or(temp, |old| old.max(temp)));
                    system_sum += temp as u32;
                    system_count += 1;
                }
                TemperatureRole::System => {
                    system_sum += temp as u32;
                    system_count += 1;
                }
            }
        }
        snapshot.cpu_temp_c = (cpu_count > 0)
            .then(|| ((cpu_sum + cpu_count as u32 / 2) / cpu_count as u32) as u8);
        for (temp, name) in [
            (snapshot.cpu_temp_c, "CPU"),
            (snapshot.gpu_temp_c, "GPU"),
        ] {
            if temp.is_some_and(|temp| snapshot.hottest_temp_c.map_or(true, |old| temp > old)) {
                snapshot.hottest_temp_c = temp;
                snapshot.hottest_sensor_name = Some(name.into());
            }
        }
        snapshot.system_sensor_count = system_count;
        snapshot.system_temp_c = (system_count > 0)
            .then(|| ((system_sum + system_count as u32 / 2) / system_count as u32) as u8);
        snapshot
    }

    pub fn effective_temp_c(&self) -> Option<u8> {
        self.hottest_temp_c
    }

    pub fn include_curve_sensor(&mut self, sources: &[TemperatureSource], key: Option<&str>) {
        let Some(key) = key else { return; };
        let Some(source) = sources.iter().find(|source| source.key == key) else { return; };
        let Some(temp) = source.last_temp_c.filter(|temp| *temp > 0) else { return; };
        self.optional_curve_temp_c = Some(temp);
        if self.hottest_temp_c.map_or(true, |hottest| temp > hottest) {
            self.hottest_temp_c = Some(temp);
            self.hottest_sensor_name = Some(source.name.clone());
        }
    }
}

impl FanEndpoint {
    pub fn refresh_state(&mut self) -> Result<()> {
        self.current_speed = Some(read_u32(&join_suffix(&self.base_path, "_input"))?);
        if !self.uses_target_api {
            self.app_controlled = read_u32(&join_suffix(&self.base_path, "_manual"))
                .ok()
                .map(|manual| manual != 0);
        }
        Ok(())
    }

    pub fn set_target_speed(&self, requested_speed: u32) -> Result<()> {
        let clamped = requested_speed.clamp(self.min_speed, self.max_speed);
        if self.uses_target_api {
            write_string(
                &join_suffix(&self.base_path, "_target"),
                &clamped.to_string(),
            )
        } else {
            write_string(&join_suffix(&self.base_path, "_manual"), "1")?;
            write_string(
                &join_suffix(&self.base_path, "_output"),
                &clamped.to_string(),
            )
        }
    }

    pub fn release_to_auto(&self) -> Result<()> {
        if self.uses_target_api {
            write_string(&join_suffix(&self.base_path, "_target"), "0")
        } else {
            write_string(&join_suffix(&self.base_path, "_manual"), "0")
        }
    }

    pub fn percent_to_rpm(&self, percent: u8) -> u32 {
        let span = self.max_speed.saturating_sub(self.min_speed);
        self.min_speed + (span * percent as u32 / 100)
    }
}

/// Find the hwmon device under /sys/class/hwmon/ with the given name.
fn find_hwmon_by_name(name: &str) -> Option<PathBuf> {
    let pattern = "/sys/class/hwmon/hwmon*/name";
    for entry in glob::glob(pattern).ok()? {
        let Ok(path) = entry else {
            continue;
        };
        if fs::read_to_string(&path).ok().map_or(false, |n| n.trim() == name) {
            return path.parent().map(Path::to_path_buf);
        }
    }
    None
}

fn discover_fans_hwmon(name: &str) -> Option<Result<Vec<FanEndpoint>>> {
    let hwmon_dir = find_hwmon_by_name(name)?;
    let pattern = format!("{}/fan*_input", hwmon_dir.display());
    let mut fans = Vec::new();

    let entries = match glob::glob(&pattern) {
        Ok(e) => e,
        Err(_) => return None,
    };

    for entry in entries {
        let input_path = match entry {
            Ok(p) => p,
            Err(_) => continue,
        };
        let fan_path = match input_to_base_path(&input_path) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let name = match fan_path
            .file_name()
            .and_then(|v| v.to_str())
        {
            Some(n) => n.to_owned(),
            None => continue,
        };

        let min_speed = read_u32(&join_suffix(&fan_path, "_min")).unwrap_or(0);
        let max_speed = match read_u32(&join_suffix(&fan_path, "_max")) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let current_speed = read_u32(&input_path).ok();
        // fanN_target reads the SMC target in both modes, the daemon sets
        // this once it applies a speed or hands the fan back
        let app_controlled = None;

        fans.push(FanEndpoint {
            name,
            base_path: fan_path,
            min_speed,
            max_speed,
            current_speed,
            app_controlled,
            uses_target_api: true,
        });
    }

    fans.sort_by(|left, right| left.name.cmp(&right.name));
    if fans.is_empty() {
        None
    } else {
        Some(Ok(fans))
    }
}

fn discover_fans_acpi() -> Option<Result<Vec<FanEndpoint>>> {
    let pattern = "/sys/devices/pci*/*/*/*/APP0001:00/fan*_input";
    let mut entries = match glob::glob(pattern) {
        Ok(e) => e,
        Err(_) => return None,
    };

    let first_fan = match entries.find_map(|e| e.ok()) {
        Some(p) => p,
        None => return None,
    };

    let fan_dir = first_fan.parent()?;
    let pattern = format!("{}/fan*_input", fan_dir.display());
    let mut fans = Vec::new();

    let entries = match glob::glob(&pattern) {
        Ok(e) => e,
        Err(_) => return None,
    };

    for entry in entries {
        let input_path = match entry {
            Ok(p) => p,
            Err(_) => continue,
        };
        let fan_path = match input_to_base_path(&input_path) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let name = match fan_path
            .file_name()
            .and_then(|v| v.to_str())
        {
            Some(n) => n.to_owned(),
            None => continue,
        };

        let min_speed = read_u32(&join_suffix(&fan_path, "_min")).unwrap_or(0);
        let max_speed = match read_u32(&join_suffix(&fan_path, "_max")) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let current_speed = read_u32(&input_path).ok();
        let app_controlled = read_u32(&join_suffix(&fan_path, "_manual"))
            .ok()
            .map(|manual| manual != 0);

        fans.push(FanEndpoint {
            name,
            base_path: fan_path,
            min_speed,
            max_speed,
            current_speed,
            app_controlled,
            uses_target_api: false,
        });
    }

    fans.sort_by(|left, right| left.name.cmp(&right.name));
    if fans.is_empty() {
        None
    } else {
        Some(Ok(fans))
    }
}

pub fn discover_fans() -> Result<Vec<FanEndpoint>> {
    discover_fans_hwmon("t2smc")
        .or_else(|| discover_fans_hwmon("macsmc"))
        .or_else(discover_fans_acpi)
        .unwrap_or(Err(FanControlError::NoFans))
}

pub fn discover_temperature_sources() -> Vec<TemperatureSource> {
    let mut sources = Vec::new();

    // Temperatures from SMC sensors (t2smc, macsmc, or applesmc hwmon)
    let smc_hwmon = find_hwmon_by_name("t2smc")
        .or_else(|| find_hwmon_by_name("macsmc"))
        .or_else(|| find_hwmon_by_name("applesmc"));
    if let Some(hwmon_dir) = smc_hwmon {
        let pattern = format!("{}/temp*_label", hwmon_dir.display());
        if let Ok(entries) = glob::glob(&pattern) {
            for entry in entries.flatten() {
                let Ok(label) = fs::read_to_string(&entry) else {
                    continue;
                };
                let label = label.trim();
                let temp_path = entry.with_file_name(entry.file_name().unwrap_or_default().to_string_lossy().replace("_label", "_input"));
                if temp_path.exists() && read_temperature(&temp_path).is_ok() {
                    sources.push(TemperatureSource {
                        key: label.to_owned(),
                        name: sensor_label(label),
                        path: temp_path,
                        last_temp_c: None,
                        role: temperature_role(label),
                    });
                }
            }
        }
    }

    sources
}

fn temperature_role(key: &str) -> TemperatureRole {
    if matches!(key, "TC0E" | "TC0F") {
        TemperatureRole::CpuDie
    } else if is_cpu_telemetry_key(key) {
        TemperatureRole::CpuTelemetry
    } else if matches!(key, "TCGC" | "TCGc" | "TG0P" | "TG0D" | "TG1D" | "TG0H" | "TG1H" | "TGDD" | "TGDF" | "TGVP") {
        TemperatureRole::Gpu
    } else {
        TemperatureRole::System
    }
}

fn is_cpu_telemetry_key(key: &str) -> bool {
    is_cpu_core_key(key)
        || matches!(key, "TCXC" | "TCXc" | "TC0D" | "TCAD" | "TC1D" | "TCBD" | "TC1E" | "TC1F")
}

fn is_cpu_core_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.len() == 4
        && bytes[0] == b'T'
        && bytes[1] == b'C'
        && bytes[3] == b'C'
        && matches!(bytes[2], b'1'..=b'8')
}

fn sensor_label(key: &str) -> String {
    let bytes = key.as_bytes();
    if bytes.len() == 4 && bytes[0] == b'T' && bytes[1] == b'C' && bytes[3] == b'C' {
        if let Some(index) = (bytes[2] as char).to_digit(10).filter(|index| (1..=8).contains(index)) {
            return format!("CPU Core {index}");
        }
    }
    match key {
        "TA0V" => String::from("Ambient"),
        "TA0P" => String::from("Airflow 1"),
        "TA1P" => String::from("Airflow 2"),
        "TA0S" => String::from("PCI Slot 1 Pos 1"),
        "TA1S" => String::from("PCI Slot 1 Pos 2"),
        "TA2S" => String::from("PCI Slot 2 Pos 1"),
        "TA3S" => String::from("PCI Slot 2 Pos 2"),
        "TB0T" => String::from("Battery TS_MAX"),
        "TB1T" => String::from("Battery 1"),
        "TB2T" => String::from("Battery 2"),
        "TB3T" => String::from("Battery"),
        "Tb0P" => String::from("BLC Proximity"),
        "TC0P" => String::from("CPU 1 Proximity"),
        "TC0H" => String::from("CPU 1 Heatsink"),
        "TC0D" => String::from("CPU 1 Diode"),
        "TC0E" => String::from("CPU 1 Diode Virtual"),
        "TC0F" => String::from("CPU 1 Diode Filtered"),
        "TCAH" => String::from("CPU 1 Heatsink Alt."),
        "TCAD" => String::from("CPU 1 Package"),
        "TC1P" => String::from("CPU 2 Proximity"),
        "TC1H" => String::from("CPU 2 Heatsink"),
        "TC1D" => String::from("CPU 2 Package"),
        "TC1E" => String::from("CPU 2 Diode Virtual"),
        "TC1F" => String::from("CPU 2 Diode Filtered"),
        "TCBH" => String::from("CPU 2 Heatsink Alt"),
        "TCBD" => String::from("CPU 2 Package Alt"),
        "TCGC" => String::from("GPU Intel Graphics"),
        "TCMX" => String::from("CPU Memory"),
        "TCSC" | "TCSc" | "TCSA" => String::from("PECI SA"),
        "TCXC" | "TCXc" => String::from("PECI CPU"),
        "TG0P" => String::from("GPU AMD Proximity"),
        "TG0D" | "TG1D" => String::from("GPU AMD Die"),
        "TG0H" | "TG1H" => String::from("GPU AMD Heatsink"),
        "TGDD" => String::from("GPU AMD Die digital"),
        "TGDF" => String::from("GPU Die analog"),
        "TGVP" => String::from("GPU VR"),
        "TH0F" => String::from("SSD Heatsink"),
        "TH0X" => String::from("SSD Controller"),
        "TH0a" => String::from("SSD NAND"),
        "TH0b" => String::from("SSD NAND 2"),
        "TH1a" => String::from("Drive 1 Raw A"),
        "TH1b" => String::from("Drive 1 Raw B"),
        "TM0P" => String::from("Mem Bank A1"),
        "TM1P" => String::from("Mem Bank A2"),
        "TM2P" => String::from("Mem Bank A3"),
        "TM3P" => String::from("Mem Bank A4"),
        "TM8P" => String::from("Mem Bank B1"),
        "TM9P" => String::from("Mem Bank B2"),
        "TM0S" => String::from("Mem Module A1"),
        "TM1S" => String::from("Mem Module A2"),
        "TM2S" => String::from("Mem Module A3"),
        "TM3S" => String::from("Mem Module A4"),
        "TM8S" => String::from("Mem Module B1"),
        "TM9S" => String::from("Mem Module B2"),
        "Tm0P" => String::from("Mainboard"),
        "Tm1P" => String::from("Mainboard Bottom"),
        "Th0H" => String::from("CPU Heatpipe"),
        "Th1H" => String::from("Right Fin Stack"),
        "Th2H" => String::from("Left Fin Stack"),
        "TN0D" => String::from("Northbridge Diode"),
        "TN0P" => String::from("Northbridge 1"),
        "TN1P" => String::from("Northbridge 2"),
        "TN0C" => String::from("MCH Diode"),
        "TN0H" => String::from("MCH Heatsink"),
        "TP0D" | "TPCD" => String::from("PCH Diode"),
        "TP0P" => String::from("PCH Proximity"),
        "Tp0P" => String::from("Powerboard"),
        "Tp0C" => String::from("Power Supply 1 Alt."),
        "Tp1P" => String::from("Power Supply 2"),
        "Tp1C" => String::from("Power Supply 2 Alt."),
        "Tp2P" => String::from("Power Supply 3"),
        "Tp3P" => String::from("Power Supply 4"),
        "Tp4P" => String::from("Power Supply 5"),
        "Tp5P" => String::from("Power Supply 6"),
        "TL0P" => String::from("LCD"),
        "TH0P" => String::from("HDD Bay 1"),
        "TH1P" => String::from("HDD Bay 2"),
        "TH2P" => String::from("HDD Bay 3"),
        "TH3P" => String::from("HDD Bay 4"),
        "TO0P" => String::from("Optical Drive"),
        "TS0C" => String::from("Expansion Slots"),
        "TTLD" => String::from("Thunderbolt L"),
        "TTRD" => String::from("Thunderbolt R"),
        "TW0P" => String::from("Airport"),
        "TaLC" => String::from("Audio L"),
        "TaRC" => String::from("Audio R"),
        "Ts0P" => String::from("Palmrest L"),
        "Ts0S" => String::from("Palmrest L skin"),
        "Ts1P" => String::from("Palmrest R"),
        "Ts1S" => String::from("Palmrest R skin"),
        "Ts2S" => String::from("Touchpad"),
        _ => format!("unknown ({key})"),
    }
}

fn input_to_base_path(input_path: &Path) -> Result<PathBuf> {
    let file_name = input_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| FanControlError::InvalidFanPath(input_path.to_path_buf()))?;
    let fan_name = file_name
        .strip_suffix("_input")
        .ok_or_else(|| FanControlError::InvalidFanPath(input_path.to_path_buf()))?;

    Ok(input_path.with_file_name(fan_name))
}

fn join_suffix(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("fan"));
    path.with_file_name(format!("{file_name}{suffix}"))
}

fn read_u32(path: &Path) -> Result<u32> {
    let contents = fs::read_to_string(path).map_err(|source| FanControlError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    contents
        .trim()
        .parse::<u32>()
        .map_err(|source| FanControlError::ParseInt {
            path: path.to_path_buf(),
            source,
        })
}

fn read_temperature(path: &Path) -> Result<u8> {
    let contents = fs::read_to_string(path).map_err(|source| FanControlError::Io { path: path.to_path_buf(), source })?;
    let raw = contents.trim().parse::<i32>().map_err(|source| FanControlError::ParseInt { path: path.to_path_buf(), source })?;
    if raw <= 0 { return Ok(0); }
    Ok((raw / 1000).clamp(0, u8::MAX as i32) as u8)
}

fn write_string(path: &Path, value: &str) -> Result<()> {
    fs::write(path, value).map_err(|source| FanControlError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_die_average_and_system_average_are_independent() {
        let sources = vec![
            source("TC0E", 80, TemperatureRole::CpuDie),
            source("TC0F", 90, TemperatureRole::CpuDie),
            source("TC1C", 95, TemperatureRole::CpuTelemetry),
            source("TCXC", 92, TemperatureRole::CpuTelemetry),
            source("TGDD", 100, TemperatureRole::Gpu),
            source("TPCD", 40, TemperatureRole::System),
        ];
        let snapshot = TemperatureSnapshot::from_cached(&sources);

        assert_eq!(snapshot.hottest_temp_c, Some(100));
        assert_eq!(snapshot.overall_hottest_temp_c, Some(100));
        assert_eq!(snapshot.cpu_temp_c, Some(85));
        assert_eq!(snapshot.gpu_temp_c, Some(100));
        assert_eq!(snapshot.system_temp_c, Some(70));
        assert_eq!(snapshot.system_sensor_count, 2);
    }

    #[test]
    fn identifies_cpu_temperature_roles() {
        assert_eq!(temperature_role("TC0E"), TemperatureRole::CpuDie);
        assert_eq!(temperature_role("TC0F"), TemperatureRole::CpuDie);
        assert_eq!(temperature_role("TC1C"), TemperatureRole::CpuTelemetry);
        assert_eq!(temperature_role("TC8C"), TemperatureRole::CpuTelemetry);
        assert_eq!(temperature_role("TCBC"), TemperatureRole::System);
        assert_eq!(temperature_role("TCXC"), TemperatureRole::CpuTelemetry);
        assert_eq!(temperature_role("TC0P"), TemperatureRole::System);
        assert_eq!(sensor_label("TCBC"), "unknown (TCBC)");
    }

    #[test]
    fn optional_curve_sensor_can_raise_but_not_replace_cpu_gpu_input() {
        let sources = vec![TemperatureSource {
            key: String::from("TPCD"),
            name: String::from("PCH Die"),
            path: PathBuf::new(),
            last_temp_c: Some(105),
            role: TemperatureRole::System,
        }];
        let mut snapshot = TemperatureSnapshot {
            hottest_temp_c: Some(90),
            hottest_sensor_name: Some(String::from("CPU 1 Die")),
            ..TemperatureSnapshot::default()
        };
        snapshot.include_curve_sensor(&sources, Some("TPCD"));
        assert_eq!(snapshot.hottest_temp_c, Some(105));
        assert_eq!(snapshot.hottest_sensor_name.as_deref(), Some("PCH Die"));

        let mut without_optional = TemperatureSnapshot {
            hottest_temp_c: Some(90),
            ..TemperatureSnapshot::default()
        };
        without_optional.include_curve_sensor(&sources, None);
        assert_eq!(without_optional.hottest_temp_c, Some(90));
    }

    fn source(key: &str, temp: u8, role: TemperatureRole) -> TemperatureSource {
        TemperatureSource {
            key: key.into(),
            name: sensor_label(key),
            path: PathBuf::new(),
            last_temp_c: Some(temp),
            role,
        }
    }
}
