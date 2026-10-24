// SPDX-License-Identifier: GPL-3.0-or-later

//! App profiles: a row of keys per app, read from
//! `$XDG_CONFIG_HOME/kait2en-touchbar/apps.toml`. While an app with a profile
//! has the focus, its keys form the app row.
//!
//! ```toml
//! [[app]]
//! name = "Blender"
//! wm_class = "blender"
//!
//! [[app.key]]
//! label = "render"
//! send = "f12"
//! ```

use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::keys::{self, CustomKey, Entry};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    app: Vec<AppEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppEntry {
    name: Option<String>,
    wm_class: String,
    #[serde(default)]
    key: Vec<Entry>,
}

#[derive(Debug)]
pub struct AppProfile {
    wm_class: String,
    pub keys: Vec<CustomKey>,
    /// Leaked once at startup like the personal key labels.
    pub labels: Vec<&'static str>,
}

impl AppProfile {
    /// The window class decides, so a profile follows the app whatever its
    /// window title says. Either the class or the instance may match.
    fn matches(&self, window: &crate::focus::Window) -> bool {
        [&window.wm_class, &window.wm_class_instance]
            .into_iter()
            .any(|class| class.eq_ignore_ascii_case(&self.wm_class))
    }
}

/// Index of the first profile for `window`.
pub fn find(profiles: &[AppProfile], window: &crate::focus::Window) -> Option<usize> {
    profiles.iter().position(|profile| profile.matches(window))
}

pub fn path() -> PathBuf {
    keys::config_dir().join("apps.toml")
}

const TEMPLATE: &str = r#"# App profiles: a Touch Bar row per app. While an app listed here has the
# focus, its keys appear on the app row after the special row (swipe with two
# fingers), and the bar switches to that row. Needs the "Focused Window D-Bus"
# GNOME Shell extension, which tells the Touch Bar which window has the focus.
# Restart after editing:  systemctl --user restart kait2en-touchbar
#
# wm_class is the window class of the app. Print the focused one with:
#   gdbus call --session --dest org.gnome.Shell \
#     --object-path /org/gnome/shell/extensions/FocusedWindow \
#     --method org.gnome.shell.extensions.FocusedWindow.Get
#
# Keys take the same send or run actions as keys.toml.
#
# [[app]]
# name = "Blender"
# wm_class = "blender"
#
# [[app.key]]
# label = "render"
# send = "f12"
#
# [[app.key]]
# label = "anim"
# send = "ctrl+f12"
"#;

/// Loads the app profiles. A missing file is created from the commented
/// template and means none. Broken profiles and keys are reported and
/// skipped, and so is a profile left without keys.
pub fn load() -> Vec<AppProfile> {
    let path = path();
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            keys::write_template(&path, TEMPLATE);
            return Vec::new();
        }
        Err(error) => {
            eprintln!("kait2en-touchbar: read {}: {error}", path.display());
            return Vec::new();
        }
    };
    match parse(&body) {
        Ok(profiles) => profiles,
        Err(error) => {
            eprintln!("kait2en-touchbar: {}: {error:#}", path.display());
            Vec::new()
        }
    }
}

fn parse(body: &str) -> Result<Vec<AppProfile>> {
    let file: File = toml::from_str(body).context("invalid apps file")?;
    let mut profiles = Vec::new();
    for (index, entry) in file.app.into_iter().enumerate() {
        match profile(entry) {
            Ok(profile) => profiles.push(profile),
            Err(error) => eprintln!("kait2en-touchbar: app {}: {error:#}", index + 1),
        }
    }
    Ok(profiles)
}

fn profile(entry: AppEntry) -> Result<AppProfile> {
    let wm_class = entry.wm_class.trim().to_owned();
    if wm_class.is_empty() {
        bail!("wm_class must not be empty");
    }
    let name = entry
        .name
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| wm_class.clone());
    let keys = keys::entries(&format!("{name} key"), entry.key);
    if keys.is_empty() {
        bail!("{name} has no usable keys");
    }
    let labels = keys.iter().map(|key| key.label).collect();
    Ok(AppProfile {
        wm_class,
        keys,
        labels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::Window;
    use crate::keys::CustomAction;
    use input_linux::Key;

    fn window(class: &str, instance: &str) -> Window {
        Window {
            wm_class: class.to_owned(),
            wm_class_instance: instance.to_owned(),
        }
    }

    #[test]
    fn apps_file_is_parsed_and_broken_profiles_skipped() {
        let profiles = parse(
            r#"
            [[app]]
            name = "Blender"
            wm_class = "blender"
            [[app.key]]
            label = "render"
            send = "f12"
            [[app.key]]
            label = "bad"
            send = "hyper+q"

            [[app]]
            wm_class = "no-keys"

            [[app]]
            wm_class = "  "
            [[app.key]]
            label = "x"
            send = "x"

            [[app]]
            wm_class = "org.gnome.Ptyxis"
            [[app.key]]
            label = "tab"
            run = "ptyxis --tab"
            "#,
        )
        .unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].labels, ["render"]);
        assert_eq!(profiles[0].keys[0].action, CustomAction::Send(vec![Key::F12]));
        assert_eq!(profiles[1].wm_class, "org.gnome.Ptyxis");
    }

    #[test]
    fn profiles_match_class_or_instance_ignoring_case() {
        let profiles = parse(
            r#"
            [[app]]
            wm_class = "Blender"
            [[app.key]]
            label = "render"
            send = "f12"
            "#,
        )
        .unwrap();
        assert_eq!(find(&profiles, &window("blender", "")), Some(0));
        assert_eq!(find(&profiles, &window("Other", "BLENDER")), Some(0));
        assert_eq!(find(&profiles, &window("blender-thumbnailer", "")), None);
    }

    #[test]
    fn template_is_valid_and_defines_no_profiles() {
        assert!(parse(TEMPLATE).unwrap().is_empty());
        // Uncommenting the example gives one working profile.
        let example: String = TEMPLATE
            .lines()
            .skip_while(|line| !line.starts_with("# [[app]]"))
            .filter_map(|line| line.strip_prefix("# "))
            .map(|line| format!("{line}\n"))
            .collect();
        let profiles = parse(&example).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].keys.len(), 2);
    }
}
