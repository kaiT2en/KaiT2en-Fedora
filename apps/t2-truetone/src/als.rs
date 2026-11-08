// SPDX-License-Identifier: GPL-3.0-or-later
//! Ambient light sensor of the T2 (USB HID 05ac:8262), read through hidraw.
//!
//! Input report 1, 18 bytes, little endian:
//! id, sensor event, illuminance (exponent -3, lux), color temperature (K),
//! chromaticity x and y (exponent -8). The kernel IIO driver exposes the same
//! fields but does not apply these exponents, so the report is decoded here.
//!
//! The T2 only sends input reports when the illuminance moves by 15 % or
//! 2 lux. A GET_REPORT asks for a fresh sample instead and returns all fields
//! of one measurement together, so the sensor is polled that way.

use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const VID_PID: &str = ":05AC:8262.";
const REPORT_LEN: usize = 18;
pub const POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub lux: f64,
    pub xy: [f64; 2],
    pub cct: f64,
    pub at: Instant,
}

pub fn find_hidraw() -> Result<PathBuf> {
    for e in fs::read_dir("/sys/bus/hid/devices").context("reading /sys/bus/hid/devices")? {
        let e = e?;
        if !e.file_name().to_string_lossy().contains(VID_PID) {
            continue;
        }
        if let Ok(mut nodes) = fs::read_dir(e.path().join("hidraw")) {
            if let Some(Ok(n)) = nodes.next() {
                return Ok(PathBuf::from("/dev").join(n.file_name()));
            }
        }
    }
    bail!("ambient light sensor 05ac:8262 not found")
}

pub fn parse(rep: &[u8]) -> Option<Sample> {
    if rep.len() < REPORT_LEN || rep[0] != 1 {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes([rep[o], rep[o + 1], rep[o + 2], rep[o + 3]]) as f64;
    Some(Sample {
        lux: u(2) / 1e3,
        cct: u(6),
        xy: [u(10) / 1e8, u(14) / 1e8],
        at: Instant::now(),
    })
}

/// HIDIOCGINPUT(len): _IOC(_IOC_READ | _IOC_WRITE, 'H', 0x0A, len)
const fn hidiocginput(len: usize) -> libc::c_ulong {
    ((3 << 30) | ((len as libc::c_ulong) << 16) | ((b'H' as libc::c_ulong) << 8) | 0x0A) as libc::c_ulong
}

pub fn read(f: &File) -> Result<Sample> {
    let mut buf = [0u8; REPORT_LEN];
    buf[0] = 1;
    // SAFETY: the buffer holds REPORT_LEN bytes, the length encoded in the request.
    let r = unsafe { libc::ioctl(f.as_raw_fd(), hidiocginput(REPORT_LEN), buf.as_mut_ptr()) };
    if r < 0 {
        return Err(std::io::Error::last_os_error()).context("HIDIOCGINPUT");
    }
    parse(&buf[..r as usize]).context("short or foreign input report")
}

/// The sensor is a USB device behind the T2 and comes back with a new hidraw
/// node after suspend, so a lost device is looked for again.
const RETRY: Duration = Duration::from_secs(5);

fn open() -> Result<File> {
    let p = find_hidraw()?;
    OpenOptions::new().read(true).write(true).open(&p).with_context(|| format!("opening {}", p.display()))
}

/// Poll the sensor and send each sample to every receiver until none is left.
/// A problem is logged once when it starts and once when it is over, not on
/// every retry.
pub fn run(txs: Vec<Sender<Sample>>) {
    let mut txs = txs;
    let mut failing: Option<String> = None;
    loop {
        let err = match open() {
            Ok(f) => {
                if failing.take().is_some() {
                    eprintln!("t2-truetone: ambient light sensor available again");
                }
                loop {
                    match read(&f) {
                        Ok(s) => {
                            txs.retain(|tx| tx.send(s).is_ok());
                            if txs.is_empty() {
                                return;
                            }
                        }
                        Err(e) => break e,
                    }
                    std::thread::sleep(POLL);
                }
            }
            Err(e) => e,
        };
        let msg = format!("{err:#}");
        if failing.as_deref() != Some(msg.as_str()) {
            eprintln!("t2-truetone: ambient light sensor: {msg}, retrying every {}s", RETRY.as_secs());
            failing = Some(msg);
        }
        std::thread::sleep(RETRY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_macos_reference_report() {
        // illum 7322, cct 3066, x 43838113, y 39835248, logged by macOS as
        // lux 7.322 x 0.438381 y 0.398352 CCT 3066
        let mut r = vec![1u8, 2];
        for v in [7322u32, 3066, 43838113, 39835248] {
            r.extend_from_slice(&v.to_le_bytes());
        }
        let s = parse(&r).unwrap();
        assert!((s.lux - 7.322).abs() < 1e-9);
        assert!((s.xy[0] - 0.43838113).abs() < 1e-9);
        assert!((s.xy[1] - 0.39835248).abs() < 1e-9);
        assert_eq!(s.cct, 3066.0);
    }

    #[test]
    fn ioctl_number() {
        // HIDIOCGINPUT(18) from linux/hidraw.h
        assert_eq!(hidiocginput(18), 0xC012_480A);
    }
}
