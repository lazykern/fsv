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

use crate::delete_state::delete_state_tokens;
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
    selection_with(c, ids, where_, select_all, max_records, None)
}

/// Loads the ticket schema for a filtered selection (`schema.load`, cached).
pub type SchemaLoader<'a> = &'a dyn Fn() -> Result<Value>;

/// A [`crate::query::Directory`] over the session client, like
/// `query._resolve_*` calling `c.autocomplete` and `c.int_get`. The first
/// client error is kept so one that reaches the caller unmapped surfaces
/// as itself, not as a `ValueError`.
struct ApiDirectory<'a> {
    c: &'a dyn Api,
    failure: std::cell::RefCell<Option<Error>>,
}

impl ApiDirectory<'_> {
    fn keep(&self, e: Error) -> crate::QueryError {
        let msg = e.to_string();
        self.failure.borrow_mut().get_or_insert(e);
        crate::QueryError(msg)
    }
}

impl crate::query::Directory for ApiDirectory<'_> {
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        extra: &[(&str, &str)],
    ) -> std::result::Result<Vec<Value>, crate::QueryError> {
        let params: Option<Map<String, Value>> = (!extra.is_empty()).then(|| {
            extra
                .iter()
                .map(|(k, v)| ((*k).to_string(), json!(v)))
                .collect()
        });
        self.c
            .autocomplete(kind, query, params)
            .map_err(|e| self.keep(e))
    }
    fn agent_groups(&self) -> std::result::Result<Value, crate::QueryError> {
        self.c
            .int_get("bootstrap/agents_groups", None)
            .map_err(|e| self.keep(e))
    }
}

/// [`selection_bounded`] whose filter schema comes from `schema` when given
/// (else a fresh fetch).
pub fn selection_with(
    c: &dyn Api,
    ids: &[String],
    where_: &[String],
    select_all: bool,
    max_records: Option<i64>,
    schema: Option<SchemaLoader<'_>>,
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
        let schema = match schema {
            Some(load) => load()?,
            None => json!({"fields": crate::schema::fetch(c, &TICKETS)?}),
        };
        let dir = ApiDirectory {
            c,
            failure: Default::default(),
        };
        let built =
            crate::query::build_query_hash(Some(&dir), "tickets", Some(&schema), where_, None, false);
        let (query, clauses, _) = built.map_err(|e| match dir.failure.take() {
            Some(api) if api.to_string() == e.0 => api,
            _ => Error::value(e.0),
        })?;
        if query.as_deref().is_none_or(|q| q.starts_with('{')) {
            return Err(Error::value(
                "filtered writes take only basic list filters (status, priority, people, groups, date ranges); advanced-only conditions cannot select a write",
            ));
        }
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
    /// `schema.load(TICKETS, client)` for `--where`; `None` fetches.
    pub schema: Option<SchemaLoader<'a>>,
}

/// `BulkPlan`. `pickup` is the assignment plan `send` delegates to.
#[derive(Debug)]
pub struct BulkPlan {
    pub write: WritePlan,
    pub intent_json: String,
    pub ids: Vec<i64>,
    pub expected_json: String,
    pickup: Option<AssignmentPlan>,
    /// Per-ticket property plans of `bulk status --set` (`delegate`).
    props: Vec<properties::AnsweredPropertyPlan>,
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
        if self.write.action == "ticket_bulk_scenario" {
            let expected: Value = serde_json::from_str(&self.expected_json).unwrap_or(Value::Null);
            let chosen = &expected[0];
            out.insert(
                "scenario".into(),
                json!({"id": chosen["scenario_id"], "name": chosen["scenario_name"]}),
            );
            out.insert(
                "warning".into(),
                json!(format!(
                    "Runs scenario {} ({}) on every listed ticket. Its actions (ticket type changes, assignments, emails, notes) run on the server asynchronously (HTTP 202) and cannot be proven by fsv; check the scenario name above. Never replay an uncertain run.",
                    fsv_py::py_str(&chosen["scenario_id"]),
                    fsv_py::py_repr(&chosen["scenario_name"])
                )),
            );
        }
        out
    }
}

/// Port of `ticket_bulk.resolve_scenario`: one scenario by exact ID or
/// exact name (never by list position); more than one match is refused.
pub fn resolve_scenario(c: &dyn Api, value: &str) -> Result<Map<String, Value>> {
    let raw = crate::canned::scenarios(c)?;
    let rows = match &raw {
        Value::Object(m) => m
            .get("scenario_automations")
            .cloned()
            .unwrap_or(Value::Null),
        other => other.clone(),
    };
    let rows: Vec<Map<String, Value>> = match rows.as_array() {
        Some(list) if list.iter().all(Value::is_object) => {
            list.iter().filter_map(|r| r.as_object().cloned()).collect()
        }
        _ => return Err(refused("scenario list is unreadable; nothing was sent")),
    };
    let text = fsv_core::pystr::strip(value).to_string();
    if text.is_empty() {
        return Err(Error::value(
            "--scenario needs an exact scenario name or ID",
        ));
    }
    let id_of = |r: &Map<String, Value>| fsv_py::py_str(r.get("id").unwrap_or(&Value::Null));
    let digits = text.chars().all(|ch| ch.is_ascii_digit());
    let by_id = rows.iter().filter(|r| digits && id_of(r) == text);
    let exact: Vec<&Map<String, Value>> = rows
        .iter()
        .filter(|r| r.get("name").and_then(Value::as_str) == Some(text.as_str()))
        .collect();
    let by_name = if exact.is_empty() {
        rows.iter()
            .filter(|r| {
                r.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| casefold(fsv_core::pystr::strip(n)) == casefold(&text))
            })
            .collect()
    } else {
        exact
    };
    let mut found: Vec<(String, &Map<String, Value>)> = Vec::new();
    for r in by_id.chain(by_name) {
        let key = id_of(r);
        match found.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = r,
            None => found.push((key, r)),
        }
    }
    if found.len() != 1 {
        let names = found
            .iter()
            .map(|(_, r)| {
                format!(
                    "{}: {}",
                    id_of(r),
                    fsv_py::py_repr(r.get("name").unwrap_or(&Value::Null))
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let names = if names.is_empty() {
            "none".to_string()
        } else {
            names
        };
        return Err(refused(format!(
            "scenario {} must match exactly one scenario by ID or exact name (matches: {names}); nothing was sent",
            fsv_py::py_repr_str(&text)
        )));
    }
    Ok(found[0].1.clone())
}

fn intent_value(
    ids: &[String],
    args: &BulkArgs<'_>,
    max_records: Option<i64>,
    answers: Option<&Map<String, Value>>,
    scenario: Option<&str>,
) -> Value {
    let mut intent = json!({
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
    });
    // Bound only when used: older review hashes are unchanged.
    if let Some(answers) = answers.filter(|a| !a.is_empty()) {
        intent["answers"] = Value::Object(answers.clone());
    }
    if let Some(scenario) = scenario {
        intent["scenario"] = json!(scenario);
    }
    intent
}

/// `build_plan` without `max_records`: a filtered selection is refused.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    ids: &[String],
    args: &BulkArgs<'_>,
) -> Result<BulkPlan> {
    build_plan_bounded(c, store, journal, ids, args, None)
}

/// `build_plan(..., max_records=)`. `BulkArgs` stays unchanged so existing
/// CLI literals keep compiling.
pub fn build_plan_bounded(
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    ids: &[String],
    args: &BulkArgs<'_>,
    max_records: Option<i64>,
) -> Result<BulkPlan> {
    build_plan_with_answers(c, store, journal, ids, args, max_records, None)
}

/// [`build_plan_bounded`] plus `answers=` (`bulk status --set`): each
/// ticket's edit form and rules evaluate the status with these answers, and
/// [`send`] writes one reviewed property PUT per ticket.
pub fn build_plan_with_answers(
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    ids: &[String],
    args: &BulkArgs<'_>,
    max_records: Option<i64>,
    answers: Option<&Map<String, Value>>,
) -> Result<BulkPlan> {
    build_plan_scenario(c, store, journal, ids, args, max_records, answers, None)
}

/// [`build_plan_with_answers`] plus `scenario=` (`bulk scenario --scenario`):
/// the captured `PUT tickets/execute_scenario` for explicit IDs only.
#[allow(clippy::too_many_arguments)]
pub fn build_plan_scenario(
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    ids: &[String],
    args: &BulkArgs<'_>,
    max_records: Option<i64>,
    answers: Option<&Map<String, Value>>,
    scenario: Option<&str>,
) -> Result<BulkPlan> {
    let answers = answers.filter(|a| !a.is_empty());
    if args.action == "scenario" && (!args.where_.is_empty() || args.select_all) {
        return Err(Error::value(
            "bulk scenario takes explicit ticket IDs only; filters are not supported",
        ));
    }
    let cids = selection_with(
        c,
        ids,
        args.where_,
        args.select_all,
        max_records,
        args.schema,
    )?;
    if let Some(answers) = answers {
        if args.action != "status" {
            return Err(Error::value("--set is supported by bulk status only"));
        }
        if answers
            .keys()
            .any(|k| casefold(fsv_core::pystr::strip(k)) == "status")
        {
            return Err(Error::value(
                "use --status for the status; --set is for the fields its rules require",
            ));
        }
    }
    if scenario.is_some() != (args.action == "scenario") {
        return Err(Error::value(
            "--scenario NAME_OR_ID is required by, and only valid for, bulk scenario",
        ));
    }
    let intent_json = dumps_sorted(&intent_value(ids, args, max_records, answers, scenario));
    let mut delegate = Vec::new();
    if !matches!(
        args.action,
        "status" | "assign" | "pickup" | "merge" | "spam" | "unspam" | "delete" | "scenario"
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
    } else if args.action == "scenario" {
        let definition = resolve_scenario(c, scenario.unwrap_or(""))?;
        let mut baselines = Vec::with_capacity(cids.len());
        for id in &cids {
            baselines.push(Value::Object(properties::record(c, *id)?));
        }
        let sid = watchers::positive_id(definition.get("id").unwrap_or(&Value::Null))?;
        let write = WritePlan {
            action: "ticket_bulk_scenario".into(),
            target: cids[0],
            tenant: c.domain(),
            method: "PUT".into(),
            path: "tickets/execute_scenario".into(),
            payload_json: dumps_sorted(&json!({"scenario_id": sid, "ids": cids})),
            context_hash: snapshot_hash(
                &json!({"records": baselines, "scenario": Value::Object(definition.clone())}),
            )?,
        };
        let name = definition.get("name").cloned().unwrap_or(Value::Null);
        (
            write,
            json!([{"scenario_id": sid, "scenario_name": name}]),
            None,
        )
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
            // Each ticket's edit form and rules decide (e.g. a reason a status requires).
            let mut all = Map::new();
            all.insert("status".into(), Value::String(status.to_string()));
            all.extend(answers.cloned().unwrap_or_default());
            let mut props = Vec::with_capacity(cids.len());
            for id in &cids {
                props.push(properties::build_plan_with(
                    c,
                    store,
                    &id.to_string(),
                    &PropertyArgs {
                        answers: Some(&all),
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
            if answers.is_some() {
                delegate = props;
            }
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
        let mut state = json!({"plans": plan_hashes, "records": baselines});
        if args.action == "delete" {
            // a delete after a restore is a new plan, not a replay
            let rows: Vec<Map<String, Value>> = baselines
                .iter()
                .filter_map(Value::as_object)
                .cloned()
                .collect();
            state["states"] = Value::Array(delete_state_tokens(
                journal,
                &rows,
                &cids,
                &["ticket_restore"],
            ));
        }
        let context = snapshot_hash(&state)?;
        if !delegate.is_empty() {
            // The bulk update_multiple wire is captured for status/assignment
            // only; rule-required answers use the captured single-ticket PUT
            // per ticket, each hash-bound and read back independently.
            let mut expected = Vec::with_capacity(delegate.len());
            let mut bodies = Vec::with_capacity(delegate.len());
            for p in &delegate {
                let payload = p.payload().as_object().cloned().unwrap_or_default();
                let mut sent = payload.clone();
                sent.shift_remove("br_validation_excludes");
                expected.push(Value::Object(sent));
                let mut body = Map::new();
                body.insert("id".into(), json!(p.plan.ticket_id));
                body.extend(payload);
                bodies.push(Value::Object(body));
            }
            let write = WritePlan {
                action: "ticket_bulk_status_fields".into(),
                target: cids[0],
                tenant: c.domain(),
                method: "PUT".into(),
                path: "tickets/{id} (one per selected ticket)".into(),
                payload_json: fsv_py::Dumps::default()
                    .sort_keys(true)
                    .ensure_ascii(false)
                    .dump(&Value::Array(bodies)),
                context_hash: context,
            };
            (write, Value::Array(expected), None)
        } else {
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
        }
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
        props: delegate,
        selected_json: dumps_sorted(&Value::Array(selected)),
    })
}

fn plan_from_intent(
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    intent_json: &str,
) -> Result<BulkPlan> {
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
    let scenario = opt_str(&intent["scenario"]);
    build_plan_scenario(
        c,
        store,
        journal,
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
            schema: None,
        },
        intent["max_records"].as_i64(),
        intent["answers"].as_object(),
        scenario.as_deref(),
    )
}

/// `_send_each`: per-ticket reviewed property writes; stops at the first
/// failed or uncertain ticket and never retries it.
fn send_each(
    plan: &BulkPlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
) -> Result<Value> {
    let action = &plan.write.action;
    let hash = plan.write.hash();
    let target = json!(plan.write.target);
    journal.record_plan("started", &hash, action, Some(&target), None);
    let mut tickets = Map::new();
    for (index, prop) in plan.props.iter().enumerate() {
        let id = prop.plan.ticket_id;
        match properties::send_with(prop, c, journal, store, Some(&prop.hash()), None) {
            Err(e) => {
                tickets.insert(
                    id.to_string(),
                    json!({"verified": false, "error": e.to_string()}),
                );
                let rest: Vec<i64> = plan.props[index + 1..]
                    .iter()
                    .map(|p| p.plan.ticket_id)
                    .collect();
                return Ok(
                    json!({"status": "partial", "verified": false, "plan_hash": plan.hash(),
                    "ticket_ids": plan.ids, "tickets": tickets, "stopped_at": id,
                    "not_attempted": rest,
                    "warning": "Stopped at the first failed or uncertain ticket; inspect it and the journal; never replay."}),
                );
            }
            Ok(result) => {
                let kept: Map<String, Value> = ["verified", "fields", "plan_hash"]
                    .into_iter()
                    .filter_map(|k| result.get(k).map(|v| (k.to_string(), v.clone())))
                    .collect();
                tickets.insert(id.to_string(), Value::Object(kept));
            }
        }
    }
    let verified = tickets
        .values()
        .all(|t| t.get("verified").is_some_and(fsv_py::truthy));
    journal.record_plan("done", &hash, action, Some(&target), None);
    Ok(
        json!({"status": "submitted", "verified": verified, "plan_hash": plan.hash(),
              "ticket_ids": plan.ids, "tickets": tickets}),
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

/// The `verify` of a scenario run: exact ticket identities only; the
/// scenario's effects are never proven.
fn verify_scenario(plan: &BulkPlan, c: &dyn Api) -> Result<Map<String, Value>> {
    let mut seen = Map::new();
    for cid in &plan.ids {
        let same = properties::record(c, *cid)
            .map(|row| fsv_py::py_str(row.get("id").unwrap_or(&Value::Null)) == cid.to_string())
            .unwrap_or(false);
        seen.insert(cid.to_string(), json!(same));
    }
    let expected: Value = serde_json::from_str(&plan.expected_json).unwrap_or(Value::Null);
    Ok(obj(json!({
        "ticket_ids": plan.ids,
        "identity_verified": seen,
        "scenario": expected[0],
        "verified": false,
        "warning": "Accepted for asynchronous execution; exact tickets were read back, but the scenario's effects cannot be proven. Inspect the tickets; do not resend.",
    })))
}

fn rebuilt(
    plan: &BulkPlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
) -> Result<BulkPlan> {
    let fresh = plan_from_intent(c, store, journal, &plan.intent_json)?;
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
    if let Some(refusal) = service::replay_refusal(
        journal,
        &plan.write.action,
        &plan.write.hash(),
        "bulk write was sent or outcome unknown; do not replay",
    ) {
        return Err(refused(refusal));
    }
    rebuilt(plan, c, store, journal)?;
    if intent.get("answers").is_some_and(fsv_py::truthy) {
        return send_each(plan, c, store, journal);
    }
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
    let rebuild = || Ok(rebuilt(plan, c, store, journal)?.write);
    let check = |_: &Value| {
        Ok(crate::readback::bounded_readback(|| {
            if action == "scenario" {
                verify_scenario(plan, c)
            } else {
                verify(plan, c)
            }
        }))
    };
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
            schema: None,
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
            &MemJournal::default(),
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
        let plan = build_plan(
            &c,
            &store(),
            &MemJournal::default(),
            &["71".into(), "72".into()],
            &args("delete"),
        )
        .unwrap();
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
        let plan = build_plan(
            &c,
            &store(),
            &MemJournal::default(),
            &["71".into()],
            &args("unspam"),
        )
        .unwrap();
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
        let plan =
            build_plan_bounded(&c, &store(), &MemJournal::default(), &[], &bulk, Some(2)).unwrap();
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
        let err = build_plan_bounded(&c, &store(), &MemJournal::default(), &[], &bulk, Some(2))
            .unwrap_err();
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
            let err = build_plan_bounded(&c, &store(), &MemJournal::default(), &[], &bulk, bound)
                .unwrap_err();
            assert_eq!(
                msg(&err),
                "filtered writes require --max-records N (1 to 50)"
            );
            assert!(c.calls.lock().unwrap().is_empty());
        }
        let c = api(Arc::new(Mutex::new(filtered_world())));
        let (where_, mut bulk) = filtered("spam");
        bulk.where_ = &where_;
        let err = build_plan(&c, &store(), &MemJournal::default(), &[], &bulk).unwrap_err();
        assert!(msg(&err).contains("--max-records"));
        let err = build_plan_bounded(&c, &store(), &MemJournal::default(), &[], &bulk, Some(1))
            .unwrap_err();
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
                    vec![
                        "created_at>=2026-01-01".to_string(),
                        "created_at<=2026-01-31".to_string(),
                    ]
                } else {
                    GROUP.iter().map(|s| s.to_string()).collect()
                };
                let mut bulk = args(action);
                bulk.where_ = &where_;
                bulk.select_all = true;
                bulk.status = Some("Open");
                bulk.agent = Some("me");
                let err =
                    build_plan_bounded(&c, &store(), &MemJournal::default(), &[], &bulk, Some(2))
                        .unwrap_err();
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
    fn advanced_only_filters_cannot_select_a_write() {
        let c = api(Arc::new(Mutex::new(filtered_world())));
        let where_ = vec!["created_at=on:today".to_string()];
        let mut bulk = args("spam");
        bulk.where_ = &where_;
        bulk.select_all = true;
        let err = build_plan_bounded(&c, &store(), &MemJournal::default(), &[], &bulk, Some(2))
            .unwrap_err();
        assert_eq!(
            msg(&err),
            "filtered writes take only basic list filters (status, priority, people, groups, date ranges); advanced-only conditions cannot select a write"
        );
        assert!(posts(&c).is_empty());
    }

    #[test]
    fn full_selected_subjects_and_ids_are_hash_bound() {
        let state = Arc::new(Mutex::new(filtered_world()));
        let c = api(Arc::clone(&state));
        let (where_, mut bulk) = filtered("spam");
        bulk.where_ = &where_;
        let plan =
            build_plan_bounded(&c, &store(), &MemJournal::default(), &[], &bulk, Some(2)).unwrap();
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
        let err = build_plan_bounded(&c, &store(), &MemJournal::default(), &[], &bulk, Some(2))
            .unwrap_err();
        assert_eq!(
            msg(&err),
            "exact selected subject unavailable; refusing filtered review"
        );
    }

    /// `bulk status --set` (Python 676b8bb), generated inline over the
    /// ticket form and a Pending-needs-reason rule.
    static SET: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/ticket_bulk_set_oracle.json")).unwrap()
    });

    fn set_api(fail_on: Option<i64>, writes: Arc<Mutex<Vec<Value>>>) -> FnApi {
        use crate::service::testing::form_field;
        let fields = json!([
            form_field(
                9000007101,
                "status",
                "Status",
                "default_status",
                1,
                json!({"choices": [{"id": 2, "value": "Open"}, {"id": 3, "value": "Pending"},
                                   {"id": 4, "value": "Resolved"}, {"id": 5, "value": "Closed"}]})
            ),
            form_field(
                9000007103,
                "reason",
                "Pending Reason",
                "custom_text",
                3,
                json!({})
            ),
        ]);
        let rules = json!({"business_rules": [{
            "id": 9000007107_i64, "name": "Pending needs reason", "workspace_id": 2, "module_type": "ticket",
            "item_id": null, "deleted": false, "user_type": "both", "form_type": "both", "state": "active",
            "rule_type": "client_side", "description": "",
            "condition_data": {"all": [{"name": "status", "operator": "is", "value": "3", "evaluate_on": "ticket"}]},
            "action_data": {"action": [{"name": ["customFields.reason"], "property": "mandate"}]},
            "is_reversible": false, "is_validation": false, "updated_at": "2030-01-03T00:00:00Z"}]});
        let mut rows: BTreeMap<i64, Value> = [71, 72]
            .into_iter()
            .map(|i| {
                (
                    i,
                    json!({"id": i, "subject": format!("Acme {i}"), "status": 2,
                           "custom_fields": {"reason": null}}),
                )
            })
            .collect();
        FnApi::new(move |call: &Call| {
            let id = call
                .path
                .strip_prefix("tickets/")
                .and_then(|s| s.parse::<i64>().ok());
            match (call.method.as_str(), call.path.as_str(), id) {
                ("ME", ..) => Ok(json!({"id": 901})),
                ("GET", _, Some(id)) => Ok(json!({"ticket": rows[&id]})),
                ("GET", "ticket_form_fields", _) => Ok(json!({"ticket_fields": fields})),
                ("GET", p, _) if p.ends_with("business_rules") => Ok(rules.clone()),
                ("PUT", _, Some(id)) => {
                    let raw = String::from_utf8(call.raw.clone().unwrap()).unwrap();
                    writes
                        .lock()
                        .unwrap()
                        .push(json!({"method": "PUT", "id": id, "raw": raw}));
                    if Some(id) == fail_on {
                        return Err(fsv_core::ApiError::new(
                            400,
                            json!({"errors": [
                            {"field": "status", "message": "Synthetic rejection"}]}),
                        )
                        .into());
                    }
                    let row = rows.get_mut(&id).unwrap().as_object_mut().unwrap();
                    for (k, v) in call.body.as_ref().unwrap().as_object().unwrap() {
                        match k.as_str() {
                            "custom_fields" => {
                                let cf = row.get_mut(k).unwrap().as_object_mut().unwrap();
                                cf.extend(v.as_object().unwrap().clone());
                            }
                            "br_validation_excludes" => {}
                            _ => {
                                row.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    Ok(json!({"ticket": row}))
                }
                _ => Err(fsv_core::ApiError::new(404, json!("missing")).into()),
            }
        })
    }

    /// `{"error": {type, msg}}`; `PropertyError` and `ReviewedWriteError`
    /// are both exit-2 refusals in Rust.
    fn error_json(e: &Error) -> Value {
        let kind = match e {
            Error::Value(_) => "ValueError",
            Error::Exit { code: 2, message } if message.starts_with("ticket form validation") => {
                "PropertyError"
            }
            Error::Exit { code: 2, .. } => "ReviewedWriteError",
            other => panic!("unexpected error {other:?}"),
        };
        json!({"type": kind, "msg": e.to_string()})
    }

    #[test]
    fn bulk_status_set_matches_python() {
        let cases = SET["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 10);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let writes = Arc::new(Mutex::new(Vec::new()));
            let c = set_api(case["fail_on"].as_i64(), Arc::clone(&writes));
            let action = case["action"].as_str().unwrap();
            let mut bulk = args(action);
            bulk.status = (action == "status").then(|| case["status"].as_str().unwrap());
            let built = build_plan_with_answers(
                &c,
                &store(),
                &MemJournal::default(),
                &["71".into(), "72".into()],
                &bulk,
                None,
                case["answers"].as_object(),
            );
            let plan = match built {
                Err(e) => {
                    assert_eq!(json!({"error": error_json(&e)}), case["build"], "{name}");
                    assert!(writes.lock().unwrap().is_empty(), "{name}");
                    continue;
                }
                Ok(p) => p,
            };
            let w = &plan.write;
            let got = json!({"write": {"action": w.action, "target": w.target, "tenant": w.tenant,
                    "method": w.method, "path": w.path, "payload_json": w.payload_json,
                    "context_hash": w.context_hash},
                "write_hash": w.hash(), "intent_json": plan.intent_json, "ids": plan.ids,
                "expected_json": plan.expected_json, "selected_json": plan.selected_json,
                "hash": plan.hash(), "review": plan.review()});
            assert_eq!(got, case["build"], "{name} build");
            if let Some(want) = case.get("send") {
                let journal = MemJournal::default();
                let sent = send(&plan, &c, &store(), &journal, Some(&plan.hash()), false);
                let sent = sent.map_or_else(
                    |e| json!({"error": error_json(&e)}),
                    |r| json!({"result": r}),
                );
                assert_eq!(&sent, want, "{name} send");
                let replay = send(&plan, &c, &store(), &journal, Some(&plan.hash()), false);
                assert_eq!(
                    json!({"error": error_json(&replay.unwrap_err())}),
                    case["replay"],
                    "{name} replay"
                );
                let events: Vec<Value> = journal
                    .read()
                    .into_iter()
                    .filter(|r| r["plan"] == json!(w.hash()))
                    .map(|r| r["event"].clone())
                    .collect();
                assert_eq!(Value::Array(events), case["journal"], "{name} journal");
            }
            assert_eq!(
                Value::Array(writes.lock().unwrap().clone()),
                case["writes"],
                "{name} writes"
            );
        }
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
            let plan = build_plan(
                &c,
                &store(),
                &MemJournal::default(),
                &["71".into(), "72".into()],
                &args(action),
            )
            .unwrap();
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
            let err =
                build_plan(&c, &store(), &MemJournal::default(), &ids, &args("spam")).unwrap_err();
            assert!(msg(&err).contains(needle), "{}", msg(&err));
        }
        let mut select_all = args("spam");
        select_all.select_all = true;
        let err = build_plan(&c, &store(), &MemJournal::default(), &[], &select_all).unwrap_err();
        assert!(msg(&err).contains("--select-all requires an explicit filter"));
        let where_ = ["created_at>=2026-01-01".to_string()];
        let mut both = args("spam");
        both.where_ = &where_;
        both.select_all = true;
        let err =
            build_plan(&c, &store(), &MemJournal::default(), &["71".into()], &both).unwrap_err();
        assert!(msg(&err).contains("filters require --select-all"));
        let mut filtered = args("spam");
        filtered.where_ = &where_;
        let err = build_plan(&c, &store(), &MemJournal::default(), &[], &filtered).unwrap_err();
        assert!(msg(&err).contains("filters require --select-all"));
        let err = build_plan(
            &c,
            &store(),
            &MemJournal::default(),
            &["71".into()],
            &args("nope"),
        )
        .unwrap_err();
        assert!(msg(&err).contains("unsupported bulk action"));
        let mut status = args("spam");
        status.status = Some("Pending");
        let err = build_plan(
            &c,
            &store(),
            &MemJournal::default(),
            &["71".into()],
            &status,
        )
        .unwrap_err();
        assert!(msg(&err).contains("options do not match"));
        let mut resolved = args("status");
        resolved.status = Some("Resolved");
        let err = build_plan(
            &c,
            &store(),
            &MemJournal::default(),
            &["71".into()],
            &resolved,
        )
        .unwrap_err();
        assert!(msg(&err).contains("bulk closure"), "{}", msg(&err));
        let mut assign = args("assign");
        assign.agent = Some("91");
        let err = build_plan(
            &c,
            &store(),
            &MemJournal::default(),
            &["71".into()],
            &assign,
        )
        .unwrap_err();
        assert!(
            msg(&err).contains("both --agent and --group"),
            "{}",
            msg(&err)
        );
        let mut bare = row(71);
        bare.as_object_mut().unwrap().shift_remove("spam");
        state.lock().unwrap().rows.insert(71, bare);
        let err = build_plan(
            &c,
            &store(),
            &MemJournal::default(),
            &["71".into()],
            &args("spam"),
        )
        .unwrap_err();
        assert!(msg(&err).contains("baseline flag"), "{}", msg(&err));
        assert!(posts(&c).is_empty());
    }

    #[test]
    fn delete_after_restore_plans_fresh_hash_but_true_replay_refuses() {
        for restore in ["fsv", "web-bumps-updated_at"] {
            let state = Arc::new(Mutex::new(world()));
            let c = api(Arc::clone(&state));
            let journal = MemJournal::default();
            let ids = ["INC-71".to_string()];
            let plan = || build_plan(&c, &store(), &journal, &ids, &args("delete")).unwrap();
            let quiet = |p: &BulkPlan| {
                crate::readback::with_readback_sleep(
                    |_| {},
                    || send(p, &c, &store(), &journal, Some(&p.hash()), true),
                )
            };
            let undelete = || {
                let mut s = state.lock().unwrap();
                s.rows.get_mut(&71).unwrap()["deleted"] = json!(false);
            };
            let first = plan();
            assert_eq!(quiet(&first).unwrap()["verified"], json!(true), "{restore}");
            undelete();
            let same = plan();
            assert_eq!(same.hash(), first.hash(), "{restore}");
            let err = quiet(&same).unwrap_err();
            assert!(msg(&err).contains("was sent"), "{restore}: {}", msg(&err));
            if restore == "fsv" {
                journal.record_plan(
                    "done",
                    &"0".repeat(64),
                    "ticket_restore",
                    Some(&json!(71)),
                    None,
                );
            } else {
                state.lock().unwrap().rows.get_mut(&71).unwrap()["updated_at"] =
                    json!("2030-01-01T00:00:09Z");
            }
            let again = plan();
            assert_ne!(again.hash(), first.hash(), "{restore}");
            let writes = posts(&c).len();
            assert_eq!(quiet(&again).unwrap()["verified"], json!(true), "{restore}");
            assert_eq!(posts(&c).len(), writes + 1, "{restore}");
            undelete();
            let replay = plan();
            let err = crate::readback::with_readback_sleep(
                |_| {},
                || send(&replay, &c, &store(), &journal, Some(&again.hash()), true),
            )
            .unwrap_err();
            assert!(msg(&err).contains("was sent"), "{restore}: {}", msg(&err));
            assert_eq!(posts(&c).len(), writes + 1, "{restore}");
        }
    }

    /// `tests/test_ticket_bulk_scenario.py`.
    fn scenario_api(writes: Arc<Mutex<Vec<(String, String, Value)>>>) -> FnApi {
        let mut a = FnApi::new(move |call: &Call| {
            if call.method == "GET" {
                if call.path == "scenario_automations" {
                    return Ok(json!({"scenario_automations": [
                        {"id": 501, "name": "Acme convert to request", "visibility": "shared"},
                        {"id": 502, "name": "Assign to Acme Team", "visibility": "shared"},
                        {"id": 503, "name": "assign to acme team", "visibility": "personal"},
                        {"id": 504, "name": "502", "visibility": "shared"}]}));
                }
                let tid: i64 = call.path.split('/').nth(1).unwrap().parse().unwrap();
                return Ok(
                    json!({"ticket": {"id": tid, "subject": format!("Acme {tid}"),
                                            "spam": false, "deleted": false}}),
                );
            }
            writes.lock().unwrap().push((
                call.method.clone(),
                call.path.clone(),
                call.body.clone().unwrap_or(Value::Null),
            ));
            Ok(json!({}))
        });
        a.domain = "acme.example.com".into();
        a
    }

    fn scenario_args<'a>(action: &'a str, where_: &'a [String], select_all: bool) -> BulkArgs<'a> {
        BulkArgs {
            action,
            where_,
            select_all,
            status: None,
            agent: None,
            group: None,
            primary: None,
            note_primary: None,
            note_secondary: None,
            schema: None,
        }
    }

    #[test]
    fn scenario_exact_name_resolution_and_ambiguity() {
        let c = scenario_api(Arc::default());
        let id = |v: &str| resolve_scenario(&c, v).map(|r| r["id"].clone());
        assert_eq!(id("Acme convert to request").unwrap(), 501);
        assert_eq!(id("501").unwrap(), 501);
        assert_eq!(id("Assign to Acme Team").unwrap(), 502);
        let e = id("ASSIGN TO ACME TEAM").unwrap_err().to_string();
        assert_eq!(
            e,
            "scenario 'ASSIGN TO ACME TEAM' must match exactly one scenario by ID or exact name (matches: 502: 'Assign to Acme Team', 503: 'assign to acme team'); nothing was sent"
        );
        assert!(id("502").unwrap_err().to_string().contains("exactly one"));
        assert!(
            id("Assign")
                .unwrap_err()
                .to_string()
                .contains("(matches: none)")
        );
    }

    #[test]
    fn scenario_plan_payload_review_and_unverified_202() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let c = scenario_api(Arc::clone(&writes));
        let (st, j) = (store(), MemJournal::default());
        let ids = ["INC-31".to_string(), "30".to_string()];
        let args = scenario_args("scenario", &[], false);
        let plan = build_plan_scenario(
            &c,
            &st,
            &j,
            &ids,
            &args,
            None,
            None,
            Some("Acme convert to request"),
        )
        .unwrap();
        assert_eq!(
            plan.write.payload(),
            json!({"scenario_id": 501, "ids": [30, 31]})
        );
        let review = plan.review();
        assert_eq!(
            review["scenario"],
            json!({"id": 501, "name": "Acme convert to request"})
        );
        let warning = review["warning"].as_str().unwrap();
        assert!(
            warning.contains("cannot be proven") && warning.contains("Acme convert to request")
        );
        let result = crate::readback::with_readback_sleep(
            |_| {},
            || send(&plan, &c, &st, &j, Some(&plan.hash()), false),
        )
        .unwrap();
        assert_eq!(result["verified"], false);
        assert_eq!(result["identity_verified"], json!({"30": true, "31": true}));
        assert_eq!(
            *writes.lock().unwrap(),
            [(
                "PUT".to_string(),
                "tickets/execute_scenario".to_string(),
                json!({"scenario_id": 501, "ids": [30, 31]})
            )]
        );
        let again = send(&plan, &c, &st, &j, Some(&plan.hash()), false).unwrap_err();
        assert!(again.to_string().contains("sent"));
        assert_eq!(writes.lock().unwrap().len(), 1);
    }

    #[test]
    fn scenario_filters_and_option_mismatch_refused() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let c = scenario_api(Arc::clone(&writes));
        let (st, j) = (store(), MemJournal::default());
        let where_ = ["status=Open".to_string()];
        let e = build_plan_scenario(
            &c,
            &st,
            &j,
            &[],
            &scenario_args("scenario", &where_, true),
            None,
            None,
            Some("501"),
        )
        .unwrap_err();
        assert!(e.to_string().contains("explicit"));
        let ids = ["30".to_string()];
        let e = build_plan_scenario(
            &c,
            &st,
            &j,
            &ids,
            &scenario_args("scenario", &[], false),
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(e.to_string().contains("--scenario"));
        let e = build_plan_scenario(
            &c,
            &st,
            &j,
            &ids,
            &scenario_args("spam", &[], false),
            None,
            None,
            Some("501"),
        )
        .unwrap_err();
        assert!(e.to_string().contains("--scenario"));
        assert!(writes.lock().unwrap().is_empty());
    }
}
