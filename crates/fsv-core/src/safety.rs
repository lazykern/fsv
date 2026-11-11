//! The one place that decides whether a write needs confirmation (`safety.py`).
//!
//! [`write_risks`] inspects the FINAL request body (after flags, `--set` and
//! `--edit` are merged) and returns human-readable reasons; an empty list
//! means the write is safe to send without asking. Labels come from the
//! schema, so no tenant status ids or names are hard-coded.

use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::pystr;

pub const TERMINAL_STATUS_WORDS: &[&str] =
    &["closed", "resolved", "rejected", "cancelled", "canceled"];
/// Labels known NOT to close a record. A label that is neither known
/// terminal nor known open (localized/custom) counts as possibly terminal:
/// confirming one extra time beats closing silently.
pub const OPEN_STATUS_WORDS: &[&str] = &[
    "open",
    "pending",
    "in progress",
    "waiting",
    "on hold",
    "new",
    "planning",
    "awaiting",
    "assigned",
    "scheduled",
    "review",
    "reopened",
];
pub const ASSIGNEE_KEYS: &[&str] = &["agent_id", "responder_id", "owner_id"];
pub const GROUP_KEYS: &[&str] = &["group_id"];

/// Freshservice API default terminal status ids (tickets: 4 Resolved,
/// 5 Closed; problems: 3 Closed), used when the label is unknown.
pub fn terminal_status_ids(resource: &str) -> &'static [&'static str] {
    match resource {
        "tickets" => &["4", "5"],
        "problems" => &["3"],
        _ => &[],
    }
}

/// NFC-normalized, case-folded, whitespace-collapsed.
pub fn norm(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    pystr::collapse_ws(&pystr::casefold(&nfc))
}

/// [`norm`] of `str(value or "")`.
pub fn norm_value(value: &Value) -> String {
    if crate::value::truthy(value) {
        norm(&fsv_py::py_str(value))
    } else {
        String::new()
    }
}

pub fn is_terminal_label(label: &str) -> bool {
    let folded = norm(label);
    TERMINAL_STATUS_WORDS.iter().any(|w| folded.contains(w))
}

pub fn is_open_label(label: &str) -> bool {
    let folded = norm(label);
    !is_terminal_label(label) && OPEN_STATUS_WORDS.iter().any(|w| folded.contains(w))
}

/// `Some(true)`: the status closes/resolves the record; `Some(false)`: known
/// open; `None`: unknown (e.g. a localized or custom label).
pub fn status_closes(resource: &str, value: &Value, label: &str) -> Option<bool> {
    if is_terminal_label(label)
        || terminal_status_ids(resource).contains(&fsv_py::py_str(value).as_str())
    {
        return Some(true);
    }
    if is_open_label(label) {
        return Some(false);
    }
    None
}

fn plain(v: &Value) -> &Value {
    match v {
        Value::Object(m) if m.contains_key("id") => &m["id"],
        other => other,
    }
}

fn same(a: &Value, b: &Value) -> bool {
    fsv_py::py_str(plain(a)) == fsv_py::py_str(plain(b))
}

/// Inputs of [`write_risks`] besides the body.
#[derive(Default)]
pub struct RiskContext<'a> {
    /// The item as stored now (flat); unchanged values are not risks. `None`
    /// means unknown: any provided status/assignee counts.
    pub current: Option<&'a Map<String, Value>>,
    /// Status value → label (default `str(value)`).
    pub status_label: Option<&'a dyn Fn(&Value) -> String>,
    /// `"item"` or `"task"`.
    pub kind: &'a str,
}

/// Reasons this write needs confirmation (empty = none).
///
/// `resource`: `"changes"`, `"tickets"`, `"problems"`, ...
pub fn write_risks(
    resource: &str,
    body: &Map<String, Value>,
    ctx: &RiskContext<'_>,
) -> Vec<String> {
    let label_of = |v: &Value| match ctx.status_label {
        Some(f) => f(v),
        None => fsv_py::py_str(v),
    };
    let empty = Map::new();
    let cur = ctx.current.unwrap_or(&empty);
    let known =
        |key: &str| ctx.current.is_some() && cur.contains_key(key) && same(&body[key], &cur[key]);
    let kind = if ctx.kind.is_empty() {
        "item"
    } else {
        ctx.kind
    };
    let mut risks = Vec::new();

    if body.contains_key("status") && !known("status") {
        let value = plain(&body["status"]);
        let label = label_of(value);
        if kind == "task" {
            risks.push(format!("task status -> {label}"));
        } else if resource == "changes" {
            // every change status transition can fire approvals/workflows
            risks.push(format!("change status -> {label}"));
        } else {
            let what = resource.strip_suffix('s').unwrap_or(resource);
            match status_closes(resource, value, &label) {
                Some(true) => risks.push(format!("{what} status -> {label}")),
                None => risks.push(format!(
                    "{what} status -> {label} (fsv cannot tell whether this status closes it)"
                )),
                Some(false) => {}
            }
        }
    }
    if let Some(key) = ASSIGNEE_KEYS
        .iter()
        .find(|k| body.contains_key(**k) && !known(k))
    {
        risks.push(format!(
            "reassign agent ({key}={})",
            fsv_py::py_str(plain(&body[*key]))
        ));
    }
    if let Some(key) = GROUP_KEYS
        .iter()
        .find(|k| body.contains_key(**k) && !known(k))
    {
        risks.push(format!(
            "reassign group ({key}={})",
            fsv_py::py_str(plain(&body[*key]))
        ));
    }
    risks
}

pub fn note_risks(public: bool) -> Vec<String> {
    if public {
        vec!["public note: visible to the requester and may send notifications".into()]
    } else {
        Vec::new()
    }
}

pub fn reply_risks() -> Vec<String> {
    vec!["ticket reply: sent to the requester by email".into()]
}

pub fn clone_risks(with_tasks: bool) -> Vec<String> {
    if with_tasks {
        vec![
            "--with-tasks: creates tasks that can notify their groups and trigger tenant workflows"
                .into(),
        ]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lab(v: &Value) -> String {
        match v.as_i64() {
            Some(1) => "Open".into(),
            Some(2) => "Planning".into(),
            Some(3) => "Closed".into(),
            Some(4) => "Resolved".into(),
            _ => fsv_py::py_str(v),
        }
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    fn risks(res: &str, body: Value, current: Option<Value>, kind: &str) -> Vec<String> {
        let cur = current.map(obj);
        let ctx = RiskContext {
            current: cur.as_ref(),
            status_label: Some(&lab),
            kind,
        };
        write_risks(res, &obj(body), &ctx)
    }

    #[test]
    fn change_any_status_transition_is_risky_same_status_is_not() {
        assert!(
            !risks(
                "changes",
                json!({"status": 2}),
                Some(json!({"status": 1})),
                ""
            )
            .is_empty()
        );
        assert!(
            risks(
                "changes",
                json!({"status": 1}),
                Some(json!({"status": 1})),
                ""
            )
            .is_empty()
        );
        assert!(!risks("changes", json!({"status": 2}), None, "").is_empty());
    }

    #[test]
    fn ticket_only_terminal_status_is_risky() {
        assert!(
            !risks(
                "tickets",
                json!({"status": 4}),
                Some(json!({"status": 1})),
                ""
            )
            .is_empty()
        );
        assert!(
            risks(
                "tickets",
                json!({"status": 2}),
                Some(json!({"status": 1})),
                ""
            )
            .is_empty()
        );
    }

    #[test]
    fn reassignment_detected_only_when_changed() {
        assert!(
            !risks(
                "changes",
                json!({"group_id": 9}),
                Some(json!({"group_id": 22})),
                ""
            )
            .is_empty()
        );
        assert!(
            risks(
                "changes",
                json!({"group_id": 22}),
                Some(json!({"group_id": 22})),
                ""
            )
            .is_empty()
        );
        assert!(
            !risks(
                "tickets",
                json!({"responder_id": 3}),
                Some(json!({"responder_id": 11})),
                ""
            )
            .is_empty()
        );
    }

    #[test]
    fn task_status_and_owner() {
        assert!(!risks("changes", json!({"status": 3}), None, "task").is_empty());
        assert!(!risks("changes", json!({"owner_id": 3}), None, "task").is_empty());
        assert!(risks("changes", json!({"title": "x"}), None, "task").is_empty());
    }

    #[test]
    fn notifying_notes_and_replies() {
        assert!(!note_risks(true).is_empty() && note_risks(false).is_empty());
        assert!(
            !reply_risks().is_empty()
                && !clone_risks(true).is_empty()
                && clone_risks(false).is_empty()
        );
    }

    #[test]
    fn norm_casefolds_and_collapses() {
        assert_eq!(norm("  Ｃlosed  "), "ｃlosed");
        assert_eq!(norm("In   PROGRESS"), "in progress");
        assert_eq!(norm("Straße"), "strasse");
        assert_eq!(norm("e\u{301}"), "é");
        assert_eq!(norm_value(&Value::Null), "");
        assert_eq!(norm_value(&json!(0)), "");
        assert_eq!(norm_value(&json!(5)), "5");
    }

    #[test]
    fn terminal_and_open_labels() {
        for (label, terminal, open) in [
            ("Closed", true, false),
            ("Resolved by agent", true, false),
            ("Pending", false, true),
            ("Awaiting approval", false, true),
            ("Reopened", false, true),
            ("Unclosed", true, false),
            ("Geschlossen", false, false),
            ("", false, false),
            ("Open but rejected", true, false),
        ] {
            assert_eq!(
                (is_terminal_label(label), is_open_label(label)),
                (terminal, open),
                "{label:?}"
            );
        }
    }

    #[test]
    fn status_closes_by_id_or_label() {
        assert_eq!(status_closes("tickets", &json!(4), "Custom"), Some(true));
        assert_eq!(status_closes("tickets", &json!("5"), ""), Some(true));
        assert_eq!(status_closes("tickets", &json!(2), "Open"), Some(false));
        assert_eq!(status_closes("problems", &json!(3), "x"), Some(true));
        assert_eq!(status_closes("changes", &json!(4), "Weird"), None);
        assert_eq!(status_closes("changes", &json!(1), "Planning"), Some(false));
        assert_eq!(status_closes("changes", &json!(5), "Closed"), Some(true));
    }

    #[test]
    fn plain_and_same_compare_like_python() {
        assert_eq!(plain(&json!({"id": 5, "name": "x"})), &json!(5));
        assert_eq!(plain(&json!({"name": "x"})), &json!({"name": "x"}));
        assert_eq!(plain(&json!(7)), &json!(7));
        assert!(same(&json!({"id": 5}), &json!("5")));
        assert!(!same(&json!(5), &json!(6)));
        assert!(same(&Value::Null, &json!("None")));
        assert!(same(&json!({"id": null}), &Value::Null));
        assert!(!same(&json!(1.0), &json!(1)));
        assert!(same(&json!(true), &json!("True")));
    }
}
