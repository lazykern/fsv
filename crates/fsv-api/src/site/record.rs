//! Header, summary card, conversations and tab resolution (`views.py`).
//! No HTTP. Callers pass the record, schema and an optional "now".

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

use fsv_core::pystr;
use fsv_core::resource::Resource;

use crate::QueryError;
use crate::schema::py_eq;
use crate::site::format::{fmt_when, html_to_text, parse_dt, relative, span};
use crate::site::list::{
    Tab, canonical_tab, is_known_tab, priority_name, py_capitalize, requester_name, schema_label,
    status_name, tab_label,
};

const SOURCES: &[(i64, &str)] = &[
    (1, "Email"),
    (2, "Portal"),
    (3, "Phone"),
    (4, "Chat"),
    (5, "Feedback Widget"),
    (6, "Yammer"),
    (7, "AWS CloudWatch"),
    (8, "PagerDuty"),
    (9, "Walk-up"),
    (10, "Slack"),
];

const APPROVAL_STATUS: &[(i64, &str)] = &[
    (1, "Approved"),
    (2, "Rejected"),
    (3, "Requested"),
    (4, "Not requested"),
    (5, "Cancelled"),
];

/// Drop duplicate ids, then oldest `created_at` first. Rows with no id are kept.
pub fn oldest_first(rows: &[Value]) -> Vec<Value> {
    let mut seen: Vec<Value> = Vec::new();
    let mut uniq = Vec::new();
    for row in rows {
        let id = row.get("id");
        if let Some(id) = id.filter(|v| !v.is_null())
            && seen.iter().any(|s| py_eq(s, id))
        {
            continue;
        }
        if let Some(id) = id {
            seen.push(id.clone());
        }
        uniq.push(row.clone());
    }
    uniq.sort_by_key(sort_key);
    uniq
}

fn sort_key(row: &Value) -> String {
    row.get("created_at")
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

/// `private note` / `public note` / `replied`.
pub fn conversation_kind(conv: &Value) -> &'static str {
    if conv.get("private").is_some_and(fsv_py::truthy) {
        return "private note";
    }
    if conv.get("source").is_some_and(|v| py_eq(v, &json!(2))) {
        return "public note";
    }
    "replied"
}

pub fn source_name(record: &Value, schema: Option<&Value>) -> String {
    let Some(v) = record
        .get("source")
        .filter(|v| !v.is_null() && v.as_str() != Some(""))
    else {
        return String::new();
    };
    let named = crate::site::list::choice_name(schema, "source", Some(v));
    if !named.is_empty() {
        return named;
    }
    if let Some(n) = v.as_i64()
        && let Some((_, label)) = SOURCES.iter().find(|(k, _)| *k == n)
    {
        return (*label).to_string();
    }
    fsv_py::py_str(v)
}

/// Status text, with ` (late)` when a ticket was resolved after `due_by`.
pub fn status_badge(res: &Resource, record: &Value, schema: Option<&Value>) -> String {
    let mut name = status_name(record, schema);
    if res.name == "tickets" {
        let stats = record.get("stats").filter(|v| fsv_py::truthy(v));
        let done = stats
            .and_then(|s| {
                s.get("resolved_at")
                    .filter(|v| fsv_py::truthy(v))
                    .or_else(|| s.get("closed_at").filter(|v| fsv_py::truthy(v)))
            })
            .and_then(parse_dt);
        let due = record.get("due_by").and_then(parse_dt);
        if let (Some(done), Some(due)) = (done, due)
            && done > due
        {
            name.push_str(" (late)");
        }
    }
    name
}

pub fn approval_name(record: &Value) -> String {
    if let Some(name) = record
        .get("approval_status_name")
        .filter(|v| fsv_py::truthy(v))
    {
        return fsv_py::py_str(name);
    }
    let Some(v) = record
        .get("approval_status")
        .filter(|v| !v.is_null() && v.as_str() != Some(""))
    else {
        return String::new();
    };
    if let Some(n) = v.as_i64()
        && let Some((_, label)) = APPROVAL_STATUS.iter().find(|(k, _)| *k == n)
    {
        return (*label).to_string();
    }
    fsv_py::py_str(v)
}

pub fn description_text(record: &Value) -> String {
    let html = record
        .get("description")
        .filter(|v| fsv_py::truthy(v))
        .cloned()
        .unwrap_or(Value::String(String::new()));
    let text = html_to_text(&html);
    if !text.is_empty() {
        return text;
    }
    record
        .get("description_text")
        .filter(|v| fsv_py::truthy(v))
        .map(|v| pystr::strip(&fsv_py::py_str(v)).to_string())
        .unwrap_or_default()
}

pub fn attachments(record: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for key in ["attachments", "cloud_files"] {
        let Some(items) = record
            .get(key)
            .filter(|v| fsv_py::truthy(v))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for a in items.iter().filter_map(Value::as_object) {
            let name = ["name", "content_file_name", "filename", "id"]
                .iter()
                .find_map(|k| a.get(*k).filter(|v| fsv_py::truthy(v)))
                .map(fsv_py::py_str)
                .unwrap_or_else(|| "None".to_string());
            let size = a
                .get("size")
                .filter(|v| fsv_py::truthy(v))
                .or_else(|| a.get("content_file_size").filter(|v| fsv_py::truthy(v)))
                .cloned()
                .unwrap_or(Value::Null);
            out.push(json!({"name": name, "size": size}));
        }
    }
    out
}

pub fn conversation_json(conv: &Value, _tz: Option<&TimeZone>) -> Value {
    let author = crate::site::list::obj_name(conv.get("user"));
    let author = if author.is_empty() {
        conv.get("user_id")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default()
    } else {
        author
    };
    let body_html = conv
        .get("body")
        .filter(|v| fsv_py::truthy(v))
        .cloned()
        .unwrap_or(Value::String(String::new()));
    let body = html_to_text(&body_html);
    let body = if body.is_empty() {
        conv.get("body_text")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default()
    } else {
        body
    };
    json!({
        "id": conv.get("id").cloned().unwrap_or(Value::Null),
        "author": author,
        "kind": conversation_kind(conv),
        "private": conv.get("private").is_some_and(fsv_py::truthy),
        "created_at": conv.get("created_at").cloned().unwrap_or(Value::Null),
        "relative": relative(conv.get("created_at").unwrap_or(&Value::Null), None),
        "body": body,
        "attachments": file_names(conv.get("attachments")),
    })
}

fn file_names(rows: Option<&Value>) -> Vec<String> {
    rows.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_object)
                .map(|a| {
                    ["name", "content_file_name", "file_name"]
                        .iter()
                        .find_map(|k| a.get(*k).filter(|v| fsv_py::truthy(v)))
                        .or_else(|| a.get("id"))
                        .map_or_else(|| "None".to_string(), fsv_py::py_str)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Right-hand summary card. `now` is the clock `span` counts down from.
pub fn summary(
    res: &Resource,
    record: &Value,
    schema: Option<&Value>,
    tz: Option<&TimeZone>,
    now: Timestamp,
) -> Vec<(String, String)> {
    let mut rows = vec![(
        schema_label(schema, "status").unwrap_or_else(|| "Status".to_string()),
        status_badge(res, record, schema),
    )];
    let pri = priority_name(record, schema);
    if !pri.is_empty() {
        rows.push((
            schema_label(schema, "priority").unwrap_or_else(|| "Priority".to_string()),
            pri,
        ));
    }
    if let Some(due) = record.get("due_by").and_then(parse_dt) {
        let left = (due.as_microsecond() - now.as_microsecond()) as f64 / 1_000_000.0;
        let mut remain = if left > 0.0 {
            format!("in {}", span(Some(left)))
        } else {
            format!("overdue by {}", span(Some(left)))
        };
        let stats = record.get("stats").filter(|v| fsv_py::truthy(v));
        let closed = stats.is_some_and(|s| {
            s.get("resolved_at").is_some_and(fsv_py::truthy)
                || s.get("closed_at").is_some_and(fsv_py::truthy)
        });
        let terminal = res.name == "tickets"
            && record
                .get("status")
                .is_some_and(|s| py_eq(s, &json!(5)) || py_eq(s, &json!(6)));
        if closed || terminal {
            remain.clear();
        }
        let label = if res.name == "changes" {
            "Resolution due by"
        } else {
            "Due by"
        };
        let when = fmt_when(record.get("due_by").unwrap_or(&Value::Null), tz, true);
        let value = if remain.is_empty() {
            when
        } else {
            format!("{when}  ({remain})")
        };
        rows.push((label.to_string(), value));
    }
    if res.name == "changes" {
        let appr = approval_name(record);
        if !appr.is_empty() {
            rows.push(("Approval".to_string(), appr));
        }
    }
    for key in ["planned_start_date", "planned_end_date"] {
        if record.get(key).is_some_and(fsv_py::truthy) {
            let label =
                schema_label(schema, key).unwrap_or_else(|| py_capitalize(&key.replace('_', " ")));
            rows.push((label, fmt_when(record.get(key).unwrap(), tz, true)));
        }
    }
    if res.name == "tickets" {
        if let Some(secs) = record
            .get("stats")
            .filter(|v| fsv_py::truthy(v))
            .and_then(|s| s.get("resolution_time_in_secs"))
            .filter(|v| fsv_py::truthy(v))
            .and_then(Value::as_f64)
        {
            rows.push(("Time taken".to_string(), span(Some(secs))));
        }
        if let Some(title) = record
            .get("requester")
            .and_then(Value::as_object)
            .and_then(|r| r.get("job_title"))
            .filter(|v| fsv_py::truthy(v))
        {
            rows.push(("Requester title".to_string(), fsv_py::py_str(title)));
        }
    }
    rows
}

/// Canonical tab the server lists, or a known tab when the list itself is missing.
pub fn resolve_tab(
    tabs: &[Tab],
    text: &str,
    id: &str,
    tabs_missing: bool,
) -> Result<String, QueryError> {
    let name = canonical_tab(text);
    let names: Vec<&str> = tabs.iter().map(|t| t.name.as_str()).collect();
    if names.contains(&name.as_str()) {
        return Ok(name);
    }
    let folded = pystr::casefold(text.trim());
    let matches: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| n.starts_with(&name) || pystr::casefold(&tab_label(n)).starts_with(&folded))
        .collect();
    if matches.len() == 1 {
        return Ok(matches[0].to_string());
    }
    if tabs_missing && is_known_tab(&name) {
        return Ok(name);
    }
    let shown = names.join(", ");
    Err(QueryError(format!(
        "tab {} is not shown for {id}; tabs: {shown}",
        fsv_py::py_repr_str(text)
    )))
}

/// Reporter line. `request_for` is already resolved (`Self`, a name, or `#id`).
pub fn reporter(
    res: &Resource,
    record: &Value,
    schema: Option<&Value>,
    tz: Option<&TimeZone>,
    request_for: Option<&str>,
) -> Map<String, Value> {
    let mut name = requester_name(record);
    if name.is_empty() {
        name = record
            .get("created_by_name")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default();
    }
    let created = record.get("created_at").cloned().unwrap_or(Value::Null);
    let mut out = Map::new();
    out.insert("name".into(), Value::String(name));
    out.insert("created_at".into(), created.clone());
    out.insert("relative".into(), Value::String(relative(&created, None)));
    out.insert("when".into(), Value::String(fmt_when(&created, tz, false)));
    if res.name == "tickets" {
        out.insert("via".into(), Value::String(source_name(record, schema)));
        if let Some(rf) = request_for {
            out.insert("request_for".into(), Value::String(rf.to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fsv_core::resource::{CHANGES, TICKETS};
    use serde_json::json;

    #[test]
    fn file_names_and_conversation_json_match_python() {
        let o: Value =
            serde_json::from_str(include_str!("../testdata/coverage_oracle.json")).unwrap();
        for case in o["views"]["file_names"].as_array().unwrap() {
            let rows = (!case[0].is_null()).then_some(&case[0]);
            assert_eq!(json!(file_names(rows)), case[1], "{}", case[0]);
        }
        for case in o["views"]["conversation_json"].as_array().unwrap() {
            assert_eq!(conversation_json(&case[0], None), case[1], "{}", case[0]);
        }
    }

    #[test]
    fn conversations_dedup_and_kind() {
        let rows = oldest_first(&[
            json!({"id": 2, "created_at": "2026-02-01T00:00:00Z", "private": true}),
            json!({"id": 1, "created_at": "2026-01-01T00:00:00Z", "source": 2}),
            json!({"id": 2, "created_at": "2026-03-01T00:00:00Z"}),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], json!(1));
        assert_eq!(conversation_kind(&rows[0]), "public note");
        assert_eq!(conversation_kind(&rows[1]), "private note");
        assert_eq!(conversation_kind(&json!({"incoming": true})), "replied");
    }

    #[test]
    fn source_and_approval_fallbacks() {
        assert_eq!(source_name(&json!({"source": 3}), None), "Phone");
        assert_eq!(source_name(&json!({"source": ""}), None), "");
        assert_eq!(
            approval_name(&json!({"approval_status": 4})),
            "Not requested"
        );
        assert_eq!(
            approval_name(&json!({"approval_status_name": "Pending"})),
            "Pending"
        );
    }

    #[test]
    fn late_ticket_and_tab_resolution() {
        let record = json!({
            "status_name": "Closed",
            "due_by": "2026-01-01T00:00:00Z",
            "stats": {"resolved_at": "2026-01-02T00:00:00Z"}
        });
        assert_eq!(status_badge(&TICKETS, &record, None), "Closed (late)");
        assert_eq!(status_badge(&CHANGES, &record, None), "Closed");
        let tabs = vec![
            Tab {
                name: "details".into(),
                label: "Details".into(),
                count: None,
            },
            Tab {
                name: "notes".into(),
                label: "Notes".into(),
                count: None,
            },
        ];
        assert_eq!(resolve_tab(&tabs, "note", "CHN-1", false).unwrap(), "notes");
        let err = resolve_tab(&tabs, "risk", "CHN-1", false).unwrap_err().0;
        assert!(err.contains("not shown"), "{err}");
        assert_eq!(resolve_tab(&tabs, "risk", "CHN-1", true).unwrap(), "risk");
    }
}
