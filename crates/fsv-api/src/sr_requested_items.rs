//! Captured service-request requested items: plain-text answers, the
//! Delivered stage, add and non-parent removal (`sr_requested_items.py`).
//!
//! The plan's `context_hash` is `json.dumps(stable_snapshot(context),
//! sort_keys=True)`: the parent, the inventory, the detail snapshot and the
//! item metadata fingerprint.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::strip;
use fsv_core::resource::{TICKETS, parse_id};
use fsv_forms::value::values_from_json;
use fsv_forms::{FormState, Kind, StateOptions};
use fsv_py::{py_str, truthy};

use crate::attachments::owner_row;
use crate::catalog;
use crate::formmeta::{MetaStore, apply_answers, form_err};
use crate::readback::submitted_value_matches;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, stable_snapshot};
use crate::watchers::positive_id;

fn dumps_sorted(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

fn is_int(v: &Value) -> bool {
    matches!(v, Value::Number(n) if n.is_i64() || n.is_u64())
}

fn row_id(row: &Value) -> Result<i64> {
    positive_id(row.get("id").unwrap_or(&Value::Null))
}

/// `inventory`: every requested item of `target`, sorted by id.
pub fn inventory(c: &dyn Api, target: i64) -> Result<Vec<Value>> {
    let raw = c.int_get(&format!("tickets/{target}/requested_items"), None)?;
    if raw
        .get("requested_items")
        .and_then(Value::as_array)
        .is_none()
    {
        return Err(refused("requested-item inventory unavailable"));
    }
    let rows = crate::reviewed_inventory::single_page_inventory(&raw, "requested_items")?;
    let target_text = target.to_string();
    if rows
        .iter()
        .any(|r| py_str(r.get("ticket_id").unwrap_or(&Value::Null)) != target_text)
    {
        return Err(refused(
            "requested item belongs to a different or unavailable ticket",
        ));
    }
    let mut rows = rows;
    rows.sort_by_key(|r| row_id(r).unwrap_or(0));
    Ok(rows)
}

/// `detail`: the `more_info` view of one requested item.
pub fn detail(c: &dyn Api, target: i64, item: i64) -> Result<Value> {
    let raw = c.int_get(
        &format!("tickets/{target}/requested_items/{item}"),
        Some(obj(json!({"view": "more_info"}))),
    )?;
    let exact = raw
        .get("requested_item")
        .filter(|r| r.is_object())
        .map(|r| row_id(r).ok() == Some(item))
        .unwrap_or(false);
    if !exact {
        return Err(refused("exact requested-item detail unavailable"));
    }
    Ok(raw)
}

/// `text_answers`: plain text answers validated through the catalog rules;
/// returns the item metadata fingerprint.
pub fn text_answers(
    c: &dyn Api,
    store: &dyn MetaStore,
    raw_item: &Value,
    answers: &Map<String, Value>,
    existing: Option<&Value>,
) -> Result<String> {
    let Some(fields) = raw_item.get("custom_fields").and_then(Value::as_array) else {
        return Err(Error::value(
            "nonempty text answers and item field metadata are required",
        ));
    };
    if answers.is_empty() {
        return Err(Error::value(
            "nonempty text answers and item field metadata are required",
        ));
    }
    let display = raw_item.get("display_id").map(py_str).unwrap_or_default();
    let meta = catalog::load_item_meta(c, store, &display, Some(raw_item))?;
    let spec = meta.spec();
    for (name, value) in answers {
        let found: Vec<&Value> = fields
            .iter()
            .filter(|f| f.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .collect();
        let plain = spec
            .get(name)
            .is_some_and(|f| matches!(f.kind, Kind::Text | Kind::Textarea));
        if found.len() != 1 || !plain {
            return Err(refused(
                "only captured plain text/paragraph answer types are supported",
            ));
        }
        let text_ok = value
            .as_str()
            .is_some_and(|s| !strip(s).is_empty() && !s.contains('\0'));
        if !text_ok {
            return Err(Error::value("requested-item answers must be nonempty text"));
        }
        if found[0].get("deleted").is_some_and(truthy)
            || found[0].get("archived").is_some_and(truthy)
        {
            return Err(refused("requested-item field is unavailable"));
        }
    }
    let editing = existing.filter(|v| !v.is_null());
    let mode = if editing.is_some() { "edit" } else { "new" };
    let mut state = FormState::new(
        spec.clone(),
        Some(meta.engine()?),
        mode,
        None,
        StateOptions {
            existing: editing.and_then(Value::as_object).map(values_from_json),
            ..StateOptions::default()
        },
    )
    .map_err(form_err)?;
    let mut errors = apply_answers(&mut state, answers, None)?;
    let (_, rule_errors) = catalog::rule_diagnostics(&meta, &state);
    errors.extend(state.errors().map_err(form_err)?);
    errors.extend(rule_errors);
    if !errors.is_empty() {
        return Err(refused(format!(
            "requested-item validation failed: {}",
            errors.join("; ")
        )));
    }
    if answers
        .keys()
        .any(|name| !state.evaluated.visible.contains(name))
    {
        return Err(refused("requested-item answer is hidden"));
    }
    // Catalog reads carry signed icon/attachment URLs that rotate per GET.
    Ok(fsv_forms::plan::metadata_fingerprint(
        None,
        Some(&stable_snapshot(&meta.rules)?),
        Some(&stable_snapshot(&meta.item)?),
    )
    .hash)
}

/// Keywords of [`build_plan`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ItemArgs<'a> {
    pub action: &'a str,
    pub item: Option<i64>,
    pub catalog_item: Option<&'a str>,
    pub answers: Option<&'a Map<String, Value>>,
    pub stage: Option<i64>,
}

/// `build_plan`.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    target: &str,
    args: &ItemArgs<'_>,
) -> Result<WritePlan> {
    let cid = positive_id(&json!(parse_id(target, &TICKETS)?))?;
    if !matches!(args.action, "edit" | "stage" | "add" | "remove") {
        return Err(Error::value("unsupported requested-item action"));
    }
    let parent = owner_row(c, "tickets", cid)?;
    crate::ticket_kind::require_kind(
        &Value::Object(parent.clone()),
        crate::ticket_kind::SERVICE_REQUEST,
        "exact parent",
    )?;
    let rows = inventory(c, cid)?;
    let empty = Map::new();
    let answers = args.answers.unwrap_or(&empty);
    let mut path = format!("tickets/{cid}/requested_items");
    let mut metadata_hash = Value::Null;
    let mut snapshot = Value::Null;
    let mut item = args.item;
    let (method, payload) = if args.action == "add" {
        let catalog_ok = args.catalog_item.is_some_and(|s| {
            !s.is_empty()
                && s.bytes().all(|b| b.is_ascii_digit())
                && s.parse::<u128>().is_ok_and(|n| n > 0)
        });
        if args.item.is_some() || args.stage.is_some() || !catalog_ok {
            return Err(Error::value(
                "add needs an exact positive catalog display ID as text, answers, and no existing item/stage",
            ));
        }
        let catalog_item = args.catalog_item.unwrap();
        let raw_item = catalog::get_item(c, &json!(catalog_item))?;
        if raw_item.get("display_id").map_or("None".into(), py_str) != catalog_item
            || raw_item.get("deleted").is_some_and(truthy)
        {
            return Err(refused("catalog display identity changed/unavailable"));
        }
        metadata_hash = json!(text_answers(c, store, &raw_item, answers, None)?);
        (
            "POST",
            json!({"item_id": catalog_item, "custom_fields": args.answers}),
        )
    } else {
        let ident = positive_id(&json!(args.item))?;
        item = Some(ident);
        let exact: Vec<&Value> = rows
            .iter()
            .filter(|r| row_id(r).ok() == Some(ident))
            .collect();
        if exact.len() != 1 || args.catalog_item.is_some() {
            return Err(refused("exact requested-item identity unavailable"));
        }
        path.push_str(&format!("/{ident}"));
        if args.action == "remove" {
            if exact[0].get("is_parent") != Some(&Value::Bool(false))
                || !answers.is_empty()
                || args.stage.is_some()
            {
                return Err(refused(
                    "only an explicitly non-parent item may be removed, without answer/stage edits",
                ));
            }
            ("DELETE", Value::Null)
        } else if args.action == "stage" {
            if args.stage != Some(2) || !answers.is_empty() {
                return Err(Error::value(
                    "only captured stage 2 (observed Delivered) is supported; no combined answer update",
                ));
            }
            path.push_str("/update_stage");
            ("PATCH", json!({"stage": 2}))
        } else {
            if args.stage.is_some() {
                return Err(Error::value("stage is a separate requested-item action"));
            }
            snapshot = detail(c, cid, ident)?;
            let row = snapshot["requested_item"].clone();
            let Some(rules) = snapshot.get("business_rules").and_then(Value::as_object) else {
                return Err(refused("requested-item editability rules unavailable"));
            };
            for key in ["hidden_req_item_fields", "non_editable_item_fields"] {
                let open = rules
                    .get(key)
                    .and_then(Value::as_object)
                    .and_then(|m| m.get(&ident.to_string()))
                    == Some(&json!([]));
                if !open {
                    return Err(refused(
                        "nonempty or unavailable item editability restrictions require a new capture",
                    ));
                }
            }
            let linked = exact[0].get("item").cloned().unwrap_or(json!({}));
            let display_id = positive_id(linked.get("display_id").unwrap_or(&Value::Null))?;
            let raw_item = catalog::get_item(c, &json!(display_id))?;
            let text = |v: Option<&Value>| v.map_or("None".into(), py_str);
            if text(raw_item.get("id")) != text(linked.get("id"))
                || text(raw_item.get("display_id")) != display_id.to_string()
            {
                return Err(refused("requested-item catalog identity changed"));
            }
            metadata_hash = json!(text_answers(
                c,
                store,
                &raw_item,
                answers,
                row.get("custom_fields")
            )?);
            ("PUT", json!({"custom_fields": args.answers}))
        }
    };
    let intent = json!({
        "action": args.action,
        "answers": args.answers,
        "catalog_item": args.catalog_item,
        "item": item,
        "stage": args.stage,
        "target": cid,
    });
    let context = json!({
        "detail": snapshot,
        "intent": intent,
        "metadata": metadata_hash,
        "parent": Value::Object(parent),
        "rows": rows,
    });
    Ok(WritePlan {
        action: format!("sr_requested_item_{}", args.action),
        target: cid,
        tenant: c.domain(),
        method: method.into(),
        path,
        payload_json: dumps_sorted(&payload),
        context_hash: dumps_sorted(&stable_snapshot(&context)?),
    })
}

fn plan_from_intent(c: &dyn Api, store: &dyn MetaStore, intent: &Value) -> Result<WritePlan> {
    let action = intent["action"].as_str().unwrap_or("").to_string();
    let catalog_item = intent["catalog_item"].as_str().map(str::to_string);
    build_plan(
        c,
        store,
        &py_str(&intent["target"]),
        &ItemArgs {
            action: &action,
            item: intent["item"].as_i64(),
            catalog_item: catalog_item.as_deref(),
            answers: intent["answers"].as_object(),
            stage: intent["stage"].as_i64(),
        },
    )
}

fn verify(plan: &WritePlan, c: &dyn Api, response: &Value) -> Result<Map<String, Value>> {
    let before: Value = serde_json::from_str(&plan.context_hash).unwrap_or(Value::Null);
    let intent = &before["intent"];
    let action = intent["action"].as_str().unwrap_or("");
    let rows = inventory(c, plan.target)?;
    let old: BTreeSet<i64> = before["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(row_id)
        .collect::<Result<_>>()?;
    let current: BTreeSet<i64> = rows.iter().map(row_id).collect::<Result<_>>()?;
    let ident = if action == "add" {
        let returned = response.get("requested_item").cloned().unwrap_or(json!({}));
        positive_id(returned.get("id").unwrap_or(&Value::Null))?
    } else {
        intent["item"].as_i64().unwrap_or(0)
    };
    if action == "remove" {
        let mut wanted = old.clone();
        wanted.remove(&ident);
        return Ok(obj(
            json!({"item_id": ident, "verified": current == wanted}),
        ));
    }
    let exact: Vec<&Value> = rows
        .iter()
        .filter(|r| row_id(r).ok() == Some(ident))
        .collect();
    let mut wanted = old.clone();
    if action == "add" {
        wanted.insert(ident);
    }
    if exact.len() != 1 || current != wanted {
        return Ok(obj(json!({"item_id": ident, "verified": false})));
    }
    if action == "stage" {
        let actual = exact[0].get("stage");
        let verified = actual
            .and_then(|s| s.get("id"))
            .is_some_and(|id| is_int(id) && id.as_i64() == intent["stage"].as_i64());
        return Ok(obj(json!({"item_id": ident, "verified": verified})));
    }
    let raw = detail(c, plan.target, ident)?["requested_item"].clone();
    let actual = raw.get("custom_fields").cloned().unwrap_or(json!({}));
    let mut fields = Map::new();
    for (k, v) in intent["answers"].as_object().into_iter().flatten() {
        let verdict = match actual.get(k) {
            None => "unavailable",
            Some(a) if submitted_value_matches(a, v) => "matched",
            Some(_) => "mismatched",
        };
        fields.insert(k.clone(), json!(verdict));
    }
    let identity = action != "add"
        || (exact[0]
            .get("item")
            .and_then(|i| i.get("display_id"))
            .map_or("None".into(), py_str)
            == intent["catalog_item"].as_str().unwrap_or("")
            && !old.contains(&ident));
    let verified = identity && fields.values().all(|v| v == "matched");
    Ok(obj(
        json!({"item_id": ident, "fields": fields, "verified": verified}),
    ))
}

/// `send`.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let before: Value = serde_json::from_str(&plan.context_hash).unwrap_or(Value::Null);
    let rebuild = || plan_from_intent(c, store, &before["intent"]);
    let check = |response: &Value| verify(plan, c, response);
    execute(plan, c, journal, reviewed_hash, &rebuild, &check)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, LazyLock, Mutex};

    use fsv_core::error::ApiError;
    use serde_json::{Value, json};

    use super::*;
    use crate::formmeta::MemStore;
    use crate::service::testing::{Call, FnApi, MemJournal};

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/sr_requested_items_oracle.json")).unwrap()
    });

    fn fields() -> Value {
        json!([
            {"id": 1, "name": "title", "label": "Title", "field_type": "custom_text", "type": 1001,
             "required": true, "field_options": {}, "position": 1},
            {"id": 2, "name": "details", "label": "Details", "field_type": "custom_paragraph", "type": 1008,
             "required": false, "field_options": {}, "position": 2},
        ])
    }

    fn item() -> Value {
        json!({"id": 801, "display_id": 71, "name": "Acme service", "workspace_id": 2, "custom_fields": fields()})
    }

    #[derive(Default)]
    struct World {
        rows: Vec<Value>,
        answers: BTreeMap<i64, Value>,
        writes: usize,
        blocked: bool,
        missing: bool,
        rules_fail: bool,
        fail: bool,
        kind_fields: Option<Value>,
        rotate: bool,
        signed: usize,
        shard: Option<String>,
    }

    fn world() -> World {
        let mut answers = BTreeMap::new();
        answers.insert(
            901,
            json!({"title": "Original", "details": "Original details"}),
        );
        World {
            rows: vec![
                json!({"id": 901, "ticket_id": 123, "item_id": 801, "item": item(),
                "is_parent": true, "stage": {"id": 1, "name": "Requested"}}),
            ],
            answers,
            ..World::default()
        }
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            let path = call.path.as_str();
            if call.method == "GET" {
                if path == "tickets/123" {
                    let mut row = json!({"id": 123});
                    let kind = s
                        .kind_fields
                        .clone()
                        .unwrap_or_else(|| json!({"ticket_type": "Service Request"}));
                    for (k, v) in kind.as_object().into_iter().flatten() {
                        row[k] = v.clone();
                    }
                    return Ok(json!({ "ticket": row }));
                }
                if path.contains("business_rules") {
                    if s.rules_fail {
                        return Err(ApiError::new(403, json!({"error": "denied"})).into());
                    }
                    return Ok(json!({"business_rules": []}));
                }
                if path == "catalog/service-items/71" && !s.rotate && s.shard.is_none() {
                    return Ok(json!({"service_item": item()}));
                }
                if path == "catalog/service-items/71" {
                    s.signed += 1;
                    let mut signed = item();
                    signed["icon_url"] = json!(format!(
                        "https://cdn.example.com/icons/71.png?X-Amz-Date=2026{:04}&X-Amz-Signature=sig{}&X-Amz-Expires=300",
                        s.signed, s.signed
                    ));
                    if let Some(shard) = &s.shard {
                        signed["icon_url"] = json!(format!(
                            "https://assets{}.freshservice.com/assets/cdn-ignored/{shard}",
                            s.signed % 9 + 1
                        ));
                    }
                    return Ok(json!({ "service_item": signed }));
                }
                if path == "tickets/123/requested_items" {
                    return Ok(json!({"requested_items": s.rows.clone(),
                        "meta": {"e_signature_limit_reached": false}}));
                }
                let Ok(ident) = path.rsplit('/').next().unwrap().parse::<i64>() else {
                    return Err(ApiError::new(404, json!({"error": path})).into());
                };
                let mut answers = s.answers[&ident].clone();
                if s.missing && s.writes > 0 {
                    answers.as_object_mut().unwrap().remove("details");
                }
                let hidden = if s.blocked {
                    json!(["details"])
                } else {
                    json!([])
                };
                return Ok(json!({
                    "requested_item": {"id": ident, "item_id": 71, "custom_fields": answers,
                        "item": {"custom_fields": fields(), "description": "Acme item"}},
                    "business_rules": {
                        "hidden_req_item_fields": {ident.to_string(): hidden},
                        "non_editable_item_fields": {ident.to_string(): []},
                    },
                }));
            }
            s.writes += 1;
            if s.fail {
                return Err(Error::session("disconnect"));
            }
            let body = call.body.clone().unwrap_or(Value::Null);
            if call.method == "POST" {
                s.rows.push(
                    json!({"id": 902, "ticket_id": 123, "item_id": 801, "item": item(),
                    "is_parent": false, "stage": {"id": 1, "name": "Requested"}}),
                );
                s.answers.insert(902, body["custom_fields"].clone());
                return Ok(json!({"requested_item": {"id": 902, "item_id": 71}}));
            }
            let ident: i64 = path.split('/').nth(3).unwrap().parse().unwrap();
            match call.method.as_str() {
                "DELETE" => {
                    s.rows.retain(|r| r["id"] != json!(ident));
                    return Ok(Value::Null);
                }
                "PATCH" => {
                    let row = s.rows.iter_mut().find(|r| r["id"] == json!(ident)).unwrap();
                    row["stage"] = json!({"id": 2, "name": "Delivered"});
                }
                _ => {
                    let answers = s.answers.get_mut(&ident).unwrap().as_object_mut().unwrap();
                    for (k, v) in body["custom_fields"].as_object().unwrap() {
                        answers.insert(k.clone(), v.clone());
                    }
                }
            }
            Ok(json!({"requested_item": {"id": ident, "item_id": 801}}))
        })
    }

    fn writes(c: &FnApi) -> Vec<Call> {
        c.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.method != "GET")
            .cloned()
            .collect()
    }

    fn edit_answers() -> Map<String, Value> {
        obj(json!({"title": "Acme edited", "details": "Updated details"}))
    }

    fn add_answers() -> Map<String, Value> {
        obj(json!({"title": "Acme request", "details": "Additional item"}))
    }

    fn args<'a>(action: &'a str, answers: &'a Map<String, Value>) -> ItemArgs<'a> {
        match action {
            "add" => ItemArgs {
                action,
                catalog_item: Some("71"),
                answers: Some(answers),
                ..ItemArgs::default()
            },
            "edit" => ItemArgs {
                action,
                item: Some(901),
                answers: Some(answers),
                ..ItemArgs::default()
            },
            "stage" => ItemArgs {
                action,
                item: Some(901),
                stage: Some(2),
                ..ItemArgs::default()
            },
            _ => ItemArgs {
                action,
                item: Some(902),
                ..ItemArgs::default()
            },
        }
    }

    /// `test_parent_type_proof_uses_native_exact_read`.
    #[test]
    fn parent_type_proof_uses_native_exact_read() {
        for (fields, allowed) in [
            (json!({"type": "Service Request"}), true),
            (json!({"ticket_type": "Service Request"}), true),
            (
                json!({"type": "Service Request", "ticket_type": "service_request"}),
                true,
            ),
            (json!({"type": "Incident"}), false),
            (
                json!({"type": "Service Request", "ticket_type": "Incident"}),
                false,
            ),
            (json!({}), false),
            (json!({"type": null}), false),
        ] {
            for action in ["edit", "stage", "add"] {
                let mut w = world();
                w.kind_fields = Some(fields.clone());
                let c = api(Arc::new(Mutex::new(w)));
                let answers = if action == "add" {
                    add_answers()
                } else {
                    edit_answers()
                };
                let got = build_plan(&c, &MemStore::default(), "SR-123", &args(action, &answers));
                if allowed {
                    let plan = got.unwrap();
                    assert!(
                        matches!(plan.method.as_str(), "PUT" | "PATCH" | "POST"),
                        "{fields} {action}"
                    );
                } else {
                    let err = got.unwrap_err().to_string();
                    assert!(
                        err.contains("service request") || err.contains("ticket type"),
                        "{fields} {action}: {err}"
                    );
                }
                assert!(writes(&c).is_empty());
            }
        }
    }

    fn wire(action: &str, missing: bool) -> (WritePlan, Value, Vec<Call>) {
        let mut w = world();
        if action == "remove" {
            let mut extra = w.rows[0].clone();
            extra["id"] = json!(902);
            extra["is_parent"] = json!(false);
            w.rows.push(extra);
        }
        let state = Arc::new(Mutex::new(w));
        let c = api(Arc::clone(&state));
        let store = MemStore::default();
        let answers = if action == "add" {
            add_answers()
        } else {
            edit_answers()
        };
        let plan = build_plan(&c, &store, "SR-123", &args(action, &answers)).unwrap();
        assert!(writes(&c).is_empty(), "{action}");
        state.lock().unwrap().missing = missing;
        let journal = MemJournal::default();
        let result = send(&plan, &c, &store, &journal, Some(&plan.hash())).unwrap();
        let err = send(&plan, &c, &store, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("was sent"), "{err}");
        (plan, result, writes(&c))
    }

    /// `test_edit_plan_hash_stable_across_rotating_signed_catalog_urls`.
    #[test]
    fn edit_plan_hash_stable_across_rotating_signed_catalog_urls() {
        let state = Arc::new(Mutex::new(World {
            rotate: true,
            ..world()
        }));
        let c = api(Arc::clone(&state));
        let store = MemStore::default();
        let answers = edit_answers();
        let first = build_plan(&c, &store, "SR-123", &args("edit", &answers)).unwrap();
        let second =
            build_plan(&c, &MemStore::default(), "SR-123", &args("edit", &answers)).unwrap();
        assert!(state.lock().unwrap().signed >= 2);
        assert_eq!(first.hash(), second.hash());
        state.lock().unwrap().rows[0]["item"]["custom_fields"][0]["label"] = json!("Renamed");
        let renamed =
            build_plan(&c, &MemStore::default(), "SR-123", &args("edit", &answers)).unwrap();
        assert_ne!(renamed.hash(), first.hash());
    }

    /// `test_plan_hash_stable_across_rotating_cdn_shard_hosts`.
    #[test]
    fn plan_hash_stable_across_rotating_cdn_shard_hosts() {
        for action in ["edit", "add"] {
            let state = Arc::new(Mutex::new(World {
                shard: Some("images/icons/acme.png".into()),
                ..world()
            }));
            let c = api(Arc::clone(&state));
            let answers = if action == "edit" {
                edit_answers()
            } else {
                add_answers()
            };
            let plan = || build_plan(&c, &MemStore::default(), "SR-123", &args(action, &answers));
            let first = plan().unwrap();
            let second = plan().unwrap();
            assert!(state.lock().unwrap().signed >= 2, "{action}");
            assert_eq!(first.hash(), second.hash(), "{action}");
            state.lock().unwrap().shard = Some("images/icons/globex.png".into());
            assert_ne!(plan().unwrap().hash(), first.hash(), "{action}");
        }
    }

    #[test]
    fn wires_match_python() {
        for (key, action, missing) in [
            ("edit", "edit", false),
            ("stage", "stage", false),
            ("add", "add", false),
            ("remove", "remove", false),
            ("edit-missing", "edit", true),
        ] {
            let (plan, result, sent) = wire(action, missing);
            let want = &ORACLE[key];
            let p = &want["plan"];
            assert_eq!(plan.action, p["action"].as_str().unwrap(), "{key}");
            assert_eq!(plan.method, p["method"].as_str().unwrap(), "{key}");
            assert_eq!(plan.path, p["path"].as_str().unwrap(), "{key}");
            assert_eq!(plan.payload_json, p["payload"].as_str().unwrap(), "{key}");
            assert_eq!(plan.tenant, p["tenant"].as_str().unwrap(), "{key}");
            assert_eq!(plan.target, p["target"].as_i64().unwrap(), "{key}");
            assert_eq!(plan.context_hash, p["context"].as_str().unwrap(), "{key}");
            assert_eq!(plan.hash(), p["hash"].as_str().unwrap(), "{key}");
            let wired: Vec<Value> = sent
                .iter()
                .map(|c| json!([c.method, c.path, c.body]))
                .collect();
            assert_eq!(Value::Array(wired), want["writes"], "{key}");
            for k in ["item_id", "fields", "verified"] {
                assert_eq!(result.get(k), want["result"].get(k), "{key} {k}");
            }
        }
    }

    #[test]
    fn refusals_match_python() {
        let edit = edit_answers();
        let nope = obj(json!({"nope": "x"}));
        let blank = obj(json!({"title": "  "}));
        let short = obj(json!({"title": "t"}));
        let cases: Vec<(&str, Option<&str>, ItemArgs)> = vec![
            ("blocked", Some("blocked"), args("edit", &edit)),
            ("rules_fail", Some("rules_fail"), args("edit", &edit)),
            (
                "parent_remove",
                None,
                ItemArgs {
                    action: "remove",
                    item: Some(901),
                    ..ItemArgs::default()
                },
            ),
            (
                "stage_3",
                None,
                ItemArgs {
                    action: "stage",
                    item: Some(901),
                    stage: Some(3),
                    ..ItemArgs::default()
                },
            ),
            (
                "hidden_type",
                None,
                ItemArgs {
                    action: "edit",
                    item: Some(901),
                    answers: Some(&nope),
                    ..ItemArgs::default()
                },
            ),
            (
                "blank",
                None,
                ItemArgs {
                    action: "edit",
                    item: Some(901),
                    answers: Some(&blank),
                    ..ItemArgs::default()
                },
            ),
            (
                "add_bad",
                None,
                ItemArgs {
                    action: "add",
                    catalog_item: Some("x71"),
                    answers: Some(&short),
                    ..ItemArgs::default()
                },
            ),
            (
                "bad_action",
                None,
                ItemArgs {
                    action: "move",
                    item: Some(901),
                    ..ItemArgs::default()
                },
            ),
        ];
        for (key, mode, a) in cases {
            let mut w = world();
            match mode {
                Some("blocked") => w.blocked = true,
                Some("rules_fail") => w.rules_fail = true,
                _ => {}
            }
            let c = api(Arc::new(Mutex::new(w)));
            let err = build_plan(&c, &MemStore::default(), "123", &a).unwrap_err();
            let want = &ORACLE["refusals"][key];
            let kind = if matches!(err, Error::Value(_)) {
                "ValueError"
            } else {
                "ReviewedWriteError"
            };
            assert_eq!(kind, want["kind"].as_str().unwrap(), "{key}: {err}");
            assert_eq!(err.to_string(), want["message"].as_str().unwrap(), "{key}");
            assert!(writes(&c).is_empty(), "{key}");
        }
    }

    #[test]
    fn ambiguous_send_never_replays() {
        let state = Arc::new(Mutex::new(world()));
        let c = api(Arc::clone(&state));
        let store = MemStore::default();
        let answers = edit_answers();
        let plan = build_plan(&c, &store, "123", &args("edit", &answers)).unwrap();
        state.lock().unwrap().fail = true;
        let journal = MemJournal::default();
        let err = send(&plan, &c, &store, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("unknown"), "{err}");
        let err = send(&plan, &c, &store, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("was sent"), "{err}");
        assert_eq!(writes(&c).len(), 1);
    }

    #[test]
    fn inventory_fails_closed() {
        let incomplete = "complete, consistent mutation inventory unavailable";
        let cases = [
            (json!({"rows": []}), "requested-item inventory unavailable"),
            (
                json!({"requested_items": [], "meta": {"has_more": true}}),
                incomplete,
            ),
            (
                json!({"requested_items": [{"id": 1, "ticket_id": 123}, {"id": 1, "ticket_id": 123}]}),
                incomplete,
            ),
            (
                json!({"requested_items": [{"id": 1, "ticket_id": 123}], "total": 2}),
                incomplete,
            ),
            (
                json!({"requested_items": [{"id": 1, "ticket_id": 124}]}),
                "requested item belongs to a different or unavailable ticket",
            ),
        ];
        for (reply, msg) in cases {
            let c = FnApi::new(move |_: &Call| Ok(reply.clone()));
            assert_eq!(inventory(&c, 123).unwrap_err().to_string(), msg);
        }
        let c = FnApi::new(|_: &Call| {
            Ok(
                json!({"requested_items": [{"id": 3, "ticket_id": 123}, {"id": 2, "ticket_id": "123"}],
                "meta": {"total": 2}}),
            )
        });
        let ids: Vec<Value> = inventory(&c, 123)
            .unwrap()
            .iter()
            .map(|r| r["id"].clone())
            .collect();
        assert_eq!(ids, [json!(2), json!(3)]);
    }
}
