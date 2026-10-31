// SPDX-License-Identifier: GPL-3.0-or-later

//! Media players over MPRIS: seeking in the active player (desktops treat
//! the rewind and fast forward keys inconsistently), track changes and
//! whether anything plays, for the dark bar.

use std::{
    collections::HashMap,
    io::Write,
    os::unix::net::UnixStream,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use zbus::blocking::{Connection, MessageIterator, connection::Builder, fdo::DBusProxy};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{MatchRule, message::Type};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// A hung player must not stall the Touch Bar.
const CALL_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Default)]
pub struct Mpris {
    connection: Option<Connection>,
}

impl Mpris {
    /// Picks the player a seek should go to: a playing one first, then a
    /// paused one, then any.
    pub fn active_player(&mut self) -> Option<String> {
        let connection = self.connection().ok()?;
        let names: Vec<String> = connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "ListNames",
                &(),
            )
            .ok()?
            .body()
            .deserialize()
            .ok()?;
        names
            .into_iter()
            .filter(|name| name.starts_with(PREFIX))
            .map(|name| {
                let rank = match playback_status(connection, &name).as_deref() {
                    Some("Playing") => 0,
                    Some("Paused") => 1,
                    _ => 2,
                };
                (rank, name)
            })
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, name)| name)
    }

    pub fn seek(&mut self, player: &str, offset: Duration, forward: bool) -> Result<()> {
        let micros = offset.as_micros() as i64;
        let micros = if forward { micros } else { -micros };
        self.connection()?
            .call_method(Some(player), PATH, Some(PLAYER), "Seek", &(micros,))?;
        Ok(())
    }

    fn connection(&mut self) -> Result<&Connection> {
        if self.connection.is_none() {
            self.connection = Some(Builder::session()?.method_timeout(CALL_TIMEOUT).build()?);
        }
        Ok(self
            .connection
            .as_ref()
            .expect("connection was just created"))
    }
}

fn playback_status(connection: &Connection, name: &str) -> Option<String> {
    String::try_from(property(connection, name, "PlaybackStatus")?).ok()
}

fn property(connection: &Connection, name: &str, property: &str) -> Option<OwnedValue> {
    connection
        .call_method(
            Some(name),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(PLAYER, property),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Track {
    pub artist: String,
    pub title: String,
}

pub enum MediaEvent {
    /// A track is playing that differs from the last reported one.
    Track(Track),
    /// Whether any player plays at all, and whether any is paused.
    Status { playing: bool, paused: bool },
}

/// Reports a track once it is playing and differs from the last reported
/// one, and whether anything plays. Players repeat their metadata often
/// (cover art, position updates), so duplicates are dropped here.
pub fn watch() -> Result<(Receiver<MediaEvent>, UnixStream)> {
    let (sender, receiver) = mpsc::channel();
    let (mut wake_writer, wake_reader) = UnixStream::pair()?;
    thread::Builder::new()
        .name("mpris-tracks".to_owned())
        .spawn(move || {
            let mut reported = false;
            loop {
                if let Err(error) = listen(&sender, &mut wake_writer) {
                    if !reported {
                        eprintln!("kait2en-touchbar: track listener: {error:#}");
                        reported = true;
                    }
                    thread::sleep(Duration::from_secs(30));
                }
            }
        })?;
    Ok((receiver, wake_reader))
}

#[derive(Default)]
struct PlayerState {
    status: Status,
    track: Option<Track>,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl Status {
    fn parse(status: Option<&str>) -> Self {
        match status {
            Some("Playing") => Self::Playing,
            Some("Paused") => Self::Paused,
            _ => Self::Stopped,
        }
    }
}

/// Players by unique bus name, which is what their signals carry.
struct Players {
    players: HashMap<String, PlayerState>,
    last: Option<Track>,
    status: (bool, bool),
}

impl Players {
    /// Sends what changed. Returns false once the receiver is gone.
    fn report(&mut self, name: &str, sender: &Sender<MediaEvent>, wake: &mut UnixStream) -> bool {
        let mut events = Vec::new();
        if let Some(track) = self
            .players
            .get(name)
            .filter(|state| state.status == Status::Playing)
            .and_then(|state| state.track.clone())
            && self.last.as_ref() != Some(&track)
        {
            self.last = Some(track.clone());
            events.push(MediaEvent::Track(track));
        }
        // The track goes first, so the overlay starts before anything that
        // waits for it to end.
        let any = |status| self.players.values().any(|state| state.status == status);
        let (playing, paused) = (any(Status::Playing), any(Status::Paused));
        if (playing, paused) != self.status {
            self.status = (playing, paused);
            events.push(MediaEvent::Status { playing, paused });
        }
        if events.is_empty() {
            return true;
        }
        for event in events {
            if sender.send(event).is_err() {
                return false;
            }
        }
        let _ = wake.write_all(&[1]);
        true
    }
}

fn listen(sender: &Sender<MediaEvent>, wake: &mut UnixStream) -> Result<()> {
    let connection = Builder::session()?
        .method_timeout(CALL_TIMEOUT)
        .build()
        .context("connect session bus")?;
    let iterator = MessageIterator::from(&connection);
    let bus = DBusProxy::new(&connection)?;
    bus.add_match_rule(
        MatchRule::builder()
            .msg_type(Type::Signal)
            .path(PATH)?
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")?
            .arg(0, PLAYER)?
            .build(),
    )?;
    // Players that quit without reporting a stop.
    bus.add_match_rule(
        MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg0ns("org.mpris.MediaPlayer2")?
            .build(),
    )?;
    let mut players = Players {
        players: HashMap::new(),
        last: None,
        status: (false, false),
    };
    // Whatever already plays when the daemon starts.
    for name in bus.list_names()? {
        if !name.starts_with(PREFIX) {
            continue;
        }
        let Ok(owner) = bus.get_name_owner(name.as_ref()) else {
            continue;
        };
        let state = PlayerState {
            status: Status::parse(playback_status(&connection, &name).as_deref()),
            track: property(&connection, &name, "Metadata").and_then(|value| parse_track(&value)),
        };
        players.players.insert(owner.to_string(), state);
        if !players.report(&owner, sender, wake) {
            return Ok(());
        }
    }
    for message in iterator {
        let message = message?;
        if message.message_type() != Type::Signal {
            continue;
        }
        let header = message.header();
        let name = if header
            .member()
            .is_some_and(|member| member == "NameOwnerChanged")
        {
            let Ok((_, old, new)) = message.body().deserialize::<(String, String, String)>() else {
                continue;
            };
            players.players.remove(&old);
            if !new.is_empty() {
                let state = PlayerState {
                    status: Status::parse(playback_status(&connection, &new).as_deref()),
                    track: None,
                };
                players.players.insert(new.clone(), state);
            }
            new
        } else {
            let Some(name) = header.sender().map(ToString::to_string) else {
                continue;
            };
            let Ok((_, changed, _)) =
                message
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
            else {
                continue;
            };
            let known = players.players.contains_key(&name);
            let state = players.players.entry(name.clone()).or_default();
            if let Some(status) = changed.get("PlaybackStatus") {
                state.status = Status::parse(match &**status {
                    Value::Str(status) => Some(status.as_str()),
                    _ => None,
                });
            } else if !known {
                state.status = Status::parse(playback_status(&connection, &name).as_deref());
            }
            if let Some(metadata) = changed.get("Metadata") {
                state.track = parse_track(metadata);
            }
            name
        };
        if !players.report(&name, sender, wake) {
            return Ok(());
        }
    }
    Err(anyhow::anyhow!("session bus iterator ended"))
}

fn parse_track(metadata: &OwnedValue) -> Option<Track> {
    let Value::Dict(dict) = unwrap_variant(metadata) else {
        return None;
    };
    let mut artist = String::new();
    let mut title = String::new();
    for (key, value) in dict.iter() {
        let Value::Str(key) = unwrap_variant(key) else {
            continue;
        };
        match (key.as_str(), unwrap_variant(value)) {
            ("xesam:title", Value::Str(value)) => title = value.to_string(),
            ("xesam:artist", Value::Array(values)) => {
                artist = values
                    .iter()
                    .filter_map(|value| match unwrap_variant(value) {
                        Value::Str(name) => Some(name.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
            }
            // Some players send a single string instead of the list.
            ("xesam:artist", Value::Str(value)) => artist = value.to_string(),
            _ => {}
        }
    }
    let (artist, title) = (artist.trim().to_owned(), title.trim().to_owned());
    (!title.is_empty()).then_some(Track { artist, title })
}

fn unwrap_variant<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    match value {
        Value::Value(inner) => unwrap_variant(inner),
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(entries: Vec<(&str, Value<'static>)>) -> OwnedValue {
        let map: HashMap<String, Value<'static>> = entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        OwnedValue::try_from(Value::from(map)).unwrap()
    }

    #[test]
    fn artist_list_and_title_are_read() {
        let value = metadata(vec![
            ("xesam:title", Value::from("Duality")),
            (
                "xesam:artist",
                Value::from(vec!["Slipknot", "Corey Taylor"]),
            ),
            ("mpris:length", Value::from(252_000_000i64)),
        ]);
        assert_eq!(
            parse_track(&value),
            Some(Track {
                artist: "Slipknot, Corey Taylor".to_owned(),
                title: "Duality".to_owned(),
            })
        );
    }

    #[test]
    fn missing_title_is_no_track() {
        let value = metadata(vec![("xesam:artist", Value::from("Heino"))]);
        assert_eq!(parse_track(&value), None);
    }
}
