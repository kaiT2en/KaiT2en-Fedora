// SPDX-License-Identifier: MIT

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use flate2::read::GzDecoder;

pub struct Extracted {
    pub root: PathBuf,
    pub logarchive: PathBuf,
}

pub fn extract(archive: &Path, destination: &Path) -> Result<Extracted> {
    std::fs::create_dir_all(destination)?;
    let file = File::open(archive).with_context(|| format!("open {}", archive.display()))?;
    let mut tar = tar::Archive::new(GzDecoder::new(file));
    for item in tar.entries().context("read sysdiagnose archive")? {
        let mut item = item.context("read sysdiagnose member")?;
        let kind = item.header().entry_type();
        ensure!(
            !kind.is_symlink() && !kind.is_hard_link(),
            "refusing link in sysdiagnose: {}",
            item.path()?.display()
        );
        ensure!(
            item.unpack_in(destination)
                .context("extract sysdiagnose member")?,
            "sysdiagnose member escapes extraction directory"
        );
    }
    let logarchive = walk(destination)?
        .into_iter()
        .find(|path| path.extension().is_some_and(|ext| ext == "logarchive"));
    let logarchive =
        logarchive.ok_or_else(|| anyhow::anyhow!("system_logs.logarchive not found"))?;
    Ok(Extracted {
        root: destination.to_path_buf(),
        logarchive,
    })
}

pub fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).with_context(|| format!("read {}", path.display()))? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                bail!(
                    "refusing symlink in sysdiagnose: {}",
                    entry.path().display()
                );
            }
            if kind.is_dir() {
                pending.push(entry.path());
            }
            paths.push(entry.path());
        }
    }
    Ok(paths)
}
