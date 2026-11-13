//! Reviewed ticket-side associations: problems, changes (both directions)
//! and child tickets, using the wires the ticket page sends
//! (`ticket_associations.py`).
//!
//! * problem link/unlink: `PUT tickets/{id}/problems/{pid}`
//!   `{"type": "link"|"unlink"}`; readback `GET tickets/{id}/problems`.
//! * change link: `PUT tickets/{id}/changes/associate`
//!   `{"item_ids": [cid], "change_type": "change_cause"|"change", "type": "link"}`;
//!   unlink `PUT tickets/{id}/changes/{cid}` `{"type": "unlink", "change_type": ...}`;
//!   readback `GET tickets/{id}/changes?change_type=...` (the type is mandatory).
//! * child link: `POST tickets/{id}/associate-child-tickets` `{"ticket_ids": [child]}`;
//!   unlink `DELETE tickets/{id}/remove_child?ticket_id=child` (204);
//!   readback `GET tickets/{id}/child-tickets?type=active` plus the
//!   `related_tickets` tab `hierarchy` (2 parent, 1 child, 0 none): the
//!   child list answers 400 on a ticket that is not a parent.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{CHANGES, PROBLEMS, TICKETS, parse_id};
use fsv_py::{py_str, truthy};

use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, snapshot_hash};
use crate::watchers::positive_id;

pub const KIND_HELP: &str = "initiating (change_cause: the change caused this ticket) | initiated-by (change: this ticket initiated the change)";

pub const PARENT: i64 = 2;
pub const CHILD: i64 = 1;
pub const NONE: i64 = 0;

/// `HIERARCHY`.
fn hierarchy_name(level: i64) -> &'static str {
    match level {
        PARENT => "parent",
        CHILD => "child",
        _ => "none",
    }
}

/// `CHANGE_TYPE_LABELS`.
fn change_type_label(ctype: &str) -> &'static str {
    if ctype == "change_cause" {
        "change initiating this ticket"
    } else {
        "change initiated by this ticket"
    }
}

/// Port of fsv.ticket_associations.LinkPlan: a [`WritePlan`] with display
/// fields that are not part of the hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPlan {
    pub plan: WritePlan,
    pub kind: String,
    pub item: i64,
    pub change_type: Option<String>,
}

impl LinkPlan {
    pub fn hash(&self) -> String {
        self.plan.hash()
    }

    /// Port of fsv.ticket_associations.LinkPlan.review.
    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.plan.review();
        out.insert("ticket_id".into(), json!(self.plan.target));
        out.insert("kind".into(), json!(self.kind));
        out.insert("item_id".into(), json!(self.item));
        if let Some(ct) = self.change_type.as_deref().filter(|c| !c.is_empty()) {
            out.insert("change_type".into(), json!(ct));
            out.insert("meaning".into(), json!(change_type_label(ct)));
        }
        out
    }
}

/// Port of fsv.ticket_associations.change_type.
pub fn change_type(value: &str) -> Result<String> {
    Ok(match casefold(strip(value)).as_str() {
        "initiating" | "change_cause" | "cause" => "change_cause",
        "initiated-by" | "initiated_by" | "change" => "change",
        _ => {
            return Err(Error::value(
                "--kind must be initiating (change_cause) or initiated-by (change)",
            ));
        }
    }
    .to_string())
}

fn not_found(e: Error, what: &str) -> Error {
    if e.status() == Some(404) {
        refused(format!("{what} not found (HTTP 404); nothing was sent"))
    } else {
        e
    }
}

/// Port of fsv.ticket_associations._read.
fn read(c: &dyn Api, path: &str, what: &str) -> Result<Value> {
    c.int_get(path, None).map_err(|e| not_found(e, what))
}

/// Port of fsv.ticket_associations._exact.
fn exact(c: &dyn Api, module: &str, ident: i64) -> Result<Map<String, Value>> {
    if module == "tickets" {
        return crate::properties::record(c, ident).map_err(|e| match e {
            Error::Api(_) => not_found(e, &format!("ticket {ident}")),
            // PropertyError (and anything else): exact identity
            other => refused(other.to_string()),
        });
    }
    let singular = &module[..module.len() - 1];
    let raw = read(
        c,
        &format!("{module}/{ident}"),
        &format!("{singular} {ident}"),
    )?;
    let row = match raw {
        Value::Object(mut m) => match m.remove(singular) {
            Some(inner) => inner,
            None => Value::Object(m),
        },
        _ => Value::Null,
    };
    match row {
        Value::Object(r) if py_str(r.get("id").unwrap_or(&Value::Null)) == ident.to_string() => {
            Ok(r)
        }
        _ => Err(refused(format!(
            "{singular} read did not identify the exact target; nothing was sent"
        ))),
    }
}

/// Port of fsv.ticket_associations._complete.
fn complete(
    c: &dyn Api,
    path: &str,
    key: &str,
    params: Option<&Map<String, Value>>,
    what: &str,
) -> Result<Vec<Value>> {
    let pages = collect_pages(
        |p, q| c.int_get(p, q),
        path,
        key,
        params,
        what,
        MAX_DETAIL_PAGES,
    )?;
    if pages.missing.is_some() || !pages.complete() {
        return Err(refused(format!(
            "complete {what} list unavailable; nothing was sent"
        )));
    }
    let ids: Vec<String> = pages
        .rows
        .iter()
        .map(|r| py_str(r.get("id").unwrap_or(&Value::Null)))
        .collect();
    let unique: BTreeSet<&String> = ids.iter().collect();
    if ids.iter().any(|i| i == "None" || i.is_empty()) || unique.len() != ids.len() {
        return Err(refused(format!("{what} list has missing or duplicate IDs")));
    }
    Ok(pages.rows)
}

/// Port of fsv.ticket_associations.problems.
pub fn problems(c: &dyn Api, ticket: i64) -> Result<Vec<Value>> {
    complete(
        c,
        &format!("tickets/{ticket}/problems"),
        "problems",
        None,
        "associated problems",
    )
}

/// Port of fsv.ticket_associations.changes.
pub fn changes(c: &dyn Api, ticket: i64, ctype: &str) -> Result<Vec<Value>> {
    complete(
        c,
        &format!("tickets/{ticket}/changes"),
        "changes",
        Some(&obj(json!({"change_type": ctype}))),
        "associated changes",
    )
}

/// Port of fsv.ticket_associations.hierarchy: the `related_tickets` tab
/// `hierarchy` of the ticket.
pub fn hierarchy(c: &dyn Api, ticket: i64) -> Result<i64> {
    let raw = read(
        c,
        &format!("tickets/{ticket}/tabs"),
        &format!("ticket {ticket} tabs"),
    )?;
    let tabs = match &raw {
        Value::Array(items) => Some(items.as_slice()),
        Value::Object(m) => m.get("tabs").and_then(Value::as_array).map(Vec::as_slice),
        _ => None,
    };
    let mut found: Vec<&Value> = tabs
        .unwrap_or_default()
        .iter()
        .filter(|t| t.get("name").and_then(Value::as_str) == Some("related_tickets"))
        .filter(|t| t.is_object())
        .collect();
    if found.is_empty()
        && let Some(tab) = raw.get("related_tickets").filter(|t| t.is_object())
    {
        found.push(tab);
    }
    let mut values: Vec<&Value> = Vec::new();
    for tab in found {
        let value = tab.get("hierarchy").unwrap_or(&Value::Null);
        if !values.contains(&value) {
            values.push(value);
        }
    }
    if values.len() != 1 {
        return Err(refused(
            "ticket hierarchy (related_tickets tab) unavailable; nothing was sent",
        ));
    }
    match values[0].as_i64() {
        Some(level @ (PARENT | CHILD | NONE)) => Ok(level),
        _ => Err(refused("ticket hierarchy value is unreadable")),
    }
}

/// Port of fsv.ticket_associations.children: active children of a parent;
/// a ticket that is not a parent has none (its child list answers 400, so
/// the tab decides).
pub fn children(c: &dyn Api, ticket: i64) -> Result<Vec<Value>> {
    if hierarchy(c, ticket)? != PARENT {
        return Ok(Vec::new());
    }
    complete(
        c,
        &format!("tickets/{ticket}/child-tickets"),
        "child_tickets",
        Some(&obj(json!({"page": 1, "per_page": 30, "type": "active"}))),
        "child tickets",
    )
}

/// Port of fsv.ticket_associations._ids.
fn ids(rows: &[Value]) -> Result<BTreeSet<i64>> {
    rows.iter()
        .map(|r| positive_id(r.get("id").unwrap_or(&Value::Null)))
        .collect()
}

fn ticket_id(ticket: &str) -> Result<i64> {
    positive_id(&json!(parse_id(ticket, &TICKETS)?))
}

/// Port of fsv.ticket_associations.listing.
pub fn listing(c: &dyn Api, ticket: &str, kind: &str, ctype: Option<&str>) -> Result<Value> {
    let tid = ticket_id(ticket)?;
    if kind == "problem" {
        return Ok(json!({"ticket_id": tid, "problems": problems(c, tid)?, "complete": true}));
    }
    if kind == "change" {
        let ct = change_type(ctype.unwrap_or(""))?;
        return Ok(json!({
            "ticket_id": tid, "change_type": ct, "meaning": change_type_label(&ct),
            "changes": changes(c, tid, &ct)?, "complete": true,
        }));
    }
    let level = hierarchy(c, tid)?;
    Ok(json!({
        "ticket_id": tid, "hierarchy": hierarchy_name(level), "child_tickets": children(c, tid)?,
        "complete": true, "note": "active children only (type=active)",
    }))
}

/// Port of fsv.ticket_associations._rows_of.
fn rows_of(raw: &Value, keys: &[&str]) -> Result<Vec<Value>> {
    let rows = match raw {
        Value::Array(items) => Some(items),
        Value::Object(m) => keys
            .iter()
            .find_map(|k| m.get(*k).and_then(Value::as_array))
            .or_else(|| {
                let lists: Vec<&Vec<Value>> = m
                    .values()
                    .filter_map(Value::as_array)
                    .filter(|v| v.iter().all(Value::is_object))
                    .collect();
                (lists.len() == 1).then(|| lists[0])
            }),
        _ => None,
    };
    match rows {
        Some(rows) if rows.iter().all(Value::is_object) => Ok(rows.clone()),
        _ => Err(refused("candidate list is unreadable")),
    }
}

/// Port of fsv.ticket_associations._brief.
fn brief(row: &Value) -> Value {
    let mut out = Map::new();
    for k in ["id", "human_display_id", "subject", "status_name", "status"] {
        if let Some(v) = row.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    Value::Object(out)
}

/// Port of fsv.ticket_associations.candidates: read-only picker lists
/// (single page; never a complete inventory).
pub fn candidates(
    c: &dyn Api,
    ticket: &str,
    kind: &str,
    search: Option<&str>,
    ctype: Option<&str>,
) -> Result<Value> {
    let tid = ticket_id(ticket)?;
    let term = search.filter(|s| !s.is_empty());
    let rows = if kind == "problem" {
        let raw = match term {
            Some(q) => c.int_get("problems/search", Some(obj(json!({"q": q}))))?,
            None => c.int_get(
                &format!("tickets/{tid}/problems"),
                Some(obj(json!({"filter": "list", "page": 1, "per_page": 30}))),
            )?,
        };
        rows_of(&raw, &["problems"])?
    } else if kind == "change" {
        let ct = change_type(ctype.unwrap_or(""))?;
        let raw = match term {
            Some(q) => c.int_get("changes/search", Some(obj(json!({"q": q}))))?,
            None => c.int_get(
                &format!("tickets/{tid}/changes"),
                Some(obj(json!({
                    "filter": "list", "change_type": ct, "page": 1, "per_page": 30, "ticket_id": tid,
                }))),
            )?,
        };
        rows_of(&raw, &["changes"])?
    } else {
        let params = term.map(|q| obj(json!({"search_term": q})));
        let raw = c.int_get(&format!("tickets/{tid}/suggest_child_tickets"), params)?;
        let mut rows = rows_of(
            &raw,
            &[
                "tickets",
                "child_tickets",
                "suggested_tickets",
                "suggest_child_tickets",
            ],
        )?;
        if let Some(q) = term {
            // The server list is not filtered by the term; filter locally.
            let needle = casefold(q);
            rows.retain(|r| {
                let text: Vec<String> = ["subject", "human_display_id", "id"]
                    .iter()
                    .map(|k| match r.get(*k) {
                        Some(v) if truthy(v) => py_str(v),
                        _ => String::new(),
                    })
                    .collect();
                casefold(&text.join(" ")).contains(&needle)
            });
        }
        rows
    };
    let mut out = obj(json!({
        "ticket_id": tid, "kind": kind, "search": search,
        "candidates": rows.iter().map(brief).collect::<Vec<_>>(),
        "scope": "one picker page or search result; not a complete inventory", "mutation": "none",
    }));
    if term.is_some() && matches!(kind, "problem" | "change") {
        out.insert(
            "note".into(),
            json!(
                "search is token based; records created in the last minute may not be indexed yet"
            ),
        );
    }
    Ok(Value::Object(out))
}

/// Port of fsv.ticket_associations.build_plan.
pub fn build_plan(
    c: &dyn Api,
    ticket: &str,
    kind: &str,
    item: &str,
    unlink: bool,
    ctype: Option<&str>,
) -> Result<LinkPlan> {
    let tid = ticket_id(ticket)?;
    let resource = match kind {
        "problem" => &PROBLEMS,
        "change" => &CHANGES,
        "child" => &TICKETS,
        _ => return Err(Error::value("unsupported ticket association")),
    };
    let iid = positive_id(&json!(parse_id(item, resource)?))?;
    let record = Value::Object(exact(c, "tickets", tid)?);
    let mut context = obj(json!({"ticket": record}));
    let mut ct = None;
    let (method, path, payload, present) = if kind == "problem" {
        let evidence = crate::ticket_kind::kind_evidence(&record);
        if evidence.is_empty()
            || evidence
                .iter()
                .any(|(_, k)| k != crate::ticket_kind::INCIDENT)
        {
            return Err(refused(
                "problems associate with incidents only; this ticket is not a verified incident",
            ));
        }
        context.insert("item".into(), Value::Object(exact(c, "problems", iid)?));
        let rows = problems(c, tid)?;
        let present = ids(&rows)?.contains(&iid);
        context.insert("inventory".into(), Value::Array(rows));
        (
            "PUT",
            format!("tickets/{tid}/problems/{iid}"),
            json!({"type": if unlink { "unlink" } else { "link" }}),
            present,
        )
    } else if kind == "change" {
        let t = change_type(ctype.unwrap_or(""))?;
        context.insert("item".into(), Value::Object(exact(c, "changes", iid)?));
        let rows = changes(c, tid, &t)?;
        let present = ids(&rows)?.contains(&iid);
        context.insert("inventory".into(), Value::Array(rows));
        let wire = if unlink {
            (
                "PUT",
                format!("tickets/{tid}/changes/{iid}"),
                json!({"type": "unlink", "change_type": t}),
                present,
            )
        } else {
            (
                "PUT",
                format!("tickets/{tid}/changes/associate"),
                json!({"item_ids": [iid], "change_type": t, "type": "link"}),
                present,
            )
        };
        ct = Some(t);
        wire
    } else {
        if iid == tid {
            return Err(Error::value("a ticket cannot be its own child"));
        }
        context.insert("item".into(), Value::Object(exact(c, "tickets", iid)?));
        let parent_level = hierarchy(c, tid)?;
        let child_level = hierarchy(c, iid)?;
        let rows = children(c, tid)?;
        let present = ids(&rows)?.contains(&iid);
        context.insert("inventory".into(), Value::Array(rows));
        context.insert("levels".into(), json!([parent_level, child_level]));
        if !unlink {
            if parent_level == CHILD {
                return Err(refused(format!(
                    "ticket {tid} is itself a child ticket; it cannot take children"
                )));
            }
            if child_level != NONE && !present {
                return Err(refused(format!(
                    "ticket {iid} is already a {} ticket; nothing was sent",
                    hierarchy_name(child_level)
                )));
            }
            (
                "POST",
                format!("tickets/{tid}/associate-child-tickets"),
                json!({"ticket_ids": [iid]}),
                present,
            )
        } else {
            (
                "DELETE",
                format!("tickets/{tid}/remove_child?ticket_id={iid}"),
                Value::Null,
                present,
            )
        }
    };
    if present != unlink {
        let state = if present { "already" } else { "not" };
        let direction = ct
            .as_deref()
            .map(|t| format!(" as {}", change_type_label(t)))
            .unwrap_or_default();
        return Err(refused(format!(
            "{kind} {iid} is {state} associated with ticket {tid}{direction}; nothing was sent"
        )));
    }
    let action = format!("ticket_{}_{kind}", if unlink { "unlink" } else { "link" });
    Ok(LinkPlan {
        plan: WritePlan {
            action,
            target: tid,
            tenant: c.domain(),
            method: method.into(),
            path,
            payload_json: fsv_py::Dumps::default().sort_keys(true).dump(&payload),
            context_hash: snapshot_hash(&Value::Object(context))?,
        },
        kind: kind.into(),
        item: iid,
        change_type: ct,
    })
}

/// Port of fsv.ticket_associations.send.
pub fn send(
    plan: &LinkPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let unlink = plan.plan.action.starts_with("ticket_unlink_");
    let verify = |_response: &Value| -> Result<Map<String, Value>> {
        let mut out = obj(json!({
            "ticket_id": plan.plan.target, "item_id": plan.item, "kind": plan.kind,
        }));
        let linked = if plan.kind == "problem" {
            ids(&problems(c, plan.plan.target)?)?.contains(&plan.item)
        } else if plan.kind == "change" {
            let ct = plan.change_type.as_deref().unwrap_or("");
            let linked = ids(&changes(c, plan.plan.target, ct)?)?.contains(&plan.item);
            out.insert("change_type".into(), json!(plan.change_type));
            linked
        } else {
            let in_list = ids(&children(c, plan.plan.target)?)?.contains(&plan.item);
            let level = hierarchy(c, plan.item)?;
            out.insert("child_hierarchy".into(), json!(hierarchy_name(level)));
            if unlink {
                in_list || level != NONE
            } else {
                in_list && level == CHILD
            }
        };
        out.insert("linked".into(), json!(linked));
        out.insert("verified".into(), json!(linked != unlink));
        Ok(out)
    };
    let target = plan.plan.target.to_string();
    let item = plan.item.to_string();
    let rebuild = || {
        build_plan(
            c,
            &target,
            &plan.kind,
            &item,
            unlink,
            plan.change_type.as_deref(),
        )
        .map(|p| p.plan)
    };
    execute(&plan.plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, LazyLock, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::service::testing::{Call, FnApi, MemJournal};

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/ticket_associations_oracle.json")).unwrap()
    });

    #[derive(Default)]
    struct World {
        problems: BTreeSet<i64>,
        changes: BTreeMap<String, BTreeSet<i64>>,
        children: BTreeSet<i64>,
        fail: Option<u16>,
        no_effect: bool,
    }

    fn ticket_type(t: i64) -> Option<&'static str> {
        match t {
            41 | 42 | 44 => Some("Incident"),
            43 => Some("Service Request"),
            _ => None,
        }
    }

    fn api_error(status: u16, body: Value) -> Error {
        fsv_core::ApiError::new(status, body).into()
    }

    /// The `tenant` fixture of tests/test_ticket_associations.py.
    fn tenant() -> (FnApi, Arc<Mutex<World>>) {
        let state = Arc::new(Mutex::new(World::default()));
        let s2 = Arc::clone(&state);
        let mut c = FnApi::new(move |call: &Call| {
            let mut s = s2.lock().unwrap();
            let level = |s: &World, t: i64| {
                if t == 41 && !s.children.is_empty() {
                    2
                } else if s.children.contains(&t) {
                    1
                } else {
                    0
                }
            };
            let (path, query) = call.path.split_once('?').unwrap_or((&call.path, ""));
            let parts: Vec<&str> = path.split('/').collect();
            let q = call.params.clone().unwrap_or_default();
            let qs = |k: &str| q.get(k).map(py_str);
            if call.method == "GET" {
                if parts[0] == "tickets"
                    && parts.len() == 2
                    && let Some(kind) = parts[1].parse().ok().and_then(ticket_type)
                {
                    let t: i64 = parts[1].parse().unwrap();
                    let prefix = if t == 43 { "SR" } else { "INC" };
                    return Ok(
                        json!({"ticket": {"id": t, "type": kind, "subject": format!("Acme {t}"),
                                                "human_display_id": format!("{prefix}-{t}")}}),
                    );
                }
                match path {
                    "problems/7" => {
                        return Ok(json!({"problem": {"id": 7, "subject": "Acme outage"}}));
                    }
                    "changes/9" => {
                        return Ok(json!({"change": {"id": 9, "subject": "Globex patch"}}));
                    }
                    "tickets/41/problems" if !q.contains_key("filter") => {
                        let rows: Vec<Value> = s
                            .problems
                            .iter()
                            .map(|p| json!({"id": p, "human_display_id": format!("PRB-{p}")}))
                            .collect();
                        return Ok(
                            json!({"problems": rows, "meta": {"has_next": false}, "requesters": []}),
                        );
                    }
                    "tickets/41/changes" => {
                        let ct = qs("change_type").unwrap_or_default();
                        if ct != "change" && ct != "change_cause" {
                            return Err(api_error(
                                400,
                                json!({"errors": "not a valid change_type"}),
                            ));
                        }
                        if qs("filter").as_deref() == Some("list") {
                            return Ok(
                                json!({"changes": [{"id": 9, "subject": "Globex patch"}], "meta": {"has_next": false}}),
                            );
                        }
                        let rows: Vec<Value> = s
                            .changes
                            .get(&ct)
                            .into_iter()
                            .flatten()
                            .map(|c| json!({"id": c}))
                            .collect();
                        return Ok(json!({"changes": rows, "meta": {"has_next": false}}));
                    }
                    "tickets/41/child-tickets" => {
                        if level(&s, 41) != 2 {
                            return Err(api_error(
                                400,
                                json!({"errors": {"parent_ticket": "not_a_major_ticket"}}),
                            ));
                        }
                        let rows: Vec<Value> = s
                            .children
                            .iter()
                            .map(|c| json!({"id": c, "deleted": false, "spam": false}))
                            .collect();
                        return Ok(json!({"child_tickets": rows, "meta": {"has_next": false}}));
                    }
                    "tickets/41/suggest_child_tickets" => {
                        return Ok(json!({"tickets": [{"id": 42, "subject": "Acme printer"},
                                                     {"id": 44, "subject": "Globex laptop"}]}));
                    }
                    "problems/search" => {
                        return Ok(
                            json!({"problems": [{"id": 7, "subject": "Acme outage", "status": 1}]}),
                        );
                    }
                    "problems/99" | "problems/77" => return Err(api_error(404, json!({}))),
                    _ => {}
                }
                if parts.last() == Some(&"tabs") {
                    let t: i64 = parts[1].parse().unwrap();
                    return Ok(
                        json!([{"name": "related_tickets", "count": 0, "hierarchy": level(&s, t)},
                                     {"name": "associations", "count": 0}]),
                    );
                }
                panic!("unexpected GET {path}");
            }
            if let Some(status) = s.fail {
                return Err(api_error(status, json!({"errors": "synthetic"})));
            }
            let body = call.body.clone().unwrap_or(Value::Null);
            if !s.no_effect {
                match parts[2] {
                    "problems" => {
                        let p: i64 = parts[3].parse().unwrap();
                        if body["type"] == "link" {
                            s.problems.insert(p);
                        } else {
                            s.problems.remove(&p);
                        }
                    }
                    "changes" => {
                        let ct = body["change_type"].as_str().unwrap().to_string();
                        let set = s.changes.entry(ct).or_default();
                        if parts[3] == "associate" {
                            set.extend(
                                body["item_ids"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|v| v.as_i64().unwrap()),
                            );
                        } else {
                            set.remove(&parts[3].parse::<i64>().unwrap());
                        }
                    }
                    "associate-child-tickets" => {
                        let ids: Vec<i64> = body["ticket_ids"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_i64().unwrap())
                            .collect();
                        s.children.extend(ids);
                        return Ok(json!({"success": true}));
                    }
                    "remove_child" => {
                        let id: i64 = query.strip_prefix("ticket_id=").unwrap().parse().unwrap();
                        s.children.remove(&id);
                        return Ok(Value::Null);
                    }
                    _ => {}
                }
            }
            Ok(json!({}))
        });
        c.domain = "acme.example.com".into();
        (c, state)
    }

    fn writes(c: &FnApi) -> Vec<(String, String, Value)> {
        c.writes()
            .into_iter()
            .map(|w| (w.method, w.path, w.body.unwrap_or(Value::Null)))
            .collect()
    }

    fn plan_json(p: &LinkPlan) -> Value {
        json!({"review": p.review(), "hash": p.hash(), "context": p.plan.context_hash})
    }

    fn sent(p: &LinkPlan, c: &FnApi, j: &MemJournal) -> Value {
        send(p, c, j, Some(&p.hash())).unwrap()
    }

    /// The whole oracle sequence of the Python generator, in order.
    #[test]
    fn sequence_matches_python() {
        let (c, _s) = tenant();
        let j = MemJournal::default();
        let o = &*ORACLE;
        assert_eq!(
            listing(&c, "INC-41", "problem", None).unwrap(),
            o["list_problem_empty"]
        );
        let p = build_plan(&c, "INC-41", "problem", "PRB-7", false, None).unwrap();
        assert_eq!(plan_json(&p), o["problem_link"]);
        assert_eq!(sent(&p, &c, &j), o["problem_link_sent"]);
        assert_eq!(
            listing(&c, "INC-41", "problem", None).unwrap(),
            o["list_problem"]
        );
        let u = build_plan(&c, "INC-41", "problem", "7", true, None).unwrap();
        assert_eq!(plan_json(&u), o["problem_unlink"]);
        assert_eq!(sent(&u, &c, &j), o["problem_unlink_sent"]);
        for kind in ["initiating", "initiated-by"] {
            let p = build_plan(&c, "41", "change", "CHN-9", false, Some(kind)).unwrap();
            assert_eq!(plan_json(&p), o[format!("change_link_{kind}")]);
            assert_eq!(sent(&p, &c, &j), o[format!("change_link_{kind}_sent")]);
            assert_eq!(
                listing(&c, "41", "change", Some(kind)).unwrap(),
                o[format!("list_change_{kind}")]
            );
            let u = build_plan(&c, "41", "change", "9", true, Some(kind)).unwrap();
            assert_eq!(plan_json(&u), o[format!("change_unlink_{kind}")]);
            assert_eq!(sent(&u, &c, &j), o[format!("change_unlink_{kind}_sent")]);
        }
        assert_eq!(
            listing(&c, "41", "child", None).unwrap(),
            o["list_child_empty"]
        );
        let p = build_plan(&c, "41", "child", "INC-42", false, None).unwrap();
        assert_eq!(plan_json(&p), o["child_link"]);
        assert_eq!(sent(&p, &c, &j), o["child_link_sent"]);
        assert_eq!(listing(&c, "41", "child", None).unwrap(), o["list_child"]);
        let u = build_plan(&c, "41", "child", "42", true, None).unwrap();
        assert_eq!(plan_json(&u), o["child_unlink"]);
        assert_eq!(sent(&u, &c, &j), o["child_unlink_sent"]);
        assert_eq!(
            candidates(&c, "INC-41", "child", Some("printer"), None).unwrap(),
            o["cand_child_search"]
        );
        assert_eq!(
            candidates(&c, "INC-41", "child", None, None).unwrap(),
            o["cand_child"]
        );
        assert_eq!(
            candidates(&c, "INC-41", "problem", Some("acme"), None).unwrap(),
            o["cand_problem_search"]
        );
        assert_eq!(
            candidates(&c, "INC-41", "change", None, Some("initiating")).unwrap(),
            o["cand_change"]
        );
        let errors: [(&str, Result<LinkPlan>); 7] = [
            (
                "sr_problem",
                build_plan(&c, "SR-43", "problem", "7", false, None),
            ),
            (
                "missing_problem",
                build_plan(&c, "INC-41", "problem", "99", false, None),
            ),
            (
                "self_child",
                build_plan(&c, "41", "child", "41", false, None),
            ),
            (
                "bad_ctype",
                build_plan(&c, "41", "change", "9", false, Some("x")),
            ),
            (
                "problem_not_linked",
                build_plan(&c, "41", "problem", "7", true, None),
            ),
            (
                "change_not_linked",
                build_plan(&c, "41", "change", "9", true, Some("initiating")),
            ),
            (
                "child_not_linked",
                build_plan(&c, "41", "child", "42", true, None),
            ),
        ];
        for (name, got) in errors {
            let err = got.unwrap_err();
            let class = if matches!(err, Error::Value(_)) {
                "ValueError"
            } else {
                "ReviewedWriteError"
            };
            assert_eq!(json!([class, err.to_string()]), o["errors"][name], "{name}");
            assert_eq!(err.exit_code(), 2, "{name}");
        }
        let want: Vec<(String, String, Value)> = o["writes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                let mut path = w[1].as_str().unwrap().to_string();
                if let Some(q) = w[3].as_object().filter(|q| !q.is_empty()) {
                    let pairs: Vec<String> = q
                        .iter()
                        .map(|(k, v)| format!("{k}={}", py_str(v)))
                        .collect();
                    path = format!("{path}?{}", pairs.join("&"));
                }
                (w[0].as_str().unwrap().to_string(), path, w[2].clone())
            })
            .collect();
        assert_eq!(writes(&c), want);
    }

    /// `test_child_link_unlink_hierarchy_and_404_free_readback` (the child
    /// of a parent cannot take children).
    #[test]
    fn child_cannot_take_children() {
        let (c, _s) = tenant();
        let j = MemJournal::default();
        let p = build_plan(&c, "41", "child", "INC-42", false, None).unwrap();
        assert_eq!(p.plan.payload(), json!({"ticket_ids": [42]}));
        sent(&p, &c, &j);
        let err = build_plan(&c, "42", "child", "44", false, None).unwrap_err();
        assert!(err.to_string().contains("child ticket"), "{err}");
    }

    /// `test_no_effect_is_unverified_and_failures_never_resend`.
    #[test]
    fn no_effect_is_unverified_and_failures_never_resend() {
        let (c, s) = tenant();
        let j = MemJournal::default();
        s.lock().unwrap().no_effect = true;
        let p = build_plan(&c, "41", "problem", "7", false, None).unwrap();
        assert_eq!(sent(&p, &c, &j)["verified"], json!(false));
        let err = send(&p, &c, &j, Some(&p.hash())).unwrap_err();
        assert!(err.to_string().contains("sent"), "{err}");
        {
            let mut w = s.lock().unwrap();
            w.no_effect = false;
            w.fail = Some(500);
        }
        let q = build_plan(&c, "41", "change", "9", false, Some("initiating")).unwrap();
        let err = send(&q, &c, &j, Some(&q.hash())).unwrap_err();
        assert!(err.to_string().contains("unknown"), "{err}");
        assert_eq!(writes(&c).len(), 2);
    }

    #[test]
    fn change_type_aliases_and_hierarchy_shapes() {
        for (v, want) in [
            (" Initiating ", "change_cause"),
            ("cause", "change_cause"),
            ("initiated_by", "change"),
            ("CHANGE", "change"),
        ] {
            assert_eq!(change_type(v).unwrap(), want);
        }
        assert!(change_type("").is_err());
        for (raw, want) in [
            (
                json!({"tabs": [{"name": "related_tickets", "hierarchy": 2}]}),
                Ok(2),
            ),
            (json!({"related_tickets": {"hierarchy": 1}}), Ok(1)),
            (
                json!([{"name": "related_tickets", "hierarchy": true}]),
                Err("ticket hierarchy value is unreadable"),
            ),
            (
                json!([{"name": "related_tickets", "hierarchy": 2.0}]),
                Err("ticket hierarchy value is unreadable"),
            ),
            (
                json!([{"name": "related_tickets", "hierarchy": 3}]),
                Err("ticket hierarchy value is unreadable"),
            ),
            (
                json!([{"name": "related_tickets", "hierarchy": 0}, {"name": "related_tickets", "hierarchy": 2}]),
                Err("ticket hierarchy (related_tickets tab) unavailable; nothing was sent"),
            ),
            (
                json!([]),
                Err("ticket hierarchy (related_tickets tab) unavailable; nothing was sent"),
            ),
        ] {
            let c = FnApi::new(move |_| Ok(raw.clone()));
            let got = hierarchy(&c, 5).map_err(|e| e.to_string());
            assert_eq!(got, want.map_err(str::to_string));
        }
    }

    #[test]
    fn rows_of_shapes() {
        assert_eq!(
            rows_of(&json!({"other": [{"id": 1}]}), &["tickets"]).unwrap(),
            vec![json!({"id": 1})]
        );
        assert!(rows_of(&json!({"a": [], "b": []}), &["tickets"]).is_err());
        assert!(rows_of(&json!([1]), &["tickets"]).is_err());
        assert!(rows_of(&json!("x"), &["tickets"]).is_err());
    }
}
