// SPDX-License-Identifier: GPL-3.0-or-later
//! Quick Share over a BLE L2CAP connection oriented channel, the way an
//! Android sender tries before Bluetooth Classic. The PSM goes into the BLE
//! advertisement header and the GATT advertisement. A connection starts
//! with the commands of google/nearby (commit 203b6e1)
//! connections/implementation/mediums/ble/ble_l2cap_packet.h: the sender
//! asks for the data connection, the receiver answers that it is ready.
//! After that the BLE socket of mediums/ble/ble_socket.cc carries the same
//! frames as TCP and RFCOMM, either plain or, with the nearby flag
//! kRefactorBleL2cap, as packets with the service id hash in front, a
//! length and an acknowledgement per packet. The first bytes of the sender
//! tell which.

use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;

use anyhow::{Result, bail};
use log::{debug, info};
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::Mutex;

const AF_BLUETOOTH: libc::c_int = 31;
const BTPROTO_L2CAP: libc::c_int = 0;
const SOL_BLUETOOTH: libc::c_int = 274;
const BT_SECURITY: libc::c_int = 4;
const BT_SECURITY_LOW: u8 = 1;
const BT_SNDMTU: libc::c_int = 12;
const BDADDR_LE_PUBLIC: u8 = 1;

/// sha256("NearbySharing")[..3]
const SERVICE_ID_HASH: [u8; 3] = [0xfc, 0x9f, 0x5e];
/// The first three bytes of a BLE socket control packet.
const CONTROL: [u8; 3] = [0, 0, 0];
/// Largest packet of a data connection with lengths
const MAX_PACKET: usize = 64 * 1024;

const REQUEST_ADVERTISEMENT: u8 = 1;
const REQUEST_ADVERTISEMENT_FINISH: u8 = 2;
const REQUEST_DATA_CONNECTION: u8 = 3;
const RESPONSE_ADVERTISEMENT: u8 = 21;
const RESPONSE_SERVICE_ID_NOT_FOUND: u8 = 22;
const RESPONSE_DATA_CONNECTION_READY: u8 = 23;

#[repr(C)]
struct SockaddrL2 {
    family: libc::sa_family_t,
    psm: u16,
    bdaddr: [u8; 6],
    cid: u16,
    bdaddr_type: u8,
}

fn check(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(ret) }
}

fn register(fd: OwnedFd) -> io::Result<AsyncFd<OwnedFd>> {
    // SAFETY: the AsyncFd owns the descriptor, it stays open as long
    unsafe { AsyncFd::register(fd) }.map_err(io::Error::from)
}

/// A listening LE L2CAP socket on a PSM the kernel picks.
pub struct Server {
    fd: AsyncFd<OwnedFd>,
    pub psm: u16,
}

impl Server {
    pub fn open() -> io::Result<Self> {
        // SAFETY: plain socket calls on a descriptor owned right away
        let fd = unsafe {
            OwnedFd::from_raw_fd(check(libc::socket(
                AF_BLUETOOTH,
                libc::SOCK_SEQPACKET | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                BTPROTO_L2CAP,
            ))?)
        };
        // Android opens an insecure channel, without pairing
        let security = [BT_SECURITY_LOW, 0u8];
        let mut addr = SockaddrL2 {
            family: AF_BLUETOOTH as libc::sa_family_t,
            psm: 0,
            bdaddr: [0; 6],
            cid: 0,
            bdaddr_type: BDADDR_LE_PUBLIC,
        };
        let mut len = size_of::<SockaddrL2>() as libc::socklen_t;
        // SAFETY: the pointers point at live values of the given sizes
        unsafe {
            check(libc::setsockopt(
                fd.as_raw_fd(),
                SOL_BLUETOOTH,
                BT_SECURITY,
                security.as_ptr().cast(),
                security.len() as libc::socklen_t,
            ))?;
            check(libc::bind(fd.as_raw_fd(), (&raw const addr).cast(), len))?;
            check(libc::listen(fd.as_raw_fd(), 4))?;
            check(libc::getsockname(fd.as_raw_fd(), (&raw mut addr).cast(), &mut len))?;
        }
        Ok(Self { fd: register(fd)?, psm: u16::from_le(addr.psm) })
    }

    pub async fn accept(&self) -> io::Result<Channel> {
        loop {
            let mut guard = self.fd.readable().await?;
            let accepted = guard.try_io(|fd| {
                // SAFETY: accept4 on our listening socket, peer address unused
                check(unsafe {
                    libc::accept4(
                        fd.as_raw_fd(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                    )
                })
            });
            if let Ok(result) = accepted {
                // SAFETY: accept4 returned a new descriptor we own
                let fd = unsafe { OwnedFd::from_raw_fd(result?) };
                return Channel::new(fd);
            }
        }
    }
}

/// One accepted channel. Every read returns one SDU, writes go out as SDUs
/// of at most the send MTU.
pub struct Channel {
    fd: AsyncFd<OwnedFd>,
    mtu: usize,
}

impl Channel {
    fn new(fd: OwnedFd) -> io::Result<Self> {
        let mut mtu: u16 = 0;
        let mut len = size_of::<u16>() as libc::socklen_t;
        // SAFETY: mtu is a live u16
        unsafe {
            check(libc::getsockopt(
                fd.as_raw_fd(),
                SOL_BLUETOOTH,
                BT_SNDMTU,
                (&raw mut mtu).cast(),
                &mut len,
            ))?;
        }
        Ok(Self { fd: register(fd)?, mtu: usize::from(mtu.max(23)) })
    }

    async fn recv(&self) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; 65536];
        loop {
            let mut guard = self.fd.readable().await?;
            let read = guard.try_io(|fd| {
                // SAFETY: buf is large enough for any SDU
                check(unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) as libc::c_int })
            });
            if let Ok(result) = read {
                buf.truncate(result? as usize);
                return Ok(buf);
            }
        }
    }

    async fn send(&self, data: &[u8]) -> io::Result<()> {
        for sdu in data.chunks(self.mtu) {
            loop {
                let mut guard = self.fd.writable().await?;
                let written = guard.try_io(|fd| {
                    // SAFETY: sdu is a live slice
                    check(unsafe { libc::write(fd.as_raw_fd(), sdu.as_ptr().cast(), sdu.len()) as libc::c_int })
                });
                if let Ok(result) = written {
                    result?;
                    break;
                }
            }
        }
        Ok(())
    }
}

/// The SDUs of a channel read as a byte stream, as nearby reads them.
struct Reader {
    channel: Arc<Channel>,
    buf: Vec<u8>,
}

impl Reader {
    async fn take(&mut self, n: usize) -> io::Result<Vec<u8>> {
        while self.buf.len() < n {
            let sdu = self.channel.recv().await?;
            if sdu.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.buf.extend_from_slice(&sdu);
        }
        Ok(self.buf.drain(..n).collect())
    }

    async fn u32_be(&mut self) -> io::Result<usize> {
        let b = self.take(4).await?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }

    /// The rest of a control packet after its three zero bytes: a
    /// SocketControlFrame whose fourth byte is the length of the rest.
    async fn skip_control(&mut self) -> io::Result<()> {
        let head = self.take(4).await?;
        let rest = self.take(usize::from(head[3])).await?;
        debug!("L2CAP control frame {:02x?}{:02x?}", head, rest);
        Ok(())
    }
}

/// SocketControlFrame PACKET_ACKNOWLEDGEMENT for `size` received bytes.
fn acknowledgement(size: usize) -> Vec<u8> {
    let mut ack = vec![0x0a, 0x03];
    ack.extend_from_slice(&SERVICE_ID_HASH);
    ack.push(0x10);
    let mut v = size as u64;
    while v >= 0x80 {
        ack.push((v as u8) | 0x80);
        v >>= 7;
    }
    ack.push(v as u8);
    let mut packet = CONTROL.to_vec();
    packet.extend_from_slice(&[0x08, 0x03, 0x22, ack.len() as u8]);
    packet.extend_from_slice(&ack);
    packet
}

/// An L2CAP command packet with the 4 byte length an S22 puts in front.
fn command(packet: &[u8]) -> Vec<u8> {
    let mut framed = (packet.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(packet);
    framed
}

/// Runs the L2CAP commands on `channel`. For a data connection it returns
/// a stream with the plain frames the receiver reads, bridged to the
/// channel in the background. An advertisement request is answered with
/// `advertisement` and ends without a stream.
pub async fn serve(channel: Channel, advertisement: Arc<Vec<u8>>) -> Result<Option<DuplexStream>> {
    let channel = Arc::new(channel);
    let mut reader = Reader { channel: channel.clone(), buf: Vec::new() };
    // An S22 sends every command as [length u32][command][length u16][data]
    loop {
        let len = reader.u32_be().await?;
        if len == 0 || len > 512 {
            bail!("L2CAP command of {len} bytes");
        }
        let packet = reader.take(len).await?;
        debug!("L2CAP command {:02x?}", packet);
        match packet[0] {
            REQUEST_ADVERTISEMENT => {
                let hash = packet.get(3..).unwrap_or_default();
                let response = if hash == SERVICE_ID_HASH {
                    let mut response = vec![RESPONSE_ADVERTISEMENT];
                    response.extend_from_slice(&(advertisement.len() as u16).to_be_bytes());
                    response.extend_from_slice(&advertisement);
                    response
                } else {
                    vec![RESPONSE_SERVICE_ID_NOT_FOUND]
                };
                channel.send(&command(&response)).await?;
            }
            REQUEST_ADVERTISEMENT_FINISH => return Ok(None),
            REQUEST_DATA_CONNECTION => {
                channel.send(&command(&[RESPONSE_DATA_CONNECTION_READY])).await?;
                break;
            }
            other => bail!("unexpected L2CAP command {other}: {packet:02x?}"),
        }
    }

    // An S22 Ultra puts the 4 byte length in front of every packet of the
    // data connection as well, as in front of the commands
    let head = reader.take(7).await?;
    let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let lengths = (3..=MAX_PACKET).contains(&length) && (head[4..7] == CONTROL || head[4..7] == SERVICE_ID_HASH);
    reader.buf.splice(0..0, head);
    if lengths {
        info!("L2CAP data connection, BLE socket packets with lengths");
        return Ok(Some(serve_with_lengths(channel, reader)));
    }

    // control packets first, the first data bytes tell the framing
    let packets = loop {
        let first = reader.take(3).await?;
        if first == CONTROL {
            reader.skip_control().await?;
            continue;
        }
        if first == SERVICE_ID_HASH {
            break true;
        }
        reader.buf.splice(0..0, first);
        break false;
    };
    info!("L2CAP data connection, {}", if packets { "BLE socket packets" } else { "plain frames" });

    let (ours, theirs) = tokio::io::duplex(256 * 1024);
    let (mut from_us, mut to_us) = tokio::io::split(ours);
    let write_lock = Arc::new(Mutex::new(()));

    // phone to receiver
    let lock = write_lock.clone();
    let ack_channel = channel.clone();
    tokio::spawn(async move {
        let mut first = packets;
        let result: io::Result<()> = async {
            loop {
                if packets {
                    if !first {
                        let hash = reader.take(3).await?;
                        if hash == CONTROL {
                            reader.skip_control().await?;
                            continue;
                        }
                        if hash != SERVICE_ID_HASH {
                            return Err(io::Error::other(format!("packet for service {hash:02x?}")));
                        }
                    }
                    first = false;
                }
                let len = reader.u32_be().await?;
                if packets {
                    let _guard = lock.lock().await;
                    ack_channel.send(&acknowledgement(len)).await?;
                }
                let data = reader.take(len).await?;
                to_us.write_all(&(len as u32).to_be_bytes()).await?;
                to_us.write_all(&data).await?;
            }
        }
        .await;
        debug!("L2CAP from the phone ended: {result:?}");
        let _ = to_us.shutdown().await;
    });

    // receiver to phone
    tokio::spawn(async move {
        let result: io::Result<()> = async {
            loop {
                let len = from_us.read_u32().await? as usize;
                let mut data = vec![0u8; len];
                from_us.read_exact(&mut data).await?;
                let mut packet = Vec::with_capacity(len + 7);
                if packets {
                    packet.extend_from_slice(&SERVICE_ID_HASH);
                }
                packet.extend_from_slice(&(len as u32).to_be_bytes());
                packet.extend_from_slice(&data);
                let _guard = write_lock.lock().await;
                channel.send(&packet).await?;
            }
        }
        .await;
        debug!("L2CAP to the phone ended: {result:?}");
    });
    Ok(Some(theirs))
}

/// The data connection when every packet carries its length: [length u32]
/// then the service id hash and the data, or three zero bytes and a
/// SocketControlFrame. The data holds the plain frames, which the stream
/// gets as they are. Packets to the phone get the same layout.
fn serve_with_lengths(channel: Arc<Channel>, mut reader: Reader) -> DuplexStream {
    let (ours, theirs) = tokio::io::duplex(256 * 1024);
    let (mut from_us, mut to_us) = tokio::io::split(ours);
    let write_lock = Arc::new(Mutex::new(()));

    // phone to receiver
    let lock = write_lock.clone();
    let ack_channel = channel.clone();
    tokio::spawn(async move {
        let result: io::Result<()> = async {
            let mut frames = Vec::new();
            loop {
                let len = reader.u32_be().await?;
                if !(3..=MAX_PACKET).contains(&len) {
                    return Err(io::Error::other(format!("L2CAP packet of {len} bytes")));
                }
                let packet = reader.take(len).await?;
                let (hash, data) = packet.split_at(3);
                if hash == CONTROL {
                    debug!("L2CAP control frame {:02x?}", data);
                    continue;
                }
                if hash != SERVICE_ID_HASH {
                    return Err(io::Error::other(format!("packet for service {hash:02x?}")));
                }
                frames.extend_from_slice(data);
                // acknowledge each frame once it is complete, as for the
                // packets without lengths
                while frames.len() >= 4 {
                    let size = u32::from_be_bytes([frames[0], frames[1], frames[2], frames[3]]) as usize;
                    if frames.len() < 4 + size {
                        break;
                    }
                    {
                        let _guard = lock.lock().await;
                        ack_channel.send(&command(&acknowledgement(size))).await?;
                    }
                    to_us.write_all(&frames[..4 + size]).await?;
                    frames.drain(..4 + size);
                }
            }
        }
        .await;
        debug!("L2CAP from the phone ended: {result:?}");
        let _ = to_us.shutdown().await;
    });

    // receiver to phone
    tokio::spawn(async move {
        let result: io::Result<()> = async {
            loop {
                let len = from_us.read_u32().await? as usize;
                let mut data = vec![0u8; len];
                from_us.read_exact(&mut data).await?;
                let mut packet = Vec::with_capacity(len + 7);
                packet.extend_from_slice(&SERVICE_ID_HASH);
                packet.extend_from_slice(&(len as u32).to_be_bytes());
                packet.extend_from_slice(&data);
                let _guard = write_lock.lock().await;
                channel.send(&command(&packet)).await?;
            }
        }
        .await;
        debug!("L2CAP to the phone ended: {result:?}");
    });
    theirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledgement_layout() {
        assert_eq!(
            acknowledgement(300),
            [0, 0, 0, 0x08, 0x03, 0x22, 0x08, 0x0a, 0x03, 0xfc, 0x9f, 0x5e, 0x10, 0xac, 0x02]
        );
    }
}
