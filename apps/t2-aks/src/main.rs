// SPDX-License-Identifier: GPL-2.0-only

//! Userspace side of the Apple T2 AppleKeyStore transport. It builds the AKS
//! wire request, including the SHA-256 header digest, and exchanges it with
//! the Secure Enclave through the t2sep driver's ioctl. The kernel keeps the
//! bytes opaque; the format lives here.

use std::fs;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

const DEVICE: &str = "/dev/t2sep";
const AKS_ENDPOINT: u8 = 7;

const OP_LOAD_KEYBAG: u8 = 0x03;
const OP_CHANGE_LOCK_STATE: u8 = 0x04;
const OP_COPY_KEYBAG_UUID: u8 = 0x06;
const OP_MAKE_SYSTEM_KEYBAG: u8 = 0x0d;
const OP_GET_CAPABILITIES: u8 = 0x4d;

const HEADER_V1_SIZE: usize = 0x48;
const HEADER_V2_SIZE: usize = 0x50;
const OOL_SIZE: usize = 16 * 1024;
const MAX_KEYBAG: usize = 16000;

// _IOWR(0xa7, 0, struct t2sep_exchange), sizeof == 32.
const T2SEP_IOC_EXCHANGE: libc::c_ulong = (3 << 30) | (32u64 << 16) | (0xa7u64 << 8);

#[repr(C)]
#[derive(Default)]
struct Exchange {
    endpoint: u8,
    operation: u8,
    reserved: [u8; 2],
    request_length: u32,
    response_capacity: u32,
    response_length: u32,
    request: u64,
    response: u64,
}

/// Wrap an AKS body in the wire header and fill in the SHA-256 digest. The
/// digest covers the header after its digest field and then the body, which
/// are contiguous from offset 20 onward.
fn build_request(body: &[u8], v2: bool) -> Vec<u8> {
    let header_size = if v2 { HEADER_V2_SIZE } else { HEADER_V1_SIZE };
    let wire_header = 4 + header_size;
    let mut buf = vec![0u8; wire_header + body.len()];
    buf[0..4].copy_from_slice(&(header_size as u32).to_le_bytes());
    // buf[4..20] is the digest, filled last.
    buf[20..24].copy_from_slice(&(if v2 { 2u32 } else { 1u32 }).to_le_bytes());
    let usec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    buf[24..32].copy_from_slice(&usec.to_le_bytes());
    // flags, clock_id and platform_data stay zero.
    if v2 {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        buf[4 + HEADER_V1_SIZE..4 + HEADER_V2_SIZE].copy_from_slice(&secs.to_le_bytes());
    }
    buf[wire_header..].copy_from_slice(body);

    let mut hasher = Sha256::new();
    hasher.update(&buf[20..]);
    let digest = hasher.finalize();
    buf[4..20].copy_from_slice(&digest[..16]);
    buf
}

fn exchange(fd: i32, operation: u8, request: &[u8]) -> io::Result<Vec<u8>> {
    let mut response = vec![0u8; OOL_SIZE];
    let mut ex = Exchange {
        endpoint: AKS_ENDPOINT,
        operation,
        request_length: request.len() as u32,
        response_capacity: response.len() as u32,
        request: request.as_ptr() as u64,
        response: response.as_mut_ptr() as u64,
        ..Default::default()
    };
    let ret = unsafe { libc::ioctl(fd, T2SEP_IOC_EXCHANGE, &mut ex as *mut Exchange) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    response.truncate(ex.response_length as usize);
    Ok(response)
}

/// Skip the response wire header, whichever version SEP used, and return the body.
fn response_body(response: &[u8]) -> io::Result<&[u8]> {
    if response.len() < 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated response"));
    }
    let header_size = u32::from_le_bytes(response[0..4].try_into().unwrap()) as usize;
    let wire_header = 4 + header_size;
    if response.len() < wire_header {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "response shorter than header"));
    }
    Ok(&response[wire_header..])
}

fn le32(v: u32) -> [u8; 4] { v.to_le_bytes() }
fn le64(v: u64) -> [u8; 8] { v.to_le_bytes() }

fn get_capabilities(fd: i32, v2: bool) -> io::Result<()> {
    let mut body = Vec::new();
    body.extend_from_slice(&le32(0)); // result placeholder
    body.extend_from_slice(&le64(1)); // selector 1
    body.extend_from_slice(&le32(0)); // empty blob
    let response = exchange(fd, OP_GET_CAPABILITIES, &build_request(&body, v2))?;
    let payload = response_body(&response)?;
    if payload.len() < 12 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short capability body"));
    }
    let status = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    if status != 0 {
        return Err(io::Error::other(format!("SEP status {status:#010x}")));
    }
    let caps = u64::from_le_bytes(payload[4..12].try_into().unwrap());
    println!("capabilities={caps:#018x} reply_length={}", response.len());
    Ok(())
}

fn load_keybag(fd: i32, path: &str, session: u64, v2: bool) -> io::Result<()> {
    let keybag = fs::read(path)?;
    if keybag.is_empty() || keybag.len() > MAX_KEYBAG {
        return Err(io::Error::other(format!("invalid keybag size {}", keybag.len())));
    }
    let padded = (keybag.len() + 3) & !3;
    let mut body = vec![0u8; 16 + padded];
    body[4..12].copy_from_slice(&le64(session));
    body[12..16].copy_from_slice(&le32(keybag.len() as u32));
    body[16..16 + keybag.len()].copy_from_slice(&keybag);
    let response = exchange(fd, OP_LOAD_KEYBAG, &build_request(&body, v2))?;
    let payload = response_body(&response)?;
    if payload.len() < 8 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short load response"));
    }
    let status = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    let handle = i32::from_le_bytes(payload[4..8].try_into().unwrap());
    println!("status={status:#010x} handle={handle}");
    Ok(())
}

fn copy_keybag_uuid(fd: i32, handle: i32, session: u64, v2: bool) -> io::Result<()> {
    let mut body = Vec::new();
    body.extend_from_slice(&le32(0));
    body.extend_from_slice(&le64(session));
    body.extend_from_slice(&le32(handle as u32));
    let response = exchange(fd, OP_COPY_KEYBAG_UUID, &build_request(&body, v2))?;
    let payload = response_body(&response)?;
    if payload.len() < 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short uuid response"));
    }
    let status = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    if status != 0 {
        return Err(io::Error::other(format!("SEP status {status:#010x}")));
    }
    let uuid = &payload[4..];
    let prefix: String = uuid.iter().take(4).map(|b| format!("{b:02x}")).collect();
    println!("status=0 uuid_prefix={prefix} uuid_bytes={}", uuid.len());
    Ok(())
}

fn make_system_keybag(fd: i32, handle: i32, special: i32, session: u64, v2: bool) -> io::Result<()> {
    let mut body = vec![0u8; 24];
    body[4..12].copy_from_slice(&le64(session));
    body[12..16].copy_from_slice(&le32(handle as u32));
    body[16..20].copy_from_slice(&le32(special as u32));
    // body[20..24] empty blob length (0).
    let response = exchange(fd, OP_MAKE_SYSTEM_KEYBAG, &build_request(&body, v2))?;
    let payload = response_body(&response)?;
    if payload.len() < 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short set-system response"));
    }
    let status = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    println!("status={status:#010x}");
    Ok(())
}

fn unlock_keybag(fd: i32, handle: i32, session: u64, v2: bool) -> io::Result<()> {
    // Password on stdin, so PAM (expose_authtok) and a manual pipe share the
    // same path. Never on argv. Unlock attempts are rate limited by SEP, so
    // do not loop.
    let mut secret = Vec::new();
    io::stdin().read_to_end(&mut secret)?;
    while matches!(secret.last(), Some(b'\n') | Some(b'\r')) {
        secret.pop();
    }
    if secret.is_empty() || secret.len() > 128 {
        return Err(io::Error::other("password must be 1 to 128 bytes"));
    }
    let padded = (secret.len() + 3) & !3;
    let mut body = vec![0u8; 24 + padded];
    body[4..12].copy_from_slice(&le64(session));
    body[12..16].copy_from_slice(&le32(handle as u32));
    // body[16..20] lock-state 0 (unlock).
    body[20..24].copy_from_slice(&le32(secret.len() as u32));
    body[24..24 + secret.len()].copy_from_slice(&secret);
    let request = build_request(&body, v2);
    body.iter_mut().for_each(|b| *b = 0);
    secret.iter_mut().for_each(|b| *b = 0);
    let response = exchange(fd, OP_CHANGE_LOCK_STATE, &request)?;
    let payload = response_body(&response)?;
    if payload.len() < 4 {
        let raw: String = response.iter().map(|b| format!("{b:02x}")).collect();
        return Err(io::Error::other(format!(
            "short unlock response: {} bytes body={} raw={raw}",
            response.len(),
            payload.len()
        )));
    }
    let status = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    println!("status={status:#010x}");
    Ok(())
}

fn usage() -> ! {
    eprintln!(
        "usage: t2-aks [--v1] [--session N] COMMAND\n\
         \n\
         commands:\n\
         \x20 capabilities            read the AKS capability word\n\
         \x20 load-keybag FILE        load a keybag, print its handle\n\
         \x20 copy-uuid HANDLE        read a loaded keybag's UUID prefix\n\
         \x20 set-system HANDLE SPECIAL  bind the loaded keybag as the system keybag\n\
         \x20 unlock HANDLE           unlock a keybag; password on stdin\n\
         \n\
         The v2 header is the default; --v1 selects the older header.\n\
         Unlock attempts are rate limited by SEP: do not guess the password."
    );
    std::process::exit(2);
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut v2 = true;
    let mut session: u64 = 1;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--v1" => { v2 = false; args.remove(i); }
            "--v2" => { v2 = true; args.remove(i); }
            "--session" => {
                if i + 1 >= args.len() { usage(); }
                session = args[i + 1].parse().unwrap_or_else(|_| usage());
                args.drain(i..i + 2);
            }
            _ => i += 1,
        }
    }
    if args.is_empty() {
        usage();
    }

    let file = match fs::OpenOptions::new().read(true).write(true).open(DEVICE) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("open {DEVICE}: {e}");
            std::process::exit(1);
        }
    };
    let fd = file.as_raw_fd();

    let parse_handle = |s: &str| -> i32 {
        s.parse::<i32>().unwrap_or_else(|_| {
            eprintln!("invalid handle: {s}");
            std::process::exit(2);
        })
    };

    let result = match args[0].as_str() {
        "capabilities" => get_capabilities(fd, v2),
        "load-keybag" if args.len() == 2 => load_keybag(fd, &args[1], session, v2),
        "copy-uuid" if args.len() == 2 => copy_keybag_uuid(fd, parse_handle(&args[1]), session, v2),
        "set-system" if args.len() == 3 => {
            make_system_keybag(fd, parse_handle(&args[1]), parse_handle(&args[2]), session, v2)
        }
        "unlock" if args.len() == 2 => unlock_keybag(fd, parse_handle(&args[1]), session, v2),
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("{}: {e}", args[0]);
        std::process::exit(1);
    }
}
