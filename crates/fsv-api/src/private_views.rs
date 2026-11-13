//! Private saved-view lifecycle (tickets and changes), gated on exact
//! ownership (`private_views.py`).
//!
//! The web UI's "Save as" dialog offers three visibility radios with fixed
//! codes (captured on tickets and changes alike): Myself = 3, All agents = 1,
//! Agents in groups = 2. Exact view reads carry only the number, never a
//! label. A view is treated as private when it is owned by the signed-in
//! agent, is not a default or system view, has an empty `group_ids` list and
//! either carries a private label or the Myself code. Anything else stays
//! read-only here.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_py::{py_repr_str, py_str};

use crate::properties::dumps_sorted;
use crate::readback::py_equal;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, snapshot_hash};
use crate::watchers::{positive_id, self_id};

/// `VISIBILITY_MYSELF`: the Save-as dialog's Myself radio code.
pub const VISIBILITY_MYSELF: i64 = 3;

/// `VISIBILITY_LABELS`: the Save-as dialog's radio codes.
pub fn visibility_label(code: i64) -> Option<&'static str> {
    match code {
        1 => Some("All agents"),
        2 => Some("Agents in groups"),
        3 => Some("Myself"),
        _ => None,
    }
}

const PRIVATE_LABELS: [&str; 4] = ["me", "only me", "private", "myself"];

/// Port of fsv.private_views.ViewKind.
#[derive(Debug, PartialEq, Eq)]
pub struct ViewKind {
    pub module: &'static str,
    pub path: &'static str,
    pub envelope: &'static str,
    pub actions: &'static [&'static str],
    pub prefix: &'static str,
}

pub static TICKETS: ViewKind = ViewKind {
    module: "tickets",
    path: "ticket_filters",
    envelope: "ticket_filter",
    actions: &["create", "edit", "delete"],
    prefix: "private_view_",
};

/// Only the change-view create wire is captured; rename/delete stay refused.
pub static CHANGES: ViewKind = ViewKind {
    module: "changes",
    path: "change-filters",
    envelope: "change_filter",
    actions: &["create"],
    prefix: "change_private_view_",
};

/// `KINDS[module]`.
pub fn kind(module: &str) -> &'static ViewKind {
    if module == "changes" {
        &CHANGES
    } else {
        &TICKETS
    }
}

fn is_changes(k: &ViewKind) -> bool {
    std::ptr::eq(k, &CHANGES)
}

/// `except ReviewedWriteError`.
fn is_refusal(e: &Error) -> bool {
    matches!(e, Error::Exit { code: 2, .. })
}

/// Port of fsv.private_views.ViewPlan: a [`WritePlan`] that remembers its
/// source view (not part of the hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewPlan {
    pub plan: WritePlan,
    pub source_id: i64,
}

impl ViewPlan {
    pub fn hash(&self) -> String {
        self.plan.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        self.plan.review()
    }

    pub fn payload(&self) -> Value {
        self.plan.payload()
    }
}

/// Port of fsv.private_views._detail: the exact saved view, with the
/// envelope's `meta` appended when the row has none.
fn detail_of(c: &dyn Api, view_id: i64, k: &ViewKind) -> Result<Map<String, Value>> {
    let raw = c.int_get(
        &format!("{}/{}", k.path, positive_id(&json!(view_id))?),
        None,
    )?;
    let enveloped = raw.as_object().and_then(|m| m.get(k.envelope));
    let row = match &raw {
        Value::Object(_) => enveloped.unwrap_or(&raw),
        _ => &Value::Null,
    };
    let mut row = match row {
        Value::Object(r) if positive_id(r.get("id").unwrap_or(&Value::Null))? == view_id => {
            r.clone()
        }
        _ => return Err(refused("exact saved-view identity is unavailable")),
    };
    if enveloped.is_some()
        && let Some(meta @ Value::Object(_)) = raw.get("meta")
        && !row.contains_key("meta")
    {
        row.insert("meta".into(), meta.clone());
    }
    Ok(row)
}

/// Port of fsv.private_views.detail.
pub fn detail(c: &dyn Api, view_id: i64, k: &ViewKind) -> Result<Map<String, Value>> {
    detail_of(c, view_id, k)
}

fn int_code(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_i64)
}

/// Port of fsv.private_views.private_visibility: the visibility of a
/// self-owned, explicitly non-default view visible only to its owner.
pub fn private_visibility(row: &Map<String, Value>, user: i64, k: &ViewKind) -> Result<i64> {
    let owner = positive_id(row.get("user_id").unwrap_or(&Value::Null))?;
    if owner != user
        || row.get("default") != Some(&Value::Bool(false))
        || row.get("system").unwrap_or(&Value::Bool(false)) != &Value::Bool(false)
    {
        return Err(refused(
            "only self-owned, explicitly non-default private views are supported",
        ));
    }
    let empty = json!([]);
    let groups = row.get("group_ids").unwrap_or(&empty);
    if groups != &empty || !row.get("group_id").unwrap_or(&Value::Null).is_null() {
        return Err(refused(
            "the view is shared with groups; only views visible to you alone are supported",
        ));
    }
    let (label, value) = match row.get("visibility") {
        Some(Value::Object(v)) => (v.get("label"), v.get("id")),
        other => (row.get("visibility_name"), other),
    };
    let code = int_code(value);
    if let Some(Value::String(label)) = label {
        let folded = fsv_core::pystr::casefold(fsv_core::pystr::strip(label));
        if !PRIVATE_LABELS.contains(&folded.as_str()) {
            return Err(refused(format!(
                "view visibility is {}, not private; nothing was sent",
                py_repr_str(label)
            )));
        }
        return positive_id(value.unwrap_or(&Value::Null));
    }
    let has_groups = row.contains_key("group_ids");
    if code != Some(VISIBILITY_MYSELF) || !has_groups {
        let (shown, code_text) = match code {
            Some(n) => (visibility_label(n).unwrap_or("unknown"), n.to_string()),
            None => ("unreadable", "unreadable".to_string()),
        };
        return Err(refused(format!(
            "GET /api/_/{}/{} returns visibility {code_text} ({shown}){}; only Myself \
             ({VISIBILITY_MYSELF}) with an empty group_ids list is treated as private (1 = All agents, \
             2 = Agents in groups). Nothing was sent",
            k.path,
            py_str(row.get("id").unwrap_or(&Value::Null)),
            if has_groups { "" } else { " without group_ids" },
        )));
    }
    Ok(VISIBILITY_MYSELF)
}

fn workspace_values(value: &Value) -> Vec<Value> {
    let id = |v: &Value| v.get("id").cloned().unwrap_or(Value::Null);
    match value {
        Value::Array(a) => a
            .iter()
            .map(|v| if v.is_object() { id(v) } else { v.clone() })
            .collect(),
        Value::Object(_) => vec![id(value)],
        other => vec![other.clone()],
    }
}

fn conditions(row: &Map<String, Value>, k: &ViewKind) -> Result<Vec<Value>> {
    if is_changes(k) {
        let shape = match row.get("advanced_query_hash") {
            Some(Value::Object(a))
                if row.get("query_hash").unwrap_or(&Value::Null).is_null()
                    && a.len() == 1
                    && a.get("all").is_some_and(Value::is_array) =>
            {
                a["all"].as_array().cloned()
            }
            _ => None,
        };
        return shape.ok_or_else(|| {
            refused("only the captured change-view shape (advanced_query_hash.all) is supported")
        });
    }
    match row.get("query_hash") {
        Some(Value::Array(q))
            if row
                .get("advanced_query_hash")
                .unwrap_or(&Value::Null)
                .is_null() =>
        {
            Ok(q.clone())
        }
        _ => Err(refused(
            "only the captured ordinary query-hash subset is supported",
        )),
    }
}

fn is_workspace(cond: &Value) -> bool {
    cond.get("condition") == Some(&json!("workspace_id"))
}

/// Port of fsv.private_views.workspace: one workspace from the read; the
/// top-level id, `meta.workspace_id` and the workspace condition must agree
/// (exact reads omit the top-level id).
pub fn workspace(row: &Map<String, Value>, k: &ViewKind) -> Result<i64> {
    let mut found: Vec<Value> = Vec::new();
    if let Some(v) = row.get("workspace_id").filter(|v| !v.is_null()) {
        found.push(v.clone());
    }
    if let Some(v) = row
        .get("meta")
        .and_then(Value::as_object)
        .and_then(|m| m.get("workspace_id"))
        .filter(|v| !v.is_null())
    {
        found.extend(workspace_values(v));
    }
    let conds = match conditions(row, k) {
        Ok(c) => c,
        Err(e) if is_refusal(&e) => Vec::new(),
        Err(e) => return Err(e),
    };
    for cond in conds.iter().filter(|c| is_workspace(c)) {
        found.extend(workspace_values(cond.get("value").unwrap_or(&Value::Null)));
    }
    let mut ids = BTreeSet::new();
    for v in &found {
        match positive_id(v) {
            Ok(id) => {
                ids.insert(id);
            }
            Err(e) if is_refusal(&e) => {
                return Err(refused(
                    "the view's workspace is unreadable; nothing was sent",
                ));
            }
            Err(e) => return Err(e),
        }
    }
    if ids.len() != 1 {
        return Err(refused(
            "the view's workspace is missing or ambiguous in its read; nothing was sent",
        ));
    }
    Ok(ids.pop_first().unwrap_or_default())
}

/// Clone the runtime conditions unchanged except the workspace condition,
/// whose read form (e.g. `"2"`) differs from the write form the UI sends.
fn write_conditions(conds: &[Value], ws: i64, k: &ViewKind) -> Result<Vec<Value>> {
    conds
        .iter()
        .map(|cond| {
            let Value::Object(m) = cond else {
                return Err(refused("unreadable view condition"));
            };
            if !is_workspace(cond) {
                return Ok(cond.clone());
            }
            if !matches!(
                m.get("operator").and_then(Value::as_str),
                Some("is" | "is_in")
            ) {
                return Err(refused("unsupported workspace condition operator"));
            }
            let mut m = m.clone();
            let value = if is_changes(k) {
                json!(ws)
            } else {
                json!([{"id": ws}])
            };
            m.insert("value".into(), value);
            Ok(Value::Object(m))
        })
        .collect()
}

fn canonical(conds: &[Value], ws: i64) -> Result<Vec<String>> {
    let mut rows = Vec::new();
    for cond in conds {
        let cond = if is_workspace(cond) {
            let mut values = BTreeSet::new();
            for v in workspace_values(cond.get("value").unwrap_or(&Value::Null)) {
                match positive_id(&v) {
                    Ok(id) => {
                        values.insert(id);
                    }
                    Err(e) if is_refusal(&e) => {
                        values.clear();
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
            let workspace = if values.len() == 1 && values.contains(&ws) {
                json!(ws)
            } else {
                json!(values.into_iter().collect::<Vec<_>>())
            };
            json!({"condition": "workspace_id", "workspace": workspace})
        } else {
            cond.clone()
        };
        rows.push(dumps_sorted(&cond));
    }
    rows.sort();
    Ok(rows)
}

/// Port of fsv.private_views.build_plan: clone (`create`), rename (`edit`)
/// or `delete` a private view of `k`.
pub fn build_plan(
    c: &dyn Api,
    source_id: &Value,
    action: &str,
    name: Option<&str>,
    k: &ViewKind,
) -> Result<ViewPlan> {
    let action = if action == "rename" { "edit" } else { action };
    if !matches!(action, "create" | "edit" | "delete") {
        return Err(Error::value(
            "unsupported saved-view action (create | rename | delete)",
        ));
    }
    if !k.actions.contains(&action) {
        return Err(refused(format!(
            "{} private-view {} is not captured (PUT/DELETE /api/_/{}/<id> has no accepted wire); \
             nothing was sent; use the web UI",
            k.module,
            if action == "edit" { "rename" } else { action },
            k.path,
        )));
    }
    let source_id = positive_id(source_id)?;
    let user = self_id(c)?;
    let row = detail_of(c, source_id, k)?;
    let visibility = private_visibility(&row, user, k)?;
    let (payload, method, path) = if action == "delete" {
        if name.is_some() {
            return Err(Error::value("delete does not accept a name"));
        }
        (Value::Null, "DELETE", format!("{}/{source_id}", k.path))
    } else {
        let Some(name) = name.filter(|n| !fsv_core::pystr::strip(n).is_empty()) else {
            return Err(Error::value("create/rename requires a nonempty --name"));
        };
        if row.get("order_by") != Some(&json!("created_at"))
            || row.get("order_type") != Some(&json!("desc"))
        {
            return Err(refused(
                "only captured created-at descending views are supported",
            ));
        }
        let ws = workspace(&row, k)?;
        let conds = write_conditions(&conditions(&row, k)?, ws, k)?;
        let mut payload = json!({
            "name": name,
            "workspace_id": ws,
            "visibility": visibility,
            "order_by": row["order_by"],
            "order_type": row["order_type"],
        });
        if is_changes(k) {
            payload["query_hash"] = Value::Null;
            payload["advanced_query_hash"] = json!({"all": conds});
        } else {
            payload["query_hash"] = json!(conds);
            payload["advanced_query_hash"] = Value::Null;
        }
        if action == "create" {
            (payload, "POST", k.path.to_string())
        } else {
            (payload, "PUT", format!("{}/{source_id}", k.path))
        }
    };
    let context = snapshot_hash(&json!({"self": user, "source": row, "source_id": source_id}))?;
    Ok(ViewPlan {
        plan: WritePlan {
            action: format!("{}{action}", k.prefix),
            target: source_id,
            tenant: c.domain(),
            method: method.into(),
            path,
            payload_json: dumps_sorted(&payload),
            context_hash: context,
        },
        source_id,
    })
}

/// Port of fsv.private_views.assess: the view plus whether fsv would treat
/// it as private (read-only).
pub fn assess(c: &dyn Api, view_id: i64, k: &ViewKind) -> Result<Map<String, Value>> {
    let row = detail_of(c, view_id, k)?;
    let mut out = Map::new();
    out.insert("view".into(), Value::Object(row.clone()));
    match self_id(c).and_then(|user| private_visibility(&row, user, k)) {
        Ok(code) => {
            out.insert(
                "visibility".into(),
                json!(visibility_label(code).unwrap_or("private")),
            );
            out.insert("private".into(), json!(true));
        }
        Err(e) if is_refusal(&e) => {
            out.insert("private".into(), json!(false));
            out.insert("reason".into(), json!(e.to_string()));
        }
        Err(e) => return Err(e),
    }
    match workspace(&row, k) {
        Ok(ws) => {
            out.insert("workspace_id".into(), json!(ws));
        }
        Err(e) if is_refusal(&e) => {
            out.insert("workspace_id".into(), Value::Null);
            if !out.contains_key("reason") {
                out.insert("reason".into(), json!(e.to_string()));
            }
        }
        Err(e) => return Err(e),
    }
    Ok(out)
}

fn py_iter_strs(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().map(py_str).collect(),
        Value::Object(m) => m.keys().cloned().collect(),
        Value::String(s) => s.chars().map(String::from).collect(),
        _ => Vec::new(),
    }
}

/// Port of fsv.private_views.own_views: self-owned non-default rows from
/// the list; privacy is decided per exact read.
pub fn own_views(c: &dyn Api, k: &ViewKind) -> Result<Value> {
    let raw = c.int_get(k.path, None)?;
    let rows = match &raw {
        Value::Object(m) => {
            let lists: Vec<&Value> = m
                .iter()
                .filter(|(key, v)| {
                    v.is_array() && (key.ends_with("filters") || key.ends_with("views"))
                })
                .map(|(_, v)| v)
                .collect();
            if lists.len() == 1 {
                lists[0].as_array()
            } else {
                None
            }
        }
        Value::Array(a) => Some(a),
        _ => None,
    };
    let Some(rows) = rows else {
        return Err(refused("saved-view list shape is unreadable"));
    };
    let user = self_id(c)?.to_string();
    let editable: BTreeSet<String> = raw
        .get("meta")
        .and_then(Value::as_object)
        .and_then(|m| m.get("editable_filter_ids"))
        .filter(|v| fsv_py::truthy(v))
        .map(py_iter_strs)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let views: Vec<Value> = rows
        .iter()
        .filter_map(Value::as_object)
        .filter(|r| {
            py_str(r.get("user_id").unwrap_or(&Value::Null)) == user
                && r.get("default") == Some(&Value::Bool(false))
        })
        .map(|r| {
            let id = r.get("id").cloned().unwrap_or(Value::Null);
            let flag = if editable.is_empty() {
                Value::Null
            } else {
                json!(editable.contains(&py_str(&id)))
            };
            json!({"id": id, "name": r.get("name").cloned().unwrap_or(Value::Null), "editable": flag})
        })
        .collect();
    Ok(json!({"views": views,
              "note": "the list carries no visibility; run show ID to see whether a view is private"}))
}

/// Port of fsv.private_views.send. A delete is verified by an HTTP 404 on
/// the view and its absence from a complete re-list.
pub fn send(
    plan: &ViewPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let k: &ViewKind = if plan.plan.action.starts_with(CHANGES.prefix) {
        &CHANGES
    } else {
        &TICKETS
    };
    let action = plan
        .plan
        .action
        .strip_prefix(k.prefix)
        .unwrap_or(&plan.plan.action)
        .to_string();
    let payload = plan.payload();
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let rebuild = || Ok(build_plan(c, &json!(plan.source_id), &action, name.as_deref(), k)?.plan);
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let mut view_id = plan.source_id;
        if action == "create" {
            let row = match response {
                Value::Object(m) => m.get(k.envelope).unwrap_or(response),
                _ => &Value::Null,
            };
            let id = row
                .as_object()
                .and_then(|r| r.get("id"))
                .unwrap_or(&Value::Null);
            view_id = positive_id(id)?;
            if view_id == plan.source_id {
                return Err(refused("create returned the source identity"));
            }
        }
        if action == "delete" {
            match detail_of(c, view_id, k) {
                Err(e) if e.status() == Some(404) => {}
                Err(e) => return Err(e),
                Ok(_) => return Ok(obj(json!({"view_id": view_id, "verified": false}))),
            }
            let params = obj(json!({"page": 1, "per_page": 30}));
            let pages = collect_pages(
                |path, params| c.int_get(path, params),
                k.path,
                k.path,
                Some(&params),
                "saved views",
                MAX_DETAIL_PAGES,
            )?;
            let gone = !pages
                .rows
                .iter()
                .any(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == view_id.to_string());
            return Ok(obj(
                json!({"view_id": view_id, "verified": pages.complete() && gone}),
            ));
        }
        let row = detail_of(c, view_id, k)?;
        let ws = payload["workspace_id"].as_i64().unwrap_or_default();
        let mut fields = Map::new();
        let state = |ok: bool| json!(if ok { "matched" } else { "mismatched" });
        let visibility = match self_id(c).and_then(|user| private_visibility(&row, user, k)) {
            Ok(v) => state(py_equal(&json!(v), &payload["visibility"])),
            Err(e) if is_refusal(&e) => state(false),
            Err(e) => return Err(e),
        };
        fields.insert("visibility".into(), visibility);
        let ws_state = match workspace(&row, k) {
            Ok(found) => state(found == ws),
            Err(e) if is_refusal(&e) => json!("unavailable"),
            Err(e) => return Err(e),
        };
        fields.insert("workspace_id".into(), ws_state);
        for key in ["name", "order_by", "order_type"] {
            let s = match row.get(key) {
                None => json!("unavailable"),
                Some(actual) => state(py_equal(actual, &payload[key])),
            };
            fields.insert(key.into(), s);
        }
        let sent = if is_changes(k) {
            &payload["advanced_query_hash"]["all"]
        } else {
            &payload["query_hash"]
        };
        let sent = sent.as_array().cloned().unwrap_or_default();
        let cond_state = match conditions(&row, k)
            .and_then(|got| Ok(canonical(&got, ws)? == canonical(&sent, ws)?))
        {
            Ok(same) => state(same),
            Err(e) if is_refusal(&e) => json!("unavailable"),
            Err(e) => return Err(e),
        };
        fields.insert("conditions".into(), cond_state);
        let verified = fields.values().all(|v| v == "matched");
        Ok(obj(
            json!({"view_id": view_id, "fields": fields, "verified": verified}),
        ))
    };
    execute(&plan.plan, c, journal, reviewed_hash, &rebuild, &verify)
}
