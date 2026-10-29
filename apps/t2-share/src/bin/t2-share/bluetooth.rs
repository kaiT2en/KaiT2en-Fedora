// SPDX-License-Identifier: GPL-3.0-or-later
//! Bluetooth power through BlueZ, for the hint in the send window. Phones
//! and Apple devices only start Quick Share and AirDrop after a Bluetooth
//! LE request, so sending needs Bluetooth on. As on macOS it is switched
//! on only when the user asks for it.

use std::collections::HashMap;

use anyhow::{Context, Result};
use zbus::proxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[proxy(interface = "org.freedesktop.DBus.ObjectManager", default_service = "org.bluez", default_path = "/")]
trait BluezObjects {
    fn get_managed_objects(
        &self,
    ) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>;
}

/// The first adapter and whether it is powered, None without an adapter.
async fn adapter(connection: &zbus::Connection) -> Result<Option<(OwnedObjectPath, bool)>> {
    let objects = BluezObjectsProxy::new(connection).await?.get_managed_objects().await?;
    Ok(objects.into_iter().find_map(|(path, interfaces)| {
        let adapter = interfaces.get("org.bluez.Adapter1")?;
        let powered = adapter.get("Powered").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
        Some((path, powered))
    }))
}

/// Whether Bluetooth is on, None when there is no adapter or no BlueZ.
pub async fn powered() -> Option<bool> {
    let connection = zbus::Connection::system().await.ok()?;
    adapter(&connection).await.ok().flatten().map(|(_, powered)| powered)
}

/// Switches Bluetooth on. A soft blocked adapter is unblocked first.
pub async fn power_on() -> Result<()> {
    let connection = zbus::Connection::system().await.context("system bus")?;
    let (path, _) = adapter(&connection).await?.context("no Bluetooth adapter")?;
    let props = zbus::fdo::PropertiesProxy::builder(&connection)
        .destination("org.bluez")?
        .path(path)?
        .build()
        .await?;
    let iface = zbus::names::InterfaceName::try_from("org.bluez.Adapter1")?;
    if let Err(err) = props.set(iface.clone(), "Powered", Value::from(true)).await {
        // org.bluez.Error.Blocked: switched off through rfkill
        let unblocked = tokio::task::spawn_blocking(|| {
            std::process::Command::new("rfkill").args(["unblock", "bluetooth"]).status()
        })
        .await
        .is_ok_and(|s| s.is_ok_and(|s| s.success()));
        if !unblocked {
            return Err(err).context("switch Bluetooth on");
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        props.set(iface, "Powered", Value::from(true)).await.context("switch Bluetooth on")?;
    }
    Ok(())
}
