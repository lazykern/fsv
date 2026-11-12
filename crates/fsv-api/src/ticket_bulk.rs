//! Reviewed bulk ticket actions with a frozen explicit selection
//! (`ticket_bulk.py`).
//!
//! `selection`, `build_plan` and `send` call the real property, assignment
//! and merge builders. The bulk hash covers the write hash plus the printed
//! ids. `send` checks that hash, then `reviewed_write::execute` checks the
//! inner write hash.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::casefold;
use fsv_core::resource::{TICKETS, parse_id};

use crate::formmeta::MetaStore;
use crate::properties::{self, PropertyArgs};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{self, Api, Journal, obj, plan_hash, snapshot_hash};
use crate::ticket_assignment::{self, AssignmentArgs, AssignmentPlan};
use crate::ticket_merge;
use crate::watchers;

const WARNING: &str = "Frozen explicit IDs, not server select-all. Partial outcomes are possible; never replay an uncertain bulk action.";

fn dumps_sorted(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().dump(v)
}

/// `urllib.parse.quote_plus` with the default safe set (`/`).
fn quote_plus(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'/' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn string_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(fsv_py::py_str).collect())
        .unwrap_or_default()
}

fn opt_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn filled(v: Option<&str>) -> bool {
    v.is_some_and(|s| !s.is_empty())
}

/// Python `type(a) is type(b) and a == b` for JSON readback values.
fn py_same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => {
            let int = |n: &serde_json::Number| n.is_i64() || n.is_u64();
            int(x) == int(y) && x == y
        }
        _ => false,
    }
}

/// `selection` without a filtered-write bound: a filter is refused.
pub fn selection(
    c: &dyn Api,
    ids: &[String],
    where_: &[String],
    select_all: bool,
) -> Result<Vec<i64>> {
    selection_bounded(c, ids, where_, select_all, None)
}

/// `selection`: sorted positive ids, or a complete filtered inventory of
/// at most `max_records` tickets, each of whose exact record satisfies the
/// predicate.
pub fn selection_bounded(
    c: &dyn Api,
    ids: &[String],
    where_: &[String],
    select_all: bool,
    max_records: Option<i64>,
) -> Result<Vec<i64>> {
    let mut values = if !where_.is_empty() {
        if !ids.is_empty() || !select_all {
            return Err(Error::value(
                "filters require --select-all and no explicit IDs",
            ));
        }
        let Some(max_records) = max_records.filter(|n| (1..=50).contains(n)) else {
            return Err(Error::value(
                "filtered writes require --max-records N (1 to 50)",
            ));
        };
        let fields = crate::schema::fetch(c, &TICKETS)?;
        let schema = json!({"fields": fields});
        let (query, clauses, _) =
            crate::query::build_query_hash(None, "tickets", Some(&schema), where_, None, false)
                .map_err(|e| Error::value(e.0))?;
        let mut params = Map::new();
        params.insert(
            "query_hash".into(),
            query.map(Value::String).unwrap_or(Value::Null),
        );
        params.insert("page".into(), json!(1));
        params.insert("per_page".into(), json!(100));
        let rows = crate::reviewed_inventory::complete_inventory(
            c,
            "tickets",
            "tickets",
            Some(&params),
            crate::reviewed_inventory::Family::Internal,
        )?;
        if rows.len() as i64 > max_records {
            return Err(Error::value(
                "filtered selection exceeds --max-records; refusing the whole plan",
            ));
        }
        let mut values = Vec::with_capacity(rows.len());
        for row in &rows {
            values.push(watchers::positive_id(
                row.get("id").unwrap_or(&Value::Null),
            )?);
        }
        for cid in &values {
            let exact = properties::record(c, *cid)?;
            if !crate::filter_proof::matches(&exact, &clauses)? {
                return Err(refused(format!(
                    "ticket {cid} contradicts the predicate; refusing the whole filtered plan"
                )));
            }
        }
        values
    } else if select_all {
        return Err(Error::value("--select-all requires an explicit filter"));
    } else {
        let mut values = Vec::with_capacity(ids.len());
        for v in ids {
            let id = parse_id(v, &TICKETS)?;
            values.push(watchers::positive_id(&json!(id))?);
        }
        values
    };
    if values.is_empty()
        || values.len() > 100
        || values.len() != values.iter().collect::<BTreeSet<_>>().len()
    {
        return Err(Error::value(
            "select 1 to 100 unique tickets; never silently truncate or chunk",
        ));
    }
    values.sort_unstable();
    Ok(values)
}

/// Keywords of `build_plan` after the id list.
pub struct BulkArgs<'a> {
    pub action: &'a str,
    pub where_: &'a [String],
    pub select_all: bool,
    pub status: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub group: Option<&'a str>,
    pub primary: Option<&'a str>,
    pub note_primary: Option<&'a str>,
    pub note_secondary: Option<&'a str>,
}

/// `BulkPlan`. `pickup` is the assignment plan `send` delegates to.
#[derive(Debug)]
pub struct BulkPlan {
    pub write: WritePlan,
    pub intent_json: String,
    pub ids: Vec<i64>,
    pub expected_json: String,
    pickup: Option<AssignmentPlan>,
    /// The exact `{id, subject}` of every selected ticket, sorted JSON.
    pub selected_json: String,
}

impl BulkPlan {
    /// `hash`: write hash plus the frozen selection, not the write hash alone.
    pub fn hash(&self) -> String {
        let selection: Value = serde_json::from_str(&self.intent_json).unwrap_or(Value::Null);
        let selected: Value = serde_json::from_str(&self.selected_json).unwrap_or(Value::Null);
        plan_hash(&json!({"write": self.write.hash(), "selection": selection,
                          "selected": selected, "ids": self.ids}))
    }

    /// `review`.
    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.write.review();
        out.insert("plan_hash".into(), json!(self.hash()));
        out.insert("ticket_ids".into(), json!(self.ids));
        out.insert(
            "selection".into(),
            serde_json::from_str(&self.intent_json).unwrap_or(Value::Null),
        );
        out.insert(
            "selected_tickets".into(),
            serde_json::from_str(&self.selected_json).unwrap_or(Value::Null),
        );
        out.insert("warning".into(), json!(WARNING));
        out
    }
}

fn intent_value(ids: &[String], args: &BulkArgs<'_>, max_records: Option<i64>) -> Value {
    json!({
        "action": args.action,
        "agent": args.agent,
        "group": args.group,
        "ids": ids,
        "max_records": max_records,
        "note_primary": args.note_primary,
        "note_secondary": args.note_secondary,
        "primary": args.primary,
        "select_all": args.select_all,
        "status": args.status,
        "where": args.where_,
    })
}

/// `build_plan` without `max_records`: a filtered selection is refused.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    ids: &[String],
    args: &BulkArgs<'_>,
) -> Result<BulkPlan> {
    build_plan_bounded(c, store, ids, args, None)
}

/// `build_plan(..., max_records=)`. `BulkArgs` stays unchanged so existing
/// CLI literals keep compiling.
pub fn build_plan_bounded(
    c: &dyn Api,
    store: &dyn MetaStore,
    ids: &[String],
    args: &BulkArgs<'_>,
    max_records: Option<i64>,
) -> Result<BulkPlan> {
    let cids = selection_bounded(c, ids, args.where_, args.select_all, max_records)?;
    let intent_json = dumps_sorted(&intent_value(ids, args, max_records));
    if !matches!(
        args.action,
        "status" | "assign" | "pickup" | "merge" | "spam" | "unspam" | "delete"
    ) {
        return Err(Error::value("unsupported bulk action"));
    }
    let stray_status = args.status.is_some() && args.action != "status";
    let stray_route = (filled(args.agent) || filled(args.group)) && args.action != "assign";
    let stray_merge =
        (filled(args.primary) || filled(args.note_primary) || filled(args.note_secondary))
            && args.action != "merge";
    if stray_status || stray_route || stray_merge {
        return Err(Error::value("options do not match bulk action"));
    }
    let (write, expected, pickup) = if args.action == "pickup" {
        let id_text: Vec<String> = cids.iter().map(ToString::to_string).collect();
        let delegate = ticket_assignment::build_plan(
            c,
            store,
            &id_text,
            &AssignmentArgs {
                pickup: true,
                ..AssignmentArgs::default()
            },
            None,
        )?;
        let expected = serde_json::from_str::<Value>(&delegate.intent_json).unwrap_or(Value::Null)
            ["expected"]
            .clone();
        (delegate.write.clone(), expected, Some(delegate))
    } else if args.action == "merge" {
        let pid = watchers::positive_id(&json!(parse_id(args.primary.unwrap_or(""), &TICKETS)?))?;
        if !cids.contains(&pid) {
            return Err(Error::value(
                "merge primary must be in the printed selected ID list",
            ));
        }
        let secondaries: Vec<String> = cids
            .iter()
            .filter(|id| **id != pid)
            .map(ToString::to_string)
            .collect();
        let delegate = ticket_merge::build_plan(
            c,
            store,
            &pid.to_string(),
            &secondaries,
            args.note_primary.unwrap_or(""),
            args.note_secondary.unwrap_or(""),
            true,
            true,
        )?;
        (delegate, Value::Array(Vec::new()), None)
    } else {
        let mut baselines = Vec::with_capacity(cids.len());
        for id in &cids {
            baselines.push(Value::Object(properties::record(c, *id)?));
        }
        let (plan_hashes, fields) = if args.action == "status" {
            let status = args.status.unwrap_or("");
            if status.is_empty() || matches!(casefold(status).as_str(), "resolved" | "closed") {
                return Err(Error::value(
                    "bulk closure needs separate prerequisites; select an ordinary runtime status",
                ));
            }
            let mut answers = Map::new();
            answers.insert("status".into(), Value::String(status.to_string()));
            let mut props = Vec::with_capacity(cids.len());
            for id in &cids {
                props.push(properties::build_plan_with(
                    c,
                    store,
                    &id.to_string(),
                    &PropertyArgs {
                        answers: Some(&answers),
                        ..PropertyArgs::default()
                    },
                    None,
                )?);
            }
            let statuses: Vec<Value> = props
                .iter()
                .map(|p| p.payload().get("status").cloned().unwrap_or(Value::Null))
                .collect();
            let agreed = statuses.first().filter(|v| !v.is_null()).cloned();
            let Some(status_id) = agreed.filter(|v| statuses.iter().all(|s| s == v)) else {
                return Err(refused("status metadata disagrees across selected tickets"));
            };
            let hashes = props.iter().map(|p| p.hash()).collect();
            (hashes, json!({"status": status_id}))
        } else if args.action == "assign" {
            if !filled(args.agent) || !filled(args.group) {
                return Err(Error::value(
                    "captured bulk assignment requires both --agent and --group; null/unassignment is unproven",
                ));
            }
            let mut props = Vec::with_capacity(cids.len());
            for id in &cids {
                let one = [id.to_string()];
                props.push(ticket_assignment::build_plan(
                    c,
                    store,
                    &one,
                    &AssignmentArgs {
                        agent: args.agent,
                        group: args.group,
                        ..AssignmentArgs::default()
                    },
                    None,
                )?);
            }
            let first = props[0].write.payload();
            let mut fields = Map::new();
            for key in ["group_id", "responder_id"] {
                let Some(v) = first.get(key) else {
                    return Err(Error::other(format!("KeyError: {key}")));
                };
                fields.insert(key.to_string(), v.clone());
            }
            for prop in &props {
                let payload = prop.write.payload();
                if fields.iter().any(|(k, v)| payload.get(k) != Some(v)) {
                    return Err(refused("assignment metadata disagrees"));
                }
            }
            let hashes = props.iter().map(|p| p.hash()).collect();
            (hashes, Value::Object(fields))
        } else {
            let fields = if args.action == "delete" {
                json!({"deleted": true})
            } else {
                json!({"spam": args.action == "spam"})
            };
            let key = fields
                .as_object()
                .and_then(|m| m.keys().next())
                .cloned()
                .unwrap_or_default();
            if baselines
                .iter()
                .any(|row| !matches!(row.get(&key), Some(Value::Bool(_))))
            {
                return Err(refused("spam/deleted baseline flag unavailable"));
            }
            (Vec::new(), fields)
        };
        let expected = Value::Array(vec![fields.clone(); cids.len()]);
        let (method, path, body) = if matches!(args.action, "status" | "assign") {
            let mut body = fields.as_object().cloned().unwrap_or_default();
            body.insert("ids".into(), json!(cids));
            body.insert("reply".into(), json!({}));
            (
                "PUT",
                "bulk_ticket_actions/update_multiple".to_string(),
                Some(Value::Object(body)),
            )
        } else if args.action == "delete" {
            let joined = cids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            (
                "DELETE",
                format!("tickets/delete_multiple?ids={}", quote_plus(&joined)),
                None,
            )
        } else {
            (
                "POST",
                format!("tickets/{}_multiple", args.action),
                Some(json!({"ids": cids})),
            )
        };
        let context = snapshot_hash(&json!({"plans": plan_hashes, "records": baselines}))?;
        let write = WritePlan {
            action: format!("ticket_bulk_{}", args.action),
            target: cids[0],
            tenant: c.domain(),
            method: method.into(),
            path,
            payload_json: match &body {
                None => "null".into(),
                Some(body) => dumps_sorted(body),
            },
            context_hash: context,
        };
        (write, expected, None)
    };
    let mut selected = Vec::with_capacity(cids.len());
    for cid in &cids {
        let row = properties::record(c, *cid)?;
        let subject = row.get("subject").cloned().unwrap_or(Value::Null);
        if !args.where_.is_empty() && !subject.is_string() {
            return Err(refused(
                "exact selected subject unavailable; refusing filtered review",
            ));
        }
        selected.push(json!({"id": cid, "subject": subject}));
    }
    Ok(BulkPlan {
        write,
        intent_json,
        ids: cids,
        expected_json: dumps(&expected),
        pickup,
        selected_json: dumps_sorted(&Value::Array(selected)),
    })
}

fn plan_from_intent(c: &dyn Api, store: &dyn MetaStore, intent_json: &str) -> Result<BulkPlan> {
    let intent: Value = serde_json::from_str(intent_json).unwrap_or(Value::Null);
    let ids = string_list(&intent["ids"]);
    let where_ = string_list(&intent["where"]);
    let action = intent["action"].as_str().unwrap_or("").to_string();
    let status = opt_str(&intent["status"]);
    let agent = opt_str(&intent["agent"]);
    let group = opt_str(&intent["group"]);
    let primary = opt_str(&intent["primary"]);
    let note_primary = opt_str(&intent["note_primary"]);
    let note_secondary = opt_str(&intent["note_secondary"]);
    build_plan_bounded(
        c,
        store,
        &ids,
        &BulkArgs {
            action: &action,
            where_: &where_,
            select_all: intent["select_all"] == true,
            status: status.as_deref(),
            agent: agent.as_deref(),
            group: group.as_deref(),
            primary: primary.as_deref(),
            note_primary: note_primary.as_deref(),
            note_secondary: note_secondary.as_deref(),
        },
        intent["max_records"].as_i64(),
    )
}

fn verify(plan: &BulkPlan, c: &dyn Api) -> Result<Map<String, Value>> {
    let expected: Vec<Value> = serde_json::from_str(&plan.expected_json).unwrap_or_default();
    let mut tickets = Map::new();
    for (cid, exp) in plan.ids.iter().zip(expected.iter()) {
        let fields = exp.as_object().cloned().unwrap_or_default();
        let verdicts = match properties::record(c, *cid) {
            Ok(row) => {
                let mut out = Map::new();
                for (k, v) in &fields {
                    let status = if !row.contains_key(k) {
                        "unavailable"
                    } else if py_same(row.get(k).unwrap_or(&Value::Null), v) {
                        "matched"
                    } else {
                        "mismatched"
                    };
                    out.insert(k.clone(), json!(status));
                }
                out
            }
            Err(_) => fields
                .keys()
                .map(|k| (k.clone(), json!("unavailable")))
                .collect(),
        };
        tickets.insert(cid.to_string(), Value::Object(verdicts));
    }
    let verified = tickets.values().all(|t| {
        t.as_object()
            .is_some_and(|m| m.values().all(|v| v.as_str() == Some("matched")))
    });
    Ok(obj(json!({
        "ticket_ids": plan.ids,
        "tickets": tickets,
        "verified": verified,
    })))
}

fn rebuilt(plan: &BulkPlan, c: &dyn Api, store: &dyn MetaStore) -> Result<BulkPlan> {
    let fresh = plan_from_intent(c, store, &plan.intent_json)?;
    if fresh.hash() != plan.hash() {
        return Err(refused(
            "bulk selection, metadata or records changed; review again",
        ));
    }
    Ok(fresh)
}

/// `send`. Merge and pickup delegate to those modules' own senders.
pub fn send(
    plan: &BulkPlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    confirm_destructive: bool,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    if reviewed != plan.hash() {
        return Err(refused("full bulk review hash changed"));
    }
    let intent: Value = serde_json::from_str(&plan.intent_json).unwrap_or(Value::Null);
    let action = intent["action"].as_str().unwrap_or("");
    if matches!(action, "spam" | "delete") && !confirm_destructive {
        return Err(refused(
            "spam/delete require --confirm-destructive after inspecting the printed ID list",
        ));
    }
    if matches!(
        service::last_event(journal, &plan.write.action, &plan.write.hash()).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "bulk write was sent or outcome unknown; do not replay",
        ));
    }
    rebuilt(plan, c, store)?;
    let reported = |result: Value| {
        let mut result = result;
        if let Value::Object(m) = &mut result {
            let delegated = m.get("plan_hash").cloned().unwrap_or(Value::Null);
            m.insert("delegated_plan_hash".into(), delegated);
            m.insert("plan_hash".into(), json!(plan.hash()));
        }
        result
    };
    if action == "merge" {
        return ticket_merge::send(&plan.write, c, store, journal, Some(&plan.write.hash()))
            .map(reported);
    }
    if action == "pickup" {
        let delegate = plan
            .pickup
            .as_ref()
            .ok_or_else(|| refused("bulk selection, metadata or records changed; review again"))?;
        return ticket_assignment::send(delegate, c, store, journal, Some(&delegate.hash()), None)
            .map(reported);
    }
    let rebuild = || Ok(rebuilt(plan, c, store)?.write);
    let check = |_: &Value| Ok(crate::readback::bounded_readback(|| verify(plan, c)));
    execute(
        &plan.write,
        c,
        journal,
        Some(&plan.write.hash()),
        &rebuild,
        &check,
    )
    .map(reported)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, LazyLock, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::formmeta::MemStore;
    use crate::service::testing::{Call, FnApi, MemJournal};

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/ticket_bulk_oracle.json")).unwrap()
    });

    fn store() -> MemStore {
        MemStore::default()
    }

    fn msg(err: &Error) -> String {
        err.to_string()
    }

    fn args(action: &str) -> BulkArgs<'_> {
        BulkArgs {
            action,
            where_: &[],
            select_all: false,
            status: None,
            agent: None,
            group: None,
            primary: None,
            note_primary: None,
            note_secondary: None,
        }
    }

    fn row(id: i64) -> Value {
        json!({"id": id, "status": 2, "spam": false, "deleted": false})
    }

    struct World {
        rows: BTreeMap<i64, Value>,
        list: Vec<i64>,
        partial: bool,
        fail: bool,
        omit: bool,
        wrote: bool,
        stale: usize,
        post_reads: usize,
    }

    fn world() -> World {
        let mut rows = BTreeMap::new();
        rows.insert(71, row(71));
        rows.insert(72, row(72));
        World {
            rows,
            list: vec![71, 72],
            partial: false,
            fail: false,
            omit: false,
            wrote: false,
            stale: 0,
            post_reads: 0,
        }
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            if call.method == "V2" {
                return Err(Error::other("no v2"));
            }
            if call.method == "GET" && call.path == "ticket_form_fields" {
                return Ok(json!({"ticket_fields": [
                    {"name": "subject", "label": "Subject", "default_field": true}
                ]}));
            }
            if call.method == "GET" && call.path == "tickets" {
                let rows: Vec<Value> = s
                    .list
                    .iter()
                    .filter_map(|id| s.rows.get(id).cloned())
                    .collect();
                return Ok(json!({"tickets": rows, "has_more": s.partial}));
            }
            if call.method == "GET"
                && let Some(id) = call.path.strip_prefix("tickets/")
                && let Ok(id) = id.parse::<i64>()
            {
                let Some(mut row) = s.rows.get(&id).cloned() else {
                    return Err(Error::other("missing ticket"));
                };
                if s.stale > 0 && s.wrote {
                    s.stale -= 1;
                    let obj = row.as_object_mut().unwrap();
                    obj.insert("status".into(), json!(2));
                    obj.insert("spam".into(), json!(false));
                    obj.insert("deleted".into(), json!(false));
                }
                s.post_reads += usize::from(s.wrote);
                if s.omit && s.wrote {
                    row.as_object_mut().unwrap().shift_remove("spam");
                }
                return Ok(json!({"ticket": row}));
            }
            if s.fail {
                return Err(Error::session("expired"));
            }
            let ids: Vec<i64> = if let Some(body) = &call.body {
                body["ids"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                    .unwrap_or_default()
            } else {
                let raw = call
                    .path
                    .split_once('?')
                    .and_then(|(_, q)| q.split('&').find_map(|p| p.strip_prefix("ids=")))
                    .unwrap_or("");
                raw.replace("%2C", ",")
                    .replace("%2c", ",")
                    .split(',')
                    .filter_map(|s| s.parse::<i64>().ok())
                    .collect()
            };
            let fields = if call.path.contains("update_multiple") {
                call.body.clone().unwrap_or(Value::Null)
            } else if call.path.contains("delete_multiple") {
                json!({"deleted": true})
            } else {
                json!({"spam": call.path.ends_with("/spam_multiple")})
            };
            for id in ids {
                if let Some(row) = s.rows.get_mut(&id)
                    && let (Some(obj), Some(fields)) = (row.as_object_mut(), fields.as_object())
                {
                    for (k, v) in fields {
                        if k != "ids" && k != "reply" {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            s.wrote = true;
            if call.method == "DELETE" {
                Ok(Value::Null)
            } else {
                Ok(json!({"success": true}))
            }
        })
    }

    fn posts(c: &FnApi) -> Vec<Call> {
        c.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call.method.as_str(), "POST" | "PUT" | "DELETE"))
            .cloned()
            .collect()
    }

    fn spam_plan(c: &FnApi) -> BulkPlan {
        build_plan(
            c,
            &store(),
            &["INC-72".into(), "INC-71".into()],
            &args("spam"),
        )
        .unwrap()
    }

    fn oracle_plan(name: &str) -> &'static Value {
        &ORACLE[name]
    }

    #[test]
    fn spam_matches_python_and_replays_once() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let plan = spam_plan(&c);
        let want = oracle_plan("spam");
        assert_eq!(plan.write.action, want["action"].as_str().unwrap());
        assert_eq!(plan.write.method, want["method"].as_str().unwrap());
        assert_eq!(plan.write.path, want["path"].as_str().unwrap());
        assert_eq!(plan.write.payload_json, want["payload"].as_str().unwrap());
        assert_eq!(plan.write.tenant, want["tenant"].as_str().unwrap());
        assert_eq!(plan.write.target, want["target"].as_i64().unwrap());
        assert_eq!(plan.write.context_hash, want["context"].as_str().unwrap());
        assert_eq!(plan.write.hash(), want["write_hash"].as_str().unwrap());
        assert_eq!(plan.hash(), want["hash"].as_str().unwrap());
        assert_eq!(plan.review()["ticket_ids"], json!([71, 72]));
        assert!(
            plan.review()["warning"]
                .as_str()
                .unwrap()
                .contains("Frozen explicit")
        );
        assert!(posts(&c).is_empty());
        let journal = MemJournal::default();
        let sent = send(&plan, &c, &store(), &journal, Some(&plan.hash()), true).unwrap();
        assert_eq!(sent["verified"], json!(true));
        assert_eq!(sent["ticket_ids"], json!([71, 72]));
        let wrote = posts(&c);
        assert_eq!(wrote.len(), 1);
        assert_eq!(wrote[0].method, "POST");
        assert_eq!(wrote[0].path, "tickets/spam_multiple");
        assert_eq!(wrote[0].body.as_ref().unwrap()["ids"], json!([71, 72]));
        let err = send(&plan, &c, &store(), &journal, Some(&plan.hash()), true).unwrap_err();
        assert!(msg(&err).contains("was sent"), "{}", msg(&err));
        assert_eq!(posts(&c).len(), 1);
    }

    #[test]
    fn delete_and_unspam_match_python() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let plan = build_plan(&c, &store(), &["71".into(), "72".into()], &args("delete")).unwrap();
        let want = oracle_plan("delete");
        assert_eq!(plan.write.path, want["path"].as_str().unwrap());
        assert_eq!(plan.write.payload_json, want["payload"].as_str().unwrap());
        assert_eq!(plan.write.hash(), want["write_hash"].as_str().unwrap());
        assert_eq!(plan.hash(), want["hash"].as_str().unwrap());
        assert_eq!(plan.write.payload(), Value::Null);
        let journal = MemJournal::default();
        let sent = send(&plan, &c, &store(), &journal, Some(&plan.hash()), true).unwrap();
        assert_eq!(sent["verified"], json!(true));
        let wrote = posts(&c);
        assert_eq!(wrote.len(), 1);
        assert_eq!(wrote[0].method, "DELETE");
        assert!(wrote[0].body.is_none());

        let state = Arc::new(Mutex::new(world()));
        let c = api(state);
        let plan = build_plan(&c, &store(), &["71".into()], &args("unspam")).unwrap();
        let want = oracle_plan("unspam");
        assert_eq!(plan.write.path, want["path"].as_str().unwrap());
        assert_eq!(plan.write.payload_json, want["payload"].as_str().unwrap());
        assert_eq!(plan.write.context_hash, want["context"].as_str().unwrap());
        assert_eq!(plan.hash(), want["hash"].as_str().unwrap());
        let sent = send(
            &plan,
            &c,
            &store(),
            &MemJournal::default(),
            Some(&plan.hash()),
            false,
        )
        .unwrap();
        assert_eq!(sent["verified"], json!(true));
    }

    #[test]
    fn destructive_confirm_cancel_and_bad_hash_send_nothing() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let plan = spam_plan(&c);
        let err = send(
            &plan,
            &c,
            &store(),
            &MemJournal::default(),
            Some(&plan.hash()),
            false,
        )
        .unwrap_err();
        assert!(msg(&err).contains("--confirm-destructive"));
        let cancelled = send(&plan, &c, &store(), &MemJournal::default(), None, true).unwrap();
        assert_eq!(cancelled, json!({"status": "cancelled", "verified": false}));
        let err = send(
            &plan,
            &c,
            &store(),
            &MemJournal::default(),
            Some("nope"),
            true,
        )
        .unwrap_err();
        assert!(msg(&err).contains("full bulk review hash changed"));
        assert!(posts(&c).is_empty());
    }

    fn filtered_world() -> World {
        let mut w = world();
        for (id, row) in &mut w.rows {
            let obj = row.as_object_mut().unwrap();
            obj.insert("group_id".into(), json!(93));
            obj.insert("subject".into(), json!(format!("Acme {id}")));
        }
        w
    }

    const GROUP: [&str; 1] = ["group_id=93"];

    fn filtered(action: &str) -> (Vec<String>, BulkArgs<'_>) {
        let where_: Vec<String> = GROUP.iter().map(|s| s.to_string()).collect();
        let mut bulk = args(action);
        bulk.select_all = true;
        (where_, bulk)
    }

    #[test]
    fn filter_drift_and_partial_inventory_send_nothing() {
        let state = Arc::new(Mutex::new(filtered_world()));
        let c = api(Arc::clone(&state));
        let (where_, mut bulk) = filtered("spam");
        bulk.where_ = &where_;
        let plan = build_plan_bounded(&c, &store(), &[], &bulk, Some(2)).unwrap();
        assert_eq!(plan.ids, vec![71, 72]);
        state.lock().unwrap().list.retain(|id| *id != 72);
        let err = send(
            &plan,
            &c,
            &store(),
            &MemJournal::default(),
            Some(&plan.hash()),
            true,
        )
        .unwrap_err();
        assert!(msg(&err).contains("selection"), "{}", msg(&err));
        assert!(posts(&c).is_empty());

        let state = Arc::new(Mutex::new(filtered_world()));
        state.lock().unwrap().partial = true;
        let c = api(state);
        let err = build_plan_bounded(&c, &store(), &[], &bulk, Some(2)).unwrap_err();
        assert!(
            msg(&err).contains("complete, consistent mutation inventory"),
            "{}",
            msg(&err)
        );
        assert!(posts(&c).is_empty());
    }

    #[test]
    fn filtered_expansion_is_explicitly_bounded() {
        for bound in [None, Some(0), Some(51), Some(-1)] {
            let c = api(Arc::new(Mutex::new(filtered_world())));
            let (where_, mut bulk) = filtered("spam");
            bulk.where_ = &where_;
            let err = build_plan_bounded(&c, &store(), &[], &bulk, bound).unwrap_err();
            assert_eq!(
                msg(&err),
                "filtered writes require --max-records N (1 to 50)"
            );
            assert!(c.calls.lock().unwrap().is_empty());
        }
        let c = api(Arc::new(Mutex::new(filtered_world())));
        let (where_, mut bulk) = filtered("spam");
        bulk.where_ = &where_;
        let err = build_plan(&c, &store(), &[], &bulk).unwrap_err();
        assert!(msg(&err).contains("--max-records"));
        let err = build_plan_bounded(&c, &store(), &[], &bulk, Some(1)).unwrap_err();
        assert_eq!(
            msg(&err),
            "filtered selection exceeds --max-records; refusing the whole plan"
        );
        assert!(posts(&c).is_empty());
    }

    #[test]
    fn ignored_filter_blocks_every_action_before_write() {
        for action in [
            "status", "assign", "pickup", "merge", "spam", "unspam", "delete",
        ] {
            for bad in ["contradiction", "missing", "unsupported"] {
                let mut w = filtered_world();
                let row = w.rows.get_mut(&72).unwrap().as_object_mut().unwrap();
                match bad {
                    "contradiction" => {
                        row.insert("group_id".into(), json!(94));
                    }
                    "missing" => {
                        row.shift_remove("group_id");
                    }
                    _ => {}
                }
                let c = api(Arc::new(Mutex::new(w)));
                let where_ = if bad == "unsupported" {
                    vec!["created_at>=2026-01-01".to_string()]
                } else {
                    GROUP.iter().map(|s| s.to_string()).collect()
                };
                let mut bulk = args(action);
                bulk.where_ = &where_;
                bulk.select_all = true;
                bulk.status = Some("Open");
                bulk.agent = Some("me");
                let err = build_plan_bounded(&c, &store(), &[], &bulk, Some(2)).unwrap_err();
                let text = msg(&err);
                let want = match bad {
                    "contradiction" => {
                        "ticket 72 contradicts the predicate; refusing the whole filtered plan"
                    }
                    "missing" => "predicate field unavailable in exact record: 'group_id'",
                    _ => "predicate cannot be independently verified for 'created_at'",
                };
                assert_eq!(text, want, "{action} {bad}");
                assert!(posts(&c).is_empty());
            }
        }
    }

    #[test]
    fn full_selected_subjects_and_ids_are_hash_bound() {
        let state = Arc::new(Mutex::new(filtered_world()));
        let c = api(Arc::clone(&state));
        let (where_, mut bulk) = filtered("spam");
        bulk.where_ = &where_;
        let plan = build_plan_bounded(&c, &store(), &[], &bulk, Some(2)).unwrap();
        assert_eq!(
            plan.review()["selected_tickets"],
            json!([{"id": 71, "subject": "Acme 71"}, {"id": 72, "subject": "Acme 72"}])
        );
        let mut plan = plan;
        let full = plan.hash();
        plan.ids = vec![71];
        assert_ne!(plan.hash(), full);
        plan.ids = vec![71, 72];
        state.lock().unwrap().rows.get_mut(&72).unwrap()["subject"] = json!("Globex changed");
        let err = send(
            &plan,
            &c,
            &store(),
            &MemJournal::default(),
            Some(&plan.hash()),
            true,
        )
        .unwrap_err();
        assert!(msg(&err).contains("selection"), "{}", msg(&err));
        assert!(posts(&c).is_empty());

        let mut w = filtered_world();
        w.rows
            .get_mut(&72)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .shift_remove("subject");
        let c = api(Arc::new(Mutex::new(w)));
        let err = build_plan_bounded(&c, &store(), &[], &bulk, Some(2)).unwrap_err();
        assert_eq!(
            msg(&err),
            "exact selected subject unavailable; refusing filtered review"
        );
    }

    static READBACK: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/bulk_readback_oracle.json")).unwrap()
    });

    #[test]
    fn lagging_readback_rereads_only_and_matches_python() {
        for case in READBACK["cases"].as_array().unwrap() {
            let action = case["action"].as_str().unwrap();
            let state = Arc::new(Mutex::new(world()));
            state.lock().unwrap().stale = case["lag"].as_u64().unwrap() as usize;
            let c = api(Arc::clone(&state));
            let plan =
                build_plan(&c, &store(), &["71".into(), "72".into()], &args(action)).unwrap();
            assert_eq!(plan.hash(), case["hash"].as_str().unwrap());
            assert_eq!(plan.write.hash(), case["write_hash"].as_str().unwrap());
            let journal = MemJournal::default();
            let sleeps = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&sleeps);
            let sent = crate::readback::with_readback_sleep(
                move |s| seen.lock().unwrap().push(s),
                || send(&plan, &c, &store(), &journal, Some(&plan.hash()), true),
            )
            .unwrap();
            assert_eq!(sent, case["result"], "{action} {}", case["lag"]);
            assert_eq!(json!(*sleeps.lock().unwrap()), case["sleeps"]);
            assert_eq!(json!(state.lock().unwrap().post_reads), case["post_reads"]);
            let err = send(&plan, &c, &store(), &journal, Some(&plan.hash()), true).unwrap_err();
            assert_eq!(msg(&err), case["replay"].as_str().unwrap());
            assert_eq!(json!(posts(&c).len()), case["writes"]);
        }
    }

    #[test]
    fn disconnect_and_missing_flag_are_not_replayed() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let plan = spam_plan(&c);
        state.lock().unwrap().fail = true;
        let journal = MemJournal::default();
        let err = send(&plan, &c, &store(), &journal, Some(&plan.hash()), true).unwrap_err();
        assert!(msg(&err).contains("unknown"), "{}", msg(&err));
        let err = send(&plan, &c, &store(), &journal, Some(&plan.hash()), true).unwrap_err();
        assert!(msg(&err).contains("was sent"), "{}", msg(&err));
        assert_eq!(posts(&c).len(), 1);

        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let plan = spam_plan(&c);
        state.lock().unwrap().omit = true;
        let journal = MemJournal::default();
        let sent = send(&plan, &c, &store(), &journal, Some(&plan.hash()), true).unwrap();
        assert_eq!(sent["verified"], json!(false));
        let err = send(&plan, &c, &store(), &journal, Some(&plan.hash()), true).unwrap_err();
        assert!(msg(&err).contains("was sent"));
        assert_eq!(posts(&c).len(), 1);
    }

    #[test]
    fn invalid_selection_and_options_never_write() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        for (ids, needle) in [
            (Vec::<String>::new(), "select 1 to 100"),
            (vec!["71".into(), "71".into()], "select 1 to 100"),
        ] {
            let err = build_plan(&c, &store(), &ids, &args("spam")).unwrap_err();
            assert!(msg(&err).contains(needle), "{}", msg(&err));
        }
        let mut select_all = args("spam");
        select_all.select_all = true;
        let err = build_plan(&c, &store(), &[], &select_all).unwrap_err();
        assert!(msg(&err).contains("--select-all requires an explicit filter"));
        let where_ = ["created_at>=2026-01-01".to_string()];
        let mut both = args("spam");
        both.where_ = &where_;
        both.select_all = true;
        let err = build_plan(&c, &store(), &["71".into()], &both).unwrap_err();
        assert!(msg(&err).contains("filters require --select-all"));
        let mut filtered = args("spam");
        filtered.where_ = &where_;
        let err = build_plan(&c, &store(), &[], &filtered).unwrap_err();
        assert!(msg(&err).contains("filters require --select-all"));
        let err = build_plan(&c, &store(), &["71".into()], &args("nope")).unwrap_err();
        assert!(msg(&err).contains("unsupported bulk action"));
        let mut status = args("spam");
        status.status = Some("Pending");
        let err = build_plan(&c, &store(), &["71".into()], &status).unwrap_err();
        assert!(msg(&err).contains("options do not match"));
        let mut resolved = args("status");
        resolved.status = Some("Resolved");
        let err = build_plan(&c, &store(), &["71".into()], &resolved).unwrap_err();
        assert!(msg(&err).contains("bulk closure"), "{}", msg(&err));
        let mut assign = args("assign");
        assign.agent = Some("91");
        let err = build_plan(&c, &store(), &["71".into()], &assign).unwrap_err();
        assert!(
            msg(&err).contains("both --agent and --group"),
            "{}",
            msg(&err)
        );
        let mut bare = row(71);
        bare.as_object_mut().unwrap().shift_remove("spam");
        state.lock().unwrap().rows.insert(71, bare);
        let err = build_plan(&c, &store(), &["71".into()], &args("spam")).unwrap_err();
        assert!(msg(&err).contains("baseline flag"), "{}", msg(&err));
        assert!(posts(&c).is_empty());
    }
}
