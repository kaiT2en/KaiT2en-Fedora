// SPDX-License-Identifier: MIT

use anyhow::{Result, bail, ensure};

const WRAPPER_MAGIC: u32 = 0x29b00b92;
const XPC_MAGIC: u32 = 0x42133742;
const XPC_VERSION: u32 = 5;
const DICTIONARY: u32 = 0xf000;
const ARRAY: u32 = 0xe000;
const STRING: u32 = 0x9000;
const BOOL: u32 = 0x2000;
const INT64: u32 = 0x3000;
const UINT64: u32 = 0x4000;
const DOUBLE: u32 = 0x5000;
const POINTER: u32 = 0x6000;
const DATE: u32 = 0x7000;
const DATA: u32 = 0x8000;
const UUID: u32 = 0xa000;
const FD: u32 = 0xb000;
const SHMEM: u32 = 0xc000;
const FILE_TRANSFER: u32 = 0x1a000;

#[derive(Clone, Debug)]
pub enum Value {
    Dict(Vec<(String, Value)>),
    Array(Vec<Value>),
    String(String),
    Bool(bool),
    U64(u64),
    I64(i64),
    Uuid([u8; 16]),
    FileTransfer(u64),
    Null,
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        let Self::Dict(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U64(value) | Self::FileTransfer(value) => Some(*value),
            Self::I64(value) => u64::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes());
}
fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend(value.to_le_bytes());
}

fn pad(out: &mut Vec<u8>) {
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn encode_value(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Dict(entries) => {
            push_u32(out, DICTIONARY);
            let length_at = out.len();
            push_u32(out, 0);
            let start = out.len();
            push_u32(out, entries.len() as u32);
            for (key, value) in entries {
                out.extend(key.as_bytes());
                out.push(0);
                pad(out);
                encode_value(value, out);
            }
            let length = (out.len() - start) as u32;
            out[length_at..length_at + 4].copy_from_slice(&length.to_le_bytes());
        }
        Value::Array(entries) => {
            push_u32(out, ARRAY);
            let length_at = out.len();
            push_u32(out, 0);
            let start = out.len();
            push_u32(out, entries.len() as u32);
            for value in entries {
                encode_value(value, out);
            }
            let length = (out.len() - start) as u32;
            out[length_at..length_at + 4].copy_from_slice(&length.to_le_bytes());
        }
        Value::String(value) => {
            push_u32(out, STRING);
            push_u32(out, (value.len() + 1) as u32);
            out.extend(value.as_bytes());
            out.push(0);
            pad(out);
        }
        Value::Bool(value) => {
            push_u32(out, BOOL);
            push_u32(out, u32::from(*value));
        }
        Value::U64(value) => {
            push_u32(out, UINT64);
            push_u64(out, *value);
        }
        Value::I64(value) => {
            push_u32(out, INT64);
            out.extend(value.to_le_bytes());
        }
        Value::Uuid(value) => {
            push_u32(out, UUID);
            out.extend(value);
        }
        Value::Null => push_u32(out, 0x1000),
        Value::FileTransfer(_) => unreachable!("file transfers are only decoded"),
    }
}

pub fn wrapper(value: Option<&Value>, flags: u32, message_id: u64) -> Vec<u8> {
    let mut payload = Vec::new();
    if let Some(value) = value {
        push_u32(&mut payload, XPC_MAGIC);
        push_u32(&mut payload, XPC_VERSION);
        encode_value(value, &mut payload);
    }
    let mut out = Vec::with_capacity(24 + payload.len());
    push_u32(&mut out, WRAPPER_MAGIC);
    push_u32(&mut out, flags);
    push_u64(&mut out, payload.len() as u64);
    push_u64(&mut out, message_id);
    out.extend(payload);
    out
}

fn take<'a>(data: &mut &'a [u8], count: usize) -> Result<&'a [u8]> {
    ensure!(data.len() >= count, "truncated XPC object");
    let (head, tail) = data.split_at(count);
    *data = tail;
    Ok(head)
}
fn u32le(data: &mut &[u8]) -> Result<u32> {
    Ok(u32::from_le_bytes(take(data, 4)?.try_into()?))
}
fn u64le(data: &mut &[u8]) -> Result<u64> {
    Ok(u64::from_le_bytes(take(data, 8)?.try_into()?))
}

fn decode_value(data: &mut &[u8]) -> Result<Value> {
    let kind = u32le(data)?;
    Ok(match kind {
        DICTIONARY => {
            let length = u32le(data)? as usize;
            let mut body = take(data, length)?;
            let count = u32le(&mut body)?;
            let mut entries = Vec::new();
            for _ in 0..count {
                let end = body
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| anyhow::anyhow!("unterminated XPC key"))?;
                let key = String::from_utf8(take(&mut body, end)?.to_vec())?;
                take(&mut body, 1)?;
                let padding = (4 - ((end + 1) % 4)) % 4;
                take(&mut body, padding)?;
                entries.push((key, decode_value(&mut body)?));
            }
            Value::Dict(entries)
        }
        ARRAY => {
            let length = u32le(data)? as usize;
            let mut body = take(data, length)?;
            let count = u32le(&mut body)?;
            let mut entries = Vec::new();
            for _ in 0..count {
                entries.push(decode_value(&mut body)?);
            }
            Value::Array(entries)
        }
        STRING => {
            let length = u32le(data)? as usize;
            let raw = take(data, (length + 3) & !3)?;
            Value::String(String::from_utf8(raw[..length.saturating_sub(1)].to_vec())?)
        }
        BOOL => Value::Bool(u32le(data)? != 0),
        UINT64 => Value::U64(u64le(data)?),
        INT64 => Value::I64(u64le(data)? as i64),
        DOUBLE | DATE => {
            take(data, 8)?;
            Value::Null
        }
        POINTER => Value::Null,
        DATA => {
            let length = u32le(data)? as usize;
            take(data, (length + 3) & !3)?;
            Value::Null
        }
        UUID => Value::Uuid(take(data, 16)?.try_into()?),
        FD => {
            take(data, 4)?;
            Value::Null
        }
        SHMEM => {
            take(data, 8)?;
            Value::Null
        }
        FILE_TRANSFER => {
            let _message_id = u64le(data)?;
            let value = decode_value(data)?;
            Value::FileTransfer(
                value
                    .get("s")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("invalid XPC file transfer"))?,
            )
        }
        0x1000 => Value::Null,
        other => bail!("unsupported XPC type {other:#x}"),
    })
}

pub fn decode_wrapper(data: &[u8]) -> Result<(u32, u64, Option<Value>, usize)> {
    ensure!(data.len() >= 24, "truncated XPC wrapper");
    let mut header = data;
    ensure!(
        u32le(&mut header)? == WRAPPER_MAGIC,
        "invalid XPC wrapper magic"
    );
    let flags = u32le(&mut header)?;
    let length = u64le(&mut header)? as usize;
    let message_id = u64le(&mut header)?;
    ensure!(data.len() >= 24 + length, "fragmented XPC wrapper");
    if length == 0 {
        return Ok((flags, message_id, None, 24));
    }
    let mut payload = &data[24..24 + length];
    ensure!(
        u32le(&mut payload)? == XPC_MAGIC && u32le(&mut payload)? == XPC_VERSION,
        "invalid XPC payload"
    );
    Ok((
        flags,
        message_id,
        Some(decode_value(&mut payload)?),
        24 + length,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_round_trip() {
        let value = Value::Dict(vec![
            ("name".into(), Value::String("bridgeOS".into())),
            ("size".into(), Value::U64(1234)),
            ("ready".into(), Value::Bool(true)),
        ]);
        let encoded = wrapper(Some(&value), 0x101, 7);
        let (flags, message_id, decoded, used) = decode_wrapper(&encoded).unwrap();
        assert_eq!(flags, 0x101);
        assert_eq!(message_id, 7);
        assert_eq!(used, encoded.len());
        let decoded = decoded.unwrap();
        assert_eq!(
            decoded.get("name").and_then(|value| match value {
                Value::String(value) => Some(value.as_str()),
                _ => None,
            }),
            Some("bridgeOS")
        );
        assert_eq!(decoded.get("size").and_then(Value::as_u64), Some(1234));
    }

    #[test]
    fn empty_ack_can_be_drained_before_next_message() {
        let peer = Value::Dict(vec![("Services".into(), Value::Dict(vec![]))]);
        let mut stream = wrapper(None, 1, 0);
        stream.extend(wrapper(Some(&peer), 0x101, 1));

        let (_, _, value, used) = decode_wrapper(&stream).unwrap();
        assert!(value.is_none());
        stream.drain(..used);

        let (_, _, value, used) = decode_wrapper(&stream).unwrap();
        assert_eq!(used, stream.len());
        assert!(value.unwrap().get("Services").is_some());
    }
}
