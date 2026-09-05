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
        format!("[{timestamp}] [{tag}] {}", single_line(&self.message))
    }
}

fn single_line(message: &str) -> String {
    let mut escaped = String::with_capacity(message.len());
    for character in message.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\u{2028}' => escaped.push_str("\\u{2028}"),
            '\u{2029}' => escaped.push_str("\\u{2029}"),
            character if character.is_control() && character != '\t' => {
                escaped.extend(character.escape_default());
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_output_keeps_each_record_on_one_lossless_line() {
        let record = Record {
            timestamp_ns: 0,
            source: "T2".into(),
            process: "sysdiagnose".into(),
            subsystem: String::new(),
            category: String::new(),
            message: "first\nsecond\\literal\rthird\u{2028}fourth".into(),
        };

        let text = record.text();
        assert!(!text.contains('\n'));
        assert!(!text.contains('\r'));
        assert!(text.ends_with("first\\nsecond\\\\literal\\rthird\\u{2028}fourth"));
    }

    #[test]
    fn text_output_preserves_unicode_and_tabs() {
        let message = "Gerät\tbereit";
        assert_eq!(single_line(message), message);
    }
}
