// SPDX-License-Identifier: GPL-3.0-or-later
//! The system bus service org.t2drop. It owns AWDL, which needs
//! CAP_NET_ADMIN, while desktop programs and the t2drop command line only
//! ask it to look for receivers and to send files.
//!
//! Files come as open file descriptors, the service never opens paths of
//! its callers. AWDL runs while at least one caller browses or a transfer
//! is going on, the service exits after a minute without callers and is
//! started again by D-Bus activation.

use std::collections::HashSet;
use std::io::Read;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use log::{info, warn};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedFd;
use zbus::{Connection, fdo, interface};

use crate::airdrop::File;
use crate::ble::{AirDropBeacon, BluetoothOff};
use crate::session::{ReceiverInfo, SendRequest, Session};

pub const BUS_NAME: &str = "org.t2drop";
pub const PATH: &str = "/org/t2drop";
/// AWDL stays up this long after the last caller left.
const LINGER: Duration = Duration::from_secs(15);
/// The service exits after this long without AWDL.
const EXIT_IDLE: Duration = Duration::from_secs(60);
/// Upper limit of one transfer, the files are held in memory.
const MAX_BYTES: u64 = 2 << 30;
const MAX_FILES: usize = 256;

#[derive(Default)]
struct State {
    session: Option<Session>,
    beacon: Option<AirDropBeacon>,
    browsers: HashSet<String>,
    sending: usize,
    receivers: Vec<ReceiverInfo>,
    idle_since: Option<Instant>,
}

#[derive(Clone)]
pub struct Sender {
    state: Arc<Mutex<State>>,
    connection: Arc<Mutex<Option<Connection>>>,
    /// Serializes starting a session, two callers must not start two.
    starting: Arc<tokio::sync::Mutex<()>>,
}

impl Sender {
    fn new() -> Self {
        Self { state: Arc::default(), connection: Arc::default(), starting: Arc::default() }
    }

    /// Brings AWDL and the Bluetooth beacon up unless they run already.
    async fn ensure_session(&self) -> Result<()> {
        let _starting = self.starting.lock().await;
        if self.state.lock().unwrap().session.as_ref().is_some_and(Session::is_running) {
            return Ok(());
        }
        let beacon = AirDropBeacon::start().await?;
        let state = self.state.clone();
        let connection = self.connection.lock().unwrap().clone();
        let handle = tokio::runtime::Handle::current();
        let session = tokio::task::spawn_blocking(move || {
            Session::start(&primary_interface()?, move |list| {
                let pairs: Vec<(String, String)> = list.iter().map(|r| (r.id.clone(), r.name.clone())).collect();
                state.lock().unwrap().receivers = list;
                if let Some(conn) = connection.clone() {
                    handle.spawn(async move {
                        let _ = conn
                            .emit_signal(None::<&str>, PATH, "org.t2drop.Sender1", "ReceiversChanged", &(pairs,))
                            .await;
                    });
                }
            })
        })
        .await??;
        let mut state = self.state.lock().unwrap();
        state.session = Some(session);
        state.beacon = Some(beacon);
        state.idle_since = None;
        Ok(())
    }

    /// Ends AWDL when nobody needs it, exits after a while without AWDL.
    async fn housekeeping(&self) -> bool {
        let (session, beacon) = {
            let mut state = self.state.lock().unwrap();
            let busy = !state.browsers.is_empty() || state.sending > 0;
            if busy {
                state.idle_since = None;
                return false;
            }
            let since = *state.idle_since.get_or_insert_with(Instant::now);
            if state.session.is_none() {
                return since.elapsed() >= EXIT_IDLE;
            }
            if since.elapsed() < LINGER {
                return false;
            }
            state.receivers.clear();
            state.idle_since = Some(Instant::now());
            (state.session.take(), state.beacon.take())
        };
        // Dropping the session joins its thread, which removes AWDL
        let _ = tokio::task::spawn_blocking(move || drop(session)).await;
        if let Some(beacon) = beacon {
            beacon.stop().await;
        }
        info!("AWDL stopped, no callers");
        false
    }
}

fn sender_of(header: &Header<'_>) -> String {
    header.sender().map(|s| s.to_string()).unwrap_or_default()
}

/// Errors of org.t2drop.Sender1, BluetoothOff lets callers offer to
/// switch Bluetooth on.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.t2drop.Error")]
pub enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    BluetoothOff(String),
    InvalidArgs(String),
    Failed(String),
}

fn failed(err: anyhow::Error) -> Error {
    if err.downcast_ref::<BluetoothOff>().is_some() {
        Error::BluetoothOff(err.to_string())
    } else {
        Error::Failed(format!("{err:#}"))
    }
}

#[interface(name = "org.t2drop.Sender1")]
impl Sender {
    /// Starts looking for AirDrop receivers until StopBrowse or until the
    /// caller leaves the bus.
    async fn browse(&self, #[zbus(header)] header: Header<'_>) -> Result<(), Error> {
        self.state.lock().unwrap().browsers.insert(sender_of(&header));
        self.ensure_session().await.map_err(failed)
    }

    async fn stop_browse(&self, #[zbus(header)] header: Header<'_>) {
        self.state.lock().unwrap().browsers.remove(&sender_of(&header));
    }

    /// The receivers found so far as (id, name).
    async fn receivers(&self) -> Vec<(String, String)> {
        let state = self.state.lock().unwrap();
        state.receivers.iter().map(|r| (r.id.clone(), r.name.clone())).collect()
    }

    /// Sends the files behind `fds` under `names` to receiver `id`. Returns
    /// once the receiver has them or fails with the reason.
    async fn send(
        &self,
        id: String,
        names: Vec<String>,
        fds: Vec<OwnedFd>,
        sender_name: String,
    ) -> Result<(), Error> {
        if names.len() != fds.len() || names.is_empty() || names.len() > MAX_FILES {
            return Err(Error::InvalidArgs("one name per file descriptor".into()));
        }
        let files = tokio::task::spawn_blocking(move || read_files(names, fds))
            .await
            .map_err(|e| Error::Failed(e.to_string()))?
            .map_err(failed)?;
        let (reply, result) = mpsc::channel();
        {
            let mut state = self.state.lock().unwrap();
            let session = state.session.as_ref().ok_or_else(|| Error::Failed("not browsing".into()))?;
            session.send(SendRequest { id, sender: sender_name, files, reply }).map_err(failed)?;
            state.sending += 1;
        }
        let r = tokio::task::spawn_blocking(move || result.recv()).await;
        self.state.lock().unwrap().sending -= 1;
        match r {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(err))) => Err(failed(err)),
            _ => Err(Error::Failed("AWDL session ended".into())),
        }
    }

    #[zbus(signal)]
    async fn receivers_changed(emitter: &SignalEmitter<'_>, receivers: Vec<(String, String)>) -> zbus::Result<()>;
}

fn read_files(names: Vec<String>, fds: Vec<OwnedFd>) -> Result<Vec<File>> {
    let mut total = 0u64;
    let mut files = Vec::new();
    for (name, fd) in names.into_iter().zip(fds) {
        let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        let meta = file.metadata()?;
        if !meta.is_file() {
            bail!("{name} is not a regular file");
        }
        total += meta.len();
        if total > MAX_BYTES {
            bail!("the files are larger than {} MiB", MAX_BYTES >> 20);
        }
        let mut data = Vec::with_capacity(meta.len() as usize);
        file.read_to_end(&mut data).with_context(|| format!("read {name}"))?;
        files.push(File { name, data });
    }
    Ok(files)
}

/// The Wi-Fi interface driven by brcmfmac.
pub fn primary_interface() -> Result<String> {
    for entry in std::fs::read_dir("/sys/class/net")? {
        let path = entry?.path();
        let driver = std::fs::read_link(path.join("device/driver")).ok();
        let is_brcmfmac = driver
            .as_ref()
            .and_then(|d| d.file_name())
            .is_some_and(|n| n == "brcmfmac");
        if is_brcmfmac && path.join("wireless").exists() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                return Ok(name.to_owned());
            }
        }
    }
    bail!("no brcmfmac Wi-Fi interface")
}

/// Runs the service until it has been idle for a while.
pub async fn run() -> Result<()> {
    let sender = Sender::new();
    let connection = zbus::connection::Builder::system()?
        .serve_at(PATH, sender.clone())?
        .name(BUS_NAME)?
        .build()
        .await
        .context("org.t2drop on the system bus")?;
    *sender.connection.lock().unwrap() = Some(connection.clone());
    info!("serving {BUS_NAME}");

    // Callers that leave the bus stop browsing
    let dbus = fdo::DBusProxy::new(&connection).await?;
    let mut owners = dbus.receive_name_owner_changed().await?;
    let state = sender.state.clone();
    tokio::spawn(async move {
        while let Some(signal) = owners.next().await {
            if let Ok(args) = signal.args() {
                if args.new_owner().is_none() {
                    state.lock().unwrap().browsers.remove(args.name().as_str());
                }
            }
        }
    });

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if sender.housekeeping().await {
                    info!("idle, exiting");
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = term.recv() => break,
        }
    }
    let (session, beacon) = {
        let mut state = sender.state.lock().unwrap();
        (state.session.take(), state.beacon.take())
    };
    let _ = tokio::task::spawn_blocking(move || drop(session)).await;
    if let Some(beacon) = beacon {
        beacon.stop().await;
    }
    if let Err(err) = connection.release_name(BUS_NAME).await {
        warn!("release {BUS_NAME}: {err}");
    }
    Ok(())
}
