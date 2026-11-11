//! Filter expressions → Freshservice `query_hash` (`query.py`).
//!
//! Date bounds are inclusive on both ends: `>` is sent as `>=` because the
//! API has no strict bound. A raw `query_hash` is percent-decoded at most
//! once, and only when it is not already JSON.

use serde_json::{Map, Value, json};

use crate::QueryError;
use fsv_py::py_repr_str;

const DATE_FIELDS: &[&str] = &["created_at", "updated_at", "due_by", "planned_start_date", "planned_end_date"];

/// One row of the `--debug` explanation.
#[derive(Clone, Debug, PartialEq)]
pub struct Explain {
    pub field: String,
    pub condition: String,
    pub operator: String,
    pub type_name: String,
    pub value: String,
}

/// People and group lookups the filter builder needs. The HTTP client
/// implements this; tests pass a fake.
pub trait Directory {
    fn autocomplete(&self, kind: &str, query: &str, extra: &[(&str, &str)]) -> Result<Vec<Value>, QueryError>;
    fn agent_groups(&self) -> Result<Value, QueryError>;
}

/// `build_query_hash_from_schema`. `None` when `where` is empty.
pub fn build_query_hash_from_schema(
    dir: Option<&dyn Directory>,
    resource: &str,
    schema: &Value,
    filters: &[String],
) -> Result<Option<String>, QueryError> {
    if filters.is_empty() {
        return Ok(None);
    }
    let mut query = Vec::new();
    let mut explain = Vec::new();
    for expr in filters {
        query.push(resolve_where(dir, resource, schema, expr, &mut explain)?.0);
    }
    let query = merge_date_ranges(query);
    if query.is_empty() {
        return Ok(None);
    }
    Ok(Some(dump(&Value::Array(query))))
}

/// `build_query_hash`. Schema is loaded by the caller (`schema.load` in Python).
pub fn build_query_hash(
    dir: Option<&dyn Directory>,
    resource: &str,
    schema: Option<&Value>,
    filters: &[String],
    raw: Option<&str>,
    or_grouping: bool,
) -> Result<(Option<String>, Vec<Value>, Vec<Explain>), QueryError> {
    let mut query = Vec::new();
    let mut explain = Vec::new();
    if let Some(raw) = raw.filter(|s| !s.is_empty()) {
        query.extend(load_query_hash(raw)?);
        explain.push(Explain {
            field: "query_hash".into(),
            condition: "raw".into(),
            operator: "raw".into(),
            type_name: "raw".into(),
            value: raw.to_string(),
        });
    }
    if !filters.is_empty() {
        let schema = schema.ok_or_else(|| QueryError("field not found: schema was not loaded".into()))?;
        for expr in filters {
            query.push(resolve_where(dir, resource, schema, expr, &mut explain)?.0);
        }
    }
    let query = merge_date_ranges(query);
    if query.is_empty() {
        return Ok((None, query, explain));
    }
    let payload = if or_grouping { json!({"any": query}) } else { Value::Array(query.clone()) };
    Ok((Some(dump(&payload)), query, explain))
}

pub fn resolve_where(
    dir: Option<&dyn Directory>,
    resource: &str,
    schema: &Value,
    expr: &str,
    explain: &mut Vec<Explain>,
) -> Result<(Value, ()), QueryError> {
    let (field_text, op, raw) = parse_where(expr)?;
    if DATE_FIELDS.contains(&field_text.as_str()) {
        let value = match op.as_str() {
            ">" | ">=" => json!({"from": raw}),
            "<" | "<=" => json!({"to": raw}),
            "!=" => {
                return Err(QueryError(format!(
                    "operator '!=' is not supported for date field {}; use a range such as `>=` / `<=`",
                    py_repr_str(&field_text)
                )));
            }
            "=" => json!({"from": raw, "to": raw}),
            _ => {
                return Err(QueryError(format!(
                    "unsupported date operator {}; use = != >= <= > <",
                    py_repr_str(&op)
                )));
            }
        };
        return Ok((item(&field_text, "custom_date", value, "default", explain, &field_text, &raw), ()));
    }
    if let Some(condition) = field_text.strip_suffix("_id") {
        let _ = condition;
        if matches!(op.as_str(), ">" | "<" | ">=" | "<=" | "~=") {
            return Err(QueryError(format!(
                "operator {} not supported on ID fields; use = or !=",
                py_repr_str(&op)
            )));
        }
        let operator = if op == "=" { "is_in" } else { "is_not_in" };
        let val = if ascii_digits(&raw) { json!(raw.parse::<i64>().unwrap_or(0)) } else { Value::String(raw.clone()) };
        return Ok((item(&field_text, operator, json!([val]), "default", explain, &field_text, &raw), ()));
    }

    let field = find_field(schema, &field_text)?.ok_or_else(|| {
        QueryError(format!("field not found: {}", py_repr_str(&field_text)))
    })?;
    let name = field.get("name").and_then(Value::as_str).unwrap_or(&field_text).to_string();
    let type_ = if truthy(field.get("default_field")) { "default" } else { "custom_field" };
    let ftype = field.get("field_type").and_then(Value::as_str).unwrap_or("");
    let operator = where_operator(&op)?;

    if name == "requester" {
        let req = resolve_requester(dir, &raw)?;
        let shown = format!("{} ({})", req.1, req.0);
        return Ok((
            item("requester_id", &operator, json!([{"id": req.0, "name": req.1}]), "default", explain, &name, &shown),
            (),
        ));
    }
    if name == "agent" {
        let agent_id = resolve_agent(dir, &raw)?;
        let condition = if resource == "tickets" { "responder_id" } else { "agent_id" };
        return Ok((item(condition, &operator, json!([agent_id.clone()]), "default", explain, &name, &agent_id), ()));
    }
    if name == "group" {
        let val = choice_value(&field, &raw, "default")?;
        let shown = scalar(&val);
        return Ok((item("group_id", &operator, json!([shown.clone()]), "default", explain, &name, &shown), ()));
    }
    if field.get("choices").and_then(Value::as_array).is_some_and(|c| !c.is_empty())
        || ftype.contains("dropdown")
        || ftype == "default_status"
        || ftype == "default_priority"
    {
        let val = choice_value(&field, &raw, type_)?;
        let shown = scalar(&val);
        return Ok((item(&name, &operator, json!([val]), type_, explain, &name, &shown), ()));
    }
    let val = if ascii_digits(&raw) && ftype.ends_with("number") {
        json!(raw.parse::<i64>().unwrap_or(0))
    } else {
        Value::String(raw.clone())
    };
    Ok((item(&name, &operator, json!([val]), type_, explain, &name, &raw), ()))
}

pub fn load_query_hash(raw: &str) -> Result<Vec<Value>, QueryError> {
    let text = if raw.trim_start().starts_with(['[', '{']) { raw.to_string() } else { unquote(raw) };
    let loaded: Value = serde_json::from_str(&text).map_err(|e| QueryError(format!("invalid query-hash JSON: {e}")))?;
    loaded.as_array().cloned().ok_or_else(|| QueryError("query-hash must be a JSON array".into()))
}

fn item(
    condition: &str,
    operator: &str,
    value: Value,
    type_: &str,
    explain: &mut Vec<Explain>,
    field: &str,
    raw: &str,
) -> Value {
    explain.push(Explain {
        field: field.to_string(),
        condition: condition.to_string(),
        operator: operator.to_string(),
        type_name: type_.to_string(),
        value: raw.to_string(),
    });
    json!({"value": value, "condition": condition, "operator": operator, "type": type_})
}

fn parse_where(expr: &str) -> Result<(String, String, String), QueryError> {
    let expr = expr.trim();
    let ops = [">=", "<=", "!=", "~=", "=", ">", "<"];
    let Some(start) = ops.iter().filter_map(|op| expr.find(op).map(|i| (i, *op))).min_by_key(|(i, _)| *i) else {
        return Err(QueryError(format!("invalid filter {}; expected FIELD=VALUE", py_repr_str(expr))));
    };
    let (field, rest) = expr.split_at(start.0);
    let value = strip_quotes(rest[start.1.len()..].trim());
    if field.trim().is_empty() {
        return Err(QueryError(format!("invalid filter {}; expected FIELD=VALUE", py_repr_str(expr))));
    }
    Ok((field.trim().to_string(), start.1.to_string(), value))
}

fn strip_quotes(value: &str) -> String {
    let value = value.trim();
    let b = value.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[0] == b[b.len() - 1] {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn where_operator(op: &str) -> Result<String, QueryError> {
    match op {
        "=" => Ok("is_in".into()),
        "!=" => Ok("is_not_in".into()),
        ">" | ">=" | "<" | "<=" => {
            Err(QueryError(format!("operator {} not supported for this field; use = or !=", py_repr_str(op))))
        }
        "~=" => Err(QueryError("operator ~= (contains) not supported; use = or !=".into())),
        _ => Ok("is_in".into()),
    }
}

fn find_field(schema: &Value, text: &str) -> Result<Option<Value>, QueryError> {
    let (scope, raw) = match text.split_once(':') {
        Some((maybe, rest)) if maybe == "default" || maybe == "custom" => (Some(maybe), rest),
        _ => (None, text),
    };
    let folded = raw.to_lowercase();
    let fields = schema.get("fields").and_then(Value::as_array).cloned().unwrap_or_default();
    let matches: Vec<Value> = fields
        .into_iter()
        .filter(|f| match scope {
            Some("default") => truthy(f.get("default_field")),
            Some("custom") => !truthy(f.get("default_field")),
            _ => true,
        })
        .filter(|f| field_name(f, "name").to_lowercase().contains(&folded) || field_name(f, "label").to_lowercase().contains(&folded))
        .collect();
    let exact: Vec<Value> = matches
        .iter()
        .filter(|f| field_name(f, "name").to_lowercase() == folded || field_name(f, "label").to_lowercase() == folded)
        .cloned()
        .collect();
    if exact.len() == 1 {
        return Ok(Some(exact[0].clone()));
    }
    if exact.len() > 1 {
        let by_name: Vec<Value> = exact.iter().filter(|f| field_name(f, "name").to_lowercase() == folded).cloned().collect();
        if by_name.len() == 1 {
            return Ok(Some(by_name[0].clone()));
        }
        return Err(ambiguous(text, &exact));
    }
    if matches.len() == 1 {
        return Ok(Some(matches[0].clone()));
    }
    if matches.len() > 1 {
        return Err(ambiguous(text, &matches));
    }
    Ok(None)
}

fn ambiguous(text: &str, fields: &[Value]) -> QueryError {
    let names: Vec<&str> = fields.iter().take(5).map(|f| f.get("name").and_then(Value::as_str).unwrap_or("")).collect();
    QueryError(format!("ambiguous field {}: {}", py_repr_str(text), names.join(", ")))
}

fn choice_value(field: &Value, raw: &str, type_: &str) -> Result<Value, QueryError> {
    let choices = field.get("choices").and_then(Value::as_array).cloned().unwrap_or_default();
    if choices.is_empty() {
        return Ok(if ascii_digits(raw) { json!(raw.parse::<i64>().unwrap_or(0)) } else { Value::String(raw.to_string()) });
    }
    let folded = raw.to_lowercase();
    for choice in &choices {
        if choice.get("id").map(scalar).as_deref().unwrap_or("") == raw {
            return Ok(if type_ == "custom_field" {
                choice.get("value").cloned().unwrap_or_else(|| choice.get("id").cloned().unwrap_or(Value::Null))
            } else {
                choice.get("id").cloned().unwrap_or(Value::Null)
            });
        }
        let labels = ["value", "name", "label", "requester_display_value"];
        if labels.iter().any(|k| choice.get(*k).map(scalar).unwrap_or_default().to_lowercase() == folded) {
            return Ok(if type_ == "custom_field" {
                choice.get("value").cloned().unwrap_or_else(|| choice.get("id").cloned().unwrap_or(Value::Null))
            } else {
                choice
                    .get("id")
                    .cloned()
                    .or_else(|| choice.get("value").cloned())
                    .unwrap_or_else(|| Value::String(raw.to_string()))
            });
        }
    }
    let shown: Vec<String> = choices
        .iter()
        .take(12)
        .map(|c| c.get("value").or_else(|| c.get("name")).or_else(|| c.get("id")).map(scalar).unwrap_or_default())
        .collect();
    let label = field.get("label").or_else(|| field.get("name")).map(scalar).unwrap_or_default();
    Err(QueryError(format!("no choice {} for {label}; options: {}", py_repr_str(raw), shown.join(", "))))
}

fn resolve_requester(dir: Option<&dyn Directory>, value: &str) -> Result<(i64, String), QueryError> {
    if ascii_digits(value) {
        return Ok((value.parse().unwrap_or(0), value.to_string()));
    }
    let dir = dir.ok_or_else(|| QueryError("no directory for requester lookup".into()))?;
    let results = dir.autocomplete("requesters", value, &[("all_users", "true")])?;
    let item = pick("requester", value, &results, &["value", "email", "details"])?;
    let id = item.get("id").and_then(Value::as_i64).ok_or_else(|| QueryError(format!("requester match has no id: {value}")))?;
    let name = item.get("value").map(scalar).unwrap_or_else(|| id.to_string());
    Ok((id, name))
}

fn resolve_agent(dir: Option<&dyn Directory>, value: &str) -> Result<String, QueryError> {
    if ascii_digits(value) {
        return Ok(value.to_string());
    }
    let dir = dir.ok_or_else(|| QueryError("no directory for agent lookup".into()))?;
    let results = dir.autocomplete("agents", value, &[])?;
    let item = pick("agent", value, &results, &["value", "id", "email", "details"])?;
    match item.get("user_id") {
        Some(id) if !id.is_null() => Ok(scalar(id)),
        _ => Err(QueryError(format!("agent match has no user_id: {value}"))),
    }
}

fn pick<'a>(kind: &str, value: &str, results: &'a [Value], keys: &[&str]) -> Result<&'a Value, QueryError> {
    if results.is_empty() {
        return Err(QueryError(format!("no {kind} match: {value}")));
    }
    let folded = value.to_lowercase();
    for item in results {
        if keys.iter().any(|k| item.get(*k).map(scalar).unwrap_or_default().to_lowercase() == folded) {
            return Ok(item);
        }
    }
    if results.len() == 1 {
        return Ok(&results[0]);
    }
    let choices: Vec<String> = results
        .iter()
        .take(5)
        .map(|x| x.get("details").or_else(|| x.get("value")).or_else(|| x.get("id")).map(scalar).unwrap_or_default())
        .collect();
    Err(QueryError(format!("multiple {kind} matches for {}: {}", py_repr_str(value), choices.join(", "))))
}

fn merge_date_ranges(query: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut index: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    for item in query {
        let value = item.get("value").cloned().unwrap_or(Value::Null);
        let is_range = value.as_object().is_some_and(|o| o.contains_key("from") || o.contains_key("to"));
        if is_range {
            let key = (
                item.get("condition").and_then(Value::as_str).unwrap_or("").to_string(),
                item.get("type").and_then(Value::as_str).unwrap_or("").to_string(),
            );
            if let Some(at) = index.get(&key) {
                if let (Some(dst), Some(src)) = (out[*at].get_mut("value").and_then(Value::as_object_mut), value.as_object()) {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                }
            } else {
                let mut merged = item;
                if let Some(obj) = merged.as_object_mut() {
                    obj.insert("value".into(), Value::Object(Map::new()));
                }
                if let (Some(dst), Some(src)) = (merged.get_mut("value").and_then(Value::as_object_mut), value.as_object()) {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                index.insert(key, out.len());
                out.push(merged);
            }
        } else {
            out.push(item);
        }
    }
    out
}

fn dump(value: &Value) -> String {
    fsv_py::Dumps::default().separators(",", ":").dump(value)
}

fn ascii_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(1) != 0,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        _ => false,
    }
}

fn field_name<'a>(field: &'a Value, key: &str) -> &'a str {
    field.get(key).and_then(Value::as_str).unwrap_or("")
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// `urllib.parse.unquote`: one percent-decode, `+` stays `+`, bad `%` stays.
fn unquote(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
            out.push(u8::from_str_radix(hex, 16).unwrap_or(0));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(expr: &str) -> Value {
        let mut explain = Vec::new();
        resolve_where(None, "changes", &json!({"fields": []}), expr, &mut explain).unwrap().0
    }

    #[test]
    fn date_not_equal_is_rejected() {
        let mut explain = Vec::new();
        let err = resolve_where(None, "changes", &json!({"fields": []}), "created_at!=2026-01-01", &mut explain).unwrap_err();
        assert!(err.0.contains("'!=' is not supported for date field"), "{}", err.0);
    }

    #[test]
    fn date_bounds_are_inclusive_and_merge() {
        assert_eq!(one("created_at>=2026-01-01")["value"], json!({"from": "2026-01-01"}));
        assert_eq!(one("created_at>2026-01-01")["value"], json!({"from": "2026-01-01"}));
        assert_eq!(one("created_at<2026-02-01")["value"], json!({"to": "2026-02-01"}));
        assert_eq!(one("created_at=2026-01-01")["value"], json!({"from": "2026-01-01", "to": "2026-01-01"}));
        let merged = build_query_hash_from_schema(
            None,
            "changes",
            &json!({"fields": []}),
            &["created_at>=2026-01-01".into(), "created_at<=2026-02-01".into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(merged, r#"[{"value":{"from":"2026-01-01","to":"2026-02-01"},"condition":"created_at","operator":"custom_date","type":"default"}]"#);
    }

    #[test]
    fn isdigit_is_ascii_only() {
        assert!(ascii_digits("123"));
        assert!(!ascii_digits("\u{0661}\u{0662}"));
        assert!(!ascii_digits("\u{00b2}"));
        assert!(!ascii_digits(""));
        assert_eq!(one("group_id=12")["value"], json!([12]));
        assert_eq!(one("group_id=\u{0661}\u{0662}")["value"], json!(["\u{0661}\u{0662}"]));
    }

    #[test]
    fn raw_query_hash_decoded_at_most_once() {
        let literal = r#"[{"condition":"subject","operator":"is_in","value":["50%25 done"],"type":"default"}]"#;
        assert_eq!(load_query_hash(literal).unwrap()[0]["value"], json!(["50%25 done"]));
        let encoded = literal.replace('[', "%5B").replace('{', "%7B").replace('"', "%22").replace(':', "%3A").replace(']', "%5D").replace('}', "%7D").replace(',', "%2C");
        // quote() encodes more than this hand replacement; use the same function Python's quote uses for the structural chars plus the value.
        let quoted = percent_encode_py(literal);
        assert_eq!(load_query_hash(&quoted).unwrap()[0]["value"], json!(["50%25 done"]));
        let _ = encoded;
        assert!(load_query_hash("not json").is_err());
    }

    fn percent_encode_py(text: &str) -> String {
        let mut out = String::new();
        for b in text.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }
}
