// SPDX-License-Identifier: GPL-3.0-or-later

//! Whether the desktop has locked or blanked the screen. GNOME and KDE both
//! report it as `ActiveChanged` on their screensaver interface.

use std::{
    io::Write,
    os::unix::net::UnixStream,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use zbus::blocking::{Connection, MessageIterator, connection::Builder};
use zbus::{MatchRule, message::Type};

const SERVICES: [(&str, &str); 2] = [
    ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"),
    (
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
    ),
];

pub fn watch() -> Result<(Receiver<bool>, UnixStream)> {
    let (sender, receiver) = mpsc::channel();
    let (mut wake_writer, wake_reader) = UnixStream::pair()?;
    thread::Builder::new()
        .name("screensaver-signals".to_owned())
        .spawn(move || {
            let mut reported = false;
            loop {
                if let Err(error) = listen(&sender, &mut wake_writer) {
                    // Without it the screen just counts as on.
                    if !reported {
                        eprintln!("kait2en-touchbar: screensaver listener: {error:#}");
                        reported = true;
                    }
                    thread::sleep(Duration::from_secs(30));
                }
            }
        })?;
    Ok((receiver, wake_reader))
}

fn listen(sender: &Sender<bool>, wake: &mut UnixStream) -> Result<()> {
    let connection = Builder::session()?
        .method_timeout(Duration::from_millis(200))
        .build()
        .context("connect session bus")?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .member("ActiveChanged")?
        .build();
    let iterator = MessageIterator::for_match_rule(rule, &connection, None)?;
    if let Some(active) = active(&connection)
        && !send(active, sender, wake)
    {
        return Ok(());
    }
    for message in iterator {
        let message = message?;
        let interface = message.header().interface().map(ToString::to_string);
        if !SERVICES
            .iter()
            .any(|(service, _)| interface.as_deref() == Some(*service))
        {
            continue;
        }
        let Ok(active) = message.body().deserialize::<bool>() else {
            continue;
        };
        if !send(active, sender, wake) {
            return Ok(());
        }
    }
    Err(anyhow::anyhow!("session bus iterator ended"))
}

fn active(connection: &Connection) -> Option<bool> {
    SERVICES.iter().find_map(|(service, path)| {
        connection
            .call_method(Some(*service), *path, Some(*service), "GetActive", &())
            .ok()?
            .body()
            .deserialize()
            .ok()
    })
}

/// Returns false once the receiver is gone.
fn send(active: bool, sender: &Sender<bool>, wake: &mut UnixStream) -> bool {
    if sender.send(active).is_err() {
        return false;
    }
    let _ = wake.write_all(&[1]);
    true
}
