// SPDX-License-Identifier: GPL-3.0-or-later

//! Keyboard backlight level, so the Touch Bar glyphs can match the keys.

use std::{fs, io::Write, os::unix::net::UnixStream, path::PathBuf, thread, time::Duration};

use anyhow::{Context, Result};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message::Type};

/// Keys never get darker than this share of full luminance, so the bar stays
/// readable while the keyboard backlight is off.
const MINIMUM_LUMINANCE: f32 = 0.08;
const DISPLAY_GAMMA: f32 = 2.2;

/// Glyph intensity in percent of the configured key color. The LED output is
/// linear, the panel follows a gamma curve, so the share is gamma-encoded.
pub fn key_level() -> Option<u32> {
    let led = led()?;
    let read = |name: &str| -> Option<f32> {
        fs::read_to_string(led.join(name)).ok()?.trim().parse().ok()
    };
    let max = read("max_brightness").filter(|max| *max > 0.0)?;
    let luminance = (read("brightness")? / max).clamp(MINIMUM_LUMINANCE, 1.0);
    Some((luminance.powf(1.0 / DISPLAY_GAMMA) * 100.0).round() as u32)
}

fn led() -> Option<PathBuf> {
    fs::read_dir("/sys/class/leds")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("kbd_backlight"))
        })
}

/// Wakes the main loop whenever UPower reports a keyboard backlight change.
pub fn watch() -> Result<UnixStream> {
    let (mut wake_writer, wake_reader) = UnixStream::pair()?;
    thread::Builder::new()
        .name("kbd-backlight-signals".to_owned())
        .spawn(move || {
            let mut reported = false;
            loop {
                if let Err(error) = listen(&mut wake_writer) {
                    // Without UPower the level is still read on every wake.
                    if !reported {
                        eprintln!("kait2en-touchbar: keyboard backlight listener: {error:#}");
                        reported = true;
                    }
                    thread::sleep(Duration::from_secs(30));
                }
            }
        })?;
    Ok(wake_reader)
}

fn listen(wake: &mut UnixStream) -> Result<()> {
    let connection = Connection::system().context("connect system bus")?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.UPower")?
        .path("/org/freedesktop/UPower/KbdBacklight")?
        .interface("org.freedesktop.UPower.KbdBacklight")?
        .member("BrightnessChanged")?
        .build();
    let iterator = MessageIterator::for_match_rule(rule, &connection, None)?;
    for message in iterator {
        message?;
        let _ = wake.write_all(&[1]);
    }
    Err(anyhow::anyhow!("system bus iterator ended"))
}
