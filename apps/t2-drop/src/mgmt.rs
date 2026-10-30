// SPDX-License-Identifier: GPL-3.0-or-later
//! The Bluetooth management interface of the kernel, for the LE scan
//! timing of discovery. The kernel scans 11.25 ms out of every 11.25 ms
//! while discovering, fine for a short device search but not for looking
//! for AirDrop senders all the time. The setting applies to every
//! discovery of the adapter until it is set back.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use anyhow::{Context, Result, bail};

const BTPROTO_HCI: libc::c_int = 1;
const HCI_CHANNEL_CONTROL: u16 = 3;
const HCI_DEV_NONE: u16 = 0xffff;

const MGMT_EV_CMD_COMPLETE: u16 = 0x0001;
const MGMT_EV_CMD_STATUS: u16 = 0x0002;
const MGMT_OP_READ_DEF_SYSTEM_CONFIG: u16 = 0x004b;
const MGMT_OP_SET_DEF_SYSTEM_CONFIG: u16 = 0x004c;

/// LE scan interval and window for discovery, in units of 0.625 ms
const LE_SCAN_INT_DISCOVERY: u16 = 0x0011;
const LE_SCAN_WINDOW_DISCOVERY: u16 = 0x0012;

#[repr(C)]
struct SockaddrHci {
    family: libc::sa_family_t,
    dev: u16,
    channel: u16,
}

/// LE scan timing of discovery in units of 0.625 ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanTiming {
    pub interval: u16,
    pub window: u16,
}

struct Mgmt(OwnedFd);

impl Mgmt {
    fn open() -> Result<Self> {
        // SAFETY: plain socket call, the result is checked.
        let fd = unsafe { libc::socket(libc::AF_BLUETOOTH, libc::SOCK_RAW | libc::SOCK_CLOEXEC, BTPROTO_HCI) };
        if fd < 0 {
            return Err(io::Error::last_os_error()).context("Bluetooth management socket");
        }
        // SAFETY: fd is a new descriptor owned by nobody else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let addr = SockaddrHci { family: libc::AF_BLUETOOTH as _, dev: HCI_DEV_NONE, channel: HCI_CHANNEL_CONTROL };
        // SAFETY: addr is a valid sockaddr_hci of the given size.
        let r = unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &addr as *const SockaddrHci as *const libc::sockaddr,
                std::mem::size_of::<SockaddrHci>() as libc::socklen_t,
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error()).context("bind the Bluetooth management socket");
        }
        let timeout = libc::timeval { tv_sec: 2, tv_usec: 0 };
        // SAFETY: timeout is a valid timeval of the given size.
        unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &timeout as *const libc::timeval as *const libc::c_void,
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            );
        }
        Ok(Self(fd))
    }

    /// Sends `opcode` for adapter `index` and returns the parameters of
    /// its Command Complete event.
    fn command(&self, opcode: u16, index: u16, params: &[u8]) -> Result<Vec<u8>> {
        let mut msg = Vec::with_capacity(6 + params.len());
        msg.extend(opcode.to_le_bytes());
        msg.extend(index.to_le_bytes());
        msg.extend((params.len() as u16).to_le_bytes());
        msg.extend(params);
        // SAFETY: msg is a valid buffer of the given length.
        if unsafe { libc::write(self.0.as_raw_fd(), msg.as_ptr().cast(), msg.len()) } < 0 {
            return Err(io::Error::last_os_error()).context("Bluetooth management command");
        }
        let started = std::time::Instant::now();
        let mut buf = [0u8; 1024];
        while started.elapsed() < Duration::from_secs(2) {
            // SAFETY: buf is a valid buffer of the given length.
            let n = unsafe { libc::read(self.0.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n < 0 {
                return Err(io::Error::last_os_error()).context("Bluetooth management reply");
            }
            let n = n as usize;
            if n < 9 {
                continue;
            }
            let event = u16::from_le_bytes([buf[0], buf[1]]);
            let ev_index = u16::from_le_bytes([buf[2], buf[3]]);
            let ev_opcode = u16::from_le_bytes([buf[6], buf[7]]);
            if ev_index != index || ev_opcode != opcode {
                continue;
            }
            let status = buf[8];
            match event {
                MGMT_EV_CMD_COMPLETE if status == 0 => return Ok(buf[9..n].to_vec()),
                MGMT_EV_CMD_COMPLETE | MGMT_EV_CMD_STATUS => {
                    bail!("Bluetooth management command {opcode:#06x} failed with status {status:#04x}")
                }
                _ => {}
            }
        }
        bail!("no reply to Bluetooth management command {opcode:#06x}")
    }
}

/// The value of TLV `kind` in a system configuration.
fn tlv_u16(mut data: &[u8], kind: u16) -> Option<u16> {
    while data.len() >= 3 {
        let t = u16::from_le_bytes([data[0], data[1]]);
        let len = data[2] as usize;
        let value = data.get(3..3 + len)?;
        if t == kind && len == 2 {
            return Some(u16::from_le_bytes([value[0], value[1]]));
        }
        data = &data[3 + len..];
    }
    None
}

/// The LE scan timing of discovery of adapter `index`.
pub fn discovery_scan(index: u16) -> Result<ScanTiming> {
    let config = Mgmt::open()?.command(MGMT_OP_READ_DEF_SYSTEM_CONFIG, index, &[])?;
    Ok(ScanTiming {
        interval: tlv_u16(&config, LE_SCAN_INT_DISCOVERY).context("no LE discovery scan interval")?,
        window: tlv_u16(&config, LE_SCAN_WINDOW_DISCOVERY).context("no LE discovery scan window")?,
    })
}

/// Sets the LE scan timing of discovery of adapter `index`, it applies
/// from the next scan start.
pub fn set_discovery_scan(index: u16, timing: ScanTiming) -> Result<()> {
    let mut params = Vec::new();
    for (kind, value) in [(LE_SCAN_INT_DISCOVERY, timing.interval), (LE_SCAN_WINDOW_DISCOVERY, timing.window)] {
        params.extend(kind.to_le_bytes());
        params.push(2);
        params.extend(value.to_le_bytes());
    }
    Mgmt::open()?.command(MGMT_OP_SET_DEF_SYSTEM_CONFIG, index, &params)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tlvs() {
        let data = [0x11, 0x00, 0x02, 0x12, 0x00, 0x12, 0x00, 0x02, 0x12, 0x00];
        assert_eq!(tlv_u16(&data, LE_SCAN_INT_DISCOVERY), Some(0x12));
        assert_eq!(tlv_u16(&data, LE_SCAN_WINDOW_DISCOVERY), Some(0x12));
        assert_eq!(tlv_u16(&data, 0x0013), None);
    }
}
