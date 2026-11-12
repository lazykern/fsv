//! Change state flow (approval lifecycle) reads and their Rich-markup
//! renderings (`state_flow.py`). Renderers return what Python prints, in
//! order, so any console can show it.

use serde_json::{Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::term::escape;
use fsv_py::{ValueExt, py_str, truthy};

use crate::service::{Api, obj, rows_at, unwrap_key};

/// One `console.print` of a renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Printed {
    /// A Rich-markup line.
    Text(String),
    /// `console.print()`.
    Blank,
    /// The flow table: title and `(→, Status, State)` rows, all markup.
    Table {
        title: String,
        rows: Vec<[String; 3]>,
    },
}

/// `get_state_flow`: in the change's workspace when it has one (a 404 there
/// falls back to the plain path, e.g. accounts without workspaces).
pub fn get_state_flow(
    c: &dyn Api,
    state_flow_id: &Value,
    workspace_id: Option<&Value>,
) -> Result<Value> {
    let params = obj(json!({"include": "state_info"}));
    let flow = py_str(state_flow_id);
    if let Some(ws) = workspace_id.filter(|w| !w.is_null() && **w != json!("")) {
        match c.int_get(
            &format!("ws/{}/change_management/{flow}", py_str(ws)),
            Some(params.clone()),
        ) {
            Ok(data) => return Ok(unwrap_key(data, "state_flow")),
            Err(Error::Api(e)) if e.status == 404 => {}
            Err(e) => return Err(e),
        }
    }
    Ok(unwrap_key(
        c.int_get(&format!("change_management/{flow}"), Some(params))?,
        "state_flow",
    ))
}

/// `_try_get_approvals`: the approvals list, or nothing on any failure.
pub fn try_get_approvals(c: &dyn Api, change_id: &Value) -> Vec<Value> {
    c.int_get(&format!("changes/{}/approvals", py_str(change_id)), None)
        .map(|d| rows_at(&d, "approvals"))
        .unwrap_or_default()
}

fn unique(values: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for v in values {
        if !out.contains(v) {
            out.push(v.clone());
        }
    }
    out
}

fn flow_of(c: &dyn Api, change: &Value) -> Result<Value> {
    get_state_flow(
        c,
        change.get("state_flow_id").unwrap_or(&Value::Null),
        change.get("workspace_id"),
    )
}

/// `render_flow`: the whole flow with the current position, then approval gates.
pub fn render_flow(c: &dyn Api, change: &Value) -> Result<Vec<Printed>> {
    let mut out = Vec::new();
    if change.get_truthy("state_flow_id").is_none() {
        out.push(Printed::Text(
            "[yellow]no state_flow_id on this change[/]".into(),
        ));
        return Ok(out);
    }
    let flow = flow_of(c, change)?;
    let state_list = flow.list_or_empty("state_list");
    if state_list.is_empty() {
        out.push(Printed::Text("[yellow]empty state_list[/]".into()));
        return Ok(out);
    }
    let current = change.get("status").cloned().unwrap_or(Value::Null);
    let current_name = change
        .get_truthy("status_name")
        .map(py_str)
        .unwrap_or_else(|| py_str(&current));
    let traversal = unique(change.list_or_empty("state_traversal"));
    let flow_name = flow.get("name").map(py_str).unwrap_or_else(|| "?".into());
    let hid = change
        .get_truthy("human_display_id")
        .map(py_str)
        .unwrap_or_else(|| {
            format!(
                "CHN-{}",
                change.get("id").map(py_str).unwrap_or("None".into())
            )
        });
    let change_type = change
        .get("change_type")
        .map(py_str)
        .unwrap_or_else(|| "None".into());
    let mut rows = Vec::new();
    for s in state_list {
        let sid = s.get("id").cloned().unwrap_or(Value::Null);
        let name = s.get("value").map(py_str).unwrap_or_default();
        let visited = traversal.contains(&sid);
        let row = if sid == current {
            [
                "[bold yellow]▶[/]".into(),
                format!("[bold yellow]{name}[/]"),
                "[bold yellow]current[/]".into(),
            ]
        } else if visited {
            [
                "[green]✓[/]".into(),
                format!("[green]{name}[/]"),
                "[green]passed[/]".into(),
            ]
        } else {
            [
                "[dim]·[/]".into(),
                format!("[dim]{name}[/]"),
                "[dim]future[/]".into(),
            ]
        };
        rows.push(row);
    }
    out.push(Printed::Table {
        title: format!("{hid} flow — [bold]{flow_name}[/] (change_type={change_type})"),
        rows,
    });
    let ids: Vec<Value> = state_list
        .iter()
        .map(|s| s.get("id").cloned().unwrap_or(Value::Null))
        .collect();
    let passed_in_flow = traversal.iter().filter(|t| ids.contains(t)).count() as i64;
    let ahead = state_list.len() as i64 - passed_in_flow - 1;
    out.push(Printed::Text(format!(
        "  current: [bold yellow]{current_name}[/] · past: {} · ahead: {ahead}",
        traversal.len()
    )));
    let approvals = try_get_approvals(c, change.get("id").unwrap_or(&Value::Null));
    if !approvals.is_empty() {
        out.push(Printed::Blank);
        out.push(Printed::Text("[bold]Approval gates:[/]".into()));
        for a in &approvals {
            let level = a
                .get("level_id")
                .map(py_str)
                .unwrap_or_else(|| "None".into());
            let status = a
                .obj_or_empty("status")
                .get("name")
                .map(py_str)
                .unwrap_or_else(|| "?".into());
            let member = a
                .obj_or_empty("member")
                .get("name")
                .filter(|v| truthy(v))
                .map(py_str)
                .unwrap_or_else(|| a.get("member_id").map(py_str).unwrap_or_default());
            let icon = match status.as_str() {
                "approved" => "✓",
                "rejected" => "✗",
                _ => "◷",
            };
            out.push(Printed::Text(format!(
                "  {icon} L{level}: {member} — {status}"
            )));
        }
    }
    Ok(out)
}

/// The states within `context` of the current one: `(lo, hi, cur_idx)`.
fn window(state_list: &[Value], current: &Value, context: usize) -> Option<(usize, usize, usize)> {
    let cur = state_list
        .iter()
        .position(|s| s.get("id").unwrap_or(&Value::Null) == current)?;
    let lo = cur.saturating_sub(context);
    let hi = (cur + context).min(state_list.len() - 1);
    Some((lo, hi, cur))
}

/// `render_compact`: a `grep -C` style state line for `changes get`.
/// Nothing when the flow is missing or unreadable.
pub fn render_compact(c: &dyn Api, change: &Value, context: usize) -> Vec<Printed> {
    if change.get_truthy("state_flow_id").is_none() {
        return vec![];
    }
    let Ok(flow) = flow_of(c, change) else {
        return vec![];
    };
    let state_list = flow.list_or_empty("state_list");
    if state_list.is_empty() {
        return vec![];
    }
    let current = change.get("status").cloned().unwrap_or(Value::Null);
    let traversal = unique(change.list_or_empty("state_traversal"));
    let total = state_list.len();
    let Some((lo, hi, _)) = window(state_list, &current, context) else {
        return vec![];
    };
    let mut parts = Vec::new();
    if lo > 0 {
        parts.push("[dim]…[/]".to_string());
    }
    for s in &state_list[lo..=hi] {
        let sid = s.get("id").cloned().unwrap_or(Value::Null);
        let name = escape(&s.get("value").map(py_str).unwrap_or_default());
        parts.push(if sid == current {
            format!("[bold yellow]▶ {name}[/]")
        } else if traversal.contains(&sid) {
            format!("[green]{name}[/]")
        } else {
            format!("[dim]{name}[/]")
        });
    }
    if hi < total - 1 {
        parts.push("[dim]…[/]".into());
    }
    let hid = change
        .get("human_display_id")
        .map(py_str)
        .unwrap_or_default();
    vec![
        Printed::Text(format!(
            "state:    {}  [dim][{}/{total}][/]",
            parts.join(" [dim]→[/] "),
            traversal.len()
        )),
        Printed::Text(format!(
            "[dim]          `fsv changes state {hid}` for full flow[/]"
        )),
    ]
}

/// `render_state_diagram`: progress, then the states around the current one.
pub fn render_state_diagram(c: &dyn Api, change: &Value, context: usize) -> Vec<Printed> {
    if change.get_truthy("state_flow_id").is_none() {
        return vec![];
    }
    let Ok(flow) = flow_of(c, change) else {
        return vec![];
    };
    let state_list = flow.list_or_empty("state_list");
    if state_list.is_empty() {
        return vec![];
    }
    let current = change.get("status").cloned().unwrap_or(Value::Null);
    let traversal = unique(change.list_or_empty("state_traversal"));
    let Some((lo, hi, cur)) = window(state_list, &current, context) else {
        return vec![];
    };
    let total = state_list.len();
    let done = traversal.len();
    let pct = fsv_core::pystr::round_half_even(done as f64 / total as f64 * 100.0) as i64;
    let mut parts = Vec::new();
    if lo > 0 {
        parts.push("[dim]…[/]".to_string());
    }
    for (i, s) in state_list[lo..=hi].iter().enumerate() {
        let sid = s.get("id").cloned().unwrap_or(Value::Null);
        let name = escape(&s.get("value").map(py_str).unwrap_or_default());
        parts.push(if lo + i == cur {
            format!("[bold yellow on blue] ▶ {name} [/]")
        } else if traversal.contains(&sid) {
            format!("[green]✓ {name}[/]")
        } else {
            format!("[dim]{name}[/]")
        });
    }
    if hi < total - 1 {
        parts.push("[dim]…[/]".into());
    }
    let mut out = vec![
        Printed::Text(format!(
            "[bold cyan]state flow:[/] [bold]{pct}%[/] complete  [dim]({done}/{total})[/]"
        )),
        Printed::Text(parts.join(" [dim]→[/] ")),
    ];
    if lo > 0 || hi < total - 1 {
        let hid = change
            .get("human_display_id")
            .map(py_str)
            .unwrap_or_else(|| "None".into());
        out.push(Printed::Text(format!(
            "[dim]  use `fsv changes state {hid}` for full flow[/]"
        )));
    }
    out
}

#[cfg(test)]
#[path = "state_flow_matrix_tests.rs"]
mod matrix_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::*;

    fn t(s: &str) -> Printed {
        Printed::Text(s.into())
    }

    fn flow() -> Value {
        let states: Vec<Value> = [
            (1, "Open"),
            (2, "Plan [x]"),
            (3, "Approve"),
            (4, "Build"),
            (5, "Test"),
            (6, "Done"),
            (7, "Closed"),
        ]
        .iter()
        .map(|(i, v)| json!({"id": i, "value": v}))
        .collect();
        json!({"state_flow": {"name": "Std", "state_list": states}})
    }

    fn change() -> Value {
        json!({"id": 5, "human_display_id": "CHN-5", "state_flow_id": 9, "status": 4, "status_name": "Build",
               "state_traversal": [1, 2, 3, 2], "change_type": 1})
    }

    fn with_status(status: i64) -> Value {
        let mut c = change();
        c["status"] = json!(status);
        c
    }

    fn api() -> FakeApi {
        FakeApi::new().ok("GET change_management/9", flow()).ok(
            "GET changes/5/approvals",
            json!({"approvals": [{"level_id": 1, "status": {"name": "approved"}, "member": {"name": "Ann"}},
                                 {"level_id": 2, "status": {"name": "rejected"}, "member_id": 8},
                                 {"level_id": 2, "member": {"name": ""}, "member_id": 9}]}),
        )
    }

    #[test]
    fn get_state_flow_workspace_then_plain() {
        let api = FakeApi::new()
            .ok("GET ws/3/change_management/9", flow())
            .ok(
                "GET change_management/9",
                json!({"name": "Plain", "state_list": []}),
            );
        assert_eq!(
            get_state_flow(&api, &json!(9), Some(&json!(3))).unwrap()["name"],
            "Std"
        );
        assert_eq!(
            get_state_flow(&api, &json!(9), None).unwrap(),
            json!({"name": "Plain", "state_list": []})
        );
        let calls = api.calls.lock().unwrap();
        assert_eq!(
            (calls[0].path.as_str(), calls[1].path.as_str()),
            ("ws/3/change_management/9", "change_management/9")
        );
        assert_eq!(calls[0].params, Some(obj(json!({"include": "state_info"}))));
        drop(calls);
        let fallback = FakeApi::new().ok("GET change_management/9", flow());
        assert_eq!(
            get_state_flow(&fallback, &json!(9), Some(&json!(4))).unwrap()["name"],
            "Std"
        );
        assert_eq!(
            fallback.gets(),
            ["ws/4/change_management/9", "change_management/9"]
        );
        let down = FakeApi::new().on("GET ws/4/change_management/9", Reply::Status(500));
        assert!(
            matches!(get_state_flow(&down, &json!(9), Some(&json!(4))).unwrap_err(), Error::Api(e) if e.status == 500)
        );
    }

    #[test]
    fn try_get_approvals_never_fails() {
        let env = FakeApi::new().ok("GET changes/5/approvals", json!({"approvals": [{"id": 1}]}));
        assert_eq!(try_get_approvals(&env, &json!(5)), [json!({"id": 1})]);
        let bare = FakeApi::new().ok("GET changes/5/approvals", json!([{"id": 1}]));
        assert_eq!(try_get_approvals(&bare, &json!(5)), Vec::<Value>::new());
        assert_eq!(
            try_get_approvals(&FakeApi::new(), &json!(5)),
            Vec::<Value>::new()
        );
    }

    #[test]
    fn render_flow_table_and_gates() {
        let rows = |v: &[[&str; 3]]| v.iter().map(|r| r.map(String::from)).collect::<Vec<_>>();
        let passed = |n: &str| {
            [
                "[green]✓[/]".to_string(),
                format!("[green]{n}[/]"),
                "[green]passed[/]".into(),
            ]
        };
        let future = |n: &str| {
            [
                "[dim]·[/]".to_string(),
                format!("[dim]{n}[/]"),
                "[dim]future[/]".into(),
            ]
        };
        let want_rows = vec![
            passed("Open"),
            passed("Plan [x]"),
            passed("Approve"),
            [
                "[bold yellow]▶[/]".into(),
                "[bold yellow]Build[/]".into(),
                "[bold yellow]current[/]".into(),
            ],
            future("Test"),
            future("Done"),
            future("Closed"),
        ];
        assert_eq!(
            render_flow(&api(), &change()).unwrap(),
            [
                Printed::Table {
                    title: "CHN-5 flow — [bold]Std[/] (change_type=1)".into(),
                    rows: want_rows
                },
                t("  current: [bold yellow]Build[/] · past: 3 · ahead: 3"),
                Printed::Blank,
                t("[bold]Approval gates:[/]"),
                t("  ✓ L1: Ann — approved"),
                t("  ✗ L2: 8 — rejected"),
                t("  ◷ L2: 9 — ?"),
            ]
        );
        assert_eq!(
            render_flow(&api(), &json!({"id": 5})).unwrap(),
            [t("[yellow]no state_flow_id on this change[/]")]
        );
        let empty = FakeApi::new().ok("GET change_management/9", json!({"state_list": []}));
        assert_eq!(
            render_flow(&empty, &json!({"id": 6, "state_flow_id": 9})).unwrap(),
            [t("[yellow]empty state_list[/]")]
        );
        let bare = FakeApi::new().ok(
            "GET change_management/9",
            json!({"state_list": [{"id": 1, "value": "Open"}]}),
        );
        assert_eq!(
            render_flow(&bare, &json!({"id": 6, "state_flow_id": 9, "status": 1})).unwrap(),
            [
                Printed::Table {
                    title: "CHN-6 flow — [bold]?[/] (change_type=None)".into(),
                    rows: rows(&[[
                        "[bold yellow]▶[/]",
                        "[bold yellow]Open[/]",
                        "[bold yellow]current[/]"
                    ]])
                },
                t("  current: [bold yellow]1[/] · past: 0 · ahead: 0"),
            ]
        );
    }

    #[test]
    fn render_compact_window() {
        assert_eq!(
            render_compact(&api(), &change(), 2),
            [
                t(
                    "state:    [dim]…[/] [dim]→[/] [green]Plan \\[x][/] [dim]→[/] [green]Approve[/] [dim]→[/] [bold yellow]▶ Build[/] [dim]→[/] [dim]Test[/] [dim]→[/] [dim]Done[/] [dim]→[/] [dim]…[/]  [dim][3/7][/]"
                ),
                t("[dim]          `fsv changes state CHN-5` for full flow[/]"),
            ]
        );
        assert_eq!(
            render_compact(&api(), &with_status(1), 1),
            [
                t(
                    "state:    [bold yellow]▶ Open[/] [dim]→[/] [green]Plan \\[x][/] [dim]→[/] [dim]…[/]  [dim][3/7][/]"
                ),
                t("[dim]          `fsv changes state CHN-5` for full flow[/]"),
            ]
        );
        assert!(render_compact(&api(), &with_status(99), 2).is_empty());
        assert!(render_compact(&FakeApi::new(), &change(), 2).is_empty());
        assert!(render_compact(&api(), &json!({"id": 1}), 2).is_empty());
    }

    #[test]
    fn render_state_diagram_progress() {
        assert_eq!(
            render_state_diagram(&api(), &change(), 3),
            [
                t("[bold cyan]state flow:[/] [bold]43%[/] complete  [dim](3/7)[/]"),
                t(
                    "[green]✓ Open[/] [dim]→[/] [green]✓ Plan \\[x][/] [dim]→[/] [green]✓ Approve[/] [dim]→[/] [bold yellow on blue] ▶ Build [/] [dim]→[/] [dim]Test[/] [dim]→[/] [dim]Done[/] [dim]→[/] [dim]Closed[/]"
                ),
            ]
        );
        let mut last = with_status(7);
        last["state_traversal"] = json!([1, 2, 3]);
        assert_eq!(
            render_state_diagram(&api(), &last, 10),
            [
                t("[bold cyan]state flow:[/] [bold]43%[/] complete  [dim](3/7)[/]"),
                t(
                    "[green]✓ Open[/] [dim]→[/] [green]✓ Plan \\[x][/] [dim]→[/] [green]✓ Approve[/] [dim]→[/] [dim]Build[/] [dim]→[/] [dim]Test[/] [dim]→[/] [dim]Done[/] [dim]→[/] [bold yellow on blue] ▶ Closed [/]"
                ),
            ]
        );
        assert!(render_state_diagram(&api(), &with_status(99), 3).is_empty());
        assert!(render_state_diagram(&FakeApi::new(), &change(), 3).is_empty());
    }
}
