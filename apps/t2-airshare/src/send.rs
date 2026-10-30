// SPDX-License-Identifier: GPL-3.0-or-later
//! Sending side (client) of a Quick Share transfer: connection request,
//! UKEY2 as client, paired key exchange, the introduction, the receiver's
//! answer, then the files in chunks.
//!
//! The frame sequence follows NearDrop PROTOCOL.md and rquickshare
//! `core_lib/src/hdl/outbound.rs` (commit 378d8ae, GPL-3.0).

use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use log::{debug, info};
use prost::Message;
use tokio::net::TcpStream;

use crate::endpoint::{DeviceType, endpoint_info, random_endpoint_id};
use crate::link::{
    BytesPayloads, Frames, KEEPALIVE, Link, connection_accept, offline_frame,
    paired_key_encryption, paired_key_result, random_payload_id, write_frame,
};
use crate::proto::location::nearby::connections as nc;
use crate::proto::sharing::nearby as sn;
use crate::secure::ClientHandshake;

/// Chunk size for file payloads, the size Android uses as well.
const CHUNK: usize = 512 * 1024;
/// How long the receiver may take to accept.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub enum SendOutcome {
    Sent,
    /// The receiver declined, with the status it gave.
    Declined(String),
}

/// Progress callbacks for a running transfer.
pub trait Progress: Send {
    /// The receiver now shows this PIN and asks its user.
    fn waiting(&mut self, _pin: &str) {}
    /// `sent` of `total` bytes are on their way.
    fn sent(&mut self, _sent: u64, _total: u64) {}
}

impl Progress for () {}

struct OutgoingFile {
    path: PathBuf,
    meta: sn::FileMetadata,
}

/// Sends `files` to the receiver at `addr` and returns once it has them.
pub async fn send(
    addr: SocketAddr,
    our_name: &str,
    files: &[PathBuf],
    progress: &mut dyn Progress,
) -> Result<SendOutcome> {
    let files = files.iter().map(|p| describe(p)).collect::<Result<Vec<_>>>()?;
    if files.is_empty() {
        bail!("nothing to send");
    }
    let stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow!("{addr} did not answer"))?
        .with_context(|| format!("connect to {addr}"))?;
    let (mut frames, mut writer) = Frames::split(stream);

    let endpoint_id = random_endpoint_id();
    let request = offline_frame(nc::V1Frame {
        r#type: Some(nc::v1_frame::FrameType::ConnectionRequest.into()),
        connection_request: Some(nc::ConnectionRequestFrame {
            endpoint_id: Some(String::from_utf8_lossy(&endpoint_id).into_owned()),
            endpoint_name: Some(our_name.as_bytes().to_vec()),
            endpoint_info: Some(endpoint_info(DeviceType::Laptop, our_name)),
            mediums: vec![nc::connection_request_frame::Medium::WifiLan.into()],
            ..Default::default()
        }),
        ..Default::default()
    });
    write_frame(&mut writer, &request.encode_to_vec()).await?;

    let (handshake, client_init) = ClientHandshake::start()?;
    write_frame(&mut writer, &client_init).await?;
    let (channel, pin, client_finish) = handshake.finish(&frames.next().await?)?;
    write_frame(&mut writer, &client_finish).await?;
    write_frame(&mut writer, &connection_accept().encode_to_vec()).await?;
    info!("UKEY2 done, PIN {pin}");

    let response = frames.expect(nc::v1_frame::FrameType::ConnectionResponse).await?;
    let status = response.connection_response.unwrap_or_default().response();
    if status != nc::connection_response_frame::ResponseStatus::Accept {
        return Ok(SendOutcome::Declined(format!("{status:?}")));
    }

    let mut link = Link::new(channel, writer);
    let mut bytes = BytesPayloads::default();
    link.send_sharing(paired_key_encryption()).await?;

    let mut keepalive = tokio::time::interval(KEEPALIVE);
    let answer_deadline = tokio::time::Instant::now() + ANSWER_TIMEOUT;
    loop {
        let raw = tokio::select! {
            raw = frames.next() => raw?,
            _ = keepalive.tick() => {
                link.send_keepalive().await?;
                continue;
            }
            _ = tokio::time::sleep_until(answer_deadline) => bail!("no answer from the receiver"),
        };
        let frame = link.open(&raw)?;
        let Some(v1) = frame.v1 else { continue };
        match v1.r#type() {
            nc::v1_frame::FrameType::Disconnection => bail!("the receiver closed the connection"),
            nc::v1_frame::FrameType::PayloadTransfer => {}
            other => {
                debug!("ignoring offline frame {other:?}");
                continue;
            }
        }
        let transfer = v1.payload_transfer.unwrap_or_default();
        let header = transfer.payload_header.unwrap_or_default();
        let chunk = transfer.payload_chunk.unwrap_or_default();
        let Some((_, data)) = bytes.add(&header, &chunk)? else { continue };
        let Some(sharing) = sn::Frame::decode(data.as_slice())?.v1 else { continue };
        match sharing.r#type() {
            sn::v1_frame::FrameType::PairedKeyEncryption => {
                link.send_sharing(paired_key_result()).await?;
            }
            sn::v1_frame::FrameType::PairedKeyResult => {
                link.send_sharing(sn::V1Frame {
                    r#type: Some(sn::v1_frame::FrameType::Introduction.into()),
                    introduction: Some(sn::IntroductionFrame {
                        file_metadata: files.iter().map(|f| f.meta.clone()).collect(),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .await?;
                progress.waiting(&pin);
            }
            sn::v1_frame::FrameType::Response => {
                let status = sharing.connection_response.unwrap_or_default().status();
                if status != sn::connection_response_frame::Status::Accept {
                    link.send_disconnection().await?;
                    return Ok(SendOutcome::Declined(format!("{status:?}")));
                }
                send_files(&mut link, &files, progress).await?;
                link.send_disconnection().await?;
                // Give the receiver time to read the last chunks before the
                // connection closes, it disconnects itself when done.
                let _ = tokio::time::timeout(Duration::from_secs(10), async {
                    while frames.next().await.is_ok() {}
                })
                .await;
                return Ok(SendOutcome::Sent);
            }
            sn::v1_frame::FrameType::Cancel => bail!("the receiver cancelled"),
            other => debug!("ignoring sharing frame {other:?}"),
        }
    }
}

async fn send_files(
    link: &mut Link,
    files: &[OutgoingFile],
    progress: &mut dyn Progress,
) -> Result<()> {
    let total: u64 = files.iter().map(|f| f.meta.size() as u64).sum();
    let mut sent = 0u64;
    let mut buffer = vec![0u8; CHUNK];
    for file in files {
        let size = file.meta.size();
        let header = nc::payload_transfer_frame::PayloadHeader {
            id: Some(file.meta.payload_id()),
            r#type: Some(nc::payload_transfer_frame::payload_header::PayloadType::File.into()),
            total_size: Some(size),
            is_sensitive: Some(false),
            file_name: Some(file.meta.name().to_string()),
            ..Default::default()
        };
        let mut source =
            File::open(&file.path).with_context(|| format!("open {}", file.path.display()))?;
        let mut offset = 0i64;
        while offset < size {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                bail!("{} got shorter while sending", file.path.display());
            }
            link.send_chunk(&header, offset, false, buffer[..read].to_vec()).await?;
            offset += read as i64;
            sent += read as u64;
            progress.sent(sent, total);
        }
        link.send_chunk(&header, size, true, Vec::new()).await?;
        info!("sent {}", file.path.display());
    }
    Ok(())
}

fn describe(path: &Path) -> Result<OutgoingFile> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a file", path.display());
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow!("{} has no file name", path.display()))?;
    let mime = mime_type(&name);
    let kind = match mime.split('/').next() {
        Some("image") => sn::file_metadata::Type::Image,
        Some("video") => sn::file_metadata::Type::Video,
        Some("audio") => sn::file_metadata::Type::Audio,
        _ if name.ends_with(".apk") => sn::file_metadata::Type::App,
        _ => sn::file_metadata::Type::Unknown,
    };
    Ok(OutgoingFile {
        path: path.to_path_buf(),
        meta: sn::FileMetadata {
            name: Some(name),
            r#type: Some(kind.into()),
            payload_id: Some(random_payload_id()),
            size: Some(metadata.len() as i64),
            mime_type: Some(mime.to_string()),
            ..Default::default()
        },
    })
}

/// MIME type by extension for the types phones handle specially, the rest
/// goes as application/octet-stream.
fn mime_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "apk" => "application/vnd.android.package-archive",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "vcf" => "text/vcard",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sends_to_our_own_receiver() {
        use std::sync::Arc;

        use crate::receive::{self, Consent, Outcome};

        let dir = std::env::temp_dir().join(format!("t2airshare-loop-{}", std::process::id()));
        let inbox = dir.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let source = dir.join("photo.jpg");
        let content: Vec<u8> = (0..1_300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&source, &content).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let consent: Consent = Arc::new(|_| Box::pin(async { true }));
        let target = inbox.clone();
        let receiver = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            receive::handle(stream, target, consent).await
        });

        let outcome = send(addr, "Loopback", &[source], &mut ()).await.unwrap();
        assert!(matches!(outcome, SendOutcome::Sent));
        match receiver.await.unwrap().unwrap() {
            Outcome::Received { files, .. } => {
                assert_eq!(files, vec![inbox.join("photo.jpg")]);
                assert_eq!(std::fs::read(&files[0]).unwrap(), content);
            }
            other => panic!("unexpected {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mime_types_by_extension() {
        assert_eq!(mime_type("IMG_1234.JPG"), "image/jpeg");
        assert_eq!(mime_type("clip.mp4"), "video/mp4");
        assert_eq!(mime_type("notes"), "application/octet-stream");
    }
}
