//! Private ticket-view lifecycle, gated on runtime ownership and visibility
//! metadata (`private_views.py`).
//!
//! A numeric visibility code alone cannot prove a view is private. Accounts
//! that omit a named visibility in their view metadata remain read-only here.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_py::py_str;

use crate::properties::dumps_sorted;
use crate::readback::py_equal;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, snapshot_hash};
use crate::watchers::{positive_id, self_id};

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

/// Port of fsv.private_views.detail: the exact saved view.
pub fn detail(c: &dyn Api, view_id: i64) -> Result<Map<String, Value>> {
    let raw = c.int_get(
        &format!("ticket_filters/{}", positive_id(&json!(view_id))?),
        None,
    )?;
    let row = match &raw {
        Value::Object(m) => m.get("ticket_filter").unwrap_or(&raw),
        _ => &Value::Null,
    };
    match row {
        Value::Object(r)
            if positive_id(r.get("id").unwrap_or(&Value::Null)).ok() == Some(view_id) =>
        {
            Ok(r.clone())
        }
        Value::Object(r) => {
            positive_id(r.get("id").unwrap_or(&Value::Null))?;
            Err(refused("exact saved-view identity is unavailable"))
        }
        _ => Err(refused("exact saved-view identity is unavailable")),
    }
}

/// Port of fsv.private_views.private_visibility: the visibility id of a
/// self-owned, explicitly non-default view whose visibility is named private.
pub fn private_visibility(row: &Map<String, Value>, user: i64) -> Result<i64> {
    let owner = positive_id(row.get("user_id").unwrap_or(&Value::Null))?;
    if owner != user
        || row.get("default") != Some(&Value::Bool(false))
        || row.get("system").unwrap_or(&Value::Bool(false)) != &Value::Bool(false)
    {
        return Err(refused(
            "only self-owned, explicitly non-default private views are supported",
        ));
    }
    let (label, value) = match row.get("visibility") {
        Some(Value::Object(v)) => (v.get("label"), v.get("id")),
        other => (row.get("visibility_name"), other),
    };
    let named = label
        .and_then(Value::as_str)
        .map(|s| fsv_core::pystr::casefold(fsv_core::pystr::strip(s)))
        .is_some_and(|s| matches!(s.as_str(), "me" | "only me" | "private"));
    if !named {
        return Err(refused(
            "named private visibility metadata is unavailable; numeric visibility alone is not proof",
        ));
    }
    positive_id(value.unwrap_or(&Value::Null))
}

/// Port of fsv.private_views.build_plan: clone (`create`), rename (`edit`)
/// or `delete` a private view.
pub fn build_plan(
    c: &dyn Api,
    source_id: &Value,
    action: &str,
    name: Option<&str>,
) -> Result<ViewPlan> {
    if !matches!(action, "create" | "edit" | "delete") {
        return Err(Error::value("unsupported saved-view action"));
    }
    let source_id = positive_id(source_id)?;
    let user = self_id(c)?;
    let row = detail(c, source_id)?;
    let visibility = private_visibility(&row, user)?;
    let (payload, method, path) = if action == "delete" {
        if name.is_some() {
            return Err(Error::value("delete does not accept a name"));
        }
        (Value::Null, "DELETE", format!("ticket_filters/{source_id}"))
    } else {
        let Some(name) = name.filter(|n| !fsv_core::pystr::strip(n).is_empty()) else {
            return Err(Error::value("create/edit requires a nonempty --name"));
        };
        if !row
            .get("advanced_query_hash")
            .unwrap_or(&Value::Null)
            .is_null()
            || !row.get("query_hash").is_some_and(Value::is_array)
        {
            return Err(refused(
                "only the captured ordinary query-hash subset is supported",
            ));
        }
        if row.get("order_by") != Some(&json!("created_at"))
            || row.get("order_type") != Some(&json!("desc"))
        {
            return Err(refused(
                "only captured created-at descending views are supported",
            ));
        }
        let payload = json!({
            "name": name,
            "workspace_id": positive_id(row.get("workspace_id").unwrap_or(&Value::Null))?,
            "visibility": visibility,
            "query_hash": row["query_hash"],
            "advanced_query_hash": null,
            "order_by": row["order_by"],
            "order_type": row["order_type"],
        });
        if action == "create" {
            (payload, "POST", "ticket_filters".to_string())
        } else {
            (payload, "PUT", format!("ticket_filters/{source_id}"))
        }
    };
    let context = snapshot_hash(&json!({"self": user, "source": row, "source_id": source_id}))?;
    Ok(ViewPlan {
        plan: WritePlan {
            action: format!("private_view_{action}"),
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

/// Port of fsv.private_views.send. A delete is verified by an HTTP 404 on
/// the view and its absence from a complete re-list.
pub fn send(
    plan: &ViewPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let action = plan
        .plan
        .action
        .strip_prefix("private_view_")
        .unwrap_or(&plan.plan.action)
        .to_string();
    let payload = plan.payload();
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let rebuild = || Ok(build_plan(c, &json!(plan.source_id), &action, name.as_deref())?.plan);
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let mut view_id = plan.source_id;
        if action == "create" {
            let row = match response {
                Value::Object(m) => m.get("ticket_filter").unwrap_or(response),
                _ => &Value::Null,
            };
            view_id = positive_id(row.get("id").unwrap_or(&Value::Null))?;
            if view_id == plan.source_id {
                return Err(refused("create returned the source identity"));
            }
        }
        if action == "delete" {
            match detail(c, view_id) {
                Err(e) if e.status() == Some(404) => {}
                Err(e) => return Err(e),
                Ok(_) => return Ok(obj(json!({"view_id": view_id, "verified": false}))),
            }
            let params = obj(json!({"page": 1, "per_page": 30}));
            let pages = collect_pages(
                |path, params| c.int_get(path, params),
                "ticket_filters",
                "ticket_filters",
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
        let row = detail(c, view_id)?;
        let visibility = json!(private_visibility(&row, self_id(c)?)?);
        let mut fields = Map::new();
        for (key, value) in payload.as_object().into_iter().flatten() {
            let state = match row.get(key) {
                None => "unavailable",
                Some(actual) => {
                    let actual = if key == "visibility" {
                        &visibility
                    } else {
                        actual
                    };
                    if py_equal(actual, value) {
                        "matched"
                    } else {
                        "mismatched"
                    }
                }
            };
            fields.insert(key.clone(), json!(state));
        }
        let verified = fields.values().all(|v| v == "matched");
        Ok(obj(
            json!({"view_id": view_id, "fields": fields, "verified": verified}),
        ))
    };
    execute(&plan.plan, c, journal, reviewed_hash, &rebuild, &verify)
}
