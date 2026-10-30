// SPDX-License-Identifier: GPL-3.0-or-later
//! AirDrop receiving through the t2-drop service (org.t2drop on the system
//! bus). The service registers this process as its agent: t2-drop finds
//! the AirDrop senders nearby and runs the transfer, asks here whether to
//! accept and hands over the files as file descriptors. They are saved
//! like Quick Share transfers, into the download folder with the user's
//! rights. t2-airshare only talks to t2-drop over D-Bus.

use std::path::PathBuf;
use std::time::Duration;

use log::{info, warn};
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedFd};
use zbus::{Connection, fdo, interface, proxy};

use crate::config::Config;
use crate::receive::{Outcome, unique_path};
use crate::service::{ask_user, notify_info, report};

const AGENT_PATH: &str = "/org/t2airshare/airdrop";
const T2DROP: &str = "org.t2drop";
/// How often the registration is checked, t2-drop forgets the agent when
/// it restarts.
const CHECK: Duration = Duration::from_secs(10);

#[proxy(interface = "org.t2drop.Receiver1", default_service = "org.t2drop", default_path = "/org/t2drop")]
trait Receiver1 {
    fn register_agent(&self, path: &ObjectPath<'_>, name: &str) -> zbus::Result<()>;
}

struct Agent;

/// Only t2-drop may call the agent.
async fn check_caller(connection: &Connection, header: &Header<'_>) -> fdo::Result<()> {
    let name = zbus::names::BusName::try_from(T2DROP).map_err(zbus::Error::from)?;
    let owner = fdo::DBusProxy::new(connection).await?.get_name_owner(name).await?;
    if header.sender().map(|s| s.as_str()) != Some(owner.as_str()) {
        return Err(fdo::Error::AccessDenied("only org.t2drop may call the agent".into()));
    }
    Ok(())
}

#[interface(name = "org.t2drop.Agent1")]
impl Agent {
    /// Whether to accept `files` from `sender_name`, asks like Quick Share.
    async fn ask(
        &self,
        id: String,
        sender_name: String,
        files: Vec<String>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<bool> {
        check_caller(connection, &header).await?;
        info!("AirDrop {id} from {sender_name:?}: {files:?}");
        let body = format!("{}\nAirDrop", files.join("\n"));
        let sender = Some(sender_name).filter(|n| !n.is_empty());
        Ok(ask_user(sender, body).await)
    }

    /// Saves the accepted files into the download folder.
    async fn deliver(
        &self,
        id: String,
        sender_name: String,
        names: Vec<String>,
        fds: Vec<OwnedFd>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        check_caller(connection, &header).await?;
        if names.len() != fds.len() {
            return Err(fdo::Error::InvalidArgs("one name per file descriptor".into()));
        }
        let saved = tokio::task::spawn_blocking(move || save(names, fds))
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        match saved {
            Ok(files) => {
                info!("AirDrop {id} from {sender_name:?} saved {files:?}");
                report(Outcome::Received { files, texts: Vec::new() }).await;
                Ok(())
            }
            Err(err) => {
                warn!("AirDrop {id}: {err:#}");
                notify_info("Transfer failed", &format!("{err:#}")).await;
                Err(fdo::Error::Failed(format!("{err:#}")))
            }
        }
    }
}

/// Copies the received files into the download folder, next to existing
/// ones. Folders of the sender are not kept, only the file names.
fn save(names: Vec<String>, fds: Vec<OwnedFd>) -> anyhow::Result<Vec<PathBuf>> {
    let config = Config::load().unwrap_or_default();
    let dir = config.download_dir().ok_or_else(|| anyhow::anyhow!("no download directory"))?;
    std::fs::create_dir_all(&dir)?;
    let mut saved = Vec::new();
    for (name, fd) in names.into_iter().zip(fds) {
        let path = unique_path(&dir, &name);
        let mut source = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        let mut target = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        std::io::copy(&mut source, &mut target)?;
        saved.push(path);
    }
    Ok(saved)
}

/// Whether t2-drop runs or can be started on this system.
async fn t2drop_available(connection: &Connection) -> bool {
    let Ok(dbus) = fdo::DBusProxy::new(connection).await else { return false };
    let Ok(name) = zbus::names::BusName::try_from(T2DROP) else { return false };
    if dbus.name_has_owner(name).await.unwrap_or(false) {
        return true;
    }
    dbus.list_activatable_names()
        .await
        .map(|names| names.iter().any(|n| n.as_str() == T2DROP))
        .unwrap_or(false)
}

async fn t2drop_owner(connection: &Connection) -> Option<String> {
    let name = zbus::names::BusName::try_from(T2DROP).ok()?;
    let owner = fdo::DBusProxy::new(connection).await.ok()?.get_name_owner(name).await.ok()?;
    Some(owner.to_string())
}

/// Serves the agent and keeps it registered with t2-drop under `name`, the
/// name shown on the senders. Returns at once without t2-drop.
pub async fn run(name: String) {
    let connection = match zbus::connection::Builder::system() {
        Ok(builder) => match builder.serve_at(AGENT_PATH, Agent) {
            Ok(builder) => builder.build().await,
            Err(err) => Err(err),
        },
        Err(err) => Err(err),
    };
    let connection = match connection {
        Ok(connection) => connection,
        Err(err) => {
            warn!("AirDrop: system bus: {err}");
            return;
        }
    };
    if !t2drop_available(&connection).await {
        info!("AirDrop: t2-drop is not installed");
        return;
    }
    let mut registered: Option<String> = None;
    let mut last_error = String::new();
    loop {
        let owner = t2drop_owner(&connection).await;
        if owner.is_none() || owner != registered {
            let path = ObjectPath::try_from(AGENT_PATH).expect("valid path");
            let result = match Receiver1Proxy::new(&connection).await {
                Ok(receiver) => receiver.register_agent(&path, &name).await,
                Err(err) => Err(err),
            };
            match result {
                Ok(()) => {
                    // Registering starts t2-drop when it does not run
                    registered = t2drop_owner(&connection).await;
                    last_error.clear();
                    info!("AirDrop: receiving as {name:?}");
                }
                Err(err) => {
                    let text = err.to_string();
                    if text != last_error {
                        warn!("AirDrop: register with t2-drop: {text}");
                        last_error = text;
                    }
                }
            }
        }
        tokio::time::sleep(CHECK).await;
    }
}
