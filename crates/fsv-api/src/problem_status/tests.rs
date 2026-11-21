//! Replays `gen_fsv_api_problem_status.py` cases (Python 5d7d777).

use std::sync::{Arc, Mutex};

use super::*;
use crate::formmeta::{ClientResolver, MemStore};
use crate::service::testing::*;

const LABELS: [&str; 4] = ["Open", "In Progress", "Resolve", "Closed"];

/// Built-in Open/Closed keep their platform internal ids (1, 3); custom
/// statuses get tenant-specific ids (Python b6b741f `sid`).
fn sid(n: i64, label: &str) -> i64 {
    match label {
        "Open" => 1,
        "Closed" => 3,
        "In Progress" => n + 1,
        _ => n + 2,
    }
}

fn problem_fields(n: i64) -> Value {
    let choices: Vec<Value> = LABELS
        .iter()
        .map(|l| json!({"id": sid(n, l), "value": l}))
        .collect();
    json!([{"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1,
            "required_for_closure": true},
           {"id": 502, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
            "choices": choices}])
}

fn new_state(n: i64) -> Value {
    let analysis: Vec<Value> = PROBLEM_SECTIONS
        .iter()
        .enumerate()
        .map(|(i, (_, key, _))| json!({"id": i + 1, "name": key, "description": "<div>Acme evidence</div>"}))
        .collect();
    json!({"row": {"id": 123, "subject": "Acme problem", "status": sid(n, "Open"), "planned_start_date": null, "planned_end_date": null},
           "analysis": analysis, "solutions": [{"id": 701, "title": "Acme solution", "article_type": 1}], "fields": problem_fields(n),
           "rules": [], "fail": null, "mismatch": false, "partial": false, "drop_dates": false, "requests": []})
}

fn write(s: &mut Value, call: &Call) -> Result<Value> {
    let raw = String::from_utf8(call.raw.clone().unwrap()).unwrap();
    s["requests"]
        .as_array_mut()
        .unwrap()
        .push(json!({"method": call.method, "path": call.path, "raw": raw}));
    let body = call.body.clone().unwrap();
    match &s["fail"] {
        Value::String(_) => return Err(Error::other("disconnect")),
        Value::Object(f) => {
            let status = f["status"].as_u64().unwrap() as u16;
            return Err(ApiError::new(status, f["body"].clone()).into());
        }
        Value::Number(n) => {
            let status = n.as_u64().unwrap() as u16;
            return Err(ApiError::new(status, json!({"error": "failed"})).into());
        }
        _ => {}
    }
    if s["mismatch"] != true {
        for (k, v) in body.as_object().unwrap() {
            s["row"][k] = v.clone();
        }
    }
    s["row"]["planned_start_date"] = json!("2030-01-02T10:00:00Z");
    s["row"]["planned_end_date"] = json!("2030-01-02T11:00:00Z");
    if s["drop_dates"] == true {
        s["row"]
            .as_object_mut()
            .unwrap()
            .shift_remove("planned_end_date");
    }
    Ok(json!({"problem": {"id": 123}}))
}

fn api(state: Arc<Mutex<Value>>) -> FnApi {
    let mut a = FnApi::new(move |call: &Call| {
        let mut s = state.lock().unwrap();
        if call.method != "GET" {
            return write(&mut s, call);
        }
        let path = call.path.as_str();
        if path == "problem_form_fields" {
            return Ok(json!({"problem_fields": s["fields"]}));
        }
        if path.ends_with("business_rules") {
            if let Some(status) = s
                .get("rules_status")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
            {
                return Err(ApiError::new(status as u16, json!({"error": "synthetic"})).into());
            }
            return Ok(json!({"business_rules": s["rules"]}));
        }
        if path.ends_with("planning-fields") {
            return Ok(json!({"problem_planning_fields": s["analysis"]}));
        }
        if path.ends_with("solutions") {
            if let Some(status) = s
                .get("solution_error")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
            {
                return Err(ApiError::new(status as u16, json!({"error": "unavailable"})).into());
            }
            if let Some(raw) = s.get("solution_raw").filter(|v| !v.is_null()) {
                return Ok(raw.clone());
            }
            if let Some(pages) = s.get("solution_pages").and_then(Value::as_array) {
                let page = call
                    .params
                    .as_ref()
                    .and_then(|p| p.get("page"))
                    .map(fsv_py::py_str)
                    .and_then(|text| text.parse::<usize>().ok())
                    .filter(|n| *n >= 1)
                    .unwrap_or(1);
                return Ok(pages.get(page - 1).cloned().unwrap_or(json!({})));
            }
            return Ok(json!({"articles": s["solutions"], "has_more": s["partial"]}));
        }
        if path == "problems/123" {
            return Ok(json!({"problem": s["row"]}));
        }
        if path == "problems/123/activities" {
            let reads = s.get("activity_reads").and_then(Value::as_i64).unwrap_or(0);
            s["activity_reads"] = json!(reads + 1);
            if s.get("activities_fail") == Some(&json!(true)) {
                return Err(Error::other("activity unavailable"));
            }
            return Ok(json!({"activities": s.get("activities").cloned().unwrap_or(json!([]))}));
        }
        Err(ApiError::new(404, json!({"error": "unexpected"})).into())
    });
    a.domain = "acme.example.com".into();
    a
}

#[test]
fn cases_match_python() {
    let dir = std::env::temp_dir().join(format!("fsv-closure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    set_closure_rules_dir(Some(dir.clone()));
    let oracle: Value =
        serde_json::from_str(include_str!("../testdata/problem_status_oracle.json")).unwrap();
    let cases = oracle.as_object().unwrap();
    assert_eq!(cases.len(), 40);
    for (name, case) in cases {
        if name == "_python" {
            continue;
        }
        let state = Arc::new(Mutex::new(new_state(case["n"].as_i64().unwrap())));
        let c = Arc::new(api(state.clone()));
        let resolver = || -> Option<Arc<dyn LookupResolver>> {
            Some(Arc::new(ClientResolver::new(c.clone())))
        };
        let store = MemStore {
            workspace: json!(17),
            ..MemStore::default()
        };
        let journal = MemJournal::default();
        let mut plan: Option<StatusPlan> = None;
        let ops = case["ops"].as_array().unwrap();
        for (i, (op, want)) in ops
            .iter()
            .zip(case["outputs"].as_array().unwrap())
            .enumerate()
        {
            let ctx = format!("{name} op {i}");
            let got: Option<Result<Value>> = match op["op"].as_str().unwrap() {
                "build" => {
                    let target = op.get("target").map_or("PRB-123".to_string(), py_str);
                    Some(
                        build_plan(&*c, &store, &target, op["to"].as_str().unwrap(), resolver())
                            .map(|p| {
                                let w = &p.write;
                                let out = json!({"plan": {"action": w.action, "target": w.target,
                                    "tenant": w.tenant, "method": w.method, "path": w.path,
                                    "payload_json": w.payload_json, "context_hash": w.context_hash,
                                    "label": p.label, "before_json": p.before_json,
                                    "warnings_json": p.warnings_json,
                                    "status_labels_json": p.status_labels_json,
                                    "closure_check_json": p.closure_check_json},
                                    "hash": p.hash(), "review": p.review()});
                                plan = Some(p);
                                out
                            }),
                    )
                }
                "send" => {
                    let p = plan.as_ref().unwrap();
                    let full = p.hash();
                    let reviewed = match op.get("hash").and_then(Value::as_str).unwrap_or("full") {
                        "none" => None,
                        "prefix" => Some(&full[..16]),
                        _ => Some(full.as_str()),
                    };
                    Some(
                        send(p, &*c, &store, &journal, reviewed, resolver())
                            .map(|r| json!({"result": r})),
                    )
                }
                "set" => {
                    let mut s = state.lock().unwrap();
                    for (k, v) in op
                        .get("state")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s[k] = v.clone();
                    }
                    for (k, v) in op
                        .get("row")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s["row"][k] = v.clone();
                    }
                    for (k, v) in op
                        .get("analysis0")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        s["analysis"][0][k] = v.clone();
                    }
                    None
                }
                other => panic!("{other}"),
            };
            match got {
                Some(Ok(v)) => {
                    for (k, x) in v.as_object().unwrap() {
                        assert_eq!(&want[k], x, "{ctx} {k}");
                    }
                }
                Some(Err(e)) => {
                    assert_eq!(
                        e.to_string(),
                        want["error"]["msg"].as_str().unwrap(),
                        "{ctx}"
                    );
                    match want["error"]["type"].as_str().unwrap() {
                        "ValueError" => assert!(matches!(e, Error::Value(_)), "{ctx}"),
                        "ReviewedWriteError" => assert_eq!(e.exit_code(), 2, "{ctx}"),
                        _ => {}
                    }
                }
                None => assert!(want.get("error").is_none(), "{ctx}"),
            }
            assert_eq!(
                state.lock().unwrap()["requests"].as_array().unwrap().len() as u64,
                want["writes"].as_u64().unwrap(),
                "{ctx}"
            );
        }
        assert_eq!(
            state.lock().unwrap()["requests"],
            case["writes"],
            "{name} writes"
        );
        assert_eq!(json!(journal.events()), case["events"], "{name} events");
    }
}

const MAP: &[(&str, &str)] = &[
    ("1", "Open"),
    ("5", "In Progress"),
    ("7", "Monitor"),
    ("8", "Resolve"),
    ("9", "Closed"),
];

fn labels(extra: &[(&str, &str)]) -> Map<String, Value> {
    MAP.iter()
        .chain(extra)
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect()
}

fn at(text: &str) -> jiff::Timestamp {
    let (dt, off) = fsv_forms::pydt::datetime_fromisoformat(&text.replace('Z', "+00:00")).unwrap();
    off.unwrap().to_timestamp(dt).unwrap()
}

/// `test_problem_write_fixes._native_activity`.
fn native_activity(write_at: &str, workflow_at: &str, requested: &str, rewritten: &str) -> Vec<Value> {
    vec![
        json!({"actor": {"id": 0, "name": "Problem Workflow", "is_agent_deleted": false},
               "content": "executed  <a>Acme status workflow</a>  workflow from  <b>Problem status changed (any to any)</b>  event",
               "sub_contents": [format!("set status as <b>{rewritten}</b>"), "<b>Workflow Ends</b>"],
               "created_at": workflow_at}),
        json!({"actor": {"id": 9000001, "name": "Acme Agent", "is_agent": true},
               "content": format!("set status as  <b>{requested}</b> , set planned end date as  <b>Thu, 2 Jan  4:30 PM</b>"),
               "sub_contents": null, "created_at": write_at}),
    ]
}

/// `test_workflow_rewrite_back_to_prior_status_uses_native_activity_rows`
/// (Python d90affa).
#[test]
fn workflow_rewrite_uses_native_activity_rows() {
    let since = at("2030-01-01T00:00:00Z");
    let rows = native_activity("2030-01-01T00:01:00Z", "2030-01-01T00:01:01Z", "Resolve", "Monitor");
    let l = labels(&[]);
    let (note, conflict) = workflow_rewrite(&rows, since, &l, &json!(8), &json!(7));
    let note = note.unwrap();
    assert!(
        note.contains("Resolve")
            && note.contains("Monitor (status 7)")
            && note.contains("Problem Workflow")
            && conflict.is_none()
    );
    // No entry for this write (requested Closed): the readback stays unverified.
    assert_eq!(workflow_rewrite(&rows, since, &l, &json!(9), &json!(7)), (None, None));
    // The workflow set another status than the readback: a conflict.
    let (note, conflict) = workflow_rewrite(&rows, since, &l, &json!(8), &json!(1));
    assert!(note.is_none() && conflict.unwrap().contains("readback status is Open"));
    // Workflow entry older than the write is not evidence.
    let stale = native_activity("2030-01-01T00:02:00Z", "2030-01-01T00:01:00Z", "Resolve", "Monitor");
    assert_eq!(workflow_rewrite(&stale, since, &l, &json!(8), &json!(7)), (None, None));
    // Entries before the write window do not count.
    let old = native_activity("2029-12-31T00:01:00Z", "2029-12-31T00:01:01Z", "Resolve", "Monitor");
    assert_eq!(workflow_rewrite(&old, since, &l, &json!(8), &json!(7)), (None, None));
    // A workflow row that did not set a status is not a rewrite.
    let mut noset = native_activity("2030-01-01T00:01:00Z", "2030-01-01T00:01:01Z", "Resolve", "Monitor");
    noset[0]["sub_contents"] = json!(["send email to <b>requester</b>", "<b>Workflow Ends</b>"]);
    assert_eq!(workflow_rewrite(&noset, since, &l, &json!(8), &json!(7)), (None, None));
}

/// `test_workflow_row_is_detected_structurally_not_by_wording` (Python d90affa).
#[test]
fn workflow_row_is_detected_structurally() {
    let since = at("2030-01-01T00:00:00Z");
    let l = labels(&[]);
    let mut rows = native_activity("2030-01-01T00:01:00Z", "2030-01-01T00:01:01Z", "Resolve", "Monitor");
    rows[0]["actor"] = json!({"id": 9000002, "name": "Acme Agent Two", "is_agent": true});
    assert_eq!(workflow_rewrite(&rows, since, &l, &json!(8), &json!(7)), (None, None));
    let mut rows = native_activity("2030-01-01T00:01:00Z", "2030-01-01T00:01:01Z", "Resolve", "Monitor");
    rows[0]["content"] = json!("ran automation");
    assert!(workflow_rewrite(&rows, since, &l, &json!(8), &json!(7)).0.is_some());
    let short = labels(&[]).into_iter().filter(|(k, _)| k == "8").collect();
    assert!(workflow_rewrite(&rows, since, &short, &json!(8), &json!(7)).0.is_none());
    assert!(workflow_rewrite(&rows, since, &labels(&[("10", "Monitor")]), &json!(8), &json!(7)).0.is_none());
}

/// `test_problem_status_readback`: only the requested status verifies.
#[test]
fn status_readback_verifies_only_the_requested_status() {
    for (before, after, sent, verified) in [
        (json!(1), json!(5), json!(5), true),
        (json!(1), json!(7), json!(4), false),
        (json!(4), json!(4), json!(5), false),
        (json!(1), Value::Null, json!(5), false),
    ] {
        let (ok, note) = status_readback(&before, &after, &sent);
        assert_eq!((ok, note), (verified, None));
    }
}

fn agent_row(label: &str, stamp: &str) -> Value {
    json!({"actor": {"id": 9000001, "name": "Acme Agent", "is_agent": true},
           "content": format!("set status as  <b>{label}</b>"), "sub_contents": null, "created_at": stamp})
}

fn workflow_row(label: &str, stamp: &str) -> Value {
    json!({"actor": {"id": 0, "name": "Problem Workflow", "is_agent_deleted": false},
           "content": "executed  <a>Acme status workflow</a>  workflow from  <b>Problem status changed</b>  event",
           "sub_contents": [format!("set status as <b>{label}</b>"), "<b>Workflow Ends</b>"],
           "created_at": stamp})
}

/// `test_workflow_rewrite_back_to_the_prior_status_is_verified_once_activity_is_logged`.
#[test]
fn workflow_rewrite_is_verified_once_activity_is_logged() {
    for label in ["Open", "Resolve"] {
        let n = 41;
        let state = Arc::new(Mutex::new(new_state(n)));
        state.lock().unwrap()["fields"][1]["choices"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": n + 4, "value": "Monitor 1 month"}));
        state.lock().unwrap()["row"]["status"] = json!(n + 4);
        let c = Arc::new(api(state.clone()));
        let store = MemStore::default();
        let resolver = || -> Option<Arc<dyn LookupResolver>> {
            Some(Arc::new(ClientResolver::new(c.clone())))
        };
        let plan = build_plan(&*c, &store, "123", label, resolver()).unwrap();
        let now = jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
        {
            let mut s = state.lock().unwrap();
            s["mismatch"] = json!(true);
            s["activities"] = json!([agent_row(label, &now)]);
        }
        let reads = std::rc::Rc::new(std::cell::Cell::new(0));
        let hook_state = state.clone();
        let hook_reads = reads.clone();
        let stamp = now.clone();
        let journal = MemJournal::default();
        let result = crate::readback::with_readback_sleep(
            move |_| {
                hook_reads.set(hook_reads.get() + 1);
                hook_state.lock().unwrap()["activities"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, workflow_row("Monitor 1 month", &stamp));
            },
            || send(&plan, &*c, &store, &journal, Some(&plan.hash()), resolver()).unwrap(),
        );
        assert_eq!(state.lock().unwrap()["requests"].as_array().unwrap().len(), 1);
        assert_eq!(result["readback"]["status"], json!(n + 4));
        assert_eq!((result["verified"].clone(), reads.get()), (json!(true), 1));
        assert_eq!(result["final_status"], "Monitor 1 month");
        assert!(
            result["verified_with_note"].as_str().unwrap().contains(&format!(
                "set to {label} by this write, then rewritten server-side to Monitor 1 month (status {})",
                n + 4
            ))
        );
    }
}

/// `test_rewrite_without_activity_stays_unverified`.
#[test]
fn rewrite_without_readable_activity_stays_unverified() {
    let n = 41;
    let state = Arc::new(Mutex::new(new_state(n)));
    state.lock().unwrap()["fields"][1]["choices"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": n + 4, "value": "Monitor 1 month"}));
    state.lock().unwrap()["row"]["status"] = json!(n + 4);
    let c = Arc::new(api(state.clone()));
    let store = MemStore::default();
    let resolver = || -> Option<Arc<dyn LookupResolver>> {
        Some(Arc::new(ClientResolver::new(c.clone())))
    };
    let plan = build_plan(&*c, &store, "123", "Open", resolver()).unwrap();
    {
        let mut s = state.lock().unwrap();
        s["mismatch"] = json!(true);
        s["activities_fail"] = json!(true);
    }
    let journal = MemJournal::default();
    let result = send(&plan, &*c, &store, &journal, Some(&plan.hash()), resolver()).unwrap();
    assert_eq!(result["verified"], false);
    assert!(result.get("verified_with_note").is_none());
    assert!(result["warning"].as_str().unwrap().contains("do not resend"));
    assert_eq!(
        state.lock().unwrap()["activity_reads"],
        json!(ACTIVITY_ATTEMPTS)
    );
}

/// `test_closed_rewritten_by_workflow_is_verified_with_note` (Python 92b7a55):
/// Closed (with planned dates filled), then a workflow put the status back
/// to the pre-write monitoring status.
#[test]
fn closed_rewritten_by_workflow_is_verified_with_note() {
    let cases = [
        ("closure-workflow-stamped-first", true),
        ("closure-same-second", true),
        ("workflow-label-differs-from-readback", false),
        ("workflow-minutes-older", false),
        ("no-own-closure-entry", false),
    ];
    for n in [41, 81] {
        for (case, verified) in cases {
            let state = Arc::new(Mutex::new(new_state(n)));
            state.lock().unwrap()["fields"][1]["choices"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": n + 4, "value": "Monitor"}));
            state.lock().unwrap()["row"]["status"] = json!(n + 4);
            let c = Arc::new(api(state.clone()));
            let store = MemStore::default();
            let resolver = || -> Option<Arc<dyn LookupResolver>> {
                Some(Arc::new(ClientResolver::new(c.clone())))
            };
            let plan = build_plan(&*c, &store, "123", "Closed", resolver()).unwrap();
            let now = jiff::Timestamp::now();
            let stamp = |d: i64| {
                now.checked_add(jiff::SignedDuration::from_secs(d))
                    .unwrap()
                    .strftime("%Y-%m-%dT%H:%M:%SZ")
                    .to_string()
            };
            let wf_at = match case {
                "closure-workflow-stamped-first" => -1,
                "workflow-minutes-older" => -60,
                _ => 0,
            };
            let rewritten = if case == "workflow-label-differs-from-readback" {
                "Open"
            } else {
                "Monitor"
            };
            let own = if case == "no-own-closure-entry" {
                "set priority as  <b>Low</b>"
            } else {
                "set status as  <b>Closed</b> , set planned end date as  <b>Thu, 2 Jan  4:30 PM</b> , set planned start date as  <b>Thu, 2 Jan  3:30 PM</b>"
            };
            {
                let mut s = state.lock().unwrap();
                s["mismatch"] = json!(true);
                s["activities"] = json!([
                    {"actor": {"id": 0, "name": "Problem Workflow", "is_agent_deleted": false},
                     "content": "executed  <a>Acme status workflow</a>  workflow from  <b>Problem status changed</b>  event",
                     "sub_contents": [format!("set status as <b>{rewritten}</b>"), "<b>Workflow Ends</b>"],
                     "created_at": stamp(wf_at)},
                    {"actor": {"id": 9000001, "name": "Acme Agent", "is_agent": true},
                     "content": own, "sub_contents": null, "created_at": stamp(0)}]);
            }
            let journal = MemJournal::default();
            let result =
                send(&plan, &*c, &store, &journal, Some(&plan.hash()), resolver()).unwrap();
            let ctx = format!("{n} {case}: {result}");
            let s = state.lock().unwrap();
            let writes = s["requests"].as_array().unwrap().len();
            assert_eq!(writes, 1, "{ctx}");
            assert_eq!(result["verified"], json!(verified), "{ctx}");
            assert_eq!(result["readback"]["status"], json!(n + 4), "{ctx}");
            assert!(
                fsv_py::truthy(&result["readback"]["planned_end_date"]),
                "{ctx}"
            );
            if verified {
                assert!(
                    result["verified_with_note"].as_str().unwrap().contains(
                        "set to Closed by this write, then rewritten server-side to Monitor"
                    ),
                    "{ctx}"
                );
                assert!(result.get("warning").is_none(), "{ctx}");
            } else {
                assert!(result.get("verified_with_note").is_none(), "{ctx}");
                assert!(
                    result["warning"]
                        .as_str()
                        .unwrap()
                        .contains("do not resend"),
                    "{ctx}"
                );
            }
            if case == "workflow-label-differs-from-readback" {
                assert!(
                    result["readback_conflict"]
                        .as_str()
                        .unwrap()
                        .contains("readback status is Monitor"),
                    "{ctx}"
                );
            }
        }
    }
}
