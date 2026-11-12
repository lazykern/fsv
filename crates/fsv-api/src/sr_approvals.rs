//! SR approval requests, decisions, cancellation and reminders
//! (`sr_approvals.py`, Python `9070409`).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_forms::pydt::datetime_fromisoformat;
use fsv_py::Dumps;

use crate::approvals::{self, RULES};
use crate::recipients::EMAIL;
use crate::reviewed_write::{WritePlan, execute};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::watchers::positive_id;

/// [`WritePlan`] plus the inputs `send` rebuilds from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPlan {
    pub write: WritePlan,
    pub inputs_json: String,
}

impl ApprovalPlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        self.write.review()
    }
}

/// Ticket approval groups. `ApprovalError` stays an exit-2 refusal.
pub fn inventory(c: &dyn Api, target: i64) -> Result<Vec<Value>> {
    approvals::approval_groups(c, &json!(target), "tickets")
}

/// Read-only list on either model: chain groups, or the flat approvals list.
pub fn listing(c: &dyn Api, target: i64) -> Result<Value> {
    let (model, status) = crate::approval_model::detect(c, "tickets", target)?;
    if model != crate::approval_model::LEGACY {
        return Ok(json!({"approval_groups": inventory(c, target)?}));
    }
    let rows = crate::service::get_ticket_approvals(c, &TICKETS, target)?;
    Ok(json!({
        "model": "legacy",
        "note": crate::approval_model::describe(status),
        "approvals": rows,
    }))
}

/// Explicit requester ids, or one active exact-email autocomplete hit.
pub fn recipients(c: &dyn Api, values: &[Value]) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for value in values {
        let text = fsv_py::py_str(value);
        let ident = if text.is_ascii() && text.bytes().all(|b| b.is_ascii_digit()) {
            positive_id(value)?
        } else {
            if !EMAIL.is_match(&text) {
                return Err(Error::value(
                    "approver must be a positive user ID or exact email",
                ));
            }
            let rows = c.autocomplete("requesters", &text, None)?;
            let want = fsv_core::pystr::casefold(&text);
            let matches: Vec<&Value> = rows
                .iter()
                .filter(|row| {
                    let Some(row) = row.as_object() else {
                        return false;
                    };
                    fsv_core::pystr::casefold(&fsv_py::py_str(
                        row.get("email").unwrap_or(&Value::Null),
                    )) == want
                        && !row.get("deleted").is_some_and(fsv_py::truthy)
                        && !row.get("deactivated").is_some_and(fsv_py::truthy)
                })
                .collect();
            if matches.len() != 1 {
                return Err(crate::reviewed_write::refused(
                    "approver must resolve to one active requester user ID",
                ));
            }
            positive_id(matches[0].get("id").unwrap_or(&Value::Null))?
        };
        if ids.contains(&ident) {
            return Err(Error::value("duplicate approver"));
        }
        ids.push(ident);
    }
    if ids.is_empty() {
        return Err(Error::value("at least one explicit approver required"));
    }
    Ok(ids)
}

/// Raw status id. Names are localisable labels and are never compared.
fn status_id(row: &Value) -> Option<i64> {
    row.get("status")
        .and_then(Value::as_object)
        .and_then(|m| m.get("id"))
        .and_then(Value::as_i64)
}

fn dumps(v: &Value) -> String {
    Dumps::default().sort_keys(true).dump(v)
}

fn rule_id(name: &str) -> Option<i64> {
    RULES
        .iter()
        .find(|(k, _, _)| *k == name)
        .map(|(_, id, _)| *id)
}

/// Port of `sr_approvals.build_plan`.
#[allow(clippy::too_many_arguments)]
pub fn build_plan(
    c: &dyn Api,
    target: &Value,
    operation: &str,
    ident: Option<&Value>,
    name: Option<&str>,
    message: Option<&str>,
    approvers: Option<&[Value]>,
    rule: &str,
    remark: &str,
) -> Result<ApprovalPlan> {
    let target = positive_id(&json!(parse_id(&fsv_py::py_str(target), &TICKETS)?))?;
    if !matches!(
        operation,
        "request" | "approve" | "reject" | "cancel" | "remind"
    ) {
        return Err(Error::value(
            "unsupported approval action; delegation is unproven",
        ));
    }
    if operation == "reject" && fsv_core::pystr::strip(remark).is_empty() {
        return Err(Error::value("reject requires a nonempty --remark"));
    }
    crate::approval_model::require_chains(
        c,
        "tickets",
        target,
        &format!("sr approvals {operation}"),
    )?;
    let before = inventory(c, target)?;
    let mut path = format!("tickets/{target}");
    let payload = if operation == "request" {
        let Some(name) = name.filter(|n| !fsv_core::pystr::strip(n).is_empty()) else {
            return Err(Error::value("nonempty approval name (max 100) required"));
        };
        if name.chars().count() > 100 || name.chars().any(|ch| (ch as u32) < 32) {
            return Err(Error::value("nonempty approval name (max 100) required"));
        }
        let Some(message) = message.filter(|m| !fsv_core::pystr::strip(m).is_empty()) else {
            return Err(Error::value(
                "nonempty message and captured approval rule required",
            ));
        };
        let Some(approval_type) = rule_id(rule) else {
            return Err(Error::value(
                "nonempty message and captured approval rule required",
            ));
        };
        let content = format!(
            "<p>{}</p>",
            fsv_core::pystr::html_escape(message, true).replace('\n', "<br>")
        );
        if content.chars().count() > 65530 {
            return Err(Error::value(
                "approval email content exceeds 65530 characters",
            ));
        }
        let notified = recipients(c, approvers.unwrap_or(&[]))?;
        path.push_str("/approval-groups");
        json!({"approval_groups": [{
            "name": name,
            "approval_type": approval_type,
            "email_content": content,
            "notified_to": notified
        }]})
    } else {
        let ident = positive_id(ident.unwrap_or(&Value::Null))?;
        let rows: Vec<Value> = if operation == "cancel" {
            before.clone()
        } else {
            before
                .iter()
                .filter_map(|g| g.get("approvals").and_then(Value::as_array))
                .flatten()
                .cloned()
                .collect()
        };
        let matches: Vec<&Value> = rows
            .iter()
            .filter(|row| positive_id(row.get("id").unwrap_or(&Value::Null)).ok() == Some(ident))
            .collect();
        if matches.len() != 1 {
            return Err(crate::reviewed_write::refused(
                "exact approval/group ID unavailable",
            ));
        }
        let row = matches[0];
        let requested = if operation == "cancel" {
            row.get("approval_status").and_then(Value::as_i64) == Some(0)
        } else {
            status_id(row) == Some(0)
        };
        if !requested {
            return Err(crate::reviewed_write::refused(
                "target is not a requested approval/group; immutable history cannot be changed",
            ));
        }
        if matches!(operation, "approve" | "reject") {
            path.push_str(&format!("/approvals/{ident}"));
            json!({
                "approval_status": if operation == "approve" { 1 } else { 2 },
                "remark": remark
            })
        } else {
            let suffix = if operation == "cancel" {
                format!("/approval-groups/{ident}/cancel")
            } else {
                format!("/approvals/{ident}/resend")
            };
            path.push_str(&suffix);
            Value::Null
        }
    };
    let inputs = json!({
        "operation": operation,
        "ident": ident.cloned().unwrap_or(Value::Null),
        "name": name,
        "message": message,
        "approvers": approvers,
        "rule": rule,
        "remark": remark
    });
    let context = snapshot_hash(&json!(before))?;
    Ok(ApprovalPlan {
        write: WritePlan {
            action: format!("sr_approval_{operation}"),
            target,
            tenant: c.domain(),
            method: if operation == "request" {
                "POST".into()
            } else {
                "PUT".into()
            },
            path,
            payload_json: dumps(&payload),
            context_hash: context,
        },
        inputs_json: dumps(&inputs),
    })
}

fn aware(value: &Value) -> Result<Option<jiff::Timestamp>> {
    let Some(text) = value.as_str() else {
        return Ok(None);
    };
    let text = text.replace('Z', "+00:00");
    let Some((dt, off)) = datetime_fromisoformat(&text) else {
        return Err(Error::value("invalid isoformat string"));
    };
    let Some(off) = off else {
        return Ok(None);
    };
    off.to_timestamp(dt)
        .map(Some)
        .map_err(|e| Error::value(e.to_string()))
}

fn id_set(rows: &[Value]) -> Result<HashSet<i64>> {
    rows.iter()
        .map(|row| positive_id(row.get("id").unwrap_or(&Value::Null)))
        .collect()
}

/// Port of `sr_approvals.send`.
pub fn send(
    plan: &ApprovalPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs: Value = serde_json::from_str(&plan.inputs_json)?;
    let before = inventory(c, plan.write.target)?;
    let verify = |_: &Value| {
        let after = inventory(c, plan.write.target)?;
        let operation = inputs["operation"].as_str().unwrap_or("");
        let ident = inputs.get("ident").cloned().unwrap_or(Value::Null);
        if operation == "request" {
            let old = id_set(&before)?;
            let fresh: Vec<Value> = after
                .iter()
                .filter(|group| {
                    positive_id(group.get("id").unwrap_or(&Value::Null))
                        .ok()
                        .is_none_or(|id| !old.contains(&id))
                })
                .cloned()
                .collect();
            let requested = &plan.write.payload()["approval_groups"][0];
            let mut matched = fresh.len() == 1 && fresh[0].get("name") == requested.get("name");
            if matched {
                let group = &fresh[0];
                let rows = group
                    .get("approvals")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let members: HashSet<i64> = rows
                    .iter()
                    .map(|row| positive_id(row.get("member_id").unwrap_or(&Value::Null)))
                    .collect::<Result<_>>()?;
                let want: HashSet<i64> = requested
                    .get("notified_to")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(positive_id)
                    .collect::<Result<_>>()?;
                matched = group.get("approval_type") == requested.get("approval_type")
                    && rows.len() == want.len()
                    && members == want
                    && rows.iter().all(|row| status_id(row) == Some(0));
            }
            return Ok(obj(json!({"verified": matched, "readback": fresh})));
        }
        let rows: Vec<Value> = if operation == "cancel" {
            after.clone()
        } else {
            after
                .iter()
                .filter_map(|g| g.get("approvals").and_then(Value::as_array))
                .flatten()
                .cloned()
                .collect()
        };
        let exact = rows
            .iter()
            .find(|row| {
                positive_id(row.get("id").unwrap_or(&Value::Null)).ok() == positive_id(&ident).ok()
            })
            .cloned()
            .unwrap_or(Value::Null);
        let matched = if operation == "cancel" {
            let old_group = before
                .iter()
                .find(|group| {
                    positive_id(group.get("id").unwrap_or(&Value::Null)).ok()
                        == positive_id(&ident).ok()
                })
                .ok_or_else(|| Error::value("missing approval group"))?;
            let new_rows = exact
                .get("approvals")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let old_rows = old_group
                .get("approvals")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            exact.get("approval_status").and_then(Value::as_i64) == Some(3)
                && !new_rows.is_empty()
                && id_set(&new_rows)? == id_set(&old_rows)?
                && new_rows.iter().all(|row| status_id(row) == Some(3))
        } else if operation == "remind" {
            let old = before
                .iter()
                .filter_map(|g| g.get("approvals").and_then(Value::as_array))
                .flatten()
                .find(|row| {
                    positive_id(row.get("id").unwrap_or(&Value::Null)).ok()
                        == positive_id(&ident).ok()
                })
                .ok_or_else(|| Error::value("missing approval"))?;
            let now = aware(exact.get("last_reminded_at").unwrap_or(&Value::Null))?;
            let previous = aware(old.get("last_reminded_at").unwrap_or(&Value::Null))?;
            exact.get("status") == old.get("status")
                && now.is_some()
                && previous.is_none_or(|prev| now.is_some_and(|n| n > prev))
        } else {
            let want = plan.write.payload()["approval_status"].as_i64();
            let remarks = exact
                .get("remark")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            status_id(&exact) == want
                && remarks.iter().any(|row| {
                    row.get("status").and_then(Value::as_i64) == want
                        && row.get("data") == inputs.get("remark")
                })
        };
        Ok(obj(json!({"verified": matched, "readback": exact})))
    };
    let rebuild = || {
        let ident = inputs.get("ident").filter(|v| !v.is_null());
        let name = inputs.get("name").and_then(Value::as_str);
        let message = inputs.get("message").and_then(Value::as_str);
        let approvers = inputs.get("approvers").and_then(Value::as_array);
        build_plan(
            c,
            &json!(plan.write.target),
            inputs["operation"].as_str().unwrap_or(""),
            ident,
            name,
            message,
            approvers.map(Vec::as_slice),
            inputs
                .get("rule")
                .and_then(Value::as_str)
                .unwrap_or("everyone"),
            inputs.get("remark").and_then(Value::as_str).unwrap_or(""),
        )
        .map(|p| p.write)
    };
    execute(&plan.write, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;

    struct World {
        groups: Vec<Value>,
        persist: bool,
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        let mut a = FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            if call.method == "GET" {
                return Ok(json!({
                    "approval_chains": [{
                        "id": 71,
                        "approval_status": 0,
                        "approval_groups": s.groups
                    }],
                    "meta": {"triggerable_level": 1}
                }));
            }
            if !s.persist {
                return Ok(json!({}));
            }
            if call.method == "POST" {
                let body = &call.body.clone().unwrap()["approval_groups"][0];
                s.groups.push(json!({
                    "id": 91,
                    "name": body["name"],
                    "approval_type": body["approval_type"],
                    "approval_status": 0,
                    "approvals": [{
                        "id": 101,
                        "member_id": 81,
                        "user_id": 81,
                        "status": {"id": 0, "name": "requested"},
                        "remark": [],
                        "last_reminded_at": null
                    }]
                }));
            } else {
                let path = call.path.as_str();
                if path.ends_with("/resend") {
                    s.groups[0]["approvals"][0]["last_reminded_at"] = json!("2026-12-01T10:00:00Z");
                } else if path.ends_with("/cancel") {
                    s.groups[0]["approval_status"] = json!(3);
                    s.groups[0]["approvals"][0]["status"] = json!({"id": 3, "name": "cancelled"});
                } else {
                    let body = call.body.clone().unwrap();
                    let ident = body["approval_status"].as_i64().unwrap();
                    let name = if ident == 1 { "approved" } else { "rejected" };
                    s.groups[0]["approvals"][0]["status"] = json!({"id": ident, "name": name});
                    s.groups[0]["approvals"][0]["remark"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"status": ident, "data": body["remark"]}));
                }
            }
            Ok(json!({"approval_chain": {"id": 999}}))
        });
        a.domain = "acme.example.com".into();
        a
    }

    #[test]
    fn cases_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/sr_approvals_oracle.json")).unwrap();
        let cases = oracle["cases"].as_object().unwrap();
        assert_eq!(cases.len(), 8);
        for (name, case) in cases {
            let state = Arc::new(Mutex::new(World {
                groups: Vec::new(),
                persist: true,
            }));
            let c = api(state.clone());
            let journal = MemJournal::default();
            let mut plan: Option<ApprovalPlan> = None;
            for (i, (op, want)) in case["ops"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["outputs"].as_array().unwrap())
                .enumerate()
            {
                let ctx = format!("{name} op {i}");
                let got = match op["op"].as_str().unwrap() {
                    "set" => {
                        let mut s = state.lock().unwrap();
                        if let Some(v) = op.get("persist").and_then(Value::as_bool) {
                            s.persist = v;
                        }
                        if let Some(v) = op.get("approval_status") {
                            s.groups[0]["approval_status"] = v.clone();
                        }
                        Ok(json!({}))
                    }
                    "build" => {
                        let approvers = op.get("approvers").and_then(Value::as_array);
                        build_plan(
                            &c,
                            op.get("target").unwrap_or(&json!(23)),
                            op["operation"].as_str().unwrap(),
                            op.get("ident"),
                            op.get("name").and_then(Value::as_str),
                            op.get("message").and_then(Value::as_str),
                            approvers.map(Vec::as_slice),
                            op.get("rule").and_then(Value::as_str).unwrap_or("everyone"),
                            op.get("remark").and_then(Value::as_str).unwrap_or(""),
                        )
                        .map(|p| {
                            let out = json!({"plan": {
                                "action": p.write.action,
                                "target": p.write.target,
                                "tenant": p.write.tenant,
                                "method": p.write.method,
                                "path": p.write.path,
                                "payload": p.write.payload(),
                                "context_hash": p.write.context_hash,
                                "hash": p.hash(),
                                "review": p.review(),
                                "inputs": serde_json::from_str::<Value>(&p.inputs_json).unwrap(),
                            }});
                            plan = Some(p);
                            out
                        })
                    }
                    "send" => {
                        let p = plan.as_ref().unwrap();
                        let full = p.hash();
                        let reviewed = if op.get("hash").and_then(Value::as_str) == Some("prefix") {
                            Some(full[..12].to_string())
                        } else {
                            Some(full)
                        };
                        send(p, &c, &journal, reviewed.as_deref()).map(|r| json!({"result": r}))
                    }
                    other => panic!("{ctx} {other}"),
                };
                match got {
                    Ok(v) => assert_eq!(v, *want, "{ctx}"),
                    Err(e) => {
                        assert_eq!(
                            e.to_string(),
                            want["error"]["msg"].as_str().unwrap(),
                            "{ctx}"
                        );
                        match want["error"]["type"].as_str().unwrap() {
                            "ValueError" => assert!(matches!(e, Error::Value(_)), "{ctx}"),
                            "ReviewedWriteError" => {
                                assert!(matches!(e, Error::Exit { code: 2, .. }), "{ctx} {e:?}")
                            }
                            other => panic!("{ctx} {other}"),
                        }
                    }
                }
            }
        }
    }
}
