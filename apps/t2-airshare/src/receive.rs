// SPDX-License-Identifier: GPL-3.0-or-later
//! Receiving side (server) of a Quick Share transfer over one TCP
//! connection: connection request, UKEY2, paired key exchange, the
//! introduction with the user's consent, then payload chunks.
//!
//! The frame sequence follows NearDrop PROTOCOL.md and rquickshare
//! `core_lib/src/hdl/inbound.rs` (commit 378d8ae, GPL-3.0).

use std::collections::HashMap;
use std::fs::File;
use std::future::Future;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use log::{debug, info, warn};
use prost::Message;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

use crate::endpoint::{DeviceType, parse_endpoint_info};
use crate::link::{
    BytesPayloads, Frames, KEEPALIVE, Link, connection_accept, paired_key_encryption,
    paired_key_result, write_frame,
};
use crate::proto::location::nearby::connections as nc;
use crate::proto::sharing::nearby as sn;
use crate::secure::ServerHandshake;

/// What the sender offers, shown to the user before accepting.
#[derive(Clone, Debug)]
pub struct Incoming {
    pub sender: Option<String>,
    pub device_type: DeviceType,
    pub pin: String,
    pub files: Vec<IncomingFile>,
    pub texts: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct IncomingFile {
    pub name: String,
    pub size: i64,
}

/// Asked once per transfer, true accepts. The future runs as its own task,
/// keep-alives continue while it waits for the user.
pub type Consent =
    Arc<dyn Fn(Incoming) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

#[derive(Debug)]
pub enum Outcome {
    Received { files: Vec<PathBuf>, texts: Vec<String> },
    Rejected,
    Cancelled,
}

struct PendingFile {
    path: PathBuf,
    file: File,
    size: i64,
    written: i64,
}

struct Session {
    link: Link,
    bytes: BytesPayloads,
    files: HashMap<i64, PendingFile>,
    text_ids: HashMap<i64, String>,
    done_files: Vec<PathBuf>,
    done_texts: Vec<String>,
}

/// Runs one incoming connection to its end.
pub async fn handle(stream: TcpStream, dir: PathBuf, consent: Consent) -> Result<Outcome> {
    let peer = stream.peer_addr().ok();
    let result = run(stream, dir, consent).await;
    if let Err(err) = &result {
        warn!("connection from {peer:?}: {err:#}");
    }
    result
}

async fn run(stream: TcpStream, dir: PathBuf, consent: Consent) -> Result<Outcome> {
    let (mut frames, mut writer) = Frames::split(stream);
    let request = frames.expect(nc::v1_frame::FrameType::ConnectionRequest).await?;
    let request = request
        .connection_request
        .ok_or_else(|| anyhow!("connection request without body"))?;
    let (device_type, sender) = parse_endpoint_info(request.endpoint_info())
        .ok_or_else(|| anyhow!("connection request without endpoint info"))?;
    info!("connection request from {sender:?} ({device_type:?})");

    let (handshake, server_init) = ServerHandshake::accept(&frames.next().await?)?;
    write_frame(&mut writer, &server_init).await?;
    let (channel, pin) = handshake.finish(&frames.next().await?)?;
    info!("UKEY2 done, PIN {pin}");

    frames.expect(nc::v1_frame::FrameType::ConnectionResponse).await?;
    write_frame(&mut writer, &connection_accept().encode_to_vec()).await?;

    let mut session = Session {
        link: Link::new(channel, writer),
        bytes: BytesPayloads::default(),
        files: HashMap::new(),
        text_ids: HashMap::new(),
        done_files: Vec::new(),
        done_texts: Vec::new(),
    };
    session.link.send_sharing(paired_key_encryption()).await?;

    let mut keepalive = tokio::time::interval(KEEPALIVE);
    let mut pending_consent: Option<JoinHandle<bool>> = None;
    let mut accepted = false;
    loop {
        tokio::select! {
            raw = frames.next() => {
                let frame = session.link.open(&raw?)?;
                let Some(v1) = frame.v1 else { continue };
                match v1.r#type() {
                    nc::v1_frame::FrameType::KeepAlive => {}
                    nc::v1_frame::FrameType::Disconnection => {
                        return Ok(if accepted { session.outcome() } else { Outcome::Cancelled });
                    }
                    nc::v1_frame::FrameType::PayloadTransfer => {
                        let transfer = v1.payload_transfer.unwrap_or_default();
                        if let Some(setup) = session.on_payload(transfer)? {
                            match setup.v1.map(|v| (v.r#type(), v)) {
                                Some((sn::v1_frame::FrameType::PairedKeyEncryption, _)) => {
                                    session.link.send_sharing(paired_key_result()).await?;
                                }
                                Some((sn::v1_frame::FrameType::PairedKeyResult, _)) => {}
                                Some((sn::v1_frame::FrameType::Introduction, v)) => {
                                    let intro = v.introduction.unwrap_or_default();
                                    let incoming = session.prepare(&intro, &dir, &pin, sender.clone(), device_type)?;
                                    let consent = consent.clone();
                                    pending_consent = Some(tokio::spawn(consent(incoming)));
                                }
                                Some((sn::v1_frame::FrameType::Cancel, _)) => {
                                    info!("sender cancelled");
                                    session.discard();
                                    return Ok(Outcome::Cancelled);
                                }
                                Some((other, _)) => debug!("ignoring sharing frame {other:?}"),
                                None => {}
                            }
                        }
                        if accepted && session.complete() {
                            session.link.send_disconnection().await?;
                            return Ok(session.outcome());
                        }
                    }
                    other => debug!("ignoring offline frame {other:?}"),
                }
            }
            answer = async { pending_consent.as_mut().unwrap().await }, if pending_consent.is_some() => {
                pending_consent = None;
                let status = if answer.unwrap_or(false) {
                    accepted = true;
                    sn::connection_response_frame::Status::Accept
                } else {
                    sn::connection_response_frame::Status::Reject
                };
                session.link.send_sharing(sn::V1Frame {
                    r#type: Some(sn::v1_frame::FrameType::Response.into()),
                    connection_response: Some(sn::ConnectionResponseFrame {
                        status: Some(status.into()),
                    }),
                    ..Default::default()
                }).await?;
                if !accepted {
                    session.discard();
                    return Ok(Outcome::Rejected);
                }
            }
            _ = keepalive.tick() => session.link.send_keepalive().await?,
        }
    }
}

impl Session {
    /// Collects payload chunks. Returns a sharing frame once a bytes payload
    /// that carries one is complete.
    fn on_payload(&mut self, transfer: nc::PayloadTransferFrame) -> Result<Option<sn::Frame>> {
        let header = transfer.payload_header.unwrap_or_default();
        let chunk = transfer.payload_chunk.unwrap_or_default();
        let id = header.id();
        let last = chunk.flags() & 1 != 0;
        match header.r#type() {
            nc::payload_transfer_frame::payload_header::PayloadType::Bytes => {
                let Some((id, data)) = self.bytes.add(&header, &chunk)? else {
                    return Ok(None);
                };
                if let Some(title) = self.text_ids.remove(&id) {
                    let text = String::from_utf8_lossy(&data).into_owned();
                    info!("received text {title:?}");
                    self.done_texts.push(text);
                    return Ok(None);
                }
                Ok(Some(sn::Frame::decode(data.as_slice())?))
            }
            nc::payload_transfer_frame::payload_header::PayloadType::File => {
                let pending = self
                    .files
                    .get_mut(&id)
                    .ok_or_else(|| anyhow!("file payload {id} was not announced"))?;
                if chunk.offset() != pending.written {
                    bail!("file chunk at {} after {} bytes", chunk.offset(), pending.written);
                }
                let body = chunk.body();
                if pending.written + body.len() as i64 > pending.size {
                    bail!("file {} grows past its announced size", pending.path.display());
                }
                pending.file.write_all_at(body, pending.written as u64)?;
                pending.written += body.len() as i64;
                if last {
                    let pending = self.files.remove(&id).expect("present above");
                    pending.file.sync_all()?;
                    info!("received {}", pending.path.display());
                    self.done_files.push(pending.path);
                }
                Ok(None)
            }
            other => bail!("unsupported payload type {other:?}"),
        }
    }

    /// Creates the target files for an introduction and describes the
    /// transfer for the consent prompt.
    fn prepare(
        &mut self,
        intro: &sn::IntroductionFrame,
        dir: &Path,
        pin: &str,
        sender: Option<String>,
        device_type: DeviceType,
    ) -> Result<Incoming> {
        let mut files = Vec::new();
        for meta in &intro.file_metadata {
            let path = unique_path(dir, meta.name());
            let file = File::options()
                .write(true)
                .create_new(true)
                .open(&path)
                .with_context(|| format!("create {}", path.display()))?;
            files.push(IncomingFile { name: meta.name().to_string(), size: meta.size() });
            self.files.insert(
                meta.payload_id(),
                PendingFile { path, file, size: meta.size(), written: 0 },
            );
        }
        let mut texts = Vec::new();
        for meta in &intro.text_metadata {
            texts.push(meta.text_title().to_string());
            self.text_ids.insert(meta.payload_id(), meta.text_title().to_string());
        }
        Ok(Incoming { sender, device_type, pin: pin.to_string(), files, texts })
    }

    fn complete(&self) -> bool {
        self.files.is_empty() && self.text_ids.is_empty()
    }

    fn outcome(&mut self) -> Outcome {
        Outcome::Received {
            files: std::mem::take(&mut self.done_files),
            texts: std::mem::take(&mut self.done_texts),
        }
    }

    /// Removes files created for a transfer that did not complete.
    fn discard(&mut self) {
        for (_, pending) in self.files.drain() {
            let _ = std::fs::remove_file(&pending.path);
        }
    }

}

/// A free path in `dir` for a name the sender chose. Only the last path
/// component is used, an existing file gets " (n)" before the extension.
pub(crate) fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let base = Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .unwrap_or_else(|| "received".to_string());
    let candidate = dir.join(&base);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (base.clone(), String::new()),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("unbounded range")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_strips_directories_and_numbers_duplicates() {
        let dir = std::env::temp_dir().join(format!("t2airshare-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "../../etc/passwd"), dir.join("passwd"));
        std::fs::write(dir.join("photo.jpg"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "photo.jpg"), dir.join("photo (1).jpg"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
