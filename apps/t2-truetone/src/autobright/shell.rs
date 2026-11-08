// SPDX-License-Identifier: GPL-3.0-or-later
//! gnome-shell's brightness manager, which owns the slider in GNOME 50 and 51.
//!
//! The service keeps the shell's automatic target off, so the shell adopts the
//! service's writes instead of adding a target on top. The shell emits
//! `BrightnessChanged` for slider and key changes only, never for an adopted
//! change, which is what tells a manual change apart.

use std::sync::mpsc::Sender;
use std::thread;

use anyhow::{Context, Result};
use zbus::blocking::{Connection, Proxy};

const DESTINATION: &str = "org.gnome.Shell";
const PATH: &str = "/org/gnome/Shell/Brightness";
const INTERFACE: &str = "org.gnome.Shell.Brightness";

fn proxy(connection: &Connection) -> Result<Proxy<'_>> {
    Ok(Proxy::new(connection, DESTINATION, PATH, INTERFACE)?)
}

/// A negative target switches the shell's own automatics off.
pub fn disable_auto_target(connection: &Connection) -> Result<()> {
    proxy(connection)?
        .call_method("SetAutoBrightnessTarget", &(-1.0f64,))
        .context("gnome-shell SetAutoBrightnessTarget")?;
    Ok(())
}

pub fn watch(sender: Sender<()>) {
    thread::spawn(move || {
        if let Err(error) = run_watch(&sender) {
            super::log(&format!("gnome-shell watch stopped: {error:#}"));
        }
    });
}

fn run_watch(sender: &Sender<()>) -> Result<()> {
    let connection = Connection::session()?;
    for _ in proxy(&connection)?.receive_signal("BrightnessChanged")? {
        if sender.send(()).is_err() {
            break;
        }
    }
    Ok(())
}
