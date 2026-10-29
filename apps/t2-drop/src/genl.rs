// SPDX-License-Identifier: GPL-3.0-or-later
//! The brcmf_awdl generic netlink family of the kait2en brcmfmac patch
//! 0006: AWDL interface up and down, iovars and the firmware events of the
//! AWDL interface.

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use anyhow::{Context, Result, anyhow, bail};

const NETLINK_GENERIC: i32 = 16;
const GENL_ID_CTRL: u16 = 0x10;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const CTRL_ATTR_MCAST_GROUPS: u16 = 7;
const NLM_F_REQUEST: u16 = 1;
const NLM_F_ACK: u16 = 4;
const NLMSG_ERROR: u16 = 2;
const SOL_NETLINK: i32 = 270;
const NETLINK_ADD_MEMBERSHIP: i32 = 1;

pub const ATTR_IFINDEX: u16 = 1;
pub const ATTR_MAC: u16 = 2;
pub const ATTR_NAME: u16 = 3;
pub const ATTR_DATA: u16 = 4;
pub const ATTR_PRIMARY: u16 = 6;
pub const ATTR_AWDL_IFINDEX: u16 = 7;
pub const ATTR_EVENT: u16 = 8;
pub const ATTR_STATUS: u16 = 9;
pub const ATTR_ADDR: u16 = 11;
pub const ATTR_FWERR: u16 = 12;

const CMD_UP: u8 = 1;
const CMD_DOWN: u8 = 2;
const CMD_IOVAR_SET: u8 = 3;

pub type Attrs = HashMap<u16, Vec<u8>>;

pub fn attr(kind: u16, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + value.len() + 3);
    out.extend_from_slice(&((4 + value.len()) as u16).to_ne_bytes());
    out.extend_from_slice(&kind.to_ne_bytes());
    out.extend_from_slice(value);
    out.resize(out.len().next_multiple_of(4), 0);
    out
}

pub fn parse_attrs(mut buf: &[u8]) -> Attrs {
    let mut out = HashMap::new();
    while buf.len() >= 4 {
        let len = u16::from_ne_bytes([buf[0], buf[1]]) as usize;
        let kind = u16::from_ne_bytes([buf[2], buf[3]]) & 0x3fff;
        if len < 4 || len > buf.len() {
            break;
        }
        out.insert(kind, buf[4..len].to_vec());
        buf = &buf[len.next_multiple_of(4).min(buf.len())..];
    }
    out
}

pub fn attr_u32(attrs: &Attrs, kind: u16) -> Option<u32> {
    attrs.get(&kind).and_then(|v| v.get(..4)).map(|v| u32::from_ne_bytes(v.try_into().unwrap()))
}

fn socket() -> Result<OwnedFd> {
    // SAFETY: plain socket(2) call, the descriptor is owned right after.
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, NETLINK_GENERIC) };
    if fd < 0 {
        return Err(io::Error::last_os_error()).context("netlink socket");
    }
    // SAFETY: fd is a fresh descriptor not owned elsewhere.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as u16;
    // SAFETY: addr is a valid sockaddr_nl of the given size.
    let ret = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as u32,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error()).context("netlink bind");
    }
    Ok(fd)
}

fn send(fd: &OwnedFd, buf: &[u8]) -> io::Result<()> {
    // SAFETY: buf is valid for its length.
    let ret = unsafe { libc::send(fd.as_raw_fd(), buf.as_ptr() as *const _, buf.len(), 0) };
    if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn recv(fd: &OwnedFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: buf is valid for its length.
    let ret = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len(), 0) };
    if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(ret as usize) }
}

/// Splits a netlink datagram into (type, seq, payload) messages.
fn messages(mut data: &[u8]) -> Vec<(u16, u32, &[u8])> {
    let mut out = Vec::new();
    while data.len() >= 16 {
        let len = u32::from_ne_bytes(data[0..4].try_into().unwrap()) as usize;
        if len < 16 || len > data.len() {
            break;
        }
        let kind = u16::from_ne_bytes([data[4], data[5]]);
        let seq = u32::from_ne_bytes(data[8..12].try_into().unwrap());
        out.push((kind, seq, &data[16..len]));
        data = &data[len.next_multiple_of(4).min(data.len())..];
    }
    out
}

/// Request socket of the brcmf_awdl family.
pub struct Genl {
    fd: OwnedFd,
    seq: u32,
    family: u16,
    events_group: u32,
}

/// The firmware event of the AWDL interface.
#[derive(Debug, Clone)]
pub struct Event {
    pub code: u32,
    pub status: u32,
    pub addr: [u8; 6],
    pub data: Vec<u8>,
}

impl Genl {
    pub fn open() -> Result<Self> {
        let mut genl = Genl { fd: socket()?, seq: 0, family: GENL_ID_CTRL, events_group: 0 };
        let reply = genl
            .request_raw(GENL_ID_CTRL, CTRL_CMD_GETFAMILY, &attr(CTRL_ATTR_FAMILY_NAME, b"brcmf_awdl\0"))
            .context("brcmf_awdl netlink family not found, is the kait2en brcmfmac loaded?")?;
        let id = reply.get(&CTRL_ATTR_FAMILY_ID).ok_or_else(|| anyhow!("no family id"))?;
        genl.family = u16::from_ne_bytes([id[0], id[1]]);
        let groups = reply.get(&CTRL_ATTR_MCAST_GROUPS).cloned().unwrap_or_default();
        for group in parse_attrs(&groups).values() {
            let g = parse_attrs(group);
            if g.get(&1).map(|n| n.starts_with(b"events")) == Some(true) {
                genl.events_group = attr_u32(&g, 2).unwrap_or(0);
            }
        }
        if genl.events_group == 0 {
            bail!("brcmf_awdl has no events group");
        }
        Ok(genl)
    }

    fn request_raw(&mut self, family: u16, cmd: u8, payload: &[u8]) -> Result<Attrs> {
        self.seq += 1;
        let mut msg = Vec::with_capacity(20 + payload.len());
        msg.extend_from_slice(&((20 + payload.len()) as u32).to_ne_bytes());
        msg.extend_from_slice(&family.to_ne_bytes());
        msg.extend_from_slice(&(NLM_F_REQUEST | NLM_F_ACK).to_ne_bytes());
        msg.extend_from_slice(&self.seq.to_ne_bytes());
        msg.extend_from_slice(&0u32.to_ne_bytes());
        msg.extend_from_slice(&[cmd, 1, 0, 0]);
        msg.extend_from_slice(payload);
        send(&self.fd, &msg)?;
        let mut reply = Attrs::new();
        let mut buf = vec![0u8; 65536];
        loop {
            let n = recv(&self.fd, &mut buf)?;
            for (kind, seq, body) in messages(&buf[..n]) {
                if seq != self.seq {
                    continue;
                }
                if kind == NLMSG_ERROR {
                    let err = i32::from_ne_bytes(body[0..4].try_into().unwrap());
                    if err != 0 {
                        return Err(io::Error::from_raw_os_error(-err).into());
                    }
                    return Ok(reply);
                }
                if body.len() >= 4 {
                    reply = parse_attrs(&body[4..]);
                }
            }
        }
    }

    pub fn request(&mut self, cmd: u8, payload: &[u8]) -> Result<Attrs> {
        let family = self.family;
        self.request_raw(family, cmd, payload)
    }

    /// Creates the AWDL interface with `mac`, returns the ifindex of awdl0.
    pub fn up(&mut self, primary: u32, mac: [u8; 6]) -> Result<u32> {
        let mut p = attr(ATTR_IFINDEX, &primary.to_ne_bytes());
        p.extend(attr(ATTR_MAC, &mac));
        let reply = self.request(CMD_UP, &p).context("AWDL interface up")?;
        attr_u32(&reply, ATTR_AWDL_IFINDEX).ok_or_else(|| anyhow!("no awdl ifindex"))
    }

    pub fn down(&mut self, primary: u32) -> Result<()> {
        self.request(CMD_DOWN, &attr(ATTR_IFINDEX, &primary.to_ne_bytes()))?;
        Ok(())
    }

    /// Sets an iovar on the AWDL interface, or on the primary one.
    /// Returns the firmware error, 0 on success.
    pub fn set(&mut self, primary: u32, name: &str, data: &[u8], on_primary: bool) -> Result<i32> {
        let mut p = attr(ATTR_IFINDEX, &primary.to_ne_bytes());
        let mut n = name.as_bytes().to_vec();
        n.push(0);
        p.extend(attr(ATTR_NAME, &n));
        p.extend(attr(ATTR_DATA, data));
        if on_primary {
            p.extend(attr(ATTR_PRIMARY, &[]));
        }
        let reply = self.request(CMD_IOVAR_SET, &p).with_context(|| format!("set {name}"))?;
        Ok(attr_u32(&reply, ATTR_FWERR).unwrap_or(0) as i32)
    }

    /// A blocking socket that receives the AWDL events.
    pub fn events(&self) -> Result<EventSocket> {
        let fd = socket()?;
        let group = self.events_group;
        // SAFETY: group is a valid u32 for setsockopt.
        let ret = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                SOL_NETLINK,
                NETLINK_ADD_MEMBERSHIP,
                &group as *const u32 as *const _,
                4,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error()).context("join events group");
        }
        Ok(EventSocket { fd })
    }
}

pub struct EventSocket {
    fd: OwnedFd,
}

impl EventSocket {
    /// Blocks until events arrive.
    pub fn next(&self) -> Result<Vec<Event>> {
        let mut buf = vec![0u8; 65536];
        let n = recv(&self.fd, &mut buf)?;
        let mut out = Vec::new();
        for (_, _, body) in messages(&buf[..n]) {
            if body.len() < 4 {
                continue;
            }
            let a = parse_attrs(&body[4..]);
            let mut addr = [0u8; 6];
            if let Some(v) = a.get(&ATTR_ADDR).filter(|v| v.len() == 6) {
                addr.copy_from_slice(v);
            }
            out.push(Event {
                code: attr_u32(&a, ATTR_EVENT).unwrap_or(0),
                status: attr_u32(&a, ATTR_STATUS).unwrap_or(0),
                addr,
                data: a.get(&ATTR_DATA).cloned().unwrap_or_default(),
            });
        }
        Ok(out)
    }
}
