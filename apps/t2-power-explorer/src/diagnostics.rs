// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::collections::BTreeMap;
use std::process::Command;

use crate::model::PciPmCapabilities;

const STATUS_HELPER: &str = "/usr/local/libexec/t2-power-explorer-status";

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub display_lanes: BTreeMap<String, String>,
    pub pci_pm: BTreeMap<String, PciPmCapabilities>,
}

fn parse_flag(value: &str) -> Option<bool> {
    match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

pub fn read() -> Diagnostics {
    let Ok(output) = Command::new("pkexec").arg(STATUS_HELPER).output() else {
        return Diagnostics::default();
    };
    if !output.status.success() {
        return Diagnostics::default();
    }

    let mut diagnostics = Diagnostics::default();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["DISPLAY_LANES", device, lanes]
                if device.starts_with("0000:") && lanes.parse::<u8>().is_ok() =>
            {
                diagnostics
                    .display_lanes
                    .insert((*device).into(), format!("{lanes} lanes"));
            }
            [
                "PCI_PM",
                device,
                d1,
                d2,
                d3hot,
                pme_d0,
                pme_d1,
                pme_d2,
                pme_d3hot,
                pme_d3cold,
            ] => {
                let flags = [d1, d2, d3hot, pme_d0, pme_d1, pme_d2, pme_d3hot, pme_d3cold]
                    .map(|value| parse_flag(value));
                if let [
                    Some(d1),
                    Some(d2),
                    Some(d3hot),
                    Some(pme_d0),
                    Some(pme_d1),
                    Some(pme_d2),
                    Some(pme_d3hot),
                    Some(pme_d3cold),
                ] = flags
                {
                    diagnostics.pci_pm.insert(
                        (*device).into(),
                        PciPmCapabilities {
                            d1,
                            d2,
                            d3hot,
                            pme_d0,
                            pme_d1,
                            pme_d2,
                            pme_d3hot,
                            pme_d3cold,
                        },
                    );
                }
            }
            _ => {}
        }
    }
    diagnostics
}
