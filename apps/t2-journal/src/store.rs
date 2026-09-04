// SPDX-License-Identifier: MIT

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use anyhow::{Context, Result};

use crate::record::Record;

fn partial_path(path: &Path) -> Result<std::path::PathBuf> {
    let parent = path.parent().context("snapshot path has no parent")?;
    let mut partial_name = OsString::from(path.file_name().context("snapshot path has no name")?);
    partial_name.push(".partial");
    Ok(parent.join(partial_name))
}

fn open_partial(path: &Path) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("open {}", path.display()))
}

pub fn prepare(path: &Path) -> Result<()> {
    let parent = path.parent().context("snapshot path has no parent")?;
    fs::create_dir_all(parent)?;
    open_partial(&partial_path(path)?)?;
    Ok(())
}

pub fn write_atomic(path: &Path, records: &[Record]) -> Result<()> {
    let parent = path.parent().context("snapshot path has no parent")?;
    fs::create_dir_all(parent)?;
    let partial = partial_path(path)?;
    let temporary = open_partial(&partial)?;
    let mut writer = BufWriter::new(&temporary);
    for record in records {
        serde_json::to_writer(&mut writer, record)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    drop(writer);
    temporary.sync_all()?;
    temporary.set_permissions(fs::Permissions::from_mode(0o600))?;
    fs::rename(&partial, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn read(path: &Path) -> Result<Vec<Record>> {
    let file = File::open(path).with_context(|| {
        format!(
            "no BridgeOS snapshot at {}; run 't2journal refresh'",
            path.display()
        )
    })?;
    BufReader::new(file)
        .lines()
        .map(|line| Ok(serde_json::from_str(&line?)?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn record(message: &str) -> Record {
        Record {
            timestamp_ns: 1,
            source: "T2".into(),
            process: String::new(),
            subsystem: String::new(),
            category: String::new(),
            message: message.into(),
        }
    }

    #[test]
    fn refresh_replaces_instead_of_appending() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bridgeos.jsonl");
        write_atomic(&path, &[record("old one"), record("old two")]).unwrap();
        write_atomic(&path, &[record("new")]).unwrap();
        let records = read(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!directory.path().join("bridgeos.jsonl.partial").exists());
    }
}
