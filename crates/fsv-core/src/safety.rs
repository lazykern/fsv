//! The one place that decides whether a write needs confirmation (`safety.py`).
//!
//! [`write_risks`] inspects the FINAL request body (after flags, `--set` and
//! `--edit` are merged) and returns human-readable reasons; an empty list
//! means the write is safe to send without asking. Labels come from the
//! schema, so no tenant status ids or names are hard-coded.

use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::pystr;

pub const ASSIGNEE_KEYS: &[&str] = &["agent_id", "responder_id", "owner_id"];
pub const GROUP_KEYS: &[&str] = &["group_id"];

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
            risks.push(format!(
                "{what} status -> {label} (status semantics and workflows require confirmation)"
            ));
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
    fn ticket_every_status_transition_is_risky() {
        let t = |status, current| risks("tickets", json!({"status": status}), Some(current), "");
        assert!(!t(4, json!({"status": 1})).is_empty());
        assert!(!t(2, json!({"status": 1})).is_empty());
        assert!(t(2, json!({"status": 2})).is_empty());
    }

    #[test]
    fn localized_terminal_status_needs_confirmation() {
        let german = |v: &Value| match v.as_i64() {
            Some(2) => "Offen".to_string(),
            Some(5) => "Geschlossen".to_string(),
            _ => fsv_py::py_str(v),
        };
        let english = |v: &Value| match v.as_i64() {
            Some(2) => "Open".to_string(),
            Some(3) => "Pending".to_string(),
            Some(4) => "Resolved".to_string(),
            _ => "None".to_string(),
        };
        let cerrado = |_: &Value| "Cerrado".to_string();
        let pending = |_: &Value| "Pending final closure".to_string();
        let cur = obj(json!({"status": 2}));
        let run = |res: &str, status: i64, label: &dyn Fn(&Value) -> String| {
            let ctx = RiskContext {
                current: Some(&cur),
                status_label: Some(label),
                kind: "",
            };
            write_risks(res, &obj(json!({"status": status})), &ctx)
        };
        assert!(!run("tickets", 5, &german).is_empty());
        let unknown = run("tickets", 12, &german);
        assert!(unknown[0].contains("confirmation"));
        assert!(!run("problems", 3, &cerrado).is_empty());
        assert!(!run("tickets", 3, &english).is_empty());
        assert!(!run("tickets", 99, &pending).is_empty());
        assert!(!run("tickets", 4, &english).is_empty());
    }

    #[test]
    fn risk_texts_match_python() {
        let gate = " (status semantics and workflows require confirmation)";
        for (res, body, current, kind, want) in [
            (
                "tickets",
                json!({"status": 4}),
                Some(json!({"status": 1})),
                "",
                vec![format!("ticket status -> Resolved{gate}")],
            ),
            (
                "tickets",
                json!({"status": 2}),
                Some(json!({"status": 2})),
                "",
                vec![],
            ),
            (
                "tickets",
                json!({"status": {"id": 3}}),
                None,
                "",
                vec![format!("ticket status -> Closed{gate}")],
            ),
            (
                "releases",
                json!({"status": 2, "agent_id": 7, "responder_id": 8, "group_id": {"id": 9}}),
                None,
                "",
                vec![
                    format!("release status -> Planning{gate}"),
                    "reassign agent (agent_id=7)".into(),
                    "reassign group (group_id=9)".into(),
                ],
            ),
            (
                "changes",
                json!({"status": 2}),
                None,
                "",
                vec!["change status -> Planning".into()],
            ),
            (
                "changes",
                json!({"status": 3, "owner_id": 5}),
                None,
                "task",
                vec![
                    "task status -> Closed".into(),
                    "reassign agent (owner_id=5)".into(),
                ],
            ),
            (
                "x",
                json!({"status": 1}),
                Some(json!({})),
                "",
                vec![format!("x status -> Open{gate}")],
            ),
        ] {
            assert_eq!(
                risks(res, body.clone(), current, kind),
                want,
                "{res} {body}"
            );
        }
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
