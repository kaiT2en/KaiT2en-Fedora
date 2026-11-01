// SPDX-License-Identifier: GPL-3.0-or-later
//! Client of the root helper t2-quickshare-hotspot: a Wi-Fi hotspot for the
//! bandwidth upgrade of a Quick Share transfer that started over
//! Bluetooth. The helper stops the hotspot when this connection to the
//! system bus closes, so dropping a `Hotspot` stops it as well.

use std::time::Duration;

use anyhow::{Context, Result};
use zbus::{Connection, proxy};

#[proxy(
    interface = "org.t2airshare.QuickShareHotspot1",
    default_service = "org.t2airshare.QuickShareHotspot",
    default_path = "/org/t2airshare/QuickShareHotspot"
)]
trait QuickShareHotspot1 {
    fn start(&self, port: u16) -> zbus::Result<(String, String, String, i32)>;
    fn stop(&self) -> zbus::Result<()>;
}

pub struct Hotspot {
    _connection: Connection,
    pub ssid: String,
    pub password: String,
    pub gateway: String,
    pub frequency: i32,
}

impl Hotspot {
    /// Starts the hotspot with TCP `port` open in it.
    pub async fn start(port: u16) -> Result<Self> {
        // bringing the access point up takes a few seconds
        let connection = zbus::connection::Builder::system()?
            .method_timeout(Duration::from_secs(60))
            .build()
            .await
            .context("system bus")?;
        let (ssid, password, gateway, frequency) = QuickShareHotspot1Proxy::new(&connection)
            .await?
            .start(port)
            .await
            .context("start the hotspot")?;
        Ok(Self { _connection: connection, ssid, password, gateway, frequency })
    }
}
