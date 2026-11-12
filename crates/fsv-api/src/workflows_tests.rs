//! The reviewed workflows (canned, scenarios, private_views,
//! list_preferences, problem_workflows) against
//! `port/golden/gen_fsv_api_workflows.py`: each wire mirrors the generator's
//! stateful handler, and request logs, plans, results, error texts and
//! journal events are compared.

use std::sync::{Arc, LazyLock, Mutex};

use fsv_core::{ApiError, Error, Result};
use serde_json::{Map, Value, json};

use crate::conversations::{self, ConversationArgs};
use crate::formmeta::MemStore;
use crate::reviewed_write::WritePlan;
use crate::service::testing::{Call, FnApi, MemJournal};
use crate::{canned, list_preferences, private_views, problem_workflows, readback, scenarios};

static ORACLE: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("testdata/workflows_oracle.json")).unwrap());

/// A handler reply: JSON, an HTTP status (`{"error": "synthetic"}` body),
/// an HTML error page, or a dropped connection.
enum R {
    J(Value),
    S(u16),
    Html(u16),
    Drop,
}

type Handler = fn(&mut Value, &str, &str, Option<&Value>) -> R;

struct Wire {
    api: FnApi,
    state: Arc<Mutex<Value>>,
    journal: MemJournal,
    store: MemStore,
}

fn wire(state: Value, handle: Handler) -> Wire {
    let state = Arc::new(Mutex::new(state));
    let shared = state.clone();
    let api = FnApi::new(move |call: &Call| {
        if call.method == "ME" {
            return Ok(json!({"id": 901}));
        }
        let mut s = shared.lock().unwrap();
        match handle(&mut s, &call.method, &call.path, call.body.as_ref()) {
            R::J(v) => Ok(v),
            R::S(204) => Ok(Value::Null),
            R::S(n) => Err(ApiError::new(n, json!({"error": "synthetic"})).into()),
            R::Html(n) => Err(ApiError::new(n, json!("<html>Not found</html>")).into()),
            R::Drop => Err(Error::Http {
                kind: "ReadError".into(),
                message: "synthetic disconnect".into(),
            }),
        }
    });
    Wire {
        api,
        state,
        journal: MemJournal::default(),
        store: MemStore {
            workspace: json!(17),
            ..MemStore::default()
        },
    }
}

impl Wire {
    fn set(&self, f: impl FnOnce(&mut Value)) {
        f(&mut self.state.lock().unwrap());
    }

    fn log(&self) -> Vec<Value> {
        self.api
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method != "ME")
            .map(|c| {
                let mut path = c.path.clone();
                if c.method != "GET"
                    && let Some(q) = c.params.as_ref().filter(|q| !q.is_empty())
                {
                    let pairs: Vec<String> = q
                        .iter()
                        .map(|(k, v)| format!("{k}={}", fsv_py::py_str(v)))
                        .collect();
                    path = format!("{path}?{}", pairs.join("&"));
                }
                json!([c.method, path, c.body])
            })
            .collect()
    }

    /// The result or error of `f`, the requests it made and the journal.
    fn sent(&self, f: impl FnOnce() -> Result<Value>) -> Value {
        let n = self.log().len();
        let mut out = attempt(f());
        out["calls"] = Value::Array(self.log().split_off(n));
        out["events"] = json!(self.journal.events());
        out
    }
}

fn attempt(r: Result<Value>) -> Value {
    match r {
        Ok(v) => json!({"ok": v}),
        Err(e) => json!({"error": e.to_string(), "status": e.status()}),
    }
}

fn attempt_map(r: Result<Map<String, Value>>) -> Value {
    attempt(r.map(Value::Object))
}

/// `got` (from `attempt`/`sent`) matches the oracle entry `want`. API
/// error texts carry Python's request context, so those compare by status.
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
           "context_hash": p.context_hash})
}

fn with(mut v: Value, key: &str, extra: Value) -> Value {
    v[key] = extra;
    v
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn failure(s: &Value) -> Option<R> {
    match &s["failure"] {
        Value::Null => None,
        Value::String(_) => Some(R::Drop),
        n => Some(R::S(n.as_u64().unwrap() as u16)),
    }
}

// ---------------------------------------------------------------- canned

fn canned_handle(s: &mut Value, method: &str, path: &str, body: Option<&Value>) -> R {
    match (method, path) {
        (_, "canned_responses/701") => R::J(s["source"].clone()),
        (_, "canned_response_folders") => R::J(json!({"canned_response_folders": [{"id": 702}]})),
        (_, "canned_response_folders/702/canned_responses") => {
            R::J(json!({"canned_responses": [{"id": 701}]}))
        }
        (_, "scenario_automations") => R::J(
            s.get("scenarios")
                .cloned()
                .unwrap_or(json!({"scenario_automations": [{"id": 703, "name": "Globex"}]})),
        ),
        ("GET", "tickets/123") => {
            R::J(json!({"ticket": {"id": 123, "subject": "Acme", "status": 42}}))
        }
        ("PUT", "tickets/execute_scenario") => {
            s["writes"]
                .as_array_mut()
                .unwrap()
                .push(body.cloned().unwrap());
            failure(s).unwrap_or(R::S(204))
        }
        ("POST", "tickets/123/reply") => {
            s["writes"]
                .as_array_mut()
                .unwrap()
                .push(body.cloned().unwrap());
            failure(s).unwrap_or(R::J(json!({"conversation": {"id": 704}})))
        }
        (_, "tickets/123/conversations") => {
            let mut row = json!({"id": 704});
            row.as_object_mut()
                .unwrap()
                .extend(s["writes"][0].as_object().unwrap().clone());
            R::J(json!({"conversations": [row], "has_more": false}))
        }
        _ => R::S(404),
    }
}

fn canned_wire() -> Wire {
    wire(
        json!({"source": {"canned_response": {"id": 701, "content": "<div>Acme response</div>"}},
               "failure": null, "writes": []}),
        canned_handle,
    )
}

fn reply(
    w: &Wire,
    target: &str,
    id: Value,
    pointer: &str,
    sender: &str,
) -> Result<conversations::ConversationPlan> {
    canned::reply_plan(&w.api, &w.store, target, &id, pointer, sender)
}

fn conv_send(w: &Wire, p: &conversations::ConversationPlan, confirmed: bool) -> Result<Value> {
    conversations::send(p, &w.api, &w.journal, &w.store, Some(&p.hash()), confirmed)
}

#[test]
fn canned_reads_and_content_picker() {
    let want = &ORACLE["canned"];
    let w = canned_wire();
    let reads = &want["reads"];
    assert_eq!(canned::folders(&w.api).unwrap(), reads["folders"]);
    assert_eq!(
        canned::responses(&w.api, &json!(702)).unwrap(),
        reads["responses"]
    );
    assert_eq!(canned::show(&w.api, &json!(701)).unwrap(), reads["show"]);
    assert_eq!(canned::scenarios(&w.api).unwrap(), reads["scenarios"]);
    assert_eq!(
        Value::Object(canned::scenario(&w.api, &json!(703)).unwrap()),
        reads["scenario"]
    );
    let got = attempt_map(canned::scenario(&w.api, &json!(704)));
    check(&got, &reads["scenario_missing"], "scenario_missing");
    assert_eq!(Value::Array(w.log()), reads["calls"]);
    for (key, rows) in [
        ("scenario_not_list", json!({"rows": []})),
        ("scenario_ambiguous", json!([{"id": 703}, {"id": "703"}])),
        (
            "scenario_bare_list",
            json!([{"id": 703, "name": "bare list"}]),
        ),
    ] {
        w.set(|s| s["scenarios"] = rows);
        check(
            &attempt_map(canned::scenario(&w.api, &json!(703))),
            &reads[key],
            key,
        );
    }
    for case in want["content"].as_array().unwrap() {
        let pointer = case["pointer"].as_str().unwrap();
        let got = attempt(canned::content(&case["raw"], pointer).map(Value::String));
        check(&got, case, &format!("content {} {pointer}", case["raw"]));
    }
}

#[test]
fn canned_reply_plan_send_and_refusals() {
    let want = &ORACLE["canned"];
    let r = &want["reply"];
    let w = canned_wire();
    let p = reply(
        &w,
        "INC-123",
        json!(701),
        "/canned_response/content",
        "support@example.com",
    )
    .unwrap();
    assert_eq!(Value::Array(w.log()), r["plan_calls"]);
    assert_eq!(p.hash(), r["hash"]);
    assert_eq!(p.payload_json, r["payload_json"]);
    assert_eq!(Value::Object(p.review()), r["review"]);
    assert_eq!(json!(p.canned_hash), r["canned_hash"]);
    check(
        &attempt(conv_send(&w, &p, false)),
        &r["unconfirmed"],
        "unconfirmed",
    );
    check(&w.sent(|| conv_send(&w, &p, true)), &r["send"], "send");
    check(&w.sent(|| conv_send(&w, &p, true)), &r["resend"], "resend");
    let legacy = conversations::build_plan(
        &w.api,
        &w.store,
        "123",
        "Acme",
        &ConversationArgs::default(),
    )
    .unwrap();
    assert_eq!(legacy.hash(), want["legacy_hash"]);

    let w = canned_wire();
    let p = reply(
        &w,
        "123",
        json!(701),
        "/canned_response/content",
        "support@example.com",
    )
    .unwrap();
    w.set(|s| s["source"]["canned_response"]["name"] = json!("Changed metadata"));
    check(&w.sent(|| conv_send(&w, &p, true)), &want["stale"], "stale");
    let refusals = &want["reply_refusals"];
    for (key, id, pointer, sender) in [
        (
            "no_sender",
            json!(701),
            "/canned_response/content",
            "Help <x@example.com>",
        ),
        (
            "bad_pointer",
            json!(701),
            "/canned_response/body",
            "support@example.com",
        ),
        (
            "bad_id",
            json!(0),
            "/canned_response/content",
            "support@example.com",
        ),
    ] {
        let got = attempt(reply(&w, "123", id, pointer, sender).map(|p| json!(p.hash())));
        check(&got, &refusals[key], key);
    }

    for (key, failure) in [
        ("reply_fail_500", json!(500)),
        ("reply_fail_422", json!(422)),
        ("reply_fail_disconnect", json!("disconnect")),
    ] {
        let w = canned_wire();
        let p = reply(
            &w,
            "123",
            json!(701),
            "/canned_response/content",
            "support@example.com",
        )
        .unwrap();
        w.set(|s| s["failure"] = failure);
        check(&w.sent(|| conv_send(&w, &p, true)), &want[key], key);
    }
}

// ------------------------------------------------------------- scenarios

#[test]
fn scenarios_plan_send_failures_and_refusals() {
    let want = &ORACLE["scenarios"];
    let w = canned_wire();
    let p = scenarios::build_plan(&w.api, "INC-123", &json!(703)).unwrap();
    let plan = with(write_plan(&p), "calls", Value::Array(w.log()));
    assert_eq!(plan, want["plan"]);
    assert_eq!(
        scenarios::send(&p, &w.api, &w.journal, None).unwrap(),
        want["cancel"]
    );
    let send = || scenarios::send(&p, &w.api, &w.journal, Some(&p.hash()));
    check(&w.sent(send), &want["send"], "send");
    check(&w.sent(send), &want["resend"], "resend");
    for (key, failure) in [
        ("fail_500", json!(500)),
        ("fail_422", json!(422)),
        ("fail_disconnect", json!("disconnect")),
    ] {
        let w = canned_wire();
        let p = scenarios::build_plan(&w.api, "123", &json!(703)).unwrap();
        w.set(|s| s["failure"] = failure);
        let got = w.sent(|| scenarios::send(&p, &w.api, &w.journal, Some(&p.hash())));
        check(&got, &want[key], key);
    }
    let w = canned_wire();
    for (key, target, id) in [
        ("missing_scenario", "123", json!(9)),
        ("bad_ticket", "CHN-1", json!(703)),
        ("zero_scenario", "123", json!(0)),
    ] {
        let got = attempt(scenarios::build_plan(&w.api, target, &id).map(|p| write_plan(&p)));
        check(&got, &want["refusals"][key], key);
    }
}

// --------------------------------------------------------- private views

fn view() -> Value {
    json!({"id": 701, "user_id": 901, "default": false, "visibility": 81, "visibility_name": "Me",
           "name": "Acme source", "workspace_id": 17,
           "query_hash": [{"condition": "status", "operator": "is_in", "type": "default", "value": ["42"]}],
           "advanced_query_hash": null, "order_by": "created_at", "order_type": "desc"})
}

fn view_handle(s: &mut Value, method: &str, path: &str, body: Option<&Value>) -> R {
    let leaf = path
        .split('?')
        .next()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    if method == "GET" {
        if leaf == "ticket_filters" {
            let rows: Vec<Value> = s["rows"].as_object().unwrap().values().cloned().collect();
            return R::J(json!({"ticket_filters": rows, "has_more": s["partial"]}));
        }
        return match s["rows"].get(&leaf) {
            Some(row) => R::J(json!({"ticket_filter": row})),
            None => R::Html(404),
        };
    }
    if let Some(r) = failure(s) {
        return r;
    }
    if method == "DELETE" {
        s["rows"].as_object_mut().unwrap().shift_remove(&leaf);
        return R::S(204);
    }
    let vid: i64 = if method == "POST" {
        702
    } else {
        leaf.parse().unwrap()
    };
    let mut row = s["original"].as_object().unwrap().clone();
    row.extend(body.unwrap().as_object().unwrap().clone());
    row.insert("id".into(), json!(vid));
    s["rows"][vid.to_string()] = Value::Object(row);
    R::J(json!({"ticket_filter": {"id": vid}}))
}

fn view_wire(source: Value) -> Wire {
    let id = source["id"].to_string();
    wire(
        json!({"original": source.clone(), "rows": {id: source}, "failure": null, "partial": false}),
        view_handle,
    )
}

fn view_plan(p: &private_views::ViewPlan) -> Value {
    with(write_plan(&p.plan), "source_id", json!(p.source_id))
}

fn view_send(w: &Wire, p: &private_views::ViewPlan) -> Value {
    w.sent(|| private_views::send(p, &w.api, &w.journal, Some(&p.hash())))
}

#[test]
fn private_views_lifecycle_and_refusals() {
    let want = &ORACLE["private_views"];
    let w = view_wire(view());
    for entry in want["lifecycle"].as_array().unwrap() {
        let n = w.log().len();
        let ctx = entry["action"].as_str().unwrap();
        let action = ctx.strip_prefix("private_view_").unwrap();
        let p = private_views::build_plan(&w.api, &entry["source"], action, entry["name"].as_str())
            .unwrap();
        assert_eq!(
            Value::Array(w.log().split_off(n)),
            entry["plan_calls"],
            "{ctx}"
        );
        let plan = view_plan(&p);
        for key in plan.as_object().unwrap().keys() {
            assert_eq!(plan[key], entry[key], "{ctx} {key}");
        }
        check(&view_send(&w, &p), &entry["send"], ctx);
    }

    let w = view_wire(view());
    let p = private_views::build_plan(&w.api, &json!(701), "delete", None).unwrap();
    w.set(|s| s["partial"] = json!(true));
    check(
        &view_send(&w, &p),
        &want["delete_partial"],
        "delete_partial",
    );

    let w = view_wire(view());
    let p = private_views::build_plan(&w.api, &json!(701), "create", Some("Globex")).unwrap();
    w.set(|s| s["failure"] = json!(500));
    check(
        &view_send(&w, &p),
        &want["create_fail_500"],
        "create_fail_500",
    );

    for case in want["refusals"].as_array().unwrap() {
        let mods = case["mods"].as_object().unwrap();
        let mut source = view();
        if !mods.contains_key("id") {
            source.as_object_mut().unwrap().extend(mods.clone());
        }
        let w = view_wire(source);
        if let Some(id) = mods.get("id") {
            w.set(|s| s["rows"]["701"]["id"] = id.clone());
        }
        let got = attempt(
            private_views::build_plan(
                &w.api,
                &json!(701),
                case["action"].as_str().unwrap(),
                case["name"].as_str(),
            )
            .map(|p| view_plan(&p)),
        );
        check(&got, case, &format!("refusal {mods:?} {}", case["action"]));
    }
}

// ------------------------------------------------------ list preferences

fn headers() -> Value {
    let rows: Vec<Value> = [
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
    Value::Array(rows)
}

fn filters_path(module: &str) -> &'static str {
    match module {
        "tickets" => "ticket_filters",
        "changes" => "change-filters",
        _ => "problem-filters",
    }
}

fn list_handle(s: &mut Value, method: &str, path: &str, body: Option<&Value>) -> R {
    let module = s["module"].as_str().unwrap().to_string();
    let filters = filters_path(&module);
    if method == "GET" {
        if path == format!("{module}/headers") {
            let mut rows = s["headers"].clone();
            if s["writes"] != 0 && s["readback"] == "missing" {
                rows[0].as_object_mut().unwrap().shift_remove("frozen");
            }
            return R::J(json!({"headers": rows}));
        }
        if path == filters {
            return R::J(json!({"filters": [s["row"]]}));
        }
        if path == format!("{filters}/701") {
            return R::J(json!({format!("{}_filter", &module[..module.len() - 1]): s["row"]}));
        }
        return R::S(404);
    }
    s["writes"] = json!(s["writes"].as_i64().unwrap() + 1);
    if let Some(r) = failure(s) {
        return r;
    }
    if path.ends_with("/headers") {
        let body = body.unwrap();
        for header in s["headers"].as_array_mut().unwrap() {
            let name = header["name"].as_str().unwrap().to_string();
            header["selected"] = json!(body["columns"].get(&name).is_some());
            header["position"] = body["columns"].get(&name).cloned().unwrap_or(Value::Null);
            header["frozen"] = json!(
                body["frozen_columns"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(name))
            );
        }
    }
    R::S(204)
}

fn list_wire(module: &str) -> Wire {
    wire(
        json!({"module": module, "headers": headers(),
               "row": {"id": 701, "name": "Acme queue",
                       "query_hash": [{"condition": "status", "operator": "is_in", "value": ["47"], "type": "default"}],
                       "advanced_query_hash": null},
               "failure": null, "readback": "ok", "writes": 0}),
        list_handle,
    )
}

fn list_plan(p: &list_preferences::ListPlan) -> Value {
    with(write_plan(&p.plan), "inputs_json", json!(p.inputs_json))
}

fn list_send(w: &Wire, p: &list_preferences::ListPlan) -> Value {
    w.sent(|| list_preferences::send(p, &w.api, &w.journal, Some(&p.hash())))
}

fn owned(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn assert_plan(got: &Value, want: &Value, ctx: &str) {
    for key in got.as_object().unwrap().keys() {
        assert_eq!(got[key], want[key], "{ctx} {key}");
    }
}

#[test]
fn list_preferences_columns_export_and_refusals() {
    for module in ["tickets", "changes", "problems"] {
        let want = &ORACLE["list_preferences"][module];
        let w = list_wire(module);
        let cols = owned(&["ticket_subject", "requester", "status"]);
        let p = list_preferences::columns_plan(&w.api, module, &cols, &[]).unwrap();
        assert_eq!(
            Value::Array(w.log()),
            want["columns"]["plan_calls"],
            "{module}"
        );
        assert_plan(&list_plan(&p), &want["columns"], module);
        check(&list_send(&w, &p), &want["columns"]["send"], module);
        check(&list_send(&w, &p), &want["columns"]["resend"], module);

        let w = list_wire(module);
        let fields = owned(&["subject", "status_name", "display_id"]);
        let p = list_preferences::export_plan(&w.api, module, "701", &fields).unwrap();
        assert_eq!(
            Value::Array(w.log()),
            want["export"]["plan_calls"],
            "{module}"
        );
        assert_plan(&list_plan(&p), &want["export"], module);
        check(&list_send(&w, &p), &want["export"]["send"], module);

        let w = list_wire(module);
        let p = list_preferences::columns_plan(
            &w.api,
            module,
            &owned(&["ticket_subject", "status"]),
            &[],
        )
        .unwrap();
        w.set(|s| s["readback"] = json!("missing"));
        check(&list_send(&w, &p), &want["columns_missing"], module);

        let w = list_wire(module);
        let p = list_preferences::columns_plan(&w.api, module, &owned(&["ticket_subject"]), &[])
            .unwrap();
        w.set(|s| s["headers"][0]["label"] = json!("Drifted"));
        check(&list_send(&w, &p), &want["columns_drift"], module);

        let w = list_wire(module);
        let p = list_preferences::export_plan(&w.api, module, "701", &owned(&["subject"])).unwrap();
        w.set(|s| s["failure"] = json!(429));
        check(&list_send(&w, &p), &want["export_fail_429"], module);

        for case in want["refusals"].as_array().unwrap() {
            let w = list_wire(module);
            let args = &case["args"];
            let ctx = format!("{module} {} {args}", case["kind"]);
            let got = match case["kind"].as_str().unwrap() {
                "columns" => list_preferences::columns_plan(
                    &w.api,
                    module,
                    &strings(&args[0]),
                    &strings(&args[1]),
                ),
                "export" => list_preferences::export_plan(
                    &w.api,
                    module,
                    args[0].as_str().unwrap(),
                    &strings(&args[1]),
                ),
                _ => {
                    w.set(|s| match args[0].as_str().unwrap() {
                        "advanced" => s["row"]["advanced_query_hash"] = json!([]),
                        "query_not_list" => s["row"]["query_hash"] = json!("x"),
                        "id_mismatch" => s["row"]["id"] = json!(702),
                        "headers_empty" => s["headers"] = json!([]),
                        "headers_dup" => s["headers"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!({"name": "status"})),
                        _ => s["headers"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!({"label": "x"})),
                    });
                    list_preferences::export_plan(&w.api, module, "701", &owned(&["display_id"]))
                }
            };
            let got = attempt(got.map(|p| list_plan(&p)));
            check(&got, case, &ctx);
        }
    }
    let w = wire(json!({}), |_, _, _, _| R::S(404));
    let got = attempt(list_preferences::headers(&w.api, "assets").map(|h| json!(h)));
    check(
        &got,
        &ORACLE["list_preferences"]["unsupported"],
        "unsupported",
    );
}

// ----------------------------------------------------- problem workflows

fn problem_handle(s: &mut Value, method: &str, path: &str, body: Option<&Value>) -> R {
    if method == "GET" {
        match path {
            "problem_form_fields" => return R::J(json!({"problem_fields": s["fields"]})),
            "problem_task_fields" => return R::J(json!({"problem_task_fields": s["task_fields"]})),
            "autocomplete/requesters" => {
                return R::J(
                    json!([{"id": 901, "value": "Acme Operator", "email": "person@example.com"}]),
                );
            }
            "problems/123/tasks" | "problems/123/notes" => {
                let key = path.rsplit('/').next().unwrap();
                let mut rows = s[key].clone();
                if let Some(first) = rows.get_mut(0).filter(|_| s["readback"] == "missing") {
                    first
                        .as_object_mut()
                        .unwrap()
                        .shift_remove(if key == "notes" { "body" } else { "title" });
                }
                return R::J(json!({key: rows, "has_more": false}));
            }
            _ => {}
        }
        if path.ends_with("business_rules") {
            return if s["rules_denied"] == true {
                R::S(403)
            } else {
                R::J(json!({"business_rules": []}))
            };
        }
        if let Some(rest) = path.strip_prefix("problems/") {
            let id = rest.split('/').next().unwrap();
            let Some(mut row) = s["rows"].get(id).cloned() else {
                return R::S(404);
            };
            if s["writes"] != 0 && !s["normalize"].is_null() {
                row["custom_fields"] = s["normalize"]["custom_fields"].clone();
                row["description"] = json!("Acme details");
            }
            if s["writes"] != 0 && s["readback"] == "missing" {
                row.as_object_mut().unwrap().shift_remove("email");
            }
            return R::J(json!({"problem": row}));
        }
        return R::S(404);
    }
    s["writes"] = json!(s["writes"].as_i64().unwrap() + 1);
    if let Some(r) = failure(s) {
        return r;
    }
    let mut row = json!({"id": 456});
    row.as_object_mut()
        .unwrap()
        .extend(body.unwrap().as_object().unwrap().clone());
    let key = if path == "problems" {
        s["rows"]["456"] = row;
        "problem"
    } else {
        let key = if path.ends_with("/notes") {
            "note"
        } else {
            "task"
        };
        s[format!("{key}s")].as_array_mut().unwrap().push(row);
        key
    };
    R::J(if s["omit_id"] == true {
        json!({})
    } else {
        json!({key: {"id": 456}})
    })
}

fn problem_wire() -> Wire {
    wire(
        json!({"fields": ORACLE["problem_workflows"]["fields"], "rows": {"123": {"id": 123, "subject": "Acme"}},
               "notes": [], "tasks": [], "failure": null, "rules_denied": false, "readback": "ok",
               "omit_id": false, "normalize": null, "writes": 0,
               "task_fields": [{"name": "status", "choices": [{"id": 51, "value": "Open"}]},
                               {"name": "notify_before", "choices": [{"id": 52, "value": "Never"}]}]}),
        problem_handle,
    )
}

fn answers() -> Map<String, Value> {
    ORACLE["problem_workflows"]["answers"]
        .as_object()
        .unwrap()
        .clone()
}

fn problem_plan(p: &problem_workflows::ProblemPlan) -> Value {
    with(write_plan(&p.plan), "inputs_json", json!(p.inputs_json))
}

fn create(
    w: &Wire,
    answers: &Map<String, Value>,
    who: &str,
) -> Result<problem_workflows::ProblemPlan> {
    problem_workflows::create_plan(&w.api, &w.store, answers, who, None)
}

fn problem_send(w: &Wire, p: &problem_workflows::ProblemPlan) -> Value {
    w.sent(|| problem_workflows::send(p, &w.api, &w.journal, &w.store, Some(&p.hash()), None))
}

#[test]
fn problem_create_send_and_custom_readback() {
    let want = &ORACLE["problem_workflows"];
    let w = problem_wire();
    let p = create(&w, &answers(), "person@example.com").unwrap();
    assert_eq!(Value::Array(w.log()), want["create"]["plan_calls"]);
    assert_plan(&problem_plan(&p), &want["create"], "create");
    check(&problem_send(&w, &p), &want["create"]["send"], "send");
    check(&problem_send(&w, &p), &want["create"]["resend"], "resend");

    for case in want["create_custom"].as_array().unwrap() {
        let w = problem_wire();
        w.set(|s| {
            s["fields"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": 601, "name": "acme_count",
                "label": "Acme count", "field_type": "custom_number", "position": 20}));
        });
        let mut a = answers();
        a.insert("Acme count".into(), json!(7));
        let p = create(&w, &a, "person@example.com").unwrap();
        let ctx = format!("custom {}", case["actual"]);
        assert_eq!(p.hash(), case["hash"], "{ctx}");
        assert_eq!(p.plan.payload_json, case["payload_json"], "{ctx}");
        assert_eq!(p.inputs_json, case["inputs_json"], "{ctx}");
        w.set(|s| s["normalize"] = json!({"custom_fields": case["actual"]}));
        check(&problem_send(&w, &p), case, &ctx);
    }

    for readback in ["missing", "no-id"] {
        let w = problem_wire();
        let p = create(&w, &answers(), "person@example.com").unwrap();
        w.set(|s| {
            s["readback"] = json!(readback);
            s["omit_id"] = json!(readback == "no-id");
        });
        check(&problem_send(&w, &p), &want["create"][readback], readback);
    }

    let w = problem_wire();
    let p = create(&w, &answers(), "person@example.com").unwrap();
    w.set(|s| s["fields"][0]["label"] = json!("Changed subject"));
    check(&problem_send(&w, &p), &want["create_drift"], "drift");

    let w = problem_wire();
    let p = create(&w, &answers(), "person@example.com").unwrap();
    w.set(|s| s["failure"] = json!(500));
    check(&problem_send(&w, &p), &want["create_fail_500"], "fail_500");
}

#[test]
fn problem_note_and_task() {
    let want = &ORACLE["problem_workflows"];
    for kind in ["note", "task"] {
        let w = problem_wire();
        let p = problem_workflows::child_plan(&w.api, "PRB-123", kind, "Acme").unwrap();
        assert_eq!(Value::Array(w.log()), want[kind]["plan_calls"], "{kind}");
        assert_plan(&problem_plan(&p), &want[kind], kind);
        check(&problem_send(&w, &p), &want[kind]["send"], kind);
        for readback in ["missing", "no-id"] {
            let w = problem_wire();
            let p = problem_workflows::child_plan(&w.api, "123", kind, "Acme").unwrap();
            w.set(|s| {
                s["readback"] = json!(readback);
                s["omit_id"] = json!(readback == "no-id");
            });
            check(
                &problem_send(&w, &p),
                &want[kind][readback],
                &format!("{kind} {readback}"),
            );
        }
    }
}

#[test]
fn problem_refusals() {
    let want = &ORACLE["problem_workflows"]["refusals"];
    let with_answer = |k: &str, v: &str| {
        let mut a = answers();
        a.insert(k.into(), json!(v));
        a
    };
    type Mutate = Box<dyn Fn(&mut Value)>;
    type Build = Box<dyn Fn(&Wire) -> Result<problem_workflows::ProblemPlan>>;
    let none: fn() -> Mutate = || Box::new(|_| {});
    let task_field = |f: fn(&mut Value)| -> Mutate { Box::new(move |s| f(&mut s["task_fields"])) };
    let cases: Vec<(&str, Mutate, Build)> = vec![
        (
            "rules_denied",
            Box::new(|s| s["rules_denied"] = json!(true)),
            Box::new(|w| create(w, &answers(), "person@example.com")),
        ),
        (
            "requester_answer",
            none(),
            Box::new(move |w| create(w, &with_answer("Requester", "x"), "person@example.com")),
        ),
        (
            "unknown_answer",
            none(),
            Box::new(move |w| create(w, &with_answer("Nope", "x"), "person@example.com")),
        ),
        (
            "no_requester_field",
            Box::new(|s| {
                s["fields"].as_array_mut().unwrap().remove(2);
            }),
            Box::new(|w| create(w, &answers(), "person@example.com")),
        ),
        (
            "bad_choice",
            none(),
            Box::new(move |w| create(w, &with_answer("Status", "Closed"), "person@example.com")),
        ),
        (
            "missing_impact",
            none(),
            Box::new(|w| {
                let mut a = answers();
                a.shift_remove("Impact");
                create(w, &a, "person@example.com")
            }),
        ),
        (
            "extra_top",
            Box::new(|s| {
                s["fields"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id": 520, "name": "urgency",
            "label": "Urgency", "field_type": "default_urgency", "position": 9,
            "choices": [{"id": 9, "value": "Low"}]}))
            }),
            Box::new(move |w| create(w, &with_answer("Urgency", "Low"), "person@example.com")),
        ),
        (
            "no_requester_match",
            none(),
            Box::new(|w| create(w, &answers(), "nobody@example.com")),
        ),
        (
            "child_kind",
            none(),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "comment", "Acme")),
        ),
        (
            "child_text",
            none(),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "note", "  ")),
        ),
        (
            "child_target",
            none(),
            Box::new(|w| problem_workflows::child_plan(&w.api, "CHN-1", "note", "Acme")),
        ),
        (
            "child_missing_parent",
            Box::new(|s| s["rows"] = json!({})),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "note", "Acme")),
        ),
        (
            "task_no_meta",
            Box::new(|s| s["task_fields"] = json!("x")),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "task", "Acme")),
        ),
        (
            "task_no_status",
            task_field(|t| {
                t.as_array_mut().unwrap().remove(0);
            }),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "task", "Acme")),
        ),
        (
            "task_deleted_choice",
            task_field(|t| t[0]["choices"][0]["deleted"] = json!(true)),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "task", "Acme")),
        ),
        (
            "task_str_id",
            task_field(|t| t[0]["choices"][0]["id"] = json!("51")),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "task", "Acme")),
        ),
        (
            "task_required",
            task_field(|t| {
                t.as_array_mut()
                    .unwrap()
                    .push(json!({"name": "agent", "required": true}))
            }),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "task", "Acme")),
        ),
        (
            "task_label_match",
            task_field(|t| {
                t[0]["choices"][0]["value"] = Value::Null;
                t[0]["choices"][0]["label"] = json!("OPEN");
            }),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "task", "Acme")),
        ),
        (
            "task_dup_rows",
            Box::new(|s| s["tasks"] = json!([{"id": 5}, {"id": 5}])),
            Box::new(|w| problem_workflows::child_plan(&w.api, "123", "task", "Acme")),
        ),
    ];
    let wanted: Vec<&Value> = want.as_array().unwrap().iter().collect();
    assert_eq!(cases.len(), wanted.len());
    for ((name, mutate, build), case) in cases.into_iter().zip(wanted) {
        assert_eq!(case["name"], name);
        let w = problem_wire();
        w.set(|s| mutate(s));
        let got = attempt(build(&w).map(|p| problem_plan(&p)));
        check(&got, case, name);
    }
}

// --------------------------------------------------------------- readback

#[test]
fn readback_submitted_and_same_text() {
    for case in ORACLE["readback"]["submitted"].as_array().unwrap() {
        assert_eq!(
            readback::submitted_value_matches(&case["actual"], &case["expected"]),
            case["submitted"] == true,
            "{case}"
        );
    }
    for case in ORACLE["readback"]["same_text"].as_array().unwrap() {
        assert_eq!(
            readback::same_text(&case["actual"], &case["expected"]),
            case["same"] == true,
            "{case}"
        );
    }
}

// ------------------------------------------------- helpers called directly

/// The reads and checks the workflows call internally, called on their own
/// (the generator's `gen_direct`).
#[test]
fn direct_reads_match_python() {
    let want = &ORACLE["direct"];

    let w = canned_wire();
    let p = reply(
        &w,
        "123",
        json!(701),
        "/canned_response/content",
        "support@example.com",
    )
    .unwrap();
    let n = w.log().len();
    let fresh = &want["canned_freshen"];
    let freshen = |w: &Wire| attempt(canned::freshen(&p, &w.api).map(|()| Value::Null));
    check(&freshen(&w), &fresh["same"], "freshen same");
    w.set(|s| s["source"]["canned_response"]["name"] = json!("Changed metadata"));
    check(&freshen(&w), &fresh["metadata"], "freshen metadata");
    w.set(|s| {
        s["source"] =
            json!({"canned_response": {"id": 701, "content": "<div>Globex response</div>"}})
    });
    check(&freshen(&w), &fresh["content"], "freshen content");
    assert_eq!(json!(w.log().split_off(n)), fresh["calls"]);

    let w = canned_wire();
    let ctx = &want["scenarios_context"];
    for (key, ticket, scenario) in [
        ("ok", 123, 703),
        ("missing_scenario", 123, 9),
        ("missing_ticket", 999, 703),
    ] {
        check(
            &attempt(scenarios::context(&w.api, ticket, scenario)),
            &ctx[key],
            key,
        );
    }
    assert_eq!(json!(w.log()), ctx["calls"]);

    let w = view_wire(view());
    let detail = &want["private_detail"];
    let read = |w: &Wire, id| attempt_map(private_views::detail(&w.api, id));
    check(&read(&w, 701), &detail["ok"], "detail");
    check(&read(&w, 9), &detail["missing"], "detail missing");
    w.set(|s| s["rows"]["701"]["id"] = json!(702));
    check(&read(&w, 701), &detail["other_id"], "detail other id");
    w.set(|s| s["rows"]["701"]["id"] = json!("x"));
    check(&read(&w, 701), &detail["bad_id"], "detail bad id");
    assert_eq!(json!(w.log()), detail["calls"]);
    for case in want["private_visibility"].as_array().unwrap() {
        let mut row = view().as_object().unwrap().clone();
        row.extend(case["mods"].as_object().unwrap().clone());
        let got = attempt(private_views::private_visibility(&row, 901).map(|id| json!(id)));
        check(&got, case, &case["mods"].to_string());
    }

    for case in want["export_source"].as_array().unwrap() {
        let module = case["module"].as_str().unwrap();
        let w = list_wire(module);
        let ctx = format!("export_source {}", case["view"]);
        match case["error"].as_str() {
            Some("only ordinary saved-view queries have a captured export contract") => {
                w.set(|s| s["row"]["advanced_query_hash"] = json!([]));
            }
            Some(msg) if msg.starts_with("no view named '701'") => {
                w.set(|s| s["row"]["id"] = json!(702));
            }
            _ => {}
        }
        let mut got = attempt(
            list_preferences::export_source(&w.api, module, case["view"].as_str().unwrap()).map(
                |(selected, row)| json!({"id": selected.id, "name": selected.name, "row": row}),
            ),
        );
        got["calls"] = json!(w.log());
        check(&got, case, &ctx);
    }

    let w = problem_wire();
    let prob = &want["problem_reads"];
    w.set(|s| s["notes"] = json!([{"id": 9, "body": "b"}, {"id": "3", "body": "a"}]));
    let record = |w: &Wire, id| attempt_map(problem_workflows::record(&w.api, id));
    let inventory =
        |w: &Wire, kind| attempt(problem_workflows::inventory(&w.api, 123, kind).map(Value::from));
    check(&record(&w, 123), &prob["record"], "record");
    check(&record(&w, 9), &prob["record_missing"], "record missing");
    check(&inventory(&w, "notes"), &prob["notes"], "notes");
    check(&inventory(&w, "tasks"), &prob["tasks"], "tasks");
    w.set(|s| s["tasks"] = json!([{"id": 5}, {"id": "5"}]));
    check(&inventory(&w, "tasks"), &prob["tasks_dup"], "tasks dup");
    w.set(|s| s["tasks"] = json!([{"id": "x"}]));
    check(
        &inventory(&w, "tasks"),
        &prob["tasks_bad_id"],
        "tasks bad id",
    );
    w.set(|s| s["rows"]["123"] = json!({"id": "123"}));
    check(&record(&w, 123), &prob["record_str_id"], "record str id");
    w.set(|s| s["rows"]["123"] = json!({"id": 124}));
    check(
        &record(&w, 123),
        &prob["record_other_id"],
        "record other id",
    );
    assert_eq!(json!(w.log()), prob["calls"]);
}
