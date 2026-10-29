// SPDX-License-Identifier: GPL-3.0-or-later
//! One AWDL session: brings AWDL up, keeps it running, finds the AirDrop
//! receivers around and sends files to them on request. Runs in its own
//! thread, the netlink calls block.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use log::{info, warn};
use rand::RngCore;

use crate::airdrop::{File, Receiver};
use crate::awdl::{Awdl, link_local, mac_str};
use crate::genl::Genl;

/// How often the AWDL requests are sent again, as macOS does.
const REFRESH: Duration = Duration::from_millis(2500);
/// macOS ends the fast search settings 5 s after the enable.
const SETTLE_AFTER: Duration = Duration::from_secs(5);
/// Attempts of /Discover, each with a 3 s connect timeout and a 1 s pause.
const DISCOVER_ATTEMPTS: u32 = 20;
/// How long a receiver may stay silent before it is dropped.
const RECEIVER_TIMEOUT: Duration = Duration::from_secs(30);
/// Time the user on the receiver has to accept.
const ASK_TIMEOUT: Duration = Duration::from_secs(120);

/// An AirDrop receiver, `id` is its AWDL MAC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverInfo {
    pub id: String,
    pub name: String,
}

pub struct SendRequest {
    pub id: String,
    pub sender: String,
    pub files: Vec<File>,
    pub reply: mpsc::Sender<Result<()>>,
}

pub struct Session {
    stop: Arc<AtomicBool>,
    requests: mpsc::Sender<SendRequest>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts AWDL on `primary`. `changed` gets the receiver list whenever
    /// it changes.
    pub fn start(
        primary: &str,
        changed: impl Fn(Vec<ReceiverInfo>) + Send + 'static,
    ) -> Result<Self> {
        let primary = ifindex(primary)?;
        let genl = Genl::open()?;
        let events = genl.events()?;
        let awdl = Awdl::up(genl, primary, random_mac(), awdl_host())?;
        info!("AWDL up on {} as {}", awdl.ifname, mac_str(&awdl.mac));
        let stop = Arc::new(AtomicBool::new(false));
        let (requests, request_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(batch) = events.next() {
                for ev in batch {
                    if ev_tx.send(ev).is_err() {
                        return;
                    }
                }
            }
        });
        let thread_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            run(awdl, ev_rx, request_rx, &thread_stop, changed);
        });
        Ok(Self { stop, requests, thread: Some(thread) })
    }

    pub fn send(&self, request: SendRequest) -> Result<()> {
        self.requests.send(request).map_err(|_| anyhow!("AWDL session ended"))
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Known {
    name: String,
    port: u16,
}

fn run(
    mut awdl: Awdl,
    events: mpsc::Receiver<crate::genl::Event>,
    requests: mpsc::Receiver<SendRequest>,
    stop: &AtomicBool,
    changed: impl Fn(Vec<ReceiverInfo>),
) {
    let (found_tx, found_rx) = mpsc::channel::<([u8; 6], u16, Option<String>)>();
    let start = Instant::now();
    let mut last_refresh = Instant::now();
    let mut settled = false;
    let mut asked: HashSet<[u8; 6]> = HashSet::new();
    let mut known: HashMap<[u8; 6], Known> = HashMap::new();
    let mut published: Vec<ReceiverInfo> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        if let Ok(ev) = events.recv_timeout(Duration::from_millis(100)) {
            if let Err(err) = awdl.handle_event(&ev) {
                warn!("{err:#}");
            }
        }
        if last_refresh.elapsed() >= REFRESH {
            last_refresh = Instant::now();
            if let Err(err) = awdl.refresh() {
                warn!("AWDL refresh: {err:#}");
                break;
            }
        }
        if !settled && start.elapsed() >= SETTLE_AFTER {
            settled = true;
            let _ = awdl.settle();
        }

        // Ask every new AirDrop receiver for its name
        let new: Vec<_> = awdl
            .peers
            .values()
            .filter(|p| !asked.contains(&p.mac))
            .filter_map(|p| p.airdrop.as_ref().map(|a| (p.mac, a.1)))
            .collect();
        for (mac, port) in new {
            asked.insert(mac);
            if let Err(err) = awdl.add_peer(mac) {
                warn!("peer {}: {err:#}", mac_str(&mac));
                continue;
            }
            let Ok(receiver) = Receiver::new(&link_local(&mac), awdl.ifindex, port) else { continue };
            let tx = found_tx.clone();
            std::thread::spawn(move || {
                // The link local address of awdl0 is unusable until its
                // duplicate address detection ends, so retry for a while.
                let mut name = None;
                for attempt in 1..=DISCOVER_ATTEMPTS {
                    match receiver.discover() {
                        Ok(n) => {
                            name = n;
                            break;
                        }
                        Err(err) if attempt == DISCOVER_ATTEMPTS => {
                            warn!("discover {}: {err:#}", mac_str(&mac));
                        }
                        Err(err) => {
                            log::debug!("waiting for {} ({err})", mac_str(&mac));
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
                let _ = tx.send((mac, port, name));
            });
        }
        while let Ok((mac, port, name)) = found_rx.try_recv() {
            match name {
                Some(name) => {
                    info!("AirDrop receiver {name} ({})", mac_str(&mac));
                    known.insert(mac, Known { name, port });
                }
                // Try again when it shows up the next time
                None => {
                    asked.remove(&mac);
                }
            }
        }

        // Drop receivers that went silent
        let now = Instant::now();
        known.retain(|mac, _| {
            awdl.peers
                .get(mac)
                .and_then(|p| p.last_seen)
                .is_some_and(|t| now.duration_since(t) < RECEIVER_TIMEOUT)
        });
        asked.retain(|mac| known.contains_key(mac) || awdl.peers.contains_key(mac));
        awdl.peers.retain(|_, p| p.last_seen.is_some_and(|t| now.duration_since(t) < RECEIVER_TIMEOUT));

        let mut list: Vec<ReceiverInfo> = known
            .iter()
            .map(|(mac, k)| ReceiverInfo { id: mac_str(mac), name: k.name.clone() })
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        if list != published {
            published = list.clone();
            changed(list);
        }

        while let Ok(request) = requests.try_recv() {
            let target = known.iter().find(|(mac, _)| mac_str(mac) == request.id);
            let Some((mac, k)) = target else {
                let _ = request.reply.send(Err(anyhow!("receiver {} is gone", request.id)));
                continue;
            };
            match Receiver::new(&link_local(mac), awdl.ifindex, k.port) {
                Ok(receiver) => {
                    std::thread::spawn(move || {
                        let r = receiver.send(&request.sender, request.files, ASK_TIMEOUT);
                        let _ = request.reply.send(r);
                    });
                }
                Err(err) => {
                    let _ = request.reply.send(Err(err));
                }
            }
        }
    }
    changed(Vec::new());
    awdl.down();
    info!("AWDL down");
}

/// A host name in the form macOS uses on AWDL (a UUID).
fn awdl_host() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn random_mac() -> [u8; 6] {
    let mut mac = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut mac);
    mac[0] = (mac[0] & 0xfe) | 0x02;
    mac
}

fn ifindex(name: &str) -> Result<u32> {
    let c = CString::new(name)?;
    // SAFETY: c is a valid NUL terminated string.
    let index = unsafe { libc::if_nametoindex(c.as_ptr()) };
    if index == 0 {
        bail!("no interface {name}");
    }
    Ok(index)
}
