// SPDX-License-Identifier: GPL-3.0-or-later
//! Command line front end: receive and send Quick Share transfers in the
//! shared WLAN.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use log::{info, warn};
use tokio::net::TcpListener;

use t2_airshare::discover::{Discovery, Peer};
use t2_airshare::endpoint::random_endpoint_id;
use t2_airshare::mdns::Advertisement;
use t2_airshare::receive::{self, Incoming, Outcome};
use t2_airshare::send::{self, Progress, SendOutcome};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show up in Android's share sheet and receive files and texts
    Receive {
        /// Name shown on the phone, the hostname by default
        #[arg(long)]
        name: Option<String>,
        /// Directory for received files, the XDG download directory by default
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Accept every transfer without asking
        #[arg(long)]
        yes: bool,
        /// TCP port, a free one by default
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Run as the background service: trusted devices are accepted, others
    /// are asked through a desktop notification
    Daemon {
        /// Name shown on the phone, the hostname by default
        #[arg(long)]
        name: Option<String>,
    },
    /// Send files to a phone in the shared WLAN. The phone has to be visible
    /// to everyone in its Quick Share settings.
    Send {
        /// Files to send
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Send to the first phone whose name contains this text, without asking
        #[arg(long)]
        to: Option<String>,
        /// Seconds to look for phones
        #[arg(long, default_value_t = 10)]
        wait: u64,
        /// Name shown on the phone, the hostname by default
        #[arg(long)]
        name: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match Cli::parse().command {
        Command::Receive { name, dir, yes, port } => receive_loop(name, dir, yes, port).await,
        Command::Daemon { name } => t2_airshare::service::run(&name.unwrap_or_else(default_name)).await,
        Command::Send { files, to, wait, name } => {
            send_files(files, to, wait, name.unwrap_or_else(default_name)).await
        }
    }
}

async fn send_files(files: Vec<PathBuf>, to: Option<String>, wait: u64, name: String) -> Result<()> {
    let mut discovery = Discovery::start().await?;
    let mut peers: Vec<Peer> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(wait);
    println!("looking for phones for {wait} s");
    let target = loop {
        tokio::select! {
            peer = discovery.next() => {
                let peer = peer?;
                println!("  {}: {} ({:?})", peers.len() + 1, peer.label(), peer.device_type);
                if let Some(to) = &to {
                    if peer.label().to_lowercase().contains(&to.to_lowercase()) {
                        break peer;
                    }
                }
                peers.push(peer);
            }
            _ = tokio::time::sleep_until(deadline) => {
                if to.is_some() || peers.is_empty() {
                    anyhow::bail!(
                        "no matching phone found. The phone has to be in the same Wi-Fi, with \
                         Bluetooth on and Quick Share visible to everyone"
                    );
                }
                break pick(peers)?;
            }
        }
    };
    drop(discovery);

    println!("sending to {} at {}", target.label(), target.addr);
    let mut progress = CliProgress { last: 0 };
    match send::send(target.addr, &name, &files, &mut progress).await? {
        SendOutcome::Sent => println!("\rsent {} file(s)          ", files.len()),
        SendOutcome::Declined(status) => println!("\rdeclined by the phone ({status})"),
    }
    Ok(())
}

fn pick(peers: Vec<Peer>) -> Result<Peer> {
    if peers.len() == 1 {
        return Ok(peers.into_iter().next().expect("one peer"));
    }
    print!("send to [1-{}]: ", peers.len());
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let index: usize = line.trim().parse().context("not a number")?;
    peers.into_iter().nth(index.wrapping_sub(1)).context("no such phone")
}

struct CliProgress {
    last: u64,
}

impl Progress for CliProgress {
    fn waiting(&mut self, pin: &str) {
        println!("waiting for the phone to accept, PIN {pin}");
    }

    fn sent(&mut self, sent: u64, total: u64) {
        let percent = sent * 100 / total.max(1);
        if percent != self.last {
            self.last = percent;
            print!("\r{percent} %");
            let _ = std::io::stdout().flush();
        }
    }
}

async fn receive_loop(name: Option<String>, dir: Option<PathBuf>, yes: bool, port: u16) -> Result<()> {
    let dir = dir
        .or_else(dirs::download_dir)
        .or_else(dirs::home_dir)
        .context("no download directory")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let name = name.unwrap_or_else(default_name);

    let listener = TcpListener::bind(("0.0.0.0", port)).await.context("listen")?;
    let port = listener.local_addr()?.port();
    let advertisement = Advertisement::start(random_endpoint_id(), &name, port)?;
    info!("visible as {name:?} on TCP port {port}, saving to {}", dir.display());

    let consent: receive::Consent = Arc::new(move |incoming: Incoming| {
        Box::pin(async move {
            if yes {
                describe(&incoming);
                return true;
            }
            tokio::task::spawn_blocking(move || ask(&incoming)).await.unwrap_or(false)
        })
    });

    let mut announce = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                info!("connection from {peer}");
                let dir = dir.clone();
                let consent = consent.clone();
                tokio::spawn(async move {
                    match receive::handle(stream, dir, consent).await {
                        Ok(Outcome::Received { files, texts }) => {
                            for file in files {
                                println!("saved {}", file.display());
                            }
                            for text in texts {
                                println!("text: {text}");
                            }
                        }
                        Ok(Outcome::Rejected) => println!("rejected"),
                        Ok(Outcome::Cancelled) => println!("cancelled by the sender"),
                        Err(err) => warn!("transfer failed: {err:#}"),
                    }
                });
            }
            _ = announce.tick() => {
                if let Err(err) = advertisement.announce() {
                    warn!("{err:#}");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("stopping");
                return Ok(());
            }
        }
    }
}

fn describe(incoming: &Incoming) {
    let sender = incoming.sender.as_deref().unwrap_or("a hidden device");
    println!("{sender} ({:?}) wants to send, PIN {}", incoming.device_type, incoming.pin);
    for file in &incoming.files {
        println!("  {} ({} bytes)", file.name, file.size);
    }
    for text in &incoming.texts {
        println!("  text: {text}");
    }
}

fn ask(incoming: &Incoming) -> bool {
    describe(incoming);
    print!("accept? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim(), "y" | "Y" | "yes" | "j" | "ja")
}

fn default_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Linux".to_string())
}
