//! Captured native list-column preferences and the CSV export-request
//! subset (`list_preferences.py`).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{Resource, lookup};
use fsv_py::{Dumps, py_str, truthy};

use crate::readback::py_equal;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::site::{SavedView, find_view};
use crate::views::load_views;

/// Port of fsv.list_preferences.ListPlan: a [`WritePlan`] plus the inputs
/// it is rebuilt from (not part of the hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPlan {
    pub plan: WritePlan,
    pub inputs_json: String,
}

impl ListPlan {
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

fn resource(module: &str) -> Result<&'static Resource> {
    lookup(module).ok_or_else(|| Error::value("unsupported list module"))
}

/// Port of fsv.list_preferences.headers: the runtime list headers, each
/// with a unique `name`.
pub fn headers(c: &dyn Api, module: &str) -> Result<Vec<Map<String, Value>>> {
    resource(module)?;
    let raw = c.int_get(&format!("{module}/headers"), None)?;
    let unavailable = || refused("runtime list-header metadata is unavailable");
    let rows = raw
        .get("headers")
        .and_then(Value::as_array)
        .filter(|r| !r.is_empty())
        .ok_or_else(unavailable)?;
    let rows: Vec<Map<String, Value>> = rows
        .iter()
        .map(|r| {
            r.as_object()
                .filter(|m| m.get("name").is_some_and(truthy))
                .cloned()
                .ok_or_else(unavailable)
        })
        .collect::<Result<_>>()?;
    let names: HashSet<String> = rows.iter().map(|r| py_str(&r["name"])).collect();
    if names.len() != rows.len() {
        return Err(refused("ambiguous runtime list-header keys"));
    }
    Ok(rows)
}

fn unique(items: &[String]) -> bool {
    items.iter().collect::<HashSet<_>>().len() == items.len()
}

/// Port of fsv.list_preferences.columns_plan: select, order and freeze list
/// columns; `display_id` is anchored to `ticket_subject`.
pub fn columns_plan(
    c: &dyn Api,
    module: &str,
    columns: &[String],
    frozen: &[String],
) -> Result<ListPlan> {
    let rows = headers(c, module)?;
    let names: HashSet<String> = rows.iter().map(|r| py_str(&r["name"])).collect();
    if columns.is_empty()
        || !unique(columns)
        || columns.iter().any(|k| !names.contains(k))
        || columns.iter().any(|k| k == "display_id")
    {
        return Err(Error::value(
            "choose unique exact runtime column names; display_id is anchored automatically",
        ));
    }
    if !columns.iter().any(|k| k == "ticket_subject") {
        return Err(Error::value(
            "captured preference contract requires ticket_subject",
        ));
    }
    if !unique(frozen) || frozen.iter().any(|k| !columns.contains(k)) {
        return Err(Error::value(
            "frozen columns must be unique selected columns",
        ));
    }
    if !frozen.is_empty() && (module != "changes" || frozen != ["requester"]) {
        return Err(Error::value(
            "only change requester freezing plus the automatic display_id anchor is captured",
        ));
    }
    let mut positions = Map::new();
    for (i, key) in columns.iter().enumerate() {
        positions.insert(key.clone(), json!(i + 1));
    }
    positions.insert("display_id".into(), positions["ticket_subject"].clone());
    let mut frozen_columns: Vec<&str> = frozen.iter().map(String::as_str).collect();
    frozen_columns.push("display_id");
    let payload = json!({"columns": positions, "frozen_columns": frozen_columns});
    Ok(ListPlan {
        plan: WritePlan {
            action: "list_columns".into(),
            target: 0,
            tenant: c.domain(),
            method: "POST".into(),
            path: format!("{module}/headers"),
            payload_json: Dumps::default().sort_keys(true).dump(&payload),
            context_hash: snapshot_hash(&json!(rows))?,
        },
        inputs_json: Dumps::default()
            .dump(&json!({"module": module, "columns": columns, "frozen": frozen})),
    })
}

/// `_EXPORT_KEYS`: export field key -> the header it is labelled from.
const EXPORT_KEYS: [(&str, &str); 10] = [
    ("subject", "ticket_subject"),
    ("priority_name", "priority"),
    ("status_name", "status"),
    ("group_name", "group"),
    ("responder_name", "assigned_to"),
    ("owner", "assigned_to"),
    ("display_id", "display_id"),
    ("requester_name", "requester"),
    ("requester_info", "requester"),
    ("department_name", "department_name"),
];

/// Port of fsv.list_preferences.export_source: the selected saved view and
/// its exact settings with an ordinary query.
pub fn export_source(
    c: &dyn Api,
    module: &str,
    view: &str,
) -> Result<(SavedView, Map<String, Value>)> {
    let res = resource(module)?;
    let (views, missing) = load_views(c, res)?;
    if missing.is_some() {
        return Err(refused("saved-view inventory is unavailable"));
    }
    let selected = find_view(&views, view).map_err(|e| Error::value(e.0))?;
    let path = res.filters_path.unwrap_or_default();
    let raw = c.int_get(&format!("{path}/{}", selected.id), None)?;
    let row = match &raw {
        Value::Object(m) => m.get(&format!("{}_filter", res.item_key)).unwrap_or(&raw),
        _ => &Value::Null,
    };
    let row = match row {
        Value::Object(r) if py_str(r.get("id").unwrap_or(&Value::Null)) == selected.id => r.clone(),
        _ => return Err(refused("exact saved-view settings are unavailable")),
    };
    if !row.get("query_hash").is_some_and(Value::is_array)
        || !row
            .get("advanced_query_hash")
            .unwrap_or(&Value::Null)
            .is_null()
    {
        return Err(refused(
            "only ordinary saved-view queries have a captured export contract",
        ));
    }
    Ok((selected, row))
}

/// Port of fsv.list_preferences.export_plan: one CSV / 30-day / created-at
/// export request for a saved view.
pub fn export_plan(c: &dyn Api, module: &str, view: &str, fields: &[String]) -> Result<ListPlan> {
    let rows = headers(c, module)?;
    let excluded: &[&str] = if module == "tickets" {
        &["owner", "department_name"]
    } else {
        &["responder_name", "requester_name"]
    };
    let allowed = |k: &str| EXPORT_KEYS.iter().any(|(e, _)| *e == k) && !excluded.contains(&k);
    if fields.is_empty() || !unique(fields) || fields.iter().any(|k| !allowed(k)) {
        return Err(Error::value("select unique captured export field keys"));
    }
    let mut export_fields = Map::new();
    for key in fields {
        let header = EXPORT_KEYS
            .iter()
            .find(|(e, _)| e == key)
            .map(|(_, h)| *h)
            .unwrap_or_default();
        let label = rows
            .iter()
            .rev()
            .find(|r| py_str(&r["name"]) == header)
            .and_then(|r| r.get("label"))
            .and_then(Value::as_str)
            .filter(|l| !fsv_core::pystr::strip(l).is_empty());
        let Some(label) = label else {
            return Err(refused(format!(
                "runtime export-field label is unavailable for {key}"
            )));
        };
        export_fields.insert(key.clone(), json!(label));
    }
    let (selected, row) = export_source(c, module, view)?;
    let mut payload = json!({"format": "csv", "date_filter": 30, "export_fields": export_fields,
                             "custom_fields": {}, "data_hash": row["query_hash"]});
    if module == "tickets" {
        payload["ticket_state_filter"] = json!("created_at");
        payload["advanced_data_hash"] = Value::Null;
        payload["selected_filter_view"] = json!({"id": selected.id, "name": selected.name,
                                                 "is_additional_filters_applied": false});
    } else {
        payload["state_filter"] = json!("created_at");
        if module == "changes" {
            payload["advanced_data_hash"] = Value::Null;
        }
    }
    let context =
        snapshot_hash(&json!({"headers": rows, "view": row, "selection": selected.to_json()}))?;
    Ok(ListPlan {
        plan: WritePlan {
            action: "list_export".into(),
            target: 0,
            tenant: c.domain(),
            method: "POST".into(),
            path: format!("{module}/export"),
            payload_json: Dumps::default().sort_keys(true).dump(&payload),
            context_hash: context,
        },
        inputs_json: Dumps::default()
            .dump(&json!({"module": module, "view": selected.id, "fields": fields})),
    })
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(py_str).collect())
        .unwrap_or_default()
}

/// Port of fsv.list_preferences.send: a columns change is read back from
/// the headers; an export is only requested, never claimed delivered.
pub fn send(
    plan: &ListPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs: Value = serde_json::from_str(&plan.inputs_json)?;
    let module = inputs["module"].as_str().unwrap_or_default().to_string();
    let columns = plan.plan.action == "list_columns";
    let payload = plan.payload();
    let rebuild = || -> Result<WritePlan> {
        Ok(if columns {
            columns_plan(
                c,
                &module,
                &strings(&inputs["columns"]),
                &strings(&inputs["frozen"]),
            )?
        } else {
            export_plan(
                c,
                &module,
                inputs["view"].as_str().unwrap_or_default(),
                &strings(&inputs["fields"]),
            )?
        }
        .plan)
    };
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        if !columns {
            let view = inputs["view"].as_str().unwrap_or_default();
            let (_, row) = export_source(c, &module, view)?;
            return Ok(obj(json!({
                "verified": false,
                "source_readback": py_str(row.get("id").unwrap_or(&Value::Null)) == view,
                "job": "unavailable", "download": "unavailable",
                "warning": "Export requested once; delivery/job/download effects are unverified. Do not resend.",
            })));
        }
        let rows = headers(c, &module)?;
        let frozen = strings(&payload["frozen_columns"]);
        let empty = Map::new();
        let mut fields = Map::new();
        let wanted = payload["columns"].as_object().cloned().unwrap_or_default();
        for (key, position) in &wanted {
            let row = rows
                .iter()
                .rev()
                .find(|r| py_str(&r["name"]) == *key)
                .unwrap_or(&empty);
            let state = if !["selected", "position", "frozen"]
                .iter()
                .all(|k| row.contains_key(*k))
            {
                "unavailable"
            } else if row["selected"] == Value::Bool(true)
                && py_equal(&row["position"], position)
                && row["frozen"] == Value::Bool(frozen.contains(key))
            {
                "matched"
            } else {
                "mismatched"
            };
            fields.insert(key.clone(), json!(state));
        }
        let omitted: Vec<Value> = rows
            .iter()
            .filter(|r| {
                !wanted.contains_key(&py_str(&r["name"]))
                    && r.get("selected") != Some(&Value::Bool(false))
            })
            .map(|r| r["name"].clone())
            .collect();
        let verified = omitted.is_empty() && fields.values().all(|v| v == "matched");
        Ok(obj(
            json!({"fields": fields, "unverified_omissions": omitted, "verified": verified}),
        ))
    };
    execute(&plan.plan, c, journal, reviewed_hash, &rebuild, &verify)
}
