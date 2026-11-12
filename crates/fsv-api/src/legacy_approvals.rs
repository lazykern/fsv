//! Legacy flat-approval driver (`legacy_approvals.py`).
//!
//! Capture-based. Chains stay the explicit model. The write response is
//! never trusted; verification reads the flat approvals list.

use std::collections::HashSet;

use fsv_forms::pydt::datetime_fromisoformat;
use fsv_py::Dumps;
use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};

use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, snapshot_hash};
use crate::watchers::positive_id;

pub const PROVENANCE: &str =
    "legacy flat approvals driver: capture-based, not yet live-proven on any tenant";

const RULES: &[(&str, i64)] = &[("everyone", 1), ("first-responder", 4)];

fn rule_id(rule: &str) -> Option<i64> {
    RULES
        .iter()
        .find(|(name, _)| *name == rule)
        .map(|(_, id)| *id)
}

/// Port of `legacy_approvals.refuse`.
pub fn refuse(action: &str) -> Error {
    refused(format!(
        "{action} refused: this tenant uses legacy flat approvals and no {action} wire was captured for that model; use the web UI"
    ))
}

fn status_id(row: &Value, key: &str) -> Option<i64> {
    row.get(key)
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("id"))
        .and_then(|id| match id {
            Value::Number(n) if n.is_i64() => n.as_i64(),
            _ => None,
        })
}

fn shown(id: Option<i64>) -> String {
    match id {
        Some(n) => n.to_string(),
        None => "None".into(),
    }
}

/// Port of `legacy_approvals.inventory`.
pub fn inventory(c: &dyn Api, module: &str, target: i64) -> Result<Vec<Value>> {
    if module != "changes" && module != "tickets" {
        return Err(Error::value("unsupported approval module"));
    }
    let raw = c.int_get(&format!("{module}/{target}/approvals"), None)?;
    let raw = if raw.is_array() {
        json!({"approvals": raw})
    } else {
        raw
    };
    let Some(rows) = raw.get("approvals").and_then(Value::as_array) else {
        return Err(refused("flat approvals list unreadable"));
    };
    if let Some(meta) = raw.get("meta").and_then(Value::as_object) {
        let continued = ["partial", "has_more", "has_next", "next_page"]
            .iter()
            .any(|key| meta.get(*key).is_some_and(fsv_py::truthy));
        if continued || meta.get("complete") == Some(&Value::Bool(false)) {
            return Err(refused("flat approvals list incomplete"));
        }
    }
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        if !row.is_object() {
            return Err(refused("flat approval row unreadable"));
        }
        ids.push(positive_id(row.get("id").unwrap_or(&Value::Null))?);
        positive_id(row.get("member_id").unwrap_or(&Value::Null))?;
    }
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(refused("duplicate flat approval identities"));
    }
    Ok(rows.clone())
}

/// A [`WritePlan`] plus the baseline the verifier compares against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPlan {
    pub plan: WritePlan,
    pub module: String,
    pub inputs_json: String,
    pub before_json: String,
}

impl LegacyPlan {
    pub fn hash(&self) -> String {
        self.plan.hash()
    }

    /// Port of `LegacyPlan.review`.
    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.plan.review();
        let inputs: Value = serde_json::from_str(&self.inputs_json).unwrap_or(json!({}));
        out.insert("driver".into(), json!("legacy"));
        out.insert("provenance".into(), json!(PROVENANCE));
        if let Some(obj) = inputs.as_object() {
            for (key, value) in obj {
                out.insert(key.clone(), value.clone());
            }
        }
        out.insert(
            "verification".into(),
            json!(
                "Independent read of the flat approvals list only; the write response is not trusted."
            ),
        );
        out.insert(
            "warning".into(),
            json!(
                "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent."
            ),
        );
        out
    }
}

fn dumps_sorted(value: &Value, ascii: bool) -> String {
    Dumps::default()
        .sort_keys(true)
        .ensure_ascii(ascii)
        .dump(value)
}

fn template(c: &dyn Api) -> Result<String> {
    let raw = c.int_get("approvals/approval_template", None)?;
    let raw = if let Some(obj) = raw.as_object() {
        obj.get("approval_template")
            .cloned()
            .or_else(|| obj.get("template").cloned())
            .unwrap_or(Value::Null)
    } else {
        raw
    };
    match raw {
        Value::String(text) if !text.trim().is_empty() => Ok(text),
        _ => Err(refused("approval email template unavailable; nothing sent")),
    }
}

/// Port of `legacy_approvals.build_request`.
pub fn build_request(
    c: &dyn Api,
    module: &str,
    target: i64,
    recipients: &[i64],
    rule: &str,
    recipient_context_json: &str,
) -> Result<LegacyPlan> {
    if module != "changes" && module != "tickets" {
        return Err(Error::value("unsupported approval module"));
    }
    let target = positive_id(&json!(target))?;
    let Some(rule_type) = rule_id(rule) else {
        let names = RULES
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::value(format!(
            "legacy approvals support only the observed rules: {names}"
        )));
    };
    let mut ids: Vec<i64> = recipients
        .iter()
        .map(|id| positive_id(&json!(id)))
        .collect::<Result<_>>()?;
    ids.sort_unstable();
    if ids.is_empty() || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(Error::value(
            "at least one distinct explicit recipient is required",
        ));
    }
    let before = inventory(c, module, target)?;
    let mut payload = json!({"approval_type": rule_type});
    if module == "tickets" {
        payload["email_content"] = json!(template(c)?);
    }
    payload["notified_to"] = json!(ids);
    let sources: Value = serde_json::from_str(if recipient_context_json.is_empty() {
        "[]"
    } else {
        recipient_context_json
    })?;
    let inputs = json!({
        "operation": "request",
        "rule": rule,
        "recipients": ids,
        "recipient_sources": sources,
    });
    let context = json!({"before": before, "recipient_sources": inputs["recipient_sources"]});
    Ok(LegacyPlan {
        plan: WritePlan {
            action: format!("legacy_approval_request_{module}"),
            target,
            tenant: c.domain(),
            method: "POST".into(),
            path: format!("{module}/{target}/approvals"),
            payload_json: dumps_sorted(&payload, false),
            context_hash: snapshot_hash(&context)?,
        },
        module: module.into(),
        inputs_json: dumps_sorted(&inputs, true),
        before_json: dumps_sorted(&json!(before), true),
    })
}

/// Port of `legacy_approvals.build_remind`.
pub fn build_remind(
    c: &dyn Api,
    module: &str,
    target: i64,
    approval_id: i64,
) -> Result<LegacyPlan> {
    if module != "changes" && module != "tickets" {
        return Err(Error::value("unsupported approval module"));
    }
    let target = positive_id(&json!(target))?;
    let approval_id = positive_id(&json!(approval_id))?;
    let before = inventory(c, module, target)?;
    let matches: Vec<&Value> = before
        .iter()
        .filter(|row| positive_id(row.get("id").unwrap_or(&Value::Null)).ok() == Some(approval_id))
        .collect();
    if matches.len() != 1 {
        return Err(refused(
            "exact approval record unavailable in the flat approvals list",
        ));
    }
    if status_id(matches[0], "status") != Some(0) {
        return Err(refused(
            "only a requested approval (raw status 0) can be reminded",
        ));
    }
    let inputs = json!({"operation": "remind", "approval_id": approval_id});
    Ok(LegacyPlan {
        plan: WritePlan {
            action: format!("legacy_approval_remind_{module}"),
            target,
            tenant: c.domain(),
            method: "PUT".into(),
            path: format!("{module}/{target}/approvals/{approval_id}/resend"),
            payload_json: "null".into(),
            context_hash: snapshot_hash(&json!({"before": before}))?,
        },
        module: module.into(),
        inputs_json: dumps_sorted(&inputs, true),
        before_json: dumps_sorted(&json!(before), true),
    })
}

fn instant(value: &Value) -> Option<jiff::Timestamp> {
    let text = value.as_str()?;
    let (dt, off) = datetime_fromisoformat(&text.replace('Z', "+00:00"))?;
    off?.to_timestamp(dt).ok()
}

fn id_of(row: &Value) -> Result<i64> {
    positive_id(row.get("id").unwrap_or(&Value::Null))
}

/// Port of `legacy_approvals.verify`.
pub fn verify(c: &dyn Api, plan: &LegacyPlan) -> Result<Map<String, Value>> {
    let inputs: Value = serde_json::from_str(&plan.inputs_json)?;
    let before: Vec<Value> = serde_json::from_str(&plan.before_json)?;
    let after = inventory(c, &plan.module, plan.plan.target)?;
    let old: HashSet<i64> = before.iter().map(id_of).collect::<Result<_>>()?;
    let now_ids: HashSet<i64> = after.iter().map(id_of).collect::<Result<_>>()?;
    if !old.is_subset(&now_ids) {
        return Ok(obj(
            json!({"verified": false, "unverified": ["baseline approval records disappeared"], "readback": after}),
        ));
    }
    if inputs["operation"] == "remind" {
        let approval_id = inputs["approval_id"].as_i64().unwrap_or(0);
        let Some(exact) = after
            .iter()
            .find(|row| id_of(row).ok() == Some(approval_id))
        else {
            return Err(Error::other(
                "exact approval record missing from the flat approvals list",
            ));
        };
        let Some(prev) = before
            .iter()
            .find(|row| id_of(row).ok() == Some(approval_id))
        else {
            return Err(Error::other(
                "exact approval record missing from the flat approvals list",
            ));
        };
        let reminded = instant(exact.get("last_reminded_at").unwrap_or(&Value::Null));
        let earlier = instant(prev.get("last_reminded_at").unwrap_or(&Value::Null));
        let ok = status_id(exact, "status") == status_id(prev, "status")
            && reminded.is_some()
            && earlier.is_none_or(|then| reminded.is_some_and(|now| now > then));
        let mut out = obj(json!({"verified": ok, "readback": exact}));
        if !ok {
            out.insert(
                "unverified".into(),
                json!(["last_reminded_at did not advance"]),
            );
        }
        return Ok(out);
    }
    let fresh: Vec<Value> = after
        .into_iter()
        .filter(|row| id_of(row).ok().is_none_or(|id| !old.contains(&id)))
        .collect();
    let mut members: Vec<i64> = fresh
        .iter()
        .map(|row| positive_id(row.get("member_id").unwrap_or(&Value::Null)))
        .collect::<Result<_>>()?;
    members.sort_unstable();
    let recipients: Vec<i64> = inputs["recipients"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .collect();
    if members != recipients {
        return Ok(obj(json!({
            "verified": false,
            "unverified": ["new approval records do not exactly match the recipients"],
            "readback": fresh,
        })));
    }
    let mut notes = Vec::new();
    let want_type = rule_id(inputs["rule"].as_str().unwrap_or(""));
    for row in &fresh {
        let aid = id_of(row)?;
        if status_id(row, "status") != Some(0) {
            notes.push(format!(
                "approval {aid}: raw status {} is not requested (0)",
                shown(status_id(row, "status"))
            ));
        }
        if status_id(row, "type") != want_type {
            notes.push(format!(
                "approval {aid}: raw type {} does not match the rule",
                shown(status_id(row, "type"))
            ));
        }
    }
    let mut approval_ids: Vec<i64> = fresh.iter().map(id_of).collect::<Result<_>>()?;
    approval_ids.sort_unstable();
    let mut out =
        obj(json!({"verified": notes.is_empty(), "approval_ids": approval_ids, "readback": fresh}));
    if !notes.is_empty() {
        out.insert("unverified".into(), json!(notes));
    }
    Ok(out)
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// Port of `legacy_approvals.send`.
pub fn send(
    plan: &LegacyPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs: Value = serde_json::from_str(&plan.inputs_json)?;
    let module = plan.module.clone();
    let target = plan.plan.target;
    let operation = inputs["operation"].as_str().unwrap_or("").to_string();
    let approval_id = inputs.get("approval_id").and_then(Value::as_i64);
    let recipients: Vec<i64> = inputs
        .get("recipients")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .collect();
    let rule = inputs
        .get("rule")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let sources = inputs
        .get("recipient_sources")
        .cloned()
        .unwrap_or(json!([]));
    let rebuild = || -> Result<WritePlan> {
        if operation == "remind" {
            return Ok(build_remind(c, &module, target, approval_id.unwrap_or(0))?.plan);
        }
        let context = if fsv_py::truthy(&sources) {
            Dumps::default().dump(&sources)
        } else {
            String::new()
        };
        Ok(build_request(c, &module, target, &recipients, &rule, &context)?.plan)
    };
    let mut result = execute(
        &plan.plan,
        c,
        journal,
        reviewed_hash,
        &rebuild,
        &|_: &Value| verify(c, plan),
    )?;
    if let Some(body) = result.as_object_mut() {
        body.insert("provenance".into(), json!(PROVENANCE));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;

    fn row(id: i64, member: i64, status: i64, reminded: Option<&str>) -> Value {
        json!({
            "id": id,
            "member_id": member,
            "status": {"id": status, "name": "x"},
            "type": {"id": 1, "name": "everyone"},
            "last_reminded_at": reminded,
        })
    }

    #[test]
    fn request_remind_and_refusals_match_python() {
        let rows = Arc::new(Mutex::new(vec![row(710000000001, 9000003999, 1, None)]));
        let effect = Arc::new(Mutex::new(true));
        let rows_h = rows.clone();
        let effect_h = effect.clone();
        let c = FnApi::new(move |call: &Call| {
            if call.path == "approvals/approval_template" {
                return Ok(json!(
                    "<p>Hi {{approval.member.name}}, please review {{ticket.subject}}</p>"
                ));
            }
            if call.method == "GET" && call.path.ends_with("/approvals") {
                return Ok(json!({"approvals": rows_h.lock().unwrap().clone()}));
            }
            if call.method == "POST" {
                if *effect_h.lock().unwrap() {
                    let ids = call
                        .body
                        .as_ref()
                        .and_then(|b| b.get("notified_to"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let kind = call
                        .body
                        .as_ref()
                        .and_then(|b| b.get("approval_type"))
                        .and_then(Value::as_i64)
                        .unwrap_or(1);
                    let mut guard = rows_h.lock().unwrap();
                    for (i, member) in ids.iter().enumerate() {
                        let mut created = row(
                            720000000000 + i as i64,
                            member.as_i64().unwrap_or(0),
                            0,
                            None,
                        );
                        created["type"] = json!({"id": kind, "name": "x"});
                        guard.push(created);
                    }
                }
                return Ok(json!({"untrusted": true}));
            }
            if call.method == "PUT" && call.path.ends_with("/resend") {
                let mut guard = rows_h.lock().unwrap();
                for item in guard.iter_mut() {
                    let id = item["id"].as_i64().unwrap_or(0).to_string();
                    if call.path.contains(&format!("/{id}/resend")) {
                        item["last_reminded_at"] = json!("2030-01-02T03:04:05Z");
                    }
                }
                return Ok(json!({"untrusted": true}));
            }
            Err(Error::other(format!(
                "unexpected {} {}",
                call.method, call.path
            )))
        });
        let err = build_request(&c, "changes", 41, &[9000003101, 9000003102], "majority", "")
            .unwrap_err();
        assert!(matches!(err, Error::Value(_)));
        assert!(err.to_string().contains("observed rules"));
        let reminded = build_remind(&c, "changes", 41, 710000000001).unwrap_err();
        assert!(reminded.to_string().contains("only a requested approval"));

        let plan =
            build_request(&c, "changes", 41, &[9000003102, 9000003101], "everyone", "").unwrap();
        let review = plan.review();
        assert_eq!(review["driver"], "legacy");
        assert_eq!(review["path"], "/api/_/changes/41/approvals");
        assert_eq!(
            review["payload"],
            json!({"approval_type": 1, "notified_to": [9000003101_i64, 9000003102_i64]})
        );
        assert!(
            review["provenance"]
                .as_str()
                .unwrap()
                .contains("not yet live-proven")
        );
        let journal = MemJournal::default();
        let sent = send(&plan, &c, &journal, Some(&plan.hash())).unwrap();
        assert_eq!(sent["verified"], json!(true));
        assert_eq!(
            sent["approval_ids"],
            json!([720000000000_i64, 720000000001_i64])
        );
        assert!(
            sent["provenance"]
                .as_str()
                .unwrap()
                .contains("not yet live-proven")
        );
        let replay = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(replay.to_string().contains("was sent"));

        *effect.lock().unwrap() = false;
        let quiet = build_request(&c, "changes", 41, &[9000003101], "first-responder", "").unwrap();
        assert_eq!(quiet.plan.payload()["approval_type"], json!(4));
        let journal = MemJournal::default();
        let missed = send(&quiet, &c, &journal, Some(&quiet.hash())).unwrap();
        assert_eq!(missed["verified"], json!(false));
        assert!(
            missed["warning"]
                .as_str()
                .unwrap()
                .contains("do not resend")
        );

        let ticket = build_request(&c, "tickets", 41, &[9000004001], "everyone", "").unwrap();
        assert_eq!(
            ticket.plan.payload()["email_content"],
            "<p>Hi {{approval.member.name}}, please review {{ticket.subject}}</p>"
        );

        rows.lock()
            .unwrap()
            .push(row(730000000001, 9000003101, 0, None));
        let remind = build_remind(&c, "changes", 41, 730000000001).unwrap();
        assert_eq!(remind.plan.method, "PUT");
        assert_eq!(remind.plan.path, "changes/41/approvals/730000000001/resend");
        assert!(remind.plan.payload().is_null());
        let journal = MemJournal::default();
        let sent = send(&remind, &c, &journal, Some(&remind.hash())).unwrap();
        assert_eq!(sent["verified"], json!(true));
        assert_eq!(c.writes().len(), 3);
    }
}
