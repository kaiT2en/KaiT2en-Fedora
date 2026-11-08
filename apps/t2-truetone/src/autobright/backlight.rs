// SPDX-License-Identifier: GPL-3.0-or-later
//! The built-in panel backlight through mutter.
//!
//! gnome-shell takes every backlight change it did not make itself into its
//! slider, as long as its own automatic target is off. Writing through mutter
//! therefore keeps the slider and the brightness keys on the real value.

use std::collections::HashMap;

use anyhow::{Context, Result};
use zbus::blocking::{Connection, Proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

#[derive(Clone, Debug)]
pub struct Backlight {
    pub serial: u32,
    pub connector: String,
    pub min: i32,
    pub max: i32,
}

fn proxy(connection: &Connection) -> Result<Proxy<'_>> {
    Ok(zbus::blocking::proxy::Builder::new(connection)
        .destination("org.gnome.Mutter.DisplayConfig")?
        .path("/org/gnome/Mutter/DisplayConfig")?
        .interface("org.gnome.Mutter.DisplayConfig")?
        .cache_properties(CacheProperties::No)
        .build()?)
}

/// `None` when mutter controls no backlight, as on a desktop Mac.
pub fn read(connection: &Connection) -> Result<Option<Backlight>> {
    let (serial, outputs): (u32, Vec<HashMap<String, OwnedValue>>) =
        proxy(connection)?.get_property("Backlight").context("reading mutter Backlight")?;
    let backlights: Vec<Backlight> = outputs
        .into_iter()
        .filter(|output| output.get("active").and_then(|v| bool::try_from(v).ok()).unwrap_or(false))
        .filter_map(|output| {
            let int = |key: &str| output.get(key).and_then(|v| i32::try_from(v).ok());
            Some(Backlight {
                serial,
                connector: output.get("connector").and_then(|v| v.try_clone().ok()).and_then(|v| String::try_from(v).ok())?,
                min: int("min")?,
                max: int("max")?,
            })
        })
        .collect();
    let builtin = backlights.iter().position(|backlight| backlight.connector.starts_with("eDP")).unwrap_or(0);
    Ok(backlights.into_iter().nth(builtin))
}

pub fn set(connection: &Connection, backlight: &Backlight, value: i32) -> Result<()> {
    proxy(connection)?
        .call_method("SetBacklight", &(backlight.serial, backlight.connector.as_str(), value))
        .context("mutter SetBacklight")?;
    Ok(())
}
