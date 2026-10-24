// SPDX-License-Identifier: GPL-3.0-or-later

//! apps.toml of kait2en-touchbar: reading, checking and writing it. The
//! checks mirror the daemon's, so whatever saves here also loads there.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// The app row shows this many keys after esc. Further keys are kept but
/// stay off the bar.
pub const KEYS_THAT_FIT: usize = 11;
pub const LABEL_MAX: usize = 12;
/// The window class of the default row, for every app without its own.
pub const DEFAULT_CLASS: &str = "*";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    #[serde(default)]
    pub app: Vec<App>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub wm_class: String,
    #[serde(default)]
    pub key: Vec<Key>,
}

impl App {
    pub fn default_row() -> Self {
        Self {
            name: String::new(),
            wm_class: DEFAULT_CLASS.to_owned(),
            key: Vec::new(),
        }
    }

    pub fn is_default(&self) -> bool {
        self.wm_class.trim() == DEFAULT_CLASS
    }

    pub fn title(&self) -> &str {
        if self.is_default() {
            "All Other Apps"
        } else if self.name.trim().is_empty() {
            self.wm_class.trim()
        } else {
            self.name.trim()
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
}

impl Key {
    /// Why the daemon would skip this key, if it would.
    pub fn problem(&self) -> Option<String> {
        let label = self.label.trim();
        if label.is_empty() || label.chars().count() > LABEL_MAX {
            return Some(format!("The label needs 1 to {LABEL_MAX} characters"));
        }
        match (&self.send, &self.run) {
            (Some(send), None) => combination_problem(send),
            (None, Some(run)) if !run.trim().is_empty() => None,
            (None, Some(_)) => Some("The command is empty".to_owned()),
            _ => Some("Set a shortcut or a command".to_owned()),
        }
    }
}

const MODIFIERS: &[&str] = &[
    "ctrl", "control", "shift", "alt", "option", "altgr", "super", "meta", "cmd", "command",
];

const NAMED_KEYS: &[&str] = &[
    "-", "minus", "=", "equal", "[", "leftbrace", "]", "rightbrace", ";", "semicolon", "'",
    "apostrophe", "`", "grave", "\\", "backslash", ",", "comma", ".", "dot", "/", "slash", "iso",
    "102nd", "space", "tab", "enter", "return", "backspace", "esc", "escape", "left", "right", "up",
    "down", "home", "end", "pageup", "pagedown", "insert", "delete", "print",
];

fn is_key_name(name: &str) -> bool {
    let mut chars = name.chars();
    if let (Some(single), None) = (chars.next(), chars.next())
        && (single.is_ascii_lowercase() || single.is_ascii_digit())
    {
        return true;
    }
    if let Some(number) = name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=12).contains(&number);
    }
    NAMED_KEYS.contains(&name)
}

/// Same rules as the daemon: modifiers, then exactly one key.
pub fn combination_problem(text: &str) -> Option<String> {
    let parts: Vec<String> = text
        .split('+')
        .map(|part| part.trim().to_ascii_lowercase())
        .collect();
    let Some((last, modifiers)) = parts.split_last().filter(|(last, _)| !last.is_empty()) else {
        return Some("The shortcut is empty".to_owned());
    };
    if let Some(unknown) = modifiers.iter().find(|m| !MODIFIERS.contains(&m.as_str())) {
        return Some(format!("Unknown modifier “{unknown}”"));
    }
    (!is_key_name(last)).then(|| format!("Unknown key “{last}”"))
}

/// Shortcut names for a key press, by its evdev code, so they name the US
/// key position like the daemon expects whatever the layout.
pub fn key_name(evdev: u32) -> Option<&'static str> {
    const ROW: &[(u32, &str)] = &[
        (1, "esc"), (2, "1"), (3, "2"), (4, "3"), (5, "4"), (6, "5"), (7, "6"), (8, "7"),
        (9, "8"), (10, "9"), (11, "0"), (12, "-"), (13, "="), (14, "backspace"), (15, "tab"),
        (16, "q"), (17, "w"), (18, "e"), (19, "r"), (20, "t"), (21, "y"), (22, "u"), (23, "i"),
        (24, "o"), (25, "p"), (26, "["), (27, "]"), (28, "enter"), (30, "a"), (31, "s"),
        (32, "d"), (33, "f"), (34, "g"), (35, "h"), (36, "j"), (37, "k"), (38, "l"), (39, ";"),
        (40, "'"), (41, "`"), (43, "\\"), (44, "z"), (45, "x"), (46, "c"), (47, "v"), (48, "b"),
        (49, "n"), (50, "m"), (51, ","), (52, "."), (53, "/"), (57, "space"), (59, "f1"),
        (60, "f2"), (61, "f3"), (62, "f4"), (63, "f5"), (64, "f6"), (65, "f7"), (66, "f8"),
        (67, "f9"), (68, "f10"), (86, "iso"), (87, "f11"), (88, "f12"), (99, "print"),
        (102, "home"), (103, "up"), (104, "pageup"), (105, "left"), (106, "right"), (107, "end"),
        (108, "down"), (109, "pagedown"), (110, "insert"), (111, "delete"),
    ];
    ROW.iter()
        .find(|(code, _)| *code == evdev)
        .map(|(_, name)| *name)
}

/// "ctrl+shift+t" → "Ctrl+Shift+T", for showing a shortcut.
pub fn pretty_combination(text: &str) -> String {
    text.split('+')
        .map(|part| {
            let part = part.trim();
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

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
    config_dir().join("apps.toml")
}

/// A missing file is no apps. A file that does not parse is an error, so
/// it is never overwritten unnoticed.
pub fn load(path: &Path) -> Result<File, String> {
    match fs::read_to_string(path) {
        Ok(body) => parse(&body),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(File::default()),
        Err(error) => Err(error.to_string()),
    }
}

fn parse(body: &str) -> Result<File, String> {
    toml::from_str(body).map_err(|error| error.message().to_owned())
}

const HEADER: &str = "# Touch Bar app rows, written by Touch Bar Settings \
(kait2en-touchbar-settings).\n# Hand edits are fine; the previous version is \
kept as apps.toml.bak.\n\n";

pub fn render(file: &File) -> String {
    let body = toml::to_string(file).expect("apps file serialises");
    format!("{HEADER}{body}")
}

/// Keeps the previous file as apps.toml.bak and replaces it atomically.
pub fn save(path: &Path, file: &File) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        fs::copy(path, path.with_extension("toml.bak"))?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, render(file))?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(label: &str, send: Option<&str>, run: Option<&str>) -> Key {
        Key {
            label: label.to_owned(),
            send: send.map(str::to_owned),
            run: run.map(str::to_owned),
        }
    }

    #[test]
    fn keys_are_checked_like_the_daemon_does() {
        assert_eq!(key("tab", Some("ctrl+shift+t"), None).problem(), None);
        assert_eq!(key("back", Some("Alt + Left"), None).problem(), None);
        assert_eq!(key("~", Some("altgr+]"), None).problem(), None);
        assert_eq!(key("x", Some("f12"), None).problem(), None);
        assert_eq!(key("ping", None, Some("notify-send hi")).problem(), None);
        assert!(key("", Some("x"), None).problem().is_some());
        assert!(key("thirteen char", Some("x"), None).problem().is_some());
        assert!(key("x", Some("hyper+q"), None).problem().is_some());
        assert!(key("x", Some("ctrl+f13"), None).problem().is_some());
        assert!(key("x", Some("ctrl+"), None).problem().is_some());
        assert!(key("x", None, Some("  ")).problem().is_some());
        assert!(key("x", None, None).problem().is_some());
        assert!(key("x", Some("x"), Some("y")).problem().is_some());
    }

    #[test]
    fn recorded_key_names_are_valid_shortcuts() {
        for code in 0..256 {
            if let Some(name) = key_name(code) {
                assert_eq!(combination_problem(name), None, "{name}");
            }
        }
        assert_eq!(key_name(19), Some("r"));
        assert_eq!(key_name(42), None, "modifiers are not keys");
    }

    #[test]
    fn written_file_reads_back_the_same() {
        let file = File {
            app: vec![
                App {
                    name: "Blender".to_owned(),
                    wm_class: "blender".to_owned(),
                    key: vec![key("render", Some("f12"), None), key("q", None, Some("a 'b'"))],
                },
                App {
                    name: String::new(),
                    wm_class: "org.gnome.Ptyxis".to_owned(),
                    key: vec![key("tab", Some("ctrl+shift+t"), None)],
                },
            ],
        };
        let text = render(&file);
        assert!(text.contains("[[app.key]]"));
        assert!(!text.contains("name = \"\""));
        assert_eq!(parse(&text).unwrap(), file);
    }

    #[test]
    fn the_daemon_template_reads_as_no_apps() {
        let template = "# [[app]]\n# wm_class = \"blender\"\n";
        assert_eq!(parse(template).unwrap(), File::default());
        assert!(parse("[[app]]\nname = 3").is_err());
    }

    #[test]
    fn shortcuts_are_shown_capitalised() {
        assert_eq!(pretty_combination("ctrl+shift+t"), "Ctrl+Shift+T");
        assert_eq!(pretty_combination("alt+left"), "Alt+Left");
    }
}
