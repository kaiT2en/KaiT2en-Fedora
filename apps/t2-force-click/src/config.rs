use std::{env, fs, path::{Path, PathBuf}};

const CONFIG_VERSION: u8 = 1;

pub const CLICK_STRENGTH_LIGHT: u8 = 0;
pub const CLICK_STRENGTH_MEDIUM: u8 = 1;
pub const CLICK_STRENGTH_FIRM: u8 = 2;

pub const MIN_FORCE_CLICK_PERCENT: u32 = 120;
pub const MAX_FORCE_CLICK_PERCENT: u32 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    None,
    CopyPaste,
    KeyCombo,
    Command,
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionKind::None => "none",
            ActionKind::CopyPaste => "copy_paste",
            ActionKind::KeyCombo => "key_combo",
            ActionKind::Command => "command",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "key_combo" => ActionKind::KeyCombo,
            "copy_paste" => ActionKind::CopyPaste,
            "command" => ActionKind::Command,
            _ => ActionKind::None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub click_strength: u8,
    pub force_click_threshold_percent: u32,
    /// Keeps the plain click (click_strength) working while making the
    /// second, harder Force Click press unreachable. Needed when a user
    /// disables tap-to-click in their desktop and relies on the physical
    /// click alone: fully turning off the trackpad driver would leave them
    /// unable to click at all.
    pub force_click_disabled: bool,
    pub action_kind: ActionKind,
    /// e.g. "leftctrl+leftalt+t". Plus-separated evdev key names, held
    /// together and released, synthesized through a uinput device the
    /// daemon owns.
    pub key_combo: String,
    /// A shell command line, run via `sh -c` as the active desktop user.
    pub command: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            click_strength: CLICK_STRENGTH_MEDIUM,
            force_click_threshold_percent: 175,
            force_click_disabled: false,
            action_kind: ActionKind::None,
            key_combo: String::new(),
            command: String::new(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        for path in candidate_config_paths() {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Some(config) = parse_config(&raw) {
                    return config;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = primary_config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_disk_format())
    }

    fn to_disk_format(&self) -> String {
        format!(
            "config_version={}\nclick_strength={}\nforce_click_threshold_percent={}\nforce_click_disabled={}\naction_kind={}\nkey_combo={}\ncommand={}\n",
            CONFIG_VERSION,
            self.click_strength,
            self.force_click_threshold_percent,
            self.force_click_disabled,
            self.action_kind.as_str(),
            self.key_combo,
            self.command,
        )
    }
}

fn primary_config_path() -> PathBuf {
    env::var_os("T2_FORCE_CLICK_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/t2-force-click/config.txt"))
}

fn candidate_config_paths() -> Vec<PathBuf> {
    let mut paths = vec![primary_config_path()];
    let legacy_base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".config")));
    if let Some(base) = legacy_base {
        let legacy = base.join("t2-force-click/config.txt");
        if !paths.contains(&legacy) {
            paths.push(legacy);
        }
    }
    paths
}

fn parse_config(raw: &str) -> Option<AppConfig> {
    let mut config = AppConfig::default();
    let mut version = None;
    for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "config_version" => version = value.parse::<u8>().ok(),
            "click_strength" => config.click_strength = value.parse::<u8>().ok()?.min(2),
            "force_click_threshold_percent" => {
                config.force_click_threshold_percent = value
                    .parse::<u32>()
                    .ok()?
                    .clamp(MIN_FORCE_CLICK_PERCENT, MAX_FORCE_CLICK_PERCENT)
            }
            "force_click_disabled" => config.force_click_disabled = value.parse().ok()?,
            "action_kind" => config.action_kind = ActionKind::from_str(value),
            "key_combo" => config.key_combo = value.to_owned(),
            "command" => config.command = value.to_owned(),
            _ => {}
        }
    }
    if version != Some(CONFIG_VERSION) {
        return Some(AppConfig::default());
    }
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk_format() {
        let config = AppConfig {
            click_strength: CLICK_STRENGTH_FIRM,
            force_click_threshold_percent: 200,
            force_click_disabled: true,
            action_kind: ActionKind::KeyCombo,
            key_combo: "leftctrl+leftalt+t".to_owned(),
            command: String::new(),
        };
        let parsed = parse_config(&config.to_disk_format()).unwrap();
        assert_eq!(parsed.click_strength, CLICK_STRENGTH_FIRM);
        assert_eq!(parsed.force_click_threshold_percent, 200);
        assert!(parsed.force_click_disabled);
        assert_eq!(parsed.action_kind, ActionKind::KeyCombo);
        assert_eq!(parsed.key_combo, "leftctrl+leftalt+t");
    }

    #[test]
    fn out_of_range_percent_is_clamped() {
        let config = parse_config(
            "config_version=1\nforce_click_threshold_percent=999\n",
        )
        .unwrap();
        assert_eq!(config.force_click_threshold_percent, MAX_FORCE_CLICK_PERCENT);
    }

    #[test]
    fn unversioned_config_falls_back_to_defaults() {
        let config = parse_config("click_strength=2\n").unwrap();
        assert_eq!(config.click_strength, AppConfig::default().click_strength);
    }
}
