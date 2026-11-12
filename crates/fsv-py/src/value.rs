//! Python truthiness for JSON values.
//!
//! Freshservice code does `x.get(k) or {}` about 170 times. `0`, `""`,
//! `[]`, `{}`, `false` and `null` all count as missing. A Rust
//! `.get().and_then(Value::as_object)` keeps those and then reads the wrong
//! shape.

use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::py_str;

static EMPTY_OBJ: LazyLock<Map<String, Value>> = LazyLock::new(Map::new);

/// CPython `bool(value)` for a JSON value. `0.0` and `-0.0` are false.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i != 0
            } else if let Some(u) = n.as_u64() {
                u != 0
            } else {
                n.as_f64().is_some_and(|f| f != 0.0)
            }
        }
        Value::String(s) => !s.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

pub trait ValueExt {
    /// `x.get(k)` when that value is truthy.
    fn get_truthy(&self, key: &str) -> Option<&Value>;
    /// `x.get(k) or {}` when the caller treats the result as a dict.
    /// A truthy non-object is not a dict, so this returns the empty map
    /// rather than panicking the way Python would on `.get`.
    fn obj_or_empty(&self, key: &str) -> &Map<String, Value>;
    /// `x.get(k) or []`.
    fn list_or_empty(&self, key: &str) -> &[Value];
    /// `str(x.get(k))`, including `"None"` when the key is absent.
    fn py_get_str(&self, key: &str) -> String;
}

impl ValueExt for Value {
    fn get_truthy(&self, key: &str) -> Option<&Value> {
        self.get(key).filter(|v| truthy(v))
    }

    fn obj_or_empty(&self, key: &str) -> &Map<String, Value> {
        self.get_truthy(key)
            .and_then(Value::as_object)
            .unwrap_or(&EMPTY_OBJ)
    }

    fn list_or_empty(&self, key: &str) -> &[Value] {
        self.get_truthy(key)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn py_get_str(&self, key: &str) -> String {
        match self.get(key) {
            Some(v) => py_str(v),
            None => "None".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn falsy_values_match_python() {
        for v in [
            json!(null),
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            assert!(!truthy(&v), "{v}");
        }
        for v in [
            json!(true),
            json!(1),
            json!(-2),
            json!(0.5),
            json!("0"),
            json!([0]),
            json!({"a": null}),
        ] {
            assert!(truthy(&v), "{v}");
        }
    }

    #[test]
    fn or_empty_collapses_falsy() {
        let v = json!({"a": {}, "b": "", "c": {"k": 1}, "d": [1], "e": [], "f": 0});
        assert!(v.obj_or_empty("a").is_empty());
        assert!(v.obj_or_empty("b").is_empty());
        assert!(v.obj_or_empty("missing").is_empty());
        assert_eq!(v.obj_or_empty("c").get("k"), Some(&json!(1)));
        assert_eq!(v.list_or_empty("d"), &[json!(1)]);
        assert!(v.list_or_empty("e").is_empty());
        assert!(v.list_or_empty("c").is_empty());
        assert_eq!(v.py_get_str("f"), "0");
        assert_eq!(v.py_get_str("missing"), "None");
        assert!(v.get_truthy("b").is_none());
        assert!(v.get_truthy("c").is_some());
    }
}
