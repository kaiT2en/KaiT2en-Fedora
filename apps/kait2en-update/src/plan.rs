// SPDX-License-Identifier: GPL-3.0-or-later
//! Compares the installed state with GitHub and picks what has to run.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    git::{Commit, REMOTE_REF, Repository},
    map::{self, Entry},
};

/// Steps after which the new state is only active after a reboot.
const REBOOT_STEPS: &[&str] = &[
    "install-dkms-modules.sh",
    "install-gpu-runtime-pm.sh",
    "install-i915.sh",
    "install-wireless.sh",
    "install-acpi-fixes.sh",
    "install-plymouth-theme.sh",
    "install-kernel-args.sh",
    "rebuild-initramfs.sh",
];

/// The commit an entry was installed from.
#[derive(Clone, PartialEq, Debug)]
pub enum Base {
    Applied(String),
    /// No record yet, the checkout HEAD stands in for it.
    Estimated(String),
    /// The record names a commit this checkout does not have.
    Unknown,
}

#[derive(Debug)]
pub struct Item {
    pub name: String,
    pub label: String,
    pub is_app: bool,
    pub reasons: Vec<String>,
}

impl Item {
    pub fn needs_reboot(&self) -> bool {
        REBOOT_STEPS.contains(&self.name.as_str())
    }
}

pub struct Plan {
    pub local: String,
    pub local_date: String,
    pub remote: String,
    pub remote_date: String,
    pub commits: Vec<Commit>,
    pub items: Vec<Item>,
    pub core_changes: Vec<String>,
    pub unaffected: Vec<String>,
    pub estimated: bool,
    pub fetch_error: Option<String>,
}

/// Picks every installed entry whose sources changed since its base.
pub fn select(
    entries: &[Entry],
    known_names: Option<&HashSet<String>>,
    base_of: impl Fn(&Entry) -> Base,
    mut changed_since: impl FnMut(&str) -> Result<Vec<String>, String>,
    target: &str,
    home: &Path,
) -> Result<Vec<Item>, String> {
    let mut cache: HashMap<String, Vec<String>> = HashMap::new();
    let mut items = Vec::new();
    for entry in entries.iter().filter(|entry| !entry.is_core() && entry.is_installed(home)) {
        let reasons = match base_of(entry) {
            Base::Unknown => vec!["installed version unknown".to_owned()],
            Base::Estimated(_) if known_names.is_some_and(|names| !names.contains(&entry.name)) => {
                vec!["new in this update".to_owned()]
            }
            Base::Applied(base) | Base::Estimated(base) => {
                if base == target {
                    continue;
                }
                if !cache.contains_key(&base) {
                    cache.insert(base.clone(), changed_since(&base)?);
                }
                cache[&base].iter().filter(|path| entry.matches(path)).cloned().collect()
            }
        };
        if !reasons.is_empty() {
            items.push(Item {
                name: entry.name.clone(),
                label: entry.label().to_owned(),
                is_app: entry.app().is_some(),
                reasons,
            });
        }
    }
    Ok(items)
}

pub fn check(repo: &Repository, home: &Path) -> Result<Plan, String> {
    let fetch_error = repo.fetch().err();
    let local = repo.resolve("HEAD")?;
    let remote = repo
        .resolve(REMOTE_REF)
        .map_err(|_| format!("{REMOTE_REF} is missing in {}", repo.path().display()))?;
    let map_text = repo
        .show(&remote, map::MAP_PATH)
        .ok_or_else(|| format!("{} is missing on GitHub", map::MAP_PATH))?;
    let entries = map::parse(&map_text);
    let known_names: Option<HashSet<String>> = repo
        .show(&local, map::MAP_PATH)
        .map(|text| map::parse(&text).into_iter().map(|entry| entry.name).collect());

    let applied = map::applied_dir();
    let mut estimated = false;
    let base_of = |entry: &Entry| match map::applied_commit(&applied, &entry.name) {
        Some(commit) if repo.has_commit(&commit) => Base::Applied(commit),
        Some(_) => Base::Unknown,
        None => Base::Estimated(local.clone()),
    };
    for entry in &entries {
        estimated |= matches!(base_of(entry), Base::Estimated(_)) && !entry.is_core();
    }
    let items = select(
        &entries,
        known_names.as_ref(),
        base_of,
        |base| repo.changed_files(base, &remote),
        &remote,
        home,
    )?;

    let changed = repo.changed_files(&local, &remote)?;
    let core_changes = changed
        .iter()
        .filter(|path| entries.iter().any(|entry| entry.is_core() && entry.matches(path)))
        .cloned()
        .collect();
    let unaffected = changed
        .iter()
        .filter(|path| !entries.iter().any(|entry| entry.matches(path)))
        .cloned()
        .collect();

    Ok(Plan {
        local_date: repo.commit_date(&local),
        remote_date: repo.commit_date(&remote),
        commits: repo.commits(&local, &remote)?,
        local,
        remote,
        items,
        core_changes,
        unaffected,
        estimated,
        fetch_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = "\
core                     scripts/fedora/lib.sh
install-dkms-modules.sh  modules/
install-dsp.sh           dsp/
app:t2-fan-control       apps/t2-fan-control/
app:t2-touchbar          if=/nonexistent apps/t2-touchbar/
";

    fn run(base_of: impl Fn(&Entry) -> Base, changed: &[&str], known: Option<&[&str]>) -> Vec<String> {
        let entries = map::parse(MAP);
        let known: Option<HashSet<String>> =
            known.map(|names| names.iter().map(|name| name.to_string()).collect());
        let changed: Vec<String> = changed.iter().map(|path| path.to_string()).collect();
        select(&entries, known.as_ref(), base_of, |_| Ok(changed.clone()), "target", Path::new("/"))
            .unwrap()
            .into_iter()
            .map(|item| item.name)
            .collect()
    }

    fn applied(_: &Entry) -> Base {
        Base::Applied("old".to_owned())
    }

    #[test]
    fn module_change_selects_dkms_only() {
        assert_eq!(run(applied, &["modules/t2smc/t2smc.c"], None), ["install-dkms-modules.sh"]);
    }

    #[test]
    fn readme_change_selects_nothing() {
        assert!(run(applied, &["README.md", "website/index.html"], None).is_empty());
    }

    #[test]
    fn core_change_is_not_a_step() {
        assert!(run(applied, &["scripts/fedora/lib.sh"], None).is_empty());
    }

    #[test]
    fn missing_optional_part_is_skipped() {
        assert!(run(applied, &["apps/t2-touchbar/src/main.rs"], None).is_empty());
    }

    #[test]
    fn up_to_date_entries_are_skipped() {
        let base_of = |_: &Entry| Base::Applied("target".to_owned());
        assert!(run(base_of, &["dsp/a.json"], None).is_empty());
    }

    #[test]
    fn new_and_unknown_entries_always_run() {
        let base_of = |entry: &Entry| match entry.name.as_str() {
            "install-dsp.sh" => Base::Estimated("head".to_owned()),
            "app:t2-fan-control" => Base::Unknown,
            _ => Base::Applied("target".to_owned()),
        };
        let known = ["core", "install-dkms-modules.sh", "app:t2-fan-control"];
        assert_eq!(run(base_of, &[], Some(&known)), ["install-dsp.sh", "app:t2-fan-control"]);
    }

    #[test]
    fn reboot_is_needed_for_modules_only() {
        let item = |name: &str| Item { name: name.to_owned(), label: String::new(), is_app: false, reasons: vec![] };
        assert!(item("install-dkms-modules.sh").needs_reboot());
        assert!(!item("app:t2-fan-control").needs_reboot());
    }
}
