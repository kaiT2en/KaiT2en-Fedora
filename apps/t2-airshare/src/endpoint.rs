// SPDX-License-Identifier: GPL-3.0-or-later
//! Endpoint identity: the mDNS instance name and the "endpoint info" blob
//! that carries device type and name, see NearDrop PROTOCOL.md.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use rand::distributions::Alphanumeric;

/// Device types Android uses to pick an icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceType {
    Unknown = 0,
    Phone = 1,
    Tablet = 2,
    Laptop = 3,
}

impl DeviceType {
    pub fn from_bits(bits: u8) -> Self {
        match bits {
            1 => Self::Phone,
            2 => Self::Tablet,
            3 => Self::Laptop,
            _ => Self::Unknown,
        }
    }
}

/// Four random alphanumeric characters that identify this device.
pub fn random_endpoint_id() -> [u8; 4] {
    let mut id = [0u8; 4];
    for (byte, c) in id.iter_mut().zip(rand::thread_rng().sample_iter(&Alphanumeric)) {
        *byte = c;
    }
    id
}

/// mDNS instance name: 0x23, endpoint id, service id `fc 9f 5e`, two zero
/// bytes, URL-safe base64.
pub fn mdns_instance_name(endpoint_id: [u8; 4]) -> String {
    let mut name = vec![0x23];
    name.extend_from_slice(&endpoint_id);
    name.extend_from_slice(&[0xfc, 0x9f, 0x5e, 0x00, 0x00]);
    URL_SAFE_NO_PAD.encode(name)
}

/// Endpoint info for a visible device: bit field with the device type, 16
/// random bytes standing in for the certificate metadata, then the name with
/// a one byte length prefix.
pub fn endpoint_info(device_type: DeviceType, name: &str) -> Vec<u8> {
    let mut info = vec![(device_type as u8) << 1];
    info.extend(rand::thread_rng().r#gen::<[u8; 16]>());
    let name = truncate_utf8(name, u8::MAX as usize);
    info.push(name.len() as u8);
    info.extend_from_slice(name.as_bytes());
    info
}

/// Device type and name from a peer's endpoint info. The name is missing
/// when the peer is hidden.
pub fn parse_endpoint_info(info: &[u8]) -> Option<(DeviceType, Option<String>)> {
    let first = *info.first()?;
    let device_type = DeviceType::from_bits((first >> 1) & 0x07);
    let hidden = first & 0x10 != 0;
    if hidden || info.len() < 18 {
        return Some((device_type, None));
    }
    let len = info[17] as usize;
    let name = info.get(18..18 + len).map(|n| String::from_utf8_lossy(n).into_owned());
    Some((device_type, name))
}

fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_info_round_trip() {
        let info = endpoint_info(DeviceType::Laptop, "MacBook Pro");
        let (device_type, name) = parse_endpoint_info(&info).unwrap();
        assert_eq!(device_type, DeviceType::Laptop);
        assert_eq!(name.as_deref(), Some("MacBook Pro"));
    }

    #[test]
    fn instance_name_layout() {
        let name = mdns_instance_name(*b"AbCd");
        let raw = URL_SAFE_NO_PAD.decode(name).unwrap();
        assert_eq!(raw, [0x23, b'A', b'b', b'C', b'd', 0xfc, 0x9f, 0x5e, 0, 0]);
    }
}
