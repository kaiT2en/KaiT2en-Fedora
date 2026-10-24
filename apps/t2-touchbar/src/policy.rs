// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Media,
    Function,
    /// Keys a MacBook keyboard lacks or hides: print, insert, delete, ...
    Special,
    /// Keys of the focused app's profile from apps.toml. Part of the ring
    /// only while such an app has the focus.
    App,
}

impl Mode {
    /// Fn hold switches between the media row and `alternate`, the other
    /// row that was used last.
    pub fn toggled(self, alternate: Self) -> Self {
        match self {
            Self::Media if alternate != Self::Media => alternate,
            Self::Media => Self::Function,
            Self::Function | Self::Special | Self::App => Self::Media,
        }
    }

    /// The rows form a ring, so every row is one swipe away. The app row
    /// joins the ring after the special row while `app` is true.
    pub fn next(self, app: bool) -> Self {
        match self {
            Self::Media => Self::Function,
            Self::Function => Self::Special,
            Self::Special if app => Self::App,
            Self::Special | Self::App => Self::Media,
        }
    }

    pub fn previous(self, app: bool) -> Self {
        match self {
            Self::Media if app => Self::App,
            Self::Media => Self::Special,
            Self::Function => Self::Media,
            Self::Special => Self::Function,
            Self::App => Self::Special,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct LearnedTimeout {
    pub timeout_ms: u64,
    /// Number of sessions in which the learned extension was visibly unused.
    pub unused_extensions: u8,
}

impl Default for LearnedTimeout {
    fn default() -> Self {
        Self {
            timeout_ms: 5_000,
            unused_extensions: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PersistentState {
    pub mode: Mode,
    /// The last row other than media keys, the target of an Fn hold.
    pub alternate: Mode,
    pub media: LearnedTimeout,
    pub function: LearnedTimeout,
    pub special: LearnedTimeout,
    pub app: LearnedTimeout,
}

impl PersistentState {
    pub fn load(path: &Path, config: &Config) -> Self {
        let mut state: Self = fs::read_to_string(path)
            .ok()
            .and_then(|body| toml::from_str(&body).ok())
            .unwrap_or_default();
        state.clamp(config);
        state
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create state directory {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, body).with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
        Ok(())
    }

    pub fn reset(config: &Config) -> Self {
        let mut state = Self::default();
        state.clamp(config);
        state
    }

    pub fn learned(&self, mode: Mode) -> &LearnedTimeout {
        match mode {
            Mode::Media => &self.media,
            Mode::Function => &self.function,
            Mode::Special => &self.special,
            Mode::App => &self.app,
        }
    }

    pub fn learned_mut(&mut self, mode: Mode) -> &mut LearnedTimeout {
        match mode {
            Mode::Media => &mut self.media,
            Mode::Function => &mut self.function,
            Mode::Special => &mut self.special,
            Mode::App => &mut self.app,
        }
    }

    /// Switches rows and remembers the last non-media one.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        // The app row comes and goes with the focus, so an Fn hold never
        // targets it.
        if !matches!(mode, Mode::Media | Mode::App) {
            self.alternate = mode;
        }
    }

    fn clamp(&mut self, config: &Config) {
        for learned in [
            &mut self.media,
            &mut self.function,
            &mut self.special,
            &mut self.app,
        ] {
            learned.timeout_ms = learned
                .timeout_ms
                .clamp(config.minimum_timeout_ms, config.maximum_timeout_ms);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct OffRecord {
    mode: Mode,
    reference_ms: u64,
    off_ms: u64,
    extension_unused: bool,
}

/// Tiny deterministic learner for the only uncertain value in v1: how long a
/// chosen layer should remain visible. A quick return followed by a real key is
/// strong evidence; silence is deliberately weak evidence.
pub struct TimeoutLearner {
    pending: Option<OffRecord>,
    wake_after_off_ms: Option<u64>,
    last_meaningful_ms: Option<u64>,
    used_after_minimum: bool,
}

impl TimeoutLearner {
    pub fn new() -> Self {
        Self {
            pending: None,
            wake_after_off_ms: None,
            last_meaningful_ms: None,
            used_after_minimum: false,
        }
    }

    pub fn wake(&mut self, now_ms: u64, config: &Config) {
        if self.pending.is_some_and(|record| {
            now_ms.saturating_sub(record.off_ms) <= config.continuation_window_ms
        }) {
            self.wake_after_off_ms = Some(now_ms);
        } else {
            self.pending = None;
            self.wake_after_off_ms = None;
        }
        self.last_meaningful_ms = Some(now_ms);
        self.used_after_minimum = false;
    }

    /// Returns true when persistent state changed.
    pub fn action(
        &mut self,
        now_ms: u64,
        mode: Mode,
        state: &mut PersistentState,
        config: &Config,
    ) -> bool {
        let mut changed = false;
        if let (Some(record), Some(wake_ms)) = (self.pending, self.wake_after_off_ms)
            && record.mode == mode
            && now_ms.saturating_sub(wake_ms) <= 2_000
        {
            let observed = now_ms
                .saturating_sub(record.reference_ms)
                .saturating_add(1_000);
            let learned = state.learned_mut(mode);
            let target = learned
                .timeout_ms
                .saturating_add(2_000)
                .max(observed)
                .min(config.maximum_timeout_ms);
            if target != learned.timeout_ms || learned.unused_extensions != 0 {
                learned.timeout_ms = target;
                learned.unused_extensions = 0;
                changed = true;
            }
            self.pending = None;
            self.wake_after_off_ms = None;
        }

        if let Some(start) = self.last_meaningful_ms
            && now_ms.saturating_sub(start) >= config.minimum_timeout_ms
        {
            self.used_after_minimum = true;
        }
        self.last_meaningful_ms = Some(now_ms);
        changed
    }

    pub fn auto_off(&mut self, now_ms: u64, mode: Mode, config: &Config) {
        let reference_ms = self.last_meaningful_ms.unwrap_or(now_ms);
        let extension_unused =
            state_extension_exists(mode, config, now_ms, reference_ms) && !self.used_after_minimum;
        self.pending = Some(OffRecord {
            mode,
            reference_ms,
            off_ms: now_ms,
            extension_unused,
        });
        self.wake_after_off_ms = None;
    }

    /// Finalise weak negative evidence once the continuation window passed.
    pub fn settle(&mut self, now_ms: u64, state: &mut PersistentState, config: &Config) -> bool {
        let Some(record) = self.pending else {
            return false;
        };
        if now_ms.saturating_sub(record.off_ms) <= config.continuation_window_ms {
            return false;
        }
        self.pending = None;
        self.wake_after_off_ms = None;
        if !record.extension_unused {
            return false;
        }
        let learned = state.learned_mut(record.mode);
        learned.unused_extensions = learned.unused_extensions.saturating_add(1);
        // Require two quiet sessions: absence is weaker evidence than a quick
        // return, and should not cause 5/10 second oscillation.
        if learned.unused_extensions < 2 {
            return true;
        }
        learned.unused_extensions = 0;
        let excess = learned.timeout_ms.saturating_sub(config.minimum_timeout_ms);
        let step = (excess / 5).max(1_000).min(excess);
        learned.timeout_ms = learned
            .timeout_ms
            .saturating_sub(step)
            .max(config.minimum_timeout_ms);
        true
    }

    pub fn next_settle_ms(&self, config: &Config) -> Option<u64> {
        self.pending.map(|record| {
            record
                .off_ms
                .saturating_add(config.continuation_window_ms + 1)
        })
    }
}

fn state_extension_exists(_mode: Mode, config: &Config, now_ms: u64, reference_ms: u64) -> bool {
    now_ms.saturating_sub(reference_ms) > config.minimum_timeout_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fn_hold_returns_to_the_last_used_row() {
        let mut state = PersistentState::default();
        assert_eq!(state.mode.toggled(state.alternate), Mode::Function);
        state.set_mode(Mode::Special);
        state.set_mode(Mode::Media);
        assert_eq!(state.mode.toggled(state.alternate), Mode::Special);
        state.set_mode(Mode::Special);
        assert_eq!(state.mode.toggled(state.alternate), Mode::Media);
        // The app row is never the Fn hold target.
        state.set_mode(Mode::App);
        assert_eq!(state.alternate, Mode::Special);
        assert_eq!(state.mode.toggled(state.alternate), Mode::Media);
    }

    #[test]
    fn app_row_joins_the_ring_only_with_a_profile() {
        assert_eq!(Mode::Special.next(false), Mode::Media);
        assert_eq!(Mode::Media.previous(false), Mode::Special);
        assert_eq!(Mode::Special.next(true), Mode::App);
        assert_eq!(Mode::App.next(true), Mode::Media);
        assert_eq!(Mode::Media.previous(true), Mode::App);
        assert_eq!(Mode::App.previous(true), Mode::Special);
    }

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn quick_return_grows_timeout() {
        let cfg = config();
        let mut state = PersistentState::reset(&cfg);
        let mut learner = TimeoutLearner::new();
        learner.wake(0, &cfg);
        learner.auto_off(5_000, Mode::Media, &cfg);
        learner.wake(10_000, &cfg);
        assert!(learner.action(10_100, Mode::Media, &mut state, &cfg));
        assert_eq!(state.media.timeout_ms, 11_100);
    }

    #[test]
    fn wake_without_action_is_not_positive_evidence() {
        let cfg = config();
        let mut state = PersistentState::reset(&cfg);
        let mut learner = TimeoutLearner::new();
        learner.wake(0, &cfg);
        learner.auto_off(5_000, Mode::Media, &cfg);
        learner.wake(6_000, &cfg);
        learner.settle(30_000, &mut state, &cfg);
        assert_eq!(state.media.timeout_ms, 5_000);
    }

    #[test]
    fn quiet_extensions_decay_slowly() {
        let cfg = config();
        let mut state = PersistentState::reset(&cfg);
        state.media.timeout_ms = 15_000;
        let mut learner = TimeoutLearner::new();
        for base in [0, 40_000] {
            learner.wake(base, &cfg);
            learner.auto_off(base + 15_000, Mode::Media, &cfg);
            learner.settle(base + 31_000, &mut state, &cfg);
        }
        assert_eq!(state.media.timeout_ms, 13_000);
    }

    #[test]
    fn timeouts_are_independent() {
        let cfg = config();
        let mut state = PersistentState::reset(&cfg);
        state.media.timeout_ms = 12_000;
        assert_eq!(state.function.timeout_ms, 5_000);
    }
}
