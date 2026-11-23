// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Group,
    Pci,
    Platform,
    Virtual,
    Usb,
}

impl DeviceKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Group => "GROUP",
            Self::Pci => "PCI",
            Self::Platform => "PLATFORM",
            Self::Virtual => "VIRTUAL",
            Self::Usb => "USB",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceNode {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub sysfs_path: PathBuf,
    pub parent: Option<String>,
    pub children: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceTree {
    pub nodes: BTreeMap<String, DeviceNode>,
    pub roots: Vec<String>,
}

impl DeviceTree {
    pub fn insert(&mut self, node: DeviceNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn connect(&mut self) {
        for node in self.nodes.values_mut() {
            node.children.clear();
        }
        let links: Vec<_> = self
            .nodes
            .values()
            .filter_map(|node| {
                node.parent
                    .as_ref()
                    .map(|parent| (parent.clone(), node.id.clone()))
            })
            .collect();
        for (parent, child) in links {
            if let Some(node) = self.nodes.get_mut(&parent) {
                node.children.push(child);
            }
        }
        for node in self.nodes.values_mut() {
            node.children.sort();
        }
        self.roots = self
            .nodes
            .values()
            .filter(|node| node.parent.is_none())
            .map(|node| node.id.clone())
            .collect();
        self.roots.sort();
    }
}

#[derive(Clone, Debug)]
pub struct Property {
    pub section: &'static str,
    pub name: &'static str,
    pub value: String,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct PciPmCapabilities {
    pub d1: bool,
    pub d2: bool,
    pub d3hot: bool,
    pub pme_d0: bool,
    pub pme_d1: bool,
    pub pme_d2: bool,
    pub pme_d3hot: bool,
    pub pme_d3cold: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Normal,
    Informational,
    Notice,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Assessment {
    pub severity: Severity,
    pub message: String,
}
