// SPDX-License-Identifier: GPL-3.0-or-later

//! The focused window, for the app profiles. GNOME Shell does not publish it,
//! so this listens to the "Focused Window D-Bus" extension
//! (focused-window-dbus@flexagoon.com), which reports every focus change as
//! a JSON description of the window.

use std::{
    io::Write,
    os::unix::net::UnixStream,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use serde::Deserialize;
use zbus::blocking::{Connection, MessageIterator, connection::Builder};
use zbus::{MatchRule, message::Type};

const DESTINATION: &str = "org.gnome.Shell";
const PATH: &str = "/org/gnome/shell/extensions/FocusedWindow";
const INTERFACE: &str = "org.gnome.shell.extensions.FocusedWindow";
/// A busy shell must not stall the listener for long.
const CALL_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct Window {
    #[serde(deserialize_with = "null_as_empty")]
    pub wm_class: String,
    #[serde(deserialize_with = "null_as_empty")]
    pub wm_class_instance: String,
}

fn null_as_empty<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// Anything unreadable, like no focused window at all, is no window.
fn parse_window(json: &str) -> Window {
    serde_json::from_str(json).unwrap_or_default()
}

/// Starts the listener. Each change of the focused window's class arrives on
/// the receiver, with a byte on the stream to wake the poll loop. Without
/// the extension the listener reports once and keeps retrying, so enabling
/// it later works without a restart.
pub fn watch() -> Result<(Receiver<Window>, UnixStream)> {
    let (sender, receiver) = mpsc::channel();
    let (mut wake_writer, wake_reader) = UnixStream::pair()?;
    thread::Builder::new()
        .name("focus".to_owned())
        .spawn(move || {
            let mut reported = false;
            let mut last = None;
            loop {
                if let Err(error) = listen(&sender, &mut wake_writer, &mut last) {
                    if !reported {
                        eprintln!("kait2en-touchbar: focus listener: {error:#}");
                        reported = true;
                    }
                    thread::sleep(Duration::from_secs(30));
                }
            }
        })?;
    Ok((receiver, wake_reader))
}

fn listen(
    sender: &Sender<Window>,
    wake: &mut UnixStream,
    last: &mut Option<Window>,
) -> Result<()> {
    let connection = Builder::session()?
        .method_timeout(CALL_TIMEOUT)
        .build()
        .context("connect session bus")?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path(PATH)?
        .interface(INTERFACE)?
        .member("FocusChanged")?
        .build();
    // Subscribe before asking for the current window, so no change falls
    // between the two.
    let iterator = MessageIterator::for_match_rule(rule, &connection, None)?;
    let current = current_window(&connection)?;
    publish(current, sender, wake, last)?;
    for message in iterator {
        let message = message?;
        let Ok(json) = message.body().deserialize::<String>() else {
            continue;
        };
        publish(parse_window(&json), sender, wake, last)?;
    }
    Ok(())
}

fn current_window(connection: &Connection) -> Result<Window> {
    let json: String = connection
        .call_method(Some(DESTINATION), PATH, Some(INTERFACE), "Get", &())
        .context("ask the Focused Window D-Bus extension")?
        .body()
        .deserialize()?;
    Ok(parse_window(&json))
}

/// Title changes also count as focus changes for the extension. Only a new
/// class matters here.
fn publish(
    window: Window,
    sender: &Sender<Window>,
    wake: &mut UnixStream,
    last: &mut Option<Window>,
) -> Result<()> {
    if last.as_ref() == Some(&window) {
        return Ok(());
    }
    *last = Some(window.clone());
    sender.send(window).context("Touch Bar stopped")?;
    let _ = wake.write(&[1]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_json_is_read_leniently() {
        let window = parse_window(
            r#"{"title":"x","wm_class":"blender","wm_class_instance":null,"pid":1,"focus":true}"#,
        );
        assert_eq!(window.wm_class, "blender");
        assert_eq!(window.wm_class_instance, "");
        assert_eq!(parse_window("{}"), Window::default());
        assert_eq!(parse_window("not json"), Window::default());
    }
}
