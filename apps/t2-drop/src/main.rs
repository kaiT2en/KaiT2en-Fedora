// SPDX-License-Identifier: GPL-3.0-or-later
//! t2drop: send files to Apple devices over AWDL from T2 Macs running
//! Linux. `t2drop daemon` is the system bus service org.t2drop, which owns
//! AWDL through the kait2en brcmfmac patch. `t2drop list` and `t2drop send`
//! are clients of it and run as the normal user.

mod airdrop;
mod awdl;
mod ble;
mod genl;
mod service;
mod session;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use zbus::proxy;
use zbus::zvariant::OwnedFd;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the org.t2drop system bus service (as root)
    Daemon,
    /// List the AirDrop receivers nearby
    List {
        /// Seconds to look for receivers
        #[arg(long, default_value_t = 15)]
        wait: u64,
    },
    /// Send files to an AirDrop receiver
    Send {
        files: Vec<PathBuf>,
        /// Part of the receiver name, the first receiver found otherwise
        #[arg(long)]
        to: Option<String>,
        /// Seconds to look for the receiver
        #[arg(long, default_value_t = 30)]
        wait: u64,
        /// Name shown on the receiver
        #[arg(long)]
        name: Option<String>,
    },
}

#[proxy(interface = "org.t2drop.Sender1", default_service = "org.t2drop", default_path = "/org/t2drop")]
trait Sender1 {
    fn browse(&self) -> zbus::Result<()>;
    fn stop_browse(&self) -> zbus::Result<()>;
    fn receivers(&self) -> zbus::Result<Vec<(String, String)>>;
    fn send(&self, id: &str, names: Vec<String>, fds: Vec<OwnedFd>, sender_name: &str) -> zbus::Result<()>;
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Linux".into())
}

async fn proxy() -> Result<Sender1Proxy<'static>> {
    let connection = zbus::Connection::system().await.context("system bus")?;
    Ok(Sender1Proxy::new(&connection).await?)
}

/// Browses until `pick` chooses a receiver or `wait` runs out.
async fn wait_for(
    sender: &Sender1Proxy<'_>,
    wait: Duration,
    mut pick: impl FnMut(&[(String, String)]) -> Option<(String, String)>,
) -> Result<Option<(String, String)>> {
    sender.browse().await.context("start looking for AirDrop receivers")?;
    let start = Instant::now();
    while start.elapsed() < wait {
        let list = sender.receivers().await?;
        if let Some(found) = pick(&list) {
            return Ok(Some(found));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(None)
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match Cli::parse().command {
        Cmd::Daemon => service::run().await,
        Cmd::List { wait } => {
            let sender = proxy().await?;
            let mut shown = std::collections::HashSet::new();
            wait_for(&sender, Duration::from_secs(wait), |list| {
                for (id, name) in list {
                    if shown.insert(id.clone()) {
                        println!("{name}\t{id}");
                    }
                }
                None
            })
            .await?;
            let _ = sender.stop_browse().await;
            if shown.is_empty() {
                bail!("no AirDrop receiver found, the receiver needs AirDrop for everyone");
            }
            Ok(())
        }
        Cmd::Send { files, to, wait, name } => {
            if files.is_empty() {
                bail!("no files to send");
            }
            let mut names = Vec::new();
            let mut fds = Vec::new();
            for path in &files {
                let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
                names.push(
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .with_context(|| format!("no file name in {}", path.display()))?
                        .to_owned(),
                );
                fds.push(OwnedFd::from(std::os::fd::OwnedFd::from(file)));
            }
            let sender = proxy().await?;
            let target = wait_for(&sender, Duration::from_secs(wait), |list| {
                list.iter()
                    .find(|(_, n)| match &to {
                        Some(to) => n.to_lowercase().contains(&to.to_lowercase()),
                        None => true,
                    })
                    .cloned()
            })
            .await?;
            let Some((id, receiver)) = target else {
                let _ = sender.stop_browse().await;
                bail!("no AirDrop receiver found, the receiver needs AirDrop for everyone");
            };
            println!("sending to {receiver}, accept on the receiver");
            let r = sender.send(&id, names, fds, &name.unwrap_or_else(hostname)).await;
            let _ = sender.stop_browse().await;
            r.context("send")?;
            println!("sent");
            Ok(())
        }
    }
}
