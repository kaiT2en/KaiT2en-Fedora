// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Record {
    pub timestamp_ns: i64,
    pub source: String,
    pub process: String,
    pub subsystem: String,
    pub category: String,
    pub message: String,
}

impl Record {
    pub fn text(&self) -> String {
        let timestamp = chrono::DateTime::from_timestamp_nanos(self.timestamp_ns)
            .format("%Y-%m-%dT%H:%M:%S%.3fZ");
        let mut tag = self.source.clone();
        if !self.process.is_empty() {
            tag.push('/');
            tag.push_str(&self.process);
        }
        if !self.subsystem.is_empty() {
            tag.push(':');
            tag.push_str(&self.subsystem);
        }
        format!(
            "[{timestamp}] [{tag}] {}",
            self.message.replace('\n', "\n    ")
        )
    }
}
