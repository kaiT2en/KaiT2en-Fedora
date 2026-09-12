// SPDX-License-Identifier: LicenseRef-KAIT2EN-1.0
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

//! Client side of libfprint's virtual storage device socket. The device reads
//! one command per connection and closes it, so every command connects afresh.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;

use anyhow::Result;
use t2_biometrickit::Identity;

/// Where fprintd keeps the binding between a user and a print.
pub const STORE: &str = "/var/lib/fprint";

/// A bare connection carries no command, so it cannot disturb an operation
/// libfprint has in flight. It succeeds only while fprintd holds the device
/// open; the socket file itself stays behind after a close, so its presence
/// proves nothing.
pub fn device_is_open(path: &str) -> bool {
    UnixStream::connect(path).is_ok()
}

pub fn send(path: &str, command: &str) -> Result<()> {
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(command.as_bytes())?;
    stream.flush()?;
    Ok(())
}

/// The print identifier fprintd stores. It is the SEP identity's UUID, so a
/// binding made once keeps working as long as that finger stays enrolled.
/// Abort the operation fprintd has in flight. Silence would leave every sudo
/// waiting for its own timeout, so any failure has to be said out loud.
pub fn report_failure(path: &str) {
    if let Err(error) = send(path, "ERROR 0") {
        eprintln!("t2-touchid: could not tell fprintd about the failure: {error}");
    }
}

/// Whether fprintd already has this identity bound to some user.
///
/// The virtual device's storage is empty after every fprintd restart and a
/// verification against a print the device does not know fails, so a bound
/// identity has to be put back before each attempt. An identity fprintd does
/// not know is deliberately left out: seeding it would make the one-time
/// binding enrolment fail as a duplicate finger.
pub fn is_bound(store: &Path, id: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(store) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            is_bound(&path, id)
        } else {
            std::fs::read(&path).is_ok_and(|body| {
                body.windows(id.len()).any(|window| window == id.as_bytes())
            })
        }
    })
}

pub fn print_id(identity: &Identity) -> String {
    let u = &identity.uuid;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u[0], u[1], u[2], u[3], u[4], u[5], u[6], u[7],
        u[8], u[9], u[10], u[11], u[12], u[13], u[14], u[15]
    )
}
