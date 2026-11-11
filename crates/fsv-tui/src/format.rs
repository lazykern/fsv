//! Detail-pane text (`FsvApp._format_*`) and the `service.resolve_*`
//! helpers. Every server value goes through [`m`] (Rich escape).

use fsv_core::term::escape;
use serde_json::Value;

use crate::backend::Res;
use crate::site;

const PRIORITY: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High"), (4, "Urgent")];
const IMPACT: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High")];
const RISK: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High"), (4, "Very High")];

fn lookup(table: &[(i64, &str)], v: Option<&Value>) -> String {
    let n = v.and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
            .or_else(|| v.as_bool().map(i64::from))
    });
    n.and_then(|n| table.iter().find(|(k, _)| *k == n))
        .map_or("-", |(_, l)| l)
        .to_string()
}

/// `str(value)`, with `None` as `""` (`_markup` / `pages._e`).
pub fn s(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => fsv_py::py_str(v),
    }
}

/// `escape(str(value))`.
pub fn m(v: &str) -> String {
    escape(v)
}

/// `_markup`: `escape(str(value))`, `None` as `""`.
fn mv(v: Option<&Value>) -> String {
    escape(&s(v))
}

/// First `n` code points (Python `text[:n]`).
pub fn cut(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn slice(text: &str, from: usize, to: usize) -> String {
    text.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect()
}

/// `_clip`: longer than `width` → first `width - 1` chars + `…`.
pub fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        format!("{}…", cut(text, width.saturating_sub(1)))
    }
}

pub fn strip_html(v: &Value) -> String {
    fsv_core::output::strip_html(v.as_str())
}

fn truthy_get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).filter(|x| fsv_py::truthy(x))
}

fn obj_name(v: Option<&Value>) -> Option<&Value> {
    v.filter(|x| x.is_object()).and_then(|x| x.get("name"))
}

/// `v1 or v2 or ... or ""` over keys.
fn first_str(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| truthy_get(v, k))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

/// `service.resolve_status`.
pub fn resolve_status(item: &Value, schema: &Value) -> String {
    if let Some(v) = truthy_get(item, "status_name") {
        return fsv_py::py_str(v);
    }
    for k in ["change_status", "ticket_status", "problem_status"] {
        if let Some(n) = truthy_get(item, k)
            .and_then(|o| o.get("name"))
            .filter(|x| fsv_py::truthy(x))
        {
            return fsv_py::py_str(n);
        }
    }
    let sid = item.get("status").cloned().unwrap_or(Value::Null);
    let label = site::choice_label("status", &sid, schema);
    if !label.is_empty() {
        label
    } else {
        fsv_py::py_str(&sid)
    }
}

/// `service.resolve_priority`.
pub fn resolve_priority(item: &Value) -> String {
    let p = truthy_get(item, "priority")
        .cloned()
        .unwrap_or(Value::from(0));
    lookup(PRIORITY, Some(&p))
}

fn field_label(name: &str, schema: &Value) -> Option<String> {
    schema
        .get("fields")?
        .as_array()?
        .iter()
        .find(|f| f.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|f| truthy_get(f, "label"))
        .map(fsv_py::py_str)
}

/// Python `v not in (None, "", [], False)` (`0 == False`).
fn visible_value(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::Object(_) => true,
    }
}

/// `_format_details`: the details tab when no page is loaded.
pub fn format_details(item: &Value, res: Res, schema: &Value) -> String {
    let mut lines: Vec<String> = vec![String::new(), "[dim]── CORE ──[/]".into()];
    lines.push(format!(
        "  [cyan]Module[/]       {}",
        m(&res.name.to_uppercase())
    ));
    lines.push(format!(
        "  [cyan]ID[/]           {}",
        m(&fsv_core::resource::format_id(item, res))
    ));
    let subject = item.get("subject").map_or(String::new(), |v| s(Some(v)));
    lines.push(format!("  [cyan]Subject[/]      {}", m(&subject)));
    let desc = item.get("description").map(strip_html).unwrap_or_default();
    if !desc.is_empty() {
        lines.push(format!("  [cyan]Description[/]  {}", m(&cut(&desc, 200))));
        if desc.chars().count() > 200 {
            lines.push(format!("               {}", m(&slice(&desc, 200, 500))));
        }
    }
    lines.push(format!(
        "  [cyan]Status[/]       [green]{}[/]",
        m(&resolve_status(item, schema))
    ));
    lines.push(format!(
        "  [cyan]Priority[/]     {}",
        m(&resolve_priority(item))
    ));
    if res.name == "changes" {
        let ct = site::choice_label(
            "change_type",
            item.get("change_type").unwrap_or(&Value::Null),
            schema,
        );
        lines.push(format!("  [cyan]Type[/]         {}", m(&ct)));
        lines.push(format!(
            "  [cyan]Risk[/]         {}",
            m(&lookup(RISK, item.get("risk")))
        ));
    } else if res.name == "tickets" {
        let t = item.get("type").map_or("-".to_string(), |v| s(Some(v)));
        lines.push(format!("  [cyan]Type[/]         {}", m(&t)));
    }
    if item.get("impact").is_some_and(|v| !v.is_null()) {
        lines.push(format!(
            "  [cyan]Impact[/]       {}",
            m(&lookup(IMPACT, item.get("impact")))
        ));
    }
    if truthy_get(item, "planned_start_date").is_some()
        || truthy_get(item, "planned_end_date").is_some()
    {
        let start = item
            .get("planned_start_date")
            .map_or("-".to_string(), |v| s(Some(v)));
        let end = item
            .get("planned_end_date")
            .map_or("-".to_string(), |v| s(Some(v)));
        lines.push(format!(
            "  [cyan]Planned[/]      {}  →  {}",
            m(&start),
            m(&end)
        ));
    }
    lines.push(String::new());
    lines.push("[dim]── REQUEST ──[/]".into());
    let name_or_dash = |v: Option<&Value>| {
        if v.is_some_and(Value::is_object) {
            mv(obj_name(v))
        } else {
            "-".to_string()
        }
    };
    lines.push(format!(
        "  [cyan]Requester[/]    {}",
        name_or_dash(item.get("requester"))
    ));
    let agent = truthy_get(item, "agent").or_else(|| item.get("responder"));
    lines.push(format!("  [cyan]Agent[/]        {}", name_or_dash(agent)));
    lines.push(format!(
        "  [cyan]Group[/]        {}",
        name_or_dash(item.get("group"))
    ));
    if let Some(n) = item
        .get("department")
        .filter(|d| d.is_object())
        .and_then(|d| truthy_get(d, "name"))
    {
        lines.push(format!("  [cyan]Department[/]   {}", mv(Some(n))));
    }
    let cf = truthy_get(item, "custom_fields").and_then(Value::as_object);
    let visible: Vec<(&String, &Value)> = cf
        .into_iter()
        .flatten()
        .filter(|(_, v)| visible_value(v))
        .collect();
    if !visible.is_empty() {
        lines.push(String::new());
        lines.push("[dim]── CUSTOM FIELDS ──[/]".into());
        for (k, v) in visible {
            let label = field_label(k, schema).unwrap_or_else(|| k.clone());
            let text = match v {
                Value::Array(a) => a.iter().map(fsv_py::py_str).collect::<Vec<_>>().join(", "),
                Value::Object(_) => truthy_get(v, "name")
                    .or_else(|| truthy_get(v, "value"))
                    .map_or_else(|| fsv_py::py_str(v), fsv_py::py_str),
                other => fsv_py::py_str(other),
            };
            lines.push(format!("  [cyan]{}[/]  {}", m(&label), m(&text)));
        }
    }
    if let Some(pf) = truthy_get(item, "planning_fields").and_then(Value::as_object) {
        lines.push(String::new());
        lines.push("[dim]── PLANNING FIELDS ──[/]".into());
        for (k, v) in pf {
            if v.is_object() {
                let d = first_str(v, &["description_text", "name"]);
                if !d.is_empty() {
                    lines.push(format!("  [cyan]{}[/]  {}", m(k), m(&cut(&d, 200))));
                }
            }
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn rows(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

pub fn format_notes(notes: &Value) -> String {
    let notes = rows(notes);
    if notes.is_empty() {
        return "\n[dim]no notes[/]\n".into();
    }
    let mut lines = vec![String::new()];
    for note in notes {
        let user = note
            .get("user")
            .filter(|u| u.is_object())
            .map(|u| first_str(u, &["name"]))
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "?".into());
        let created = cut(&first_str(note, &["created_at"]), 19);
        let lock = if note.get("private").is_some_and(fsv_py::truthy) {
            " [yellow]🔒[/]"
        } else {
            ""
        };
        lines.push(format!("  [bold]{}[/]{lock}", m(&user)));
        lines.push(format!("  [dim]{}[/]", m(&created)));
        let body = strip_html(&Value::String(first_str(note, &["body", "body_text"])));
        if !body.is_empty() {
            for line in cut(&body, 500).split('\n') {
                lines.push(format!("    {}", m(line)));
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn format_tasks(tasks: &Value) -> String {
    let tasks = rows(tasks);
    if tasks.is_empty() {
        return "\n[dim]no tasks[/]\n".into();
    }
    let mut lines = vec![String::new()];
    for task in tasks {
        let tid = task.get("id").map_or("?".into(), |v| s(Some(v)));
        let title = Some(first_str(task, &["title", "description"]))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "-".into());
        let raw = truthy_get(task, "status_name")
            .or_else(|| task.get("status"))
            .cloned()
            .unwrap_or(Value::Null);
        let status = site::task_status_display(&raw);
        let name_of = |k: &str| {
            task.get(k)
                .filter(|x| x.is_object())
                .map(|x| first_str(x, &["name"]))
                .unwrap_or_default()
        };
        let (group, agent) = (name_of("group"), name_of("agent"));
        let due = cut(&first_str(task, &["due_date"]), 10);
        let icon = if status == "Completed" { "✓" } else { "○" };
        let color = match status.as_str() {
            "Completed" => "green",
            "In Progress" => "yellow",
            _ => "white",
        };
        lines.push(format!(
            "  [{color}]{icon}[/] [bold]#TSK-{}[/] {}",
            m(&tid),
            m(&title)
        ));
        let mut parts: Vec<String> = Vec::new();
        if !group.is_empty() {
            parts.push(group);
        }
        if !agent.is_empty() {
            parts.push(agent);
        }
        if !due.is_empty() {
            parts.push(format!("Due: {due}"));
        }
        lines.push(format!(
            "    [{color}]{}[/]  [dim]{}[/]",
            m(&status),
            m(&parts.join(" / "))
        ));
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn format_approvals(approvals: &Value) -> String {
    let approvals = rows(approvals);
    if approvals.is_empty() {
        return "\n[dim]no approvals[/]\n".into();
    }
    let mut lines = vec![String::new()];
    for a in approvals {
        let member = a
            .get("member")
            .filter(|x| x.is_object())
            .map(|x| first_str(x, &["name"]))
            .filter(|x| !x.is_empty())
            .unwrap_or_else(|| a.get("member_id").map_or("?".into(), fsv_py::py_str));
        let status = match truthy_get(a, "status") {
            None => String::new(),
            Some(o @ Value::Object(_)) => s(o.get("name")),
            Some(v) => fsv_py::py_str(v),
        };
        let level = a.get("level_id").map_or("?".into(), |v| s(Some(v)));
        let created = cut(&first_str(a, &["created_at"]), 19);
        let icon = match status.to_lowercase().as_str() {
            "approved" => "[green]✓[/]",
            "rejected" => "[red]✗[/]",
            _ => "[yellow]⏳[/]",
        };
        lines.push(format!("  {icon} [bold]{}[/]  L{}", m(&member), m(&level)));
        lines.push(format!("    {}  [dim]{}[/]", m(&status), m(&created)));
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn format_assets(assets: &Value) -> String {
    let assets = rows(assets);
    if assets.is_empty() {
        return "\n[dim]no assets[/]\n".into();
    }
    let mut lines = vec![String::new()];
    for asset in assets {
        let item = asset
            .get("config_item")
            .filter(|c| c.is_object())
            .unwrap_or(asset);
        let name = Some(first_str(item, &["name", "display_name"]))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "-".into());
        let did = first_str(item, &["display_id", "id"]);
        let atype = first_str(item, &["asset_type", "ci_type", "ci_type_name"]);
        let dept = item
            .get("department")
            .filter(|d| d.is_object())
            .map(|d| first_str(d, &["name"]))
            .unwrap_or_default();
        let location = first_str(item, &["location_name", "location"]);
        lines.push(format!("  [bold]{}[/]", m(&name)));
        let mut parts = Vec::new();
        if !did.is_empty() {
            parts.push(format!("ID: {did}"));
        }
        if !atype.is_empty() {
            parts.push(format!("Type: {atype}"));
        }
        if !dept.is_empty() {
            parts.push(format!("Dept: {dept}"));
        }
        if !location.is_empty() {
            parts.push(format!("Location: {location}"));
        }
        if !parts.is_empty() {
            lines.push(format!("    [dim]{}[/]", m(&parts.join(" · "))));
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn format_associations(data: &Value) -> String {
    let Some(map) = data.as_object().filter(|o| !o.is_empty()) else {
        return "\n[dim]no associations[/]\n".into();
    };
    let mut lines = vec![String::new()];
    lines.extend(
        rows(map.get("warnings").unwrap_or(&Value::Null))
            .iter()
            .map(|w| m(&fsv_py::py_str(w))),
    );
    for (key, items) in map {
        if matches!(key.as_str(), "meta" | "warnings" | "complete") {
            continue;
        }
        let Some(items) = items.as_array().filter(|a| !a.is_empty()) else {
            continue;
        };
        lines.push(format!("  [dim]── {} ──[/]", m(&key.to_uppercase())));
        for item in items {
            let did = first_str(item, &["human_display_id", "display_id", "id"]);
            let subject = cut(&first_str(item, &["subject"]), 60);
            let status = Some(first_str(item, &["status_name"]))
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| item.get("status").map_or(String::new(), fsv_py::py_str));
            let pri = lookup(
                PRIORITY,
                Some(item.get("priority").unwrap_or(&Value::from(0))),
            );
            lines.push(format!("    [cyan]{}[/]  {}", m(&did), m(&subject)));
            lines.push(format!(
                "      Status: {}  Priority: {}",
                m(&status),
                m(&pri)
            ));
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn format_activity(activities: &Value) -> String {
    let acts = rows(activities);
    if acts.is_empty() {
        return "\n[dim]no activity[/]\n".into();
    }
    let mut lines = vec![String::new()];
    let mut current = String::new();
    for act in acts {
        let created = first_str(act, &["created_at"]);
        let date = cut(&created, 10);
        let time = if created.chars().count() > 16 {
            slice(&created, 11, 16)
        } else {
            String::new()
        };
        if date != current {
            current = date.clone();
            lines.push(format!("  [dim]── {} ──[/]", m(&date)));
        }
        let actor = match act.get("actor") {
            Some(a @ Value::Object(_)) => s(a.get("name")),
            _ => "System".into(),
        };
        let content = strip_html(&Value::String(first_str(act, &["content", "note_content"])));
        lines.push(format!("  [dim]{}[/]  [bold]{}[/]", m(&time), m(&actor)));
        if !content.is_empty() {
            for line in cut(&content, 300).split('\n') {
                lines.push(format!("         {}", m(line)));
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn format_resolution(item: &Value) -> String {
    let mut lines = vec![String::new()];
    let resolved = item
        .get("stats")
        .and_then(|st| truthy_get(st, "resolved_at"))
        .or_else(|| truthy_get(item, "resolved_at"));
    if let Some(r) = resolved {
        lines.push(format!("  [cyan]Resolved at[/]  {}", mv(Some(r))));
    }
    let resolution = first_str(item, &["resolution_notes"]);
    if !resolution.is_empty() {
        lines.push(String::new());
        lines.push("  [dim]── Resolution Note ──[/]".into());
        for line in strip_html(&Value::String(resolution.clone())).split('\n') {
            lines.push(format!("    {}", m(line)));
        }
    }
    if resolved.is_none() && resolution.is_empty() {
        lines.push("  [dim]no resolution[/]".into());
    }
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup::plain;
    use serde_json::json;

    #[test]
    fn formatters_escape_markup() {
        let notes =
            json!([{"user": {"name": "[bold]x[/]"}, "body": "see [red]this[/]", "private": true}]);
        let out = plain(&format_notes(&notes));
        assert!(out.contains("[bold]x[/]"), "{out}");
        assert!(out.contains("see [red]this[/]"));
        let item = json!({"id": 5, "display_id": 5, "subject": "[P4] disk [b]full[/b]"});
        let out = plain(&format_details(
            &item,
            &fsv_core::resource::TICKETS,
            &json!({"fields": []}),
        ));
        assert!(out.contains("[P4] disk [b]full[/b]"), "{out}");
    }

    #[test]
    fn clip_and_empty_sections() {
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
        assert_eq!(format_tasks(&json!([])), "\n[dim]no tasks[/]\n");
        assert!(plain(&format_resolution(&json!({}))).contains("no resolution"));
    }

    // Expected strings: `FsvApp._format_*` / `_markup` run on the same input.

    #[test]
    fn markup_escapes_str_of_value() {
        assert_eq!(mv(None), "");
        assert_eq!(mv(Some(&json!("[b]x"))), "\\[b]x");
        assert_eq!(mv(Some(&json!(3))), "3");
    }

    #[test]
    fn format_approvals_icons_member_level_created() {
        let a = json!([
            {"member": {"name": "Ann [x]"}, "status": {"name": "Approved"}, "level_id": 2, "created_at": "2026-01-02T03:04:05Z"},
            {"member_id": 77, "status": "rejected", "created_at": null},
            {"member": {}, "status": {"name": "Requested"}, "level_id": 1}
        ]);
        assert_eq!(
            format_approvals(&a),
            "\n  [green]✓[/] [bold]Ann \\[x][/]  L2\n    Approved  [dim]2026-01-02T03:04:05[/]\n\n  [red]✗[/] [bold]77[/]  L?\n    rejected  [dim][/]\n\n  [yellow]⏳[/] [bold]?[/]  L1\n    Requested  [dim][/]\n"
        );
        assert_eq!(format_approvals(&json!([])), "\n[dim]no approvals[/]\n");
    }

    #[test]
    fn format_assets_config_item_and_parts() {
        let a = json!([
            {"config_item": {"name": "Laptop [A]", "display_id": 5, "asset_type": "Hardware", "department": {"name": "IT"}, "location_name": "HQ"}},
            {"display_name": "Server", "id": 9, "ci_type_name": "CI"},
            {}
        ]);
        assert_eq!(
            format_assets(&a),
            "\n  [bold]Laptop [A][/]\n    [dim]ID: 5 · Type: Hardware · Dept: IT · Location: HQ[/]\n\n  [bold]Server[/]\n    [dim]ID: 9 · Type: CI[/]\n\n  [bold]-[/]\n"
        );
        assert_eq!(format_assets(&json!([])), "\n[dim]no assets[/]\n");
    }

    #[test]
    fn format_associations_sections_skip_meta() {
        let d = json!({"warnings": ["partial [list]"], "meta": {"x": 1}, "complete": true,
            "Tickets": [{"human_display_id": "INC-1", "subject": "S".repeat(70), "status_name": "Open", "priority": 3},
                        {"display_id": 2, "status": 2}],
            "Problems": [], "Other": "x"});
        assert_eq!(
            format_associations(&d),
            format!(
                "\npartial \\[list]\n  [dim]── TICKETS ──[/]\n    [cyan]INC-1[/]  {}\n      Status: Open  Priority: High\n    [cyan]2[/]  \n      Status: 2  Priority: -\n",
                "S".repeat(60)
            )
        );
        assert_eq!(
            format_associations(&json!({})),
            "\n[dim]no associations[/]\n"
        );
    }

    #[test]
    fn format_activity_groups_by_day() {
        let a = json!([
            {"created_at": "2026-01-02T03:04:05Z", "actor": {"name": "Bob"}, "content": "<p>did [thing]</p>\nline2"},
            {"created_at": "2026-01-02T09:10:11Z", "note_content": "n"},
            {"created_at": "2026-01-03", "actor": "x"}
        ]);
        assert_eq!(
            format_activity(&a),
            "\n  [dim]── 2026-01-02 ──[/]\n  [dim]03:04[/]  [bold]Bob[/]\n         did \\[thing] \n         line2\n\n  [dim]09:10[/]  [bold]System[/]\n         n\n\n  [dim]── 2026-01-03 ──[/]\n  [dim][/]  [bold]System[/]\n"
        );
        assert_eq!(format_activity(&json!([])), "\n[dim]no activity[/]\n");
    }
}
