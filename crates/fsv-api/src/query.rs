//! Filter expressions → Freshservice `query_hash` (`query.py`).
//!
//! The wires follow what the web's filter panel and advanced builder send:
//!
//! * Dates: `is_in_the_range` with `{"from": D, "to": D}`. The server reads
//!   whole days with BOTH ends inclusive; a one-sided `{from}` or `{to}`
//!   answers HTTP 500, so a range must carry both ends. There is no strict
//!   `>`/`<` operator: `> D` is sent as `from D+1` and `< D` as `to D-1`.
//!   `on` + a relative token and `in_the_last`/`in_the_next` + a period
//!   exist only in the advanced builder (tickets, changes).
//! * Numbers (`custom_number`): `is`/`is_not`/`is_greater_than_or_equal`/
//!   `is_less_than_or_equal` with a STRING value, advanced builder only.
//! * Multi-select dropdowns: `is_in` with the INTEGER choice id, advanced.
//! * Text (subject, description, custom text/paragraph): no list filter
//!   exists in any list UI; refused rather than invented.
//!
//! A raw `query_hash` is percent-decoded at most once, and only when it is
//! not already JSON.

use serde_json::{Map, Value, json};

use crate::QueryError;
use fsv_core::pystr;
use fsv_py::{py_repr, py_repr_str, py_str};

pub const DATE_FIELDS: &[&str] = &[
    "created_at",
    "updated_at",
    "due_by",
    "planned_start_date",
    "planned_end_date",
    "fr_due_by",
    "closed_at",
    "resolved_at",
];
const ON_TOKENS: &[&str] = &["today", "yesterday", "tomorrow", "week", "month", "year"];
const RANGE_TOKENS: &[&str] = &[
    "five_minutes",
    "one_week",
    "one_month",
    "two_months",
    "six_months",
    "one_year",
];
const ADVANCED_MODULES: &[&str] = &["tickets", "changes"];
/// Date conditions offered by the advanced builder, keyed by fsv's field name.
/// fsv reads no filter-field metadata that names them: the builder's field
/// list ships in the web bundle, and neither the form fields nor the saved
/// views carry the advanced name, so this table is the source. The due,
/// first-response-due, closed and resolved dates differ from the record names.
const ADVANCED_DATE_CONDITIONS: &[(&str, &str)] = &[
    ("created_at", "created_at"),
    ("updated_at", "updated_at"),
    ("due_by", "due_by_date"),
    ("fr_due_by", "fr_due_by_date"),
    ("closed_at", "closed_date"),
    ("resolved_at", "resolved_date"),
    ("planned_start_date", "planned_start_date"),
    ("planned_end_date", "planned_end_date"),
];
/// Advanced dates that exist only in the ticket builder.
pub const TICKET_ONLY_DATES: &[&str] = &["fr_due_by", "closed_at", "resolved_at"];
/// Dates whose range goes through the advanced builder under its advanced
/// name: the basic `due_by` range answers 206 + `meta.search_error`
/// (unfiltered rows), and the closed/resolved/first-response dates have no
/// basic panel.
const ADVANCED_RANGE_DATES: &[&str] = &["due_by", "fr_due_by", "closed_at", "resolved_at"];
/// Problems: only the created range is proven; other shapes returned 0 rows silently.
const PROBLEM_DATE_FIELDS: &[&str] = &["created_at"];
const NUMBER_OPERATORS: &[(&str, &str)] = &[
    ("=", "is"),
    ("!=", "is_not"),
    (">=", "is_greater_than_or_equal"),
    ("<=", "is_less_than_or_equal"),
];
const TEXT_FIELD_TYPES: &[&str] = &[
    "default_subject",
    "default_description",
    "default_text",
    "custom_text",
    "custom_paragraph",
    "custom_url",
    "default_email",
];
/// Internal marker on items that need the advanced builder; never sent.
const ADV: &str = "_advanced";

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
    build_from_schema(dir, resource, schema, filters, &configured_workspace)
}

fn build_from_schema(
    dir: Option<&dyn Directory>,
    resource: &str,
    schema: &Value,
    filters: &[String],
    configured: &dyn Fn() -> Value,
) -> Result<Option<String>, QueryError> {
    if filters.is_empty() {
        return Ok(None);
    }
    let mut query = Vec::new();
    let mut explain = Vec::new();
    for expr in filters {
        query.push(resolve_where(dir, resource, schema, expr, &mut explain)?.0);
    }
    let query = merge_date_ranges(query)?;
    let workspace = if adv_needed(&query) {
        runtime_workspace(configured(), Some(schema))
    } else {
        Value::Null
    };
    Ok(serialize(resource, &query, false, &workspace)?.0)
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
    build_hash(
        dir,
        resource,
        schema,
        filters,
        raw,
        or_grouping,
        &configured_workspace,
    )
}

#[allow(clippy::type_complexity)]
fn build_hash(
    dir: Option<&dyn Directory>,
    resource: &str,
    schema: Option<&Value>,
    filters: &[String],
    raw: Option<&str>,
    or_grouping: bool,
    configured: &dyn Fn() -> Value,
) -> Result<(Option<String>, Vec<Value>, Vec<Explain>), QueryError> {
    let mut query = Vec::new();
    let mut explain = Vec::new();
    if let Some(raw) = raw.filter(|s| !s.is_empty()) {
        let loaded = load_query_hash(raw)?;
        check_raw_query(resource, &Value::Array(loaded.clone()))?;
        query.extend(loaded);
        explain.push(Explain {
            field: "query_hash".into(),
            condition: "raw".into(),
            operator: "raw".into(),
            type_name: "raw".into(),
            value: raw.to_string(),
        });
    }
    let mut loaded = None;
    if !filters.is_empty() {
        let schema =
            schema.ok_or_else(|| QueryError("field not found: schema was not loaded".into()))?;
        for expr in filters {
            query.push(resolve_where(dir, resource, schema, expr, &mut explain)?.0);
        }
        loaded = Some(schema);
    }
    let query = merge_date_ranges(query)?;
    let workspace = if or_grouping || adv_needed(&query) {
        runtime_workspace(configured(), loaded)
    } else {
        Value::Null
    };
    let (payload, _, clean) = serialize(resource, &query, or_grouping, &workspace)?;
    Ok((payload, clean, explain))
}

/// `is_text_field`: subject, description and free-text field types.
pub fn is_text_field(field: &Value) -> bool {
    let ftype = py_or_empty(field.get("field_type"));
    let name = py_or_empty(field.get("name"));
    TEXT_FIELD_TYPES.contains(&ftype.as_str()) || matches!(name.as_str(), "subject" | "description")
}

/// `text_filter_refusal`.
pub fn text_filter_refusal(resource: &str, field: &Value) -> QueryError {
    let label = field
        .get("label")
        .filter(|v| fsv_py::truthy(v))
        .or_else(|| field.get("name"))
        .map_or_else(|| "None".to_string(), py_repr);
    QueryError(format!(
        "no list filter exists for text field {label}: the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv {resource} search TEXT` for a keyword search"
    ))
}

fn advanced(mut item: Value) -> Value {
    if let Some(obj) = item.as_object_mut() {
        obj.insert(ADV.into(), Value::Bool(true));
    }
    item
}

/// `_adv_needed`: whether any item needs the advanced builder.
pub fn adv_needed(query: &[Value]) -> bool {
    query.iter().any(|i| i.get(ADV).is_some_and(fsv_py::truthy))
}

/// `re.fullmatch(r"\d{4}-\d{2}-\d{2}")` (`\d` is any Unicode decimal digit).
fn iso_day_shape(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.len() == 10
        && chars.iter().enumerate().all(|(i, &c)| match i {
            4 | 7 => c == '-',
            _ => pystr::decimal_value(c).is_some(),
        })
}

/// `datetime.date.fromisoformat` for a `YYYY-MM-DD` shape (ASCII digits only).
fn iso_day(text: &str) -> Option<jiff::civil::Date> {
    if !text.is_ascii() {
        return None;
    }
    let year: i16 = text[..4].parse().ok()?;
    let month: i8 = text[5..7].parse().ok()?;
    let day: i8 = text[8..].parse().ok()?;
    if year < 1 {
        return None;
    }
    jiff::civil::Date::new(year, month, day).ok()
}

/// `date + timedelta(days=n)`, bounded like Python's `date` (years 1 to 9999).
fn shift_day(day: jiff::civil::Date, days: i64) -> Result<String, QueryError> {
    day.checked_add(jiff::Span::new().days(days))
        .ok()
        .filter(|d| (1..=9999).contains(&d.year()))
        .map(|d| d.to_string())
        .ok_or_else(|| QueryError("date value out of range".into()))
}

/// `date_condition`: one captured date condition (see the module notes).
pub fn date_condition(
    resource: &str,
    field: &str,
    op: &str,
    raw: &str,
    explain: &mut Vec<Explain>,
    shown: &str,
) -> Result<Value, QueryError> {
    let text = pystr::strip(raw);
    let low = pystr::casefold(text);
    for (prefix, operator, tokens) in [
        ("on:", "on", ON_TOKENS),
        ("last:", "in_the_last", RANGE_TOKENS),
        ("next:", "in_the_next", RANGE_TOKENS),
    ] {
        let Some(rest) = low.strip_prefix(prefix) else {
            continue;
        };
        let token = pystr::split_ws(rest).join("_");
        if op != "=" {
            return Err(QueryError(format!(
                "relative date {prefix}PERIOD takes only ="
            )));
        }
        if !tokens.contains(&token.as_str()) {
            return Err(QueryError(format!(
                "unknown {operator} period {}; use one of: {}",
                py_repr_str(&token),
                tokens.join(", ")
            )));
        }
        if !ADVANCED_MODULES.contains(&resource) {
            return Err(QueryError(format!(
                "{operator} date filters exist only in the tickets/changes advanced filter; {resource} has none, nothing was sent"
            )));
        }
        return Ok(advanced(query_item(
            advanced_date_condition(resource, field)?,
            operator,
            json!(token),
            "default",
            explain,
            shown,
            text,
        )));
    }
    if !iso_day_shape(text) {
        return Err(QueryError(
            "date filters take a whole day YYYY-MM-DD (the server ignores time of day) or a relative on:/last:/next: period; nothing was sent"
                .into(),
        ));
    }
    let Some(day) = iso_day(text) else {
        return Err(QueryError(format!("invalid date {}", py_repr_str(text))));
    };
    if resource == "problems" && !PROBLEM_DATE_FIELDS.contains(&field) {
        return Err(QueryError(format!(
            "{field} date filters are unproven on problems (the list returned 0 rows silently); only created_at ranges are supported there, nothing was sent"
        )));
    }
    let mut note = String::new();
    let value = match op {
        "=" => json!({"from": text, "to": text}),
        ">=" => json!({"from": text}),
        "<=" => json!({"to": text}),
        ">" => {
            let shifted = shift_day(day, 1)?;
            note = format!(" (no strict operator exists: sent as from {shifted}, whole days)");
            json!({"from": shifted})
        }
        "<" => {
            let shifted = shift_day(day, -1)?;
            note = format!(" (no strict operator exists: sent as to {shifted}, whole days)");
            json!({"to": shifted})
        }
        _ => {
            return Err(QueryError(format!(
                "date operator {} is not supported; use = > >= < <= (whole days)",
                py_repr_str(op)
            )));
        }
    };
    if ADVANCED_RANGE_DATES.contains(&field) {
        if !ADVANCED_MODULES.contains(&resource) {
            return Err(QueryError(format!(
                "{field} date ranges exist only in the tickets/changes advanced filter; {resource} has none, nothing was sent"
            )));
        }
        return Ok(advanced(query_item(
            advanced_date_condition(resource, field)?,
            "is_in_the_range",
            value,
            "default",
            explain,
            shown,
            &format!("{text}{note}"),
        )));
    }
    Ok(query_item(
        field,
        "is_in_the_range",
        value,
        "default",
        explain,
        shown,
        &format!("{text}{note}"),
    ))
}

/// `advanced_date_condition`: the advanced builder's condition name for an fsv date field.
pub fn advanced_date_condition(resource: &str, field: &str) -> Result<&'static str, QueryError> {
    match lookup(ADVANCED_DATE_CONDITIONS, field) {
        Some(cond) if !(TICKET_ONLY_DATES.contains(&field) && resource != "tickets") => Ok(cond),
        _ => Err(QueryError(format!(
            "{field} is not a date in the {resource} advanced filter builder; nothing was sent"
        ))),
    }
}

fn lookup<'a>(table: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// `_int_choice_id`: a positive int, or an ASCII digit string of one.
fn int_choice_id(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64().filter(|n| *n > 0),
        Value::String(s) if ascii_digits(s) => s.parse::<i64>().ok().filter(|n| *n > 0),
        _ => None,
    }
}

/// `typed_condition`: text refusal, number and multi-select wires; `None`
/// when the field is none of these.
pub fn typed_condition(
    resource: &str,
    field: &Value,
    op: &str,
    raw: &str,
    explain: &mut Vec<Explain>,
    shown: &str,
) -> Result<Option<Value>, QueryError> {
    let ftype = py_or_empty(field.get("field_type"));
    let name = field
        .get("name")
        .filter(|v| fsv_py::truthy(v))
        .map_or_else(|| shown.to_string(), py_str);
    let label = field
        .get("label")
        .filter(|v| fsv_py::truthy(v))
        .map_or_else(|| py_repr_str(&name), py_repr);
    if is_text_field(field) {
        return Err(text_filter_refusal(resource, field));
    }
    if ftype == "custom_number" {
        let Some(operator) = lookup(NUMBER_OPERATORS, op) else {
            return Err(QueryError(format!(
                "operator {} not supported for number field {label}; use = != >= <= (the filter builder has no strict > or <)",
                py_repr_str(op)
            )));
        };
        let value = pystr::strip(raw);
        let digits = value.strip_prefix('-').unwrap_or(value);
        if !ascii_digits(digits) {
            return Err(QueryError(format!(
                "number filter {label} takes an integer value"
            )));
        }
        if !ADVANCED_MODULES.contains(&resource) {
            return Err(QueryError(
                "number filters exist only in the tickets/changes advanced filter; nothing was sent"
                    .into(),
            ));
        }
        return Ok(Some(advanced(query_item(
            &name,
            operator,
            json!(value),
            "custom_field",
            explain,
            shown,
            value,
        ))));
    }
    if ftype.contains("multi_select") {
        if !matches!(op, "=" | "!=") {
            return Err(QueryError(format!(
                "operator {} not supported for multi-select {label}; use = or !=",
                py_repr_str(op)
            )));
        }
        if !ADVANCED_MODULES.contains(&resource) {
            return Err(QueryError(
                "multi-select filters exist only in the tickets/changes advanced filter; nothing was sent"
                    .into(),
            ));
        }
        let wanted = pystr::strip(raw);
        let folded = pystr::casefold(wanted);
        let choices: &[Value] = field
            .get("choices")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice);
        let found: Vec<&Value> = choices
            .iter()
            .filter(|ch| ch.is_object())
            .filter(|ch| {
                py_str(ch.get("id").unwrap_or(&Value::Null)) == wanted
                    || ["value", "name", "label"]
                        .iter()
                        .map(|k| py_or_empty(ch.get(*k)))
                        .chain(std::iter::once(humanize_or_empty(ch.get("value"))))
                        .any(|x| pystr::casefold(pystr::strip(&x)) == folded)
            })
            .collect();
        let mut ids: Vec<Option<i64>> =
            found.iter().map(|ch| int_choice_id(ch.get("id"))).collect();
        ids.sort();
        ids.dedup();
        let cid = match ids.as_slice() {
            [Some(cid)] => *cid,
            _ => {
                let options: Vec<String> = choices
                    .iter()
                    .take(12)
                    .filter(|ch| ch.is_object())
                    .map(|ch| {
                        let v = ch
                            .get("value")
                            .filter(|v| fsv_py::truthy(v))
                            .or_else(|| ch.get("id"));
                        py_str(v.unwrap_or(&Value::Null))
                    })
                    .collect();
                return Err(QueryError(format!(
                    "no unique integer choice id for {} in {label} runtime metadata; options: {}",
                    py_repr_str(raw),
                    options.join(", ")
                )));
            }
        };
        let operator = if op == "=" { "is_in" } else { "is_not_in" };
        return Ok(Some(advanced(query_item(
            &name,
            operator,
            json!([cid]),
            "custom_field",
            explain,
            shown,
            &format!("{raw} ({cid})"),
        ))));
    }
    Ok(None)
}

/// `merge_date_ranges`: join `>=`/`<=` halves per (condition, type); refuse
/// duplicate or missing ends.
pub fn merge_date_ranges(query: Vec<Value>) -> Result<Vec<Value>, QueryError> {
    let mut out: Vec<Value> = Vec::new();
    let mut ranges: Vec<((Value, Value), usize)> = Vec::new();
    for item in query {
        let range = item
            .get("value")
            .and_then(Value::as_object)
            .filter(|v| v.contains_key("from") || v.contains_key("to"))
            .cloned();
        let Some(range) = range else {
            out.push(item);
            continue;
        };
        let key = (
            item.get("condition").cloned().unwrap_or(Value::Null),
            item.get("type").cloned().unwrap_or(Value::Null),
        );
        let at = match ranges.iter().find(|(k, _)| *k == key) {
            Some((_, at)) => *at,
            None => {
                let mut merged = item.as_object().cloned().unwrap_or_default();
                merged.insert("value".into(), Value::Object(Map::new()));
                ranges.push((key.clone(), out.len()));
                out.push(Value::Object(merged));
                out.len() - 1
            }
        };
        if let Some(Value::Object(v)) = out[at].get_mut("value") {
            if let Some(end) = range.keys().find(|end| v.contains_key(*end)) {
                return Err(QueryError(format!(
                    "two '{end}' bounds for {}; give one lower and one upper bound",
                    py_str(&key.0)
                )));
            }
            v.extend(range);
        }
    }
    for ((cond, _), at) in &ranges {
        let cond = py_str(cond);
        let value = out[*at]
            .get("value")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if value.len() != 2 || !value.contains_key("from") || !value.contains_key("to") {
            return Err(QueryError(format!(
                "a date range on {cond} needs both ends (e.g. {cond}>=2030-01-01 and {cond}<=2030-01-31): the server answers a one-sided range with HTTP 500, so nothing was sent"
            )));
        }
        let (from, to) = (py_str(&value["from"]), py_str(&value["to"]));
        if from > to {
            return Err(QueryError(format!(
                "empty date range on {cond}: from {from} is after to {to}"
            )));
        }
    }
    Ok(out)
}

/// `workspace_item`: tickets take `is_in [{id}]`, changes `is <id>`.
fn workspace_item(resource: &str, workspace: &Value) -> Option<Value> {
    let id = int_choice_id(Some(workspace))?;
    Some(if resource == "tickets" {
        json!({"condition": "workspace_id", "operator": "is_in", "value": [{"id": id}], "type": "default"})
    } else {
        json!({"condition": "workspace_id", "operator": "is", "value": id, "type": "default"})
    })
}

/// `formmeta.configured_workspace()` from the current profile's config.
pub fn configured_workspace() -> Value {
    crate::formmeta::configured_workspace_in(&fsv_core::config::load_config())
}

/// `runtime_workspace`: the configured workspace, else the form metadata's.
pub fn runtime_workspace(configured: Value, schema: Option<&Value>) -> Value {
    if fsv_py::truthy(&configured) {
        return configured;
    }
    let fields: &[Value] = schema
        .and_then(|s| s.get("fields"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    crate::formmeta::workspace_from_fields(fields)
}

/// `serialize`: `(payload, advanced, clean items)`. Advanced payloads go in
/// `advanced_query_hash`; `--or` puts the conditions in `any` and the
/// workspace in `all`.
pub fn serialize(
    resource: &str,
    query: &[Value],
    or_grouping: bool,
    workspace: &Value,
) -> Result<(Option<String>, bool, Vec<Value>), QueryError> {
    let is_advanced = or_grouping || adv_needed(query);
    let clean: Vec<Value> = query
        .iter()
        .map(|i| match i {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(k, _)| *k != ADV)
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            other => other.clone(),
        })
        .collect();
    if clean.is_empty() {
        return Ok((None, false, clean));
    }
    if !is_advanced {
        return Ok((Some(dump(&Value::Array(clean.clone()))), false, clean));
    }
    if !ADVANCED_MODULES.contains(&resource) {
        return Err(QueryError(format!(
            "{resource} has no advanced filter (OR, relative on/last/next dates, numbers, multi-select); nothing was sent"
        )));
    }
    let extra: Vec<Value> = workspace_item(resource, workspace).into_iter().collect();
    let body = if or_grouping {
        json!({"any": clean, "all": extra})
    } else {
        let mut all = clean.clone();
        all.extend(extra);
        json!({"all": all, "any": []})
    };
    Ok((Some(dump(&body)), true, clean))
}

fn refuse_unsupported_filter(resource: &str, condition: &str) -> Result<(), QueryError> {
    if resource == "tickets" && condition == "approval_status" {
        return Err(QueryError(format!(
            "{condition} is not a {resource} list filter (the server ignores it and returns unfiltered rows); read approval_status per record instead (e.g. `get`)"
        )));
    }
    Ok(())
}

/// `_check_raw_query`: refuse raw conditions the list ignores.
pub fn check_raw_query(resource: &str, items: &Value) -> Result<(), QueryError> {
    let rows = items
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(std::slice::from_ref(items));
    for item in rows {
        let Some(obj) = item.as_object() else {
            continue;
        };
        if let Some(condition) = obj.get("condition").and_then(Value::as_str) {
            refuse_unsupported_filter(resource, condition)?;
        }
        for key in ["any", "all"] {
            if let Some(nested) = obj.get(key) {
                check_raw_query(resource, nested)?;
            }
        }
    }
    Ok(())
}

pub fn resolve_where(
    dir: Option<&dyn Directory>,
    resource: &str,
    schema: &Value,
    expr: &str,
    explain: &mut Vec<Explain>,
) -> Result<(Value, ()), QueryError> {
    let (field_text, op, raw) = parse_where(expr)?;
    refuse_unsupported_filter(resource, &field_text)?;
    if field_text == "approval_status" && resource == "changes" {
        if !matches!(op.as_str(), "=" | "!=") || !ascii_digits(&raw) {
            return Err(QueryError(
                "approval_status takes a numeric value with = or != (e.g. approval_status=0)"
                    .into(),
            ));
        }
        let operator = if op == "=" { "is_in" } else { "is_not_in" };
        return Ok((
            query_item(
                "approval_status",
                operator,
                json!([raw]),
                "default",
                explain,
                &field_text,
                &raw,
            ),
            (),
        ));
    }
    if DATE_FIELDS.contains(&field_text.as_str()) {
        return Ok((
            date_condition(resource, &field_text, &op, &raw, explain, &field_text)?,
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
            pystr::int_json(&raw).unwrap_or_else(|| json!(raw))
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
    refuse_unsupported_filter(resource, &name)?;
    let type_ = field_api_type(&field);
    let ftype = field
        .get("field_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    if DATE_FIELDS.contains(&name.as_str()) {
        return Ok((
            date_condition(resource, &name, &op, &raw, explain, &field_text)?,
            (),
        ));
    }
    if let Some(typed) = typed_condition(resource, &field, &op, &raw, explain, &field_text)? {
        return Ok((typed, ()));
    }
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
    if let Some(named) = crate::filter_proof::name_valued_choice(&field, &raw)? {
        return Ok((
            query_item(
                &name,
                &operator,
                json!([named.clone()]),
                type_,
                explain,
                &name,
                &named,
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
        pystr::int_json(&raw).unwrap_or_else(|| json!(raw))
    } else {
        Value::String(raw.clone())
    };
    Ok((
        query_item(&name, &operator, json!([val]), type_, explain, &name, &raw),
        (),
    ))
}

/// Port of fsv.cli._merge_same_conditions: one `is_in` / `is_not_in`
/// item per (condition, operator, type), list values concatenated in order;
/// an item whose value is not a list passes through unchanged.
pub fn merge_same_conditions(query: Vec<Value>) -> Vec<Value> {
    let mut seen: Vec<([Value; 3], usize)> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for item in query {
        let field = |k: &str| item.get(k).cloned().unwrap_or(Value::Null);
        let op = field("operator");
        let Some(values) = item.get("value").and_then(Value::as_array).cloned() else {
            out.push(item);
            continue;
        };
        if op != "is_in" && op != "is_not_in" {
            out.push(item);
            continue;
        }
        let key = [field("condition"), op, field("type")];
        match seen.iter().find(|(k, _)| *k == key) {
            Some(&(_, at)) => {
                if let Some(Value::Array(merged)) = out[at].get_mut("value") {
                    merged.extend(values);
                }
            }
            None => {
                seen.push((key, out.len()));
                out.push(item);
            }
        }
    }
    out
}

const LIST_OPERATORS: &[&str] = &["is_in", "is_not_in"];

/// `validate_raw_query`: refuse raw query-hash shapes fsv cannot pass on
/// faithfully (no wrapping: the wire is sent exactly as given).
pub fn validate_raw_query(items: &Value, at: &str) -> Result<(), QueryError> {
    let rows = items
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(std::slice::from_ref(items));
    for (n, item) in rows.iter().enumerate() {
        let label = format!("{at} item {}", n + 1);
        let Some(obj) = item.as_object() else {
            return Err(QueryError(format!(
                "{label} must be a JSON object with condition/operator/value; nothing was sent"
            )));
        };
        for key in ["any", "all"] {
            if let Some(nested) = obj.get(key) {
                if !nested.is_array() {
                    return Err(QueryError(format!(
                        "{label}: '{key}' must be a JSON list of conditions; nothing was sent"
                    )));
                }
                validate_raw_query(nested, &format!("{label} {key}"))?;
            }
        }
        let Some(condition) = obj.get("condition") else {
            if !obj.contains_key("any") && !obj.contains_key("all") {
                return Err(QueryError(format!(
                    "{label} has no condition; nothing was sent"
                )));
            }
            continue;
        };
        let (Some(condition), Some(operator)) = (
            condition.as_str(),
            obj.get("operator").and_then(Value::as_str),
        ) else {
            return Err(QueryError(format!(
                "{label}: condition and operator must be strings; nothing was sent"
            )));
        };
        let value = obj.get("value").unwrap_or(&Value::Null);
        if LIST_OPERATORS.contains(&operator) && !value.is_array() {
            return Err(QueryError(format!(
                "{label} ({condition}): operator {operator} takes a JSON list value, e.g. \"value\": [{}]; nothing was sent",
                fsv_py::dumps(value, &fsv_py::Dumps::default())
            )));
        }
    }
    Ok(())
}

pub fn load_query_hash(raw: &str) -> Result<Vec<Value>, QueryError> {
    let text = if raw.trim_start().starts_with(['[', '{']) {
        raw.to_string()
    } else {
        unquote(raw)
    };
    let loaded: Value = serde_json::from_str(&text)
        .map_err(|e| QueryError(format!("invalid query-hash JSON: {e}")))?;
    let Value::Array(items) = loaded else {
        return Err(QueryError("query-hash must be a JSON array".into()));
    };
    validate_raw_query(&Value::Array(items.clone()), "query-hash")?;
    Ok(items)
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

/// `str(humanize_key(value) or "")`.
fn humanize_or_empty(value: Option<&Value>) -> String {
    crate::schema::humanize_value(value)
}

pub(crate) fn choice_value(field: &Value, raw: &str, type_: &str) -> Result<Value, QueryError> {
    let choices = field
        .get("choices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if choices.is_empty() {
        return Ok(if ascii_digits(raw) {
            pystr::int_json(raw).unwrap_or_else(|| json!(raw))
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
            .map(|k| py_or_empty(choice.get(*k)))
            .chain(std::iter::once(humanize_or_empty(choice.get("value"))))
            .any(|x| pystr::casefold(&x) == folded)
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
#[path = "query_wire_tests.rs"]
mod wire_tests;

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
        assert!(err.0.contains("'!=' is not supported"), "{}", err.0);
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
        // No strict operator exists: a whole-day shift, still one half of a range.
        assert_eq!(
            one("created_at>2026-01-01")["value"],
            json!({"from": "2026-01-02"})
        );
        assert_eq!(
            one("created_at<2026-01-01")["value"],
            json!({"to": "2025-12-31"})
        );
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
            r#"[{"value":{"from":"2026-01-01","to":"2026-02-01"},"condition":"created_at","operator":"is_in_the_range","type":"default"}]"#
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
        assert_eq!(outcome(merged), q["merge_date_ranges"]);

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
                build_hash(
                    None,
                    "changes",
                    Some(schema),
                    &filters,
                    raw,
                    or_grouping,
                    &|| Value::Null,
                )
                .map(
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

    #[test]
    fn approval_status_is_refused_for_tickets_and_kept_for_changes() {
        let mut explain = Vec::new();
        let err = resolve_where(
            None,
            "tickets",
            &json!({}),
            "approval_status=0",
            &mut explain,
        )
        .unwrap_err();
        assert!(err.0.contains("is not a tickets list filter"));
        let (item, _) = resolve_where(
            None,
            "changes",
            &json!({}),
            "approval_status=0",
            &mut explain,
        )
        .unwrap();
        assert_eq!(item["operator"], "is_in");
        assert_eq!(item["value"], json!(["0"]));
        let raw = r#"[{"condition":"approval_status","operator":"is_in","value":["0"]}]"#;
        let err = build_query_hash(None, "tickets", None, &[], Some(raw), false).unwrap_err();
        assert!(err.0.contains("is not a tickets list filter"));
    }

    /// Python `query._resolve_where` sends a digit string past 64 bits as
    /// the exact big int (it was `0`). Expected items from that run.
    #[test]
    fn oversized_ids_stay_exact() {
        let sch = json!({"fields": [
            {"name": "cf_count", "label": "Count", "field_type": "custom_number"},
            {"name": "cf_pick", "label": "Pick", "field_type": "custom_dropdown"}
        ]});
        let big = "99999999999999999999999";
        for (expr, want) in [
            ("group_id", r#"{"value": [99999999999999999999999], "condition": "group_id", "operator": "is_in", "type": "default"}"#),
            ("cf_count", r#"{"value": "99999999999999999999999", "condition": "cf_count", "operator": "is", "type": "custom_field", "_advanced": true}"#),
            ("cf_pick", r#"{"value": [99999999999999999999999], "condition": "cf_pick", "operator": "is_in", "type": "custom_field"}"#),
        ] {
            let mut explain = Vec::new();
            let (item, ()) =
                resolve_where(None, "tickets", &sch, &format!("{expr}={big}"), &mut explain).unwrap();
            assert_eq!(fsv_py::dumps(&item, &fsv_py::Dumps::default()), want, "{expr}");
        }
    }
}
