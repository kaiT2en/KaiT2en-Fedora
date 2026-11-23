// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! NSKeyedArchiver encode/decode, enough for the Instruments DTX payloads.
//!
//! DTX selector arguments and replies are Cocoa objects serialised with
//! NSKeyedArchiver (a binary plist with a `$objects` table). We encode the few
//! shapes the service accepts (dicts, arrays, scalars) and decode whatever it
//! sends back into a plain tree.

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Uid, Value as P};

#[derive(Clone, Debug)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Real(f64),
    Str(String),
    Data(Vec<u8>),
    Date(f64),
    Array(Vec<Obj>),
    Dict(Vec<(String, Obj)>),
    Object(String, Vec<(String, Obj)>),
}

impl Obj {
    pub fn get(&self, key: &str) -> Option<&Obj> {
        match self {
            Obj::Dict(entries) | Obj::Object(_, entries) => {
                entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
            }
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Obj::Int(v) => u64::try_from(*v).ok(),
            Obj::UInt(v) => Some(*v),
            Obj::Real(v) => Some(*v as u64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Obj::Int(v) => Some(*v as f64),
            Obj::UInt(v) => Some(*v as f64),
            Obj::Real(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Obj::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::{Map, Value as J, json};
        match self {
            Obj::Null => J::Null,
            Obj::Bool(v) => json!(v),
            Obj::Int(v) => json!(v),
            Obj::UInt(v) => json!(v),
            Obj::Real(v) => json!(v),
            Obj::Str(v) => json!(v),
            Obj::Date(v) => json!({ "$date": v }),
            Obj::Data(v) if v.len() <= 256 => json!({ "$data": hex(v) }),
            Obj::Data(v) => json!({ "$data_len": v.len() }),
            Obj::Array(v) => J::Array(v.iter().map(Obj::to_json).collect()),
            Obj::Dict(entries) => {
                J::Object(entries.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
            }
            Obj::Object(class, entries) => {
                let mut map = Map::new();
                map.insert("$class".into(), json!(class));
                for (k, v) in entries {
                    map.insert(k.clone(), v.to_json());
                }
                J::Object(map)
            }
        }
    }
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn encode(obj: &Obj) -> Vec<u8> {
    let mut objects = vec![P::String("$null".into())];
    let root = encode_into(obj, &mut objects);
    let mut top = Dictionary::new();
    top.insert("root".into(), P::Uid(root));
    let mut archive = Dictionary::new();
    archive.insert("$version".into(), P::Integer(100000.into()));
    archive.insert("$archiver".into(), P::String("NSKeyedArchiver".into()));
    archive.insert("$top".into(), P::Dictionary(top));
    archive.insert("$objects".into(), P::Array(objects));
    let mut out = Vec::new();
    plist::to_writer_binary(&mut out, &P::Dictionary(archive)).expect("in-memory plist write");
    out
}

fn push(objects: &mut Vec<P>, value: P) -> Uid {
    objects.push(value);
    Uid::new(objects.len() as u64 - 1)
}

fn class(objects: &mut Vec<P>, name: &str) -> Uid {
    let mut dict = Dictionary::new();
    dict.insert("$classname".into(), P::String(name.into()));
    dict.insert(
        "$classes".into(),
        P::Array(vec![P::String(name.into()), P::String("NSObject".into())]),
    );
    push(objects, P::Dictionary(dict))
}

fn encode_into(obj: &Obj, objects: &mut Vec<P>) -> Uid {
    match obj {
        Obj::Null => Uid::new(0),
        Obj::Bool(v) => push(objects, P::Boolean(*v)),
        Obj::Int(v) => push(objects, P::Integer((*v).into())),
        Obj::UInt(v) => push(objects, P::Integer((*v).into())),
        Obj::Real(v) => push(objects, P::Real(*v)),
        Obj::Str(v) => push(objects, P::String(v.clone())),
        Obj::Data(v) => push(objects, P::Data(v.clone())),
        Obj::Array(items) => {
            let at = push(objects, P::Boolean(false));
            let uids = items.iter().map(|v| P::Uid(encode_into(v, objects))).collect();
            let mut dict = Dictionary::new();
            dict.insert("NS.objects".into(), P::Array(uids));
            dict.insert("$class".into(), P::Uid(class(objects, "NSArray")));
            objects[at.get() as usize] = P::Dictionary(dict);
            at
        }
        Obj::Dict(entries) => {
            let at = push(objects, P::Boolean(false));
            let keys = entries
                .iter()
                .map(|(k, _)| P::Uid(push(objects, P::String(k.clone()))))
                .collect();
            let values = entries.iter().map(|(_, v)| P::Uid(encode_into(v, objects))).collect();
            let mut dict = Dictionary::new();
            dict.insert("NS.keys".into(), P::Array(keys));
            dict.insert("NS.objects".into(), P::Array(values));
            dict.insert("$class".into(), P::Uid(class(objects, "NSDictionary")));
            objects[at.get() as usize] = P::Dictionary(dict);
            at
        }
        Obj::Date(_) | Obj::Object(..) => unimplemented!("only decoded, never sent"),
    }
}

pub fn decode(data: &[u8]) -> Result<Obj> {
    let value = P::from_reader(Cursor::new(data)).context("payload is not a plist")?;
    let Some(archive) = value.as_dictionary() else {
        return Ok(plain(&value));
    };
    let (Some(objects), Some(root)) = (
        archive.get("$objects").and_then(P::as_array),
        archive
            .get("$top")
            .and_then(P::as_dictionary)
            .and_then(|top| top.get("root")),
    ) else {
        return Ok(plain(&value));
    };
    resolve(objects, root, 0)
}

fn plain(value: &P) -> Obj {
    match value {
        P::Boolean(v) => Obj::Bool(*v),
        P::Integer(v) => v
            .as_signed()
            .map(Obj::Int)
            .unwrap_or_else(|| Obj::UInt(v.as_unsigned().unwrap_or_default())),
        P::Real(v) => Obj::Real(*v),
        P::String(v) => Obj::Str(v.clone()),
        P::Data(v) => Obj::Data(v.clone()),
        P::Array(v) => Obj::Array(v.iter().map(plain).collect()),
        P::Dictionary(v) => Obj::Dict(v.iter().map(|(k, v)| (k.clone(), plain(v))).collect()),
        P::Uid(v) => Obj::UInt(v.get()),
        _ => Obj::Null,
    }
}

fn resolve(objects: &[P], value: &P, depth: usize) -> Result<Obj> {
    if depth > 64 {
        bail!("archive nesting too deep");
    }
    let P::Uid(uid) = value else {
        return Ok(plain(value));
    };
    let object = objects.get(uid.get() as usize).context("archive UID out of range")?;
    if object.as_string() == Some("$null") {
        return Ok(Obj::Null);
    }
    let Some(dict) = object.as_dictionary() else {
        return Ok(plain(object));
    };
    let Some(class_uid) = dict.get("$class") else {
        return Ok(plain(object));
    };
    let class_name = match class_uid {
        P::Uid(c) => objects
            .get(c.get() as usize)
            .and_then(P::as_dictionary)
            .and_then(|c| c.get("$classname"))
            .and_then(P::as_string)
            .unwrap_or("?")
            .to_owned(),
        _ => "?".to_owned(),
    };
    let list = |key: &str| -> Result<Option<Vec<Obj>>> {
        dict.get(key)
            .and_then(P::as_array)
            .map(|items| items.iter().map(|v| resolve(objects, v, depth + 1)).collect())
            .transpose()
    };
    if let (Some(keys), Some(values)) = (list("NS.keys")?, list("NS.objects")?) {
        let entries = keys
            .into_iter()
            .zip(values)
            .map(|(k, v)| (k.as_str().map(str::to_owned).unwrap_or_else(|| format!("{k:?}")), v))
            .collect();
        return Ok(Obj::Dict(entries));
    }
    if let Some(values) = list("NS.objects")? {
        return Ok(Obj::Array(values));
    }
    if let Some(s) = dict.get("NS.string") {
        return resolve(objects, s, depth + 1);
    }
    if let Some(t) = dict.get("NS.time").and_then(P::as_real) {
        return Ok(Obj::Date(t));
    }
    if let Some(d) = dict.get("NS.data").or_else(|| dict.get("NS.bytes")) {
        return resolve(objects, d, depth + 1);
    }
    if class_name == "NSNull" {
        return Ok(Obj::Null);
    }
    let fields = dict
        .iter()
        .filter(|(k, _)| k.as_str() != "$class")
        .map(|(k, v)| Ok((k.clone(), resolve(objects, v, depth + 1)?)))
        .collect::<Result<_>>()?;
    Ok(Obj::Object(class_name, fields))
}
