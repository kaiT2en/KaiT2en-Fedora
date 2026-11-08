// SPDX-License-Identifier: GPL-3.0-or-later

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

/* ------------------------------------------------------------------------
 * Commands beyond the verify path.
 *
 * Numbers are cross-checked against the bridgeOS log, where every one of
 * the ten commands the verify path uses lines up exactly. The comment on each line is the
 * payload size measured on the wire during a real enrollment.
 *
 * Nothing here is used by the daemon yet. PRESENCE above is this table's
 * FingerDetectMode (38) and SKS_LOCK_STATE is its GetSKSLockState (39).
 * ------------------------------------------------------------------------ */

pub const GET_CMD_PROTOCOL_VERSION: u16 = 1; // in 4, out 4
pub const ENROLL_MODE: u16 = 3; // in 68, out 0
pub const ALIGNMENT_DATA: u16 = 8; // in 0, out 3060
pub const REMOVE_TEMPLATE: u16 = 13;
pub const ENROLL_CONTINUE: u16 = 14; // in 0, out 0
pub const REQUEST_MAX_IDENTITY_COUNT: u16 = 15; // in 0, out 4
pub const REQUEST_TOPOLOGY_OF_TEMPLATE: u16 = 17; // in 20, out 3060
pub const GET_PROTECTED_CONFIG: u16 = 46; // in 4, out 32
pub const GET_ENABLED_FOR_UNLOCK: u16 = 48; // in 0, out 1
pub const GET_SENSOR_INFO: u16 = 53; // in 0, out 12
pub const GET_CATACOMB_ID: u16 = 56; // in 4, out 16
pub const GET_CATACOMB_HASH: u16 = 58; // in 4, out 33
pub const GET_CATACOMB_STATE: u16 = 60; // in 0, out 168
pub const PREPARE_SAVE_CATACOMB: u16 = 61; // in 24, out 4
pub const COMPLETE_SAVE_CATACOMB: u16 = 62; // in 24, out = the sealed blob
pub const CONFIRM_SAVE_CATACOMB: u16 = 63; // in 24, out 0
pub const LOAD_CATACOMB: u16 = 64; // in = the sealed blob, out 0
pub const GET_FREE_IDENTITY_COUNT: u16 = 65; // in 4, out 4
pub const GET_SYSTEM_PROTECTED_CONFIG: u16 = 67; // in 0, out 36
pub const REMOVE_USER_DATA: u16 = 72;
pub const SAVE_BIO_LOCKOUT_RECORD: u16 = 74; // in 0, out 4096
pub const LOAD_BIO_LOCKOUT_RECORD: u16 = 75; // in = the record, out 0
pub const IS_XART_AVAILABLE: u16 = 76; // in 0, out 1
pub const GET_CATACOMB_GROUP_STATE: u16 = 80; // in 0, out 2800
pub const GET_IDENTITY_RECORDS: u16 = 81; // in 0, out 20 per identity
pub const GET_BIO_DEVICE_LIST: u16 = 82; // in 0, out 264
pub const GET_BIO_DEVICE_INFO: u16 = 84; // in 20, out 83

/// Enroll takes the same 68-byte header as match. Proven on the wire:
/// `MatchMode` was seen at 68, 88 and 108 bytes, which is the header plus
/// zero, one and two trailing 20-byte identity records, and `EnrollMode`
/// was seen at exactly 68.
pub const ENROLL_INIT_SIZE: usize = MATCH_INIT_SIZE;

/* The bit values of these match flags are NOT known:
 * kMatchingFlagMatchForUnlock, kMatchingFlagMatchForCredentialSet and
 * kMatchingFlagMatchForExtendEnrollment. Flags 0 is the plain verify and sets
 * none of them, which is why verification needs no keybag. Do not guess the
 * bits: find them before using them. */

pub fn command_payload(command: u16, value: u16, data: &[u8], capacity: u32) -> Value {
    command_payload_versioned(command, 1, value, data, capacity)
}

/// Like `command_payload` but with an explicit header version. The driver
/// asserts `cmd->version == 1` or `== 2` per command; the verify path uses 1,
/// and enroll's 68-byte form (`cmd_enroll_mode_in_v2_t`) is version 2, which
/// is what macOS sends and what the SEP expects for that length.
pub fn command_payload_versioned(
    command: u16,
    version: u16,
    value: u16,
    data: &[u8],
    capacity: u32,
) -> Value {
    let mut bytes = Vec::with_capacity(8 + data.len());
    bytes.extend_from_slice(&MAGIC.to_le_bytes());
    bytes.extend_from_slice(&command.to_le_bytes());
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes.extend_from_slice(data);
    Value::Array(vec![
        Value::Integer(3.into()),
        Value::Integer(0.into()),
        Value::Data(bytes),
        Value::Integer(capacity.into()),
    ])
}

/// The 40-byte `authData` struct that authorises an enrollment or a
/// credential-bound match: `{ u32 usingAuthToken; u32 tokenLength; u8 token[32] }`.
/// Named by the ObjC type encoding `{?="usingAuthToken"I"tokenLength"I"token"[32C]}`
/// in biometrickitd. The token is the 16-byte AppleKeyStore unlock token minted
/// on passcode validation; without it the SEP rejects enroll with -3.
pub const AUTH_DATA_SIZE: usize = 40;
pub const AUTH_TOKEN_CAP: usize = 32;

pub fn auth_data(token: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; AUTH_DATA_SIZE];
    let using = if token.is_empty() { 0u32 } else { 1u32 };
    buf[0..4].copy_from_slice(&using.to_le_bytes());
    buf[4..8].copy_from_slice(&(token.len() as u32).to_le_bytes());
    let n = token.len().min(AUTH_TOKEN_CAP);
    buf[8..8 + n].copy_from_slice(&token[..n]);
    buf
}

/// The 68-byte `cmd_enroll_mode_in_v2_t`, sent as EnrollMode (3), version 2,
/// value 0. Layout confirmed from `performEnrollCommand:` in biometrickitd:
/// flags(0) + userID + authData(40) + deviceGroup(20).
pub const ENROLL_DEVICE_GROUP_SIZE: usize = 20;

/// Output capacity to request for a `CompleteSaveCatacomb` reply. The largest
/// sealed blob observed was the 57879-byte user catacomb, so 64 KiB is ample
/// while staying well under the 16 MiB BridgeXPC body limit.
pub const OOL_MAX: usize = 64 * 1024;

pub fn enroll_init(user_id: u32, token: &[u8], device_group: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; ENROLL_INIT_SIZE];
    // buf[0..4] processedFlags stays 0.
    buf[4..8].copy_from_slice(&user_id.to_le_bytes());
    buf[8..8 + AUTH_DATA_SIZE].copy_from_slice(&auth_data(token));
    let n = device_group.len().min(ENROLL_DEVICE_GROUP_SIZE);
    buf[48..48 + n].copy_from_slice(&device_group[..n]);
    buf
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

/// The status as the SEP actually sent it, including negative error codes.
///
/// `status` above reports `u64::MAX` for anything it cannot read as unsigned,
/// which silently hides a negative code behind a value that merely looks like
/// a failure. That is fine for `is_ok`, but when the code itself is the
/// information, use this.
pub fn status_signed(reply: &Value) -> Option<i64> {
    reply
        .as_array()
        .and_then(|items| items.first())
        .and_then(Value::as_signed_integer)
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
