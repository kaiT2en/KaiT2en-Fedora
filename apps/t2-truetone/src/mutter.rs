// SPDX-License-Identifier: GPL-3.0-or-later
//! org.gnome.Mutter.DisplayConfig: gamma LUT of the built-in panel.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

type Props = HashMap<String, OwnedValue>;
type Crtc = (u32, i64, i32, i32, i32, i32, i32, u32, Vec<u32>, Props);
type Output = (u32, i64, i32, Vec<u32>, String, Vec<u32>, Vec<u32>, Props);
type Mode = (u32, i64, u32, u32, f64, u32);

#[zbus::proxy(
    interface = "org.gnome.Mutter.DisplayConfig",
    default_service = "org.gnome.Mutter.DisplayConfig",
    default_path = "/org/gnome/Mutter/DisplayConfig",
    gen_async = false,
    gen_blocking = true
)]
pub trait DisplayConfig {
    #[allow(clippy::type_complexity)]
    fn get_resources(&self) -> zbus::Result<(u32, Vec<Crtc>, Vec<Output>, Vec<Mode>, i32, i32)>;
    fn get_crtc_gamma(&self, serial: u32, crtc: u32) -> zbus::Result<(Vec<u16>, Vec<u16>, Vec<u16>)>;
    fn set_crtc_gamma(&self, serial: u32, crtc: u32, red: &[u16], green: &[u16], blue: &[u16]) -> zbus::Result<()>;
    #[zbus(signal)]
    fn monitors_changed(&self) -> zbus::Result<()>;
}

#[derive(Clone, Debug)]
pub struct Panel {
    pub serial: u32,
    pub crtc: u32,
    pub connector: String,
}

pub struct Mutter {
    proxy: DisplayConfigProxy<'static>,
}

impl Mutter {
    pub fn connect() -> Result<Self> {
        let conn = Connection::session().context("connecting to the session bus")?;
        let proxy = DisplayConfigProxy::builder(&conn)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .context("DisplayConfig proxy")?;
        Ok(Mutter { proxy })
    }

    pub fn proxy(&self) -> &DisplayConfigProxy<'static> {
        &self.proxy
    }

    /// The built-in eDP panel and the CRTC driving it.
    pub fn panel(&self) -> Result<Panel> {
        let (serial, _crtcs, outputs, _, _, _) = self.proxy.get_resources()?;
        for o in outputs {
            let builtin = o.7.get("connector-type").and_then(|v| String::try_from(v.clone()).ok())
                == Some("eDP".into())
                || o.4.starts_with("eDP");
            if builtin && o.2 >= 0 {
                return Ok(Panel { serial, crtc: o.2 as u32, connector: o.4 });
            }
        }
        Err(anyhow!("no active built-in panel"))
    }

    pub fn gamma_len(&self, p: &Panel) -> Result<usize> {
        Ok(self.proxy.get_crtc_gamma(p.serial, p.crtc)?.0.len())
    }

    pub fn get_gamma(&self, p: &Panel) -> Result<[Vec<u16>; 3]> {
        let (r, g, b) = self.proxy.get_crtc_gamma(p.serial, p.crtc)?;
        Ok([r, g, b])
    }

    pub fn set_gamma(&self, p: &Panel, lut: &[Vec<u16>; 3]) -> Result<()> {
        self.proxy.set_crtc_gamma(p.serial, p.crtc, &lut[0], &lut[1], &lut[2])?;
        Ok(())
    }
}
