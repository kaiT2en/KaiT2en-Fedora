// SPDX-License-Identifier: GPL-3.0-or-later
//! Sending side of AirDrop as macOS 15.4.1 receives it, over HTTPS on the
//! receiver's AWDL link local address.
//!
//! What the receiver requires, from sharingd of macOS 15.4.1:
//! - no client certificate, a self signed one fails its trust evaluation
//! - /Discover on its own connection, the receiver closes it afterwards
//! - /Ask and /Upload back to back on one connection, the upload is matched
//!   to the ask of the same connection and the ask connection is dropped
//!   on accept when no upload is waiting on it yet
//! - /Upload with the headers Content-Type and TotalBytes
//!
//! Bodies are binary property lists, the upload a gzip compressed cpio
//! archive in the portable (odc) format.

use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::net::{Ipv6Addr, SocketAddrV6, TcpStream};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use plist::{Dictionary, Value};
use rand::RngCore;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};

/// AirDrop receivers present self signed certificates. As macOS, accept
/// them without verification.
#[derive(Debug)]
struct AcceptAny(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Upload bytes handed to TLS at a time.
const UPLOAD_CHUNK: usize = 128 * 1024;

/// How a send goes on.
#[derive(Debug, Clone, Copy)]
pub enum SendProgress {
    /// The user on the receiver accepted.
    Accepted,
    /// Bytes of the upload sent so far, of all.
    Sent(u64, u64),
}

/// A file to send, the name without directories.
pub struct File {
    pub name: String,
    pub data: Vec<u8>,
}

/// An AirDrop receiver reachable on awdl0.
pub struct Receiver {
    addr: SocketAddrV6,
    config: Arc<ClientConfig>,
}

pub struct Reply {
    pub status: u16,
    pub plist: Option<Dictionary>,
}

type Tls = StreamOwned<ClientConnection, TcpStream>;

impl Receiver {
    pub fn new(link_local: &str, scope: u32, port: u16) -> Result<Self> {
        let ip: Ipv6Addr = link_local.parse().context("link local address")?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
            .with_no_client_auth();
        Ok(Self { addr: SocketAddrV6::new(ip, port, 0, scope), config: Arc::new(config) })
    }

    fn connect(&self, timeout: Duration) -> Result<Tls> {
        let tcp = TcpStream::connect_timeout(&self.addr.into(), Duration::from_secs(3))
            .with_context(|| format!("connect to {}", self.addr))?;
        tcp.set_read_timeout(Some(timeout))?;
        tcp.set_write_timeout(Some(timeout))?;
        let name = ServerName::IpAddress(std::net::IpAddr::V6(*self.addr.ip()).into());
        let conn = ClientConnection::new(self.config.clone(), name)?;
        Ok(StreamOwned::new(conn, tcp))
    }

    fn request(&self, path: &str, body: &[u8], extra: &[(&str, String)]) -> Vec<u8> {
        let mut out = self.head(path, body.len(), extra);
        out.extend_from_slice(body);
        out
    }

    /// The request line and headers of a request with a body of `len` bytes.
    fn head(&self, path: &str, len: usize, extra: &[(&str, String)]) -> Vec<u8> {
        let mut head = format!(
            "POST {path} HTTP/1.1\r\nHost: [{}]:{}\r\nContent-Type: application/octet-stream\r\n\
             Connection: keep-alive\r\nAccept: */*\r\nUser-Agent: AirDrop/1.0\r\n\
             Accept-Language: en-us\r\nAccept-Encoding: gzip, deflate\r\nContent-Length: {}\r\n",
            self.addr.ip(),
            self.addr.port(),
            len
        );
        for (k, v) in extra {
            if *k == "Content-Type" {
                head = head.replace("Content-Type: application/octet-stream", &format!("Content-Type: {v}"));
            } else {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
        }
        head.push_str("\r\n");
        head.into_bytes()
    }

    /// The name the receiver shows for itself, None if it does not answer.
    pub fn discover(&self) -> Result<Option<String>> {
        let mut tls = self.connect(Duration::from_secs(15))?;
        tls.write_all(&self.request("/Discover", &plist_bytes(Dictionary::new())?, &[]))?;
        let reply = read_reply(&mut BufReader::new(&mut tls))?;
        if reply.status != 200 {
            return Ok(None);
        }
        Ok(reply.plist.and_then(|p| p.get("ReceiverComputerName").and_then(Value::as_string).map(str::to_owned)))
    }

    /// Asks the receiver and uploads `files`. Waits up to `wait` for the
    /// user on the receiver. `progress` hears when the user accepted and how
    /// much of the upload went out.
    pub fn send(
        &self,
        sender_name: &str,
        files: Vec<File>,
        wait: Duration,
        progress: &mut dyn FnMut(SendProgress),
    ) -> Result<()> {
        let mut entries = Vec::new();
        let mut archive_files = Vec::new();
        let mut total = 0u64;
        for File { name, data } in files {
            let name = std::path::Path::new(&name)
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow!("no file name in {name}"))?
                .to_owned();
            total += data.len() as u64;
            let mut entry = Dictionary::new();
            entry.insert("FileName".into(), name.clone().into());
            entry.insert("FileType".into(), uti(&name).into());
            entry.insert("FileBomPath".into(), format!("./{name}").into());
            entry.insert("FileIsDirectory".into(), false.into());
            entry.insert("ConvertMediaFormats".into(), 0.into());
            entries.push(Value::Dictionary(entry));
            archive_files.push((format!("./{name}"), data));
        }
        let mut sender_id = [0u8; 6];
        rand::thread_rng().fill_bytes(&mut sender_id);
        let mut ask = Dictionary::new();
        ask.insert("SenderComputerName".into(), sender_name.into());
        ask.insert("SenderModelName".into(), "MacBookPro".into());
        ask.insert("SenderID".into(), sender_id.iter().map(|b| format!("{b:02x}")).collect::<String>().into());
        ask.insert("BundleID".into(), "com.apple.finder".into());
        ask.insert("ConvertMediaFormats".into(), false.into());
        ask.insert("Files".into(), Value::Array(entries));

        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&cpio_odc(&archive_files))?;
        let archive = gz.finish()?;

        let mut tls = self.connect(wait)?;
        tls.write_all(&self.request("/Ask", &plist_bytes(ask)?, &[]))?;
        tls.write_all(&self.head(
            "/Upload",
            archive.len(),
            &[("Content-Type", "application/x-cpio".into()), ("TotalBytes", total.to_string())],
        ))?;
        // The ask reply comes once the user decided, while the upload waits
        // on the same connection
        let (early, accepted) = upload(&mut tls, &archive, wait, progress)?;
        let mut reader = BufReader::new(Cursor::new(early).chain(&mut tls));
        let ask = read_reply(&mut reader).context("ask")?;
        match ask.status {
            200 => {}
            403 => bail!("declined on the receiver"),
            s => bail!("ask failed with HTTP {s}"),
        }
        if !accepted {
            progress(SendProgress::Accepted);
        }
        let upload = read_reply(&mut reader).context("upload")?;
        if upload.status != 200 {
            bail!("upload failed with HTTP {}", upload.status);
        }
        Ok(())
    }
}

/// The HTTP status of a reply that starts `data`, once its status line is in.
fn status_of(data: &[u8]) -> Option<u16> {
    let end = data.iter().position(|&b| b == b'\n')?;
    std::str::from_utf8(&data[..end]).ok()?.split_whitespace().nth(1)?.parse().ok()
}

/// Writes `data` to `tls` and reads what the receiver answers meanwhile.
/// Returns the plaintext received and whether it holds an accepting ask
/// reply. Fails early when the receiver declines. Waits at most `wait`
/// without any progress.
fn upload(
    tls: &mut Tls,
    data: &[u8],
    wait: Duration,
    progress: &mut dyn FnMut(SendProgress),
) -> Result<(Vec<u8>, bool)> {
    let total = data.len() as u64;
    let mut off = 0;
    let mut received = Vec::new();
    let mut accepted = false;
    let mut last = Instant::now();
    tls.sock.set_nonblocking(true)?;
    let result = (|| -> Result<()> {
        loop {
            let mut moved = false;
            if off < data.len() && !tls.conn.wants_write() {
                let end = (off + UPLOAD_CHUNK).min(data.len());
                let n = tls.conn.writer().write(&data[off..end])?;
                off += n;
                moved |= n > 0;
            }
            while tls.conn.wants_write() {
                match tls.conn.write_tls(&mut tls.sock) {
                    Ok(n) => moved |= n > 0,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e).context("upload"),
                }
            }
            if moved && !tls.conn.wants_write() {
                progress(SendProgress::Sent(off as u64, total));
            }
            match tls.conn.read_tls(&mut tls.sock) {
                Ok(0) => bail!("the receiver closed the connection"),
                Ok(_) => {
                    tls.conn.process_new_packets().context("tls")?;
                    let mut buf = [0u8; 4096];
                    loop {
                        match tls.conn.reader().read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => received.extend_from_slice(&buf[..n]),
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(e) => return Err(e).context("tls"),
                        }
                    }
                    moved = true;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e).context("upload"),
            }
            if !accepted {
                match status_of(&received) {
                    Some(200) => {
                        accepted = true;
                        progress(SendProgress::Accepted);
                    }
                    Some(403) => bail!("declined on the receiver"),
                    Some(s) => bail!("ask failed with HTTP {s}"),
                    None => {}
                }
            }
            if off == data.len() && !tls.conn.wants_write() {
                return Ok(());
            }
            if moved {
                last = Instant::now();
                continue;
            }
            let left = wait.saturating_sub(last.elapsed());
            if left.is_zero() {
                bail!("the receiver did not answer");
            }
            let mut fd = libc::pollfd {
                fd: tls.sock.as_raw_fd(),
                events: libc::POLLIN | if tls.conn.wants_write() { libc::POLLOUT } else { 0 },
                revents: 0,
            };
            // SAFETY: fd is a valid pollfd for one descriptor.
            unsafe { libc::poll(&mut fd, 1, left.as_millis().min(1000) as i32) };
        }
    })();
    tls.sock.set_nonblocking(false)?;
    result.map(|()| (received, accepted))
}

fn plist_bytes(dict: Dictionary) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    plist::to_writer_binary(&mut out, &Value::Dictionary(dict))?;
    Ok(out)
}

/// Reads one HTTP/1.1 response with a Content-Length or chunked body.
fn read_reply(r: &mut impl BufRead) -> Result<Reply> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("bad status line {line:?}"))?;
    let mut length = None;
    let mut chunked = false;
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
            let start = body.len();
            body.resize(start + size, 0);
            r.read_exact(&mut body[start..])?;
            line.clear();
            r.read_line(&mut line)?;
        }
    } else if let Some(n) = length {
        body.resize(n, 0);
        r.read_exact(&mut body)?;
    }
    let plist = if body.is_empty() {
        None
    } else {
        plist::from_bytes::<Value>(&body).ok().and_then(Value::into_dictionary)
    };
    Ok(Reply { status, plist })
}

/// A portable (odc) cpio archive of regular files.
fn cpio_odc(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut out = Vec::new();
    let header = |out: &mut Vec<u8>, ino: u64, name: &str, mode: u32, size: u64| {
        out.extend_from_slice(
            format!(
                "070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
                0,
                ino,
                mode,
                0,
                0,
                1,
                0,
                now,
                name.len() + 1,
                size
            )
            .as_bytes(),
        );
        out.extend_from_slice(name.as_bytes());
        out.push(0);
    };
    for (i, (name, data)) in files.iter().enumerate() {
        header(&mut out, i as u64 + 1, name, 0o100644, data.len() as u64);
        out.extend_from_slice(data);
    }
    header(&mut out, 0, "TRAILER!!!", 0, 0);
    out
}

/// Uniform type identifier by file extension.
fn uti(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "txt" => "public.plain-text",
        "jpg" | "jpeg" => "public.jpeg",
        "png" => "public.png",
        "heic" => "public.heic",
        "gif" => "com.compuserve.gif",
        "pdf" => "com.adobe.pdf",
        "mp4" => "public.mpeg-4",
        "mov" => "com.apple.quicktime-movie",
        "zip" => "public.zip-archive",
        _ => "public.data",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_chunked_reply() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n";
        let reply = read_reply(&mut BufReader::new(&raw[..])).unwrap();
        assert_eq!(reply.status, 200);
    }

    #[test]
    fn cpio_starts_with_the_odc_magic() {
        let a = cpio_odc(&[("./x.txt".into(), b"hi".to_vec())]);
        assert!(a.starts_with(b"070707"));
        assert!(a.windows(10).any(|w| w == b"TRAILER!!!"));
    }
}
