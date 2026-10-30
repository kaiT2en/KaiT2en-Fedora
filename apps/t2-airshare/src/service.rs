// SPDX-License-Identifier: GPL-3.0-or-later
//! Background receiver for the systemd user service: visible in the shared
//! WLAN, trusted devices are accepted right away, everyone else is asked
//! through a notification. AirDrop transfers arrive through the t2-drop
//! service when it is installed and take the same way.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use log::{info, warn};
use tokio::net::TcpListener;

use crate::config::Config;
use crate::endpoint::random_endpoint_id;
use crate::mdns::Advertisement;
use crate::notify::{Launch, Notifier};
use crate::receive::{self, Incoming, Outcome};

/// How long an unanswered request waits before it is rejected.
const ASK_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the "open" button on a finished transfer stays usable.
const OFFER_TIMEOUT: Duration = Duration::from_secs(300);

pub async fn run(name: &str) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", 0)).await.context("listen")?;
    let port = listener.local_addr()?.port();
    let advertisement = Advertisement::start(random_endpoint_id(), name, port)?;
    info!("visible as {name:?} on TCP port {port}");

    let consent: receive::Consent = Arc::new(|incoming: Incoming| Box::pin(ask(incoming)));
    tokio::spawn(crate::airdrop::run(name.to_string()));
    let mut announce = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                info!("connection from {peer}");
                let consent = consent.clone();
                tokio::spawn(async move {
                    let config = Config::load().unwrap_or_else(|err| {
                        warn!("{err:#}, using defaults");
                        Config::default()
                    });
                    let Some(dir) = config.download_dir() else {
                        warn!("no download directory");
                        return;
                    };
                    if let Err(err) = std::fs::create_dir_all(&dir) {
                        warn!("create {}: {err}", dir.display());
                        return;
                    }
                    match receive::handle(stream, dir.clone(), consent).await {
                        Ok(outcome) => report(outcome).await,
                        Err(err) => {
                            warn!("transfer failed: {err:#}");
                            notify_info("Transfer failed", &format!("{err:#}")).await;
                        }
                    }
                });
            }
            _ = announce.tick() => {
                if let Err(err) = advertisement.announce() {
                    warn!("{err:#}");
                }
            }
            _ = shutdown_signal() => {
                info!("stopping");
                return Ok(());
            }
        }
    }
}

/// Accepts trusted devices, asks for everyone else.
async fn ask(incoming: Incoming) -> bool {
    let body = format!("{}\nPIN {}", contents(&incoming), incoming.pin);
    ask_user(incoming.sender, body).await
}

/// Accepts a trusted `sender`, otherwise asks with a notification that
/// shows `body` and offers to trust a sender with a name.
pub(crate) async fn ask_user(sender: Option<String>, body: String) -> bool {
    let config = Config::load().unwrap_or_default();
    if let Some(name) = &sender {
        if config.is_trusted(name) {
            info!("accepting {name:?}, trusted");
            return true;
        }
    }

    let who = sender.as_deref().unwrap_or("A hidden device");
    let summary = format!("{who} wants to share");
    let mut actions = vec![("accept", "Accept")];
    if sender.is_some() {
        actions.push(("trust", "Always trust"));
    }
    actions.push(("reject", "Decline"));

    let answer = match Notifier::new().await {
        Ok(notifier) => notifier.ask(&summary, &body, &actions, ASK_TIMEOUT).await,
        Err(err) => Err(err),
    };
    match answer.as_ref().map(|a| a.as_deref()) {
        Ok(Some("accept")) => true,
        Ok(Some("trust")) => {
            let name = sender.expect("only offered with a name");
            let mut config = Config::load().unwrap_or_default();
            config.trust(&name);
            if let Err(err) = config.save() {
                warn!("could not save trusted device {name:?}: {err:#}");
            } else {
                info!("trusting {name:?} from now on");
            }
            true
        }
        Ok(_) => false,
        Err(err) => {
            warn!("could not ask, declining: {err:#}");
            false
        }
    }
}

pub(crate) async fn report(outcome: Outcome) {
    match outcome {
        Outcome::Received { files, texts } => {
            for text in &texts {
                let Ok(notifier) = Notifier::new().await else { continue };
                if text.starts_with("https://") || text.starts_with("http://") {
                    let launch = Launch::OpenUri(text.clone());
                    notifier.offer("Link received", text, "Open", OFFER_TIMEOUT, launch);
                } else {
                    let _ = notifier.info("Text received", text).await;
                }
            }
            if files.is_empty() {
                return;
            }
            let summary = match files.len() {
                1 => "File received".to_string(),
                n => format!("{n} files received"),
            };
            let body = files
                .iter()
                .filter_map(|f| f.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("\n");
            if let Ok(notifier) = Notifier::new().await {
                let launch = Launch::ShowItems(files);
                notifier.offer(&summary, &body, "Show in folder", OFFER_TIMEOUT, launch);
            }
        }
        Outcome::Rejected => info!("declined"),
        Outcome::Cancelled => info!("cancelled by the sender"),
    }
}

fn contents(incoming: &Incoming) -> String {
    let mut lines: Vec<String> = incoming
        .files
        .iter()
        .map(|f| format!("{} ({})", f.name, human_size(f.size)))
        .collect();
    lines.extend(incoming.texts.iter().map(|t| format!("Text: {t}")));
    lines.join("\n")
}

pub(crate) fn human_size(bytes: i64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit < UNITS.len() - 1 {
        size /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

pub(crate) async fn notify_info(summary: &str, body: &str) {
    if let Ok(notifier) = Notifier::new().await {
        let _ = notifier.info(summary, body).await;
    }
}

async fn shutdown_signal() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
