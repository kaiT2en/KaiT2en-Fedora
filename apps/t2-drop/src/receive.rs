// SPDX-License-Identifier: GPL-3.0-or-later
//! Receiving side of AirDrop: an HTTPS server on the AWDL link local
//! address that answers /Discover, asks the user on /Ask and unpacks the
//! files of /Upload.
//!
//! The server presents a self signed certificate made at start, it neither
//! asks for nor checks a client certificate. Only regular files of the
//! uploaded archive are kept, paths with ".." or a leading "/" are dropped.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv6Addr, SocketAddrV6, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::{GzDecoder, ZlibDecoder};
use log::{debug, info, warn};
use plist::{Dictionary, Value};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use crate::airdrop::File;

/// Largest request body accepted, the files are held in memory.
const MAX_BODY: usize = 2 << 30;
/// How long a connection may stay idle, the ask waits for the user.
const IDLE: Duration = Duration::from_secs(180);

/// What the sender asks for.
#[derive(Debug, Clone)]
pub struct Ask {
    /// Random id of this transfer
    pub id: String,
    pub sender_name: String,
    pub sender_model: String,
    pub files: Vec<String>,
}

/// Decides on transfers and takes the received files.
pub trait Handler: Send + Sync {
    /// The name shown on the sender.
    fn name(&self) -> String;
    /// Whether the user accepts, may block until the user decides.
    fn ask(&self, ask: &Ask) -> bool;
    fn deliver(&self, ask: &Ask, files: Vec<File>) -> Result<()>;
    /// An accepted transfer starts (true) or its connection ends (false).
    fn transfer(&self, active: bool);
}

/// Reports the end of an accepted transfer however its connection ends.
struct Transfer<'a>(&'a dyn Handler);

impl Drop for Transfer<'_> {
    fn drop(&mut self) {
        self.0.transfer(false);
    }
}

pub struct Server {
    listener: TcpListener,
    config: Arc<ServerConfig>,
}

impl Server {
    /// Listens on `link_local` of interface `scope`. Fails while the
    /// address still runs duplicate address detection.
    pub fn bind(link_local: &str, scope: u32, port: u16) -> Result<Self> {
        let ip: Ipv6Addr = link_local.parse().context("link local address")?;
        let listener = TcpListener::bind(SocketAddrV6::new(ip, port, 0, scope))
            .with_context(|| format!("listen on [{ip}%{scope}]:{port}"))?;
        let cert = rcgen::generate_simple_self_signed(vec!["airdrop.local".to_string()])?;
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(cert.cert.der().to_vec())], key)?;
        Ok(Self { listener, config: Arc::new(config) })
    }

    /// Serves connections until `stop` is set or the listener fails. Every
    /// connection gets its own thread.
    pub fn serve(self, handler: Arc<dyn Handler>, stop: &AtomicBool) {
        if let Err(err) = self.listener.set_nonblocking(true) {
            warn!("AirDrop server: {err}");
            return;
        }
        while !stop.load(Ordering::Relaxed) {
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(err) => {
                    warn!("AirDrop server: {err}");
                    break;
                }
            };
            if stream.set_nonblocking(false).is_err() {
                continue;
            }
            let config = self.config.clone();
            let handler = handler.clone();
            std::thread::spawn(move || {
                let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
                if let Err(err) = serve_connection(stream, config, handler.as_ref()) {
                    debug!("AirDrop connection {peer}: {err:#}");
                }
            });
        }
    }
}

struct Request {
    path: String,
    content_type: Option<String>,
    body: Vec<u8>,
}

fn serve_connection(tcp: TcpStream, config: Arc<ServerConfig>, handler: &dyn Handler) -> Result<()> {
    tcp.set_read_timeout(Some(IDLE))?;
    tcp.set_write_timeout(Some(IDLE))?;
    let conn = ServerConnection::new(config)?;
    let mut tls = StreamOwned::new(conn, tcp);
    let mut ask: Option<(Ask, bool)> = None;
    let mut transfer = None;
    loop {
        let request = {
            let mut reader = BufReader::new(&mut tls);
            match read_request(&mut reader)? {
                Some(r) => r,
                None => return Ok(()),
            }
        };
        debug!("AirDrop {} {} bytes", request.path, request.body.len());
        let (status, body) = match request.path.as_str() {
            "/Discover" => (200, plist_reply(handler, Dictionary::new())?),
            "/Ask" => {
                let a = parse_ask(&request.body)?;
                info!("AirDrop from {} ({}): {:?}", a.sender_name, a.sender_model, a.files);
                let accepted = handler.ask(&a);
                ask = Some((a, accepted));
                if accepted && transfer.is_none() {
                    handler.transfer(true);
                    transfer = Some(Transfer(handler));
                }
                if accepted {
                    (200, plist_reply(handler, Dictionary::new())?)
                } else {
                    (403, Vec::new())
                }
            }
            "/Upload" => match &ask {
                Some((a, true)) => {
                    info!("AirDrop upload {:?}, {} bytes", request.content_type, request.body.len());
                    match unpack(&request.body, request.content_type.as_deref()) {
                        Ok(files) => {
                            handler.deliver(a, files)?;
                            (200, Vec::new())
                        }
                        Err(err) => {
                            warn!("AirDrop upload: {err:#}");
                            (415, Vec::new())
                        }
                    }
                }
                _ => (403, Vec::new()),
            },
            _ => (404, Vec::new()),
        };
        write_reply(&mut tls, status, &body)?;
    }
}

fn plist_reply(handler: &dyn Handler, mut dict: Dictionary) -> Result<Vec<u8>> {
    dict.insert("ReceiverComputerName".into(), handler.name().into());
    dict.insert("ReceiverModelName".into(), "MacBookPro".into());
    let mut out = Vec::new();
    plist::to_writer_binary(&mut out, &Value::Dictionary(dict))?;
    Ok(out)
}

fn write_reply(w: &mut impl Write, status: u16, body: &[u8]) -> Result<()> {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        415 => "Unsupported Media Type",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/octet-stream\r\n\
         Content-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    );
    w.write_all(head.as_bytes())?;
    w.write_all(body)?;
    w.flush()?;
    Ok(())
}

/// Reads one request, None when the peer closed the connection.
fn read_request(r: &mut impl BufRead) -> Result<Option<Request>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().ok_or_else(|| anyhow!("bad request line {line:?}"))?.to_owned();
    if method != "POST" {
        bail!("unexpected {method} {path}");
    }
    let mut length = None;
    let mut chunked = false;
    let mut content_type = None;
    loop {
        line.clear();
        r.read_line(&mut line)?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            let v = v.trim();
            if k.eq_ignore_ascii_case("content-length") {
                length = v.parse::<usize>().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked") {
                chunked = true;
            } else if k.eq_ignore_ascii_case("content-type") {
                content_type = Some(v.to_owned());
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            r.read_line(&mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16)?;
            if size == 0 {
                line.clear();
                r.read_line(&mut line)?;
                break;
            }
            if body.len() + size > MAX_BODY {
                bail!("request body too large");
            }
            let start = body.len();
            body.resize(start + size, 0);
            r.read_exact(&mut body[start..])?;
            line.clear();
            r.read_line(&mut line)?;
        }
    } else if let Some(n) = length {
        if n > MAX_BODY {
            bail!("request body too large");
        }
        body.resize(n, 0);
        r.read_exact(&mut body)?;
    }
    Ok(Some(Request { path, content_type, body }))
}

fn parse_ask(body: &[u8]) -> Result<Ask> {
    let dict = plist::from_bytes::<Value>(body)?.into_dictionary().ok_or_else(|| anyhow!("ask is no dictionary"))?;
    let text = |k: &str| dict.get(k).and_then(Value::as_string).unwrap_or_default().to_owned();
    let mut files = Vec::new();
    if let Some(list) = dict.get("Files").and_then(Value::as_array) {
        for f in list {
            if let Some(name) = f.as_dictionary().and_then(|d| d.get("FileName")).and_then(Value::as_string) {
                files.push(name.to_owned());
            }
        }
    }
    if let Some(items) = dict.get("Items").and_then(Value::as_array) {
        for item in items {
            if let Some(url) = item.as_string() {
                files.push(url.to_owned());
            }
        }
    }
    let id = format!("{:016x}", rand::random::<u64>());
    Ok(Ask { id, sender_name: text("SenderComputerName"), sender_model: text("SenderModelName"), files })
}

/// The files of an upload: a cpio archive, gzip compressed, in DVZip
/// chunks or plain.
fn unpack(body: &[u8], content_type: Option<&str>) -> Result<Vec<File>> {
    let data = if content_type.is_some_and(|t| t.contains("dvzip")) {
        dvzip(body)?
    } else if body.starts_with(&[0x1f, 0x8b]) {
        let mut data = Vec::new();
        GzDecoder::new(body).read_to_end(&mut data).context("gunzip")?;
        data
    } else {
        body.to_vec()
    };
    cpio_files(&data)
}

/// The cpio archive in a DVZip upload, as SDAirDropCompressor of sharingd
/// writes it: chunks with a 32 bit big endian header, bit 31 set for
/// stored data, otherwise zlib data of at most 128 KiB unpacked, the low
/// 31 bits are the length.
fn dvzip(mut body: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while !body.is_empty() {
        let header = u32::from_be_bytes(body.get(..4).context("short DVZip chunk header")?.try_into()?);
        let len = (header & 0x7fff_ffff) as usize;
        let chunk = body.get(4..4 + len).context("short DVZip chunk")?;
        if header & 0x8000_0000 != 0 {
            out.extend_from_slice(chunk);
        } else {
            ZlibDecoder::new(chunk).take(0x20000).read_to_end(&mut out).context("DVZip chunk")?;
        }
        if out.len() > MAX_BODY {
            bail!("upload too large");
        }
        body = &body[4 + len..];
    }
    Ok(out)
}

/// Regular files of an odc (070707) or newc (070701, 070702) cpio archive.
fn cpio_files(mut data: &[u8]) -> Result<Vec<File>> {
    let mut files = Vec::new();
    loop {
        if data.len() < 6 {
            bail!("cpio archive ends early");
        }
        let (mode, namesize, filesize, header, align) = match &data[..6] {
            b"070707" => {
                let field = |o: usize, l: usize| -> Result<u64> {
                    Ok(u64::from_str_radix(std::str::from_utf8(data.get(o..o + l).context("short header")?)?, 8)?)
                };
                (field(18, 6)?, field(59, 6)? as usize, field(65, 11)? as usize, 76, 1)
            }
            b"070701" | b"070702" => {
                let field = |n: usize| -> Result<u64> {
                    let o = 6 + n * 8;
                    Ok(u64::from_str_radix(std::str::from_utf8(data.get(o..o + 8).context("short header")?)?, 16)?)
                };
                (field(1)?, field(11)? as usize, field(6)? as usize, 110, 4)
            }
            _ => bail!("not a cpio archive"),
        };
        let name_end = header + namesize;
        let raw_name = data.get(header..name_end.saturating_sub(1)).context("short name")?;
        let name = String::from_utf8_lossy(raw_name).into_owned();
        let data_start = name_end.next_multiple_of(align);
        let data_end = data_start + filesize;
        let content = data.get(data_start..data_end).context("short file data")?;
        if name == "TRAILER!!!" {
            return Ok(files);
        }
        if mode & 0o170000 == 0o100000 {
            match safe_path(&name) {
                Some(path) => files.push(File { name: path, data: content.to_vec() }),
                None => warn!("AirDrop upload: skipping {name:?}"),
            }
        }
        data = &data[data_end.next_multiple_of(align).min(data.len())..];
    }
}

/// A relative path without "." and ".." components, None if unsafe.
fn safe_path(name: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p => parts.push(p),
        }
    }
    if name.starts_with('/') || parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_odc_archives() {
        let mut a = Vec::new();
        for (name, mode, data) in [("./x.txt", 0o100644u32, &b"hi"[..]), ("TRAILER!!!", 0, &b""[..])] {
            a.extend(format!("070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
                0, 1, mode, 0, 0, 1, 0, 0, name.len() + 1, data.len()).as_bytes());
            a.extend(name.as_bytes());
            a.push(0);
            a.extend(data);
        }
        let files = cpio_files(&a).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].name.as_str(), files[0].data.as_slice()), ("x.txt", &b"hi"[..]));
    }

    #[test]
    fn reads_dvzip_chunks() {
        use flate2::{Compression, write::ZlibEncoder};
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(b"hello ").unwrap();
        let packed = z.finish().unwrap();
        let mut body = (packed.len() as u32).to_be_bytes().to_vec();
        body.extend(&packed);
        body.extend((5u32 | 0x8000_0000).to_be_bytes());
        body.extend(b"world");
        assert_eq!(dvzip(&body).unwrap(), b"hello world");
    }

    #[test]
    fn rejects_paths_that_leave_the_folder() {
        assert_eq!(safe_path("./a/b.jpg").as_deref(), Some("a/b.jpg"));
        assert!(safe_path("../etc/passwd").is_none());
        assert!(safe_path("/etc/passwd").is_none());
        assert!(safe_path("a/../../b").is_none());
    }
}
