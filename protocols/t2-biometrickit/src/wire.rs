// SPDX-License-Identifier: MIT

//! BridgeXPC framing: a small header followed by a binary plist. This is the
//! transport BiometricKit speaks, and it is not the HTTP/2 RemoteXPC used for
//! service discovery.

use std::io::{Cursor, Read, Write};
use std::net::TcpStream;

use anyhow::{Result, bail, ensure};
use plist::Value;

const MAGIC: u16 = 0xb892;
const VERSION: u16 = 1;
pub const HELO: u32 = 1;
pub const MESSAGE: u32 = 2;
const MAX_BODY: u64 = 16 * 1024 * 1024;

pub struct Frame {
    pub kind: u32,
    pub body: Vec<u8>,
}

pub fn send(stream: &mut TcpStream, kind: u32, body: &[u8]) -> Result<()> {
    let mut header = Vec::with_capacity(16);
    header.extend_from_slice(&MAGIC.to_le_bytes());
    header.extend_from_slice(&VERSION.to_le_bytes());
    header.extend_from_slice(&kind.to_le_bytes());
    header.extend_from_slice(&(body.len() as u64).to_le_bytes());
    stream.write_all(&header)?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    Ok(())
}

pub fn receive(stream: &mut TcpStream) -> Result<Frame> {
    let mut header = [0u8; 16];
    stream.read_exact(&mut header)?;
    let magic = u16::from_le_bytes([header[0], header[1]]);
    let version = u16::from_le_bytes([header[2], header[3]]);
    ensure!(magic == MAGIC && version == VERSION, "not a BridgeXPC frame");
    let kind = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let length = u64::from_le_bytes(header[8..16].try_into().unwrap());
    ensure!(length <= MAX_BODY, "BridgeXPC body too large: {length}");
    let mut body = vec![0u8; length as usize];
    stream.read_exact(&mut body)?;
    Ok(Frame { kind, body })
}

pub fn send_plist(stream: &mut TcpStream, value: &Value) -> Result<()> {
    let mut body = Vec::new();
    plist::to_writer_binary(&mut body, value)?;
    send(stream, MESSAGE, &body)
}

pub fn receive_plist(stream: &mut TcpStream) -> Result<Value> {
    let frame = receive(stream)?;
    if frame.kind != MESSAGE {
        bail!("expected a message frame, got {:#x}", frame.kind);
    }
    Ok(Value::from_reader(Cursor::new(frame.body))?)
}

/// The peer opens with a HELO carrying a JSON blob; we answer in kind.
pub fn handshake(stream: &mut TcpStream, process: &str) -> Result<u32> {
    let frame = receive(stream)?;
    ensure!(frame.kind == HELO, "peer did not send HELO");
    let text = String::from_utf8_lossy(&frame.body);
    let version = text
        .split("\"BridgeXPCVersion\"")
        .nth(1)
        .and_then(|rest| rest.split(':').nth(1))
        .map(|rest| rest.trim_start())
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u32>().ok()
        })
        .unwrap_or(0);
    ensure!(version != 0, "invalid peer HELO");
    let reply = format!(
        "{{\"MaxSupportedProtocolVersion\":1,\"OSBuild\":\"Linux\",\
         \"BridgeXPCVersion\":{version},\"ProcessName\":\"{process}\"}}"
    );
    send(stream, HELO, reply.as_bytes())?;
    Ok(version)
}
