//! Lifecycle rendering matrix (Python `92d76ff`:
//! `tests/test_state_flow_render.py`). Expected values are the exact
//! `console.print` arguments recorded by
//! `port/golden/gen_fsv_api_query_flow_matrix.py`.

use std::sync::LazyLock;

use fsv_core::ApiError;
use serde_json::{Value, json};

use super::*;
use crate::service::testing::FnApi;

static ORACLE: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("testdata/query_flow_oracle.json")).unwrap()
});

fn sf() -> &'static Value {
    &ORACLE["state_flow"]
}

fn printed(out: &[Printed]) -> Value {
    json!(
        out.iter()
            .map(|p| match p {
                Printed::Text(s) => json!({"text": s}),
                Printed::Blank => json!({"blank": true}),
                Printed::Table { title, rows } => json!({"table": title, "rows": rows}),
            })
            .collect::<Vec<_>>()
    )
}

fn denial(status: u16, text: &str) -> Error {
    ApiError::new(status, json!(text)).into()
}

/// `[path, params]` per read.
fn reads(c: &FnApi) -> Value {
    json!(
        c.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| json!([c.path, c.params]))
            .collect::<Vec<_>>()
    )
}

/// The Python `flow_env`: flow 51 in workspace 17, approvals on change 123.
fn flow_env(flow: Value, deny_flow: Option<u16>) -> FnApi {
    FnApi::new(move |call| match call.path.as_str() {
        "ws/17/change_management/51" => match deny_flow {
            Some(s) => Err(denial(s, "synthetic failure")),
            None => Ok(json!({"state_flow": flow})),
        },
        "changes/123/approvals" => Ok(json!({"approvals": sf()["approvals"]})),
        other => panic!("unexpected read {other}"),
    })
}

fn renderer(name: &str) -> fn(&dyn Api, &Value, usize) -> Vec<Printed> {
    match name {
        "compact" => render_compact,
        "diagram" => render_state_diagram,
        other => panic!("{other}"),
    }
}

/// Python's default `context` per renderer.
fn default_context(name: &str) -> usize {
    if name == "compact" { 2 } else { 3 }
}

#[test]
fn flow_uses_change_workspace_and_unwraps_response() {
    for case in sf()["workspace"].as_array().unwrap() {
        let wrapped = case["wrapped"].as_bool().unwrap();
        let c = FnApi::new(move |_| {
            let flow = json!({"name": "Acme lifecycle", "state_list": []});
            Ok(if wrapped {
                json!({"state_flow": flow})
            } else {
                flow
            })
        });
        let ws = &case["workspace"];
        let got = get_state_flow(&c, &json!(51), (!ws.is_null()).then_some(ws)).unwrap();
        assert_eq!(got, case["result"], "{ws}");
        assert_eq!(reads(&c), case["calls"], "{ws}");
    }
}

#[test]
fn only_workspace_404_can_fall_back_to_plain_flow() {
    for case in sf()["fallback"].as_array().unwrap() {
        let status = case["status"].as_u64().unwrap() as u16;
        let c = FnApi::new(move |call| {
            if call.path.starts_with("ws/") {
                Err(denial(status, "synthetic denial"))
            } else {
                Ok(json!({"state_flow": {"name": "Acme fallback"}}))
            }
        });
        let got = match get_state_flow(&c, &json!(51), Some(&json!(17))) {
            Ok(v) => json!({"ok": v}),
            Err(e) => json!({"status": e.as_api().map(|a| a.status)}),
        };
        let mut want = case.clone();
        want.as_object_mut()
            .unwrap()
            .retain(|k, _| k == "ok" || k == "status");
        if case.get("ok").is_some() {
            want.as_object_mut().unwrap().remove("status");
        }
        assert_eq!(got, want, "{status}");
        let paths: Vec<String> = c
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.path.clone())
            .collect();
        assert_eq!(json!(paths), case["calls"], "{status}");
    }
}

#[test]
fn full_flow_displays_current_visited_future_and_approval_statuses() {
    let case = &sf()["full"];
    let c = flow_env(sf()["flow"].clone(), None);
    let out = render_flow(&c, &sf()["change"]).unwrap();
    assert_eq!(printed(&out), case["printed"]);
    assert_eq!(reads(&c), case["calls"]);
}

#[test]
fn context_windows_clip_without_losing_current_state() {
    for case in sf()["windows"].as_array().unwrap() {
        let name = case["renderer"].as_str().unwrap();
        let c = flow_env(sf()["flow"].clone(), None);
        let mut change = sf()["change"].clone();
        change["status"] = case["status"].clone();
        let context = case["context"].as_u64().unwrap() as usize;
        let label = format!("{name} status={} context={context}", case["status"]);
        assert_eq!(
            printed(&renderer(name)(&c, &change, context)),
            case["printed"],
            "{label}"
        );
        assert_eq!(reads(&c), case["calls"], "{label}");
    }
}

#[test]
fn compact_renderers_escape_names_instead_of_interpreting_markup() {
    for case in sf()["escaped"].as_array().unwrap() {
        let name = case["renderer"].as_str().unwrap();
        let mut flow = sf()["flow"].clone();
        flow["state_list"][3]["value"] = json!("Review [Acme] → Δ");
        let c = flow_env(flow, None);
        let out = renderer(name)(&c, &sf()["change"], default_context(name));
        assert_eq!(printed(&out), case["printed"], "{name}");
    }
}

#[test]
fn optional_diagrams_never_invent_state_when_data_is_unavailable() {
    for case in sf()["unavailable"].as_array().unwrap() {
        let name = case["renderer"].as_str().unwrap();
        let kind = case["case"].as_str().unwrap();
        let mut flow = sf()["flow"].clone();
        let mut change = sf()["change"].clone();
        match kind {
            "missing-flow" => {
                change
                    .as_object_mut()
                    .unwrap()
                    .shift_remove("state_flow_id");
            }
            "empty-states" => flow["state_list"] = json!([]),
            "unknown-current" => change["status"] = json!(999),
            _ => {}
        }
        let c = flow_env(flow, (kind == "denied").then_some(500));
        let out = renderer(name)(&c, &change, default_context(name));
        assert_eq!(printed(&out), case["printed"], "{name} {kind}");
        assert_eq!(reads(&c), case["calls"], "{name} {kind}");
    }
}

#[test]
fn full_flow_explicitly_reports_absent_lifecycle() {
    for (kind, want) in sf()["absent"].as_object().unwrap() {
        let mut flow = sf()["flow"].clone();
        let mut change = sf()["change"].clone();
        if kind == "missing" {
            change
                .as_object_mut()
                .unwrap()
                .shift_remove("state_flow_id");
        } else {
            flow["state_list"] = json!([]);
        }
        let c = flow_env(flow, None);
        assert_eq!(printed(&render_flow(&c, &change).unwrap()), *want, "{kind}");
    }
}

#[test]
fn approval_denial_is_optional_but_main_flow_denial_propagates() {
    let case = &sf()["denied"];
    let c = FnApi::new(|_| Err(denial(403, "synthetic denial")));
    assert_eq!(json!(try_get_approvals(&c, &json!(123))), case["approvals"]);
    let err = render_flow(&c, &sf()["change"]).unwrap_err();
    assert_eq!(json!(err.as_api().map(|a| a.status)), case["status"]);
    assert_eq!(case["printed"], json!([]));
}
