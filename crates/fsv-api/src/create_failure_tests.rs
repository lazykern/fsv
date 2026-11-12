//! Attachment and clone failure matrices (Python `3432bea`:
//! `tests/test_attachment_failure_matrix.py`,
//! `tests/test_clone_failure_matrix.py`). Expected values come from
//! `port/golden/gen_fsv_api_failure_matrix.py`, which drives the same handlers
//! through the Python library; the clone's CLI output and exit handling are
//! fsv-cli's.

use std::path::PathBuf;
use std::sync::LazyLock;

use fsv_core::client::Response;
use serde_json::{Map, Value, json};

use super::tests::Script;
use super::*;
use crate::service::testing::{Call, FnApi};

static ORACLE: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("testdata/failure_matrix_oracle.json")).unwrap()
});

/// Python's `APIError` for a rejected request (the real client attaches the request).
fn denied(call: &Call, status: u16, description: &str) -> Error {
    ApiError::with_request(
        status,
        json!({"description": description}),
        call.method.clone(),
        format!("https://acme.freshservice.com/api/_/{}", call.path),
    )
    .into()
}

/// `{"ok": value}` or the oracle's error projection (`status` only for API errors).
fn outcome(r: Result<Value>, want: &Value) -> Value {
    match r {
        Ok(v) => json!({"ok": v}),
        Err(e) => {
            let mut out = json!({"msg": e.to_string(), "exit": e.exit_code()});
            if want["error"] == "APIError" {
                out["status"] = json!(e.as_api().map(|a| a.status));
            }
            out
        }
    }
}

fn want_outcome(case: &Value) -> Value {
    if case.get("ok").is_some() {
        return json!({"ok": case["ok"]});
    }
    let mut out = json!({"msg": case["msg"], "exit": case["exit"]});
    if case["error"] == "APIError" {
        out["status"] = case["status"].clone();
    }
    out
}

fn writes(c: &FnApi) -> Value {
    json!(
        c.writes()
            .iter()
            .filter(|w| w.method != "FETCH")
            .map(|w| json!([w.method, w.path, w.body]))
            .collect::<Vec<_>>()
    )
}

fn calls_of(c: &FnApi, method: &str) -> Value {
    json!(
        c.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|w| w.method == method)
            .map(|w| w.path.clone())
            .collect::<Vec<_>>()
    )
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fsv_api_failure_matrix_{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn plan_file(tag: &str, data: &[u8]) -> String {
    let d = fresh_dir(tag);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("plan.txt");
    std::fs::write(&p, data).unwrap();
    p.to_string_lossy().into_owned()
}

// ------------------------------------------------------------ attachments

/// The Python `attach_wire`: `fail` names the download or a write path that
/// answers 503; uploads echo name and size unless `upload` overrides.
fn attach_api(fail: Option<&'static str>, upload: Option<Value>) -> FnApi {
    let mut n = 0;
    FnApi::new(move |call| {
        if call.method == "GET" {
            assert_eq!(call.path, "changes/123");
            return Ok(json!({"change": {"attachments": [
                {"id": 71, "name": "plan.txt", "attachment_url": "https://example.com/old"},
                {"id": 72, "name": "keep.txt"}]}}));
        }
        n += 1;
        if fail == Some(call.path.as_str()) {
            return Err(denied(call, 503, "synthetic outage"));
        }
        if call.path == "attachments" {
            let body = call.body.as_ref().unwrap();
            return Ok(upload.clone().unwrap_or_else(|| {
                json!({"id": 90 + n, "content_file_name": body["content_file_name"],
                       "content_file_size": body["content_file_size"]})
            }));
        }
        assert_eq!(call.path, "changes/123");
        Ok(json!({"change": {"id": 123}}))
    })
    .with_fetch(move |url| {
        assert_eq!(url, "https://example.com/old");
        let status = if fail == Some("download") { 503 } else { 200 };
        Ok(Response::new(status).with_body(b"Acme old bytes".to_vec()))
    })
}

fn replace_opts(mode: &str, backup: bool) -> DuplicateOpts {
    DuplicateOpts {
        duplicate: Some(mode.into()),
        backup_replaced: Some(backup),
        backup_name: Some("plan.backup.txt".into()),
    }
}

fn uploaded_bytes(c: &FnApi) -> Vec<String> {
    c.writes()
        .iter()
        .filter(|w| w.path == "attachments")
        .map(|w| {
            let content = w.body.as_ref().unwrap()["content"].as_str().unwrap();
            let b64 = content.split_once(',').unwrap().1;
            let raw = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap();
            String::from_utf8(raw).unwrap()
        })
        .collect()
}

#[test]
fn duplicate_policies_write_exactly_what_python_writes() {
    for case in ORACLE["attachments"]["policies"].as_array().unwrap() {
        let mode = case["mode"].as_str().unwrap();
        let backup = case["backup"].as_bool().unwrap();
        let c = attach_api(None, None);
        let path = plan_file(&format!("policy_{mode}_{backup}"), b"Globex new bytes");
        let r = attach_files_to_change(
            &c,
            123,
            &[path],
            &replace_opts(mode, backup),
            &Script::new(false, &[]),
        );
        let label = format!("{mode} backup={backup}");
        assert_eq!(outcome(r, case), want_outcome(case), "{label}");
        assert_eq!(writes(&c), case["writes"], "{label}");
        assert_eq!(json!(uploaded_bytes(&c)), case["upload_bytes"], "{label}");
        assert_eq!(calls_of(&c, "FETCH"), case["downloads"], "{label}");
    }
}

#[test]
fn replacement_failures_stop_at_the_failed_step() {
    for case in ORACLE["attachments"]["replace_failure"].as_array().unwrap() {
        let failure: &'static str = match case["failure"].as_str().unwrap() {
            "download" => "download",
            "attachments" => "attachments",
            "changes/123" => "changes/123",
            other => panic!("{other}"),
        };
        let c = attach_api(Some(failure), None);
        let path = plan_file(
            &format!("replace_{}", failure.replace('/', "_")),
            b"Globex new bytes",
        );
        let r = attach_files_to_change(
            &c,
            123,
            &[path],
            &replace_opts("replace", true),
            &Script::new(false, &[]),
        );
        assert_eq!(outcome(r, case), want_outcome(case), "{failure}");
        assert_eq!(writes(&c), case["writes"], "{failure}");
        assert_eq!(calls_of(&c, "FETCH"), case["downloads"], "{failure}");
    }
}

#[test]
fn upload_without_an_id_is_an_error_and_never_attaches() {
    for (i, case) in ORACLE["attachments"]["missing_id"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let c = attach_api(None, Some(case["response"].clone()));
        let path = plan_file(&format!("missing_id_{i}"), b"new");
        let r = attach_files_to_change(
            &c,
            123,
            &[path],
            &DuplicateOpts {
                duplicate: Some("append".into()),
                ..Default::default()
            },
            &Script::new(false, &[]),
        );
        assert_eq!(outcome(r, case), want_outcome(case), "{}", case["response"]);
        assert_eq!(writes(&c), case["writes"], "{}", case["response"]);
    }
}

#[test]
fn upload_receipt_diagnostics() {
    for case in ORACLE["attachments"]["receipt"].as_array().unwrap() {
        let response = case["response"].clone();
        let c = attach_api(None, Some(response.clone()));
        let r = upload_bytes(&c, "plan.txt", b"new", "text/plain").map(|id| json!(id));
        assert_eq!(outcome(r, case), want_outcome(case), "{response}");
        assert_eq!(writes(&c), case["writes"], "{response}");
        assert_eq!(
            format!(
                "  uploading plan.txt... {}\n",
                upload_receipt("plan.txt", 3, &response)
            ),
            case["stderr"].as_str().unwrap(),
        );
    }
}

#[test]
fn invalid_duplicate_mode_writes_nothing() {
    let case = &ORACLE["attachments"]["invalid_mode"];
    let c = attach_api(None, None);
    let r = attach_files_to_change(
        &c,
        123,
        &["missing.txt".into()],
        &DuplicateOpts {
            duplicate: Some("typo".into()),
            ..Default::default()
        },
        &Script::new(false, &[]),
    );
    assert_eq!(outcome(r, case), want_outcome(case));
    assert_eq!(writes(&c), case["writes"]);
}

#[test]
fn failed_download_keeps_existing_bytes_even_with_force() {
    let case = &ORACLE["attachments"]["download_error"];
    let c = attach_api(Some("download"), None);
    let dir = fresh_dir("download_error");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plan.txt"), b"local bytes").unwrap();
    let att = json!({"name": "plan.txt", "attachment_url": "https://example.com/old"});
    let r = download_attachment(&c, &att, &dir, true);
    assert_eq!(outcome(r, case), want_outcome(case));
    assert_eq!(
        std::fs::read_to_string(dir.join("plan.txt")).unwrap(),
        case["kept"].as_str().unwrap()
    );
    assert_eq!(calls_of(&c, "FETCH"), case["downloads"]);
}

#[test]
fn missing_download_url_creates_no_directory() {
    let case = &ORACLE["attachments"]["missing_url"];
    let c = attach_api(None, None);
    let dir = fresh_dir("missing_url");
    let r = download_attachment(&c, &json!({"name": "plan.txt"}), &dir, false);
    assert_eq!(outcome(r, case), want_outcome(case));
    assert_eq!(json!(dir.exists()), case["created_dir"]);
    assert_eq!(calls_of(&c, "FETCH"), case["downloads"]);
}

// ------------------------------------------------------------ clone

/// The Python `clone_wire`: `fail` (`base`, `tasks`, `assets`, `planning`)
/// answers `status`; the base create returns no id when `missing_id`.
fn clone_api(fail: Option<String>, status: u16, missing_id: bool) -> FnApi {
    FnApi::new(move |call| {
        if call.method == "GET" {
            return Ok(match call.path.as_str() {
                "changes/123" => json!({"change": {"id": 123, "subject": "Acme maintenance",
                    "status": 47, "attachments": [{"id": 11}], "state_flow_id": 27}}),
                "changes/123/tasks" => json!({"tasks": [
                    {"id": 501, "title": "Verify Acme", "owner_id": 71, "status": 3}]}),
                "changes/123/assets" => json!({"associated_assets": [
                    {"id": 801, "config_item": {"display_id": 701}}]}),
                "changes/123/planning-fields" => json!({"change_planning_fields": [
                    {"name": "test_plan", "description": "Acme checks", "attachments": []}]}),
                other => panic!("unexpected read {other}"),
            });
        }
        let path = call.path.split('?').next().unwrap();
        let kind = if path == "changes" {
            "base"
        } else if path.ends_with("/tasks") {
            "tasks"
        } else if path.ends_with("/associate") {
            "assets"
        } else {
            "planning"
        };
        if fail.as_deref() == Some(kind) {
            return Err(denied(call, status, "synthetic denial"));
        }
        if kind == "base" {
            return Ok(if missing_id {
                json!({"change": {}})
            } else {
                json!({"change": {"id": 456, "human_display_id": "CHN-456"}})
            });
        }
        Ok(json!({"task": {"id": 901}}))
    })
}

/// The library calls of `changes clone 123 --no-edit --with-tasks
/// --with-assets --with-planning`: base create, then each step once.
fn run_clone(c: &FnApi, want: &Value) -> Value {
    let data = change_clone_data(c, 123).unwrap();
    let created = submit_change(c, &data, &[]).unwrap();
    let mut steps = Map::new();
    let mut first: Option<Error> = None;
    if let Some(new) = created.get("id").and_then(Value::as_i64) {
        let mut dropped = Vec::new();
        let results: [(&str, Result<Value>); 3] = [
            (
                "tasks",
                clone_tasks(c, 123, new, false, None, Some(&mut dropped)).map(|v| json!(v)),
            ),
            ("assets", clone_assets(c, 123, new).map(|v| json!(v))),
            (
                "planning",
                clone_planning_fields(c, 123, new).map(|v| json!(v)),
            ),
        ];
        for (name, r) in results {
            let r = r.map_err(|e| {
                let shown = e.to_string();
                let code = e.exit_code();
                let status = e.as_api().map(|a| a.status);
                if first.is_none() {
                    first = Some(e);
                }
                (shown, code, status)
            });
            steps.insert(
                name.into(),
                match r {
                    Ok(v) => json!({"ok": v}),
                    Err((msg, exit, status)) => {
                        let mut out = json!({"msg": msg, "exit": exit});
                        if want["steps"][name]["error"] == "APIError" {
                            out["status"] = json!(status);
                        }
                        out
                    }
                },
            );
        }
    }
    json!({
        "data": data,
        "created": created,
        "steps": steps,
        "exit": first.map_or(0, |e| e.exit_code()),
        "writes": writes(c),
        "reads": calls_of(c, "GET"),
    })
}

fn want_clone(case: &Value) -> Value {
    let steps: Map<String, Value> = case["steps"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), want_outcome(v)))
        .collect();
    json!({
        "data": case["data"], "created": case["created"], "steps": steps,
        "exit": case["exit"], "writes": case["writes"], "reads": case["reads"],
    })
}

#[test]
fn partial_clone_attempts_each_step_once_and_keeps_the_new_identity() {
    for case in ORACLE["clone"]["partial"].as_array().unwrap() {
        let failed = case["failed"].as_str().unwrap().to_string();
        let status = case["status"].as_u64().unwrap() as u16;
        let c = clone_api(Some(failed.clone()), status, false);
        assert_eq!(run_clone(&c, case), want_clone(case), "{failed} {status}");
    }
}

#[test]
fn clone_side_effects_are_read_only_counts() {
    let case = &ORACLE["clone"]["side_effects"];
    let c = clone_api(None, 500, false);
    assert_eq!(json!(change_clone_data(&c, 123).unwrap()), case["data"]);
    assert_eq!(json!(clone_side_effects(&c, 123).unwrap()), case["effects"]);
    assert_eq!(writes(&c), case["writes"]);
    assert_eq!(calls_of(&c, "GET"), case["reads"]);
}

#[test]
fn clone_without_a_returned_id_runs_no_steps() {
    let case = &ORACLE["clone"]["missing_id"];
    let c = clone_api(None, 500, true);
    assert_eq!(run_clone(&c, case), want_clone(case));
}
