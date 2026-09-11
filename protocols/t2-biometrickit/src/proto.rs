// SPDX-License-Identifier: MIT

//! BiometricKit command encoding and reply decoding.

use anyhow::{Result, bail};
use plist::Value;

pub const MAGIC: u16 = 0x4d42;

pub const CANCEL: u16 = 12;
pub const PROVISIONING_STATE: u16 = 0x10;
pub const LOAD_CALIBRATION: u16 = 0x20;
pub const PRESENCE: u16 = 0x26;
pub const SKS_LOCK_STATE: u16 = 0x27;
pub const MATCH: u16 = 0x04;
pub const READINESS: u16 = 0x53;
pub const USER_IDENTITIES: u16 = 0x42;
pub const RESET_SENSOR: u16 = 2;

/// bridgeOS returns this sentinel instead of data when a reply carries nothing.
pub const NIL_OUTPUT: &str = "d4161201-daf5-4bbd-ae4f-9bf319fabbe0";

pub const EVENT_STATUS: u32 = 0xe3ff_8001;
pub const EVENT_MATCH: u32 = 0xe3ff_8002;
pub const EVENT_STATS: u32 = 0xe3ff_8004;

pub const STATUS_FINGER_DOWN: u32 = 63;
pub const STATUS_FINGER_UP: u32 = 64;

pub const MATCH_INIT_SIZE: usize = 68;
pub const IDENTITY_RECORD_SIZE: usize = 20;
pub const MAX_IDENTITIES: usize = 10;

/// No credential set bound. A handle bridgeOS cannot resolve makes it declare a
/// match lockout and pause the capture, so this is the value for a plain verify.
pub const NO_CREDENTIAL_SET: u32 = 0xffff_ffff;

pub fn command_payload(command: u16, value: u16, data: &[u8], capacity: u32) -> Value {
    let mut bytes = Vec::with_capacity(8 + data.len());
    bytes.extend_from_slice(&MAGIC.to_le_bytes());
    bytes.extend_from_slice(&command.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes.extend_from_slice(data);
    Value::Array(vec![
        Value::Integer(3.into()),
        Value::Integer(0.into()),
        Value::Data(bytes),
        Value::Integer(capacity.into()),
    ])
}

pub fn status(reply: &Value) -> u64 {
    reply
        .as_array()
        .and_then(|items| items.first())
        .and_then(Value::as_unsigned_integer)
        .unwrap_or(u64::MAX)
}

pub fn is_ok(reply: &Value) -> bool {
    status(reply) == 0
}

pub fn data(reply: &Value) -> Result<Vec<u8>> {
    if !is_ok(reply) {
        bail!("BiometricKit status {:#010x}", status(reply) as u32);
    }
    match reply.as_array().and_then(|items| items.get(1)) {
        Some(Value::Data(bytes)) => Ok(bytes.clone()),
        Some(Value::String(text)) if text == NIL_OUTPUT => Ok(Vec::new()),
        _ => bail!("reply carries no data"),
    }
}

/// The match request. Its first eight bytes reach the driver as one quadword,
/// so the flags and the credential-set handle sit side by side.
pub fn match_init(flags: u32, credential_set: u32, identities: &[u8]) -> Vec<u8> {
    let mut payload = vec![0u8; MATCH_INIT_SIZE + identities.len()];
    payload[0..4].copy_from_slice(&flags.to_le_bytes());
    payload[4..8].copy_from_slice(&credential_set.to_le_bytes());
    payload[MATCH_INIT_SIZE..].copy_from_slice(identities);
    payload
}
