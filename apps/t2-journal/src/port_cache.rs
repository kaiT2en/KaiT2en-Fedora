// SPDX-License-Identifier: MIT

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::discovery::{FIRST_DYNAMIC_PORT, LAST_DYNAMIC_PORT};

const MAX_PORTS: usize = 10;

pub fn path_for(snapshot: &Path) -> Result<PathBuf> {
    Ok(snapshot
        .parent()
        .context("snapshot path has no parent")?
        .join("remote-ports"))
}

pub fn read(path: &Path) -> Result<Vec<u16>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut ports = Vec::new();
    for line in BufReader::new(file).lines() {
        let Ok(port) = line?.trim().parse::<u16>() else {
            continue;
        };
        if (FIRST_DYNAMIC_PORT..=LAST_DYNAMIC_PORT).contains(&port) && !ports.contains(&port) {
            ports.push(port);
        }
        if ports.len() == MAX_PORTS {
            break;
        }
    }
    Ok(ports)
}

pub fn remember(path: &Path, previous: &[u16], successful: &[u16]) -> Result<()> {
    let mut ports = Vec::with_capacity(MAX_PORTS);
    for &port in successful.iter().chain(previous) {
        if (FIRST_DYNAMIC_PORT..=LAST_DYNAMIC_PORT).contains(&port) && !ports.contains(&port) {
            ports.push(port);
        }
        if ports.len() == MAX_PORTS {
            break;
        }
    }

    let partial = path.with_extension("partial");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&partial)?;
    for port in ports {
        writeln!(file, "{port}")?;
    }
    file.sync_all()?;
    fs::rename(partial, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_ten_unique_ports_in_mru_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote-ports");
        let old: Vec<u16> = (49152..49162).collect();
        remember(&path, &old, &[49157, 50000]).unwrap();
        assert_eq!(
            read(&path).unwrap(),
            vec![
                49157, 50000, 49152, 49153, 49154, 49155, 49156, 49158, 49159, 49160
            ]
        );
    }
}
