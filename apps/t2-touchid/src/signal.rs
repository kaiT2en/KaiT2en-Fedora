// SPDX-License-Identifier: LicenseRef-KAIT2EN-1.0
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

//! Broadcasts the fingerprint prompt state on the D-Bus system bus so the Touch
//! Bar (react-drm) can show a "touch to unlock" animation. It is best effort:
//! if the bus or the name is unavailable the bridge carries on unaffected, the
//! animation simply does not play.

use zbus::blocking::Connection;

const NAME: &str = "org.kait2en.TouchId";
const PATH: &str = "/org/kait2en/TouchId";
const IFACE: &str = "org.kait2en.TouchId";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Waiting,
    Scanning,
    Matched,
    Retry,
    Failed,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Waiting => "waiting",
            State::Scanning => "scanning",
            State::Matched => "matched",
            State::Retry => "retry",
            State::Failed => "failed",
        }
    }
}

pub struct Signal {
    connection: Option<Connection>,
    last: Option<State>,
}

impl Signal {
    pub fn new() -> Self {
        let connection = match Connection::system() {
            Ok(connection) => {
                // Owning the name is only for discoverability; a failure to get
                // it does not stop us broadcasting the signal.
                let _ = connection.request_name(NAME);
                Some(connection)
            }
            Err(error) => {
                eprintln!("t2-touchid: no system bus for the Touch Bar signal: {error}");
                None
            }
        };
        Self { connection, last: None }
    }

    /// Emit the state, but only when it actually changed, so the bus does not
    /// see a burst of identical signals during a scan.
    pub fn set(&mut self, state: State) {
        if self.last == Some(state) {
            return;
        }
        self.last = Some(state);
        if let Some(connection) = &self.connection
            && let Err(error) =
                connection.emit_signal(None::<&str>, PATH, IFACE, "Changed", &(state.as_str(),))
        {
            eprintln!("t2-touchid: could not emit the Touch Bar signal: {error}");
        }
    }
}
