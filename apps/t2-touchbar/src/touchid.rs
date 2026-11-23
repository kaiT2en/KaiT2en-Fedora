// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

use std::{
    io::Write,
    os::unix::net::UnixStream,
    sync::mpsc::{self, Receiver},
    thread,
};

use anyhow::{Context, Result};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message::Type};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchIdState {
    Idle,
    Waiting,
    Scanning,
    Matched,
    Retry,
    Failed,
}

impl TouchIdState {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "idle" => Self::Idle,
            "waiting" => Self::Waiting,
            "scanning" => Self::Scanning,
            "matched" => Self::Matched,
            "retry" => Self::Retry,
            "failed" => Self::Failed,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Waiting => "waiting",
            Self::Scanning => "scanning",
            Self::Matched => "matched",
            Self::Retry => "retry",
            Self::Failed => "failed",
        }
    }
}

/// A blocking zbus iterator lives on its own sleeping thread. The socket pair
/// gives the main poll loop a regular fd, avoiding any periodic D-Bus polling.
pub fn watch() -> Result<(Receiver<TouchIdState>, UnixStream)> {
    let (sender, receiver) = mpsc::channel();
    let (mut wake_writer, wake_reader) = UnixStream::pair()?;
    thread::Builder::new()
        .name("touch-id-signals".to_owned())
        .spawn(move || {
            loop {
                if let Err(error) = listen(&sender, &mut wake_writer) {
                    eprintln!("kait2en-touchbar: Touch ID signal listener: {error:#}");
                    thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        })?;
    Ok((receiver, wake_reader))
}

fn listen(sender: &mpsc::Sender<TouchIdState>, wake: &mut UnixStream) -> Result<()> {
    let connection = Connection::system().context("connect system bus")?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.kait2en.TouchId")?
        .path("/org/kait2en/TouchId")?
        .interface("org.kait2en.TouchId")?
        .member("Changed")?
        .build();
    let iterator = MessageIterator::for_match_rule(rule, &connection, None)?;
    for message in iterator {
        let message = message?;
        let (raw,): (String,) = message.body().deserialize()?;
        let Some(state) = TouchIdState::parse(&raw) else {
            continue;
        };
        if sender.send(state).is_err() {
            return Ok(());
        }
        let _ = wake.write_all(&[1]);
    }
    Err(anyhow::anyhow!("system bus iterator ended"))
}
