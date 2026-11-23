// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//
// This decoder is a derivative work: the opcode VM and message rendering are
// ported from pymobiledevice3's `ActivityTraceTap`
// (pymobiledevice3/services/dvt/instruments/activity_trace_tap.py), which is
// licensed GPL-3.0. Our work builds on and modifies theirs; it is redistributed
// here under GPL-3.0-or-later, as that license permits.
// Upstream: https://github.com/doronz88/pymobiledevice3
//
//! Decoder for the activitytracetap record stream.
//!
//! The stream is a stack-based opcode VM read as 16-bit little-endian words
//! (`opcode = word >> 8`). State (tables and stack) persists across DTX frames,
//! and an opcode split across frames is carried into the next.

const CMD_DEFINE_TABLE: u16 = 1;
const CMD_END_ROW: u16 = 2;
const CMD_CONVERT_MACH_CONTINUOUS: u16 = 5;
const CMD_TABLE_RESET: u16 = 0x64;
const CMD_COPY: u16 = 0x65;
const CMD_SENTINEL: u16 = 0x68;
const CMD_STRUCT: u16 = 0x69;
const CMD_PLACEHOLDER_COUNT: u16 = 0x6a;
const CMD_DEBUG: u16 = 0x6b;

/// A value on the VM stack: raw pushed bytes, a sentinel, or a grouped tuple.
#[derive(Clone)]
pub enum Item {
    Bytes(Vec<u8>),
    Null,
    List(Vec<Item>),
}

impl Item {
    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Item::Bytes(b) => Some(b),
            _ => None,
        }
    }
    fn list(&self) -> Option<&[Item]> {
        match self {
            Item::List(items) => Some(items),
            _ => None,
        }
    }
    /// Little-endian integer of the bytes (columns store ints least-significant first).
    fn as_u64(&self) -> u64 {
        let bytes = match self {
            Item::Bytes(b) => b.as_slice(),
            // process/thread arrive wrapped in a one-element tuple
            Item::List(items) => items.first().and_then(Item::bytes).unwrap_or(&[]),
            Item::Null => &[],
        };
        let mut value = 0u64;
        for (i, byte) in bytes.iter().take(8).enumerate() {
            value |= (*byte as u64) << (8 * i);
        }
        value
    }
}

struct Table {
    name: String,
    columns: Vec<String>,
}

/// One decoded log row: the table name plus its columns as raw items.
pub struct Row {
    pub table: String,
    pub columns: Vec<String>,
    pub items: Vec<Item>,
}

impl Row {
    fn get(&self, column: &str) -> Option<&Item> {
        self.columns
            .iter()
            .position(|c| c == column)
            .and_then(|i| self.items.get(i))
    }
    /// A string column, truncated at its first NUL.
    pub fn string(&self, column: &str) -> Option<String> {
        self.get(column).and_then(Item::bytes).map(decode_str)
    }
    /// An integer column (process, thread).
    pub fn integer(&self, column: &str) -> Option<u64> {
        self.get(column).map(Item::as_u64)
    }
    /// The rendered log line from the `message` column and `format-string`.
    pub fn message(&self) -> Option<String> {
        let items = self.get("message")?.list()?;
        let format = self.get("format-string").and_then(Item::bytes).map(decode_str);
        Some(render_message(items, format.as_deref()))
    }
}

/// Decode a byte string up to its first NUL.
pub fn decode_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The stateful VM. Feed it raw record chunks; it yields completed rows.
pub struct Decoder {
    stack: Vec<Item>,
    tables: Vec<Table>,
    carry: Vec<u8>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self { stack: Vec::new(), tables: Vec::new(), carry: Vec::new() }
    }
}

impl Decoder {
    /// Feed one record chunk, returning every row completed within it.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Row> {
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(chunk);
        let mut pos = 0usize;
        let mut rows = Vec::new();
        loop {
            let start = pos;
            match self.step(&data, &mut pos) {
                Step::Row(row) => rows.push(row),
                Step::Ok => {}
                Step::NeedMore => {
                    // Frame ended mid-opcode; carry the tail into the next feed.
                    self.carry = data[start..].to_vec();
                    break;
                }
            }
        }
        rows
    }

    fn step(&mut self, data: &[u8], pos: &mut usize) -> Step {
        let Some(word) = read_word(data, pos) else {
            return Step::NeedMore;
        };
        let opcode = word >> 8;
        let operand = (word & 0xff) as usize;
        match opcode {
            CMD_TABLE_RESET => self.stack.clear(),
            CMD_SENTINEL => self.stack.push(Item::Null),
            CMD_CONVERT_MACH_CONTINUOUS => {}
            CMD_DEBUG => {
                self.stack.pop();
            }
            CMD_PLACEHOLDER_COUNT => {
                let keep = self.stack.len().saturating_sub(operand);
                self.stack.truncate(keep);
            }
            CMD_STRUCT => {
                let at = self.stack.len().saturating_sub(operand);
                let grouped = self.stack.split_off(at);
                self.stack.push(Item::List(grouped));
            }
            CMD_COPY => {
                if operand != 0xff {
                    if let Some(index) = self.stack.len().checked_sub(operand + 1) {
                        self.stack.push(self.stack[index].clone());
                    }
                } else if let Some(item) = self.stack.pop() {
                    let reference = (item.as_u64() as usize).wrapping_sub(1);
                    if let Some(item) = self.stack.get(reference) {
                        self.stack.push(item.clone());
                    }
                }
            }
            CMD_DEFINE_TABLE => {
                if self.stack.len() >= 4 {
                    let raw = self.stack.split_off(self.stack.len() - 4);
                    let name = raw[2].bytes().map(decode_str).unwrap_or_default();
                    let columns = raw[3]
                        .list()
                        .unwrap_or(&[])
                        .iter()
                        .map(|c| c.bytes().map(decode_str).unwrap_or_default())
                        .collect();
                    self.tables.push(Table { name, columns });
                }
            }
            CMD_END_ROW => {
                let Some(table) = self.tables.get(operand) else {
                    return Step::Ok;
                };
                let width = table.columns.len();
                if self.stack.len() < width {
                    return Step::Ok;
                }
                let items = self.stack.split_off(self.stack.len() - width);
                return Step::Row(Row {
                    table: table.name.clone(),
                    columns: table.columns.clone(),
                    items,
                });
            }
            _ => return self.push(word, data, pos),
        }
        Step::Ok
    }

    /// A pushed value: 14 payload bits per word, high bits `0b10` continue and
    /// `0b11` end, then padded to whole bytes as big-endian. Values can be long
    /// (whole strings), so the bits are packed into a byte buffer, not an int.
    fn push(&mut self, first: u16, data: &[u8], pos: &mut usize) -> Step {
        let mut writer = BitWriter::default();
        let mut word = first;
        while word >> 14 != 0b11 {
            writer.push_bits((word & 0x3fff) as u32, 14);
            let Some(next) = read_word(data, pos) else {
                return Step::NeedMore;
            };
            word = next;
        }
        writer.push_bits((word & 0x3fff) as u32, 14);
        self.stack.push(Item::Bytes(writer.finish()));
        Step::Ok
    }
}

enum Step {
    Ok,
    Row(Row),
    NeedMore,
}

/// Packs bits most-significant-first into bytes, padding the tail with zeros.
#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    current: u8,
    filled: u32,
}

impl BitWriter {
    fn push_bits(&mut self, value: u32, count: u32) {
        for i in (0..count).rev() {
            self.current = (self.current << 1) | ((value >> i) & 1) as u8;
            self.filled += 1;
            if self.filled == 8 {
                self.bytes.push(self.current);
                self.current = 0;
                self.filled = 0;
            }
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.filled > 0 {
            self.current <<= 8 - self.filled;
            self.bytes.push(self.current);
        }
        self.bytes
    }
}

fn read_word(data: &[u8], pos: &mut usize) -> Option<u16> {
    let word = data.get(*pos..*pos + 2)?;
    *pos += 2;
    Some(u16::from_le_bytes([word[0], word[1]]))
}

/// The 8-byte chunks a uuid or data item is pushed in, each padded back to 8.
fn join_chunks(items: &[Item]) -> Vec<[u8; 8]> {
    items
        .iter()
        .filter_map(Item::bytes)
        .map(|b| {
            let mut out = [0u8; 8];
            let take = b.len().min(8);
            out[..take].copy_from_slice(&b[..take]);
            out
        })
        .collect()
}

/// Join a row's `message` items into the logged line. Narrative text is the
/// format string's own; each other item is one argument, paired in order with
/// the format string's printf conversions so numbers print per their
/// conversion (a `%f` over a decimal reinterprets its bits as a double).
/// Width/precision are applied; the niche typed enums (errno, tcp, mdns) are not
/// (see the channel doc).
fn render_message(items: &[Item], format: Option<&str>) -> String {
    let specifiers = parse_specifiers(format.unwrap_or(""));
    // os_log keeps one item per conversion, in order; if the counts disagree we
    // cannot pair them, so numbers fall back to their plain form.
    let arg_items = items
        .iter()
        .filter_map(Item::list)
        .filter(|p| p.len() >= 2 && p[0].bytes().map(decode_str).as_deref() != Some("narrative-text"))
        .count();
    let mut specs = if arg_items == specifiers.len() {
        specifiers.into_iter()
    } else {
        Vec::new().into_iter()
    };

    let mut out = String::new();
    for item in items {
        let Some(pair) = item.list() else { continue };
        if pair.len() < 2 {
            continue;
        }
        let type_ = pair[0].bytes().map(decode_str).unwrap_or_default();
        let data = &pair[1];
        if type_ == "narrative-text" {
            if let Some(bytes) = data.bytes() {
                out.push_str(&decode_str(bytes).replace("%%", "%"));
            }
            continue;
        }
        let spec = specs.next();
        match type_.as_str() {
            "string" => match data.bytes() {
                Some(bytes) => {
                    let string = decode_str(bytes);
                    if spec.as_ref().map(|s| s.conversion) == Some('@') && string.is_empty() {
                        out.push_str("(null)");
                    } else {
                        out.push_str(&string);
                    }
                }
                None => out.push_str("<None>"),
            },
            "private" => out.push_str("<private>"),
            "uuid" => {
                if let Some(halves) = data.list() {
                    out.push_str(&format_uuid(&join_chunks(halves)));
                }
            }
            "data" => {
                if let Some(chunks) = data.list() {
                    let parts = join_chunks(chunks);
                    if let Some((length, rest)) = parts.split_first() {
                        let len = u64::from_le_bytes(*length) as usize;
                        let raw: Vec<u8> = rest.iter().flatten().copied().take(len).collect();
                        out.push_str(&raw.iter().map(|b| format!("{b:02x}")).collect::<String>());
                    }
                }
            }
            t => {
                // uint64, decimal, address, errno-value and their hex variants
                let value = data.bytes().map(le_u64).unwrap_or_default();
                if let Some(spec) = spec.as_ref().filter(|s| item_fits(t, s.conversion)) {
                    out.push_str(&format_number(value, spec));
                } else if t.contains("hex") || t == "address" {
                    out.push_str(&format!("{value:x}"));
                } else {
                    out.push_str(&value.to_string());
                }
            }
        }
    }
    out.trim_end().to_owned()
}

/// A parsed printf conversion from the format string.
struct Spec {
    width: Option<usize>,
    precision: Option<usize>,
    length: Option<String>,
    conversion: char,
    plus: bool,
    space: bool,
    zero: bool,
    left: bool,
}

/// Whether an item of `type_` can be the argument of a `conversion`.
fn item_fits(type_: &str, conversion: char) -> bool {
    if type_.contains("decimal") {
        "fFeEgG".contains(conversion)
    } else if type_.contains("hex") {
        "xXp".contains(conversion)
    } else {
        !"fFeEgG".contains(conversion)
    }
}

/// Scan a format string for its conversions, skipping "%%".
fn parse_specifiers(format: &str) -> Vec<Spec> {
    let chars: Vec<char> = format.chars().collect();
    let mut specs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            i += 1;
            continue;
        }
        i += 1;
        if i >= chars.len() {
            break;
        }
        if chars[i] == '%' {
            i += 1;
            continue;
        }
        // os_log's "{options}" (privacy and value type) between % and the flags
        if chars[i] == '{' {
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            i += 1;
        }
        let (mut plus, mut space, mut zero, mut left) = (false, false, false, false);
        while i < chars.len() {
            match chars[i] {
                '+' => plus = true,
                ' ' => space = true,
                '0' => zero = true,
                '-' => left = true,
                '#' | '\'' => {}
                _ => break,
            }
            i += 1;
        }
        let width = scan_number(&chars, &mut i);
        let mut precision = None;
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            precision = Some(scan_number(&chars, &mut i).unwrap_or(0));
        }
        let mut length = String::new();
        while i < chars.len() && "hlqjztL".contains(chars[i]) {
            length.push(chars[i]);
            i += 1;
        }
        if i < chars.len() {
            let conversion = chars[i];
            i += 1;
            specs.push(Spec {
                width,
                precision,
                length: (!length.is_empty()).then_some(length),
                conversion,
                plus,
                space,
                zero,
                left,
            });
        }
    }
    specs
}

fn scan_number(chars: &[char], i: &mut usize) -> Option<usize> {
    if *i < chars.len() && chars[*i] == '*' {
        *i += 1;
        return None;
    }
    let start = *i;
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        *i += 1;
    }
    (start != *i).then(|| chars[start..*i].iter().collect::<String>().parse().unwrap_or(0))
}

/// Print `value` (the raw 64-bit argument) the way printf prints it for `spec`.
fn format_number(value: u64, spec: &Spec) -> String {
    let sign = |negative: bool| {
        if negative {
            "-"
        } else if spec.plus {
            "+"
        } else if spec.space {
            " "
        } else {
            ""
        }
    };
    let body = match spec.conversion {
        'f' | 'F' => {
            let v = f64::from_bits(value);
            format!("{}{:.*}", sign(v.is_sign_negative()), spec.precision.unwrap_or(6), v.abs())
        }
        'e' | 'E' => {
            let v = f64::from_bits(value);
            let s = format!("{:.*e}", spec.precision.unwrap_or(6), v.abs());
            let s = if spec.conversion == 'E' { s.to_uppercase() } else { s };
            format!("{}{s}", sign(v.is_sign_negative()))
        }
        'g' | 'G' => {
            let v = f64::from_bits(value);
            format!("{}{}", sign(v.is_sign_negative()), v.abs())
        }
        'd' | 'i' => {
            let bits = integer_bits(spec.length.as_deref());
            let mut v = value & mask(bits);
            let negative = v >> (bits - 1) & 1 == 1;
            if negative {
                v = (!v + 1) & mask(bits);
            }
            format!("{}{v}", sign(negative))
        }
        'u' => (value & mask(integer_bits(spec.length.as_deref()))).to_string(),
        'x' => format!("{:x}", value & mask(integer_bits(spec.length.as_deref()))),
        'X' => format!("{:X}", value & mask(integer_bits(spec.length.as_deref()))),
        'o' => format!("{:o}", value & mask(integer_bits(spec.length.as_deref()))),
        'p' => format!("0x{value:x}"),
        'c' => char::from_u32((value & 0xff) as u32).map(String::from).unwrap_or_default(),
        _ => return value.to_string(),
    };
    pad(body, spec)
}

fn integer_bits(length: Option<&str>) -> u32 {
    match length {
        Some("hh") => 8,
        Some("h") => 16,
        None => 32,
        _ => 64,
    }
}

fn mask(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 }
}

fn pad(body: String, spec: &Spec) -> String {
    let Some(width) = spec.width else { return body };
    if body.len() >= width {
        return body;
    }
    let fill = width - body.len();
    if spec.left {
        format!("{body}{}", " ".repeat(fill))
    } else if spec.zero {
        // keep a leading sign in front of the zeros
        let (sign, digits) = body.split_at(body.starts_with(['-', '+', ' ']) as usize);
        format!("{sign}{}{digits}", "0".repeat(fill))
    } else {
        format!("{}{body}", " ".repeat(fill))
    }
}

fn le_u64(bytes: &[u8]) -> u64 {
    let mut value = 0u64;
    for (i, byte) in bytes.iter().take(8).enumerate() {
        value |= (*byte as u64) << (8 * i);
    }
    value
}

fn format_uuid(halves: &[[u8; 8]]) -> String {
    let mut bytes = [0u8; 16];
    for (half, slot) in halves.iter().take(2).zip(bytes.chunks_mut(8)) {
        for (i, b) in half.iter().rev().enumerate() {
            slot[i] = *b;
        }
    }
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}
