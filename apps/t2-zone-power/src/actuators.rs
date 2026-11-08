//! Linux counterparts of the macOS power limit listeners

use std::{
    cell::Cell,
    io,
    path::{Path, PathBuf},
};

use crate::{
    error::{Result, ZonePowerError},
    sysfs::{glob_paths, read_string, read_value, write_value},
};

/// macOS caps XCPM at the CPUPStates entry the CPU step indexes.
/// CPUPStates is the XCPM ratio ladder from the highest turbo frequency down
/// to the lowest one in 100 MHz steps (ioreg of a MacBookPro16,1 under
/// macOS: 5000 down to 800 MHz, 43 entries). Step 0 leaves the CPU uncapped.
pub struct CpuCap {
    policies: Vec<PathBuf>,
    min_khz: u32,
    max_khz: u32,
}

const CPU_RATIO_KHZ: u32 = 100_000;

impl CpuCap {
    pub fn discover() -> Result<Option<Self>> {
        let policies = glob_paths("/sys/devices/system/cpu/cpufreq/policy*")?;
        let Some(first) = policies.first() else { return Ok(None) };
        Ok(Some(Self {
            min_khz: read_value(&first.join("cpuinfo_min_freq"))?,
            max_khz: read_value(&first.join("cpuinfo_max_freq"))?,
            policies,
        }))
    }

    pub fn target_khz(&self, step: u8) -> u32 {
        self.max_khz
            .saturating_sub(u32::from(step) * CPU_RATIO_KHZ)
            .max(self.min_khz)
    }

    pub fn apply(&self, khz: u32) -> Result<()> {
        for policy in &self.policies {
            let path = policy.join("scaling_max_freq");
            if read_value::<u32>(&path).ok() != Some(khz) {
                match write_value(&path, khz) {
                    // A policy is busy while CPU hotplug rebuilds it on
                    // resume, the next tick writes it again
                    Err(ZonePowerError::Io { source, .. })
                        if source.kind() == io::ErrorKind::ResourceBusy => {}
                    result => result?,
                }
            }
        }
        Ok(())
    }

    pub fn restore(&self) -> Result<()> {
        self.apply(self.max_khz)
    }
}

/// CPUFloor of the macOS board plists (macOS 15.4.1). Macmini8,1
/// has none, so macOS reports no CPU floor there.
const CPU_FLOOR_MHZ: &[(&str, u32)] = &[
    ("MacBookAir8,", 800),
    ("MacBookAir9,", 800),
    ("MacBookPro15,", 1000),
    ("MacBookPro16,", 1000),
    ("iMacPro1,", 1200),
    ("MacPro7,", 1200),
    ("iMac20,", 1300),
];

/// MSTS and MSPC as macOS computes them from the
/// XCPM P-state table: the turbo P-state count and the non-turbo P-states
/// above CPUFloor
pub fn cpu_floor() -> Option<(u8, u8)> {
    let product = read_string(Path::new("/sys/class/dmi/id/product_name")).ok()?;
    let floor = CPU_FLOOR_MHZ.iter().find(|(prefix, _)| product.starts_with(prefix))?.1;
    let policy = Path::new("/sys/devices/system/cpu/cpufreq/policy0");
    let mhz = |file: &str| read_value::<u32>(&policy.join(file)).ok().map(|khz| khz / 1000);
    floor_states(mhz("cpuinfo_min_freq")?, mhz("cpuinfo_max_freq")?, mhz("base_frequency")?, floor)
}

fn floor_states(min: u32, max_turbo: u32, max_non_turbo: u32, floor: u32) -> Option<(u8, u8)> {
    let states = (max_turbo - min) / 100 + 1;
    let turbo = (max_turbo - max_non_turbo) / 100;
    let non_turbo = states - turbo;
    if floor < min {
        return None;
    }
    let ps_delta = ((floor - min) / 100) as u8;
    match non_turbo {
        0 => None,
        1 => Some(((turbo as u8).wrapping_sub(ps_delta), 1)),
        _ if non_turbo > u32::from(ps_delta) => Some((turbo as u8, (non_turbo - u32::from(ps_delta)) as u8)),
        _ => Some((((non_turbo + turbo) as u8).wrapping_sub(ps_delta).wrapping_sub(1), 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::floor_states;

    #[test]
    fn floor_states_like_set_cpu_floor() {
        // MacBookPro15,1 with i7-9750H
        assert_eq!(floor_states(800, 4500, 2600, 1000), Some((19, 17)));
        // iMac20,x from a macOS log: 41 states, 15 turbo, 26 non-turbo
        assert_eq!(floor_states(800, 4800, 3300, 1300), Some((15, 21)));
        assert_eq!(floor_states(800, 4500, 2600, 700), None);
    }
}

/// macOS turns the idle step into a run ratio of 100 - step
/// percent, at least 30 %. intel_powerclamp takes the idle share.
pub struct ForcedIdle {
    cur_state: PathBuf,
    max_state: u32,
}

impl ForcedIdle {
    pub fn discover() -> Result<Option<Self>> {
        for device in glob_paths("/sys/class/thermal/cooling_device*")? {
            if read_string(&device.join("type")).is_ok_and(|kind| kind == "intel_powerclamp") {
                return Ok(Some(Self {
                    max_state: read_value(&device.join("max_state"))?,
                    cur_state: device.join("cur_state"),
                }));
            }
        }
        Ok(None)
    }

    pub fn target_percent(&self, step: u8) -> u32 {
        if step == 0 {
            return 0;
        }
        let run = 100u32.saturating_sub(u32::from(step)).max(30);
        (100 - run).min(self.max_state)
    }

    pub fn apply(&self, percent: u32) -> Result<()> {
        if read_value::<u32>(&self.cur_state).ok() != Some(percent) {
            write_value(&self.cur_state, percent)?;
        }
        Ok(())
    }

    pub fn restore(&self) -> Result<()> {
        self.apply(0)
    }
}

/// The dGPU step is an index into the PowerTune table of the AMD driver.
/// Both macOS and Linux fill it linearly from the TDP (step 0) down to a
/// floor in 16 steps.
pub struct DGpuCap {
    device: PathBuf,
    hwmon: PathBuf,
    tdp_w: u32,
    floor_w: u32,
    /// Every hwmon access restarts the autosuspend timer of amdgpu, so the
    /// cap is only touched when it changes or the GPU woke up
    applied_w: Cell<Option<u32>>,
}

/// macOS drives the dGPU through power states 0 to 15 on every Mac with one
pub const DGPU_STEPS: u32 = 16;

impl DGpuCap {
    pub fn discover() -> Result<Option<Self>> {
        for hwmon in glob_paths("/sys/class/drm/card*/device/hwmon/hwmon*")? {
            if !read_string(&hwmon.join("name")).is_ok_and(|name| name == "amdgpu") {
                continue;
            }
            let device = hwmon.join("device");
            let Some(floor_ratio) = floor_ratio(&device) else { continue };
            // Reads fail with EBUSY while the GPU is runtime suspended
            let Ok(tdp_uw) = read_value::<u64>(&hwmon.join("power1_cap_default")) else { continue };
            let tdp_w = u32::try_from(tdp_uw / 1_000_000).unwrap_or(u32::MAX);
            let floor_w = (f64::from(tdp_w) * floor_ratio).ceil() as u32;
            return Ok(Some(Self { device, hwmon, tdp_w, floor_w, applied_w: Cell::new(None) }));
        }
        Ok(None)
    }

    /// generatePowerTuneTable and populatePowerTuneTable
    pub fn target_w(&self, step: u8) -> u32 {
        let index = u32::from(step).min(DGPU_STEPS - 1);
        let span = f64::from(self.tdp_w - self.floor_w);
        let above_floor = span * f64::from(DGPU_STEPS - 1 - index) / f64::from(DGPU_STEPS - 1);
        self.floor_w + above_floor as u32
    }

    /// Leaves a runtime suspended GPU alone. The cap is applied once it wakes.
    pub fn apply(&self, watts: u32) -> Result<()> {
        if !self.is_active() {
            self.applied_w.set(None);
            return Ok(());
        }
        if self.applied_w.get() == Some(watts) {
            return Ok(());
        }
        let min: u64 = read_value(&self.hwmon.join("power1_cap_min"))?;
        let max: u64 = read_value(&self.hwmon.join("power1_cap_max"))?;
        let microwatts = (u64::from(watts) * 1_000_000).clamp(min, max);
        let path = self.hwmon.join("power1_cap");
        if read_value::<u64>(&path).ok() != Some(microwatts) {
            write_value(&path, microwatts)?;
        }
        self.applied_w.set(Some(watts));
        Ok(())
    }

    pub fn restore(&self) -> Result<()> {
        self.apply(self.tdp_w)
    }

    pub fn tdp_w(&self) -> u32 {
        self.tdp_w
    }

    pub fn is_active(&self) -> bool {
        read_string(&self.device.join("power/runtime_status")).is_ok_and(|status| status == "active")
    }
}

/// Floor of the PowerTune table in parts of the TDP, as on macOS
fn floor_ratio(device: &Path) -> Option<f64> {
    let id = read_string(&device.join("device")).ok()?;
    let id = u16::from_str_radix(id.trim_start_matches("0x"), 16).ok()?;
    match id {
        // Navi 1x: ceil(TDP * 0.4)
        0x7310..=0x73ff => Some(0.4),
        // Polaris: down to 0 W
        0x67c0..=0x67ff | 0x6980..=0x699f => Some(0.0),
        _ => None,
    }
}

/// The iGPU step indexes the P-state table, which runs from RP0 down to RPn
/// one ratio per state.
/// A Gen9 ratio is 50 MHz.
pub struct IGpuCap {
    max_freq: PathBuf,
    rp0_mhz: u32,
    rpn_mhz: u32,
}

const IGPU_RATIO_MHZ: u32 = 50;

impl IGpuCap {
    pub fn discover() -> Result<Option<Self>> {
        for rp0 in glob_paths("/sys/class/drm/card*/gt_RP0_freq_mhz")? {
            let Some(card) = rp0.parent() else { continue };
            return Ok(Some(Self {
                rp0_mhz: read_value(&rp0)?,
                rpn_mhz: read_value(&card.join("gt_RPn_freq_mhz"))?,
                max_freq: card.join("gt_max_freq_mhz"),
            }));
        }
        Ok(None)
    }

    /// Number of P-states macOS reports to the SMC
    pub fn steps(&self) -> u32 {
        (self.rp0_mhz - self.rpn_mhz) / IGPU_RATIO_MHZ + 1
    }

    pub fn target_mhz(&self, step: u8) -> u32 {
        let index = u32::from(step).min(self.steps() - 1);
        (self.rp0_mhz - index * IGPU_RATIO_MHZ).max(self.rpn_mhz)
    }

    pub fn apply(&self, mhz: u32) -> Result<()> {
        if read_value::<u32>(&self.max_freq).ok() != Some(mhz) {
            write_value(&self.max_freq, mhz)?;
        }
        Ok(())
    }

    pub fn restore(&self) -> Result<()> {
        self.apply(self.rp0_mhz)
    }
}
