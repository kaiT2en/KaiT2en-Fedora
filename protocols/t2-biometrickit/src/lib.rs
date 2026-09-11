// SPDX-License-Identifier: MIT

//! BiometricKit protocol for the Apple T2 Touch ID sensor.
//!
//! The sensor hangs on the SEP endpoint `sbio` inside bridgeOS and is reachable
//! only over BridgeXPC on the CDC-NCM link, never through the host's SEP
//! mailbox. This crate speaks that protocol and makes no policy decisions: it
//! reports which enrolled finger matched, nothing more.

pub mod proto;
pub mod wire;

use std::net::{Ipv6Addr, SocketAddrV6, TcpStream};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use plist::Value;
use t2_bridgexpc::{discovery, remote};

pub const SERVICE: &str = "com.apple.eos.BiometricKit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub slot: usize,
    pub user_id: u32,
    pub uuid: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    FingerDown,
    FingerUp,
    Status(u32),
    Statistics(usize),
    /// The SEP's verdict. `slot` names the enrolled finger that matched.
    MatchResult { slot: Option<usize>, bytes: usize },
    Other { kind: u32, bytes: usize },
}

pub struct Session {
    stream: TcpStream,
    identities: Vec<Identity>,
    records: Vec<u8>,
}

impl Session {
    /// Find the link, activate BiometricKit and bring the sensor up to the
    /// point where it can scan. Without the calibration load the sensor stays
    /// dark, so it is part of opening a session rather than a separate step.
    pub fn open(interface: Option<String>, host: Option<String>) -> Result<Self> {
        let interface = discovery::interface(interface)?;
        let host = discovery::host(&interface, host)?;
        let service = remote::discover_direct_named_service(&interface, host, SERVICE)
            .context("BiometricKit was not advertised")?;
        let mut session = Self::connect(&interface, host, service.service_port)?;
        session.start()?;
        Ok(session)
    }

    pub fn connect(interface: &str, host: Ipv6Addr, port: u16) -> Result<Self> {
        let scope = unsafe {
            let name = std::ffi::CString::new(interface)?;
            libc::if_nametoindex(name.as_ptr())
        };
        ensure!(scope != 0, "unknown interface {interface}");
        let address = SocketAddrV6::new(host, port, 0, scope);
        let stream = TcpStream::connect_timeout(&address.into(), Duration::from_secs(3))?;
        stream.set_nodelay(true)?;
        let mut stream = stream;
        wire::handshake(&mut stream, "t2-touchid")?;
        Ok(Self { stream, identities: Vec::new(), records: Vec::new() })
    }

    fn start(&mut self) -> Result<()> {
        let opened = self.request(Value::Array(vec![Value::Integer(1.into())]))?;
        ensure!(proto::is_ok(&opened), "BiometricKit service did not open");
        self.command(proto::RESET_SENSOR, 2, &[], 0)?;
        self.command(proto::CANCEL, 0, &[], 0)?;

        let ready = self.command(proto::READINESS, 0, &[], 1)?;
        let ready = proto::data(&ready)?;
        ensure!(ready.first() == Some(&1), "sensor is not ready");

        let calibration = self.request(Value::Array(vec![Value::Integer(11.into())]))?;
        let calibration = match calibration.as_array().and_then(|items| items.first()) {
            Some(Value::Data(bytes)) if !bytes.is_empty() => bytes.clone(),
            _ => bail!("bridgeOS returned no FDR calibration"),
        };
        let loaded = self.command(proto::LOAD_CALIBRATION, 3, &calibration, 0)?;
        ensure!(proto::is_ok(&loaded), "calibration load rejected");

        let version = Value::Array(vec![Value::Integer(10.into()), Value::Integer(2.into())]);
        ensure!(proto::is_ok(&self.request(version)?), "setClientVersion failed");
        Ok(())
    }

    /// Enrolled fingers for a macOS user id, in slot order.
    pub fn read_identities(&mut self, user_id: u32) -> Result<&[Identity]> {
        let capacity = (proto::IDENTITY_RECORD_SIZE * proto::MAX_IDENTITIES) as u32;
        let reply = self.command(
            proto::USER_IDENTITIES,
            0,
            &user_id.to_le_bytes(),
            capacity,
        )?;
        let records = proto::data(&reply).unwrap_or_default();
        ensure!(
            records.len() % proto::IDENTITY_RECORD_SIZE == 0,
            "malformed identity inventory: {} bytes",
            records.len()
        );
        self.identities.clear();
        for (slot, record) in records.chunks_exact(proto::IDENTITY_RECORD_SIZE).enumerate() {
            let prefix = u32::from_le_bytes(record[0..4].try_into().unwrap());
            let suffix = u32::from_le_bytes(record[16..20].try_into().unwrap());
            let uuid = if prefix == user_id {
                &record[4..20]
            } else if suffix == user_id {
                &record[0..16]
            } else {
                continue;
            };
            self.identities.push(Identity {
                slot,
                user_id,
                uuid: uuid.try_into().unwrap(),
            });
        }
        self.records = records;
        Ok(&self.identities)
    }

    pub fn identities(&self) -> &[Identity] {
        &self.identities
    }

    pub fn provisioning_state(&mut self, user_id: u32) -> Result<u32> {
        let reply = self.command(proto::PROVISIONING_STATE, 0, &[], 64)?;
        let bytes = match proto::data(&reply) {
            Ok(bytes) if bytes.len() >= 4 => bytes,
            _ => proto::data(&self.command(
                proto::PROVISIONING_STATE,
                0,
                &user_id.to_le_bytes(),
                64,
            )?)?,
        };
        ensure!(bytes.len() >= 4, "short provisioning state");
        Ok(u32::from_le_bytes(bytes[0..4].try_into().unwrap()))
    }

    pub fn sks_lock_state(&mut self, user_id: u32) -> Result<u32> {
        let reply = self.command(proto::SKS_LOCK_STATE, 0, &user_id.to_le_bytes(), 4)?;
        let bytes = proto::data(&reply)?;
        ensure!(bytes.len() >= 4, "short lock state");
        Ok(u32::from_le_bytes(bytes[0..4].try_into().unwrap()))
    }

    pub fn start_presence(&mut self) -> Result<()> {
        let reply = self.command(proto::PRESENCE, 0, &[], 0)?;
        ensure!(proto::is_ok(&reply), "presence request rejected");
        Ok(())
    }

    /// Start a verify-only match against the identities read earlier.
    pub fn start_match(&mut self, flags: u32, credential_set: u32) -> Result<()> {
        let payload = proto::match_init(flags, credential_set, &self.records);
        let reply = self.command(proto::MATCH, 0, &payload, 0)?;
        if !proto::is_ok(&reply) {
            bail!("match rejected: status {:#010x}", proto::status(&reply) as u32);
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<()> {
        self.command(proto::CANCEL, 0, &[], 0)?;
        Ok(())
    }

    /// Wait for the next sensor event, acknowledging it as bridgeOS expects.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<Event>> {
        self.stream.set_read_timeout(Some(timeout))?;
        let incoming = match wire::receive_plist(&mut self.stream) {
            Ok(value) => value,
            Err(error) => {
                if let Some(io) = error.downcast_ref::<std::io::Error>()
                    && matches!(
                        io.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    )
                {
                    return Ok(None);
                }
                return Err(error);
            }
        };
        let (is_reply, id, body) = split_envelope(&incoming)?;
        ensure!(!is_reply, "unexpected reply while waiting for events");
        self.acknowledge(&id)?;
        Ok(Some(self.decode_event(&body)))
    }

    fn command(&mut self, command: u16, value: u16, data: &[u8], capacity: u32) -> Result<Value> {
        self.request(proto::command_payload(command, value, data, capacity))
    }

    /// Send one request and pump the event stream until its reply arrives.
    /// bridgeOS expects every event to be acknowledged, so they cannot simply
    /// be dropped while waiting.
    fn request(&mut self, payload: Value) -> Result<Value> {
        let id = uuid::Uuid::new_v4().to_string().to_uppercase();
        wire::send_plist(&mut self.stream, &envelope(&id, false, payload))?;
        self.stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        loop {
            let incoming = wire::receive_plist(&mut self.stream)?;
            let (is_reply, received, body) = split_envelope(&incoming)?;
            if is_reply {
                ensure!(received == id, "reply for a different request");
                return Ok(body);
            }
            self.acknowledge(&received)?;
        }
    }

    fn acknowledge(&mut self, id: &str) -> Result<()> {
        let ack = envelope(id, true, Value::Array(vec![Value::Integer(0.into())]));
        wire::send_plist(&mut self.stream, &ack)
    }

    fn decode_event(&self, body: &Value) -> Event {
        let Some(items) = body.as_array() else {
            return Event::Other { kind: 0, bytes: 0 };
        };
        let method = items.first().and_then(Value::as_unsigned_integer);
        let Some(Value::Data(data)) = items.get(2) else {
            return Event::Other { kind: 0, bytes: 0 };
        };
        if method != Some(9) || data.len() < 24 {
            return Event::Other { kind: 0, bytes: data.len() };
        }
        let kind = u32::from_le_bytes(data[8..12].try_into().unwrap());
        let payload = &data[24..];
        match kind {
            proto::EVENT_STATUS if payload.len() >= 4 => {
                match u32::from_le_bytes(payload[0..4].try_into().unwrap()) {
                    proto::STATUS_FINGER_DOWN => Event::FingerDown,
                    proto::STATUS_FINGER_UP => Event::FingerUp,
                    code => Event::Status(code),
                }
            }
            proto::EVENT_STATS => Event::Statistics(payload.len()),
            proto::EVENT_MATCH => Event::MatchResult {
                slot: self.matched_slot(payload),
                bytes: payload.len(),
            },
            _ => Event::Other { kind, bytes: payload.len() },
        }
    }

    /// The result record carries the matched identity's UUID somewhere inside
    /// it; find which enrolled finger that is.
    fn matched_slot(&self, payload: &[u8]) -> Option<usize> {
        self.identities.iter().position(|identity| {
            payload
                .windows(16)
                .any(|window| window == identity.uuid)
        })
    }
}

fn envelope(id: &str, reply: bool, payload: Value) -> Value {
    Value::Array(vec![
        Value::Integer(1.into()),
        Value::Boolean(reply),
        Value::String(id.to_owned()),
        payload,
    ])
}

fn split_envelope(value: &Value) -> Result<(bool, String, Value)> {
    let items = value.as_array().context("envelope is not an array")?;
    ensure!(items.len() == 4, "malformed envelope");
    let reply = items[1].as_boolean().context("envelope flag missing")?;
    let id = items[2].as_string().context("envelope id missing")?.to_owned();
    Ok((reply, id, items[3].clone()))
}
