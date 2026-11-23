// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! DTX message framing for Apple's Instruments dtservicehub service.
//!
//! dtservicehub speaks the plain-TCP "DTXMessage" protocol (no RemoteXPC
//! wrapper). Each message has a 32-byte header, an auxiliary argument block and
//! an NSKeyedArchiver payload. Channels multiplex selector calls; the server
//! streams reply messages tagged with the originating conversation index.
//!
//! This is a read/diagnostics client against Apple's own profiling service on
//! the local T2 over the CDC-NCM link. It only issues the service's documented
//! selectors.

use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv6Addr, Shutdown, SocketAddrV6, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Set by the CLI's SIGINT/SIGTERM handler. A blocked read notices it within the
/// poll interval and unwinds, so streaming stops cleanly and `Drop` sends the
/// FIN. The GUI drives per-connection stop via `Dtx::stop_handle` instead.
pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// True once a signal asked us to stop (process-wide; CLI only).
pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

use anyhow::{Context, Result, bail, ensure};
use socket2::{Domain, Protocol, Socket, Type};

use crate::archive::{self, Obj};

const MAGIC: u32 = 0x1f3d_5b79;
const EXPECTS_REPLY: u32 = 0x1000;
const MESSAGE_TYPE_INVOKE: u32 = 2;
const AUX_PREAMBLE: u64 = 0x1f0;

/// A selector argument placed in the auxiliary block.
pub enum Arg {
    Obj(Obj),
    I32(u32),
    I64(u64),
}

/// What a received message carried as its primary payload.
pub enum Payload {
    None,
    Obj(Obj),
    Raw(Vec<u8>),
}

pub struct Message {
    pub ident: u32,
    pub conversation: u32,
    pub channel: i32,
    pub flags: u32,
    pub aux: Vec<Obj>,
    pub payload: Payload,
}

impl Message {
    pub fn selector(&self) -> Option<&str> {
        match &self.payload {
            Payload::Obj(o) => o.as_str(),
            _ => None,
        }
    }
    pub fn object(&self) -> Option<&Obj> {
        match &self.payload {
            Payload::Obj(o) => Some(o),
            _ => None,
        }
    }
    pub fn raw(&self) -> Option<&[u8]> {
        match &self.payload {
            Payload::Raw(bytes) => Some(bytes),
            _ => None,
        }
    }
}

pub struct Dtx {
    stream: TcpStream,
    ident: u32,
    partial: HashMap<u32, Vec<u8>>,
    opened: HashSet<i32>,
    /// Per-connection stop: set it to make a blocked read unwind promptly.
    stop: Arc<AtomicBool>,
    pub capabilities: Obj,
}

impl Drop for Dtx {
    fn drop(&mut self) {
        // Send a FIN so the hub reaps our connection promptly instead of
        // waiting on TCP keepalive. A half-open connection is what wedges the
        // bridgeOS diagnostic backend.
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Dtx {
    /// Connect, perform the capability handshake, and keep what the hub
    /// advertises in `capabilities`.
    pub fn connect(interface: &str, host: Ipv6Addr, port: u16) -> Result<Self> {
        Self::connect_with_stop(interface, host, port, Arc::new(AtomicBool::new(false)))
    }

    /// Like `connect`, but with a caller-owned stop flag so a background thread
    /// can interrupt a blocked read (used by the GUI's per-tab session worker).
    pub fn connect_with_stop(
        interface: &str,
        host: Ipv6Addr,
        port: u16,
        stop: Arc<AtomicBool>,
    ) -> Result<Self> {
        let index = unsafe { libc::if_nametoindex(std::ffi::CString::new(interface)?.as_ptr()) };
        ensure!(index != 0, "unknown interface {interface}");
        let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
        socket.bind_device(Some(interface.as_bytes()))?;
        // Short poll timeout; read_exact retries on WouldBlock up to its own
        // deadline, so a slow or fragmented reply never aborts a command.
        socket.set_read_timeout(Some(Duration::from_secs(2)))?;
        socket
            .connect_timeout(
                &SocketAddrV6::new(host, port, 0, index).into(),
                Duration::from_secs(3),
            )
            .with_context(|| format!("connect [{host}%{interface}]:{port}"))?;
        let mut dtx = Self {
            stream: socket.into(),
            ident: 0,
            partial: HashMap::new(),
            opened: HashSet::new(),
            stop,
            capabilities: Obj::Null,
        };
        let ours = Obj::Dict(vec![
            ("com.apple.private.DTXBlockCompression".into(), Obj::Int(0)),
            ("com.apple.private.DTXConnection".into(), Obj::Int(1)),
        ]);
        dtx.send(0, Some("_notifyOfPublishedCapabilities:"), &[Arg::Obj(ours)], false)?;
        loop {
            let message = dtx.recv()?;
            if message.selector() == Some("_notifyOfPublishedCapabilities:") {
                dtx.capabilities = message.aux.into_iter().next().unwrap_or(Obj::Null);
                return Ok(dtx);
            }
        }
    }

    /// A handle to this connection's stop flag. Setting it makes a blocked
    /// read unwind within the poll interval, for a clean background teardown.
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    /// Send a selector on a channel and return the identifier the reply carries.
    pub fn send(
        &mut self,
        channel: i32,
        selector: Option<&str>,
        args: &[Arg],
        expects_reply: bool,
    ) -> Result<u32> {
        self.ident += 1;
        let ident = self.ident;
        let aux = encode_aux(args);
        let payload = selector
            .map(|name| archive::encode(&Obj::Str(name.into())))
            .unwrap_or_default();

        let mut body = Vec::new();
        let mut flags = MESSAGE_TYPE_INVOKE;
        if expects_reply {
            flags |= EXPECTS_REPLY;
        }
        body.extend(flags.to_le_bytes());
        body.extend((aux.len() as u32).to_le_bytes());
        body.extend(((aux.len() + payload.len()) as u64).to_le_bytes());
        body.extend(&aux);
        body.extend(&payload);

        let mut header = Vec::with_capacity(32);
        header.extend(MAGIC.to_le_bytes());
        header.extend(32u32.to_le_bytes());
        header.extend(0u16.to_le_bytes()); // fragment index
        header.extend(1u16.to_le_bytes()); // fragment count
        header.extend((body.len() as u32).to_le_bytes());
        header.extend(ident.to_le_bytes());
        header.extend(0u32.to_le_bytes()); // conversation index
        header.extend((channel as u32).to_le_bytes());
        header.extend(u32::from(expects_reply).to_le_bytes());

        self.stream.write_all(&header)?;
        self.stream.write_all(&body)?;
        self.stream.flush()?;
        Ok(ident)
    }

    /// Open a channel by code and service name. Idempotent: opening a code that
    /// is already open is a no-op, so several queries can share one connection.
    pub fn open_channel(&mut self, code: i32, name: &str) -> Result<()> {
        if !self.opened.insert(code) {
            return Ok(());
        }
        let ident = self.send(
            0,
            Some("_requestChannelWithCode:identifier:"),
            &[Arg::I32(code as u32), Arg::Obj(Obj::Str(name.into()))],
            true,
        )?;
        self.wait(ident)?;
        Ok(())
    }

    /// Call a selector on a channel and return its reply message.
    pub fn call(&mut self, channel: i32, selector: &str, args: &[Arg]) -> Result<Message> {
        let ident = self.send(channel, Some(selector), args, true)?;
        self.wait(ident)
    }

    /// Read until a reply with the given identifier and a conversation index
    /// (i.e. the answer, not an unsolicited stream message) arrives.
    pub fn wait(&mut self, ident: u32) -> Result<Message> {
        loop {
            let message = self.recv()?;
            if message.ident == ident && message.conversation != 0 {
                return Ok(message);
            }
        }
    }

    /// Read exactly `len` bytes, retrying on the poll timeout until no data has
    /// arrived for `RECV_DEADLINE`. The service sends large replies in bursts,
    /// so a transient EAGAIN between bursts must not fail the read.
    fn read_exact(&mut self, len: usize) -> Result<Vec<u8>> {
        const RECV_DEADLINE: Duration = Duration::from_secs(15);
        let mut buffer = vec![0u8; len];
        let mut filled = 0;
        let mut deadline = Instant::now() + RECV_DEADLINE;
        while filled < len {
            match self.stream.read(&mut buffer[filled..]) {
                Ok(0) => bail!("dtservicehub closed the connection"),
                Ok(count) => {
                    filled += count;
                    deadline = Instant::now() + RECV_DEADLINE;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) =>
                {
                    ensure!(
                        !interrupted() && !self.stop.load(Ordering::Relaxed),
                        "interrupted"
                    );
                    ensure!(Instant::now() < deadline, "timed out waiting for dtservicehub");
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(buffer)
    }

    /// Read one logical message, reassembling fragments.
    pub fn recv(&mut self) -> Result<Message> {
        loop {
            let header = self.read_exact(32)?;
            ensure!(
                u32::from_le_bytes(header[0..4].try_into().unwrap()) == MAGIC,
                "bad DTX magic"
            );
            let fragment_index = u16::from_le_bytes(header[8..10].try_into().unwrap());
            let fragment_count = u16::from_le_bytes(header[10..12].try_into().unwrap());
            let length = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
            let ident = u32::from_le_bytes(header[16..20].try_into().unwrap());
            let conversation = u32::from_le_bytes(header[20..24].try_into().unwrap());
            let channel = i32::from_le_bytes(header[24..28].try_into().unwrap());

            // The first fragment of a multi-fragment message is header-only.
            if fragment_count > 1 && fragment_index == 0 {
                continue;
            }
            let chunk = self.read_exact(length)?;
            let data = if fragment_count <= 1 {
                chunk
            } else {
                let buffer = self.partial.entry(ident).or_default();
                buffer.extend(chunk);
                if fragment_index + 1 != fragment_count {
                    continue;
                }
                self.partial.remove(&ident).unwrap_or_default()
            };
            if std::env::var_os("DTX_DEBUG").is_some() {
                eprintln!(
                    "recv ident={ident} conv={conversation} chan={channel} fi={fragment_index}/{fragment_count} len={}",
                    data.len()
                );
            }
            return parse_body(ident, conversation, channel, &data);
        }
    }
}

fn parse_body(ident: u32, conversation: u32, channel: i32, data: &[u8]) -> Result<Message> {
    ensure!(data.len() >= 16, "DTX body too short");
    let flags = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let aux_len = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let total = u64::from_le_bytes(data[8..16].try_into().unwrap()) as usize;
    let aux = decode_aux(&data[16..16 + aux_len.min(data.len() - 16)]);
    let payload_bytes = &data[16 + aux_len..(16 + total).min(data.len())];
    let payload = if payload_bytes.is_empty() {
        Payload::None
    } else {
        match archive::decode(payload_bytes) {
            Ok(obj) => Payload::Obj(obj),
            Err(_) => Payload::Raw(payload_bytes.to_vec()),
        }
    };
    Ok(Message { ident, conversation, channel, flags, aux, payload })
}

fn encode_aux(args: &[Arg]) -> Vec<u8> {
    if args.is_empty() {
        return Vec::new();
    }
    let mut body = Vec::new();
    for arg in args {
        body.extend(0x0a_u32.to_le_bytes()); // argument separator
        match arg {
            Arg::Obj(obj) => {
                let encoded = archive::encode(obj);
                body.extend(2u32.to_le_bytes());
                body.extend((encoded.len() as u32).to_le_bytes());
                body.extend(encoded);
            }
            Arg::I32(value) => {
                body.extend(3u32.to_le_bytes());
                body.extend(value.to_le_bytes());
            }
            Arg::I64(value) => {
                body.extend(4u32.to_le_bytes());
                body.extend(value.to_le_bytes());
            }
        }
    }
    let mut out = Vec::with_capacity(16 + body.len());
    out.extend(AUX_PREAMBLE.to_le_bytes());
    out.extend((body.len() as u64).to_le_bytes());
    out.extend(body);
    out
}

fn decode_aux(data: &[u8]) -> Vec<Obj> {
    let mut out = Vec::new();
    let mut offset = 16usize; // skip preamble + length
    while offset + 8 <= data.len() {
        let kind = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
        offset += 8;
        match kind {
            2 => {
                if offset + 4 > data.len() {
                    break;
                }
                let len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
                offset += 4;
                if offset + len > data.len() {
                    break;
                }
                out.push(archive::decode(&data[offset..offset + len]).unwrap_or(Obj::Null));
                offset += len;
            }
            3 => {
                if offset + 4 > data.len() {
                    break;
                }
                out.push(Obj::UInt(
                    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()).into(),
                ));
                offset += 4;
            }
            4 => {
                if offset + 8 > data.len() {
                    break;
                }
                out.push(Obj::UInt(u64::from_le_bytes(
                    data[offset..offset + 8].try_into().unwrap(),
                )));
                offset += 8;
            }
            _ => break,
        }
    }
    out
}
