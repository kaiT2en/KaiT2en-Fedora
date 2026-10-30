// SPDX-License-Identifier: GPL-3.0-or-later
//! The system bus service org.t2drop. It owns AWDL, which needs
//! CAP_NET_ADMIN, while desktop programs and the t2drop command line only
//! ask it to look for receivers, to send files and to receive files.
//!
//! Files come as open file descriptors, the service never opens paths of
//! its callers. Received files go to the registered agent as memfds, the
//! agent asks its user and saves them with the user's rights.
//!
//! Receiving follows macOS 15: while an agent is registered and its
//! session is active and unlocked, the service scans Bluetooth LE for the
//! AirDrop advertisement of senders. It brings AWDL up and advertises the
//! own AirDrop service while one is near, BlueZ forgets a sender 30 s after
//! its last advertisement, and stops then unless a transfer goes on. Before sleep everything is taken down, after
//! resume it comes back as needed. Without an agent the service exits
//! after a minute without callers and is started again by D-Bus
//! activation.

use std::collections::HashSet;
use std::ffi::CString;
use std::io::{Read, Seek, Write};
use std::os::fd::FromRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use log::{info, warn};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedFd, OwnedObjectPath};
use zbus::{Connection, fdo, interface, proxy};

use crate::airdrop::File;
use crate::ble::{AirDropBeacon, BeaconScanner, BluetoothOff};
use crate::logind;
use crate::receive::{Ask, Handler};
use crate::session::{ReceiverInfo, SendRequest, Session};

pub const BUS_NAME: &str = "org.t2drop";
pub const PATH: &str = "/org/t2drop";
/// AWDL stays up this long after it is no longer needed.
const LINGER: Duration = Duration::from_secs(15);
/// The service exits after this long without AWDL and without an agent.
const EXIT_IDLE: Duration = Duration::from_secs(60);
/// Upper limit of one transfer, the files are held in memory.
const MAX_BYTES: u64 = 2 << 30;
const MAX_FILES: usize = 256;
/// Time the agent has to answer an ask, the sender gives up after that.
const AGENT_TIMEOUT: Duration = Duration::from_secs(120);

/// The program that asks the user about incoming transfers.
#[derive(Clone)]
struct Agent {
    bus_name: String,
    path: OwnedObjectPath,
    /// Name shown on the senders
    name: String,
    /// logind session of the agent, None if it has none
    session: Option<OwnedObjectPath>,
}

#[proxy(interface = "org.t2drop.Agent1")]
trait Agent1 {
    fn ask(&self, id: &str, sender_name: &str, files: Vec<String>) -> zbus::Result<bool>;
    fn deliver(&self, id: &str, sender_name: &str, names: Vec<String>, fds: Vec<OwnedFd>) -> zbus::Result<()>;
}

#[derive(Default)]
struct State {
    session: Option<Session>,
    /// The own advertisement while browsing or sending
    beacon: Option<AirDropBeacon>,
    /// Looks for senders while an agent can take transfers
    scanner: Option<BeaconScanner>,
    browsers: HashSet<String>,
    /// Callers of StartReceiving
    listeners: HashSet<String>,
    agent: Option<Agent>,
    sending: usize,
    /// Accepted incoming transfers still connected
    incoming: usize,
    /// Whether a sender was near at the last housekeeping
    sender_near: bool,
    receivers: Vec<ReceiverInfo>,
    /// Since when AWDL is not needed
    idle_since: Option<Instant>,
    /// Between PrepareForSleep(true) and PrepareForSleep(false)
    sleeping: bool,
    /// Last problem logged by the housekeeping, logged once
    problem: String,
}

impl State {
    fn report(&mut self, problem: String) {
        if problem != self.problem {
            warn!("{problem}");
            self.problem = problem;
        }
    }
}

#[derive(Clone)]
pub struct Sender {
    state: Arc<Mutex<State>>,
    connection: Arc<Mutex<Option<Connection>>>,
    /// Serializes starting a session, two callers must not start two.
    starting: Arc<tokio::sync::Mutex<()>>,
    /// Whether the session advertises the own AirDrop service
    receiving: Arc<AtomicBool>,
}

/// org.t2drop.Receiver1 on the same object as Sender1.
pub struct Receiver(Sender);

impl Sender {
    fn new() -> Self {
        Self {
            state: Arc::default(),
            connection: Arc::default(),
            starting: Arc::default(),
            receiving: Arc::default(),
        }
    }

    /// Forgets a caller that left the bus.
    fn caller_left(&self, name: &str) {
        let mut state = self.state.lock().unwrap();
        state.browsers.remove(name);
        state.listeners.remove(name);
        if state.agent.as_ref().is_some_and(|a| a.bus_name == name) {
            info!("AirDrop agent {name} left");
            state.agent = None;
        }
    }

    /// Starts the own AirDrop advertisement unless it runs already.
    async fn ensure_beacon(&self) -> Result<()> {
        if self.state.lock().unwrap().beacon.is_some() {
            return Ok(());
        }
        let beacon = AirDropBeacon::start().await?;
        let mut state = self.state.lock().unwrap();
        if state.beacon.is_none() {
            state.beacon = Some(beacon);
        }
        Ok(())
    }

    /// Brings AWDL up unless it runs already.
    async fn ensure_session(&self) -> Result<()> {
        let _starting = self.starting.lock().await;
        {
            let state = self.state.lock().unwrap();
            if state.sleeping {
                bail!("the system is going to sleep");
            }
            if state.session.as_ref().is_some_and(Session::is_running) {
                return Ok(());
            }
        }
        let state = self.state.clone();
        let connection = self.connection.lock().unwrap().clone();
        let handle = tokio::runtime::Handle::current();
        let handler: Arc<dyn Handler> = Arc::new(AgentHandler {
            state: self.state.clone(),
            connection: connection.clone(),
            handle: handle.clone(),
        });
        let receiving = self.receiving.clone();
        let session = tokio::task::spawn_blocking(move || {
            Session::start(&primary_interface()?, handler, receiving, move |list| {
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
        state.idle_since = None;
        Ok(())
    }

    /// Whether the agent's user can see and answer a request.
    async fn agent_usable(&self) -> bool {
        let agent = self.state.lock().unwrap().agent.clone();
        let connection = self.connection.lock().unwrap().clone();
        match (agent.and_then(|a| a.session), connection) {
            (Some(session), Some(connection)) => logind::session_usable(&connection, &session).await,
            _ => false,
        }
    }

    /// Keeps the scanner running while the agent can take transfers.
    async fn update_scanner(&self, wanted: bool) {
        let scanner = self.state.lock().unwrap().scanner.take();
        let scanner = match scanner {
            Some(scanner) if wanted && scanner.alive().await => Some(scanner),
            Some(scanner) => {
                scanner.stop().await;
                None
            }
            None => None,
        };
        let scanner = match scanner {
            None if wanted => {
                match BeaconScanner::start().await {
                    Ok(scanner) => {
                        info!("looking for AirDrop senders");
                        self.state.lock().unwrap().problem.clear();
                        Some(scanner)
                    }
                    Err(err) => {
                        self.state.lock().unwrap().report(format!("AirDrop sender scan: {err:#}"));
                        None
                    }
                }
            }
            s => s,
        };
        self.state.lock().unwrap().scanner = scanner;
    }

    /// Runs once a second: brings the scanner, the own advertisement and
    /// AWDL up or down as needed. Returns true when the service may exit.
    async fn housekeeping(&self) -> bool {
        if self.state.lock().unwrap().sleeping {
            return false;
        }
        let usable = self.agent_usable().await;
        let has_agent = self.state.lock().unwrap().agent.is_some();
        self.update_scanner(has_agent && usable).await;

        let (receive, want_beacon, want_awdl) = {
            let mut state = self.state.lock().unwrap();
            let sender_near = state.scanner.as_ref().is_some_and(BeaconScanner::sender_near);
            if sender_near != state.sender_near {
                state.sender_near = sender_near;
                info!("AirDrop sender {}", if sender_near { "nearby" } else { "gone" });
            }
            let receive = state.agent.is_some() && (!state.listeners.is_empty() || sender_near);
            let want_beacon = !state.browsers.is_empty() || state.sending > 0;
            let want_awdl = want_beacon || receive || state.incoming > 0;
            (receive, want_beacon, want_awdl)
        };
        self.receiving.store(receive, Ordering::Relaxed);

        if want_beacon {
            if let Err(err) = self.ensure_beacon().await {
                self.state.lock().unwrap().report(format!("AirDrop advertisement: {err:#}"));
            }
        } else if let Some(beacon) = self.state.lock().unwrap().beacon.take() {
            tokio::spawn(beacon.stop());
        }

        if want_awdl {
            self.state.lock().unwrap().idle_since = None;
            if let Err(err) = self.ensure_session().await {
                self.state.lock().unwrap().report(format!("AWDL: {err:#}"));
            }
            return false;
        }
        let session = {
            let mut state = self.state.lock().unwrap();
            let since = *state.idle_since.get_or_insert_with(Instant::now);
            if state.session.is_none() {
                return state.agent.is_none() && since.elapsed() >= EXIT_IDLE;
            }
            if since.elapsed() < LINGER {
                return false;
            }
            state.receivers.clear();
            state.idle_since = Some(Instant::now());
            state.session.take()
        };
        // Dropping the session joins its thread, which removes AWDL
        let _ = tokio::task::spawn_blocking(move || drop(session)).await;
        info!("AWDL stopped, not needed");
        false
    }

    /// Takes the scanner, the advertisement and AWDL down. Transfers that
    /// go on fail, the senders see the error.
    async fn stop_all(&self) {
        let (session, beacon, scanner) = {
            let mut state = self.state.lock().unwrap();
            state.receivers.clear();
            (state.session.take(), state.beacon.take(), state.scanner.take())
        };
        self.receiving.store(false, Ordering::Relaxed);
        let _ = tokio::task::spawn_blocking(move || drop(session)).await;
        if let Some(beacon) = beacon {
            beacon.stop().await;
        }
        if let Some(scanner) = scanner {
            scanner.stop().await;
        }
    }
}

/// Asks the registered agent and hands it the received files.
struct AgentHandler {
    state: Arc<Mutex<State>>,
    connection: Option<Connection>,
    handle: tokio::runtime::Handle,
}

impl AgentHandler {
    fn agent(&self) -> Option<Agent> {
        self.state.lock().unwrap().agent.clone()
    }

    async fn proxy(&self, agent: &Agent) -> Result<Agent1Proxy<'static>> {
        let connection = self.connection.clone().context("not on the bus")?;
        Ok(Agent1Proxy::builder(&connection)
            .destination(agent.bus_name.clone())?
            .path(agent.path.clone())?
            .build()
            .await?)
    }
}

impl Handler for AgentHandler {
    fn name(&self) -> String {
        self.agent().map(|a| a.name).unwrap_or_else(|| "Linux".into())
    }

    fn ask(&self, ask: &Ask) -> bool {
        let Some(agent) = self.agent() else { return false };
        let r = self.handle.block_on(async {
            let proxy = self.proxy(&agent).await?;
            let call = proxy.ask(&ask.id, &ask.sender_name, ask.files.clone());
            Ok::<_, anyhow::Error>(tokio::time::timeout(AGENT_TIMEOUT, call).await??)
        });
        match r {
            Ok(accepted) => accepted,
            Err(err) => {
                warn!("AirDrop agent ask: {err:#}");
                false
            }
        }
    }

    fn deliver(&self, ask: &Ask, files: Vec<File>) -> Result<()> {
        let agent = self.agent().context("no AirDrop agent")?;
        let mut names = Vec::new();
        let mut fds = Vec::new();
        for file in files {
            fds.push(OwnedFd::from(memfd(&file.name, &file.data)?));
            names.push(file.name);
        }
        self.handle.block_on(async {
            let proxy = self.proxy(&agent).await?;
            proxy.deliver(&ask.id, &ask.sender_name, names, fds).await?;
            Ok(())
        })
    }

    fn transfer(&self, active: bool) {
        let mut state = self.state.lock().unwrap();
        if active {
            state.incoming += 1;
        } else {
            state.incoming = state.incoming.saturating_sub(1);
        }
    }
}

/// A sealed in memory file with `data`, read from the start.
fn memfd(name: &str, data: &[u8]) -> Result<std::os::fd::OwnedFd> {
    let c = CString::new(name.replace('\0', "_"))?;
    // SAFETY: c is a valid NUL terminated string.
    let fd = unsafe { libc::memfd_create(c.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("memfd_create");
    }
    // SAFETY: fd is a new descriptor owned by nobody else.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(data)?;
    file.rewind()?;
    let seals = libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;
    // SAFETY: fd is valid, F_ADD_SEALS takes an int.
    if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, seals) } < 0 {
        return Err(std::io::Error::last_os_error()).context("seal memfd");
    }
    Ok(file.into())
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
        self.ensure_beacon().await.map_err(failed)?;
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

#[interface(name = "org.t2drop.Receiver1")]
impl Receiver {
    /// Registers the object `path` of the caller as the agent, it
    /// implements org.t2drop.Agent1. `name` is shown on the senders. A
    /// later registration replaces it. Senders nearby find this device
    /// while the agent's session is active and unlocked.
    async fn register_agent(
        &self,
        path: OwnedObjectPath,
        name: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<(), Error> {
        if name.trim().is_empty() {
            return Err(Error::InvalidArgs("empty name".into()));
        }
        let bus_name = sender_of(&header);
        let pid = fdo::DBusProxy::new(connection)
            .await?
            .get_connection_unix_process_id(zbus::names::BusName::try_from(bus_name.as_str()).map_err(zbus::Error::from)?)
            .await
            .map_err(|e| Error::Failed(e.to_string()))?;
        let session = match logind::session_of(connection, pid).await {
            Ok(session) => Some(session),
            Err(err) => {
                warn!("AirDrop agent {bus_name}: {err:#}, only StartReceiving makes it visible");
                None
            }
        };
        info!(
            "AirDrop agent {bus_name} {} for session {}",
            path.as_str(),
            session.as_ref().map(|s| s.as_str()).unwrap_or("none")
        );
        self.0.state.lock().unwrap().agent = Some(Agent { bus_name, path, name, session });
        Ok(())
    }

    async fn unregister_agent(&self, #[zbus(header)] header: Header<'_>) {
        let caller = sender_of(&header);
        let mut state = self.0.state.lock().unwrap();
        if state.agent.as_ref().is_some_and(|a| a.bus_name == caller) {
            state.agent = None;
        }
    }

    /// Makes this device visible to AirDrop senders until StopReceiving or
    /// until the caller leaves the bus, without waiting for a sender's
    /// Bluetooth advertisement. Senders are only answered while an agent
    /// is registered.
    async fn start_receiving(&self, #[zbus(header)] header: Header<'_>) -> Result<(), Error> {
        self.0.state.lock().unwrap().listeners.insert(sender_of(&header));
        self.0.receiving.store(self.0.state.lock().unwrap().agent.is_some(), Ordering::Relaxed);
        self.0.ensure_session().await.map_err(failed)
    }

    async fn stop_receiving(&self, #[zbus(header)] header: Header<'_>) {
        self.0.state.lock().unwrap().listeners.remove(&sender_of(&header));
    }
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
        .serve_at(PATH, Receiver(sender.clone()))?
        .name(BUS_NAME)?
        .build()
        .await
        .context("org.t2drop on the system bus")?;
    *sender.connection.lock().unwrap() = Some(connection.clone());
    info!("serving {BUS_NAME}");

    // Callers that leave the bus stop browsing and receiving
    let dbus = fdo::DBusProxy::new(&connection).await?;
    let mut owners = dbus.receive_name_owner_changed().await?;
    let core = sender.clone();
    tokio::spawn(async move {
        while let Some(signal) = owners.next().await {
            if let Ok(args) = signal.args() {
                if args.new_owner().is_none() {
                    core.caller_left(args.name().as_str());
                }
            }
        }
    });

    // Sleep waits for the inhibitor, which is released once AWDL is down
    let manager = logind::ManagerProxy::new(&connection).await?;
    let mut sleep_signals = manager.receive_prepare_for_sleep().await?;
    let mut inhibitor = match logind::inhibit_sleep(&connection).await {
        Ok(fd) => Some(fd),
        Err(err) => {
            warn!("no sleep inhibitor: {err:#}");
            None
        }
    };

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if sender.housekeeping().await {
                    info!("idle, exiting");
                    break;
                }
            }
            Some(signal) = sleep_signals.next() => {
                let Ok(args) = signal.args() else { continue };
                if *args.start() {
                    info!("going to sleep, stopping AirDrop");
                    sender.state.lock().unwrap().sleeping = true;
                    sender.stop_all().await;
                    drop(inhibitor.take());
                } else {
                    info!("resumed");
                    sender.state.lock().unwrap().sleeping = false;
                    inhibitor = logind::inhibit_sleep(&connection).await.ok();
                }
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = term.recv() => break,
        }
    }
    sender.stop_all().await;
    if let Err(err) = connection.release_name(BUS_NAME).await {
        warn!("release {BUS_NAME}: {err}");
    }
    Ok(())
}
