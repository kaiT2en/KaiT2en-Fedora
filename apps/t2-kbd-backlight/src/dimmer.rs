// SPDX-License-Identifier: GPL-3.0-or-later

//! Fades the keyboard backlight out after inactivity and back in on input.

use std::time::Instant;

use crate::kbdlight::KbdBacklight;

/// The backlight comes back quicker than it leaves.
const FADE_IN_MS: u64 = 500;
const FADE_OUT_MS: u64 = 700;
const FRAME_MS: u64 = 33;
/// UPower reports each fade step back, possibly after the next step went out,
/// so changes shortly after an own write are not taken as the user's.
const OWN_ECHO_MS: u64 = 250;

struct Fade {
    start_ms: u64,
    next_ms: u64,
    from: u32,
    to: u32,
}

pub struct Dimmer {
    kbd: KbdBacklight,
    timeout_ms: u64,
    started: Instant,
    deadline_ms: Option<u64>,
    /// Faded out by inactivity, the next input fades back to `restore`.
    dimmed: bool,
    /// The level from before the inactivity fade.
    restore: Option<u32>,
    maximum: Option<u32>,
    current: Option<u32>,
    fade: Option<Fade>,
    last_write_ms: Option<u64>,
}

impl Dimmer {
    pub fn new(kbd: KbdBacklight, timeout_ms: u64) -> Self {
        let current = match kbd.brightness() {
            Ok(brightness) => Some(brightness),
            Err(error) => {
                eprintln!("t2-kbd-backlight: read keyboard brightness: {error:#}");
                None
            }
        };
        Self {
            kbd,
            timeout_ms,
            started: Instant::now(),
            deadline_ms: Some(timeout_ms),
            dimmed: current == Some(0),
            restore: current.filter(|brightness| *brightness > 0),
            maximum: None,
            current,
            fade: None,
            last_write_ms: None,
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub fn activity(&mut self) {
        let now = self.now_ms();
        self.deadline_ms = Some(now + self.timeout_ms);
        let fading_out = self.fade.as_ref().is_some_and(|fade| fade.to == 0);
        if !self.dimmed && !fading_out {
            return;
        }
        let Some(target) = self.target() else {
            return;
        };
        if self.fade.as_ref().is_some_and(|fade| fade.to == target) {
            return;
        }
        let from = self.current.unwrap_or(0);
        self.start_fade(now, from, target);
    }

    /// The level from before the fade, or full brightness when the keyboard
    /// was already dark at start.
    fn target(&mut self) -> Option<u32> {
        if let Some(brightness) = self.restore.filter(|brightness| *brightness > 0) {
            return Some(brightness);
        }
        if self.maximum.is_none() {
            self.maximum = Some(self.kbd.maximum().unwrap_or_else(|error| {
                eprintln!("t2-kbd-backlight: read maximum keyboard brightness: {error:#}");
                0
            }));
        }
        self.maximum.filter(|brightness| *brightness > 0)
    }

    /// UPower reported a change. Turning the backlight on while it is faded
    /// out counts as input.
    pub fn changed(&mut self) {
        let now = self.now_ms();
        if self.fade.is_some()
            || self
                .last_write_ms
                .is_some_and(|written| now < written + OWN_ECHO_MS)
        {
            return;
        }
        let Ok(brightness) = self.kbd.brightness() else {
            return;
        };
        self.current = Some(brightness);
        if self.dimmed && brightness > 0 {
            self.restore = Some(brightness);
            self.dimmed = false;
            self.deadline_ms = Some(now + self.timeout_ms);
        }
    }

    pub fn tick(&mut self) {
        let now = self.now_ms();
        if self.deadline_ms.is_some_and(|deadline| now >= deadline) {
            self.idle(now);
        }
        self.advance_fade(now);
    }

    fn idle(&mut self, now: u64) {
        self.deadline_ms = None;
        match self.kbd.brightness() {
            Ok(brightness) => {
                self.current = Some(brightness);
                if brightness > 0 {
                    self.restore = Some(brightness);
                    self.start_fade(now, brightness, 0);
                } else {
                    self.fade = None;
                    self.dimmed = true;
                }
            }
            Err(error) => {
                eprintln!("t2-kbd-backlight: read keyboard brightness before dimming: {error:#}")
            }
        }
    }

    fn start_fade(&mut self, now: u64, from: u32, to: u32) {
        if from == to {
            self.current = Some(to);
            self.dimmed = to == 0;
            self.fade = None;
            return;
        }
        self.fade = Some(Fade {
            start_ms: now,
            next_ms: now,
            from,
            to,
        });
    }

    fn advance_fade(&mut self, now: u64) {
        let Some(fade) = self.fade.as_mut() else {
            return;
        };
        if now < fade.next_ms {
            return;
        }
        let duration = if fade.to > fade.from {
            FADE_IN_MS
        } else {
            FADE_OUT_MS
        };
        let elapsed = now.saturating_sub(fade.start_ms).min(duration);
        let progress = elapsed as f32 / duration as f32;
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let value = (fade.from as f32 + (fade.to as f32 - fade.from as f32) * eased).round() as u32;
        let target = fade.to;
        fade.next_ms = now + FRAME_MS;
        if self.current != Some(value) {
            if let Err(error) = self.write(now, value) {
                eprintln!("t2-kbd-backlight: fade keyboard backlight: {error:#}");
                self.fade = None;
                return;
            }
        }
        if elapsed >= duration {
            self.current = Some(target);
            self.dimmed = target == 0;
            self.fade = None;
        }
    }

    fn write(&mut self, now: u64, value: u32) -> anyhow::Result<()> {
        self.kbd.set(value)?;
        self.current = Some(value);
        self.last_write_ms = Some(now);
        Ok(())
    }

    /// The driver restores the last brightness when the keyboard comes back
    /// after resume, so the level from before the inactivity fade goes in
    /// now. The lock screen then comes back lit, and the next input starts
    /// the inactivity timer again.
    pub fn prepare_for_sleep(&mut self) {
        let target = match &self.fade {
            Some(fade) if fade.to > 0 => Some(fade.to),
            Some(_) => self.restore,
            None if self.dimmed => self.restore,
            None => None,
        };
        self.fade = None;
        self.deadline_ms = None;
        let Some(brightness) = target.filter(|brightness| *brightness > 0) else {
            return;
        };
        let now = self.now_ms();
        match self.write(now, brightness) {
            Ok(()) => self.dimmed = false,
            Err(error) => {
                eprintln!("t2-kbd-backlight: restore keyboard brightness before suspend: {error:#}")
            }
        }
    }

    /// Leaves the keyboard at the level from before the inactivity fade.
    pub fn restore_on_exit(&mut self) {
        let dark = self.dimmed || self.fade.as_ref().is_some_and(|fade| fade.to == 0);
        if dark && let Some(brightness) = self.restore {
            if let Err(error) = self.kbd.set(brightness) {
                eprintln!("t2-kbd-backlight: restore keyboard brightness on exit: {error:#}");
            }
        }
    }

    /// Milliseconds until the next deadline, -1 while there is none.
    pub fn next_timeout(&self) -> i32 {
        let now = self.now_ms();
        [
            self.deadline_ms,
            self.fade.as_ref().map(|fade| fade.next_ms),
        ]
        .into_iter()
        .flatten()
        .min()
        .map(|deadline| deadline.saturating_sub(now).min(i32::MAX as u64) as i32)
        .unwrap_or(-1)
    }
}
