// SPDX-License-Identifier: GPL-3.0-or-later

//! Touch Bar Settings: edits the per-app rows of kait2en-touchbar in
//! `$XDG_CONFIG_HOME/kait2en-touchbar/apps.toml` and restarts the Touch Bar
//! to apply them.

mod app_page;
mod focus;
mod key_dialog;
mod model;
mod pickers;
mod window;

use adw::prelude::*;
use gtk4::{gdk, glib};

const APP_ID: &str = "org.kait2entouchbar.gtk";

const CSS: &str = "
.touchbar-preview {
    background: #000;
    border-radius: 8px;
    padding: 6px;
}
.touchbar-preview label {
    background: #3a3a3c;
    color: #fff;
    border-radius: 5px;
    padding: 4px 10px;
    min-width: 36px;
    font-size: 0.9em;
}
.touchbar-preview label.esc {
    background: #2a2a2c;
}
";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        window::build(app);
    });
    app.set_accels_for_action("win.save", &["<Control>s"]);
    app.run()
}
