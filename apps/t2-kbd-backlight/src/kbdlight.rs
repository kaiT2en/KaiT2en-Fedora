// SPDX-License-Identifier: GPL-3.0-or-later

//! Keyboard backlight through UPower, so GNOME keeps seeing the level it sets.

use std::{io::Write, os::unix::net::UnixStream, thread, time::Duration};

use anyhow::{Context, Result};
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::{MatchRule, message::Type};

const SERVICE: &str = "org.freedesktop.UPower";
const PATH: &str = "/org/freedesktop/UPower/KbdBacklight";
const INTERFACE: &str = "org.freedesktop.UPower.KbdBacklight";

pub struct KbdBacklight {
    proxy: Proxy<'static>,
}

impl KbdBacklight {
    pub fn connect() -> Result<Self> {
        let connection = Connection::system().context("connect to the system bus")?;
        let proxy = Proxy::new(&connection, SERVICE, PATH, INTERFACE)?;
        Ok(Self { proxy })
    }

    pub fn brightness(&self) -> Result<u32> {
        let brightness: i32 = self.proxy.call("GetBrightness", &())?;
        u32::try_from(brightness).context("UPower returned a negative keyboard brightness")
    }

    pub fn maximum(&self) -> Result<u32> {
        let maximum: i32 = self.proxy.call("GetMaxBrightness", &())?;
        u32::try_from(maximum).context("UPower returned a negative maximum keyboard brightness")
    }

    pub fn set(&self, brightness: u32) -> Result<()> {
        let brightness = i32::try_from(brightness).context("keyboard brightness is too large")?;
        let _: () = self.proxy.call("SetBrightness", &(brightness))?;
        Ok(())
    }
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
                    if !reported {
                        eprintln!("t2-kbd-backlight: keyboard backlight listener: {error:#}");
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
        .sender(SERVICE)?
        .path(PATH)?
        .interface(INTERFACE)?
        .member("BrightnessChanged")?
        .build();
    let iterator = MessageIterator::for_match_rule(rule, &connection, None)?;
    for message in iterator {
        message?;
        let _ = wake.write_all(&[1]);
    }
    Err(anyhow::anyhow!("system bus iterator ended"))
}
