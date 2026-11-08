//! Power limit steps the SMC publishes

use std::path::Path;

use crate::{
    error::{Result, ZonePowerError},
    sysfs::read_string,
};

/// One channel. Bits 0 to 6 hold the step, bit 7 flags a hard limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub step: u8,
    pub hard: bool,
}

impl Step {
    fn from_byte(byte: u8) -> Self {
        Self { step: byte & 0x7f, hard: byte & 0x80 != 0 }
    }
}

/// Data version 3 layout of the port window at +0x10 and +0x14
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PLimits {
    pub cpu: Step,
    pub igpu: Step,
    pub memory: Step,
    pub idle: Step,
    pub dgpu: Step,
}

impl PLimits {
    /// Parses `smc_plimit_raw`, for example `2e001627 0f`
    pub fn read(t2smc: &Path) -> Result<Self> {
        let path = t2smc.join("smc_plimit_raw");
        let raw = read_string(&path)?;
        let parse_error = || ZonePowerError::Parse { path: path.clone(), value: raw.clone() };
        let (word, gpu) = raw.split_once(' ').ok_or_else(parse_error)?;
        let word = u32::from_str_radix(word, 16).map_err(|_| parse_error())?;
        let gpu = u8::from_str_radix(gpu, 16).map_err(|_| parse_error())?;
        let [cpu, igpu, memory, idle] = word.to_le_bytes();
        Ok(Self {
            cpu: Step::from_byte(cpu),
            igpu: Step::from_byte(igpu),
            memory: Step::from_byte(memory),
            idle: Step::from_byte(idle),
            dgpu: Step::from_byte(gpu),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_version_3_word() {
        let [cpu, igpu, memory, idle] = 0x2e00_1627u32.to_le_bytes();
        assert_eq!((cpu, igpu, memory, idle), (0x27, 0x16, 0x00, 0x2e));
        assert_eq!(Step::from_byte(0x8f), Step { step: 0x0f, hard: true });
    }
}
