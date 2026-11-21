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
    /// Export only: the signed-in agent's mailbox (the request itself names
    /// no recipient; the server mails the file to the requesting agent).
    pub recipient_json: String,
    /// Export only: `{"id", "name"}` of the saved view.
    pub view_json: String,
}

impl ListPlan {
    pub fn hash(&self) -> String {
        self.plan.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.plan.review();
        if let Ok(Value::Object(recipient)) = serde_json::from_str(&self.recipient_json)
            && !recipient.is_empty()
        {
            out.insert("recipient".into(), Value::Object(recipient));
        }
        if self.plan.action == "list_export" {
            let mut keys: Vec<String> = self
                .payload()
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            keys.sort();
            let payload = self.payload();
            out.insert("body_keys".into(), json!(keys));
            out.insert(
                "view".into(),
                serde_json::from_str(&self.view_json).unwrap_or(Value::Null),
            );
            out.insert(
                "format".into(),
                payload.get("format").cloned().unwrap_or(Value::Null),
            );
            out.insert(
                "fields".into(),
                payload.get("export_fields").cloned().unwrap_or(Value::Null),
            );
            out.insert("submit_only".into(), json!(true));
            out.insert("delivery".into(), json!(EXPORT_DELIVERY));
            out.insert("note".into(), json!(EXPORT_NOTE));
            out.insert(
                "requires".into(),
                json!("--yes --expect-plan FULL_HASH --confirm-send-to-me"),
            );
        }
        out
    }

    pub fn payload(&self) -> Value {
        self.plan.payload()
    }
}

const EXPORT_DELIVERY: &str = "emailed by the server, not downloadable here";

const EXPORT_NOTE: &str = "Export is submit-only: the server answers 202 and emails the file to the signed-in agent's own mailbox (shown as recipient), usually within 15 minutes and up to 2 hours. fsv cannot read the mailbox, so the result is never verified; check the mailbox and do not resend.";

const RECIPIENT_NOTE: &str = "the export request carries no recipient field; the server delivers the export to the requesting agent's mailbox, which fsv cannot read back";

/// Port of fsv.list_preferences.export_recipient: the signed-in agent's
/// mailbox, so the reviewer can confirm the export goes only to themselves.
/// Read-only; unavailable is stated, not guessed.
pub fn export_recipient(c: &dyn Api) -> Map<String, Value> {
    let agent = c.me().ok();
    let email = agent
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|a| a.get("email"))
        .and_then(Value::as_str)
        .filter(|e| e.contains('@') && fsv_core::pystr::strip(e) == *e);
    match email {
        Some(email) => obj(json!({"mailbox": email,
                                  "source": "signed-in agent profile (GET /api/v2/agents/me)",
                                  "note": RECIPIENT_NOTE})),
        None => obj(json!({"mailbox": "unavailable",
                           "source": "signed-in agent profile could not be read",
                           "note": format!("{RECIPIENT_NOTE}; do not send unless you know which mailbox this session belongs to")})),
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
        recipient_json: "{}".into(),
        view_json: "{}".into(),
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
pub fn export_plan(
    c: &dyn Api,
    journal: &dyn Journal,
    module: &str,
    view: &str,
    fields: &[String],
) -> Result<ListPlan> {
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
    let recipient = export_recipient(c);
    let mut bound = json!({"headers": rows, "view": row, "selection": selected.to_json(),
                           "recipient": recipient["mailbox"]});
    let accepted = journal.verified_sends(&[accepted_action(module).as_str()], &json!(0));
    if accepted != 0 {
        // a later export of the same view plans a fresh hash; an unknown outcome does not
        bound["accepted_exports"] = json!(accepted);
    }
    let context = snapshot_hash(&bound)?;
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
        recipient_json: Dumps::default()
            .sort_keys(true)
            .dump(&Value::Object(recipient)),
        view_json: Dumps::default()
            .ensure_ascii(false)
            .dump(&json!({"id": selected.id, "name": selected.name})),
    })
}

/// `accepted_action`: journal kind counting exports the server accepted
/// (2xx `success: true`).
pub fn accepted_action(module: &str) -> String {
    format!("list_export_accepted_{module}")
}

/// `send_export`: POST the reviewed export once (journaled; an unknown
/// outcome is never replayed). Delivery is by mail and cannot be read back:
/// the result says `submitted` with the HTTP status and `verified: false`.
pub fn send_export(
    plan: &ListPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs: Value = serde_json::from_str(&plan.inputs_json)?;
    let recipient: Value = serde_json::from_str(&plan.recipient_json)?;
    let mailbox = match recipient.get("mailbox") {
        Some(Value::String(m)) if m.contains('@') => m.clone(),
        _ => {
            return Err(refused(
                "the signed-in agent's mailbox is unreadable, so the export recipient is unknown; nothing was sent",
            ));
        }
    };
    let module = inputs["module"].as_str().unwrap_or_default().to_string();
    let rebuild = || -> Result<WritePlan> {
        Ok(export_plan(
            c,
            journal,
            &module,
            inputs["view"].as_str().unwrap_or_default(),
            &strings(&inputs["fields"]),
        )?
        .plan)
    };
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let accepted = response.get("success") == Some(&Value::Bool(true));
        Ok(obj(json!({
            "submitted": true, "status": c.last_status(), "accepted": accepted,
            "recipient": mailbox, "delivery": EXPORT_DELIVERY, "verified": false,
            "warning": "export is submit-only; check the mailbox (usually within 15 minutes, up to 2 hours) and do not resend",
        })))
    };
    let result = execute(&plan.plan, c, journal, reviewed_hash, &rebuild, &verify)?;
    if result.get("submitted").is_some_and(truthy) && result.get("accepted").is_some_and(truthy) {
        journal.record_verified(&plan.hash(), &accepted_action(&module), Some(&json!(0)));
    }
    Ok(result)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(py_str).collect())
        .unwrap_or_default()
}

/// Port of fsv.list_preferences.send: a columns change is read back from
/// the headers; an export is submitted once ([`send_export`]).
pub fn send(
    plan: &ListPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs: Value = serde_json::from_str(&plan.inputs_json)?;
    let module = inputs["module"].as_str().unwrap_or_default().to_string();
    if plan.plan.action != "list_columns" {
        return send_export(plan, c, journal, reviewed_hash);
    }
    let payload = plan.payload();
    let rebuild = || -> Result<WritePlan> {
        Ok(columns_plan(
            c,
            &module,
            &strings(&inputs["columns"]),
            &strings(&inputs["frozen"]),
        )?
        .plan)
    };
    let verify = |_: &Value| -> Result<Map<String, Value>> {
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

#[cfg(test)]
mod export_tests {
    use std::sync::{Arc, Mutex};

    use fsv_core::{ApiError, Error};

    use super::*;
    use crate::service::testing::{Call, FnApi, MemJournal};

    const EMAIL: &str = "agent@example.com";

    #[derive(Default)]
    struct World {
        row: Value,
        failure: Option<Value>,
        writes: usize,
    }

    fn headers() -> Value {
        json!([
            {"name": "ticket_subject", "label": "Acme subject", "position": 1, "selected": true, "frozen": false},
            {"name": "status", "label": "Acme state", "position": 2, "selected": true, "frozen": false},
            {"name": "display_id", "label": "Acme ID", "position": 3, "selected": true, "frozen": true},
        ])
    }

    fn view_row(query_hash: Value) -> Value {
        json!({"id": 701, "name": "Acme queue", "query_hash": query_hash, "advanced_query_hash": null})
    }

    /// The `tests/test_list_preferences.py` wire for `module`; `me` is the
    /// signed-in agent and `string_values_only` the trial's server rule for
    /// `data_hash`.
    fn api(module: &'static str, me: Value) -> (Arc<Mutex<World>>, FnApi) {
        let world = Arc::new(Mutex::new(World {
            row: view_row(
                json!([{"condition": "status", "operator": "is_in", "value": ["47"], "type": "default"}]),
            ),
            ..World::default()
        }));
        let shared = world.clone();
        let filters = match module {
            "tickets" => "ticket_filters",
            "changes" => "change-filters",
            _ => "problem-filters",
        };
        let api = FnApi::new(move |call: &Call| {
            if call.method == "ME" {
                return Ok(me.clone());
            }
            let mut w = shared.lock().unwrap();
            if call.method == "GET" {
                if call.path == format!("{module}/headers") {
                    return Ok(json!({"headers": headers()}));
                }
                if call.path == filters {
                    return Ok(json!({"filters": [w.row.clone()]}));
                }
                if call.path == format!("{filters}/701") {
                    return Ok(
                        json!({format!("{}_filter", &module[..module.len() - 1]): w.row.clone()}),
                    );
                }
                panic!("unexpected read {}", call.path);
            }
            assert_eq!(call.path, format!("{module}/export"));
            w.writes += 1;
            match w.failure.clone() {
                Some(Value::String(_)) => Err(Error::Http {
                    kind: "ReadError".into(),
                    message: "synthetic disconnect".into(),
                }),
                Some(n) => Err(ApiError::new(
                    n.as_u64().unwrap() as u16,
                    json!({"error": "synthetic"}),
                )
                .into()),
                None => {
                    let data = &call.body.as_ref().unwrap()["data_hash"];
                    let strings = data.as_array().unwrap().iter().all(|c| match &c["value"] {
                        Value::String(_) => true,
                        Value::Array(v) => v.iter().all(Value::is_string),
                        _ => false,
                    });
                    if strings {
                        Ok(json!({"success": true}))
                    } else {
                        Err(ApiError::new(400, json!({"description": "Validation failed"})).into())
                    }
                }
            }
        });
        (world, api)
    }

    fn agent() -> Value {
        json!({"id": 9000001, "email": EMAIL})
    }

    fn fields() -> Vec<String> {
        vec!["subject".into(), "display_id".into()]
    }

    fn plan(c: &FnApi, j: &MemJournal, module: &str) -> ListPlan {
        export_plan(c, j, module, "701", &fields()).unwrap()
    }

    #[test]
    fn export_review_carries_view_format_fields_and_the_send_requirements() {
        for module in ["tickets", "changes", "problems"] {
            let (_, c) = api(module, agent());
            let j = MemJournal::default();
            let p = plan(&c, &j, module);
            let review = p.review();
            assert_eq!(review["view"], json!({"id": "701", "name": "Acme queue"}));
            assert_eq!(review["format"], json!("csv"));
            assert_eq!(review["fields"], p.payload()["export_fields"]);
            assert_eq!(review["submit_only"], json!(true));
            assert_eq!(
                review["delivery"],
                json!("emailed by the server, not downloadable here")
            );
            assert!(review["note"].as_str().unwrap().contains("submit-only"));
            assert_eq!(review["recipient"]["mailbox"], json!(EMAIL));
            assert_eq!(
                review["requires"],
                json!("--yes --expect-plan FULL_HASH --confirm-send-to-me")
            );
            assert!(c.writes().is_empty());
        }
    }

    #[test]
    fn export_is_submitted_once_journaled_and_never_replayed() {
        let (world, c) = api("tickets", agent());
        let j = MemJournal::default();
        let p = plan(&c, &j, "tickets");
        let sent = send(&p, &c, &j, Some(&p.hash())).unwrap();
        assert_eq!(sent["submitted"], json!(true));
        assert_eq!(sent["accepted"], json!(true));
        assert_eq!(sent["verified"], json!(false));
        assert_eq!(sent["recipient"], json!(EMAIL));
        assert!(
            sent["delivery"]
                .as_str()
                .unwrap()
                .contains("not downloadable")
        );
        assert_eq!(sent["plan_hash"], json!(p.hash()));
        assert_eq!(world.lock().unwrap().writes, 1);
        assert!(
            c.writes()[0]
                .raw
                .as_ref()
                .is_some_and(|b| !String::from_utf8_lossy(b).contains("recipient"))
        );
        let again = send(&p, &c, &j, Some(&p.hash())).unwrap_err();
        assert_eq!(again.exit_code(), 2);
        assert_eq!(world.lock().unwrap().writes, 1);
        // an accepted export lets a fresh review of the same view plan a new hash
        let fresh = plan(&c, &j, "tickets");
        assert_ne!(fresh.hash(), p.hash());
    }

    #[test]
    fn export_unknown_outcome_is_never_replayed() {
        for failure in [json!(500), json!("disconnect")] {
            let (world, c) = api("tickets", agent());
            let j = MemJournal::default();
            let p = plan(&c, &j, "tickets");
            world.lock().unwrap().failure = Some(failure.clone());
            let err = send(&p, &c, &j, Some(&p.hash())).unwrap_err();
            assert!(err.to_string().contains("unknown"), "{failure}: {err}");
            let again = plan(&c, &j, "tickets");
            assert_eq!(again.hash(), p.hash());
            let err = send(&again, &c, &j, Some(&again.hash())).unwrap_err();
            assert!(err.to_string().contains("was sent"), "{failure}: {err}");
            assert_eq!(world.lock().unwrap().writes, 1);
        }
    }

    #[test]
    fn export_refused_when_the_mailbox_is_unreadable() {
        let (world, c) = api("tickets", json!({"id": 9000001}));
        let j = MemJournal::default();
        let p = plan(&c, &j, "tickets");
        let err = send(&p, &c, &j, Some(&p.hash())).unwrap_err();
        assert!(err.to_string().contains("mailbox"), "{err}");
        assert_eq!(world.lock().unwrap().writes, 0);
    }

    #[test]
    fn export_hash_binds_the_session_mailbox() {
        let (_, c) = api("tickets", agent());
        let j = MemJournal::default();
        let first = plan(&c, &j, "tickets");
        let (_, other) = api("tickets", json!({"email": "other@example.com"}));
        assert_ne!(plan(&other, &j, "tickets").hash(), first.hash());
    }
}
