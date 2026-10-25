//! TOML ↔ [`Value`].

use td_mod_api::Value;

pub fn from_toml(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::Str(s.clone()),
        toml::Value::Integer(i) => Value::Int(*i),
        toml::Value::Float(f) => Value::Float(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::Str(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.iter().map(from_toml).collect()),
        toml::Value::Table(t) => {
            Value::Map(t.iter().map(|(k, v)| (k.clone(), from_toml(v))).collect())
        }
    }
}

pub fn table_to_value(t: &toml::Table) -> Value {
    Value::Map(t.iter().map(|(k, v)| (k.clone(), from_toml(v))).collect())
}

pub fn to_toml(v: &Value) -> Option<toml::Value> {
    Some(match v {
        Value::Nil => return None,
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Int(i) => toml::Value::Integer(*i),
        Value::Float(f) => toml::Value::Float(*f),
        Value::Str(s) => toml::Value::String(s.clone()),
        Value::Array(a) => toml::Value::Array(a.iter().filter_map(to_toml).collect()),
        Value::Map(m) => toml::Value::Table(
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), to_toml(v)?)))
                .collect(),
        ),
    })
}
