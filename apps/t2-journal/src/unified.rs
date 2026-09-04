// SPDX-License-Identifier: MIT

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use macos_unifiedlogs::cache::MemoryStringCache;
use macos_unifiedlogs::filesystem::LogarchiveProvider;
use macos_unifiedlogs::iterator::UnifiedLogIterator;
use macos_unifiedlogs::parser::{build_log, collect_timesync};
use macos_unifiedlogs::traits::{FileProvider, SourceFile};
use macos_unifiedlogs::unified_log::UnifiedLogData;

use crate::record::Record;

pub fn parse(path: &Path, mut progress: impl FnMut(u64, u64)) -> Result<Vec<Record>> {
    let provider = LogarchiveProvider::new(path);
    let cache = MemoryStringCache::default();
    let timesync = collect_timesync(&provider).context("parse bridgeOS timesync")?;
    let mut oversize = UnifiedLogData::default();
    let mut missing = Vec::new();
    let mut records = Vec::new();

    let total = provider.tracev3_files().count() as u64;
    let mut current = 0;
    for mut source in provider.tracev3_files() {
        if Path::new(source.source_path())
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("._"))
        {
            current += 1;
            progress(current, total);
            continue;
        }
        let mut data = Vec::new();
        source.reader().read_to_end(&mut data)?;
        let iterator = UnifiedLogIterator {
            data,
            header: Vec::new(),
            evidence: source.source_path().to_owned(),
        };
        for mut chunk in iterator {
            chunk.oversize.append(&mut oversize.oversize);
            let (logs, unresolved) = build_log(&chunk, &provider, &cache, &timesync, true);
            oversize.oversize = chunk.oversize;
            if !unresolved.catalog_data.is_empty()
                || !unresolved.header.is_empty()
                || !unresolved.oversize.is_empty()
            {
                missing.push(unresolved);
            }
            records.extend(logs.into_iter().map(|log| Record {
                timestamp_ns: log.time as i64,
                source: "T2".into(),
                process: log.process,
                subsystem: log.subsystem,
                category: log.category,
                message: log.message,
            }));
        }
        current += 1;
        progress(current, total);
    }

    for mut unresolved in missing {
        unresolved.oversize.clone_from(&oversize.oversize);
        let (logs, _) = build_log(&unresolved, &provider, &cache, &timesync, false);
        records.extend(logs.into_iter().map(|log| Record {
            timestamp_ns: log.time as i64,
            source: "T2".into(),
            process: log.process,
            subsystem: log.subsystem,
            category: log.category,
            message: log.message,
        }));
    }

    records.sort_by_key(|record| record.timestamp_ns);
    Ok(records)
}
