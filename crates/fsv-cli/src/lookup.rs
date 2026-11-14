//! Schema field lookup and person/group resolution (`cli.py` helpers).

use serde_json::{Map, Value, json};

use fsv_api::agent_lookup;
use fsv_core::resource::Resource;
use fsv_core::safety::{self, RiskContext};
use fsv_core::term::{self, err};
use fsv_core::{Client, Error, Result, pystr};

use crate::util::{interactive, usage};

/// `str(d.get(k))`: `"None"` when absent.
pub fn s(v: &Value, key: &str) -> String {
    v.get(key).map_or_else(|| "None".into(), fsv_py::py_str)
}

/// `str(d.get(k) or "")`.
pub fn s_or(v: &Value, key: &str) -> String {
    v.get(key)
        .filter(|x| fsv_py::truthy(x))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

fn repr(s: &str) -> String {
    fsv_py::py_repr_str(s)
}

pub fn field_scope(f: &Value) -> &'static str {
    if f.get("default_field").is_some_and(fsv_py::truthy) {
        "default"
    } else {
        "custom"
    }
}

fn field_api_type(f: &Value) -> &'static str {
    if f.get("default_field").is_some_and(fsv_py::truthy) {
        "default"
    } else {
        "custom_field"
    }
}

/// `_field_matches`.
pub fn field_matches(f: &Value, text: &str) -> bool {
    let needle = pystr::casefold(text);
    pystr::casefold(&s_or(f, "name")).contains(&needle)
        || pystr::casefold(&s_or(f, "label")).contains(&needle)
}

fn format_field_ref(f: &Value) -> String {
    format!(
        "{}  {}  {}  {}",
        s(f, "name"),
        s(f, "label"),
        s(f, "field_type"),
        field_scope(f)
    )
}

pub fn schema_fields(sch: &Value) -> Vec<Value> {
    crate::schema::fields(sch)
}

/// `_find_schema_field`: `Ok(None)` when nothing matches; ambiguity exits 2.
pub fn find_schema_field(sch: &Value, text: &str) -> Result<Option<Value>> {
    let (mut scope, mut raw) = (None, text);
    if let Some((maybe, rest)) = text.split_once(':')
        && (maybe == "default" || maybe == "custom")
    {
        scope = Some(maybe);
        raw = rest;
    }
    let folded = pystr::casefold(raw);
    let matches: Vec<Value> = schema_fields(sch)
        .into_iter()
        .filter(|f| match scope {
            Some("default") => f.get("default_field").is_some_and(fsv_py::truthy),
            Some(_) => !f.get("default_field").is_some_and(fsv_py::truthy),
            None => true,
        })
        .filter(|f| field_matches(f, raw))
        .collect();
    let name_is = |f: &Value| pystr::casefold(&s_or(f, "name")) == folded;
    let exact: Vec<&Value> = matches
        .iter()
        .filter(|f| name_is(f) || pystr::casefold(&s_or(f, "label")) == folded)
        .collect();
    let ambiguous = |list: &[&Value]| {
        let lines: Vec<String> = list
            .iter()
            .take(10)
            .map(|f| format!("  {}", format_field_ref(f)))
            .collect();
        usage(&format!(
            "ambiguous field {}:\n{}",
            repr(text),
            lines.join("\n")
        ))
    };
    if exact.len() == 1 {
        return Ok(Some(exact[0].clone()));
    }
    if exact.len() > 1 {
        let by_name: Vec<&&Value> = exact.iter().filter(|f| name_is(f)).collect();
        if by_name.len() == 1 {
            return Ok(Some((*by_name[0]).clone()));
        }
        return Err(ambiguous(&exact));
    }
    if matches.len() == 1 {
        return Ok(Some(matches[0].clone()));
    }
    if matches.len() > 1 {
        let all: Vec<&Value> = matches.iter().collect();
        return Err(ambiguous(&all));
    }
    Ok(None)
}

/// `_candidate_label`.
fn candidate_label(item: &Value) -> String {
    let name = ["value", "name"]
        .iter()
        .find_map(|k| item.get(*k).filter(|v| fsv_py::truthy(v)))
        .or_else(|| item.get("id"))
        .map_or_else(|| "None".into(), fsv_py::py_str);
    let extra = ["details", "email"]
        .iter()
        .find_map(|k| item.get(*k).filter(|v| fsv_py::truthy(v)))
        .map(fsv_py::py_str);
    match extra {
        Some(e) if e != name => format!("{name} <{e}>"),
        _ => name,
    }
}

/// `_pick_lookup`.
pub fn pick_lookup(
    kind: &str,
    value: &str,
    results: &[Value],
    exact_keys: &[&str],
) -> Result<Value> {
    if results.is_empty() {
        return Err(usage(&format!("no {kind} match: {value}")));
    }
    let folded = safety::norm(value);
    let exact: Vec<&Value> = results
        .iter()
        .filter(|x| {
            exact_keys
                .iter()
                .any(|k| safety::norm_value(x.get(*k).unwrap_or(&Value::Null)) == folded)
        })
        .collect();
    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    let candidates: Vec<&Value> = if exact.is_empty() {
        results.iter().collect()
    } else {
        exact.clone()
    };
    let problem = if exact.is_empty() {
        format!("no exact {kind} match for {}", repr(value))
    } else {
        format!("multiple exact {kind} matches for {}", repr(value))
    };
    if interactive() {
        let shown = &candidates[..candidates.len().min(20)];
        err().print(&format!("{problem}; candidates:"));
        for (i, item) in shown.iter().enumerate() {
            err().print(&format!("  {}. {}", i + 1, candidate_label(item)));
        }
        let choice = term::prompt_int("Pick a number (0 to cancel)", Some(0), true)?;
        if choice >= 1 && (choice as usize) <= shown.len() {
            return Ok(shown[choice as usize - 1].clone());
        }
        return Err(usage(&format!("no {kind} chosen")));
    }
    let listed: Vec<String> = candidates
        .iter()
        .take(8)
        .map(|x| candidate_label(x))
        .collect();
    let more = if candidates.len() > 8 {
        format!(" (+{} more)", candidates.len() - 8)
    } else {
        String::new()
    };
    Err(usage(&format!(
        "{problem}: {}{more}. Use the exact name/email or a numeric ID.",
        listed.join("; ")
    )))
}

/// `str.isdigit()` for decimal digits. `१२` counts; `²` does not (`int` rejects it).
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| pystr::decimal_value(c).is_some())
}

/// `int(row[key])`, failing with Python's `KeyError`/`ValueError`/`TypeError` text.
fn py_int(row: &Value, key: &str) -> Result<i64> {
    let Some(v) = row.get(key) else {
        return Err(Error::Other(repr(key)));
    };
    let kind = match v {
        Value::Number(n) => {
            return n
                .as_i64()
                .or_else(|| {
                    n.as_f64()
                        .filter(|f| f.is_finite())
                        .map(|f| f.trunc() as i64)
                })
                .ok_or_else(|| Error::Other(format!("cannot convert float {n} to integer")));
        }
        Value::Bool(b) => return Ok(i64::from(*b)),
        Value::String(s) => {
            // int() skips Unicode White_Space only, not str.isspace()'s U+001C..U+001F.
            let t = s.trim();
            let (neg, digits) = match t.strip_prefix('-') {
                Some(d) => (true, d),
                None => (false, t.strip_prefix('+').unwrap_or(t)),
            };
            let grouped =
                !digits.starts_with('_') && !digits.ends_with('_') && !digits.contains("__");
            return pystr::parse_decimal(&digits.replace('_', ""))
                .filter(|_| !digits.is_empty() && grouped)
                .map(|n| if neg { -n } else { n })
                .ok_or_else(|| {
                    Error::value(format!(
                        "invalid literal for int() with base 10: {}",
                        repr(s)
                    ))
                });
        }
        Value::Null => "NoneType",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    };
    Err(Error::Other(format!(
        "int() argument must be a string, a bytes-like object or a real number, not '{kind}'"
    )))
}

/// `_resolve_requester`: `(id, name)`.
pub fn resolve_requester(c: &Client, value: &str) -> Result<(Value, String)> {
    if is_digits(value) {
        return Ok((int_id(value), value.to_string()));
    }
    let params: Map<String, Value> = [("all_users".to_string(), json!("true"))]
        .into_iter()
        .collect();
    let rows = c.autocomplete("requesters", value, Some(params))?;
    let item = pick_lookup("requester", value, &rows, &["value", "email", "details"])?;
    let id = item.get("id").cloned().unwrap_or(Value::Null);
    let name = item
        .get("value")
        .filter(|v| fsv_py::truthy(v))
        .map_or_else(|| fsv_py::py_str(&id), fsv_py::py_str);
    Ok((id, name))
}

/// `_resolve_agent`: the agent's user id as text.
pub fn resolve_agent(c: &Client, value: &str) -> Result<String> {
    if is_digits(value) {
        return Ok(value.to_string());
    }
    if agent_lookup::is_email(value) {
        let exact = c
            .autocomplete("agents", value, None)
            .map_err(|_| Error::value("could not resolve agent; lookup unavailable"))
            .and_then(|rows| agent_lookup::exact_agent_id(&rows, value, false));
        return match exact {
            Ok(id) => Ok(id.to_string()),
            Err(e) => Err(usage(&e.to_string())),
        };
    }
    let rows = c.autocomplete("agents", value, None)?;
    let item = pick_lookup("agent", value, &rows, &["value", "id", "email", "details"])?;
    match item.get("user_id").filter(|v| !v.is_null()) {
        Some(u) => Ok(fsv_py::py_str(u)),
        None => Err(usage(&format!("agent match has no user_id: {value}"))),
    }
}

/// `_resolve_group`: the group id as text.
pub fn resolve_group(c: &Client, value: &str) -> Result<String> {
    if is_digits(value) {
        return Ok(value.to_string());
    }
    let data = c.int_get("bootstrap/agents_groups", None)?;
    let folded = safety::norm(value);
    let groups: Vec<Value> = data
        .get("groups")
        .and_then(Value::as_array)
        .map(|gs| {
            gs.iter()
                .filter(|g| {
                    safety::norm_value(g.get("name").unwrap_or(&Value::Null)).contains(&folded)
                        || s_or(g, "id") == value
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let item = pick_lookup("group", value, &groups, &["name", "id"])?;
    match item.get("id").filter(|v| !v.is_null()) {
        Some(g) => Ok(fsv_py::py_str(g)),
        None => Err(usage(&format!("group match has no id: {value}"))),
    }
}

/// `int(text)` of a resolved id.
pub fn int_id(text: &str) -> Value {
    pystr::int_json(text).unwrap_or_else(|| json!(text))
}

/// `int(raw)` of a `str.isdigit()` value: Python's `ValueError` text for a
/// digit `int()` rejects (`²`).
pub(crate) fn int_literal(raw: &str) -> Result<Value> {
    pystr::int_json(raw).ok_or_else(|| {
        Error::value(format!(
            "invalid literal for int() with base 10: {}",
            repr(raw)
        ))
    })
}

/// `_resolve_lookup_user`.
fn resolve_lookup_user(c: &Client, value: &str, link: Option<&str>) -> Result<Value> {
    if is_digits(value) {
        return Ok(int_id(value));
    }
    let names = |rows: &[Value], keys: &[&str], id_key: &str| -> String {
        rows.iter()
            .take(5)
            .map(|r| {
                keys.iter()
                    .find_map(|k| r.get(*k).filter(|v| fsv_py::truthy(v)))
                    .or_else(|| r.get(id_key))
                    .map_or_else(|| "None".into(), fsv_py::py_str)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(link) = link.filter(|l| !l.is_empty()) {
        let rows = c.lookup_choices(link, value)?;
        if rows.len() == 1 {
            return Ok(json!(py_int(&rows[0], "id")?));
        }
        if rows.len() > 1 {
            return Err(usage(&format!(
                "ambiguous user {}: {}",
                repr(value),
                names(&rows, &["name", "value"], "id")
            )));
        }
        return Err(usage(&format!("user not found: {}", repr(value))));
    }
    let agents: Vec<Value> = c
        .autocomplete("agents", value, None)?
        .into_iter()
        .filter(|r| {
            !s_or(r, "value").contains("(Deactivated)")
                && r.get("user_id").is_some_and(fsv_py::truthy)
        })
        .collect();
    if agents.len() == 1 {
        return Ok(json!(py_int(&agents[0], "user_id")?));
    }
    if agents.len() > 1 {
        return Err(usage(&format!(
            "ambiguous user {}: {}",
            repr(value),
            names(&agents, &["value"], "user_id")
        )));
    }
    let params: Map<String, Value> = [("all_users".to_string(), json!("true"))]
        .into_iter()
        .collect();
    let active: Vec<Value> = c
        .autocomplete("requesters", value, Some(params))?
        .into_iter()
        .filter(|r| !r.get("deleted").is_some_and(fsv_py::truthy))
        .collect();
    if active.len() == 1 {
        return Ok(json!(py_int(&active[0], "id")?));
    }
    if active.len() > 1 {
        return Err(usage(&format!(
            "ambiguous user {}: {}",
            repr(value),
            names(&active, &["value"], "id")
        )));
    }
    Err(usage(&format!("user not found: {}", repr(value))))
}

/// `_resolve_update_choice`.
pub fn resolve_update_choice(sch: &Value, field_name: &str, raw: &str) -> Result<Value> {
    let f = find_schema_field(sch, field_name)?;
    let choices: Vec<Value> = f
        .as_ref()
        .and_then(|f| f.get("choices"))
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .filter(|c| {
                    c.is_object()
                        && !c.get("deleted").is_some_and(fsv_py::truthy)
                        && !c.get("is_deleted").is_some_and(fsv_py::truthy)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let matches: Vec<&Value> = if is_digits(raw) {
        choices.iter().filter(|c| s(c, "id") == raw).collect()
    } else {
        let folded = pystr::casefold(raw);
        choices
            .iter()
            .filter(|c| {
                ["value", "name", "label"]
                    .iter()
                    .any(|k| pystr::casefold(&s_or(c, k)) == folded)
            })
            .collect()
    };
    if matches.len() > 1 {
        return Err(usage(&format!(
            "ambiguous {field_name}: {}; select a unique active ID",
            repr(raw)
        )));
    }
    if matches.len() == 1
        && let Some(id) = matches[0].get("id").filter(|v| !v.is_null())
    {
        return Ok(id.clone());
    }
    let labels: Vec<String> = choices.iter().map(choice_text).collect();
    let suffix = if labels.is_empty() {
        "; active choice metadata unavailable".to_string()
    } else {
        format!("; choices: {}", labels[..labels.len().min(12)].join(", "))
    };
    Err(usage(&format!(
        "unknown {field_name}: {}{suffix}",
        repr(raw)
    )))
}

/// `str(c.get("value") or c.get("name") or c.get("label") or c.get("id"))`.
fn choice_text(c: &Value) -> String {
    ["value", "name", "label"]
        .iter()
        .find_map(|k| c.get(*k).filter(|v| fsv_py::truthy(v)))
        .or_else(|| c.get("id"))
        .map_or_else(|| "None".into(), fsv_py::py_str)
}

/// `_status_label`.
pub fn status_label(sch: &Value, status: &Value) -> String {
    let label = fsv_api::choice_label("status", status, sch);
    if label != "-" {
        label
    } else {
        fsv_py::py_str(status)
    }
}

/// `_choice_value`.
pub(crate) fn choice_value(field: &Value, raw: &str, type_: &str) -> Result<Value> {
    let choices = field
        .get("choices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if choices.is_empty() {
        return if pystr::is_digit(raw) {
            int_literal(raw)
        } else {
            Ok(json!(raw))
        };
    }
    let folded = pystr::casefold(raw);
    let get_or = |c: &Value, k: &str, d: Value| c.get(k).cloned().unwrap_or(d);
    for c in &choices {
        if s_or(c, "id") == raw {
            return Ok(if type_ == "custom_field" {
                get_or(c, "value", get_or(c, "id", Value::Null))
            } else {
                get_or(c, "id", Value::Null)
            });
        }
        let labels = ["value", "name", "label", "requester_display_value"];
        if labels
            .iter()
            .map(|k| s_or(c, k))
            .chain(std::iter::once(fsv_api::humanize_value(c.get("value"))))
            .any(|x| pystr::casefold(&x) == folded)
        {
            return Ok(if type_ == "custom_field" {
                get_or(c, "value", get_or(c, "id", Value::Null))
            } else {
                get_or(c, "id", get_or(c, "value", json!(raw)))
            });
        }
    }
    let shown: Vec<String> = choices
        .iter()
        .take(12)
        .map(|c| {
            ["value", "name"]
                .iter()
                .find_map(|k| c.get(*k).filter(|v| fsv_py::truthy(v)))
                .or_else(|| c.get("id"))
                .map_or_else(|| "None".into(), fsv_py::py_str)
        })
        .collect();
    let label = field
        .get("label")
        .filter(|v| fsv_py::truthy(v))
        .or_else(|| field.get("name"))
        .map_or_else(|| "None".into(), fsv_py::py_str);
    Err(usage(&format!(
        "no choice {} for {label}; choices: {}",
        repr(raw),
        shown.join(", ")
    )))
}

/// `_resolve_set_field`: `(api key, value)`.
pub fn resolve_set_field(
    c: &Client,
    sch: &Value,
    field_text: &str,
    raw_value: &str,
) -> Result<(String, Value)> {
    let Some(f) = find_schema_field(sch, field_text)? else {
        return Err(usage(&format!(
            "field not found: {}; run `fsv changes fields`",
            repr(field_text)
        )));
    };
    let name = f
        .get("name")
        .filter(|v| fsv_py::truthy(v))
        .map_or_else(|| field_text.to_string(), fsv_py::py_str);
    let ftype = s_or(&f, "field_type");
    let type_ = field_api_type(&f);
    if ftype == "default_agent" || name == "agent" {
        return Ok(("agent_id".into(), int_id(&resolve_agent(c, raw_value)?)));
    }
    if ftype == "default_group" || name == "group" {
        return Ok(("group_id".into(), int_id(&resolve_group(c, raw_value)?)));
    }
    if ftype == "default_requester" || name == "requester" {
        return Ok(("requester_id".into(), resolve_requester(c, raw_value)?.0));
    }
    let choices = f
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|c| !c.is_empty());
    if ftype == "default_department" || name == "department" {
        if choices {
            return Ok((
                "department_id".into(),
                choice_value(&f, raw_value, "default")?,
            ));
        }
        let v = if is_digits(raw_value) {
            int_id(raw_value)
        } else {
            json!(raw_value)
        };
        return Ok(("department_id".into(), v));
    }
    if ftype.contains("lookup") {
        let link = f
            .get("lookup_config")
            .and_then(|l| l.get("link"))
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str);
        if ftype.contains("multi") {
            let mut out = Vec::new();
            for p in raw_value
                .split(',')
                .map(pystr::strip)
                .filter(|p| !p.is_empty())
            {
                out.push(resolve_lookup_user(c, p, link.as_deref())?);
            }
            return Ok((name, Value::Array(out)));
        }
        return Ok((name, resolve_lookup_user(c, raw_value, link.as_deref())?));
    }
    if ftype.contains("checkbox") {
        let value = pystr::casefold(pystr::strip(raw_value));
        if !["true", "1", "yes", "on", "false", "0", "no", "off"].contains(&value.as_str()) {
            return Err(usage(&format!(
                "invalid boolean for {}: {}",
                repr(field_text),
                repr(raw_value)
            )));
        }
        return Ok((
            name,
            json!(["true", "1", "yes", "on"].contains(&value.as_str())),
        ));
    }
    if ftype.contains("multi_select") {
        let mut out = Vec::new();
        for p in raw_value
            .split(',')
            .map(pystr::strip)
            .filter(|p| !p.is_empty())
        {
            out.push(choice_value(&f, p, type_)?);
        }
        return Ok((name, Value::Array(out)));
    }
    if (name == "status" || name == "priority")
        && (ftype == "default_status" || ftype == "default_priority" || type_ != "custom_field")
    {
        let v = resolve_update_choice(sch, &name, raw_value)?;
        return Ok((name, v));
    }
    if choices {
        let v = choice_value(&f, raw_value, type_)?;
        return Ok((name, v));
    }
    Ok((name, json!(raw_value)))
}

/// `_current_flat`: the stored item, best effort.
pub fn current_flat(res: &Resource, cid: i64, c: &Client) -> Option<Map<String, Value>> {
    let data = c.int_get(&format!("{}/{cid}", res.api_path), None).ok()?;
    let item = match data.get(res.item_key) {
        Some(inner) => inner.clone(),
        None => data,
    };
    item.as_object().cloned()
}

/// `_risks_for_item_write`.
pub fn risks_for_item_write(
    res: &Resource,
    cid: i64,
    body: &Map<String, Value>,
    sch: &Value,
    c: &Client,
    current: Option<Map<String, Value>>,
) -> Vec<String> {
    let current = current.or_else(|| {
        ["status", "agent_id", "responder_id", "group_id"]
            .iter()
            .any(|k| body.contains_key(*k))
            .then(|| current_flat(res, cid, c))
            .flatten()
    });
    let label = |v: &Value| status_label(sch, v);
    safety::write_risks(
        res.name,
        body,
        &RiskContext {
            current: current.as_ref(),
            status_label: Some(&label),
            kind: "item",
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values from `cli._format_field_ref` and `cli._candidate_label`.
    #[test]
    fn unicode_decimal_ids_become_ints() {
        assert_eq!(int_id("१२"), json!(12));
        assert_eq!(int_id("12"), json!(12));
        assert_eq!(int_id("²"), json!("²"));
        assert_eq!(int_id("-3"), json!(-3));
        assert_eq!(
            fsv_py::py_str(&int_id("99999999999999999999999")),
            "99999999999999999999999"
        );
    }

    /// `_choice_value` with no choices: `int(raw) if raw.isdigit()`, so a
    /// superscript digit is Python's `ValueError`.
    #[test]
    fn choice_value_without_choices_is_python_int() {
        let f = json!({"name": "x"});
        assert_eq!(choice_value(&f, "\u{661}", "default").unwrap(), json!(1));
        assert_eq!(choice_value(&f, "x1", "default").unwrap(), json!("x1"));
        assert_eq!(
            choice_value(&f, "\u{b2}", "default")
                .unwrap_err()
                .to_string(),
            "invalid literal for int() with base 10: '\u{b2}'"
        );
    }

    /// `int(rows[0]["id"])` in `_resolve_lookup_user`: a bad id raises
    /// instead of becoming 0.
    #[test]
    fn lookup_user_id_is_python_int() {
        let ok = [
            (json!({"id": 7}), 7),
            (json!({"id": "\u{a0}12\u{3000}"}), 12),
            (json!({"id": " 1_0 "}), 10),
            (json!({"id": "-3"}), -3),
            (json!({"id": 4.9}), 4),
            (json!({"id": true}), 1),
        ];
        for (row, want) in ok {
            assert_eq!(py_int(&row, "id").unwrap(), want, "{row}");
        }
        let bad = [
            (
                json!({"id": "abc"}),
                "invalid literal for int() with base 10: 'abc'",
            ),
            (
                json!({"id": ""}),
                "invalid literal for int() with base 10: ''",
            ),
            (
                json!({"id": "\u{1c}12"}),
                "invalid literal for int() with base 10: '\\x1c12'",
            ),
            (
                json!({"id": null}),
                "int() argument must be a string, a bytes-like object or a real number, not 'NoneType'",
            ),
            (json!({}), "'id'"),
        ];
        for (row, want) in bad {
            assert_eq!(py_int(&row, "id").unwrap_err().to_string(), want, "{row}");
        }
    }

    #[test]
    fn format_field_ref_columns() {
        let f = json!({"name": "cf_x", "label": "X", "field_type": "custom_dropdown", "default_field": false});
        assert_eq!(format_field_ref(&f), "cf_x  X  custom_dropdown  custom");
        assert_eq!(
            format_field_ref(&json!({"name": "status"})),
            "status  None  None  custom"
        );
    }

    #[test]
    fn candidate_labels() {
        let got: Vec<String> = [
            json!({"value": "Ann", "email": "a@x", "details": "d"}),
            json!({"id": 3}),
            json!({"name": "N", "id": 4}),
            json!({"value": "V", "email": "V"}),
        ]
        .iter()
        .map(candidate_label)
        .collect();
        assert_eq!(got, ["Ann <d>", "3", "N", "V"]);
    }

    /// Values from `cli._resolve_update_choice`; failures exit 2.
    #[test]
    fn resolve_update_choice_active_only() {
        let sch = json!({"fields": [
            {"name": "status", "label": "Status", "choices": [
                {"id": 2, "value": "Open"}, {"id": 3, "value": "Old", "deleted": true},
                {"id": 4, "name": "Pending"}, {"id": 5, "value": "Dup"}, {"id": 6, "label": "dup"},
            ]},
            {"name": "empty", "label": "Empty", "choices": []},
        ]});
        for (field, raw, want) in [
            ("status", "open", json!(2)),
            ("status", "2", json!(2)),
            ("status", "PENDING", json!(4)),
        ] {
            assert_eq!(
                resolve_update_choice(&sch, field, raw).unwrap(),
                want,
                "{raw}"
            );
        }
        for (field, raw) in [
            ("status", "3"),
            ("status", "Old"),
            ("status", "dup"),
            ("empty", "x"),
            ("nosuch", "x"),
        ] {
            let e = resolve_update_choice(&sch, field, raw).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{field}={raw}");
        }
    }
}
