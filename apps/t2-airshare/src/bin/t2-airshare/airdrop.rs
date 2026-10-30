// SPDX-License-Identifier: GPL-3.0-or-later
//! AirDrop receivers through the t2-drop service (org.t2drop on the system
//! bus). t2-airshare only talks to it over D-Bus, the AWDL and AirDrop code
//! lives in t2-drop.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use zbus::proxy;
use zbus::zvariant::OwnedFd;

#[proxy(interface = "org.t2drop.Sender1", default_service = "org.t2drop", default_path = "/org/t2drop")]
trait Sender1 {
    fn browse(&self) -> zbus::Result<()>;
    fn receivers(&self) -> zbus::Result<Vec<(String, String)>>;
    fn send(&self, id: &str, names: Vec<String>, fds: Vec<OwnedFd>, sender_name: &str) -> zbus::Result<()>;
}

/// Whether the t2-drop service runs or can be started on this system.
pub async fn available(connection: &zbus::Connection) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(connection).await else { return false };
    let Ok(name) = zbus::names::BusName::try_from("org.t2drop") else { return false };
    if dbus.name_has_owner(name).await.unwrap_or(false) {
        return true;
    }
    dbus.list_activatable_names()
        .await
        .map(|names| names.iter().any(|n| n.as_str() == "org.t2drop"))
        .unwrap_or(false)
}

/// What the AirDrop browse reports to the send window.
pub enum Update {
    /// The receivers found, by id
    Receivers(HashMap<String, String>),
    /// The service cannot look for receivers while Bluetooth is off
    BluetoothOff,
    Failed(String),
}

fn bluetooth_off(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<zbus::Error>(),
        Some(zbus::Error::MethodError(name, _, _)) if name.as_str() == "org.t2drop.Error.BluetoothOff"
    )
}

/// Browses for AirDrop receivers and reports every change. Ends when
/// `report` returns false. Waits for the service while it is missing and
/// browses again after it restarted or failed. Stopping the task ends the
/// browse, the service notices the caller leaving the bus.
pub async fn browse(mut report: impl FnMut(Update) -> bool) -> Result<()> {
    let connection = zbus::Connection::system().await.context("system bus")?;
    let mut known = HashMap::new();
    let mut last_error = String::new();
    loop {
        if available(&connection).await {
            match browse_once(&connection, &mut known, &mut report).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    let update = if bluetooth_off(&err) {
                        Update::BluetoothOff
                    } else {
                        Update::Failed(format!("{err:#}"))
                    };
                    let text = format!("{err:#}");
                    if text != last_error {
                        last_error = text;
                        if !report(update) {
                            return Ok(());
                        }
                    }
                }
            }
            if !known.is_empty() {
                known.clear();
                if !report(Update::Receivers(known.clone())) {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn browse_once(
    connection: &zbus::Connection,
    known: &mut HashMap<String, String>,
    report: &mut impl FnMut(Update) -> bool,
) -> Result<()> {
    let sender = Sender1Proxy::new(connection).await?;
    sender.browse().await?;
    loop {
        let list: HashMap<String, String> = sender.receivers().await?.into_iter().collect();
        if &list != known {
            *known = list;
            if !report(Update::Receivers(known.clone())) {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Sends `files` to the receiver `id` and returns once it has them.
pub async fn send(id: &str, files: &[PathBuf], sender_name: &str) -> Result<()> {
    let mut names = Vec::new();
    let mut fds = Vec::new();
    for path in files {
        let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        names.push(
            path.file_name()
                .and_then(|n| n.to_str())
                .with_context(|| format!("no file name in {}", path.display()))?
                .to_owned(),
        );
        fds.push(OwnedFd::from(std::os::fd::OwnedFd::from(file)));
    }
    let connection = zbus::Connection::system().await.context("system bus")?;
    let sender = Sender1Proxy::new(&connection).await?;
    // The service only sends while someone browses, this caller included.
    sender.browse().await?;
    sender.send(id, names, fds, sender_name).await?;
    Ok(())
}
