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
    Table { title: String, rows: Vec<[String; 3]> },
}

/// `get_state_flow`: in the change's workspace when it has one (a 404 there
/// falls back to the plain path, e.g. accounts without workspaces).
pub fn get_state_flow(c: &dyn Api, state_flow_id: &Value, workspace_id: Option<&Value>) -> Result<Value> {
    let params = obj(json!({"include": "state_info"}));
    let flow = py_str(state_flow_id);
    if let Some(ws) = workspace_id.filter(|w| !w.is_null() && **w != json!("")) {
        match c.int_get(&format!("ws/{}/change_management/{flow}", py_str(ws)), Some(params.clone())) {
            Ok(data) => return Ok(unwrap_key(data, "state_flow")),
            Err(Error::Api(e)) if e.status == 404 => {}
            Err(e) => return Err(e),
        }
    }
    Ok(unwrap_key(c.int_get(&format!("change_management/{flow}"), Some(params))?, "state_flow"))
}

/// `_try_get_approvals`: the approvals list, or nothing on any failure.
pub fn try_get_approvals(c: &dyn Api, change_id: &Value) -> Vec<Value> {
    c.int_get(&format!("changes/{}/approvals", py_str(change_id)), None).map(|d| rows_at(&d, "approvals")).unwrap_or_default()
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
    get_state_flow(c, change.get("state_flow_id").unwrap_or(&Value::Null), change.get("workspace_id"))
}

/// `render_flow`: the whole flow with the current position, then approval gates.
pub fn render_flow(c: &dyn Api, change: &Value) -> Result<Vec<Printed>> {
    let mut out = Vec::new();
    if change.get_truthy("state_flow_id").is_none() {
        out.push(Printed::Text("[yellow]no state_flow_id on this change[/]".into()));
        return Ok(out);
    }
    let flow = flow_of(c, change)?;
    let state_list = flow.list_or_empty("state_list");
    if state_list.is_empty() {
        out.push(Printed::Text("[yellow]empty state_list[/]".into()));
        return Ok(out);
    }
    let current = change.get("status").cloned().unwrap_or(Value::Null);
    let current_name = change.get_truthy("status_name").map(py_str).unwrap_or_else(|| py_str(&current));
    let traversal = unique(change.list_or_empty("state_traversal"));
    let flow_name = flow.get("name").map(py_str).unwrap_or_else(|| "?".into());
    let hid = change.get_truthy("human_display_id").map(py_str).unwrap_or_else(|| format!("CHN-{}", change.get("id").map(py_str).unwrap_or("None".into())));
    let change_type = change.get("change_type").map(py_str).unwrap_or_else(|| "None".into());
    let mut rows = Vec::new();
    for s in state_list {
        let sid = s.get("id").cloned().unwrap_or(Value::Null);
        let name = s.get("value").map(py_str).unwrap_or_default();
        let visited = traversal.contains(&sid);
        let row = if sid == current {
            ["[bold yellow]▶[/]".into(), format!("[bold yellow]{name}[/]"), "[bold yellow]current[/]".into()]
        } else if visited {
            ["[green]✓[/]".into(), format!("[green]{name}[/]"), "[green]passed[/]".into()]
        } else {
            ["[dim]·[/]".into(), format!("[dim]{name}[/]"), "[dim]future[/]".into()]
        };
        rows.push(row);
    }
    out.push(Printed::Table { title: format!("{hid} flow — [bold]{flow_name}[/] (change_type={change_type})"), rows });
    let ids: Vec<Value> = state_list.iter().map(|s| s.get("id").cloned().unwrap_or(Value::Null)).collect();
    let passed_in_flow = traversal.iter().filter(|t| ids.contains(t)).count() as i64;
    let ahead = state_list.len() as i64 - passed_in_flow - 1;
    out.push(Printed::Text(format!("  current: [bold yellow]{current_name}[/] · past: {} · ahead: {ahead}", traversal.len())));
    let approvals = try_get_approvals(c, change.get("id").unwrap_or(&Value::Null));
    if !approvals.is_empty() {
        out.push(Printed::Blank);
        out.push(Printed::Text("[bold]Approval gates:[/]".into()));
        for a in &approvals {
            let level = a.get("level_id").map(py_str).unwrap_or_else(|| "None".into());
            let status = a.obj_or_empty("status").get("name").map(py_str).unwrap_or_else(|| "?".into());
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
            out.push(Printed::Text(format!("  {icon} L{level}: {member} — {status}")));
        }
    }
    Ok(out)
}

/// The states within `context` of the current one: `(lo, hi, cur_idx)`.
fn window(state_list: &[Value], current: &Value, context: usize) -> Option<(usize, usize, usize)> {
    let cur = state_list.iter().position(|s| s.get("id").unwrap_or(&Value::Null) == current)?;
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
    let Ok(flow) = flow_of(c, change) else { return vec![] };
    let state_list = flow.list_or_empty("state_list");
    if state_list.is_empty() {
        return vec![];
    }
    let current = change.get("status").cloned().unwrap_or(Value::Null);
    let traversal = unique(change.list_or_empty("state_traversal"));
    let total = state_list.len();
    let Some((lo, hi, _)) = window(state_list, &current, context) else { return vec![] };
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
    let hid = change.get("human_display_id").map(py_str).unwrap_or_default();
    vec![
        Printed::Text(format!("state:    {}  [dim][{}/{total}][/]", parts.join(" [dim]→[/] "), traversal.len())),
        Printed::Text(format!("[dim]          `fsv changes state {hid}` for full flow[/]")),
    ]
}

/// `render_state_diagram`: progress, then the states around the current one.
pub fn render_state_diagram(c: &dyn Api, change: &Value, context: usize) -> Vec<Printed> {
    if change.get_truthy("state_flow_id").is_none() {
        return vec![];
    }
    let Ok(flow) = flow_of(c, change) else { return vec![] };
    let state_list = flow.list_or_empty("state_list");
    if state_list.is_empty() {
        return vec![];
    }
    let current = change.get("status").cloned().unwrap_or(Value::Null);
    let traversal = unique(change.list_or_empty("state_traversal"));
    let Some((lo, hi, cur)) = window(state_list, &current, context) else { return vec![] };
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
        Printed::Text(format!("[bold cyan]state flow:[/] [bold]{pct}%[/] complete  [dim]({done}/{total})[/]")),
        Printed::Text(parts.join(" [dim]→[/] ")),
    ];
    if lo > 0 || hi < total - 1 {
        let hid = change.get("human_display_id").map(py_str).unwrap_or_else(|| "None".into());
        out.push(Printed::Text(format!("[dim]  use `fsv changes state {hid}` for full flow[/]")));
    }
    out
}
