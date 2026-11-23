// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! scripts/fedora/update-map: which sources each step and app is built from.

use std::path::{Path, PathBuf};

pub const MAP_PATH: &str = "scripts/fedora/update-map";
pub const CORE: &str = "core";
const APP_PREFIX: &str = "app:";

pub struct Entry {
    pub name: String,
    /// Optional part, it counts only where this path exists.
    pub installed_if: Option<String>,
    pub prefixes: Vec<String>,
}

impl Entry {
    pub fn app(&self) -> Option<&str> {
        self.name.strip_prefix(APP_PREFIX)
    }

    pub fn is_core(&self) -> bool {
        self.name == CORE
    }

    pub fn label(&self) -> &str {
        self.app()
            .unwrap_or_else(|| self.name.strip_suffix(".sh").unwrap_or(&self.name))
    }

    pub fn matches(&self, path: &str) -> bool {
        self.prefixes.iter().any(|prefix| {
            path == prefix || (prefix.ends_with('/') && path.starts_with(prefix.as_str()))
        })
    }

    pub fn is_installed(&self, home: &Path) -> bool {
        let Some(path) = &self.installed_if else {
            return true;
        };
        match path.strip_prefix("~/") {
            Some(rest) => home.join(rest).exists(),
            None => Path::new(path).exists(),
        }
    }
}

pub fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?.to_owned();
            let mut installed_if = None;
            let mut prefixes = Vec::new();
            for field in fields {
                match field.strip_prefix("if=") {
                    Some(path) => installed_if = Some(path.to_owned()),
                    None => prefixes.push(field.to_owned()),
                }
            }
            // Installer scripts also depend on themselves.
            if !name.starts_with(APP_PREFIX) && name.ends_with(".sh") {
                prefixes.push(format!("scripts/fedora/{name}"));
            }
            Some(Entry { name, installed_if, prefixes })
        })
        .collect()
}

/// Where the installer records the commit each entry was installed from.
pub fn applied_dir() -> PathBuf {
    std::env::var_os("KAIT2EN_APPLIED_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/kait2en-installer/applied"))
}

pub fn applied_commit(dir: &Path, name: &str) -> Option<String> {
    let commit = std::fs::read_to_string(dir.join(name)).ok()?;
    let commit = commit.trim();
    (!commit.is_empty()).then(|| commit.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = "\
# comment
core            scripts/fedora/lib.sh
install-dsp.sh  dsp/
app:t2-touchbar if=/nonexistent/kait2en-touchbar apps/t2-touchbar/
install-vornehm.sh if=~/.local/share/x assets/vornehm/
rebuild-initramfs.sh
";

    #[test]
    fn parses_names_conditions_and_own_script() {
        let entries = parse(MAP);
        assert_eq!(entries.len(), 5);
        assert!(entries[0].is_core());
        assert_eq!(entries[1].prefixes, ["dsp/", "scripts/fedora/install-dsp.sh"]);
        assert_eq!(entries[2].app(), Some("t2-touchbar"));
        assert_eq!(entries[2].prefixes, ["apps/t2-touchbar/"]);
        assert_eq!(entries[4].prefixes, ["scripts/fedora/rebuild-initramfs.sh"]);
    }

    #[test]
    fn matches_directories_and_files() {
        let entries = parse(MAP);
        assert!(entries[1].matches("dsp/graphs/a.json"));
        assert!(entries[1].matches("scripts/fedora/install-dsp.sh"));
        assert!(!entries[1].matches("dspx/a"));
        assert!(entries[0].matches("scripts/fedora/lib.sh"));
        assert!(!entries[0].matches("scripts/fedora/lib.sh.orig"));
    }

    #[test]
    fn optional_parts_need_their_path() {
        let entries = parse(MAP);
        assert!(!entries[2].is_installed(Path::new("/")));
        assert!(entries[1].is_installed(Path::new("/")));
        assert!(!entries[3].is_installed(Path::new("/nonexistent")));
    }
}
