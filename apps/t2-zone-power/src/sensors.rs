// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! Read-only view of the SMC zones and the limits the daemon applied

use std::{
    cell::OnceCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    actuators::DGpuCap,
    plimit::PLimits,
    sysfs::{glob_paths, read_string, read_value},
};

/// t2smc labels its hwmon channels with the raw SMC key
pub struct SmcSensors {
    t2smc: PathBuf,
    inputs: HashMap<String, PathBuf>,
    dgpu: OnceCell<DGpuCap>,
}

impl SmcSensors {
    pub fn discover(t2smc: &Path) -> Self {
        let mut inputs = HashMap::new();
        for label in glob_paths(&format!("{}/*_label", t2smc.display())).unwrap_or_default() {
            let (Ok(key), Some(name)) = (read_string(&label), label.file_name()) else { continue };
            let input = name.to_string_lossy().replace("_label", "_input");
            inputs.insert(key, t2smc.join(input));
        }
        Self { t2smc: t2smc.to_owned(), inputs, dgpu: OnceCell::new() }
    }

    fn raw(&self, key: &str) -> Option<f64> {
        read_value::<i64>(self.inputs.get(key)?).ok().map(|value| value as f64)
    }

    /// Power keys and the zone throttle keys share the micro unit
    pub fn micro(&self, key: &str) -> Option<f64> {
        self.raw(key).map(|value| value / 1_000_000.0)
    }

    pub fn celsius(&self, key: &str) -> Option<f64> {
        self.raw(key).map(|value| value / 1000.0)
    }

    pub fn plimits(&self) -> Option<PLimits> {
        PLimits::read(&self.t2smc).ok()
    }

    /// Discovery only succeeds while the dGPU is awake and never wakes it
    pub fn dgpu(&self) -> Option<&DGpuCap> {
        if self.dgpu.get().is_none() {
            if let Ok(Some(dgpu)) = DGpuCap::discover() {
                let _ = self.dgpu.set(dgpu);
            }
        }
        self.dgpu.get()
    }
}

/// Limits as the daemon left them in sysfs
#[derive(Clone, Copy, Debug, Default)]
pub struct Applied {
    pub cpu_cap_mhz: Option<u32>,
    pub cpu_max_mhz: Option<u32>,
    pub idle_percent: Option<u32>,
    pub dgpu_cap_w: Option<u32>,
    pub dgpu_tdp_w: Option<u32>,
    pub igpu_cap_mhz: Option<u32>,
    pub igpu_max_mhz: Option<u32>,
}

impl Applied {
    /// Every hwmon access of amdgpu restarts its autosuspend timer, so the
    /// dGPU cap comes from the step like in the daemon instead of power1_cap
    pub fn read(dgpu: Option<&DGpuCap>, dgpu_step: Option<u8>) -> Self {
        let policy = Path::new("/sys/devices/system/cpu/cpufreq/policy0");
        let khz = |file: &str| read_value::<u32>(&policy.join(file)).ok().map(|khz| khz / 1000);
        Self {
            cpu_cap_mhz: khz("scaling_max_freq"),
            cpu_max_mhz: khz("cpuinfo_max_freq"),
            idle_percent: powerclamp_state(),
            dgpu_cap_w: dgpu.filter(|dgpu| dgpu.is_active()).zip(dgpu_step).map(|(dgpu, step)| dgpu.target_w(step)),
            dgpu_tdp_w: dgpu.map(DGpuCap::tdp_w),
            igpu_cap_mhz: i915_mhz("gt_max_freq_mhz"),
            igpu_max_mhz: i915_mhz("gt_RP0_freq_mhz"),
        }
    }
}

fn i915_mhz(file: &str) -> Option<u32> {
    let path = glob_paths(&format!("/sys/class/drm/card*/{file}")).ok()?.into_iter().next()?;
    read_value(&path).ok()
}

fn powerclamp_state() -> Option<u32> {
    glob_paths("/sys/class/thermal/cooling_device*")
        .ok()?
        .into_iter()
        .find(|device| read_string(&device.join("type")).is_ok_and(|kind| kind == "intel_powerclamp"))
        .and_then(|device| read_value(&device.join("cur_state")).ok())
}
