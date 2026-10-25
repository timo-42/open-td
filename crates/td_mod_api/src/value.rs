//! A dynamically typed value, used for map `[params]`, raw data tables and
//! request payloads. It maps one to one onto MessagePack and TOML.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

static NIL: Value = Value::Nil;

impl Value {
    pub fn map() -> Value {
        Value::Map(BTreeMap::new())
    }

    /// Looks up a key in a map. Returns `Nil` for anything else, so lookups
    /// chain: `params.get("lives").get("start")`.
    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Map(m) => m.get(key).unwrap_or(&NIL),
            _ => &NIL,
        }
    }

    /// Looks up a dotted path, `"lives.start"`.
    pub fn path(&self, path: &str) -> &Value {
        path.split('.').fold(self, |v, k| v.get(k))
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> &[Value] {
        match self {
            Value::Array(a) => a,
            _ => &[],
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn i64_or(&self, default: i64) -> i64 {
        self.as_i64().unwrap_or(default)
    }

    pub fn f64_or(&self, default: f64) -> f64 {
        self.as_f64().unwrap_or(default)
    }

    pub fn str_or<'a>(&'a self, default: &'a str) -> &'a str {
        self.as_str().unwrap_or(default)
    }

    /// Reads a `{ name = amount }` table, as used for costs and bounties.
    pub fn as_amounts(&self) -> BTreeMap<String, i64> {
        let mut out = BTreeMap::new();
        if let Some(m) = self.as_map() {
            for (k, v) in m {
                if let Some(n) = v.as_i64() {
                    out.insert(k.clone(), n);
                }
            }
        }
        out
    }

    /// Inserts into a map value, turning `Nil` into a map first.
    pub fn insert(&mut self, key: impl Into<String>, value: Value) {
        if self.is_nil() {
            *self = Value::map();
        }
        if let Value::Map(m) = self {
            m.insert(key.into(), value);
        }
    }

    /// Deep merge: maps merge key by key, `Nil` keeps what is there,
    /// anything else in `other` replaces.
    pub fn merge(&mut self, other: &Value) {
        match (self, other) {
            (_, Value::Nil) => {}
            (Value::Map(a), Value::Map(b)) => {
                for (k, v) in b {
                    match a.get_mut(k) {
                        Some(existing) => existing.merge(v),
                        None => {
                            a.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            (this, other) => *this = other.clone(),
        }
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(v.into())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(v)
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Nil => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Int(i) => s.serialize_i64(*i),
            Value::Float(f) => s.serialize_f64(*f),
            Value::Str(v) => s.serialize_str(v),
            Value::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            Value::Map(m) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any value")
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Nil)
    }
    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Nil)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        Deserialize::deserialize(d)
    }
    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Int(v))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        i64::try_from(v)
            .map(Value::Int)
            .map_err(|_| E::custom("integer out of range"))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Float(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::Str(v.into()))
    }
    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::Str(v))
    }
    fn visit_bytes<E>(self, v: &[u8]) -> Result<Value, E> {
        Ok(Value::Array(
            v.iter().map(|b| Value::Int(*b as i64)).collect(),
        ))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element()? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = BTreeMap::new();
        while let Some((k, v)) = map.next_entry::<String, Value>()? {
            out.insert(k, v);
        }
        Ok(Value::Map(out))
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}
