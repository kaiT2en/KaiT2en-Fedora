use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{
    Assessment, DeviceKind, DeviceNode, DeviceTree, PciPmCapabilities, Property, Severity,
};

type PciNames = BTreeMap<(String, String), String>;

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn canonical(path: impl AsRef<Path>) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

fn driver_name(path: &Path) -> Option<String> {
    canonical(path.join("driver"))?
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn pci_names() -> PciNames {
    let Some(database) = read("/usr/share/hwdata/pci.ids") else {
        return BTreeMap::new();
    };
    let mut names = BTreeMap::new();
    let mut vendor = String::new();
    for line in database.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            let Some((id, _name)) = line.split_once("  ") else {
                continue;
            };
            if id.len() == 4 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                vendor = id.to_ascii_lowercase();
            }
            continue;
        }
        if line.starts_with("\t\t") || vendor.is_empty() {
            continue;
        }
        let line = line.trim_start_matches('\t');
        let Some((device, name)) = line.split_once("  ") else {
            continue;
        };
        if device.len() == 4 && device.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            names.insert(
                (vendor.clone(), device.to_ascii_lowercase()),
                name.trim().to_owned(),
            );
        }
    }
    names
}

fn pci_name(path: &Path, names: &PciNames) -> String {
    let class = read(path.join("class")).unwrap_or_default();
    let vendor = read(path.join("vendor")).unwrap_or_default();
    let device = read(path.join("device")).unwrap_or_default();
    let vendor_id = vendor.trim_start_matches("0x").to_ascii_lowercase();
    let device_id = device.trim_start_matches("0x").to_ascii_lowercase();
    if let Some(name) = names.get(&(vendor_id, device_id)) {
        return name.clone();
    }
    let class_name = match class.get(2..4) {
        Some("01") => "Storage controller",
        Some("02") => "Network controller",
        Some("03") => "Display controller",
        Some("04") => "Multimedia controller",
        Some("06") => "Bridge",
        Some("0c") => "Serial bus controller",
        _ => "PCI device",
    };
    format!(
        "{class_name}  {}:{}",
        vendor.trim_start_matches("0x"),
        device.trim_start_matches("0x")
    )
}

fn nearest_parent(path: &Path, candidates: &BTreeMap<PathBuf, String>) -> Option<String> {
    path.ancestors()
        .skip(1)
        .find_map(|ancestor| candidates.get(ancestor).cloned())
}

pub fn collect() -> DeviceTree {
    let mut tree = DeviceTree::default();
    let mut paths = BTreeMap::<PathBuf, String>::new();
    let pci_names = pci_names();

    tree.insert(DeviceNode {
        id: "group:pci".into(),
        name: "PCI domain 0000".into(),
        kind: DeviceKind::Group,
        sysfs_path: PathBuf::new(),
        parent: None,
        children: Vec::new(),
    });

    if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
        let mut devices: Vec<_> = entries.flatten().collect();
        devices.sort_by_key(|entry| entry.file_name());
        for entry in devices {
            let id = entry.file_name().to_string_lossy().into_owned();
            let Some(path) = canonical(entry.path()) else {
                continue;
            };
            paths.insert(path.clone(), id.clone());
            tree.insert(DeviceNode {
                id,
                name: pci_name(&path, &pci_names),
                kind: DeviceKind::Pci,
                sysfs_path: path,
                parent: None,
                children: Vec::new(),
            });
        }
    }

    let pci_ids: Vec<_> = tree
        .nodes
        .values()
        .filter(|node| node.kind == DeviceKind::Pci)
        .map(|node| node.id.clone())
        .collect();
    for id in pci_ids {
        let node = &tree.nodes[&id];
        let parent = nearest_parent(&node.sysfs_path, &paths).unwrap_or_else(|| "group:pci".into());
        tree.nodes.get_mut(&id).unwrap().parent = Some(parent);
    }

    tree.insert(DeviceNode {
        id: "group:platform".into(),
        name: "Platform devices".into(),
        kind: DeviceKind::Group,
        sysfs_path: PathBuf::new(),
        parent: None,
        children: Vec::new(),
    });

    if let Ok(entries) = fs::read_dir("/sys/bus/platform/devices") {
        let mut devices: Vec<_> = entries.flatten().collect();
        devices.sort_by_key(|entry| entry.file_name());
        for entry in devices {
            let bus_id = entry.file_name().to_string_lossy().into_owned();
            let Some(path) = canonical(entry.path()) else {
                continue;
            };
            let id = format!("platform:{bus_id}");
            let name = driver_name(&path).unwrap_or_else(|| bus_id.clone());
            paths.insert(path.clone(), id.clone());
            tree.insert(DeviceNode {
                id,
                name,
                kind: DeviceKind::Platform,
                sysfs_path: path,
                parent: None,
                children: Vec::new(),
            });
        }
    }

    let platform_ids: Vec<_> = tree
        .nodes
        .values()
        .filter(|node| node.kind == DeviceKind::Platform)
        .map(|node| node.id.clone())
        .collect();
    for id in platform_ids {
        let node = &tree.nodes[&id];
        let parent =
            nearest_parent(&node.sysfs_path, &paths).unwrap_or_else(|| "group:platform".into());
        tree.nodes.get_mut(&id).unwrap().parent = Some(parent);
    }

    if let Ok(entries) = fs::read_dir("/sys/bus/auxiliary/devices") {
        let mut devices: Vec<_> = entries
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("apple_t2_bridge.vhci.")
            })
            .collect();
        devices.sort_by_key(|entry| entry.file_name());
        for entry in devices {
            let bus_id = entry.file_name().to_string_lossy().into_owned();
            let Some(path) = canonical(entry.path()) else {
                continue;
            };
            let id = format!("virtual:{bus_id}");
            let parent = nearest_parent(&path, &paths).or_else(|| Some("group:platform".into()));
            paths.insert(path.clone(), id.clone());
            tree.insert(DeviceNode {
                id,
                name: "BCE virtual USB host controller".into(),
                kind: DeviceKind::Virtual,
                sysfs_path: path,
                parent,
                children: Vec::new(),
            });
        }
    }

    if let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") {
        let mut devices: Vec<_> = entries.flatten().collect();
        devices.sort_by_key(|entry| entry.file_name());
        for entry in devices {
            let Some(path) = canonical(entry.path()) else {
                continue;
            };
            if !path.join("product").is_file() && !path.join("idVendor").is_file() {
                continue;
            }
            let bus_id = entry.file_name().to_string_lossy().into_owned();
            let id = format!("usb:{bus_id}");
            let name = read(path.join("product")).unwrap_or_else(|| "USB device".into());
            paths.insert(path.clone(), id.clone());
            tree.insert(DeviceNode {
                id,
                name,
                kind: DeviceKind::Usb,
                sysfs_path: path,
                parent: None,
                children: Vec::new(),
            });
        }
    }

    let usb_ids: Vec<_> = tree
        .nodes
        .values()
        .filter(|node| node.kind == DeviceKind::Usb)
        .map(|node| node.id.clone())
        .collect();
    for id in usb_ids {
        let node = &tree.nodes[&id];
        let parent = nearest_parent(&node.sysfs_path, &paths).unwrap_or_else(|| "group:pci".into());
        tree.nodes.get_mut(&id).unwrap().parent = Some(parent);
    }

    tree.connect();
    tree
}

fn property(section: &'static str, name: &'static str, path: PathBuf) -> Option<Property> {
    read(&path).map(|value| Property {
        section,
        name,
        value,
        path: Some(path),
    })
}

fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.into()
}

fn exposed(path: &Path) -> String {
    read(path).unwrap_or_else(|| "not exposed".into())
}

pub fn runtime_status(node: &DeviceNode) -> Option<String> {
    read(node.sysfs_path.join("power/runtime_status"))
}

pub fn pci_power_state(node: &DeviceNode) -> Option<String> {
    read(node.sysfs_path.join("power_state"))
}

pub fn assessment(node: &DeviceNode) -> Assessment {
    if node.kind == DeviceKind::Group {
        return Assessment {
            severity: Severity::Normal,
            message: "no immediate runtime PM anomaly".into(),
        };
    }

    let path = &node.sysfs_path;
    let control = exposed(&path.join("power/control"));
    let status = exposed(&path.join("power/runtime_status"));
    let error = exposed(&path.join("power/runtime_error"));
    if error != "not exposed" && error != "0" {
        Assessment {
            severity: Severity::Warning,
            message: format!("runtime PM failed ({error})"),
        }
    } else if control == "auto" && status == "active" && node.children.is_empty() {
        Assessment {
            severity: Severity::Notice,
            message: "active leaf device under automatic runtime PM".into(),
        }
    } else if control == "on" && !node.children.is_empty() {
        Assessment {
            severity: Severity::Informational,
            message: "runtime PM disabled on a parent or supplier".into(),
        }
    } else if control == "on" {
        Assessment {
            severity: Severity::Informational,
            message: "runtime PM disabled by policy".into(),
        }
    } else {
        Assessment {
            severity: Severity::Normal,
            message: "no immediate runtime PM anomaly".into(),
        }
    }
}

pub fn properties(
    node: &DeviceNode,
    display_lanes: Option<&str>,
    pci_pm: Option<&PciPmCapabilities>,
) -> Vec<Property> {
    if node.kind == DeviceKind::Group {
        return vec![Property {
            section: "Topology",
            name: "Children",
            value: node.children.len().to_string(),
            path: None,
        }];
    }

    let path = &node.sysfs_path;
    let runtime_control = exposed(&path.join("power/control"));
    let runtime_status = exposed(&path.join("power/runtime_status"));
    let runtime_error = exposed(&path.join("power/runtime_error"));
    let mut values = vec![
        Property {
            section: "Identity",
            name: "Bus",
            value: node.kind.label().into(),
            path: None,
        },
        Property {
            section: "Identity",
            name: "Kernel ID",
            value: node.id.trim_start_matches("usb:").into(),
            path: None,
        },
        Property {
            section: "Identity",
            name: "Parent",
            value: node.parent.clone().unwrap_or_else(|| "none".into()),
            path: None,
        },
        Property {
            section: "Identity",
            name: "Children",
            value: node.children.len().to_string(),
            path: None,
        },
        Property {
            section: "Identity",
            name: "Sysfs path",
            value: path.display().to_string(),
            path: None,
        },
        Property {
            section: "Kernel",
            name: "Driver",
            value: driver_name(path).unwrap_or_else(|| "unbound".into()),
            path: Some(path.join("driver")),
        },
    ];

    values.extend([
        Property {
            section: "Runtime PM",
            name: "Policy",
            value: runtime_control.clone(),
            path: Some(path.join("power/control")),
        },
        Property {
            section: "Runtime PM",
            name: "Status",
            value: runtime_status.clone(),
            path: Some(path.join("power/runtime_status")),
        },
        Property {
            section: "Runtime PM",
            name: "Error",
            value: runtime_error.clone(),
            path: Some(path.join("power/runtime_error")),
        },
    ]);

    let assessment = assessment(node);
    values.push(Property {
        section: "Assessment",
        name: "Power state",
        value: format!("{:?}: {}", assessment.severity, assessment.message).to_lowercase(),
        path: None,
    });

    if node.kind == DeviceKind::Pci {
        values.extend([
            Property {
                section: "PCI power states",
                name: "Current state",
                value: exposed(&path.join("power_state")),
                path: Some(path.join("power_state")),
            },
            Property {
                section: "PCI power states",
                name: "D1 supported",
                value: pci_pm
                    .map(|pm| yes_no(pm.d1))
                    .unwrap_or_else(|| "not exposed".into()),
                path: Some(path.join("config")),
            },
            Property {
                section: "PCI power states",
                name: "D2 supported",
                value: pci_pm
                    .map(|pm| yes_no(pm.d2))
                    .unwrap_or_else(|| "not exposed".into()),
                path: Some(path.join("config")),
            },
            Property {
                section: "PCI power states",
                name: "D3hot supported",
                value: pci_pm
                    .map(|pm| yes_no(pm.d3hot))
                    .unwrap_or_else(|| "not exposed".into()),
                path: Some(path.join("config")),
            },
            Property {
                section: "PCI power states",
                name: "D3cold allowed",
                value: exposed(&path.join("d3cold_allowed")),
                path: Some(path.join("d3cold_allowed")),
            },
        ]);
        if let Some(pm) = pci_pm {
            values.extend(
                [
                    ("PME from D0", pm.pme_d0),
                    ("PME from D1", pm.pme_d1),
                    ("PME from D2", pm.pme_d2),
                    ("PME from D3hot", pm.pme_d3hot),
                    ("PME from D3cold", pm.pme_d3cold),
                ]
                .map(|(name, supported)| Property {
                    section: "PCI wake support",
                    name,
                    value: yes_no(supported),
                    path: Some(path.join("config")),
                }),
            );
        }
    }

    let attributes = [
        ("Power", "Wakeup", "power/wakeup"),
        ("Power", "Autosuspend delay", "power/autosuspend_delay_ms"),
        ("Power", "Active time", "power/runtime_active_time"),
        ("Power", "Suspended time", "power/runtime_suspended_time"),
        ("Power", "Runtime usage", "power/runtime_usage"),
        ("Power", "Wakeup count", "power/wakeup_count"),
        ("Power", "Wakeup active count", "power/wakeup_active_count"),
        ("Power", "Wakeup abort count", "power/wakeup_abort_count"),
        ("PCI", "Vendor", "vendor"),
        ("PCI", "Device", "device"),
        ("PCI", "Class", "class"),
        ("PCI", "Current link speed", "current_link_speed"),
        ("PCI", "Current link width", "current_link_width"),
        ("PCI", "Maximum link speed", "max_link_speed"),
        ("PCI", "Maximum link width", "max_link_width"),
        ("USB", "Vendor ID", "idVendor"),
        ("USB", "Product ID", "idProduct"),
        ("USB", "USB version", "version"),
        ("USB", "Device class", "bDeviceClass"),
    ];
    values.extend(
        attributes
            .into_iter()
            .filter_map(|(section, name, relative)| property(section, name, path.join(relative))),
    );
    if let Some(lanes) = display_lanes {
        values.push(Property {
            section: "Display",
            name: "Active DDI/eDP width",
            value: lanes.into(),
            path: Some(PathBuf::from(format!(
                "/sys/kernel/debug/dri/{}/i915_display_info",
                node.id
            ))),
        });
    }
    if let Some(group) = canonical(path.join("iommu_group")).and_then(|group| {
        group
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }) {
        values.push(Property {
            section: "Kernel",
            name: "IOMMU group",
            value: group,
            path: Some(path.join("iommu_group")),
        });
    }
    if let Ok(entries) = fs::read_dir(path) {
        let mut links: Vec<_> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("supplier:") || name.starts_with("consumer:") {
                    Some((name, entry.path()))
                } else {
                    None
                }
            })
            .collect();
        links.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, link) in links {
            let (relationship, target) = name.split_once(':').unwrap_or(("link", &name));
            values.push(Property {
                section: "Device links",
                name: if relationship == "supplier" {
                    "Supplier"
                } else {
                    "Consumer"
                },
                value: target.into(),
                path: canonical(&link).or(Some(link)),
            });
        }
    }
    values
}

pub fn runtime_summary(node: &DeviceNode) -> String {
    if node.kind == DeviceKind::Group {
        return format!("{} devices", node.children.len());
    }
    let runtime =
        read(node.sysfs_path.join("power/runtime_status")).unwrap_or_else(|| "unsupported".into());
    let pci = read(node.sysfs_path.join("power_state"));
    match pci {
        Some(state) => format!("{runtime}  {state}"),
        None => runtime,
    }
}
