// SPDX-License-Identifier: MIT

use std::fs::File;
use std::io::{Read, Write};
use std::net::{Ipv6Addr, SocketAddrV6, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use socket2::{Domain, Protocol, Socket, Type};

use crate::discovery::{FIRST_DYNAMIC_PORT, LAST_DYNAMIC_PORT};
use crate::xpc::{self, Value};

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const DATA: u8 = 0;
const HEADERS: u8 = 1;
const SETTINGS: u8 = 4;
const GOAWAY: u8 = 7;
const WINDOW_UPDATE: u8 = 8;
const MAX_ARCHIVE_SIZE: u64 = 2 * 1024 * 1024 * 1024;
const SCAN_WORKERS: usize = 128;
const HANDSHAKE_WORKERS: usize = 32;

struct Frame {
    kind: u8,
    flags: u8,
    stream: u32,
    payload: Vec<u8>,
}

struct Connection {
    stream: TcpStream,
    last_peer_stream: u32,
}

impl Connection {
    fn connect(interface: &str, host: Ipv6Addr, port: u16) -> Result<Self> {
        Self::connect_with_timeout(
            interface,
            host,
            port,
            Duration::from_secs(3),
            Duration::from_secs(120),
        )
    }

    fn connect_with_timeout(
        interface: &str,
        host: Ipv6Addr,
        port: u16,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> Result<Self> {
        let index = unsafe { libc::if_nametoindex(std::ffi::CString::new(interface)?.as_ptr()) };
        ensure!(index != 0, "unknown interface {interface}");
        let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
        socket
            .bind_device(Some(interface.as_bytes()))
            .context("bind socket to CDC-NCM interface")?;
        socket.set_read_timeout(Some(io_timeout))?;
        socket.set_write_timeout(Some(Duration::from_secs(30)))?;
        socket
            .connect_timeout(
                &SocketAddrV6::new(host, port, 0, index).into(),
                connect_timeout,
            )
            .with_context(|| format!("connect [{host}%{interface}]:{port}"))?;
        Ok(Self {
            stream: socket.into(),
            last_peer_stream: 0,
        })
    }

    fn frame(&mut self, kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Result<()> {
        ensure!(payload.len() <= 0x00ff_ffff, "HTTP/2 frame too large");
        let length = payload.len() as u32;
        let header = [
            (length >> 16) as u8,
            (length >> 8) as u8,
            length as u8,
            kind,
            flags,
            (stream >> 24) as u8 & 0x7f,
            (stream >> 16) as u8,
            (stream >> 8) as u8,
            stream as u8,
        ];
        self.stream.write_all(&header)?;
        self.stream.write_all(payload)?;
        self.stream.flush()?;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<Frame> {
        let mut header = [0u8; 9];
        self.stream.read_exact(&mut header)?;
        let length =
            ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
        let mut payload = vec![0; length];
        self.stream.read_exact(&mut payload)?;
        let stream = u32::from_be_bytes([header[5] & 0x7f, header[6], header[7], header[8]]);
        if stream != 0 && stream.is_multiple_of(2) {
            self.last_peer_stream = self.last_peer_stream.max(stream);
        }
        Ok(Frame {
            kind: header[3],
            flags: header[4],
            stream,
            payload,
        })
    }

    fn start(&mut self) -> Result<()> {
        self.stream.write_all(PREFACE)?;
        let mut settings = Vec::new();
        settings.extend(3u16.to_be_bytes());
        settings.extend(100u32.to_be_bytes());
        settings.extend(4u16.to_be_bytes());
        settings.extend((16u32 * 1024 * 1024).to_be_bytes());
        self.frame(SETTINGS, 0, 0, &settings)?;
        self.window(0, 16 * 1024 * 1024 - 65535)?;
        Ok(())
    }

    fn window(&mut self, stream: u32, increment: u32) -> Result<()> {
        self.frame(
            WINDOW_UPDATE,
            0,
            stream,
            &(increment & 0x7fff_ffff).to_be_bytes(),
        )
    }

    fn handle_control(&mut self, frame: &Frame) -> Result<()> {
        if frame.kind == SETTINGS && frame.flags & 1 == 0 {
            self.frame(SETTINGS, 1, 0, &[])?;
        }
        if frame.kind == GOAWAY {
            bail!("T2 closed the HTTP/2 connection");
        }
        Ok(())
    }

    fn accept_peer_settings(&mut self) -> Result<()> {
        loop {
            let frame = self.read_frame()?;
            if frame.kind == GOAWAY {
                bail!("T2 closed the HTTP/2 connection during handshake");
            }
            if frame.kind == SETTINGS && frame.flags & 1 == 0 {
                self.frame(SETTINGS, 1, 0, &[])?;
                return Ok(());
            }
        }
    }

    fn close(&mut self) -> Result<()> {
        let mut payload = Vec::with_capacity(8);
        payload.extend(self.last_peer_stream.to_be_bytes());
        payload.extend(0u32.to_be_bytes());
        self.frame(GOAWAY, 0, 0, &payload)
    }
}

pub fn discover_service(
    interface: &str,
    host: Ipv6Addr,
    mut progress: impl FnMut(u64, u64),
) -> Result<u16> {
    let next = Arc::new(AtomicU32::new(FIRST_DYNAMIC_PORT.into()));
    let stop = Arc::new(AtomicBool::new(false));
    let (candidate_sender, candidate_receiver) = mpsc::channel();
    let candidate_receiver = Arc::new(Mutex::new(candidate_receiver));
    let (service_sender, service_receiver) = mpsc::channel();

    let mut handshakes = Vec::with_capacity(HANDSHAKE_WORKERS);
    for _ in 0..HANDSHAKE_WORKERS {
        let interface = interface.to_owned();
        let receiver = Arc::clone(&candidate_receiver);
        let sender = service_sender.clone();
        let stop = Arc::clone(&stop);
        handshakes.push(thread::spawn(move || {
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(receiver) = receiver.lock() else {
                    break;
                };
                let port = match receiver.recv() {
                    Ok(port) => port,
                    Err(_) => break,
                };
                drop(receiver);
                if let Ok(found) = discover_service_at(&interface, host, port) {
                    stop.store(true, Ordering::Relaxed);
                    let _ = sender.send(found);
                    break;
                }
            }
        }));
    }
    drop(service_sender);

    let mut scanners = Vec::with_capacity(SCAN_WORKERS);
    for _ in 0..SCAN_WORKERS {
        let interface = interface.to_owned();
        let next = Arc::clone(&next);
        let stop = Arc::clone(&stop);
        let sender = candidate_sender.clone();
        scanners.push(thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let port = next.fetch_add(1, Ordering::Relaxed);
                if port > u32::from(LAST_DYNAMIC_PORT) {
                    break;
                }
                if probe(&interface, host, port as u16) {
                    let _ = sender.send(port as u16);
                }
            }
        }));
    }
    drop(candidate_sender);

    let total = u64::from(LAST_DYNAMIC_PORT) - u64::from(FIRST_DYNAMIC_PORT) + 1;
    let mut service = None;
    loop {
        match service_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(found) => {
                service = Some(found);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let current = u64::from(next.load(Ordering::Relaxed))
            .saturating_sub(u64::from(FIRST_DYNAMIC_PORT))
            .min(total);
        progress(current, total);
    }
    stop.store(true, Ordering::Relaxed);
    for worker in scanners {
        let _ = worker.join();
    }
    for worker in handshakes {
        let _ = worker.join();
    }
    service.ok_or_else(|| anyhow::anyhow!("T2 did not advertise com.apple.sysdiagnose.remote"))
}

fn probe(interface: &str, host: Ipv6Addr, port: u16) -> bool {
    let Ok(mut connection) = Connection::connect_with_timeout(
        interface,
        host,
        port,
        Duration::from_millis(150),
        Duration::from_millis(150),
    ) else {
        return false;
    };
    connection
        .read_frame()
        .is_ok_and(|frame| frame.kind == SETTINGS && frame.stream == 0)
}

fn discover_service_at(interface: &str, host: Ipv6Addr, port: u16) -> Result<u16> {
    let mut connection = Connection::connect_with_timeout(
        interface,
        host,
        port,
        Duration::from_secs(3),
        Duration::from_secs(3),
    )?;
    connection.start()?;
    connection.frame(HEADERS, 4, 1, &[])?;
    connection.frame(DATA, 0, 1, &xpc::wrapper(Some(&Value::Dict(vec![])), 1, 0))?;
    connection.frame(HEADERS, 4, 3, &[])?;
    connection.frame(DATA, 0, 1, &xpc::wrapper(None, 0x201, 0))?;
    connection.frame(DATA, 0, 3, &xpc::wrapper(None, 0x400001, 0))?;
    connection.accept_peer_settings()?;

    let uuid = *uuid::Uuid::new_v4().as_bytes();
    let handshake = Value::Dict(vec![
        ("MessageType".into(), Value::String("Handshake".into())),
        ("MessagingProtocolVersion".into(), Value::U64(7)),
        ("UUID".into(), Value::Uuid(uuid)),
        (
            "Properties".into(),
            Value::Dict(vec![
                (
                    "RemoteXPCVersionFlags".into(),
                    Value::U64(0x0100000000000006),
                ),
                ("SensitivePropertiesVisible".into(), Value::Bool(true)),
            ]),
        ),
        ("Services".into(), Value::Dict(vec![])),
    ]);
    connection.frame(DATA, 0, 1, &xpc::wrapper(Some(&handshake), 0x101, 1))?;

    let mut buffered = Vec::new();
    loop {
        let frame = connection.read_frame()?;
        connection.handle_control(&frame)?;
        if frame.kind != DATA || frame.stream != 1 {
            continue;
        }
        buffered.extend(frame.payload);
        let Ok((_, _, value, used)) = xpc::decode_wrapper(&buffered) else {
            continue;
        };
        buffered.drain(..used);
        let Some(peer) = value else {
            continue;
        };
        let Some(services) = peer.get("Services") else {
            continue;
        };
        let service = services
            .get("com.apple.sysdiagnose.remote")
            .ok_or_else(|| anyhow::anyhow!("peer did not advertise sysdiagnose"))?;
        let port = service
            .get("Port")
            .and_then(|port| {
                port.as_u64()
                    .or_else(|| port.as_str().and_then(|value| value.parse().ok()))
            })
            .ok_or_else(|| anyhow::anyhow!("T2 did not advertise com.apple.sysdiagnose.remote"))?;
        let port = u16::try_from(port).context("invalid sysdiagnose port")?;
        ensure!(
            (FIRST_DYNAMIC_PORT..=LAST_DYNAMIC_PORT).contains(&port),
            "T2 advertised invalid sysdiagnose port {port}"
        );
        connection.close()?;
        return Ok(port);
    }
}

pub fn fetch_sysdiagnose(
    interface: &str,
    host: Ipv6Addr,
    port: u16,
    output: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<()> {
    let mut connection = Connection::connect(interface, host, port)?;
    connection.start()?;
    connection.frame(HEADERS, 4, 1, &[])?;
    connection.frame(DATA, 0, 1, &xpc::wrapper(Some(&Value::Dict(vec![])), 1, 0))?;
    connection.frame(HEADERS, 4, 3, &[])?;
    connection.frame(DATA, 0, 3, &xpc::wrapper(None, 0x400001, 0))?;

    let request = Value::Dict(vec![
        ("MSG_TYPE".into(), Value::U64(1)),
        ("REQUEST_TYPE".into(), Value::U64(1)),
        ("initiatedByRemoteHost".into(), Value::Bool(true)),
    ]);
    let mut requested = false;
    let mut accepted = false;
    let mut file_stream_open = false;
    let mut expected = None;
    let mut received = 0u64;
    let mut buffered = Vec::new();
    let mut file = File::create(output)?;

    loop {
        let frame = connection.read_frame()?;
        connection.handle_control(&frame)?;
        if frame.kind != DATA {
            continue;
        }
        if frame.stream == 1 && !accepted {
            buffered.extend(frame.payload);
            let Ok((_, _, value, used)) = xpc::decode_wrapper(&buffered) else {
                continue;
            };
            buffered.drain(..used);
            if value.is_none() && !requested {
                connection.frame(DATA, 0, 1, &xpc::wrapper(Some(&request), 0x101, 1))?;
                requested = true;
            } else if let Some(reply) = value {
                let Some(response_type) = reply.get("RESPONSE_TYPE").and_then(Value::as_u64) else {
                    continue;
                };
                ensure!(response_type == 1, "sysdiagnose request rejected");
                let size = reply
                    .get("FILE_TX")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("missing FILE_TX in sysdiagnose response"))?;
                ensure!(size <= MAX_ARCHIVE_SIZE, "refusing sysdiagnose size {size}");
                expected = Some(size);
                progress(received, size);
                accepted = file_stream_open;
            }
        } else if frame.stream == 2 && !accepted {
            let (flags, message_id, _, _) = xpc::decode_wrapper(&frame.payload)?;
            if flags == 0x100001 {
                connection.frame(HEADERS, 4, 2, &[])?;
                connection.frame(DATA, 0, 2, &xpc::wrapper(None, 0x200001, message_id))?;
                file_stream_open = true;
                accepted = expected.is_some();
            }
        } else if frame.stream == 2 && accepted {
            if frame.payload.is_empty() {
                ensure!(
                    received == expected.unwrap(),
                    "incomplete sysdiagnose: {received} of {} bytes",
                    expected.unwrap()
                );
                connection.frame(DATA, 1, 2, &[])?;
                connection.frame(DATA, 0, 1, &[])?;
                break;
            }
            file.write_all(&frame.payload)?;
            received += frame.payload.len() as u64;
            progress(received, expected.unwrap());
            connection.window(0, frame.payload.len() as u32)?;
            connection.window(2, frame.payload.len() as u32)?;
            ensure!(
                received <= expected.unwrap(),
                "sysdiagnose exceeded advertised FILE_TX length"
            );
        }
    }
    file.sync_all()?;
    connection.close()?;
    Ok(())
}
