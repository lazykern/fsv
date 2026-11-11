//! Reviewed change-task CREATE from a blocked UI request (`change_tasks.py`).
//! The response contract is unobserved: never replay a POST to recover an id.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, parse_id};
use fsv_forms::pydt::{DateTimeValue, datetime_fromisoformat};

use crate::associations::SendOpts;
use crate::service::{Api, Journal, change_tab, definite_rejection, dumps_bytes, escape_lines, has_control, last_event, obj, plan_hash, positive_id};

pub const STATUS_LABELS: &[&str] = &["Open", "In Progress", "Completed", "Blocked"];
pub const NOTIFY_BEFORE: &[(&str, i64)] =
    &[("Never", 0), ("15 Minutes", 900), ("30 Minutes", 1800), ("45 Minutes", 2700), ("1 Hour", 3600), ("2 Hours", 7200)];
const ACTION: &str = "change_task_create";
const UNKNOWN: &str = "task outcome unknown; inspect the change and journal before any retry; do not resubmit";

static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+$").expect("regex"));

/// `TaskError`: a safe user-facing failure (the CLI exits 2).
fn refused(msg: impl Into<String>) -> Error {
    Error::exit(2, msg)
}

fn casefold(v: &str) -> String {
    fsv_core::pystr::casefold(v)
}

fn str_at(row: &Value, key: &str) -> String {
    row.get(key).map(fsv_py::py_str).unwrap_or_default()
}

/// `resolve_owner`: no implicit self; a positive user id or one exact active agent email.
pub fn resolve_owner(c: &dyn Api, value: Option<&str>) -> Result<Option<i64>> {
    let Some(value) = value.filter(|v| !v.is_empty()) else { return Ok(None) };
    if let Some(id) = positive_id(&json!(value)) {
        return Ok(Some(id));
    }
    if !EMAIL.is_match(value) {
        return Err(Error::value("owner must be a positive user ID or exact email (not 'me')"));
    }
    let rows = c
        .autocomplete("agents", value, None)
        .map_err(|_| refused("could not resolve owner email; no task POST was attempted; use an explicit user ID if unavailable"))?;
    let wanted = casefold(value);
    let matches: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            r.is_object()
                && casefold(&str_at(r, "email")) == wanted
                && !r.get("deleted").is_some_and(fsv_py::truthy)
                && !r.get("deactivated").is_some_and(fsv_py::truthy)
                && !str_at(r, "value").contains("(Deactivated)")
        })
        .collect();
    match matches.as_slice() {
        [one] => match one.get("user_id").and_then(positive_id) {
            Some(id) => Ok(Some(id)),
            None => Err(refused("owner email must match exactly one active agent with a user ID; use an explicit user ID if unavailable")),
        },
        _ => Err(refused("owner email must match exactly one active agent with a user ID; use an explicit user ID if unavailable")),
    }
}

/// `resolve_group`: a positive id or one exact group name.
pub fn resolve_group(c: &dyn Api, value: Option<&str>) -> Result<Option<i64>> {
    let Some(value) = value.filter(|v| !fsv_core::pystr::strip(v).is_empty()) else { return Ok(None) };
    if let Some(id) = positive_id(&json!(value)) {
        return Ok(Some(id));
    }
    let fail = || refused("group must resolve to exactly one exact name or be a positive ID; no POST attempted");
    let data = c.int_get("bootstrap/agents_groups", None).map_err(|_| fail())?;
    let rows = data.get("groups").and_then(Value::as_array).ok_or_else(fail)?;
    let wanted = casefold(value);
    let matches: Vec<&Value> = rows.iter().filter(|r| r.is_object() && casefold(&str_at(r, "name")) == wanted).collect();
    match matches.as_slice() {
        [one] => one.get("id").and_then(positive_id).map(Some).ok_or_else(fail),
        _ => Err(fail()),
    }
}

/// `status_label`: a nonempty runtime label or id.
pub fn status_label(value: &str) -> Result<String> {
    let t = fsv_core::pystr::strip(value);
    if t.is_empty() || value.chars().any(|c| (c as u32) < 32) {
        return Err(Error::value("status must be a nonempty runtime label or ID"));
    }
    Ok(t.to_string())
}

/// `resolve_status`: the exact runtime task-status id; nothing is inferred.
pub fn resolve_status(c: &dyn Api, value: &str) -> Result<i64> {
    let label = status_label(value)?;
    let resolved = c
        .int_get("change_task_fields", Some(obj(json!({"include_deleted_choices": "true"}))))
        .ok()
        .and_then(|d| crate::task_status::fields_from_payload(&d).ok())
        .and_then(|fields| crate::task_status::resolve(&label, &fields).ok());
    resolved.map(|c| c.0).ok_or_else(|| refused(format!("{label} requires an exact runtime task-status metadata ID; no ID inferred and no POST attempted")))
}

/// `_date`: ISO-8601 date/time with an explicit offset and whole seconds,
/// normalized with `isoformat(timespec="seconds")`.
pub fn date(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|v| !fsv_core::pystr::strip(v).is_empty()) else { return Ok(None) };
    let bad = || Error::value("planned dates require ISO-8601 date/time with explicit offset, whole seconds (no host timezone inferred)");
    if !value.contains('T') {
        return Err(bad());
    }
    let (dt, off) = datetime_fromisoformat(&value.replace('Z', "+00:00")).ok_or_else(bad)?;
    let off = off.ok_or_else(bad)?;
    if dt.subsec_nanosecond() != 0 || off.seconds() % 60 != 0 {
        return Err(bad());
    }
    Ok(Some(DateTimeValue::new(dt, off).isoformat_seconds()))
}

/// An aware instant or a naive wall time, for Python `datetime ==`.
fn instant(text: &str) -> Option<(jiff::civil::DateTime, Option<jiff::Timestamp>)> {
    let (dt, off) = datetime_fromisoformat(text)?;
    Some((dt, off.and_then(|o| o.to_timestamp(dt).ok())))
}

/// `datetime.fromisoformat(a) == datetime.fromisoformat(b)`; `None` on a parse error.
pub(crate) fn same_datetime(a: &str, b: &str) -> Option<bool> {
    let (x, y) = (instant(a)?, instant(b)?);
    Some(match (x.1, y.1) {
        (Some(p), Some(q)) => p == q,
        (None, None) => x.0 == y.0,
        _ => false,
    })
}

/// `_effort`: `1h30m` or `01:30`, sent as `1h 30m`.
pub fn effort(value: Option<&str>) -> Result<Option<String>> {
    static CLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]+):([0-5][0-9])$").expect("regex"));
    static UNITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:(\d+)h)?\s*(?:(\d+)m)?$").expect("regex"));
    let Some(value) = value.filter(|v| !fsv_core::pystr::strip(v).is_empty()) else { return Ok(None) };
    let raw = fsv_core::pystr::strip(value);
    let num = |m: Option<regex::Match<'_>>| m.and_then(|m| fsv_core::pystr::parse_decimal(m.as_str())).unwrap_or(0);
    let (hours, minutes) = if let Some(c) = CLOCK.captures(raw) {
        (num(c.get(1)), num(c.get(2)))
    } else if let Some(u) = UNITS.captures(raw).filter(|u| u.get(1).is_some() || u.get(2).is_some()) {
        (num(u.get(1)), num(u.get(2)))
    } else {
        return Err(Error::value("planned effort must be duration text (1h30m) or HH:MM (01:30)"));
    };
    let total = hours * 60 + minutes;
    Ok(Some(format!("{}h {}m", total / 60, total % 60)))
}

/// `_note`: plain text without unsafe control characters.
pub fn note(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|v| !fsv_core::pystr::strip(v).is_empty()) else { return Ok(None) };
    if has_control(value, "\r\n\t") {
        return Err(Error::value("note must be plain text without unsafe control characters"));
    }
    Ok(Some(value.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPlan {
    pub change_id: i64,
    pub title: String,
    pub owner_id: Option<i64>,
    pub tenant: String,
    pub group_id: Option<i64>,
    pub status_label: String,
    pub status_id: i64,
    pub note: Option<String>,
    pub notify_before: String,
    pub planned_start: Option<String>,
    pub planned_end: Option<String>,
    pub planned_effort: Option<String>,
}

impl TaskPlan {
    pub fn path(&self) -> String {
        format!("changes/{}/tasks", self.change_id)
    }

    pub fn payload(&self) -> Map<String, Value> {
        let notify = NOTIFY_BEFORE.iter().find(|(k, _)| *k == self.notify_before).map_or(0, |(_, v)| *v);
        let mut body = obj(json!({"title": self.title, "status": self.status_id, "notify_before": notify}));
        if let Some(n) = &self.note {
            body.insert("description".into(), json!(format!("<div><p>{}</p></div>", escape_lines(n))));
        }
        if let Some(g) = self.group_id {
            body.insert("group_id".into(), json!(g));
        }
        for (key, value) in [("planned_start_date", &self.planned_start), ("planned_end_date", &self.planned_end), ("planned_effort", &self.planned_effort)] {
            if let Some(v) = value {
                body.insert(key.into(), json!(v));
            }
        }
        if let Some(o) = self.owner_id {
            body.insert("owner_id".into(), json!(o));
        }
        body
    }

    /// Default-status plans keep their historical hash (no `status_label`).
    pub fn hash(&self) -> String {
        let mut body = obj(json!({"tenant": self.tenant, "path": self.path(), "payload": self.payload()}));
        if !["Open", "In Progress"].contains(&self.status_label.as_str()) {
            body.insert("status_label".into(), json!(self.status_label));
        }
        plan_hash(&Value::Object(body))
    }

    pub fn review(&self) -> Map<String, Value> {
        obj(json!({
            "action": ACTION, "tenant": self.tenant, "change_id": self.change_id,
            "method": "POST", "path": format!("/api/_/{}", self.path()), "payload": self.payload(),
            "owner": self.owner_id, "status_label": self.status_label, "reminder": self.notify_before,
            "note_text": self.note, "plan_hash": self.hash(),
            "warning": "Task creation/assignment may notify people or trigger tenant workflows. Never is a reminder setting, not a no-notification guarantee.",
            "contract": "Blocked request captured; accepted response unobserved. Blank optional fields omitted.",
        }))
    }
}

/// Keyword arguments of `build_plan` (Python defaults via [`Default`]).
#[derive(Debug, Clone)]
pub struct TaskArgs<'a> {
    pub owner_id: Option<i64>,
    pub group_id: Option<i64>,
    pub status: &'a str,
    pub status_id: Option<i64>,
    pub note: Option<&'a str>,
    pub notify_before: &'a str,
    pub planned_start: Option<&'a str>,
    pub planned_end: Option<&'a str>,
    pub planned_effort: Option<&'a str>,
}

impl Default for TaskArgs<'_> {
    fn default() -> Self {
        TaskArgs {
            owner_id: None,
            group_id: None,
            status: "Open",
            status_id: None,
            note: None,
            notify_before: "Never",
            planned_start: None,
            planned_end: None,
            planned_effort: None,
        }
    }
}

/// `build_plan`: pure local validation.
pub fn build_plan(change_id: &str, title: &str, args: &TaskArgs<'_>, tenant: &str) -> Result<TaskPlan> {
    let cid = parse_id(change_id, &CHANGES)?;
    if cid <= 0 {
        return Err(Error::value("change ID must be positive"));
    }
    if fsv_core::pystr::strip(title).is_empty() || has_control(title, "") {
        return Err(Error::value("task title must be nonempty text without control characters"));
    }
    if args.owner_id.is_some_and(|o| o <= 0) {
        return Err(Error::value("owner ID must be a positive integer or none"));
    }
    if args.group_id.is_some_and(|g| g <= 0) {
        return Err(Error::value("group ID must be a positive integer or none"));
    }
    let label = status_label(args.status)?;
    let Some(status_id) = args.status_id.filter(|s| *s > 0) else {
        return Err(Error::value("status ID must first be resolved from runtime metadata"));
    };
    let wanted = casefold(args.notify_before);
    let Some((reminder, _)) = NOTIFY_BEFORE.iter().find(|(k, _)| casefold(k) == wanted) else {
        let names: Vec<&str> = NOTIFY_BEFORE.iter().map(|(k, _)| *k).collect();
        return Err(Error::value(format!("notify before must be {}", names.join(", "))));
    };
    let (start, end) = (date(args.planned_start)?, date(args.planned_end)?);
    if let (Some(s), Some(e)) = (&start, &end) {
        if instant(e).and_then(|x| x.1) < instant(s).and_then(|x| x.1) {
            return Err(Error::value("planned end must not precede planned start"));
        }
    }
    Ok(TaskPlan {
        change_id: cid,
        title: title.into(),
        owner_id: args.owner_id,
        tenant: tenant.into(),
        group_id: args.group_id,
        status_label: label,
        status_id,
        note: note(args.note)?,
        notify_before: (*reminder).into(),
        planned_start: start,
        planned_end: end,
        planned_effort: effort(args.planned_effort)?,
    })
}

/// `_inventory`: the complete task list of the plan's change.
pub fn inventory(c: &dyn Api, plan: &TaskPlan) -> Result<Vec<Value>> {
    let tab = change_tab(|p, q| c.int_get(p, q), plan.change_id, "tasks")?;
    let pages = tab.meta.get("pages").and_then(Value::as_i64).unwrap_or(0);
    if (tab.missing.is_some() && pages == 0) || !tab.complete() {
        return Err(refused("task inventory is incomplete"));
    }
    if tab.rows.iter().any(|r| !r.is_object() || r.get("id").and_then(positive_id).is_none()) {
        return Err(refused("task inventory contains an invalid task ID"));
    }
    Ok(tab.rows)
}

/// `_matches`: same title and owner (an absent owner key never matches).
pub fn matches(row: &Value, plan: &TaskPlan) -> bool {
    let Some(owner) = row.get("owner_id") else { return false };
    row.get("title") == Some(&json!(plan.title))
        && match plan.owner_id {
            None => owner.is_null(),
            Some(o) => positive_id(owner) == Some(o),
        }
}

/// `_compare_fields`: `(verified, unverified, mismatched)` payload keys.
pub fn compare_fields(exact: Option<&Value>, plan: &TaskPlan) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut ok, mut unverified, mut mismatched) = (Vec::new(), Vec::new(), Vec::new());
    for (key, expected) in plan.payload() {
        let Some(observed) = exact.and_then(|e| e.get(&key)) else {
            unverified.push(key);
            continue;
        };
        let matched = match key.as_str() {
            "group_id" | "owner_id" => positive_id(observed).map(|i| json!(i)).as_ref() == Some(&expected),
            "planned_start_date" | "planned_end_date" => match (observed.as_str(), expected.as_str()) {
                (Some(o), Some(e)) => same_datetime(&o.replace('Z', "+00:00"), e).unwrap_or(false),
                _ => false,
            },
            _ => *observed == expected,
        };
        if matched { ok.push(key) } else { mismatched.push(key) }
    }
    (ok, unverified, mismatched)
}

/// `_returned_id`: the id of a `task` envelope or the top level.
pub fn returned_id(response: &Value) -> Option<i64> {
    if !response.is_object() {
        return None;
    }
    match response.get("task") {
        Some(task) if task.is_object() => task.get("id").and_then(positive_id),
        _ => response.get("id").and_then(positive_id),
    }
}

fn args_of(plan: &TaskPlan) -> TaskArgs<'_> {
    TaskArgs {
        owner_id: plan.owner_id,
        group_id: plan.group_id,
        status: &plan.status_label,
        status_id: Some(plan.status_id),
        note: plan.note.as_deref(),
        notify_before: &plan.notify_before,
        planned_start: plan.planned_start.as_deref(),
        planned_end: plan.planned_end.as_deref(),
        planned_effort: plan.planned_effort.as_deref(),
    }
}

/// `send`: one reviewed POST; a missing id falls back to a complete
/// before/after inventory. Ambiguity leaves the journal `started`.
pub fn send(plan: &TaskPlan, c: &dyn Api, journal: &dyn Journal, opts: SendOpts<'_>) -> Result<Value> {
    let rebuilt = build_plan(&plan.change_id.to_string(), &plan.title, &args_of(plan), &c.domain());
    if rebuilt.ok().as_ref() != Some(plan) {
        return Err(refused("task plan or tenant changed; review a new plan"));
    }
    if opts.dry_run {
        let mut out = obj(json!({"status": "dry_run"}));
        out.extend(plan.review());
        return Ok(Value::Object(out));
    }
    if opts.read_only {
        return Err(refused("read-only mode: no task was sent"));
    }
    let Some(reviewed) = opts.reviewed_hash else {
        return Ok(json!({"status": "cancelled", "change_id": plan.change_id}));
    };
    let hash = plan.hash();
    if reviewed != hash {
        return Err(refused("task review hash does not match; review the exact task again"));
    }
    if plan.tenant.is_empty() {
        return Err(refused("configure a tenant before creating a task"));
    }
    if matches!(last_event(journal, ACTION, &hash).as_deref(), Some("started" | "done")) {
        return Err(refused("this task was sent or its outcome is unknown; inspect the change and journal; do not resubmit"));
    }
    let prepared = inventory(c, plan).and_then(|rows| c.get_csrf().map(|t| (rows, t)));
    let Ok((before_rows, csrf)) = prepared else {
        return Err(refused("could not prepare a complete task inventory/session; no task POST was attempted"));
    };
    let before: BTreeSet<i64> = before_rows.iter().filter_map(|r| r.get("id").and_then(positive_id)).collect();
    let target = json!(plan.change_id);
    journal.record_plan("started", &hash, ACTION, Some(&target), None);
    if last_event(journal, ACTION, &hash).as_deref() != Some("started") {
        return Err(refused("could not persist task journal; no task was sent"));
    }
    let payload = Value::Object(plan.payload());
    let response = match c.send_once("POST", &plan.path(), Body::Raw(dumps_bytes(&payload, false)), None, &csrf) {
        Ok(r) => r,
        Err(e) => {
            if let Some(status) = definite_rejection(&e) {
                journal.record_plan("failed", &hash, ACTION, Some(&target), None);
                return Err(refused(format!("task rejected (HTTP {status}); no automatic retry")));
            }
            return Err(refused(UNKNOWN));
        }
    };
    let mut tid = returned_id(&response);
    let mut exact: Option<Value> = None;
    if let Ok(after) = inventory(c, plan) {
        let id = |r: &Value| r.get("id").and_then(positive_id);
        match tid {
            None => {
                let candidates: Vec<&Value> = after
                    .iter()
                    .filter(|r| !id(r).is_some_and(|i| before.contains(&i)) && matches(r, plan) && compare_fields(Some(r), plan).2.is_empty())
                    .collect();
                if let [one] = candidates.as_slice() {
                    exact = Some((*one).clone());
                    tid = id(one);
                }
            }
            Some(t) => exact = after.iter().find(|r| id(r) == Some(t)).cloned(),
        }
    }
    let Some(tid) = tid else { return Err(refused(UNKNOWN)) };
    journal.record_plan("done", &hash, ACTION, Some(&target), Some(&json!(tid)));
    let identity_verified = exact.as_ref().is_some_and(|e| !before.contains(&tid) && matches(e, plan));
    let (verified_fields, unverified_fields, mismatched_fields) = compare_fields(exact.as_ref(), plan);
    let verified = identity_verified && mismatched_fields.is_empty();
    let mut result = obj(json!({
        "status": "submitted", "change_id": plan.change_id, "task_id": tid,
        "verified": verified, "identity_verified": identity_verified,
        "fields_verified": verified && unverified_fields.is_empty(),
        "verified_fields": verified_fields, "unverified_fields": unverified_fields,
        "mismatched_fields": mismatched_fields, "readback": exact, "plan_hash": hash,
    }));
    if !verified {
        result.insert("warning".into(), json!("Returned task ID/state not verified in task read-back; do not resend."));
    } else if !unverified_fields.is_empty() {
        result.insert(
            "warning".into(),
            json!(format!("Task identity verified, but fields absent from GET remain unverified: {}; do not resend.", unverified_fields.join(", "))),
        );
    }
    Ok(Value::Object(result))
}
