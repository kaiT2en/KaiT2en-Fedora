// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::Ipv6Addr;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
#[path = "../cli/mod.rs"]
mod cli;
use cli::{Cli, Action, request};
use t2_bridgexpc::discovery;
#[path = "../protocols/ave.rs"]
mod ave;
use ave::Service as RemoteService;

const AVE_SERVICE: &str = "com.apple.aveservice";
const SIDECAR_SERVICE: &str = "com.apple.videoprocessingd.encode.remote";
const SIDECAR_VENDOR: &str = "05ac";
const SIDECAR_PRODUCT: &str = "9312";
/// Upper bound for the T2 to remove the SideCar AVE USB device after the
/// connection closes.
const SIDECAR_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(3);

struct Manager {
    interface: String,
    host: Ipv6Addr,
    desired: HashSet<String>,
    live: HashMap<String, RemoteService>,
    sleeping: bool,
}

impl Manager {
    fn acquire(&mut self, service: String) -> Result<()> {
        self.desired.insert(service.clone());
        if !self.sleeping && !self.live.contains_key(&service) {
            let connection = RemoteService::connect(&self.interface, self.host, &service)?;
            self.live.insert(service, connection);
        }
        Ok(())
    }

    /// Replaces a connection the T2 dropped, for example after jetsam killed
    /// the daemon behind it. The T2 launches the daemon again on connect.
    fn reconnect(&mut self, service: &str) -> Result<()> {
        if self.sleeping || !self.desired.contains(service) {
            return Ok(());
        }
        // The peer is gone, dropping the socket is all that is left to do.
        self.live.remove(service);
        let mut last_error = None;
        for attempt in 0..10 {
            match RemoteService::connect(&self.interface, self.host, service) {
                Ok(connection) => {
                    self.live.insert(service.to_owned(), connection);
                    eprintln!("reconnected {service}");
                    return Ok(());
                }
                Err(error) => last_error = Some(error),
            }
            if attempt != 9 {
                thread::sleep(Duration::from_secs(1));
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("T2 reconnect failed")))
    }

    fn release(&mut self, service: &str) -> Result<()> {
        self.desired.remove(service);
        if let Some(connection) = self.live.remove(service) {
            connection.close(service)?;
        }
        Ok(())
    }

    fn close_live(&mut self) {
        for (name, connection) in self.live.drain() {
            if let Err(error) = connection.close(&name) {
                eprintln!("could not close {name}: {error:#}");
            }
        }
    }

    fn pre_suspend(&mut self) -> Result<()> {
        self.sleeping = true;
        let sidecar = sidecar_device_present();
        self.close_live();
        // The T2 removes the SideCar AVE USB device after the connection
        // closes. A suspend that starts while it does fails in t2bce_vhci
        // with quiesce error 2, so let the removal finish first.
        if sidecar {
            let start = Instant::now();
            while sidecar_device_present() {
                if start.elapsed() >= SIDECAR_TEARDOWN_TIMEOUT {
                    eprintln!("SideCar AVE USB device still present after closing the connection");
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            eprintln!("SideCar AVE USB device removed after {} ms", start.elapsed().as_millis());
        }
        Ok(())
    }

    fn post_resume(&mut self) -> Result<()> {
        // Rebinding removes the old netdev; udev may give the new one a different name.
        self.sleeping = true;
        self.close_live();
        let mut link_error = None;
        for attempt in 0..20 {
            let ready = (|| -> Result<()> {
                self.interface = discovery::interface(None)?;
                discovery::host(&self.interface, Some(self.host.to_string()))?;
                Ok(())
            })();
            match ready {
                Ok(()) => {
                    link_error = None;
                    break;
                }
                Err(error) => link_error = Some(error),
            }
            if attempt != 19 {
                thread::sleep(Duration::from_secs(1));
            }
        }
        if let Some(error) = link_error {
            return Err(error.context("restore T2 NCM link after resume"));
        }
        self.sleeping = false;

        let desired: Vec<_> = self.desired.iter().cloned().collect();
        let mut last_error = None;
        for attempt in 0..20 {
            let mut pending = false;
            for service in &desired {
                if self.live.contains_key(service) {
                    continue;
                }
                match RemoteService::connect(&self.interface, self.host, service) {
                    Ok(connection) => {
                        self.live.insert(service.clone(), connection);
                    }
                    Err(error) => {
                        pending = true;
                        last_error = Some(error);
                    }
                }
            }
            if !pending {
                return Ok(());
            }
            if attempt != 19 {
                thread::sleep(Duration::from_secs(1));
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("T2 reconnect failed")))
    }

    fn status(&self) -> String {
        let mut desired: Vec<_> = self.desired.iter().map(String::as_str).collect();
        desired.sort_unstable();
        let mut live: Vec<_> = self.live.keys().map(String::as_str).collect();
        live.sort_unstable();
        format!(
            "sleeping={} desired={} live={}",
            self.sleeping,
            desired.join(","),
            live.join(",")
        )
    }
}

fn serve(socket: &Path, mut manager: Manager) -> Result<()> {
    if socket.exists() {
        fs::remove_file(socket).context("remove stale control socket")?;
    }
    let listener =
        UnixListener::bind(socket).with_context(|| format!("bind {}", socket.display()))?;

    loop {
        match manager.acquire(AVE_SERVICE.to_owned()) {
            Ok(()) => break,
            Err(error) => {
                eprintln!("waiting for {AVE_SERVICE}: {error:#}");
                thread::sleep(Duration::from_secs(2));
            }
        }
    }

    // The T2 exposes the Sidecar encoder as a USB device only while this
    // connection is held; t2bce_sidecar binds to that device. A failure stays
    // desired, so resume retries it, and must not hold up AVE.
    for attempt in 0..5 {
        match manager.acquire(SIDECAR_SERVICE.to_owned()) {
            Ok(()) => break,
            Err(error) => {
                eprintln!("could not connect {SIDECAR_SERVICE}: {error:#}");
                if attempt != 4 {
                    thread::sleep(Duration::from_secs(2));
                }
            }
        }
    }

    let watch_socket = socket.to_owned();
    thread::spawn(move || watch_sidecar(&watch_socket));

    for incoming in listener.incoming() {
        let mut stream = incoming.context("accept control connection")?;
        let mut line = String::new();
        BufReader::new(stream.try_clone()?).read_line(&mut line)?;
        let line = line.trim();
        let result = if let Some(service) = line.strip_prefix("acquire ") {
            manager.acquire(service.to_owned()).map(|_| "OK".to_owned())
        } else if let Some(service) = line.strip_prefix("reconnect ") {
            manager.reconnect(service).map(|_| "OK".to_owned())
        } else if let Some(service) = line.strip_prefix("release ") {
            manager.release(service).map(|_| "OK".to_owned())
        } else if line == "pre-suspend" {
            manager.pre_suspend().map(|_| "OK".to_owned())
        } else if line == "post-resume" {
            manager.post_resume().map(|_| "OK".to_owned())
        } else if line == "status" {
            Ok(manager.status())
        } else {
            Err(anyhow::anyhow!("unknown command"))
        };
        let reply = match result {
            Ok(reply) => reply,
            Err(error) => format!("ERR {error:#}"),
        };
        // A timed-out suspend caller must not terminate the service manager.
        if let Err(error) = writeln!(stream, "{reply}") {
            eprintln!("could not send control reply: {error}");
        }
    }
    Ok(())
}

fn sidecar_device_present() -> bool {
    let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let read = |name| fs::read_to_string(entry.path().join(name)).unwrap_or_default();
        read("idVendor").trim() == SIDECAR_VENDOR && read("idProduct").trim() == SIDECAR_PRODUCT
    })
}

/// The SideCar AVE USB device disappears when the T2 drops the Sidecar
/// connection, and the daemon gets no other sign of it. Two misses in a row
/// leave the device time to appear after a fresh connect.
fn watch_sidecar(socket: &Path) {
    let mut missing = 0;
    loop {
        thread::sleep(Duration::from_secs(2));
        if sidecar_device_present() {
            missing = 0;
            continue;
        }
        let Ok(status) = query(socket, "status") else {
            continue;
        };
        let expected = status.contains("sleeping=false")
            && status
                .split_whitespace()
                .find_map(|field| field.strip_prefix("live="))
                .is_some_and(|live| live.split(',').any(|name| name == SIDECAR_SERVICE));
        if !expected {
            missing = 0;
            continue;
        }
        missing += 1;
        if missing < 2 {
            continue;
        }
        missing = 0;
        eprintln!("{SIDECAR_SERVICE} lost its USB device, reconnecting");
        if let Err(error) = query(socket, &format!("reconnect {SIDECAR_SERVICE}")) {
            eprintln!("could not reconnect {SIDECAR_SERVICE}: {error:#}");
        }
    }
}

fn query(socket: &Path, command: &str) -> Result<String> {
    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("connect {}", socket.display()))?;
    writeln!(stream, "{command}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    let reply = reply.trim();
    match reply.strip_prefix("ERR ") {
        Some(error) => Err(anyhow::anyhow!("{error}")),
        None => Ok(reply.to_owned()),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Action::Daemon { interface, host } => {
            let (interface, host) = loop {
                let interface = match discovery::interface(interface.clone()) {
                    Ok(interface) => interface,
                    Err(error) => {
                        eprintln!("waiting for T2 CDC-NCM interface: {error:#}");
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                };
                match discovery::host(&interface, host.map(|host| host.to_string())) {
                    Ok(host) => break (interface, host),
                    Err(error) => {
                        eprintln!("waiting for IPv6 on {interface}: {error:#}");
                        thread::sleep(Duration::from_secs(2));
                    }
                }
            };
            eprintln!("using T2 at [{host}%{interface}]");
            serve(
                &cli.socket,
                Manager {
                    interface,
                    host,
                    desired: HashSet::new(),
                    live: HashMap::new(),
                    sleeping: false,
                },
            )
        }
        Action::Acquire { service } => request(&cli.socket, &format!("acquire {service}")),
        Action::Release { service } => request(&cli.socket, &format!("release {service}")),
        Action::PreSuspend => request(&cli.socket, "pre-suspend"),
        Action::PostResume => request(&cli.socket, "post-resume"),
        Action::Status => request(&cli.socket, "status"),
    }
}
