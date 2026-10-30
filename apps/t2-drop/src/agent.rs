// SPDX-License-Identifier: GPL-3.0-or-later
//! `t2drop receive`: an org.t2drop.Agent1 on the terminal. It makes this
//! device visible to AirDrop senders, at once or with --ble once a sender
//! nearby advertises over Bluetooth LE. It asks on the terminal or accepts
//! everything with --yes, and saves the files with the rights of the user.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedFd};
use zbus::{fdo, interface, proxy};

const AGENT_PATH: &str = "/org/t2drop/agent";
/// Time the user has to answer on the terminal.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(100);

#[proxy(interface = "org.t2drop.Receiver1", default_service = "org.t2drop", default_path = "/org/t2drop")]
trait Receiver1 {
    fn register_agent(&self, path: &ObjectPath<'_>, name: &str) -> zbus::Result<()>;
    fn unregister_agent(&self) -> zbus::Result<()>;
    fn start_receiving(&self) -> zbus::Result<()>;
    fn stop_receiving(&self) -> zbus::Result<()>;
}

struct Agent {
    dir: PathBuf,
    yes: bool,
}

/// Only the service may call the agent.
async fn check_caller(connection: &zbus::Connection, header: &Header<'_>) -> fdo::Result<()> {
    let owner = fdo::DBusProxy::new(connection)
        .await?
        .get_name_owner(zbus::names::BusName::try_from("org.t2drop").map_err(zbus::Error::from)?)
        .await?;
    if header.sender().map(|s| s.as_str()) != Some(owner.as_str()) {
        return Err(fdo::Error::AccessDenied("only org.t2drop may call the agent".into()));
    }
    Ok(())
}

#[interface(name = "org.t2drop.Agent1")]
impl Agent {
    async fn ask(
        &self,
        id: String,
        sender_name: String,
        files: Vec<String>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        check_caller(connection, &header).await?;
        println!("{sender_name} wants to send {}: {}", files.len(), files.join(", "));
        if self.yes {
            println!("accepted {id}");
            return Ok(true);
        }
        print!("accept? [y/N] ");
        let _ = std::io::stdout().flush();
        let answer = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).map(|_| line)
        });
        let accepted = match tokio::time::timeout(ANSWER_TIMEOUT, answer).await {
            Ok(Ok(Ok(line))) => matches!(line.trim(), "y" | "Y" | "yes"),
            _ => false,
        };
        println!("{} {id}", if accepted { "accepted" } else { "declined" });
        Ok(accepted)
    }

    async fn deliver(
        &self,
        id: String,
        sender_name: String,
        names: Vec<String>,
        fds: Vec<OwnedFd>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<()> {
        check_caller(connection, &header).await?;
        if names.len() != fds.len() {
            return Err(fdo::Error::InvalidArgs("one name per file descriptor".into()));
        }
        let dir = self.dir.clone();
        let saved = tokio::task::spawn_blocking(move || save(&dir, names, fds))
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?
            .map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        for path in saved {
            println!("received {} from {sender_name} ({id})", path.display());
        }
        Ok(())
    }
}

/// Copies the received files into `dir`, next to existing ones.
fn save(dir: &Path, names: Vec<String>, fds: Vec<OwnedFd>) -> Result<Vec<PathBuf>> {
    let mut saved = Vec::new();
    for (name, fd) in names.into_iter().zip(fds) {
        if name.starts_with('/') || name.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
            bail!("unsafe file name {name:?}");
        }
        let path = crate::free_path(&dir.join(&name));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut source = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        let mut target = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("create {}", path.display()))?;
        std::io::copy(&mut source, &mut target).with_context(|| format!("write {}", path.display()))?;
        saved.push(path);
    }
    Ok(saved)
}

/// ~/Downloads, or the home folder without one.
pub fn downloads() -> Result<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let downloads = home.join("Downloads");
    Ok(if downloads.is_dir() { downloads } else { home })
}

pub async fn run(dir: PathBuf, name: String, yes: bool, ble: bool) -> Result<()> {
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let connection = zbus::connection::Builder::system()?
        .serve_at(AGENT_PATH, Agent { dir: dir.clone(), yes })?
        .build()
        .await
        .context("system bus")?;
    let receiver = Receiver1Proxy::new(&connection).await?;
    receiver.register_agent(&ObjectPath::try_from(AGENT_PATH)?, &name).await.context("register agent")?;
    if ble {
        println!("visible as {name} while an AirDrop sender is nearby, files go to {}, Ctrl+C ends", dir.display());
    } else {
        receiver.start_receiving().await.context("start receiving")?;
        println!("visible as {name} to AirDrop senders, files go to {}, Ctrl+C ends", dir.display());
    }
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    if !ble {
        let _ = receiver.stop_receiving().await;
    }
    let _ = receiver.unregister_agent().await;
    Ok(())
}
