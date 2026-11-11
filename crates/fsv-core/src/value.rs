//! Python truthiness and small accessors over `serde_json::Value`.

use serde_json::Value;

/// Python `bool(value)`.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `bool(d.get(key))` for an optional value.
pub fn truthy_opt(v: Option<&Value>) -> bool {
    v.is_some_and(truthy)
}

/// `d.get(key)` when `d` is an object (`None` otherwise or when missing).
pub fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(key))
}

/// First truthy value among `keys` (`d.get(a) or d.get(b) or ...`).
pub fn first_truthy<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().filter_map(|k| get(v, k)).find(|x| truthy(x))
}

/// Python `isinstance(v, int) and not isinstance(v, bool)` for a JSON number.
pub fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) if !n.is_f64() => n.as_i64(),
        _ => None,
    }
}
