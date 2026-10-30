// SPDX-License-Identifier: GPL-3.0-or-later
//! AWDL through the firmware of the T2 Macs: the iovar requests that bring
//! AWDL up as in macOS 15.4.1, election by following the master the peers
//! announce, and the AirDrop peers found in their action frames.
//!
//! The requests start from what macOS sent on a MacBookAir9,1 (BCM4377).
//! The changes the BCM4364 needs were found by testing on a
//! MacBookPro15,1: slot_bss in its SDB off form, the role SLAVE in
//! awdl_opmode, an ARPA TLV with the own host name, see
//! reverse-engineering/airdrop/README.md.

use std::collections::HashMap;
use std::process::Command;
use std::time::Instant;

use anyhow::{Result, bail};
use log::{debug, info, warn};

use crate::genl::{Event, Genl};

/// The AWDL MAC in the captured requests, replaced by the own one.
const CAPTURE_MAC: [u8; 6] = [0x62, 0xaf, 0x12, 0xd8, 0x0c, 0xd9];

const PAYLOAD: &str = "8b0006090000000000000000000018280062af12d80cd962af12d80cd9b90100000000000041000000410000000000000000000000b90100000c2f00639f4445000300000000000000240062af12d80cd90400000000007d100000b901000000000000085800008902000007090000006f001fffff0000110e00bf0c32008003faff0000faff00001502009201";
const OOB_AF_AUTO: &str = "ffffffffd8400000002500ff9473ffffffffffff000000000000000006000000000000000c000000000000008b0018280062af12d80cd962af12d80cd9b90100000000000041000000410000000000000000000000ba0100000c2f00639f4445000300000000000000240062af12d80cd90400000000007d100000b90100001e000000085800008b02000007090000006f001fffff0000110e00bf0c32008003faff0000faff00001502009201060900000000000000000000";
const SYNC_PARAMS: &str = "002400000000060010006e00000010001000000000000000000000000000000000000000";
const PSF_DWELL_ON: &str = "000000006e006e00";
const SLOT_BSS: &str = "0080010101009000010000000403000001002a00000000000f020003ffffe22ae22ae22ae22ae22a0000000000001006e22ae22ae22ae22a0000000000000000020028000000000000100200e22ae22ae22ae22ae22a0000000000000000e22ae22ae22ae22a0000000000000200280000000000011002000000000000000000000000000000000010060000000000000000000000000000000000000000";
const ELECTION_TREE: &str = "000000410000000005ba000001030462af12d80cd9410000000000000000000000000000000000000000";
const OPMODE: &str = "0102000062af12d80cd9000000000000000000000000000000000000";

/// Event codes of the firmware.
const EV_ACTION_FRAME_RX: u32 = 0x4b;
const EV_ROLE: u32 = 0x61;
const EV_AWDL: u32 = 0x62;

/// Apple OUI action frame of type AWDL
const AWDL_AF: [u8; 5] = [0x7f, 0x00, 0x17, 0xf2, 0x08];
const AWDL_AF_HDR: usize = 16;

const TLV_SERVICE_RESPONSE: u8 = 0x02;
const TLV_SERVICE_PARAMS: u8 = 0x06;
const TLV_ARPA: u8 = 0x10;
const TLV_VERSION: u8 = 0x15;
const TLV_ELECTION_V2: u8 = 0x18;

/// The compressed name macOS uses for _airdrop._tcp.local in PTR records.
const AIRDROP_PTR_NAME: [u8; 2] = [0xc0, 0x07];
const DNS_PTR: u8 = 0x0c;
const DNS_SRV: u8 = 0x21;
const DNS_TXT: u8 = 0x10;
/// Compressed name suffixes as macOS writes them
const NAME_LOCAL: [u8; 2] = [0xc0, 0x0c];
const NAME_SELF: [u8; 2] = [0xc0, 0x00];
/// Service params TLV value macOS sends while it advertises AirDrop
const AIRDROP_SERVICE_PARAMS: [u8; 11] = [0, 0, 0, 0x0c, 0, 0, 0, 0x28, 0, 0x40, 0x04];

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn replace_mac(buf: &mut [u8], from: &[u8; 6], to: &[u8; 6]) {
    let mut i = 0;
    while i + 6 <= buf.len() {
        if &buf[i..i + 6] == from {
            buf[i..i + 6].copy_from_slice(to);
            i += 6;
        } else {
            i += 1;
        }
    }
}

/// TLVs of an AWDL action frame body after the fixed header.
fn tlvs(mut body: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    while body.len() >= 3 {
        let len = u16::from_le_bytes([body[1], body[2]]) as usize;
        if body.len() < 3 + len {
            break;
        }
        out.push((body[0], &body[3..3 + len]));
        body = &body[3 + len..];
    }
    out
}

/// Inserts `tlv` in front of the version TLV. TLVs start at `tlv_off`, the
/// 16 bit length at `len_off` counts everything after it.
fn insert_tlv(data: &[u8], tlv_off: usize, len_off: usize, tlv: &[u8]) -> Vec<u8> {
    let mut i = tlv_off;
    while i + 3 <= data.len() && data[i] != TLV_VERSION {
        i += 3 + u16::from_le_bytes([data[i + 1], data[i + 2]]) as usize;
    }
    let i = i.min(data.len());
    let mut out = data[..i].to_vec();
    out.extend_from_slice(tlv);
    out.extend_from_slice(&data[i..]);
    let len = u16::from_le_bytes([data[len_off], data[len_off + 1]]) + tlv.len() as u16;
    out[len_off..len_off + 2].copy_from_slice(&len.to_le_bytes());
    out
}

/// IPv6 link local address of an AWDL MAC (EUI-64), as macOS forms it.
pub fn link_local(mac: &[u8; 6]) -> String {
    format!(
        "fe80::{:02x}{:02x}:{:02x}ff:fe{:02x}:{:02x}{:02x}",
        mac[0] ^ 0x02,
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5]
    )
}

pub fn mac_str(mac: &[u8; 6]) -> String {
    mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

/// What a peer announces in its election TLV.
#[derive(Debug, Clone, Copy)]
struct Election {
    master: [u8; 6],
    master_counter: u32,
    distance: u32,
    master_metric: u32,
}

/// A peer seen in AWDL action frames.
#[derive(Debug, Clone, Default)]
pub struct Peer {
    pub mac: [u8; 6],
    pub host: Option<String>,
    /// AirDrop service instance and port, from its PTR and SRV records
    pub airdrop: Option<(String, u16)>,
    pub last_seen: Option<Instant>,
}

/// The AirDrop service this device advertises while it receives.
#[derive(Debug, Clone)]
pub struct Service {
    pub instance: String,
    pub port: u16,
    pub flags: u32,
}

pub struct Awdl {
    service: Option<Service>,
    genl: Genl,
    primary: u32,
    pub mac: [u8; 6],
    pub ifindex: u32,
    pub ifname: String,
    host: String,
    master: Option<Election>,
    pub peers: HashMap<[u8; 6], Peer>,
    peer_ops: Vec<[u8; 6]>,
    /// slot_bss as macOS sends it, with SDB, on chips other than the BCM4364
    sdb: bool,
}

/// PCI device id of the BCM4364
const BCM4364: u16 = 0x4464;

/// The PCI device id of the chip behind the network interface `ifindex`.
fn pci_device(ifindex: u32) -> Option<u16> {
    let name = ifname(ifindex).ok()?;
    let id = std::fs::read_to_string(format!("/sys/class/net/{name}/device/device")).ok()?;
    u16::from_str_radix(id.trim().trim_start_matches("0x"), 16).ok()
}

impl Awdl {
    /// Creates the AWDL interface and starts AWDL with the firmware.
    pub fn up(mut genl: Genl, primary: u32, mac: [u8; 6], host: String) -> Result<Self> {
        let device = pci_device(primary);
        let sdb = device != Some(BCM4364);
        info!("Wi-Fi chip {:04x}, slot_bss {}", device.unwrap_or(0), if sdb { "with SDB" } else { "without SDB" });
        let ifindex = genl.up(primary, mac)?;
        let ifname = ifname(ifindex)?;
        let mut awdl = Awdl {
            service: None,
            genl,
            primary,
            mac,
            ifindex,
            ifname,
            host,
            master: None,
            peers: HashMap::new(),
            peer_ops: Vec::new(),
            sdb,
        };
        if let Err(err) = awdl.enable() {
            awdl.down();
            return Err(err);
        }
        Ok(awdl)
    }

    fn set(&mut self, name: &str, data: &[u8]) -> Result<()> {
        let fwerr = self.genl.set(self.primary, name, data, false)?;
        if fwerr != 0 {
            debug!("set {name}: firmware error {fwerr}");
        }
        Ok(())
    }

    fn template(&self, hexstr: &str) -> Vec<u8> {
        let mut buf = hex(hexstr);
        replace_mac(&mut buf, &CAPTURE_MAC, &self.mac);
        buf
    }

    /// ARPA TLV with the own host name, laid out as macOS sends it.
    fn arpa_tlv(&self) -> Vec<u8> {
        let host = self.host.as_bytes();
        let mut tlv = vec![TLV_ARPA];
        tlv.extend_from_slice(&((host.len() + 4) as u16).to_le_bytes());
        tlv.push(0x03);
        tlv.push(host.len() as u8);
        tlv.extend_from_slice(host);
        tlv.extend_from_slice(&[0xc0, 0x0c]);
        tlv
    }

    /// Election TLV in the payload: follows the master if there is one.
    fn patch_election_tlv(&self, buf: &mut [u8], tlv_off: usize) {
        let Some(m) = self.master else { return };
        let mut i = tlv_off;
        while i + 3 <= buf.len() {
            let len = u16::from_le_bytes([buf[i + 1], buf[i + 2]]) as usize;
            if buf[i] == TLV_ELECTION_V2 && len >= 0x28 {
                let v = i + 3;
                buf[v..v + 6].copy_from_slice(&m.master);
                buf[v + 6..v + 12].copy_from_slice(&m.master);
                buf[v + 12..v + 16].copy_from_slice(&m.master_counter.to_le_bytes());
                buf[v + 16..v + 20].copy_from_slice(&(m.distance + 1).to_le_bytes());
                buf[v + 20..v + 24].copy_from_slice(&m.master_metric.to_le_bytes());
                return;
            }
            i += 3 + len;
        }
    }

    fn payload(&self) -> Vec<u8> {
        let mut buf = insert_tlv(&self.template(PAYLOAD), 14, 0, &self.arpa_tlv());
        self.patch_election_tlv(&mut buf, 14);
        if self.service.is_some() {
            // The empty service params TLV at offset 2 gets the value macOS sends
            buf = replace_tlv(&buf, 2, 0, TLV_SERVICE_PARAMS, &AIRDROP_SERVICE_PARAMS);
        }
        buf
    }

    fn oob_af_auto(&self) -> Vec<u8> {
        let mut buf = insert_tlv(&self.template(OOB_AF_AUTO), 46, 44, &self.arpa_tlv());
        self.patch_election_tlv(&mut buf, 46);
        if let Some(service) = &self.service {
            // The empty service params TLV at the end is replaced by the
            // filled one and the PTR, SRV and TXT records
            let mut tail = tlv(TLV_SERVICE_PARAMS, &AIRDROP_SERVICE_PARAMS);
            for record in self.service_records(service) {
                tail.extend(tlv(TLV_SERVICE_RESPONSE, &record));
            }
            buf = replace_tlv(&buf, 46, 44, TLV_SERVICE_PARAMS, &[]);
            let len = u16::from_le_bytes([buf[44], buf[45]]) + tail.len() as u16;
            buf[44..46].copy_from_slice(&len.to_le_bytes());
            buf.extend(tail);
        }
        buf
    }

    /// PTR, SRV and TXT records of the AirDrop service, in the layout of
    /// the service response TLVs of macOS.
    fn service_records(&self, s: &Service) -> Vec<Vec<u8>> {
        let inst = s.instance.as_bytes();
        let host = self.host.as_bytes();
        let txt = format!("flags={}", s.flags);
        let mut instance_name = vec![inst.len() as u8];
        instance_name.extend_from_slice(inst);
        instance_name.extend_from_slice(&AIRDROP_PTR_NAME);

        let mut ptr_data = vec![inst.len() as u8];
        ptr_data.extend_from_slice(inst);
        ptr_data.extend_from_slice(&NAME_SELF);
        let mut srv_data = vec![0, 0, 0, 0];
        srv_data.extend_from_slice(&s.port.to_be_bytes());
        srv_data.push(host.len() as u8);
        srv_data.extend_from_slice(host);
        srv_data.extend_from_slice(&NAME_LOCAL);
        let mut txt_data = vec![txt.len() as u8];
        txt_data.extend_from_slice(txt.as_bytes());

        vec![
            service_record_bytes(&AIRDROP_PTR_NAME, DNS_PTR, &ptr_data),
            service_record_bytes(&instance_name, DNS_SRV, &srv_data),
            service_record_bytes(&instance_name, DNS_TXT, &txt_data),
        ]
    }

    /// Starts or stops advertising the AirDrop service in the frames.
    pub fn set_service(&mut self, service: Option<Service>) -> Result<()> {
        self.service = service;
        self.refresh()
    }

    /// slot_bss as macOS sent it for the BCM4377, with SDB. The BCM4364 gets
    /// the SDB off form: byte 0x0c 9, every chanspec in slice 0. The SDB on
    /// form traps the BCM4364 firmware while the station is on 5 GHz.
    fn slot_bss(&self) -> Vec<u8> {
        let mut buf = self.template(SLOT_BSS);
        if self.sdb {
            return buf;
        }
        buf[0x0c] = 9;
        for i in (0..32).step_by(2) {
            let (s0, s1) = (0x4c + i, 0x78 + i);
            if buf[s0..s0 + 2] == [0, 0] {
                let v = [buf[s1], buf[s1 + 1]];
                buf[s0..s0 + 2].copy_from_slice(&v);
            }
            buf[s1..s1 + 2].copy_from_slice(&[0, 0]);
        }
        buf
    }

    fn election_tree(&self) -> Vec<u8> {
        let mut buf = self.template(ELECTION_TREE);
        if let Some(m) = self.master {
            buf[0x0f..0x15].copy_from_slice(&m.master);
            buf[0x15..0x19].copy_from_slice(&m.master_metric.to_le_bytes());
            buf[0x19] = (m.distance + 1) as u8;
        }
        buf
    }

    /// Role byte 1: 0 SLAVE, 2 MASTER. Bytes 4 to 9: the address synced to.
    fn opmode(&self) -> Vec<u8> {
        let mut buf = self.template(OPMODE);
        if let Some(m) = self.master {
            buf[1] = 0;
            buf[4..10].copy_from_slice(&m.master);
        }
        buf
    }

    /// The enable sequence of macOS.
    fn enable(&mut self) -> Result<()> {
        let zero = [0u8; 4];
        self.set("awdl_payload", &self.payload())?;
        self.set("awdl_aftxmode", &zero)?;
        self.set("awdl_sync_params", &hex(SYNC_PARAMS))?;
        self.set("awdl_osoc_chan", &zero)?;
        self.set("awdl_config", &[0x83, 0, 0, 0])?;
        self.set("awdl_psf_dwell", &hex(PSF_DWELL_ON))?;
        self.set("slot_bss", &self.slot_bss())?;
        self.set("awdl_payload", &self.payload())?;
        self.set("awdl_election_tree", &self.election_tree())?;
        self.set("awdl", &[1, 0, 0, 0])?;
        self.set("awdl_presencemode", &[4, 0, 0, 0])?;
        self.set("slot_bss", &self.slot_bss())?;
        self.refresh()?;
        std::thread::sleep(std::time::Duration::from_millis(500));
        self.set("awdl_aftxmode", &zero)?;
        self.set("awdl_osoc_chan", &[0x2c, 0, 0, 0])?;
        self.configure_netdev();
        Ok(())
    }

    /// The group macOS repeats while AWDL is up.
    pub fn refresh(&mut self) -> Result<()> {
        self.set("awdl_oob_af_auto", &self.oob_af_auto())?;
        self.set("awdl_payload", &self.payload())?;
        self.set("awdl_sync_params", &hex(SYNC_PARAMS))?;
        self.set("awdl_opmode", &self.opmode())?;
        self.set("awdl_election_tree", &self.election_tree())?;
        self.set("awdl_sync_params", &hex(SYNC_PARAMS))?;
        Ok(())
    }

    /// Stops the search phase settings macOS changes 5 s after the enable.
    pub fn settle(&mut self) -> Result<()> {
        self.set("awdl_config", &[0x73, 0, 0, 0])?;
        self.set("awdl_psf_dwell", &[0u8; 8])
    }

    /// awdl0 outside NetworkManager, with an EUI-64 link local address that
    /// is usable at once. Duplicate address detection would hold it back
    /// for about 2 s, the address comes from a random MAC.
    fn configure_netdev(&self) {
        let _ = Command::new("nmcli").args(["dev", "set", &self.ifname, "managed", "no"]).output();
        let conf = format!("/proc/sys/net/ipv6/conf/{}", self.ifname);
        let _ = std::fs::write(format!("{conf}/addr_gen_mode"), "0");
        let _ = std::fs::write(format!("{conf}/accept_dad"), "0");
        let _ = Command::new("ip").args(["link", "set", &self.ifname, "up"]).status();
    }

    /// Updates master and peers from a firmware event.
    pub fn handle_event(&mut self, ev: &Event) -> Result<()> {
        match ev.code {
            EV_ROLE => debug!("AWDL role {}", ev.status),
            EV_ACTION_FRAME_RX | EV_AWDL => {
                if let Some(body) = find_af(&ev.data) {
                    self.handle_frame(ev.addr, body)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_frame(&mut self, addr: [u8; 6], body: &[u8]) -> Result<()> {
        let mut election = None;
        let mut host = None;
        let mut airdrop = None;
        let mut instance = None;
        for (kind, value) in tlvs(&body[AWDL_AF_HDR..]) {
            match kind {
                TLV_ELECTION_V2 if value.len() >= 24 => {
                    election = Some(Election {
                        master: value[0..6].try_into().unwrap(),
                        master_counter: u32::from_le_bytes(value[12..16].try_into().unwrap()),
                        distance: u32::from_le_bytes(value[16..20].try_into().unwrap()),
                        master_metric: u32::from_le_bytes(value[20..24].try_into().unwrap()),
                    });
                }
                TLV_ARPA if value.len() > 2 => {
                    let len = value[1] as usize;
                    if let Some(name) = value.get(2..2 + len) {
                        host = Some(String::from_utf8_lossy(name).into_owned());
                    }
                }
                TLV_SERVICE_RESPONSE => {
                    if let Some((name, kind, data)) = service_record(value) {
                        if kind == DNS_PTR && name == AIRDROP_PTR_NAME {
                            if let Some(label) = first_label(data) {
                                instance = Some(label);
                            }
                        } else if kind == DNS_SRV && data.len() >= 6 {
                            airdrop = Some(u16::from_be_bytes([data[4], data[5]]));
                        }
                    }
                }
                _ => {}
            }
        }
        if addr == [0; 6] || addr == self.mac {
            return Ok(());
        }
        let now = Instant::now();
        let new = !self.peers.contains_key(&addr);
        let peer = self.peers.entry(addr).or_insert_with(|| Peer { mac: addr, ..Default::default() });
        peer.last_seen = Some(now);
        if host.is_some() {
            peer.host = host;
        }
        if let (Some(instance), Some(port)) = (instance, airdrop) {
            if peer.airdrop.as_ref().map(|a| &a.0) != Some(&instance) {
                info!("AirDrop service {instance} on {} port {port}", mac_str(&addr));
            }
            peer.airdrop = Some((instance, port));
        }
        if new {
            info!("AWDL peer {}", mac_str(&addr));
        }
        if let Some(e) = election {
            self.follow(e)?;
        }
        Ok(())
    }

    /// Follows the master with the highest metric any peer announces.
    fn follow(&mut self, e: Election) -> Result<()> {
        if e.master == self.mac {
            return Ok(());
        }
        let better = match self.master {
            None => true,
            Some(m) => {
                m.master != e.master
                    && (e.master_metric > m.master_metric
                        || (e.master_metric == m.master_metric && e.master > m.master))
            }
        };
        let same = self.master.map(|m| m.master == e.master) == Some(true);
        if better {
            info!("following AWDL master {} metric {:#x}", mac_str(&e.master), e.master_metric);
            self.master = Some(e);
            self.refresh()?;
        } else if same {
            // Keep the counter of the current master up to date
            self.master = Some(e);
        }
        Ok(())
    }

    /// Makes a peer reachable for unicast: neighbour entry for its link
    /// local address and the firmware peer entry.
    pub fn add_peer(&mut self, mac: [u8; 6]) -> Result<()> {
        if self.peer_ops.contains(&mac) {
            return Ok(());
        }
        let ll = link_local(&mac);
        let status = Command::new("ip")
            .args(["-6", "neigh", "replace", &ll, "lladdr", &mac_str(&mac), "nud", "permanent", "dev", &self.ifname])
            .status()?;
        if !status.success() {
            warn!("neighbour entry for {ll} failed");
        }
        let mut op = vec![0u8, 0];
        op.extend_from_slice(&mac);
        op.push(1);
        op.resize(0x25, 0);
        self.set("awdl_peer_op", &op)?;
        self.peer_ops.push(mac);
        Ok(())
    }

    /// Removes peers and the interface.
    pub fn down(&mut self) {
        for mac in std::mem::take(&mut self.peer_ops) {
            let mut op = vec![0u8, 1];
            op.extend_from_slice(&mac);
            op.push(0);
            let _ = self.set("awdl_peer_op", &op);
        }
        let _ = self.set("awdl", &[0u8; 4]);
        if let Err(err) = self.genl.down(self.primary) {
            warn!("AWDL interface down: {err:#}");
        }
    }
}

/// The AWDL action frame body inside the data of an event.
fn find_af(data: &[u8]) -> Option<&[u8]> {
    for off in [16, 24] {
        if let Some(body) = data.get(off..) {
            if body.len() > AWDL_AF_HDR && body.starts_with(&AWDL_AF) {
                return Some(body);
            }
        }
    }
    None
}

fn tlv(kind: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![kind];
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value);
    out
}

/// Replaces the value of the first TLV of `kind`, an empty `value` removes
/// it. TLVs start at `tlv_off`, the 16 bit length at `len_off` counts
/// everything after it.
fn replace_tlv(data: &[u8], tlv_off: usize, len_off: usize, kind: u8, value: &[u8]) -> Vec<u8> {
    let mut i = tlv_off;
    while i + 3 <= data.len() {
        let len = u16::from_le_bytes([data[i + 1], data[i + 2]]) as usize;
        if data[i] == kind {
            let mut out = data[..i].to_vec();
            if !value.is_empty() {
                out.extend(tlv(kind, value));
            }
            out.extend_from_slice(&data[(i + 3 + len).min(data.len())..]);
            let delta = out.len() as i32 - data.len() as i32;
            let total = (u16::from_le_bytes([data[len_off], data[len_off + 1]]) as i32 + delta) as u16;
            out[len_off..len_off + 2].copy_from_slice(&total.to_le_bytes());
            return out;
        }
        i += 3 + len;
    }
    data.to_vec()
}

/// A service response record: u16 length of name and type, name, type,
/// u16 data length, u16 0, data.
fn service_record_bytes(name: &[u8], kind: u8, data: &[u8]) -> Vec<u8> {
    let mut out = ((name.len() + 1) as u16).to_le_bytes().to_vec();
    out.extend_from_slice(name);
    out.push(kind);
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(data);
    out
}

/// One record of a service response TLV: name, record type and data.
/// Layout: u16 length of name and type, name, type, u16 data length,
/// u16 0, data.
fn service_record(v: &[u8]) -> Option<(&[u8], u8, &[u8])> {
    let n = u16::from_le_bytes([*v.first()?, *v.get(1)?]) as usize;
    if n == 0 {
        return None;
    }
    let name = v.get(2..1 + n)?;
    let kind = *v.get(1 + n)?;
    let dlen = u16::from_le_bytes([*v.get(2 + n)?, *v.get(3 + n)?]) as usize;
    let data = v.get(6 + n..6 + n + dlen)?;
    Some((name, kind, data))
}

/// The first DNS label of a record data.
fn first_label(data: &[u8]) -> Option<String> {
    let len = *data.first()? as usize;
    if len == 0 || len >= 0xc0 {
        return None;
    }
    data.get(1..1 + len).map(|l| String::from_utf8_lossy(l).into_owned())
}

fn ifname(ifindex: u32) -> Result<String> {
    let mut buf = [0u8; libc::IF_NAMESIZE];
    // SAFETY: buf has IF_NAMESIZE bytes as if_indextoname requires.
    let ret = unsafe { libc::if_indextoname(ifindex, buf.as_mut_ptr() as *mut libc::c_char) };
    if ret.is_null() {
        bail!("no interface with index {ifindex}");
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_airdrop_records_of_macos() {
        let ptr = hex("0300c0070c0f0000000c303561323034633462316135c000");
        let (name, kind, data) = service_record(&ptr).unwrap();
        assert_eq!((name, kind), (&[0xc0, 0x07][..], DNS_PTR));
        assert_eq!(first_label(data).unwrap(), "05a204c4b1a5");
        let srv = hex(concat!(
            "10000c303561323034633462316135c007212d00000000000000224224",
            "64626364373433632d386666322d343735392d393830372d343533653964336565396132c00c"
        ));
        let (_, kind, data) = service_record(&srv).unwrap();
        assert_eq!(kind, DNS_SRV);
        assert_eq!(u16::from_be_bytes([data[4], data[5]]), 8770);
    }

    #[test]
    fn builds_the_airdrop_records_like_macos() {
        let inst = b"05a204c4b1a5";
        let host = b"dbcd743c-8ff2-4759-9807-453e9d3ee9a2";
        let mut name = vec![12u8];
        name.extend_from_slice(inst);
        name.extend_from_slice(&AIRDROP_PTR_NAME);
        let mut ptr = vec![12u8];
        ptr.extend_from_slice(inst);
        ptr.extend_from_slice(&NAME_SELF);
        assert_eq!(
            service_record_bytes(&AIRDROP_PTR_NAME, DNS_PTR, &ptr),
            hex("0300c0070c0f0000000c303561323034633462316135c000")
        );
        let mut srv = vec![0, 0, 0, 0, 0x22, 0x42, host.len() as u8];
        srv.extend_from_slice(host);
        srv.extend_from_slice(&NAME_LOCAL);
        assert_eq!(
            service_record_bytes(&name, DNS_SRV, &srv),
            hex(concat!(
                "10000c303561323034633462316135c007212d00000000000000224224",
                "64626364373433632d386666322d343735392d393830372d343533653964336565396132c00c"
            ))
        );
    }

    #[test]
    fn link_local_flips_the_local_bit() {
        assert_eq!(link_local(&[0x32, 0x09, 0xdb, 0x27, 0x82, 0x08]), "fe80::3009:dbff:fe27:8208");
    }

    #[test]
    fn inserts_the_arpa_tlv_before_the_version() {
        let p = hex(PAYLOAD);
        let out = insert_tlv(&p, 14, 0, &[0x10, 1, 0, 0xaa]);
        assert_eq!(out.len(), p.len() + 4);
        assert_eq!(u16::from_le_bytes([out[0], out[1]]), 0x8b + 4);
        assert_eq!(&out[out.len() - 9..out.len() - 5], &[0x10, 1, 0, 0xaa]);
    }
}
