//! Reviewed ticket routing with fresh, explicit agent/group membership
//! (`ticket_assignment.py`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_forms::LookupResolver;
use fsv_py::py_str;

use crate::formmeta::MetaStore;
use crate::properties::{PropertyArgs, build_plan_with, record};
use crate::readback::submitted_value_matches;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};

const WARNING: &str = "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.";

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

/// `_positive`: a positive int or digit string, never a bool.
fn positive(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) if n.is_i64() || n.is_u64() => n.as_i64().filter(|i| *i > 0),
        Value::String(s) if !s.is_empty() => fsv_core::pystr::parse_decimal(s).filter(|i| *i > 0),
        _ => None,
    }
}

/// `_resolve`: exactly one active row whose id, name or email equals `value`.
fn resolve(rows: &Value, value: &str, kind: &str) -> Result<Map<String, Value>> {
    let Some(rows) = rows.as_array().filter(|_| !strip(value).is_empty()) else {
        return Err(refused(format!("runtime {kind} metadata unavailable")));
    };
    let query = casefold(strip(value));
    let matches: Vec<&Map<String, Value>> = rows
        .iter()
        .filter_map(Value::as_object)
        .filter(|r| {
            !r.get("deleted").is_some_and(fsv_py::truthy)
                && !r.get("deactivated").is_some_and(fsv_py::truthy)
                && ["id", "name", "email"]
                    .iter()
                    .any(|k| casefold(&r.get(*k).map_or(String::new(), py_str)) == query)
        })
        .collect();
    match matches.as_slice() {
        [one] if positive(one.get("id").unwrap_or(&Value::Null)).is_some() => Ok((*one).clone()),
        _ => Err(refused(format!(
            "{kind} must identify exactly one active runtime entry"
        ))),
    }
}

/// `_member`: both membership lists are explicit and agree.
fn member(agent: &Map<String, Value>, group: &Map<String, Value>) -> Result<()> {
    let id = |m: &Map<String, Value>| positive(m.get("id").unwrap_or(&Value::Null));
    let (aid, gid) = (id(agent), id(group));
    let (Some(agent_groups), Some(group_agents)) = (
        agent.get("group_ids").and_then(Value::as_array),
        group.get("agent_ids").and_then(Value::as_array),
    ) else {
        return Err(refused("agent/group membership metadata unavailable"));
    };
    if agent_groups
        .iter()
        .chain(group_agents)
        .any(|v| positive(v).is_none())
    {
        return Err(refused("agent/group membership metadata is invalid"));
    }
    if !agent_groups.iter().any(|v| positive(v) == gid)
        || !group_agents.iter().any(|v| positive(v) == aid)
    {
        return Err(refused("agent is not a verified member of the group"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentPlan {
    pub write: WritePlan,
    pub intent_json: String,
    pub before_json: String,
}

impl AssignmentPlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.write.review();
        out.insert(
            "before".into(),
            serde_json::from_str(&self.before_json).unwrap_or(Value::Null),
        );
        out.insert("warning".into(), json!(WARNING));
        out
    }
}

/// Options of [`build_plan`] (Python keyword arguments).
#[derive(Debug, Clone, Copy, Default)]
pub struct AssignmentArgs<'a> {
    /// Agent id, name or email.
    pub agent: Option<&'a str>,
    /// Group id, name or email.
    pub group: Option<&'a str>,
    pub unassign: bool,
    /// Pick up `ids` as the current agent (bulk `tickets/pick_up`).
    pub pickup: bool,
}

/// `build_plan(client, ids, agent=..., group=..., unassign=..., pickup=...)`.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    ids: &[String],
    args: &AssignmentArgs<'_>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<AssignmentPlan> {
    let AssignmentArgs {
        agent,
        group,
        unassign,
        pickup,
    } = *args;
    if ids.is_empty() || ids.len() > 100 {
        return Err(Error::value("specify between 1 and 100 ticket IDs"));
    }
    let cids = ids
        .iter()
        .map(|v| parse_id(v, &TICKETS))
        .collect::<Result<Vec<i64>>>()?;
    let unique: std::collections::HashSet<&i64> = cids.iter().collect();
    if cids.iter().any(|i| *i <= 0) || unique.len() != cids.len() {
        return Err(Error::value("ticket IDs must be positive and unique"));
    }
    if pickup && (agent.is_some() || group.is_some() || unassign) {
        return Err(Error::value("pick-up does not accept assignment options"));
    }
    if !pickup
        && (cids.len() != 1
            || (unassign && (agent.is_some() || group.is_some()))
            || (!unassign && agent.is_none() && group.is_none()))
    {
        return Err(Error::value(
            "assign one ticket with --agent and/or --group, or --unassign",
        ));
    }
    let boot = c.int_get("bootstrap/agents_groups", None)?;
    if !boot.get("agents").is_some_and(Value::is_array)
        || !boot.get("groups").is_some_and(Value::is_array)
    {
        return Err(refused("runtime agent/group metadata unavailable"));
    }
    let mut agent_row = None;
    if pickup {
        let me = c.me()?;
        let Some(mid) = me.get("id").and_then(positive) else {
            return Err(refused("current agent identity unavailable"));
        };
        agent_row = Some(resolve(&boot["agents"], &mid.to_string(), "agent")?);
    } else if let Some(agent) = agent {
        agent_row = Some(resolve(&boot["agents"], agent, "agent")?);
    }
    let group_row = group
        .map(|g| resolve(&boot["groups"], g, "group"))
        .transpose()?;
    let row_id = |m: &Map<String, Value>| json!(positive(m.get("id").unwrap_or(&Value::Null)));
    let mut before = Vec::new();
    let mut plans = Vec::new();
    for cid in &cids {
        let row = record(c, *cid)?;
        let (Some(group_id), Some(responder_id)) = (row.get("group_id"), row.get("responder_id"))
        else {
            return Err(refused("ticket routing fields unavailable"));
        };
        let mut selected = group_row.clone();
        if group.is_none() && !group_id.is_null() && agent_row.is_some() {
            selected = Some(resolve(&boot["groups"], &py_str(group_id), "group")?);
        }
        if let (Some(g), Some(a)) = (&selected, &agent_row) {
            member(a, g)?;
        }
        let mut answers = Map::new();
        if let Some(g) = &group_row {
            answers.insert("group".into(), row_id(g));
            // Native group assignment clears the previous agent unless supplied.
            answers.insert(
                "agent".into(),
                agent_row.as_ref().map_or(Value::Null, row_id),
            );
        } else if unassign {
            answers.insert("agent".into(), Value::Null);
        } else if let Some(a) = &agent_row {
            answers.insert("agent".into(), row_id(a));
        }
        let prop = build_plan_with(
            c,
            store,
            &cid.to_string(),
            &PropertyArgs {
                answers: Some(&answers),
                ..PropertyArgs::default()
            },
            resolver.clone(),
        )?;
        before.push(json!({"ticket_id": cid, "group_id": group_id, "responder_id": responder_id}));
        plans.push(prop);
    }
    let (path, body, action) = if pickup {
        (
            "tickets/pick_up".to_string(),
            json!({"ids": cids}),
            "ticket_pick_up",
        )
    } else {
        (
            format!("tickets/{}", cids[0]),
            plans[0].payload(),
            "ticket_assign",
        )
    };
    let pickup_agent = match (&agent_row, pickup) {
        (Some(a), true) => row_id(a),
        _ => Value::Null,
    };
    let hashes: Vec<String> = plans.iter().map(|p| p.hash()).collect();
    let context = snapshot_hash(
        &json!({"properties": hashes, "bootstrap": boot, "pickup_agent": pickup_agent}),
    )?;
    let write = WritePlan {
        action: action.into(),
        target: cids[0],
        tenant: c.domain(),
        method: "PUT".into(),
        path,
        payload_json: dumps(&body),
        context_hash: context,
    };
    let expected: Vec<Value> = plans
        .iter()
        .map(|p| {
            let mut m = obj(p.payload());
            m.shift_remove("br_validation_excludes");
            Value::Object(m)
        })
        .collect();
    let intent = json!({"ids": cids, "agent": agent, "group": group, "unassign": unassign,
                        "pickup": pickup, "expected": expected});
    Ok(AssignmentPlan {
        write,
        intent_json: dumps(&intent),
        before_json: dumps(&json!(before)),
    })
}

/// `send(plan, client, reviewed_hash=...)`: one PUT after the intent is
/// rebuilt unchanged, then each ticket's submitted fields are read back.
pub fn send(
    plan: &AssignmentPlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<Value> {
    let intent: Value = serde_json::from_str(&plan.intent_json).unwrap_or(Value::Null);
    let ids: Vec<String> = intent["ids"]
        .as_array()
        .map(|a| a.iter().map(py_str).collect())
        .unwrap_or_default();
    let rebuild = || -> Result<WritePlan> {
        let args = AssignmentArgs {
            agent: intent["agent"].as_str(),
            group: intent["group"].as_str(),
            unassign: intent["unassign"] == true,
            pickup: intent["pickup"] == true,
        };
        let fresh = build_plan(c, store, &ids, &args, resolver.clone())?;
        if fresh.intent_json != plan.intent_json {
            return Err(refused("assignment intent changed; review again"));
        }
        Ok(fresh.write)
    };
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let mut results = Map::new();
        let expected = intent["expected"].as_array().cloned().unwrap_or_default();
        for (cid, want) in ids.iter().zip(&expected) {
            let want = want.as_object().cloned().unwrap_or_default();
            let fields: Map<String, Value> = match cid
                .parse::<i64>()
                .map_err(|e| Error::other(e.to_string()))
                .and_then(|id| record(c, id))
            {
                Ok(row) => want
                    .iter()
                    .map(|(k, v)| {
                        let state = match row.get(k) {
                            None => "unavailable",
                            Some(got) if submitted_value_matches(got, v) => "matched",
                            Some(_) => "mismatched",
                        };
                        (k.clone(), json!(state))
                    })
                    .collect(),
                Err(_) => want
                    .keys()
                    .map(|k| (k.clone(), json!("unavailable")))
                    .collect(),
            };
            results.insert(cid.clone(), Value::Object(fields));
        }
        let verified = results.values().all(|f| {
            f.as_object()
                .is_some_and(|m| !m.is_empty() && m.values().all(|v| v == "matched"))
        });
        Ok(obj(json!({"tickets": results, "verified": verified})))
    };
    let bounded = |r: &Value| Ok(crate::readback::bounded_readback(|| verify(r)));
    execute(&plan.write, c, journal, reviewed_hash, &rebuild, &bounded)
}

#[cfg(test)]
mod tests;
