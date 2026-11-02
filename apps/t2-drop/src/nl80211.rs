// SPDX-License-Identifier: GPL-3.0-or-later
//! The AWDL vendor commands and events of brcmfmac (Broadcom OUI) over
//! nl80211, kait2en wireless patches 0010 and 0011. The commands go to the
//! primary interface, AWDL_START and AWDL_STOP answer with an event.

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

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
const NLA_F_NESTED: u16 = 0x8000;
const SOL_NETLINK: i32 = 270;
const NETLINK_ADD_MEMBERSHIP: i32 = 1;

const NL80211_CMD_VENDOR: u8 = 103;
const NL80211_ATTR_IFINDEX: u16 = 3;
const NL80211_ATTR_VENDOR_ID: u16 = 195;
const NL80211_ATTR_VENDOR_SUBCMD: u16 = 196;
const NL80211_ATTR_VENDOR_DATA: u16 = 197;

const BROADCOM_OUI: u32 = 0x001018;

/// enum brcmf_vndr_cmds
const CMD_AWDL_START: u32 = 2;
const CMD_AWDL_STOP: u32 = 3;
const CMD_AWDL_SET_FRAMES: u32 = 4;
const CMD_AWDL_SET_SYNC: u32 = 5;
const CMD_AWDL_PEER_ADD: u32 = 6;
const CMD_AWDL_PEER_DEL: u32 = 7;

/// enum brcmf_vndr_evts
const EVT_AWDL_STARTED: u32 = 0;
const EVT_AWDL_STOPPED: u32 = 1;
const EVT_AWDL_FRAME: u32 = 2;
const EVT_AWDL_ROLE: u32 = 3;

/// enum brcmf_awdl_attrs
const ATTR_ADDR: u16 = 1;
const ATTR_PAYLOAD: u16 = 2;
const ATTR_OOB: u16 = 3;
const ATTR_MASTER: u16 = 4;
const ATTR_MASTER_METRIC: u16 = 5;
const ATTR_MASTER_DISTANCE: u16 = 6;
const ATTR_STATUS: u16 = 7;
const ATTR_IFINDEX: u16 = 8;
const ATTR_PEER_IES: u16 = 9;
const ATTR_FRAME: u16 = 10;
const ATTR_ROLE: u16 = 11;

/// How long AWDL_START and AWDL_STOP may take, the driver waits up to 2 s
/// for each firmware interface event.
const RESULT_TIMEOUT: Duration = Duration::from_secs(5);

type Attrs = HashMap<u16, Vec<u8>>;

fn attr(kind: u16, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + value.len() + 3);
    out.extend_from_slice(&((4 + value.len()) as u16).to_ne_bytes());
    out.extend_from_slice(&kind.to_ne_bytes());
    out.extend_from_slice(value);
    out.resize(out.len().next_multiple_of(4), 0);
    out
}

fn parse_attrs(mut buf: &[u8]) -> Attrs {
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

fn attr_u32(attrs: &Attrs, kind: u16) -> Option<u32> {
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
    // SAFETY: an all zero sockaddr_nl is valid.
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

fn set_timeout(fd: &OwnedFd, timeout: Duration) -> Result<()> {
    let tv = libc::timeval {
        tv_sec: timeout.as_secs() as libc::time_t,
        tv_usec: timeout.subsec_micros() as libc::suseconds_t,
    };
    // SAFETY: tv is a valid timeval for setsockopt.
    let ret = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &tv as *const libc::timeval as *const _,
            std::mem::size_of::<libc::timeval>() as u32,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error()).context("socket timeout");
    }
    Ok(())
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

/// An AWDL event of the driver.
#[derive(Debug, Clone)]
pub enum Event {
    Started { status: i32, ifindex: u32 },
    Stopped { status: i32 },
    /// An AWDL action frame from `addr`, the body from the category on.
    Frame { addr: [u8; 6], body: Vec<u8> },
    Role(u32),
}

/// The master to synchronize to.
#[derive(Debug, Clone, Copy)]
pub struct Master {
    pub addr: [u8; 6],
    pub metric: u32,
    /// Hops from this device to the master
    pub distance: u8,
}

/// The AWDL vendor commands on one primary interface.
pub struct Nl80211 {
    fd: OwnedFd,
    seq: u32,
    family: u16,
    vendor_group: u32,
    primary: u32,
}

impl Nl80211 {
    pub fn open(primary: u32) -> Result<Self> {
        let mut nl = Nl80211 { fd: socket()?, seq: 0, family: GENL_ID_CTRL, vendor_group: 0, primary };
        let reply = nl
            .request_raw(GENL_ID_CTRL, CTRL_CMD_GETFAMILY, &attr(CTRL_ATTR_FAMILY_NAME, b"nl80211\0"))
            .context("nl80211 netlink family not found")?;
        let id = reply.get(&CTRL_ATTR_FAMILY_ID).ok_or_else(|| anyhow!("no family id"))?;
        nl.family = u16::from_ne_bytes([id[0], id[1]]);
        let groups = reply.get(&CTRL_ATTR_MCAST_GROUPS).cloned().unwrap_or_default();
        for group in parse_attrs(&groups).values() {
            let g = parse_attrs(group);
            if g.get(&1).map(|n| n.as_slice() == b"vendor\0") == Some(true) {
                nl.vendor_group = attr_u32(&g, 2).unwrap_or(0);
            }
        }
        if nl.vendor_group == 0 {
            bail!("nl80211 has no vendor group");
        }
        Ok(nl)
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

    /// Sends the vendor command `subcmd` with the AWDL attributes `data`.
    fn vendor(&mut self, subcmd: u32, data: &[u8]) -> Result<()> {
        let mut p = attr(NL80211_ATTR_IFINDEX, &self.primary.to_ne_bytes());
        p.extend(attr(NL80211_ATTR_VENDOR_ID, &BROADCOM_OUI.to_ne_bytes()));
        p.extend(attr(NL80211_ATTR_VENDOR_SUBCMD, &subcmd.to_ne_bytes()));
        p.extend(attr(NL80211_ATTR_VENDOR_DATA | NLA_F_NESTED, data));
        let family = self.family;
        self.request_raw(family, NL80211_CMD_VENDOR, &p)?;
        Ok(())
    }

    /// Waits for the result event `evt` after sending `subcmd`.
    fn vendor_wait(&mut self, subcmd: u32, data: &[u8], evt: u32) -> Result<Event> {
        let events = self.events()?;
        set_timeout(&events.fd, RESULT_TIMEOUT)?;
        self.vendor(subcmd, data)?;
        let end = Instant::now() + RESULT_TIMEOUT;
        while Instant::now() < end {
            let batch = match events.next_raw() {
                Ok(batch) => batch,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => return Err(err.into()),
            };
            for ev in batch {
                if ev.0 == evt {
                    return Ok(ev.1);
                }
            }
        }
        bail!("no result event from the driver")
    }

    /// Creates the AWDL interface with `mac` and starts AWDL with the TLVs
    /// of awdl_payload and awdl_oob_af_auto. Returns the ifindex of awdl0.
    pub fn start(&mut self, mac: [u8; 6], payload: &[u8], oob: &[u8]) -> Result<u32> {
        let mut d = attr(ATTR_ADDR, &mac);
        d.extend(attr(ATTR_PAYLOAD, payload));
        d.extend(attr(ATTR_OOB, oob));
        match self.vendor_wait(CMD_AWDL_START, &d, EVT_AWDL_STARTED).context("AWDL start")? {
            Event::Started { status: 0, ifindex } => Ok(ifindex),
            Event::Started { status, .. } => Err(io::Error::from_raw_os_error(-status)).context("AWDL start"),
            _ => unreachable!(),
        }
    }

    /// Stops AWDL and removes the AWDL interface.
    pub fn stop(&mut self) -> Result<()> {
        match self.vendor_wait(CMD_AWDL_STOP, &[], EVT_AWDL_STOPPED).context("AWDL stop")? {
            Event::Stopped { status: 0 } => Ok(()),
            Event::Stopped { status } => Err(io::Error::from_raw_os_error(-status)).context("AWDL stop"),
            _ => unreachable!(),
        }
    }

    /// Replaces the TLVs the firmware sends.
    pub fn set_frames(&mut self, payload: &[u8], oob: &[u8]) -> Result<()> {
        let mut d = attr(ATTR_PAYLOAD, payload);
        d.extend(attr(ATTR_OOB, oob));
        self.vendor(CMD_AWDL_SET_FRAMES, &d).context("AWDL frames")
    }

    /// Synchronizes to `master`, or to this device without one.
    pub fn set_sync(&mut self, master: Option<Master>) -> Result<()> {
        let mut d = Vec::new();
        if let Some(m) = master {
            d.extend(attr(ATTR_MASTER, &m.addr));
            d.extend(attr(ATTR_MASTER_METRIC, &m.metric.to_ne_bytes()));
            d.extend(attr(ATTR_MASTER_DISTANCE, &[m.distance]));
        }
        self.vendor(CMD_AWDL_SET_SYNC, &d).context("AWDL sync")
    }

    /// Adds the peer `mac`, or updates it, with its HT and VHT capabilities
    /// elements `ies`. Without them the firmware does not know its rates.
    pub fn peer_add(&mut self, mac: [u8; 6], ies: &[u8]) -> Result<()> {
        let mut d = attr(ATTR_ADDR, &mac);
        if !ies.is_empty() {
            d.extend(attr(ATTR_PEER_IES, ies));
        }
        self.vendor(CMD_AWDL_PEER_ADD, &d).context("AWDL peer add")
    }

    pub fn peer_del(&mut self, mac: [u8; 6]) -> Result<()> {
        self.vendor(CMD_AWDL_PEER_DEL, &attr(ATTR_ADDR, &mac)).context("AWDL peer remove")
    }

    /// A blocking socket that receives the AWDL events of the primary
    /// interface.
    pub fn events(&self) -> Result<EventSocket> {
        let fd = socket()?;
        let group = self.vendor_group;
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
            return Err(io::Error::last_os_error()).context("join the nl80211 vendor group");
        }
        Ok(EventSocket { fd, primary: self.primary })
    }
}

pub struct EventSocket {
    fd: OwnedFd,
    primary: u32,
}

impl EventSocket {
    /// Blocks until events arrive.
    pub fn next(&self) -> Result<Vec<Event>> {
        Ok(self.next_raw().context("AWDL events")?.into_iter().map(|(_, ev)| ev).collect())
    }

    /// Events with their vendor subcommand.
    fn next_raw(&self) -> io::Result<Vec<(u32, Event)>> {
        let mut buf = vec![0u8; 65536];
        let n = recv(&self.fd, &mut buf)?;
        let mut out = Vec::new();
        for (_, _, body) in messages(&buf[..n]) {
            if body.len() < 4 || body[0] != NL80211_CMD_VENDOR {
                continue;
            }
            let a = parse_attrs(&body[4..]);
            if attr_u32(&a, NL80211_ATTR_VENDOR_ID) != Some(BROADCOM_OUI)
                || attr_u32(&a, NL80211_ATTR_IFINDEX) != Some(self.primary)
            {
                continue;
            }
            let Some(subcmd) = attr_u32(&a, NL80211_ATTR_VENDOR_SUBCMD) else { continue };
            let d = parse_attrs(a.get(&NL80211_ATTR_VENDOR_DATA).map(Vec::as_slice).unwrap_or_default());
            let status = attr_u32(&d, ATTR_STATUS).unwrap_or(0) as i32;
            let ev = match subcmd {
                EVT_AWDL_STARTED => Event::Started { status, ifindex: attr_u32(&d, ATTR_IFINDEX).unwrap_or(0) },
                EVT_AWDL_STOPPED => Event::Stopped { status },
                EVT_AWDL_FRAME => {
                    let mut addr = [0u8; 6];
                    if let Some(v) = d.get(&ATTR_ADDR).filter(|v| v.len() == 6) {
                        addr.copy_from_slice(v);
                    }
                    Event::Frame { addr, body: d.get(&ATTR_FRAME).cloned().unwrap_or_default() }
                }
                EVT_AWDL_ROLE => Event::Role(attr_u32(&d, ATTR_ROLE).unwrap_or(0)),
                _ => continue,
            };
            out.push((subcmd, ev));
        }
        Ok(out)
    }
}
