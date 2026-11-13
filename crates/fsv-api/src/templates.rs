//! Ticket templates (read-only) and agent notification entries
//! (`templates.py`).
//!
//! `GET /api/_/ticket-templates` -> `{"ticket_templates": [{id, name, type,
//! workspace_id}], ...}` and `GET /api/_/ticket-templates/<id>` ->
//! `{"ticket_template": {id, name, template_data: {...}}}`. Applying a
//! template only prefills the new-ticket form; every value still goes
//! through the form's fields, business rules and review.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_py::{py_repr_str, py_str, truthy};

use crate::recipients::valid_recipient;
use crate::service::{Api, positive_id};

/// Default `template_data` keys that map 1:1 onto new-ticket form fields.
pub const PREFILL_KEYS: [&str; 9] = [
    "subject",
    "status",
    "source",
    "priority",
    "urgency",
    "impact",
    "category",
    "sub_category",
    "item_category",
];

/// `TemplateError` (a `ValueError`).
fn template_error(msg: impl Into<String>) -> Error {
    Error::value(msg.into())
}

/// Port of fsv.templates.list_templates.
pub fn list_templates(c: &dyn Api) -> Result<Vec<Value>> {
    let raw = c.int_get("ticket-templates", None)?;
    let rows = match raw.get("ticket_templates") {
        Some(Value::Array(rows))
            if rows.iter().all(|r| {
                r.is_object()
                    && positive_id(r.get("id").unwrap_or(&Value::Null)).is_some()
                    && r.get("name").is_some_and(Value::is_string)
            }) =>
        {
            rows.clone()
        }
        _ => {
            return Err(template_error(
                "ticket template list unavailable or malformed",
            ));
        }
    };
    let mut ids: Vec<i64> = rows.iter().filter_map(|r| positive_id(&r["id"])).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != n {
        return Err(template_error("ticket template list has duplicate ids"));
    }
    Ok(rows)
}

fn name_of(r: &Value) -> &str {
    r["name"].as_str().unwrap_or_default()
}

/// Port of fsv.templates.find: exact id, else case-insensitive exact name.
pub fn find(c: &dyn Api, ident: &str) -> Result<Value> {
    let rows = list_templates(c)?;
    let wanted = strip(ident);
    let mut found: Vec<&Value> = rows.iter().filter(|r| py_str(&r["id"]) == wanted).collect();
    if found.is_empty() {
        found = rows
            .iter()
            .filter(|r| casefold(strip(name_of(r))) == casefold(wanted))
            .collect();
    }
    if found.len() != 1 {
        let names = rows
            .iter()
            .take(12)
            .map(name_of)
            .collect::<Vec<_>>()
            .join(", ");
        let names = if names.is_empty() {
            "none".to_string()
        } else {
            names
        };
        return Err(template_error(format!(
            "no unique ticket template {}; templates: {names}",
            py_repr_str(ident)
        )));
    }
    Ok(found[0].clone())
}

/// Port of fsv.templates.show: the exact template with its `template_data`.
pub fn show(c: &dyn Api, ident: &str) -> Result<Value> {
    let tid = positive_id(&find(c, ident)?["id"]);
    let raw = c.int_get(
        &format!("ticket-templates/{}", tid.unwrap_or_default()),
        None,
    )?;
    match raw.get("ticket_template") {
        Some(row @ Value::Object(m))
            if positive_id(m.get("id").unwrap_or(&Value::Null)) == tid
                && m.get("template_data").is_some_and(Value::is_object) =>
        {
            Ok(row.clone())
        }
        _ => Err(template_error("exact ticket template unavailable")),
    }
}

/// What a template prefills (`to_draft`'s `(draft, applied, ignored)`; the
/// CLI builds the `Draft` from `answers` and `description_html`).
#[derive(Debug, Clone, PartialEq)]
pub struct Prefill {
    pub answers: Map<String, Value>,
    pub description_html: Option<String>,
    pub applied: Vec<String>,
    pub ignored: Vec<String>,
}

/// `value in (None, "", [], {})`.
fn blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

fn is_int(v: &Value) -> bool {
    matches!(v, Value::Number(n) if n.is_i64() || n.is_u64())
}

/// Port of fsv.templates.to_draft. Empty values are not prefilled.
pub fn to_draft(template: &Value) -> Prefill {
    let empty = Map::new();
    let data = template["template_data"].as_object().unwrap_or(&empty);
    let mut answers = Map::new();
    let (mut applied, mut ignored) = (Vec::new(), Vec::new());
    for (key, value) in data {
        if blank(value) {
            continue;
        }
        if PREFILL_KEYS.contains(&key.as_str()) && (value.is_string() || is_int(value)) {
            answers.insert(key.clone(), value.clone());
            applied.push(key.clone());
        } else if let (true, Value::Object(custom)) = (key == "custom_fields", value) {
            for (name, item) in custom {
                if blank(item) {
                    continue;
                }
                if item.is_string() || item.is_boolean() || is_int(item) {
                    answers.insert(name.clone(), item.clone());
                    applied.push(name.clone());
                } else {
                    ignored.push(name.clone());
                }
            }
        } else if key != "description" {
            ignored.push(key.clone());
        }
    }
    let description = data
        .get("description")
        .and_then(Value::as_str)
        .filter(|d| !strip(d).is_empty())
        .map(str::to_string);
    if description.is_some() {
        applied.push("description".into());
    }
    ignored.sort();
    Prefill {
        answers,
        description_html: description,
        applied,
        ignored,
    }
}

/// Port of fsv.templates.prefill_changes: after a template create, read the
/// exact ticket back and list every template-prefilled field the server
/// stored differently. Description HTML is not compared.
pub fn prefill_changes(
    c: &dyn Api,
    ticket_id: &Value,
    payload: &Map<String, Value>,
    applied: &[String],
) -> Value {
    let empty = Map::new();
    let custom = payload
        .get("custom_fields")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut sent: Vec<(String, bool, Value)> = Vec::new();
    let scalar = |v: &Value| !v.is_object() && !v.is_array();
    for key in applied {
        if key == "description" {
            continue;
        }
        let entry = if let Some(v) = payload.get(key).filter(|v| scalar(v)) {
            Some((key.clone(), true, v.clone()))
        } else {
            custom
                .get(key)
                .filter(|v| scalar(v))
                .map(|v| (key.clone(), false, v.clone()))
        };
        if let Some(e) = entry {
            match sent.iter_mut().find(|(k, _, _)| *k == e.0) {
                Some(slot) => *slot = e,
                None => sent.push(e),
            }
        }
    }
    let mut names: Vec<String> = sent.iter().map(|(k, _, _)| k.clone()).collect();
    names.sort();
    let row = c
        .int_get(&format!("tickets/{}", py_str(ticket_id)), None)
        .ok()
        .and_then(|raw| match &raw {
            Value::Object(m) => Some(m.get("ticket").cloned().unwrap_or(raw.clone())),
            _ => None,
        })
        .filter(|row| {
            row.is_object() && py_str(row.get("id").unwrap_or(&Value::Null)) == py_str(ticket_id)
        });
    let Some(row) = row else {
        return json!({"checked": false, "changes": [], "fields": names,
                      "warning": "could not read the ticket back to compare template-prefilled fields"});
    };
    let row_custom = row
        .get("custom_fields")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let (mut changes, mut unavailable) = (Vec::new(), Vec::new());
    for (key, top, value) in &sent {
        let source = if *top {
            row.as_object().unwrap_or(&empty)
        } else {
            &row_custom
        };
        match source.get(key) {
            None => unavailable.push(json!(key)),
            Some(got) if py_str(got) != py_str(value) => {
                changes.push(json!({"field": key, "sent": value, "server": got}));
            }
            Some(_) => {}
        }
    }
    json!({"checked": true, "changes": changes, "unavailable": unavailable, "fields": names})
}

/// Port of fsv.templates.notify_entry: `Name <email>` for one active agent
/// from the agent autocomplete (exact name or email only).
pub fn notify_entry(c: &dyn Api, text: &str) -> Result<String> {
    let wanted = casefold(strip(text));
    if wanted.is_empty() {
        return Err(template_error("--notify-agent needs a name or email"));
    }
    let rows = if text.contains('@') {
        c.autocomplete("agents", strip(text), None)?
    } else {
        let mut params = Map::new();
        params.insert("qf".into(), json!("name"));
        c.autocomplete("agents", strip(text), Some(params))?
    };
    let mut found: Vec<(String, String)> = Vec::new();
    for row in &rows {
        let Some(m) = row.as_object() else { continue };
        if m.get("active") == Some(&Value::Bool(false)) || m.get("deleted").is_some_and(truthy) {
            continue;
        }
        let name = m
            .get("value")
            .filter(|v| truthy(v))
            .or_else(|| m.get("name"));
        let emails: Vec<&str> = ["email", "details", "id"]
            .iter()
            .filter_map(|k| m.get(*k).and_then(Value::as_str))
            .filter(|v| v.contains('@'))
            .collect();
        let mut distinct: Vec<String> = emails.iter().map(|e| casefold(e)).collect();
        distinct.sort();
        distinct.dedup();
        let Some(Value::String(name)) = name else {
            continue;
        };
        if distinct.len() != 1 {
            continue;
        }
        let email = emails[0];
        if casefold(strip(name)) == wanted || casefold(email) == wanted {
            let entry = format!("{} <{email}>", strip(name));
            let key = casefold(email);
            match found.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = entry,
                None => found.push((key, entry)),
            }
        }
    }
    if found.len() != 1 {
        return Err(template_error(format!(
            "no unique active agent {} for --notify-agent; use the exact name or email",
            py_repr_str(text)
        )));
    }
    let entry = found.remove(0).1;
    if !valid_recipient(&entry) {
        return Err(template_error(format!(
            "agent entry for {} is not a valid notification address",
            py_repr_str(text)
        )));
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::FakeApi;

    fn templates() -> Value {
        json!({"ticket_templates": [{"id": 501, "name": "Acme outage", "type": "shared", "workspace_id": 3},
                                    {"id": 502, "name": "Globex access", "type": "shared", "workspace_id": 3}],
               "meta": {"max_template_count": 2}})
    }

    fn template() -> Value {
        json!({"ticket_template": {"id": 501, "name": "Acme outage", "template_data": {
            "subject": "Acme outage", "description": "<p>Please help</p>", "status": null, "tags": ["x"],
            "custom_fields": {"stage": "Live", "nested": {"a": 1}}}}})
    }

    fn api() -> FakeApi {
        FakeApi::new()
            .ok("GET ticket-templates", templates())
            .ok("GET ticket-templates/501", template())
    }

    /// `test_template_list_show_and_prefill_mapping`.
    #[test]
    fn list_show_and_prefill_mapping() {
        let c = api();
        let names: Vec<String> = list_templates(&c)
            .unwrap()
            .iter()
            .map(|r| name_of(r).to_string())
            .collect();
        assert_eq!(names, ["Acme outage", "Globex access"]);
        let row = show(&c, "acme OUTAGE").unwrap();
        let p = to_draft(&row);
        assert_eq!(
            Value::Object(p.answers),
            json!({"subject": "Acme outage", "stage": "Live"})
        );
        assert_eq!(p.description_html.as_deref(), Some("<p>Please help</p>"));
        assert_eq!(p.applied, ["subject", "stage", "description"]);
        assert_eq!(p.ignored, ["nested", "tags"]);
        assert_eq!(
            find(&c, "Initech").unwrap_err().to_string(),
            "no unique ticket template 'Initech'; templates: Acme outage, Globex access"
        );
        assert_eq!(py_str(&show(&c, "501").unwrap()["id"]), "501");
    }

    #[test]
    fn malformed_lists_refuse() {
        for raw in [
            json!({}),
            json!({"ticket_templates": [{"id": 0, "name": "x"}]}),
            json!({"ticket_templates": [{"id": 5}]}),
        ] {
            let c = FakeApi::new().ok("GET ticket-templates", raw);
            assert_eq!(
                list_templates(&c).unwrap_err().to_string(),
                "ticket template list unavailable or malformed"
            );
        }
        let c = FakeApi::new().ok(
            "GET ticket-templates",
            json!({"ticket_templates": [
            {"id": 5, "name": "a"}, {"id": "5", "name": "b"}]}),
        );
        assert_eq!(
            list_templates(&c).unwrap_err().to_string(),
            "ticket template list has duplicate ids"
        );
    }

    /// `test_notify_agent_entry_is_name_and_email_from_agent_lookup`.
    #[test]
    fn notify_agent_entry_is_name_and_email_from_agent_lookup() {
        let rows = json!([{"id": "robin@example.com", "value": "Robin Agent", "user_id": 77},
                          {"id": 8, "value": "Robin Agentson", "email": "ra@example.com"}]);
        let c = FakeApi::new().ok("GET autocomplete/agents", rows);
        assert_eq!(
            notify_entry(&c, "Robin Agent").unwrap(),
            "Robin Agent <robin@example.com>"
        );
        assert_eq!(
            notify_entry(&c, "ra@example.com").unwrap(),
            "Robin Agentson <ra@example.com>"
        );
        for bad in ["Robin", "", "nobody@example.com"] {
            assert!(
                matches!(notify_entry(&c, bad), Err(Error::Value(_))),
                "{bad}"
            );
        }
        let inactive = FakeApi::new().ok(
            "GET autocomplete/agents",
            json!([{"id": "x@example.com", "value": "Robin Agent", "active": false}]),
        );
        assert!(notify_entry(&inactive, "Robin Agent").is_err());
    }

    /// `test_template_create_reports_server_changed_prefill` (readback half).
    #[test]
    fn prefill_changes_reports_server_values() {
        let payload = obj(
            json!({"subject": "Acme outage", "custom_fields": {"stage": "Live"}, "description": "<p>x</p>"}),
        );
        let applied: Vec<String> = ["subject", "stage", "description"]
            .map(String::from)
            .to_vec();
        let c = FakeApi::new().ok(
            "GET tickets/123",
            json!({"ticket": {"id": 123, "subject": "Acme outage (P2)", "custom_fields": {}}}),
        );
        assert_eq!(
            prefill_changes(&c, &json!(123), &payload, &applied),
            json!({"checked": true, "changes": [{"field": "subject", "sent": "Acme outage", "server": "Acme outage (P2)"}],
                   "unavailable": ["stage"], "fields": ["stage", "subject"]})
        );
        let same = FakeApi::new().ok(
            "GET tickets/123",
            json!({"id": 123, "subject": "Acme outage", "custom_fields": {"stage": "Live"}}),
        );
        assert_eq!(
            prefill_changes(&same, &json!(123), &payload, &applied),
            json!({"checked": true, "changes": [], "unavailable": [], "fields": ["stage", "subject"]})
        );
        let broken = FakeApi::new();
        assert_eq!(
            prefill_changes(&broken, &json!(123), &payload, &applied),
            json!({"checked": false, "changes": [], "fields": ["stage", "subject"],
                   "warning": "could not read the ticket back to compare template-prefilled fields"})
        );
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }
}
