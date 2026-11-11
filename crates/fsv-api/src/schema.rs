//! Form-schema cache shape and choice labels (`schema.py`).
//!
//! Fetching and the on-disk cache wait on `fsv-core`. The functions here are
//! the pure part: reading a cache document, merging the richer v2 choices
//! into the internal field list, and resolving a choice label.

use serde_json::{Map, Value};

/// Field definitions rarely change. `schema.CACHE_TTL`.
pub const CACHE_TTL_SECS: u64 = 7 * 86_400;

pub const PRIORITY: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High"), (4, "Urgent")];
pub const IMPACT: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High")];
pub const RISK: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High"), (4, "Very High")];

/// Non-empty field list stored under `data` or `fields`. Empty or malformed
/// documents are a cache miss.
pub fn doc_fields(doc: &Value) -> Option<&Vec<Value>> {
    let obj = doc.as_object()?;
    for key in ["data", "fields"] {
        if let Some(items) = obj.get(key).and_then(Value::as_array)
            && !items.is_empty()
        {
            return Some(items);
        }
    }
    None
}

/// `{saved_at, fields}` from an envelope or an old cache document.
/// Key presence wins over truthiness, so a stored empty list stays.
pub fn unwrap_schema(doc: &Map<String, Value>) -> Value {
    let fields = doc_fields(&Value::Object(doc.clone())).cloned().unwrap_or_default();
    let saved_at = doc.get("saved_at").cloned().unwrap_or(Value::from(0));
    serde_json::json!({"saved_at": saved_at, "fields": fields})
}

/// The field named `name`, or `None`.
pub fn field<'a>(name: &str, schema: &'a Value) -> Option<&'a Map<String, Value>> {
    schema.get("fields")?.as_array()?.iter().find_map(|f| {
        let obj = f.as_object()?;
        (obj.get("name").and_then(Value::as_str) == Some(name)).then_some(obj)
    })
}

/// Label of a dropdown value. Unlike `site::choice_name`, this compares with
/// Python `==` (so `1 == 1.0` and `True == 1`) and falls back to `str(val)`.
pub fn choice_label(field_name: &str, val: &Value, schema: &Value) -> String {
    let Some(f) = field(field_name, schema) else {
        return if val.is_null() { String::new() } else { fsv_py::py_str(val) };
    };
    if val.is_null() {
        return String::new();
    }
    if val.as_str() == Some("") {
        return String::new();
    }
    let choices = f.get("choices").and_then(Value::as_array);
    if let Some(choices) = choices {
        for c in choices.iter().filter_map(Value::as_object) {
            let id_hit = c.get("id").is_some_and(|id| py_eq(id, val));
            let value_hit = c.get("value").is_some_and(|v| py_eq(v, val));
            if id_hit || value_hit {
                return c
                    .get("value")
                    .filter(|v| fsv_py::truthy(v))
                    .or_else(|| c.get("name").filter(|v| fsv_py::truthy(v)))
                    .map(fsv_py::py_str)
                    .unwrap_or_else(|| fsv_py::py_str(val));
            }
        }
    }
    fsv_py::py_str(val)
}

/// Copy v2 `choices` onto an internal field when v2 lists more of them.
/// Internal fields stay in their original order. A v2 field with no `name`
/// key is ignored, matching `{"name" in f}`.
pub fn merge_v2_choices(int_fields: &mut [Value], v2_fields: &[Value]) {
    for field in int_fields.iter_mut() {
        let Some(obj) = field.as_object_mut() else { continue };
        let Some(name) = obj.get("name").filter(|v| !v.is_null()) else { continue };
        let Some(v2) = v2_fields.iter().find_map(|f| {
            let obj = f.as_object()?;
            if !obj.contains_key("name") {
                return None;
            }
            obj.get("name").is_some_and(|n| py_eq(n, name)).then_some(obj)
        }) else {
            continue;
        };
        if choice_len(v2.get("choices")) > choice_len(obj.get("choices")) {
            let choices = v2.get("choices").cloned().unwrap_or(Value::Array(vec![]));
            obj.insert("choices".to_string(), choices);
        }
    }
}

fn choice_len(v: Option<&Value>) -> usize {
    v.and_then(Value::as_array).map(|a| a.len()).unwrap_or(0)
}

/// Python `==` for JSON values the schema compares (ids and choice values).
pub(crate) fn py_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Bool(b), n) | (n, Value::Bool(b)) if n.is_number() => num(n) == Some(if *b { 1.0 } else { 0.0 }),
        (Value::Number(_), Value::Number(_)) => num(a) == num(b),
        _ => false,
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)).or_else(|| v.as_u64().map(|u| u as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn doc_fields_rejects_empty_and_reads_either_key() {
        assert!(doc_fields(&json!({"fields": []})).is_none());
        assert!(doc_fields(&json!([])).is_none());
        assert_eq!(doc_fields(&json!({"data": [{"name": "status"}]})).unwrap().len(), 1);
    }

    #[test]
    fn unwrap_keeps_an_explicit_empty_field_list_out_of_the_miss_path() {
        let doc = json!({"saved_at": 3, "fields": []}).as_object().unwrap().clone();
        let got = unwrap_schema(&doc);
        assert_eq!(got.get("fields").and_then(Value::as_array).unwrap().len(), 0);
        assert_eq!(got.get("saved_at"), Some(&json!(3)));
    }

    #[test]
    fn choice_label_uses_python_equality_and_string_fallback() {
        let schema = json!({"fields": [{"name": "priority", "choices": [
            {"id": 1, "value": "Low"},
            {"id": 2, "value": "Medium"}
        ]}]});
        assert_eq!(choice_label("priority", &json!(1), &schema), "Low");
        assert_eq!(choice_label("priority", &json!(1.0), &schema), "Low");
        assert_eq!(choice_label("priority", &json!(true), &schema), "Low");
        assert_eq!(choice_label("priority", &json!(9), &schema), "9");
        assert_eq!(choice_label("priority", &Value::Null, &schema), "");
        assert_eq!(choice_label("missing", &json!(1), &schema), "1");
    }

    #[test]
    fn merge_keeps_internal_choices_unless_v2_has_more() {
        let mut int_fields = vec![
            json!({"name": "status", "choices": [{"id": 1}]}),
            json!({"name": "source", "choices": [{"id": 1}]}),
        ];
        let v2 = vec![
            json!({"name": "status", "choices": [{"id": 1}, {"id": 2}]}),
            json!({"name": "source", "choices": []}),
            json!({"id": 9, "choices": [{"id": 1}, {"id": 2}, {"id": 3}]}),
        ];
        merge_v2_choices(&mut int_fields, &v2);
        assert_eq!(int_fields[0]["choices"].as_array().unwrap().len(), 2);
        assert_eq!(int_fields[1]["choices"].as_array().unwrap().len(), 1);
    }
}
