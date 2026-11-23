// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
mod actuators;
mod daemon;
mod error;
mod gui;
mod plimit;
mod sensors;
mod style;
mod sysfs;

use gtk4::glib;

fn main() -> glib::ExitCode {
    if std::env::args().any(|arg| arg == "--daemon") {
        let debug = std::env::args().any(|arg| arg == "--debug");
        if let Err(error) = daemon::run(debug) {
            eprintln!("{error}");
            return glib::ExitCode::from(1);
        }
        return glib::ExitCode::SUCCESS;
    }
    gui::run()
}
