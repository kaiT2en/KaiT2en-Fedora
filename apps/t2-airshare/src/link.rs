// SPDX-License-Identifier: GPL-3.0-or-later
//! The transport both directions share: length prefixed frames over TCP,
//! the encrypted channel after UKEY2, and the payload layer for the Nearby
//! Sharing frames.
//!
//! The payload framing follows NearDrop PROTOCOL.md and rquickshare
//! `core_lib/src/hdl/inbound.rs` (commit 378d8ae, GPL-3.0).

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use log::warn;
use prost::Message;
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::proto::location::nearby::connections as nc;
use crate::proto::sharing::nearby as sn;
use crate::secure::SecureChannel;

/// Largest frame accepted from the wire, Android sends file chunks well
/// below this.
const MAX_FRAME: usize = 5 * 1024 * 1024;
/// Largest bytes payload, these carry protobuf frames and short texts.
const MAX_BYTES_PAYLOAD: i64 = 5 * 1024 * 1024;
/// Android expects a keep-alive at least every 10 s.
pub const KEEPALIVE: Duration = Duration::from_secs(5);
/// Silence after which the peer is considered gone.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Incoming frames, read by a task of their own so the reader never loses
/// a half read frame to a `select!`.
pub struct Frames {
    rx: mpsc::Receiver<Vec<u8>>,
    task: JoinHandle<()>,
}

impl Frames {
    pub fn split(stream: TcpStream) -> (Self, OwnedWriteHalf) {
        let (reader, writer) = stream.into_split();
        let (tx, rx) = mpsc::channel(16);
        let task = tokio::spawn(read_frames(reader, tx));
        (Self { rx, task }, writer)
    }

    pub async fn next(&mut self) -> Result<Vec<u8>> {
        match tokio::time::timeout(READ_TIMEOUT, self.rx.recv()).await {
            Ok(Some(frame)) => Ok(frame),
            Ok(None) => bail!("connection closed"),
            Err(_) => bail!("no frame for {} s", READ_TIMEOUT.as_secs()),
        }
    }

    /// The next frame as an offline frame of the expected type.
    pub async fn expect(&mut self, expected: nc::v1_frame::FrameType) -> Result<nc::V1Frame> {
        let frame = nc::OfflineFrame::decode(self.next().await?.as_slice())?;
        let v1 = frame.v1.ok_or_else(|| anyhow!("offline frame without v1"))?;
        if v1.r#type() != expected {
            bail!("expected {expected:?}, got {:?}", v1.r#type());
        }
        Ok(v1)
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_frames(mut reader: OwnedReadHalf, tx: mpsc::Sender<Vec<u8>>) {
    loop {
        let mut len = [0u8; 4];
        if reader.read_exact(&mut len).await.is_err() {
            return;
        }
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_FRAME {
            warn!("dropping connection, frame of {len} bytes");
            return;
        }
        let mut frame = vec![0u8; len];
        if reader.read_exact(&mut frame).await.is_err() || tx.send(frame).await.is_err() {
            return;
        }
    }
}

pub async fn write_frame(writer: &mut OwnedWriteHalf, data: &[u8]) -> Result<()> {
    writer.write_all(&(data.len() as u32).to_be_bytes()).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

pub fn offline_frame(v1: nc::V1Frame) -> nc::OfflineFrame {
    nc::OfflineFrame {
        version: Some(nc::offline_frame::Version::V1.into()),
        v1: Some(v1),
    }
}

/// Our side of the plaintext connection response, accepting the connection.
pub fn connection_accept() -> nc::OfflineFrame {
    offline_frame(nc::V1Frame {
        r#type: Some(nc::v1_frame::FrameType::ConnectionResponse.into()),
        connection_response: Some(nc::ConnectionResponseFrame {
            response: Some(nc::connection_response_frame::ResponseStatus::Accept.into()),
            os_info: Some(nc::OsInfo {
                r#type: Some(nc::os_info::OsType::Linux.into()),
            }),
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// The encrypted part of a connection.
pub struct Link {
    channel: SecureChannel,
    writer: OwnedWriteHalf,
}

impl Link {
    pub fn new(channel: SecureChannel, writer: OwnedWriteHalf) -> Self {
        Self { channel, writer }
    }

    pub fn open(&mut self, raw: &[u8]) -> Result<nc::OfflineFrame> {
        self.channel.open(raw)
    }

    pub async fn send_offline(&mut self, v1: nc::V1Frame) -> Result<()> {
        let sealed = self.channel.seal(&offline_frame(v1));
        write_frame(&mut self.writer, &sealed).await
    }

    pub async fn send_keepalive(&mut self) -> Result<()> {
        self.send_offline(nc::V1Frame {
            r#type: Some(nc::v1_frame::FrameType::KeepAlive.into()),
            keep_alive: Some(nc::KeepAliveFrame { ack: Some(false), ..Default::default() }),
            ..Default::default()
        })
        .await
    }

    pub async fn send_disconnection(&mut self) -> Result<()> {
        self.send_offline(nc::V1Frame {
            r#type: Some(nc::v1_frame::FrameType::Disconnection.into()),
            disconnection: Some(nc::DisconnectionFrame::default()),
            ..Default::default()
        })
        .await
    }

    /// Sends one chunk of a payload.
    pub async fn send_chunk(
        &mut self,
        header: &nc::payload_transfer_frame::PayloadHeader,
        offset: i64,
        last: bool,
        body: Vec<u8>,
    ) -> Result<()> {
        self.send_offline(nc::V1Frame {
            r#type: Some(nc::v1_frame::FrameType::PayloadTransfer.into()),
            payload_transfer: Some(nc::PayloadTransferFrame {
                packet_type: Some(nc::payload_transfer_frame::PacketType::Data.into()),
                payload_header: Some(header.clone()),
                payload_chunk: Some(nc::payload_transfer_frame::PayloadChunk {
                    offset: Some(offset),
                    flags: Some(last as i32),
                    body: Some(body),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
    }

    /// Sends a sharing frame as a bytes payload: one chunk with the data,
    /// one empty chunk with the last chunk flag, as Android does.
    pub async fn send_sharing(&mut self, v1: sn::V1Frame) -> Result<()> {
        let data = sn::Frame {
            version: Some(sn::frame::Version::V1.into()),
            v1: Some(v1),
        }
        .encode_to_vec();
        let size = data.len() as i64;
        let header = nc::payload_transfer_frame::PayloadHeader {
            id: Some(random_payload_id()),
            r#type: Some(nc::payload_transfer_frame::payload_header::PayloadType::Bytes.into()),
            total_size: Some(size),
            is_sensitive: Some(false),
            ..Default::default()
        };
        self.send_chunk(&header, 0, false, data).await?;
        self.send_chunk(&header, size, true, Vec::new()).await
    }
}

/// Reassembles bytes payloads from their chunks.
#[derive(Default)]
pub struct BytesPayloads {
    buffers: HashMap<i64, Vec<u8>>,
}

impl BytesPayloads {
    /// Adds a chunk. Returns the payload id and data once the last chunk is
    /// in.
    pub fn add(
        &mut self,
        header: &nc::payload_transfer_frame::PayloadHeader,
        chunk: &nc::payload_transfer_frame::PayloadChunk,
    ) -> Result<Option<(i64, Vec<u8>)>> {
        if header.total_size() > MAX_BYTES_PAYLOAD {
            bail!("bytes payload of {} bytes", header.total_size());
        }
        let id = header.id();
        let buffer = self.buffers.entry(id).or_default();
        if chunk.offset() != buffer.len() as i64 {
            bail!("bytes payload chunk at {} after {} bytes", chunk.offset(), buffer.len());
        }
        buffer.extend_from_slice(chunk.body());
        if chunk.flags() & 1 == 0 {
            return Ok(None);
        }
        Ok(Some((id, self.buffers.remove(&id).unwrap_or_default())))
    }
}

pub fn random_payload_id() -> i64 {
    rand::thread_rng().r#gen::<i64>().abs()
}

pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut data = vec![0u8; len];
    rand::thread_rng().fill(data.as_mut_slice());
    data
}

/// Our paired key encryption frame. Without Google account certificates the
/// fields carry random bytes, which Android accepts.
pub fn paired_key_encryption() -> sn::V1Frame {
    sn::V1Frame {
        r#type: Some(sn::v1_frame::FrameType::PairedKeyEncryption.into()),
        paired_key_encryption: Some(sn::PairedKeyEncryptionFrame {
            secret_id_hash: Some(random_bytes(6)),
            signed_data: Some(random_bytes(72)),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn paired_key_result() -> sn::V1Frame {
    sn::V1Frame {
        r#type: Some(sn::v1_frame::FrameType::PairedKeyResult.into()),
        paired_key_result: Some(sn::PairedKeyResultFrame {
            status: Some(sn::paired_key_result_frame::Status::Unable.into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}
