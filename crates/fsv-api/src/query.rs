//! Filter expressions → Freshservice `query_hash` (`query.py`).
//!
//! Date bounds are inclusive on both ends: `>` is sent as `>=` because the
//! API has no strict bound. A raw `query_hash` is percent-decoded at most
//! once, and only when it is not already JSON.

use serde_json::{Map, Value, json};

use crate::QueryError;
use fsv_core::pystr;
use fsv_py::py_repr_str;

const DATE_FIELDS: &[&str] = &[
    "created_at",
    "updated_at",
    "due_by",
    "planned_start_date",
    "planned_end_date",
];

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
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        extra: &[(&str, &str)],
    ) -> Result<Vec<Value>, QueryError>;
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
#[allow(clippy::type_complexity)]
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
        let schema =
            schema.ok_or_else(|| QueryError("field not found: schema was not loaded".into()))?;
        for expr in filters {
            query.push(resolve_where(dir, resource, schema, expr, &mut explain)?.0);
        }
    }
    let query = merge_date_ranges(query);
    if query.is_empty() {
        return Ok((None, query, explain));
    }
    let payload = if or_grouping {
        json!({"any": query})
    } else {
        Value::Array(query.clone())
    };
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
            ">" | "<" => {
                return Err(QueryError(format!(
                    "strict date operator {} is not supported; use >= or <= for inclusive bounds",
                    py_repr_str(&op)
                )));
            }
            ">=" => json!({"from": raw}),
            "<=" => json!({"to": raw}),
            "!=" => {
                return Err(QueryError(format!(
                    "operator '!=' is not supported for date field {}; use a range such as `>=` / `<=`",
                    py_repr_str(&field_text)
                )));
            }
            "=" => json!({"from": raw, "to": raw}),
            _ => {
                return Err(QueryError(format!(
                    "unsupported date operator {}; use = >= <=",
                    py_repr_str(&op)
                )));
            }
        };
        return Ok((
            query_item(
                &field_text,
                "custom_date",
                value,
                "default",
                explain,
                &field_text,
                &raw,
            ),
            (),
        ));
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
        let val = if ascii_digits(&raw) {
            json!(raw.parse::<i64>().unwrap_or(0))
        } else {
            Value::String(raw.clone())
        };
        return Ok((
            query_item(
                &field_text,
                operator,
                json!([val]),
                "default",
                explain,
                &field_text,
                &raw,
            ),
            (),
        ));
    }

    let field = find_schema_field(schema, &field_text)?
        .ok_or_else(|| QueryError(format!("field not found: {}", py_repr_str(&field_text))))?;
    let name = field
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&field_text)
        .to_string();
    let type_ = field_api_type(&field);
    let ftype = field
        .get("field_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    crate::filter_proof::require_filterable_field(&field)?;
    let operator = where_operator(&op)?;

    if name == "requester" {
        let req = resolve_requester(dir, &raw)?;
        let shown = format!("{} ({})", req.1, req.0);
        return Ok((
            query_item(
                "requester_id",
                &operator,
                json!([{"id": req.0, "name": req.1}]),
                "default",
                explain,
                &name,
                &shown,
            ),
            (),
        ));
    }
    if name == "agent" {
        let agent_id = resolve_agent(dir, &raw)?;
        let condition = if resource == "tickets" {
            "responder_id"
        } else {
            "agent_id"
        };
        return Ok((
            query_item(
                condition,
                &operator,
                json!([agent_id.clone()]),
                "default",
                explain,
                &name,
                &agent_id,
            ),
            (),
        ));
    }
    if name == "group" {
        let val = choice_value(&field, &raw, "default")?;
        let shown = scalar(&val);
        return Ok((
            query_item(
                "group_id",
                &operator,
                json!([shown.clone()]),
                "default",
                explain,
                &name,
                &shown,
            ),
            (),
        ));
    }
    if field
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|c| !c.is_empty())
        || ftype.contains("dropdown")
        || ftype == "default_status"
        || ftype == "default_priority"
    {
        let val = choice_value(&field, &raw, type_)?;
        let shown = scalar(&val);
        return Ok((
            query_item(
                &name,
                &operator,
                json!([val]),
                type_,
                explain,
                &name,
                &shown,
            ),
            (),
        ));
    }
    let val = if ascii_digits(&raw) && ftype.ends_with("number") {
        json!(raw.parse::<i64>().unwrap_or(0))
    } else {
        Value::String(raw.clone())
    };
    Ok((
        query_item(&name, &operator, json!([val]), type_, explain, &name, &raw),
        (),
    ))
}

/// Port of fsv.cli._merge_same_conditions: one `is_in` / `is_not_in`
/// item per (condition, operator, type), values concatenated in order.
pub fn merge_same_conditions(query: Vec<Value>) -> Vec<Value> {
    let mut seen: Vec<([Value; 3], usize)> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for item in query {
        let field = |k: &str| item.get(k).cloned().unwrap_or(Value::Null);
        let op = field("operator");
        if op != "is_in" && op != "is_not_in" {
            out.push(item);
            continue;
        }
        let key = [field("condition"), op, field("type")];
        let values = py_list(item.get("value"));
        match seen.iter().find(|(k, _)| *k == key) {
            Some(&(_, at)) => {
                if let Some(Value::Array(merged)) = out[at].get_mut("value") {
                    merged.extend(values);
                }
            }
            None => {
                let mut merged = item;
                if let Value::Object(m) = &mut merged {
                    m.insert("value".into(), Value::Array(values));
                }
                seen.push((key, out.len()));
                out.push(merged);
            }
        }
    }
    out
}

/// `list(value or [])`: strings split into code points, objects into keys;
/// a truthy scalar (a `TypeError` in Python) becomes a one-item list.
fn py_list(value: Option<&Value>) -> Vec<Value> {
    match value {
        None => Vec::new(),
        Some(v) if !fsv_py::truthy(v) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(Value::String(s)) => s.chars().map(|c| Value::String(c.into())).collect(),
        Some(Value::Object(m)) => m.keys().map(|k| Value::String(k.clone())).collect(),
        Some(other) => vec![other.clone()],
    }
}

pub fn load_query_hash(raw: &str) -> Result<Vec<Value>, QueryError> {
    let text = if raw.trim_start().starts_with(['[', '{']) {
        raw.to_string()
    } else {
        unquote(raw)
    };
    let loaded: Value = serde_json::from_str(&text)
        .map_err(|e| QueryError(format!("invalid query-hash JSON: {e}")))?;
    loaded
        .as_array()
        .cloned()
        .ok_or_else(|| QueryError("query-hash must be a JSON array".into()))
}

fn query_item(
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

/// Port of query._parse_where: `FIELD OP VALUE`, where the field is the
/// text before the first operator character and `OP` is one of
/// `>= <= != ~= = > <`. Controls, an empty field or value, an unterminated
/// quote and a value starting with an operator character are refused; a
/// malformed operator is never repaired.
pub fn parse_where(expr: &str) -> Result<(String, String, String), QueryError> {
    if expr.chars().any(|ch| (ch as u32) < 32) {
        return Err(QueryError(
            "invalid filter; controls and multiline expressions are not supported".into(),
        ));
    }
    let invalid = || {
        QueryError(format!(
            "invalid filter {}; expected FIELD followed by = != >= <= > < or ~=",
            py_repr_str(expr)
        ))
    };
    let text = pystr::strip(expr);
    let start = text.find(['<', '>', '=', '!', '~']).ok_or_else(invalid)?;
    if start == 0 {
        return Err(invalid());
    }
    let rest = &text[start..];
    let op = [">=", "<=", "!=", "~=", "=", ">", "<"]
        .into_iter()
        .find(|op| rest.starts_with(op))
        .ok_or_else(invalid)?;
    let field = pystr::strip(&text[..start]);
    let value = pystr::strip(&rest[op.len()..]);
    if field.is_empty() || value.is_empty() {
        return Err(QueryError(
            "invalid filter; field and value are required (quote literal empty text)".into(),
        ));
    }
    let first = value.chars().next().unwrap_or_default();
    if first == '"' || first == '\'' {
        if value.chars().count() < 2 || !value.ends_with(first) {
            return Err(QueryError(
                "invalid filter; unterminated quoted value".into(),
            ));
        }
    } else if "<>=!~".contains(first) {
        return Err(QueryError(
            "invalid filter operator sequence; quote a literal value starting with an operator"
                .into(),
        ));
    }
    Ok((field.to_string(), op.to_string(), strip_quotes(value)))
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

/// Port of query._where_operator: the wire operator; an unknown operator is
/// refused (no equality fallback).
pub fn where_operator(op: &str) -> Result<String, QueryError> {
    match op {
        "=" => Ok("is_in".into()),
        "!=" => Ok("is_not_in".into()),
        ">" | ">=" | "<" | "<=" => Err(QueryError(format!(
            "operator {} not supported for this field; use = or !=",
            py_repr_str(op)
        ))),
        "~=" => Err(QueryError(
            "operator ~= (contains) not supported; use = or !=".into(),
        )),
        _ => Err(QueryError(format!(
            "unknown filter operator {}; no equality fallback",
            py_repr_str(op)
        ))),
    }
}

/// `_field_api_type`.
fn field_api_type(field: &Value) -> &'static str {
    if truthy(field.get("default_field")) {
        "default"
    } else {
        "custom_field"
    }
}

/// `_schema_field_matches`: fields whose name or label contains `text`
/// (casefolded), limited to `scope` `default` / `custom`.
fn schema_field_matches(schema: &Value, text: &str, scope: Option<&str>) -> Vec<Value> {
    let needle = pystr::casefold(text);
    schema
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| match scope {
            Some("default") => truthy(f.get("default_field")),
            Some("custom") => !truthy(f.get("default_field")),
            _ => true,
        })
        .filter(|f| {
            pystr::casefold(&py_or_empty(f.get("name"))).contains(&needle)
                || pystr::casefold(&py_or_empty(f.get("label"))).contains(&needle)
        })
        .cloned()
        .collect()
}

fn find_schema_field(schema: &Value, text: &str) -> Result<Option<Value>, QueryError> {
    let (scope, raw) = match text.split_once(':') {
        Some((maybe, rest)) if maybe == "default" || maybe == "custom" => (Some(maybe), rest),
        _ => (None, text),
    };
    let folded = pystr::casefold(raw);
    let name_is = |f: &Value| pystr::casefold(&py_or_empty(f.get("name"))) == folded;
    let matches = schema_field_matches(schema, raw, scope);
    let exact: Vec<Value> = matches
        .iter()
        .filter(|f| name_is(f) || pystr::casefold(&py_or_empty(f.get("label"))) == folded)
        .cloned()
        .collect();
    if exact.len() == 1 {
        return Ok(Some(exact[0].clone()));
    }
    if exact.len() > 1 {
        let by_name: Vec<Value> = exact.iter().filter(|f| name_is(f)).cloned().collect();
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
    let names: Vec<String> = fields
        .iter()
        .take(5)
        .map(|f| scalar(f.get("name").unwrap_or(&Value::Null)))
        .collect();
    QueryError(format!(
        "ambiguous field {}: {}",
        py_repr_str(text),
        names.join(", ")
    ))
}

fn choice_value(field: &Value, raw: &str, type_: &str) -> Result<Value, QueryError> {
    let choices = field
        .get("choices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if choices.is_empty() {
        return Ok(if ascii_digits(raw) {
            json!(raw.parse::<i64>().unwrap_or(0))
        } else {
            Value::String(raw.to_string())
        });
    }
    let folded = pystr::casefold(raw);
    let custom_value = |c: &Value| {
        c.get("value")
            .or_else(|| c.get("id"))
            .cloned()
            .unwrap_or(Value::Null)
    };
    for choice in &choices {
        if py_or_empty(choice.get("id")) == raw {
            return Ok(if type_ == "custom_field" {
                custom_value(choice)
            } else {
                choice.get("id").cloned().unwrap_or(Value::Null)
            });
        }
        let labels = ["value", "name", "label", "requester_display_value"];
        if labels
            .iter()
            .any(|k| pystr::casefold(&py_or_empty(choice.get(*k))) == folded)
        {
            return Ok(if type_ == "custom_field" {
                custom_value(choice)
            } else {
                choice
                    .get("id")
                    .or_else(|| choice.get("value"))
                    .cloned()
                    .unwrap_or_else(|| Value::String(raw.to_string()))
            });
        }
    }
    let shown: Vec<String> = choices
        .iter()
        .take(12)
        .map(|c| py_or(c, &["value", "name", "id"]))
        .collect();
    Err(QueryError(format!(
        "no choice {} for {}; options: {}",
        py_repr_str(raw),
        py_or(field, &["label", "name"]),
        shown.join(", ")
    )))
}

fn resolve_requester(
    dir: Option<&dyn Directory>,
    value: &str,
) -> Result<(i64, String), QueryError> {
    if ascii_digits(value) {
        return Ok((value.parse().unwrap_or(0), value.to_string()));
    }
    let dir = dir.ok_or_else(|| QueryError("no directory for requester lookup".into()))?;
    let results = dir.autocomplete("requesters", value, &[("all_users", "true")])?;
    let item = pick_lookup("requester", value, &results, &["value", "email", "details"])?;
    let id = item
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| QueryError(format!("requester match has no id: {value}")))?;
    let name = item
        .get("value")
        .filter(|v| fsv_py::truthy(v))
        .map(scalar)
        .unwrap_or_else(|| id.to_string());
    Ok((id, name))
}

fn resolve_agent(dir: Option<&dyn Directory>, value: &str) -> Result<String, QueryError> {
    if ascii_digits(value) {
        return Ok(value.to_string());
    }
    let dir = dir.ok_or_else(|| QueryError("no directory for agent lookup".into()))?;
    if crate::agent_lookup::is_email(value) {
        let results = dir
            .autocomplete("agents", value, &[])
            .map_err(|_| QueryError("could not resolve agent; lookup unavailable".into()))?;
        return crate::agent_lookup::exact_agent_id(&results, value, false)
            .map(|id| id.to_string())
            .map_err(|err| QueryError(err.to_string()));
    }
    let results = dir.autocomplete("agents", value, &[])?;
    let item = pick_lookup(
        "agent",
        value,
        &results,
        &["value", "id", "email", "details"],
    )?;
    match item.get("user_id") {
        Some(id) if !id.is_null() => Ok(scalar(id)),
        _ => Err(QueryError(format!("agent match has no user_id: {value}"))),
    }
}

/// `_resolve_group`: the group id as text, from `bootstrap/agents_groups`
/// (exact id, else a unique casefolded name substring).
pub fn resolve_group(dir: &dyn Directory, value: &str) -> Result<String, QueryError> {
    if ascii_digits(value) {
        return Ok(value.to_string());
    }
    let data = dir.agent_groups()?;
    let folded = pystr::casefold(value);
    let groups: Vec<Value> = data
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|g| {
            pystr::casefold(&py_or_empty(g.get("name"))).contains(&folded)
                || py_or_empty(g.get("id")) == value
        })
        .cloned()
        .collect();
    let item = pick_lookup("group", value, &groups, &["name", "id"])?;
    match item.get("id") {
        Some(id) if !id.is_null() => Ok(scalar(id)),
        _ => Err(QueryError(format!("group match has no id: {value}"))),
    }
}

fn pick_lookup<'a>(
    kind: &str,
    value: &str,
    results: &'a [Value],
    keys: &[&str],
) -> Result<&'a Value, QueryError> {
    if results.is_empty() {
        return Err(QueryError(format!("no {kind} match: {value}")));
    }
    let folded = pystr::casefold(value);
    for item in results {
        if keys
            .iter()
            .any(|k| pystr::casefold(&py_or_empty(item.get(*k))) == folded)
        {
            return Ok(item);
        }
    }
    if results.len() == 1 {
        return Ok(&results[0]);
    }
    let choices: Vec<String> = results
        .iter()
        .take(5)
        .map(|x| py_or(x, &["details", "value", "id"]))
        .collect();
    Err(QueryError(format!(
        "multiple {kind} matches for {}: {}",
        py_repr_str(value),
        choices.join(", ")
    )))
}

fn merge_date_ranges(query: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut index: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    for item in query {
        let value = item.get("value").cloned().unwrap_or(Value::Null);
        let is_range = value
            .as_object()
            .is_some_and(|o| o.contains_key("from") || o.contains_key("to"));
        if is_range {
            let key = (
                item.get("condition")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                item.get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            );
            if let Some(at) = index.get(&key) {
                if let (Some(dst), Some(src)) = (
                    out[*at].get_mut("value").and_then(Value::as_object_mut),
                    value.as_object(),
                ) {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                }
            } else {
                let mut merged = item;
                if let Some(obj) = merged.as_object_mut() {
                    obj.insert("value".into(), Value::Object(Map::new()));
                }
                if let (Some(dst), Some(src)) = (
                    merged.get_mut("value").and_then(Value::as_object_mut),
                    value.as_object(),
                ) {
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

/// `_isdigit`: ASCII-only digits (`str.isdigit` also accepts superscripts
/// and other scripts' digits).
fn ascii_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(fsv_py::truthy)
}

/// `str(v or "")`.
fn py_or_empty(value: Option<&Value>) -> String {
    value
        .filter(|v| fsv_py::truthy(v))
        .map(scalar)
        .unwrap_or_default()
}

/// `str(o.get(a) or o.get(b) or ... or o.get(last))`.
fn py_or(obj: &Value, keys: &[&str]) -> String {
    keys.iter()
        .map(|k| obj.get(*k))
        .find(|v| truthy(*v))
        .flatten()
        .or_else(|| keys.last().and_then(|k| obj.get(*k)))
        .map_or_else(|| "None".to_string(), scalar)
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
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
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
#[path = "query_matrix_tests.rs"]
mod matrix_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn one(expr: &str) -> Value {
        let mut explain = Vec::new();
        resolve_where(None, "changes", &json!({"fields": []}), expr, &mut explain)
            .unwrap()
            .0
    }

    #[test]
    fn date_not_equal_is_rejected() {
        let mut explain = Vec::new();
        let err = resolve_where(
            None,
            "changes",
            &json!({"fields": []}),
            "created_at!=2026-01-01",
            &mut explain,
        )
        .unwrap_err();
        assert!(
            err.0.contains("'!=' is not supported for date field"),
            "{}",
            err.0
        );
    }

    #[test]
    fn date_bounds_are_inclusive_and_merge() {
        assert_eq!(
            one("created_at>=2026-01-01")["value"],
            json!({"from": "2026-01-01"})
        );
        assert_eq!(
            one("created_at<=2026-02-01")["value"],
            json!({"to": "2026-02-01"})
        );
        for op in [">", "<"] {
            let err = resolve_where(
                None,
                "changes",
                &json!({"fields": []}),
                &format!("created_at{op}2026-01-01"),
                &mut Vec::new(),
            )
            .unwrap_err();
            assert!(err.0.contains("strict date operator"), "{}", err.0);
        }
        assert_eq!(
            one("created_at=2026-01-01")["value"],
            json!({"from": "2026-01-01", "to": "2026-01-01"})
        );
        let merged = build_query_hash_from_schema(
            None,
            "changes",
            &json!({"fields": []}),
            &[
                "created_at>=2026-01-01".into(),
                "created_at<=2026-02-01".into(),
            ],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            merged,
            r#"[{"value":{"from":"2026-01-01","to":"2026-02-01"},"condition":"created_at","operator":"custom_date","type":"default"}]"#
        );
    }

    #[test]
    fn isdigit_is_ascii_only() {
        assert!(ascii_digits("123"));
        assert!(!ascii_digits("\u{0661}\u{0662}"));
        assert!(!ascii_digits("\u{00b2}"));
        assert!(!ascii_digits(""));
        assert_eq!(one("group_id=12")["value"], json!([12]));
        assert_eq!(
            one("group_id=\u{0661}\u{0662}")["value"],
            json!(["\u{0661}\u{0662}"])
        );
    }

    #[test]
    fn raw_query_hash_decoded_at_most_once() {
        let literal = r#"[{"condition":"subject","operator":"is_in","value":["50%25 done"],"type":"default"}]"#;
        assert_eq!(
            load_query_hash(literal).unwrap()[0]["value"],
            json!(["50%25 done"])
        );
        let encoded = literal
            .replace('[', "%5B")
            .replace('{', "%7B")
            .replace('"', "%22")
            .replace(':', "%3A")
            .replace(']', "%5D")
            .replace('}', "%7D")
            .replace(',', "%2C");
        // quote() encodes more than this hand replacement; use the same function Python's quote uses for the structural chars plus the value.
        let quoted = percent_encode_py(literal);
        assert_eq!(
            load_query_hash(&quoted).unwrap()[0]["value"],
            json!(["50%25 done"])
        );
        let _ = encoded;
        assert!(load_query_hash("not json").is_err());
    }

    fn oracle() -> Value {
        serde_json::from_str(include_str!("testdata/coverage_oracle.json")).unwrap()
    }

    fn outcome<T: Into<Value>>(r: Result<T, QueryError>) -> Value {
        match r {
            Ok(v) => json!({"ok": v.into()}),
            Err(e) => json!({"err": e.0}),
        }
    }

    fn strs(v: &Value) -> Vec<&str> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect()
    }

    #[test]
    fn small_helpers_match_python() {
        let o = oracle();
        let q = &o["query"];
        for (text, want) in q["isdigit"].as_object().unwrap() {
            assert_eq!(json!(ascii_digits(text)), *want, "{text:?}");
        }
        for (text, want) in q["strip_quotes"].as_object().unwrap() {
            assert_eq!(strip_quotes(text), want.as_str().unwrap(), "{text:?}");
        }
        for (expr, want) in q["parse_where"].as_object().unwrap() {
            let got = outcome(parse_where(expr).map(|(f, op, v)| json!([f, op, v])));
            assert_eq!(got, *want, "{expr:?}");
        }
        for case in q["field_api_type"].as_array().unwrap() {
            assert_eq!(field_api_type(&case[0]), case[1], "{case}");
        }
        for (op, want) in q["where_operator"].as_object().unwrap() {
            assert_eq!(outcome(where_operator(op)), *want, "{op:?}");
        }
        let mut explain = Vec::new();
        let item = query_item(
            "status",
            "is_in",
            json!([2]),
            "default",
            &mut explain,
            "status",
            "Planning",
        );
        assert_eq!(item, q["query_item"]["item"]);
        let e = &explain[0];
        assert_eq!(
            json!({"field": e.field, "condition": e.condition, "operator": e.operator, "type": e.type_name, "value": e.value}),
            q["query_item"]["explain"][0]
        );
    }

    #[test]
    fn schema_field_lookup_matches_python() {
        let o = oracle();
        let schema = &o["schema"];
        let q = &o["query"];
        for (key, want) in q["schema_field_matches"].as_object().unwrap() {
            let (text, scope) = key.split_once('|').unwrap();
            let scope = (scope != "None").then_some(scope);
            let got: Vec<String> = schema_field_matches(schema, text, scope)
                .iter()
                .map(|f| f["name"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(got, strs(want), "{key:?}");
        }
        for (text, want) in q["find_schema_field"].as_object().unwrap() {
            let got = outcome(
                find_schema_field(schema, text)
                    .map(|f| f.map_or(Value::Null, |f| f["name"].clone())),
            );
            assert_eq!(got, *want, "{text:?}");
        }
    }

    #[test]
    fn choices_and_lookups_match_python() {
        let o = oracle();
        let q = &o["query"];
        for case in q["pick_lookup"].as_array().unwrap() {
            let rows = case[1].as_array().unwrap();
            let keys = strs(&case[2]);
            let got =
                outcome(pick_lookup("agent", case[0].as_str().unwrap(), rows, &keys).cloned());
            assert_eq!(got, case[3], "{case}");
        }
        for case in q["choice_value"].as_array().unwrap() {
            let got = outcome(choice_value(
                &case[0],
                case[1].as_str().unwrap(),
                case[2].as_str().unwrap(),
            ));
            assert_eq!(got, case[3], "{case}");
        }
    }

    struct Dir {
        rows: Vec<Value>,
        calls: std::cell::RefCell<Vec<Value>>,
    }

    impl Directory for Dir {
        fn autocomplete(
            &self,
            kind: &str,
            query: &str,
            extra: &[(&str, &str)],
        ) -> Result<Vec<Value>, QueryError> {
            let extra: Map<String, Value> = extra
                .iter()
                .map(|(k, v)| ((*k).to_string(), json!(v)))
                .collect();
            self.calls.borrow_mut().push(json!([kind, query, extra]));
            Ok(self.rows.clone())
        }
        fn agent_groups(&self) -> Result<Value, QueryError> {
            Ok(json!({}))
        }
    }

    #[test]
    fn resolve_agent_and_requester_match_python() {
        let o = oracle();
        for (name, case) in o["query"]["resolve_agent"].as_object().unwrap() {
            let dir = Dir {
                rows: case["rows"].as_array().unwrap().clone(),
                calls: Default::default(),
            };
            let got = outcome(resolve_agent(Some(&dir), case["value"].as_str().unwrap()));
            assert_eq!(got, case["result"], "{name}");
            assert_eq!(Value::Array(dir.calls.take()), case["calls"], "{name}");
        }
        for (name, case) in o["query"]["resolve_requester"].as_object().unwrap() {
            let dir = Dir {
                rows: case["rows"].as_array().unwrap().clone(),
                calls: Default::default(),
            };
            let got = outcome(
                resolve_requester(Some(&dir), case["value"].as_str().unwrap())
                    .map(|(id, name)| json!({"id": id, "name": name})),
            );
            assert_eq!(got, case["result"], "{name}");
            assert_eq!(Value::Array(dir.calls.take()), case["calls"], "{name}");
        }
    }

    #[test]
    fn merge_and_build_query_hash_match_python() {
        let o = oracle();
        let q = &o["query"];
        let input = json!([
            {"condition": "created_at", "type": "default", "operator": "custom_date", "value": {"from": "a"}},
            {"condition": "status", "type": "default", "operator": "is_in", "value": [1]},
            {"condition": "created_at", "type": "default", "operator": "custom_date", "value": {"to": "b"}},
            {"condition": "created_at", "type": "custom_field", "operator": "custom_date", "value": {"to": "c"}},
            {"condition": "created_at", "type": "default", "operator": "custom_date", "value": {"from": "z"}},
        ]);
        let merged = merge_date_ranges(input.as_array().unwrap().clone());
        assert_eq!(Value::Array(merged), q["merge_date_ranges"]);

        let schema = &o["schema"];
        let raw = r#"[{"condition":"x","operator":"is_in","value":[1],"type":"default"}]"#;
        let cases: [(&str, &[&str], Option<&str>, bool); 5] = [
            ("empty", &[], None, false),
            ("where", &["status=Open", "cf_env!=Test"], None, false),
            (
                "or",
                &[
                    "status=Open",
                    "created_at>=2026-01-01",
                    "created_at<2026-02-01",
                ],
                None,
                true,
            ),
            ("raw", &["status=2"], Some(raw), false),
            ("bad", &["nope=1"], None, false),
        ];
        for (name, filters, raw, or_grouping) in cases {
            let filters: Vec<String> = filters.iter().map(|s| s.to_string()).collect();
            let got = outcome(
                build_query_hash(None, "changes", Some(schema), &filters, raw, or_grouping).map(
                    |(hash, query, explain)| {
                        let explain: Vec<Value> = explain
                            .iter()
                            .map(|e| json!({"field": e.field, "condition": e.condition, "operator": e.operator, "type": e.type_name, "value": e.value}))
                            .collect();
                        json!([hash, query, explain])
                    },
                ),
            );
            assert_eq!(got, q["build_query_hash"][name], "{name}");
        }
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

    /// `cli._merge_same_conditions` on lists, strings, objects and missing keys.
    #[test]
    fn merge_same_conditions_matches_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        let case = &oracle["merge_same_conditions"];
        let input = case["input"].as_array().unwrap().clone();
        assert_eq!(Value::Array(merge_same_conditions(input)), case["output"]);
    }

    #[test]
    fn resolve_agent_email_matches_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/agent_lookup_oracle.json")).unwrap();
        let native = json!({"id": "agent@example.com", "value": "Alex Acme", "user_id": 81});
        let email = Dir {
            rows: vec![native.clone()],
            calls: Default::default(),
        };
        assert_eq!(
            outcome(resolve_agent(Some(&email), "AGENT@example.com")),
            json!({"ok": oracle["cases"]["query_email"]["ok"]})
        );
        let ambiguous = Dir {
            rows: vec![native.clone(), native],
            calls: Default::default(),
        };
        assert_eq!(
            outcome(resolve_agent(Some(&ambiguous), "agent@example.com"))["err"],
            oracle["cases"]["query_ambiguous"]["msg"]
        );
    }
}
