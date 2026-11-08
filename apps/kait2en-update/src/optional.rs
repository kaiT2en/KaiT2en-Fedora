// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional parts the installer asks about. Updates keep them as they are,
//! here they are added or removed.

use std::path::Path;

use gtk4::glib;

pub struct Part {
    /// Name in update-map.
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Exists once the part is installed, the same path as if= in update-map.
    marker: &'static str,
    /// The helper needs root for it, otherwise it runs as the user.
    pub privileged: bool,
}

pub const TOUCH_BAR: Part = Part {
    name: "app:t2-touchbar",
    title: "T2 Touch Bar",
    description: "Dark-first Touch Bar with a media and an F-key row, Touch ID prompt and haptic \
                  feedback. Without it, Apple's native row stays active.",
    marker: "/usr/local/bin/kait2en-touchbar",
    privileged: true,
};

pub const VORNEHM: Part = Part {
    name: "install-vornehm.sh",
    title: "KaiT2en Vornehm",
    description: "Calm dark look for GNOME with frosted glass, a slim dock and app icons on dark \
                  tiles. Removing it brings back your previous wallpaper, style and icons.",
    marker: "~/.local/share/gnome-shell/extensions/kait2en-vornehm@kait2en",
    privileged: false,
};

fn has_touch_bar() -> bool {
    std::fs::read_to_string("/sys/class/dmi/id/product_name").is_ok_and(|model| {
        let model = model.trim();
        model.starts_with("MacBookPro15,") || model.starts_with("MacBookPro16,")
    })
}

/// The parts this machine and desktop can use.
pub fn available() -> Vec<&'static Part> {
    let mut parts = Vec::new();
    if has_touch_bar() {
        parts.push(&TOUCH_BAR);
    }
    let gnome = glib::find_program_in_path("gnome-shell").is_some();
    if gnome {
        parts.push(&VORNEHM);
    }
    parts
}

impl Part {
    pub fn is_installed(&self, home: &Path) -> bool {
        match self.marker.strip_prefix("~/") {
            Some(rest) => home.join(rest).exists(),
            None => Path::new(self.marker).exists(),
        }
    }
}
