// SPDX-License-Identifier: MIT

use crate::record::Record;

const SECOND_NS: i64 = 1_000_000_000;

#[derive(Clone, Copy, Debug)]
pub struct Acquisition {
    scan_start_ns: i64,
    collection_start_ns: i64,
    collection_end_ns: i64,
}

impl Acquisition {
    pub fn from_records(records: &[Record]) -> Option<Self> {
        let mut sysdiagnose = records
            .iter()
            .filter(|record| record.source == "T2" && record.process == "sysdiagnose")
            .map(|record| record.timestamp_ns);
        let first = sysdiagnose.next()?;
        let (collection_start_ns, collection_end_ns) = sysdiagnose
            .fold((first, first), |(start, end), timestamp| {
                (start.min(timestamp), end.max(timestamp))
            });
        let scan_limit = collection_start_ns.saturating_sub(30 * SECOND_NS);
        let scan_start_ns = records
            .iter()
            .filter(|record| {
                record.source == "T2"
                    && record.timestamp_ns >= scan_limit
                    && record.timestamp_ns <= collection_start_ns
                    && record.message.contains("tcp connect incoming:")
            })
            .map(|record| record.timestamp_ns)
            .min()
            .unwrap_or(collection_start_ns);

        Some(Self {
            scan_start_ns,
            collection_start_ns,
            collection_end_ns: collection_end_ns.saturating_add(SECOND_NS),
        })
    }

    pub fn contains(self, record: &Record) -> bool {
        if record.source != "T2"
            || record.timestamp_ns < self.scan_start_ns
            || record.timestamp_ns > self.collection_end_ns
        {
            return false;
        }

        if record.timestamp_ns >= self.collection_start_ns && is_audio_state_dump(record) {
            return true;
        }

        matches!(
            record.subsystem.as_str(),
            "com.apple.network" | "com.apple.xpc.remote" | "com.apple.RemoteServiceDiscovery"
        ) || (record.subsystem == "com.apple.xnu.net.tcp"
            && record.message.contains("tcp connect incoming:"))
            || (record.subsystem == "com.apple.xnu"
                && record.message.contains("flow_entry_alloc")
                && record.message.contains("remoted"))
    }
}

fn is_audio_state_dump(record: &Record) -> bool {
    if !record.process.ends_with("/bridgeaudiod") {
        return false;
    }
    let message = record.message.trim_start();
    message.starts_with("title: AudioServerDriver")
        || message.starts_with("title: CACentralStateDump")
        || message.starts_with("HALS_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(timestamp_ns: i64, process: &str, subsystem: &str, message: &str) -> Record {
        Record {
            timestamp_ns,
            source: "T2".into(),
            process: process.into(),
            subsystem: subsystem.into(),
            category: String::new(),
            message: message.into(),
        }
    }

    #[test]
    fn hides_only_acquisition_transport_and_explicit_state_dump() {
        let records = vec![
            record(
                80 * SECOND_NS,
                "/kernel",
                "com.apple.xnu.net.tcp",
                "tcp connect incoming: RemoteXPC scan",
            ),
            record(100 * SECOND_NS, "sysdiagnose", "", "collection starts"),
            record(110 * SECOND_NS, "sysdiagnose", "", "collection ends"),
        ];
        let acquisition = Acquisition::from_records(&records).unwrap();

        assert!(acquisition.contains(&record(
            90 * SECOND_NS,
            "/usr/libexec/remoted",
            "com.apple.xpc.remote",
            "Header read returned without data"
        )));
        assert!(acquisition.contains(&record(
            105 * SECOND_NS,
            "/usr/sbin/bridgeaudiod",
            "",
            "HALS_ObjectMap.cpp:554 Object ID: 1"
        )));
        assert!(!acquisition.contains(&record(
            105 * SECOND_NS,
            "/kernel",
            "com.apple.usb",
            "AppleUSBVHCIFirmwareBCE failed to change power state"
        )));
        assert!(!acquisition.contains(&record(
            105 * SECOND_NS,
            "/usr/sbin/bridgeaudiod",
            "com.apple.coreaudio",
            "real runtime error"
        )));
        assert!(!acquisition.contains(&records[1]));
    }

    #[test]
    fn does_not_filter_without_sysdiagnose_boundaries() {
        let records = [record(
            80 * SECOND_NS,
            "/usr/libexec/remoted",
            "com.apple.xpc.remote",
            "Connecting",
        )];
        assert!(Acquisition::from_records(&records).is_none());
    }
}
