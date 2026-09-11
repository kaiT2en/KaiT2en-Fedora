// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

//! Watches systemd-logind for resume from suspend. The bridge holds a
//! BiometricKit session that the suspend kills silently; without notice it
//! would block for the socket timeout on the first prompt after waking, longer
//! than the login screen waits. logind's PrepareForSleep(false) signal marks
//! the wake, so the bridge can drop the dead session and open a fresh one
//! before fprintd asks.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

/// Returns a flag set to true on each resume. Best effort: if logind or the
/// bus is unavailable the flag simply never fires and the bridge falls back to
/// noticing the dead session by timeout.
pub fn watch() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let handle = Arc::clone(&flag);
    thread::spawn(move || {
        if let Err(error) = run(&handle) {
            eprintln!("t2-touchid: resume watch unavailable: {error}");
        }
    });
    flag
}

fn run(flag: &AtomicBool) -> zbus::Result<()> {
    let connection = zbus::blocking::Connection::system()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;
    let signals = proxy.receive_signal("PrepareForSleep")?;
    for message in signals {
        // The argument is true when going to sleep, false when resuming.
        let sleeping: bool = message.body().deserialize().unwrap_or(true);
        if !sleeping {
            flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}
