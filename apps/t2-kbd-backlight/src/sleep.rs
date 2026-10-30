// SPDX-License-Identifier: GPL-3.0-or-later

//! Suspend notifications from logind, held off by a delay inhibitor until the
//! main loop has prepared the keyboard backlight.

use std::{
    io::Write,
    os::unix::net::UnixStream,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::{MatchRule, message::Type, zvariant::OwnedFd};

/// logind waits at most InhibitDelayMaxSec anyway, this keeps a stuck main
/// loop from holding suspend that long.
const PREPARE_TIMEOUT: Duration = Duration::from_secs(2);

/// Each suspend sends a sender, the main loop answers on it once it is ready.
pub fn watch() -> Result<(Receiver<Sender<()>>, UnixStream)> {
    let (sender, receiver) = mpsc::channel();
    let (mut wake_writer, wake_reader) = UnixStream::pair()?;
    thread::Builder::new()
        .name("sleep-signals".to_owned())
        .spawn(move || {
            loop {
                if let Err(error) = listen(&sender, &mut wake_writer) {
                    eprintln!("t2-kbd-backlight: sleep signal listener: {error:#}");
                    thread::sleep(Duration::from_secs(2));
                }
            }
        })?;
    Ok((receiver, wake_reader))
}

fn listen(sender: &Sender<Sender<()>>, wake: &mut UnixStream) -> Result<()> {
    let connection = Connection::system().context("connect system bus")?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.login1")?
        .path("/org/freedesktop/login1")?
        .interface("org.freedesktop.login1.Manager")?
        .member("PrepareForSleep")?
        .build();
    // Subscribed before the inhibitor exists, so no suspend slips through.
    let iterator = MessageIterator::for_match_rule(rule, &connection, None)?;
    let manager = Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;
    let mut inhibitor = Some(inhibit(&manager)?);
    for message in iterator {
        let message = message?;
        let (suspending,): (bool,) = message.body().deserialize()?;
        if suspending {
            let (done, ready) = mpsc::channel();
            sender.send(done).context("main loop is gone")?;
            let _ = wake.write_all(&[1]);
            let _ = ready.recv_timeout(PREPARE_TIMEOUT);
            inhibitor = None;
        } else if inhibitor.is_none() {
            inhibitor = Some(inhibit(&manager)?);
        }
    }
    drop(inhibitor);
    Err(anyhow::anyhow!("system bus iterator ended"))
}

fn inhibit(manager: &Proxy) -> Result<OwnedFd> {
    manager
        .call(
            "Inhibit",
            &(
                "sleep",
                "T2 keyboard backlight",
                "Restore the keyboard backlight before suspend",
                "delay",
            ),
        )
        .context("take logind sleep delay inhibitor")
}
