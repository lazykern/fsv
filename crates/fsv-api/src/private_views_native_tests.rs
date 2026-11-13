//! Private views from native read shapes (numeric Myself code, workspace in
//! `meta` and the string workspace condition, change-filters) and the export
//! recipient, against `private_views_native_oracle.json`
//! (`tests/test_private_views_native.py`, `tests/test_list_preferences.py`).

use std::sync::{Arc, LazyLock, Mutex};

use fsv_core::{ApiError, Result};
use serde_json::{Value, json};

use crate::list_preferences;
use crate::private_views::{self, ViewKind, ViewPlan};
use crate::reviewed_write::WritePlan;
use crate::service::testing::{Call, FnApi, MemJournal};

static ORACLE: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("testdata/private_views_native_oracle.json")).unwrap()
});

const USER: i64 = 905;
const WS: i64 = 7;

fn ticket_read(vid: i64, name: &str) -> Value {
    json!({"id": vid, "name": name, "default": false, "visibility": 3, "user_id": USER, "group_id": null,
           "group_ids": [], "advanced_query_hash": null, "advanced": false, "order_by": "created_at",
           "order_type": "desc", "per_page": 30,
           "query_hash": [{"condition": "priority", "operator": "is_in", "value": ["3"], "type": "default"},
                          {"condition": "workspace_id", "operator": "is", "value": WS.to_string(), "type": "default"}],
           "meta": {"workspace_id": [{"id": WS, "name": "Acme IT"}]}})
}

fn change_read(vid: i64, name: &str) -> Value {
    json!({"id": vid, "name": name, "default": false, "visibility": 3, "user_id": USER, "group_ids": [],
           "query_hash": null, "order_by": "created_at", "order_type": "desc",
           "advanced_query_hash": {"all": [
               {"condition": "planned_start_date", "operator": "in_the_last", "value": "one_week", "type": "default"},
               {"condition": "workspace_id", "operator": "is", "value": WS, "type": "default"}]}})
}

fn not_found() -> fsv_core::Error {
    ApiError::new(404, json!({"error": "synthetic"})).into()
}

/// The generator's `native_wire`: stores keyed by collection path.
fn native_handle(s: &mut Value, call: &Call) -> Result<Value> {
    let parts: Vec<&str> = call.path.split('/').collect();
    let coll = parts[0].to_string();
    if call.method == "GET" {
        let store = s[&coll].as_object().unwrap();
        if parts.len() == 1 {
            let mut rows: Vec<Value> = store
                .iter()
                .map(|(k, v)| {
                    json!({"id": k.parse::<i64>().unwrap(), "name": v["name"], "user_id": v["user_id"],
                           "default": v["default"], "advanced": false})
                })
                .collect();
            rows.push(json!({"id": "my_open", "name": "System", "default": true}));
            let ids: Vec<i64> = store.keys().map(|k| k.parse().unwrap()).collect();
            return Ok(json!({coll.replace('-', "_"): rows, "meta": {"editable_filter_ids": ids}}));
        }
        let env = if coll == "ticket_filters" {
            "ticket_filter"
        } else {
            "change_filter"
        };
        return match store.get(parts[1]) {
            Some(row) => Ok(json!({env: row})),
            None => Err(not_found()),
        };
    }
    if call.method == "DELETE" {
        s[&coll].as_object_mut().unwrap().shift_remove(parts[1]);
        return Ok(Value::Null);
    }
    let body = call.body.clone().unwrap();
    let name = body["name"].as_str().unwrap();
    let next = s["next"].as_i64().unwrap();
    s["next"] = json!(next + 1);
    if coll == "ticket_filters" {
        let vid = if call.method == "POST" {
            next
        } else {
            parts[1].parse().unwrap()
        };
        let mut stored = ticket_read(vid, name);
        let query: Vec<Value> = body["query_hash"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let mut c = c.clone();
                if c["condition"] == "workspace_id" {
                    c["value"] = json!(c["value"][0]["id"].to_string());
                }
                c
            })
            .collect();
        stored["query_hash"] = json!(query);
        s[&coll][vid.to_string()] = stored;
        return Ok(json!({"ticket_filter": {"id": vid}}));
    }
    let mut stored = change_read(next, name);
    stored["advanced_query_hash"] = body["advanced_query_hash"].clone();
    s[&coll][next.to_string()] = stored;
    Ok(json!({"change_filter": {"id": next}}))
}

struct Wire {
    api: FnApi,
    state: Arc<Mutex<Value>>,
    journal: MemJournal,
}

impl Wire {
    fn log(&self) -> Vec<Value> {
        self.api
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method != "ME")
            .map(|c| json!([c.method, c.path, c.body]))
            .collect()
    }

    fn sent(&self, f: impl FnOnce() -> Result<Value>) -> Value {
        let n = self.log().len();
        let mut out = attempt(f());
        out["calls"] = Value::Array(self.log().split_off(n));
        out["events"] = json!(self.journal.events());
        out
    }
}

fn native_wire() -> Wire {
    let state = Arc::new(Mutex::new(json!({
        "ticket_filters": {"801": ticket_read(801, "Acme queue")},
        "change-filters": {"811": change_read(811, "Globex changes")},
        "next": 900,
    })));
    let shared = state.clone();
    let api = FnApi::new(move |call: &Call| {
        if call.method == "ME" {
            return Ok(json!({"id": USER}));
        }
        native_handle(&mut shared.lock().unwrap(), call)
    });
    Wire {
        api,
        state,
        journal: MemJournal::default(),
    }
}

fn attempt(r: Result<Value>) -> Value {
    match r {
        Ok(v) => json!({"ok": v}),
        Err(e) => json!({"error": e.to_string(), "status": e.status()}),
    }
}

/// `got` matches the oracle entry `want`; API errors compare by status.
fn check(got: &Value, want: &Value, ctx: &str) {
    if want.get("ok").is_some() {
        assert_eq!(got.get("ok"), want.get("ok"), "{ctx}: {got}");
    } else {
        let msg = want["error"].as_str().unwrap();
        assert!(
            got.get("error").is_some(),
            "{ctx}: expected {msg:?}, got {got}"
        );
        if want["type"] == "APIError" {
            let status: u64 = msg[5..8].parse().unwrap();
            assert_eq!(got["status"], json!(status), "{ctx}: {got}");
        } else {
            assert_eq!(got["error"], json!(msg), "{ctx}");
        }
    }
    for key in ["calls", "events"] {
        if let Some(w) = want.get(key) {
            assert_eq!(got.get(key), Some(w), "{ctx}: {key}");
        }
    }
}

fn write_plan(p: &WritePlan) -> Value {
    json!({"hash": p.hash(), "review": p.review(), "action": p.action, "target": p.target,
           "method": p.method, "path": p.path, "payload_json": p.payload_json,
           "context_hash": p.context_hash, "payload": p.payload()})
}

fn view_plan(p: &ViewPlan) -> Value {
    let mut v = write_plan(&p.plan);
    v["source_id"] = json!(p.source_id);
    v
}

fn assert_plan(got: &Value, want: &Value, ctx: &str) {
    for key in got.as_object().unwrap().keys() {
        assert_eq!(got[key], want[key], "{ctx} {key}");
    }
}

fn send(w: &Wire, p: &ViewPlan) -> Value {
    w.sent(|| private_views::send(p, &w.api, &w.journal, Some(&p.hash())))
}

#[test]
fn ticket_view_clone_rename_delete_with_meta_workspace() {
    let w = native_wire();
    for entry in ORACLE["views"]["ticket_lifecycle"].as_array().unwrap() {
        let ctx = entry["action"].as_str().unwrap();
        let n = w.log().len();
        let p = private_views::build_plan(
            &w.api,
            &entry["source"],
            ctx.strip_prefix("private_view_").unwrap(),
            entry["name"].as_str(),
            &private_views::TICKETS,
        )
        .unwrap();
        assert_eq!(
            Value::Array(w.log().split_off(n)),
            entry["plan_calls"],
            "{ctx}"
        );
        assert_plan(&view_plan(&p), entry, ctx);
        check(&send(&w, &p), &entry["send"], ctx);
    }
    // `rename` is the CLI spelling of `edit`.
    let w = native_wire();
    let p = private_views::build_plan(
        &w.api,
        &json!(801),
        "rename",
        Some("Acme renamed"),
        &private_views::TICKETS,
    )
    .unwrap();
    assert_eq!(
        (p.plan.method.as_str(), p.plan.action.as_str()),
        ("PUT", "private_view_edit")
    );
}

#[test]
fn change_view_create_verified_rename_delete_not_captured() {
    let want = &ORACLE["views"]["change_create"];
    let w = native_wire();
    let p = private_views::build_plan(
        &w.api,
        &json!(811),
        "create",
        Some("Globex clone"),
        &private_views::CHANGES,
    )
    .unwrap();
    assert_eq!(Value::Array(w.log()), want["plan_calls"]);
    assert_plan(&view_plan(&p), want, "change create");
    check(&send(&w, &p), &want["send"], "change create");
    for case in ORACLE["views"]["change_refusals"].as_array().unwrap() {
        let n = w.log().len();
        let mut got = attempt(
            private_views::build_plan(
                &w.api,
                &json!(811),
                case["action"].as_str().unwrap(),
                case["name"].as_str(),
                &private_views::CHANGES,
            )
            .map(|p| view_plan(&p)),
        );
        got["calls"] = Value::Array(w.log().split_off(n));
        check(&got, case, case["action"].as_str().unwrap());
    }
}

#[test]
fn shared_foreign_and_workspace_cases_plan_and_assess() {
    for case in ORACLE["views"]["cases"].as_array().unwrap() {
        let module = case["module"].as_str().unwrap();
        let k: &ViewKind = private_views::kind(module);
        let (coll, vid) = if module == "tickets" {
            ("ticket_filters", 801)
        } else {
            ("change-filters", 811)
        };
        let w = native_wire();
        {
            let mut s = w.state.lock().unwrap();
            let row = s[coll][vid.to_string()].as_object_mut().unwrap();
            for (key, v) in case["mods"].as_object().unwrap() {
                if v.is_null() && (key == "group_ids" || key == "meta") {
                    row.shift_remove(key);
                } else {
                    row.insert(key.clone(), v.clone());
                }
            }
        }
        let ctx = format!("{module} {}", case["mods"]);
        let got = attempt(
            private_views::build_plan(&w.api, &json!(vid), "create", Some("Acme"), k)
                .map(|p| view_plan(&p)),
        );
        if let Some(ok) = case["plan"].get("ok") {
            assert_plan(&got["ok"], ok, &ctx);
        } else {
            check(&got, &case["plan"], &ctx);
        }
        assert!(w.api.writes().is_empty(), "{ctx}");
        let got = attempt(private_views::assess(&w.api, vid, k).map(Value::Object));
        check(&got, &case["assess"], &format!("assess {ctx}"));
    }
}

#[test]
fn list_and_show_reads() {
    let w = native_wire();
    for module in ["tickets", "changes"] {
        let want = &ORACLE["views"]["reads"][module];
        let k = private_views::kind(module);
        let n = w.log().len();
        check(
            &attempt(private_views::own_views(&w.api, k)),
            &want["own_views"],
            module,
        );
        let vid = if module == "tickets" { 801 } else { 811 };
        check(
            &attempt(private_views::assess(&w.api, vid, k).map(Value::Object)),
            &want["assess"],
            module,
        );
        check(
            &attempt(private_views::assess(&w.api, 999, k).map(Value::Object)),
            &want["missing"],
            module,
        );
        assert_eq!(
            Value::Array(w.log().split_off(n)),
            want["calls"],
            "{module}"
        );
    }
    for case in ORACLE["views"]["own_views_shapes"].as_array().unwrap() {
        let raw = case["raw"].clone();
        let api = FnApi::new(move |call: &Call| {
            if call.method == "ME" {
                Ok(json!({"id": USER}))
            } else {
                Ok(raw.clone())
            }
        });
        let got = attempt(private_views::own_views(&api, &private_views::TICKETS));
        check(&got, case, &case["raw"].to_string());
    }
}

#[test]
fn numeric_visibility_refusals_name_the_endpoint() {
    for case in ORACLE["views"]["private_visibility"].as_array().unwrap() {
        let mut row = ticket_read(801, "Acme queue").as_object().unwrap().clone();
        let mods = case["mods"].as_object().unwrap();
        row.extend(mods.clone());
        if mods.get("group_ids") == Some(&Value::Null) {
            row.shift_remove("group_ids");
        }
        let got = attempt(
            private_views::private_visibility(&row, USER, &private_views::TICKETS)
                .map(|v| json!(v)),
        );
        check(&got, case, &case["mods"].to_string());
        if let Some(msg) = got["error"].as_str() {
            assert!(!msg.contains('\n'));
        }
    }
}

fn list_api(me: Value) -> FnApi {
    let headers: Vec<Value> = [
        ("ticket_subject", "Acme subject"),
        ("requester", "Acme requester"),
        ("status", "Acme state"),
        ("priority", "Acme priority"),
        ("display_id", "Acme ID"),
        ("assigned_to", "Acme agent"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (key, label))| {
        json!({"name": key, "label": label, "position": i + 1, "selected": true,
               "frozen": *key == "display_id"})
    })
    .collect();
    let row = json!({"id": 701, "name": "Acme queue",
                     "query_hash": [{"condition": "status", "operator": "is_in", "value": ["47"], "type": "default"}],
                     "advanced_query_hash": null});
    FnApi::new(move |call: &Call| {
        if call.method == "ME" {
            return match &me {
                Value::String(s) if s == "403" => {
                    Err(ApiError::new(403, json!({"error": "hidden"})).into())
                }
                other => Ok(other.clone()),
            };
        }
        let path = call.path.as_str();
        if path.ends_with("/headers") {
            return Ok(json!({"headers": headers}));
        }
        if path.ends_with("/701") {
            let module = path.split('/').next().unwrap();
            let env = if module == "ticket_filters" {
                "ticket_filter".to_string()
            } else {
                format!("{}_filter", module.trim_end_matches("-filters"))
            };
            return Ok(json!({env: row}));
        }
        if path.ends_with("filters") {
            return Ok(json!({"filters": [row]}));
        }
        Err(not_found())
    })
}

#[test]
fn export_review_shows_the_recipient_mailbox() {
    for case in ORACLE["recipient"].as_array().unwrap() {
        let module = case["module"].as_str().unwrap();
        let ctx = format!("{module} {}", case["me"]);
        let api = list_api(case["me"].clone());
        let p =
            list_preferences::export_plan(&api, module, "701", &["subject".to_string()]).unwrap();
        assert_eq!(Value::Object(p.review()), case["review"], "{ctx}");
        assert_eq!(p.hash(), case["hash"], "{ctx}");
        assert_eq!(p.recipient_json, case["recipient_json"], "{ctx}");
        // The recipient read comes after the view read.
        let calls = api.calls.lock().unwrap();
        assert_eq!(calls.last().unwrap().method, "ME", "{ctx}");
    }
}
