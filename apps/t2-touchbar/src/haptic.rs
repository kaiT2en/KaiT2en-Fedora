// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::mpsc::{self, Sender},
    thread,
    time::{Duration, Instant},
};

const PLAY: &str = "/sys/module/t2_trackpad_actuator/parameters/play";
const MIN_INTERVAL: Duration = Duration::from_millis(40);
/// A key's release follows its press at least this late, so even a tap
/// that is only resolved on lift feels like going down and coming back up.
const RELEASE_GAP: Duration = Duration::from_millis(80);

/// Plays the captured actuator waveforms on a worker thread, which holds a
/// release back until it is due without stalling touch handling.
pub struct Haptic {
    worker: Option<Sender<(&'static [u8], Instant)>>,
    touch_up_enabled: bool,
    last: Option<Instant>,
    last_press: Option<Instant>,
}

impl Haptic {
    pub fn open(enabled: bool, touch_up_enabled: bool) -> Self {
        let file = enabled
            .then(|| OpenOptions::new().write(true).open(Path::new(PLAY)).ok())
            .flatten();
        if enabled && file.is_none() {
            eprintln!("kait2en-touchbar: haptic actuator is unavailable");
        }
        let worker = file.and_then(|file| spawn(file).ok());
        Self {
            worker,
            touch_up_enabled,
            last: None,
            last_press: None,
        }
    }

    pub fn click(&mut self) {
        if self.last.is_some_and(|last| last.elapsed() < MIN_INTERVAL) {
            return;
        }
        self.play(b"light\n", Instant::now());
    }

    /// The strongest captured click, for running into a stop. It is never
    /// rate limited, so it also lands right after a light click.
    pub fn firm(&mut self) {
        self.play(b"firm\n", Instant::now());
    }

    /// A key going down: the light click.
    pub fn key_press(&mut self) {
        self.last_press = Some(Instant::now());
        self.play(b"light\n", Instant::now());
    }

    /// A key coming back up: the firm click, never sooner than
    /// `RELEASE_GAP` after the press.
    pub fn key_release(&mut self) {
        if !self.touch_up_enabled {
            return;
        }
        let due = self
            .last_press
            .take()
            .map_or_else(Instant::now, |press| press + RELEASE_GAP)
            .max(Instant::now());
        self.play(b"firm\n", due);
    }

    fn play(&mut self, waveform: &'static [u8], due: Instant) {
        let Some(worker) = self.worker.as_ref() else {
            return;
        };
        if worker.send((waveform, due)).is_ok() {
            self.last = Some(due);
        }
    }
}

fn spawn(mut file: File) -> std::io::Result<Sender<(&'static [u8], Instant)>> {
    let (sender, jobs) = mpsc::channel::<(&'static [u8], Instant)>();
    thread::Builder::new()
        .name("haptic".to_owned())
        .spawn(move || {
            while let Ok((waveform, due)) = jobs.recv() {
                thread::sleep(due.saturating_duration_since(Instant::now()));
                let _ = file
                    .seek(SeekFrom::Start(0))
                    .and_then(|_| file.write_all(waveform));
            }
        })?;
    Ok(sender)
}
