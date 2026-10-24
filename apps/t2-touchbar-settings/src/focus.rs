// SPDX-License-Identifier: GPL-3.0-or-later

//! Asks the "Focused Window D-Bus" GNOME Shell extension which window has
//! the focus, the same source the Touch Bar follows.

use gtk4::gio;
use gtk4::glib;
use serde::Deserialize;

pub const EXTENSION_URL: &str = "https://extensions.gnome.org/extension/5592/focused-window-d-bus/";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Window {
    pub title: Option<String>,
    pub wm_class: Option<String>,
    pub wm_class_instance: Option<String>,
}

impl Window {
    /// The class a profile should match: the class, else the instance.
    pub fn class(&self) -> Option<&str> {
        [&self.wm_class, &self.wm_class_instance]
            .into_iter()
            .flatten()
            .map(|class| class.trim())
            .find(|class| !class.is_empty())
    }
}

pub async fn focused() -> Result<Window, String> {
    let connection = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(|error| error.to_string())?;
    let reply = connection
        .call_future(
            Some("org.gnome.Shell"),
            "/org/gnome/shell/extensions/FocusedWindow",
            "org.gnome.shell.extensions.FocusedWindow",
            "Get",
            None,
            Some(glib::VariantTy::new("(s)").unwrap()),
            gio::DBusCallFlags::NONE,
            1000,
        )
        .await
        .map_err(|_| {
"The Focused Window D-Bus extension is not running".to_owned()
        })?;
    let json: String = reply
        .child_value(0)
        .get()
        .ok_or("Unexpected reply from the extension")?;
    Ok(serde_json::from_str(&json).unwrap_or_default())
}

/// True when the extension answers, for the status row.
pub async fn available() -> bool {
    focused().await.is_ok()
}
