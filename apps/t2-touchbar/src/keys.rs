// SPDX-License-Identifier: GPL-3.0-or-later

//! Personal keys for the free space of the special row, read from
//! `$XDG_CONFIG_HOME/kait2en-touchbar/keys.toml`. The file lives in the home
//! directory, so installs and updates never touch it.
//!
//! ```toml
//! [[key]]
//! label = "~"
//! send = "altgr+]"    # types ~ on the German layout
//!
//! [[key]]
//! label = "term"
//! run = "kgx"         # started through systemd-run
//! ```

use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use anyhow::{Context, Result, anyhow, bail};
use input_linux::Key;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    key: Vec<Entry>,
}

/// One key as written in keys.toml and apps.toml.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    label: String,
    send: Option<String>,
    run: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CustomAction {
    /// Modifiers first, the key last.
    Send(Vec<Key>),
    Run(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomKey {
    /// Leaked once at startup, so layouts can keep plain `&'static str`
    /// labels like the built-in keys.
    pub label: &'static str,
    pub action: CustomAction,
}

impl CustomKey {
    /// Keys the virtual keyboard must be able to send for this entry.
    pub fn keys(&self) -> &[Key] {
        match &self.action {
            CustomAction::Send(keys) => keys,
            CustomAction::Run(_) => &[],
        }
    }

    /// Starts the command outside the daemon's service, so it survives a
    /// restart of the Touch Bar.
    pub fn run(command: &str) {
        let spawned = Command::new("systemd-run")
            .args(["--user", "--collect", "--quiet", "--", "sh", "-c", command])
            .spawn();
        match spawned {
            Ok(mut child) => {
                thread::spawn(move || child.wait());
            }
            Err(error) => eprintln!("kait2en-touchbar: could not start {command:?}: {error}"),
        }
    }
}

/// `$XDG_CONFIG_HOME/kait2en-touchbar`, home of the personal files.
pub fn config_dir() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("kait2en-touchbar")
}

pub fn path() -> PathBuf {
    config_dir().join("keys.toml")
}

/// Written once when no keys file exists, so the format is discoverable
/// where it is used. Everything is commented out: no personal keys until
/// the user adds some.
const TEMPLATE: &str = r#"# Personal keys for the free space of the third Touch Bar row (swipe with
# two fingers to reach it). Up to four keys fit. Further entries are ignored.
# Restart after editing:  systemctl --user restart kait2en-touchbar
#
# Every key has a text label (1-12 characters) and exactly one action:
#   send = "<combination>"   modifiers ctrl, shift, alt (option), altgr, super
#                            plus one key, named by its US key position:
#                            a-z, 0-9, f1-f12, - = [ ] ; ' ` \ , . /,
#                            iso (extra key next to left shift), space, tab,
#                            enter, backspace, esc, left, right, up, down,
#                            home, end, pageup, pagedown, insert, delete, print
#   run = "<command>"        started through systemd-run --user
#
# The combination types whatever your keyboard layout puts there, so pick the
# one that produces the character on your layout. Examples for the German
# layout (de), where AltGr is the right Alt key:
#
# [[key]]
# label = "~"
# send = "altgr+]"
#
# [[key]]
# label = "|"
# send = "altgr+iso"
#
# [[key]]
# label = '\'          # single quotes keep the backslash literal
# send = "altgr+-"
#
# [[key]]
# label = "term"
# run = "kgx"
"#;

/// Creates the commented template if there is no keys file yet. Never
/// overwrites an existing file.
fn create_template(path: &Path) {
    write_template(path, TEMPLATE);
}

/// Writes `template` to `path` unless a file is already there.
pub fn write_template(path: &Path, template: &str) {
    let created = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| OpenOptions::new().write(true).create_new(true).open(path))
        .and_then(|mut file| file.write_all(template.as_bytes()));
    if let Err(error) = created {
        eprintln!(
            "kait2en-touchbar: could not create {}: {error}",
            path.display()
        );
    }
}

/// Loads the personal keys. A missing file is created from the commented
/// template and means none. A broken entry is reported and skipped, so one
/// typo never takes the Touch Bar down.
pub fn load() -> Vec<CustomKey> {
    let path = path();
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_template(&path);
            return Vec::new();
        }
        Err(error) => {
            eprintln!("kait2en-touchbar: read {}: {error}", path.display());
            return Vec::new();
        }
    };
    match parse(&body) {
        Ok(keys) => keys,
        Err(error) => {
            eprintln!("kait2en-touchbar: {}: {error:#}", path.display());
            Vec::new()
        }
    }
}

fn parse(body: &str) -> Result<Vec<CustomKey>> {
    let file: File = toml::from_str(body).context("invalid keys file")?;
    Ok(entries("key", file.key))
}

/// Turns entries into keys. A broken entry is reported with `context` and
/// skipped.
pub fn entries(context: &str, entries: Vec<Entry>) -> Vec<CustomKey> {
    let mut keys = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        match entry_key(entry) {
            Ok(key) => keys.push(key),
            Err(error) => eprintln!("kait2en-touchbar: {context} {}: {error:#}", index + 1),
        }
    }
    keys
}

fn entry_key(entry: Entry) -> Result<CustomKey> {
    let label = entry.label.trim();
    if label.is_empty() || label.chars().count() > 12 {
        bail!("label must be 1 to 12 characters");
    }
    let action = match (entry.send, entry.run) {
        (Some(send), None) => CustomAction::Send(combination(&send)?),
        (None, Some(run)) if !run.trim().is_empty() => CustomAction::Run(run),
        _ => bail!("set exactly one of send or run"),
    };
    Ok(CustomKey {
        label: Box::leak(label.to_owned().into_boxed_str()),
        action,
    })
}

/// "ctrl+alt+t" → modifiers, then one key. Key names follow the US key
/// positions, so the result depends on the active layout like a real
/// keyboard does.
fn combination(text: &str) -> Result<Vec<Key>> {
    let parts: Vec<String> = text
        .split('+')
        .map(|part| part.trim().to_ascii_lowercase())
        .collect();
    let (last, modifiers) = parts
        .split_last()
        .filter(|(last, _)| !last.is_empty())
        .ok_or_else(|| anyhow!("empty combination"))?;
    let mut keys = Vec::new();
    for modifier in modifiers {
        let key = match modifier.as_str() {
            "ctrl" | "control" => Key::LeftCtrl,
            "shift" => Key::LeftShift,
            "alt" | "option" => Key::LeftAlt,
            "altgr" => Key::RightAlt,
            "super" | "meta" | "cmd" | "command" => Key::LeftMeta,
            other => bail!("unknown modifier {other:?}"),
        };
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.push(key_by_name(last).ok_or_else(|| anyhow!("unknown key {last:?}"))?);
    Ok(keys)
}

fn key_by_name(name: &str) -> Option<Key> {
    const LETTERS: [Key; 26] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    const DIGITS: [Key; 10] = [
        Key::Num0,
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    const FUNCTION: [Key; 12] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    let mut chars = name.chars();
    if let (Some(single), None) = (chars.next(), chars.next()) {
        if single.is_ascii_lowercase() {
            return Some(LETTERS[(single as u8 - b'a') as usize]);
        }
        if single.is_ascii_digit() {
            return Some(DIGITS[(single as u8 - b'0') as usize]);
        }
    }
    if let Some(number) = name.strip_prefix('f').and_then(|n| n.parse::<usize>().ok()) {
        return (1..=12).contains(&number).then(|| FUNCTION[number - 1]);
    }
    Some(match name {
        "-" | "minus" => Key::Minus,
        "=" | "equal" => Key::Equal,
        "[" | "leftbrace" => Key::LeftBrace,
        "]" | "rightbrace" => Key::RightBrace,
        ";" | "semicolon" => Key::Semicolon,
        "'" | "apostrophe" => Key::Apostrophe,
        "`" | "grave" => Key::Grave,
        "\\" | "backslash" => Key::Backslash,
        "," | "comma" => Key::Comma,
        "." | "dot" => Key::Dot,
        "/" | "slash" => Key::Slash,
        // The extra key next to the left shift on ISO keyboards.
        "iso" | "102nd" => Key::NonUsBackslashAndPipe,
        "space" => Key::Space,
        "tab" => Key::Tab,
        "enter" | "return" => Key::Enter,
        "backspace" => Key::Backspace,
        "esc" | "escape" => Key::Esc,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "insert" => Key::Insert,
        "delete" => Key::Delete,
        "print" => Key::Sysrq,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_file_is_parsed_and_broken_entries_skipped() {
        let keys = parse(
            r#"
            [[key]]
            label = "~"
            send = "Alt+N"

            [[key]]
            label = "|"
            send = "alt+shift+7"

            [[key]]
            label = "term"
            run = "kgx"

            [[key]]
            label = "bad"
            send = "hyper+q"

            [[key]]
            label = "both"
            send = "a"
            run = "b"
            "#,
        )
        .unwrap();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0].label, "~");
        assert_eq!(
            keys[0].action,
            CustomAction::Send(vec![Key::LeftAlt, Key::N])
        );
        assert_eq!(
            keys[1].action,
            CustomAction::Send(vec![Key::LeftAlt, Key::LeftShift, Key::Num7])
        );
        assert_eq!(keys[2].action, CustomAction::Run("kgx".to_owned()));
    }

    #[test]
    fn template_is_valid_and_defines_no_keys() {
        assert!(parse(TEMPLATE).unwrap().is_empty());
        // Uncommenting the examples gives four working keys.
        let examples: String = TEMPLATE
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| {
                line.starts_with('[')
                    || line.starts_with("label")
                    || line.starts_with("send")
                    || line.starts_with("run")
            })
            .map(|line| format!("{line}\n"))
            .collect();
        assert_eq!(parse(&examples).unwrap().len(), 4);
    }

    #[test]
    fn template_never_replaces_an_existing_file() {
        let dir = env::temp_dir().join(format!("kait2en-keys-{}", std::process::id()));
        let path = dir.join("kait2en-touchbar/keys.toml");
        create_template(&path);
        assert_eq!(fs::read_to_string(&path).unwrap(), TEMPLATE);
        fs::write(&path, "# mine").unwrap();
        create_template(&path);
        assert_eq!(fs::read_to_string(&path).unwrap(), "# mine");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn key_names_cover_positions_and_specials() {
        assert_eq!(
            combination("ctrl+alt+t").unwrap(),
            [Key::LeftCtrl, Key::LeftAlt, Key::T]
        );
        assert_eq!(
            combination("altgr+iso").unwrap(),
            [Key::RightAlt, Key::NonUsBackslashAndPipe]
        );
        assert_eq!(combination("F12").unwrap(), [Key::F12]);
        assert!(combination("ctrl+").is_err());
        assert!(combination("f13").is_err());
    }
}
