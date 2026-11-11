//! `render.py`: the classic list table and detail panel, requested-item
//! answers, and the output blocks both page styles print through.

use serde_json::{Map, Value, json};

use fsv_api::{IMPACT, PRIORITY, RISK, choice_label, field};
use fsv_core::output::{fmt_dt, fmt_value, strip_html};
use fsv_core::pystr;
use fsv_core::resource::{self, Resource};
use fsv_core::term::{self, Column, Console, Overflow, Table, cell_len, escape, strip_markup};

use crate::source::name_of;

mod flow;
pub use flow::{change_flow, state_label};

#[cfg(test)]
mod coverage;

/// One thing `console.print` / `console.rule` would emit.
#[derive(Debug, Clone)]
pub enum Block {
    /// Rich markup; may hold several lines.
    Markup(String),
    Blank,
    /// `console.rule(title)`: centered, default rule style.
    Rule(String),
    /// `console.rule(style="dim")`.
    DimRule,
    /// `console.rule(title, align="left", style="dim")`.
    LeftRule(String),
    Table(Table),
    /// Details tab: two columns (3:2) when the console is at least 100 wide,
    /// otherwise `main`, a blank line, then `side`.
    Columns {
        main: String,
        side: String,
    },
}

/// Width at which the Details tab switches to two columns.
pub const GRID_MIN_WIDTH: usize = 100;

fn rule_chars(n: usize) -> String {
    "─".repeat(n)
}

/// Width-dependent blocks as markup, blank lines, centered rules and tables.
fn lower(block: &Block, width: usize) -> Vec<Block> {
    match block {
        Block::DimRule => vec![Block::Markup(format!("[dim]{}[/]", rule_chars(width)))],
        Block::LeftRule(title) => {
            let used = cell_len(&strip_markup(title)) + 1;
            vec![Block::Markup(format!(
                "{title} [dim]{}[/]",
                rule_chars(width.saturating_sub(used))
            ))]
        }
        Block::Columns { main, side } if width >= GRID_MIN_WIDTH => {
            // Rich `Table.grid(expand=True, padding=(0, 2))`: the 3:2 split includes the
            // collapsed 2-cell gap, which here is the 1+1 padding around the column edge.
            let main_width = (3 * width).div_ceil(5);
            let mut grid = Table::new().no_box().show_header(false).pad_edge(false);
            grid.add_column(
                Column::new("")
                    .width(main_width - 2)
                    .overflow(Overflow::Fold),
            );
            grid.add_column(
                Column::new("")
                    .width(width - main_width)
                    .overflow(Overflow::Fold),
            );
            grid.add_row([main.clone(), side.clone()]);
            vec![Block::Table(grid)]
        }
        Block::Columns { main, side } => vec![
            Block::Markup(main.clone()),
            Block::Blank,
            Block::Markup(side.clone()),
        ],
        other => vec![other.clone()],
    }
}

/// Render blocks to text lines at `width` (plain when `color` is false).
pub fn blocks_lines(blocks: &[Block], width: usize, color: bool) -> Vec<String> {
    let mut out = Vec::new();
    for b in blocks.iter().flat_map(|b| lower(b, width)) {
        match b {
            Block::Markup(m) if color => {
                out.extend(term::out().render(&m).split('\n').map(str::to_string))
            }
            Block::Markup(m) => out.extend(strip_markup(&m).split('\n').map(str::to_string)),
            Block::Blank => out.push(String::new()),
            Block::Rule(title) => out.push(term::rule_line(
                &title,
                width,
                color,
                term::Style::default(),
            )),
            Block::Table(t) => out.extend(t.render_lines(width, color)),
            Block::DimRule | Block::LeftRule(_) | Block::Columns { .. } => {}
        }
    }
    out
}

/// Plain text of `blocks` (one string, newline-terminated lines).
pub fn blocks_text(blocks: &[Block], width: usize) -> String {
    blocks_lines(blocks, width, false)
        .into_iter()
        .map(|l| l + "\n")
        .collect()
}

/// Print `blocks` on `console`.
pub fn print_blocks(console: &Console, blocks: &[Block]) {
    let width = console.width();
    for b in blocks.iter().flat_map(|b| lower(b, width)) {
        match b {
            Block::Markup(m) => console.print(&m),
            Block::Blank => console.blank(),
            Block::Rule(title) => console.rule(&title),
            Block::Table(t) => console.print_table(&t),
            Block::DimRule | Block::LeftRule(_) | Block::Columns { .. } => {}
        }
    }
}

// --------------------------------------------------------------------------
// value helpers

fn truthy_str(v: Option<&Value>) -> Option<String> {
    v.filter(|v| fsv_py::truthy(v)).map(fsv_py::py_str)
}

fn obj_get<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    v.and_then(Value::as_object).and_then(|m| m.get(key))
}

fn lookup(table: &[(i64, &'static str)], v: Option<&Value>) -> Option<&'static str> {
    let n = match v? {
        Value::Bool(b) => i64::from(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i,
            None => {
                let f = n.as_f64()?;
                if f.fract() != 0.0 {
                    return None;
                }
                f as i64
            }
        },
        _ => return None,
    };
    table.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
}

/// `value in (None, "", [], False)` (`0 == False` in Python).
fn blank_answer(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::Object(_) => false,
    }
}

fn schema_or_empty(schema: Option<&Value>) -> Value {
    schema.cloned().unwrap_or_else(|| json!({}))
}

// --------------------------------------------------------------------------
// classic list

const CUSTOM_FIELD_CATEGORY_ORDER: [&str; 6] =
    ["choices", "text", "dates", "numbers", "booleans", "other"];

fn custom_field_category(name: &str, schema: &Value) -> &'static str {
    let ftype = field(name, schema)
        .and_then(|f| f.get("field_type"))
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .map(|s| pystr::casefold(&s))
        .unwrap_or_default();
    let has = |tokens: &[&str]| tokens.iter().any(|t| ftype.contains(t));
    if has(&["dropdown", "choice", "nested", "lookup"]) {
        "choices"
    } else if has(&["text", "paragraph", "description"]) {
        "text"
    } else if has(&["date", "time"]) {
        "dates"
    } else if has(&["number", "decimal", "integer"]) {
        "numbers"
    } else if has(&["checkbox", "boolean"]) {
        "booleans"
    } else {
        "other"
    }
}

fn resolve_status_name(x: &Value, sid: &Value, schema: &Value) -> String {
    if let Some(s) = truthy_str(x.get("status_name")) {
        return s;
    }
    for key in ["change_status", "ticket_status", "problem_status"] {
        if let Some(s) = truthy_str(obj_get(x.get(key), "name")) {
            return s;
        }
    }
    choice_label("status", sid, schema)
}

fn requester_cell(x: &Value, missing: &str) -> String {
    match x.get("requester") {
        Some(Value::Object(r)) => truthy_str(r.get("name")).unwrap_or_else(|| missing.to_string()),
        _ => missing.to_string(),
    }
}

fn priority_cell(v: Option<&Value>) -> &'static str {
    let v = v.filter(|v| fsv_py::truthy(v)).cloned().unwrap_or(json!(0));
    lookup(PRIORITY, Some(&v)).unwrap_or("-")
}

/// `list_rows`: the `--csv`/`--tsv`/`--json` rows of the classic list.
pub fn list_rows(
    items: &[Value],
    res: &Resource,
    schema: Option<&Value>,
) -> Vec<Map<String, Value>> {
    let schema = schema_or_empty(schema);
    items
        .iter()
        .map(|x| {
            let sid = x.get("status").cloned().unwrap_or(Value::Null);
            let mut row = Map::new();
            row.insert("id".into(), json!(resource::format_id(x, res)));
            row.insert(
                "status".into(),
                json!(resolve_status_name(x, &sid, &schema)),
            );
            row.insert("status_id".into(), sid);
            row.insert("priority".into(), json!(priority_cell(x.get("priority"))));
            row.insert(
                "subject".into(),
                x.get("subject")
                    .filter(|v| fsv_py::truthy(v))
                    .cloned()
                    .unwrap_or(json!("")),
            );
            let requester = match x.get("requester") {
                Some(Value::Object(r)) => r.get("name").filter(|v| fsv_py::truthy(v)).cloned(),
                _ => None,
            };
            row.insert("requester".into(), requester.unwrap_or(json!("")));
            if res.name == "tickets" {
                row.insert(
                    "type".into(),
                    x.get("type")
                        .filter(|v| fsv_py::truthy(v))
                        .cloned()
                        .unwrap_or(json!("")),
                );
            } else if res.name == "changes" {
                row.insert(
                    "type".into(),
                    json!(choice_label(
                        "change_type",
                        x.get("change_type").unwrap_or(&Value::Null),
                        &schema
                    )),
                );
            }
            row
        })
        .collect()
}

/// `render.list_table`: ID, Status (with id), Type, Pri, Subject, Requester.
/// Cells are passed through as markup, like Rich's `add_row(str)`.
pub fn list_table(items: &[Value], res: &Resource, schema: Option<&Value>) -> Table {
    let schema = schema_or_empty(schema);
    let mut t = Table::new();
    t.add_column(Column::new("ID").style("cyan").no_wrap());
    t.add_column(Column::new("Status").style("green"));
    if res.name == "tickets" || res.name == "changes" {
        t.add_column(Column::new("Type"));
    }
    t.add_column(Column::new("Pri"));
    t.add_column(Column::new("Subject"));
    t.add_column(Column::new("Requester").style("dim"));
    for x in items {
        let sid = x.get("status").cloned().unwrap_or(Value::Null);
        let status = resolve_status_name(x, &sid, &schema);
        let mut row = vec![
            resource::format_id(x, res),
            format!("{status} ({})", fsv_py::py_str(&sid)),
        ];
        if res.name == "tickets" {
            row.push(truthy_str(x.get("type")).unwrap_or_else(|| "-".into()));
        } else if res.name == "changes" {
            let label = choice_label(
                "change_type",
                x.get("change_type").unwrap_or(&Value::Null),
                &schema,
            );
            row.push(if label.is_empty() { "-".into() } else { label });
        }
        row.push(priority_cell(x.get("priority")).to_string());
        row.push(truthy_str(x.get("subject")).unwrap_or_default());
        row.push(requester_cell(x, "-"));
        t.add_row(row);
    }
    t
}

// --------------------------------------------------------------------------
// state flow (`state_flow.render_state_diagram`, flow already fetched)

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if !a.is_boolean() && !b.is_boolean() => x == y,
        _ => a == b,
    }
}

/// The compact state diagram the classic detail prints for a change.
/// `flow` is the `get_state_flow` payload; nothing is drawn without it.
pub fn state_diagram(change: &Value, flow: Option<&Value>, context: usize) -> Vec<Block> {
    if !change.get("state_flow_id").is_some_and(fsv_py::truthy) {
        return Vec::new();
    }
    let Some(states) = flow
        .and_then(|f| f.get("state_list"))
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty())
    else {
        return Vec::new();
    };
    let current = change.get("status").cloned().unwrap_or(Value::Null);
    let mut traversal: Vec<Value> = Vec::new();
    for v in change
        .get("state_traversal")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !traversal.iter().any(|t| same(t, v)) {
            traversal.push(v.clone());
        }
    }
    let id_of = |s: &Value| s.get("id").cloned().unwrap_or(Value::Null);
    let Some(cur_idx) = states.iter().position(|s| same(&id_of(s), &current)) else {
        return Vec::new();
    };
    let total = states.len();
    let done = traversal.len();
    let pct = pystr::round_half_even(done as f64 / total as f64 * 100.0) as i64;
    let lo = cur_idx.saturating_sub(context);
    let hi = (cur_idx + context).min(total - 1);
    let mut parts = Vec::new();
    if lo > 0 {
        parts.push("[dim]…[/]".to_string());
    }
    for (i, state) in states[lo..=hi].iter().enumerate() {
        let name = escape(&state.get("value").map(fsv_py::py_str).unwrap_or_default());
        let sid = id_of(state);
        if lo + i == cur_idx {
            parts.push(format!("[bold yellow on blue] ▶ {name} [/]"));
        } else if traversal.iter().any(|t| same(t, &sid)) {
            parts.push(format!("[green]✓ {name}[/]"));
        } else {
            parts.push(format!("[dim]{name}[/]"));
        }
    }
    if hi < total - 1 {
        parts.push("[dim]…[/]".to_string());
    }
    let mut out = vec![
        Block::Markup(format!(
            "[bold cyan]state flow:[/] [bold]{pct}%[/] complete  [dim]({done}/{total})[/]"
        )),
        Block::Markup(parts.join(" [dim]→[/] ")),
    ];
    if lo > 0 || hi < total - 1 {
        let hid = change
            .get("human_display_id")
            .map(fsv_py::py_str)
            .unwrap_or_else(|| "None".into());
        out.push(Block::Markup(format!(
            "[dim]  use `fsv changes state {hid}` for full flow[/]"
        )));
    }
    out
}

// --------------------------------------------------------------------------
// classic detail

fn party_display(
    obj: Option<&Value>,
    id_value: Option<&Value>,
    fallback_name: Option<&Value>,
) -> String {
    let id_value = id_value.filter(|v| !v.is_null());
    if let Some(Value::Object(o)) = obj {
        let name = ["name", "email", "value"]
            .iter()
            .find_map(|k| o.get(*k).filter(|v| fsv_py::truthy(v)));
        let obj_id = o.get("id").filter(|v| fsv_py::truthy(v)).or(id_value);
        if let (Some(name), Some(id)) = (name, obj_id.filter(|v| !v.is_null())) {
            return format!("{} ({})", fsv_py::py_str(name), fsv_py::py_str(id));
        }
        if let Some(name) = name {
            return fsv_py::py_str(name);
        }
    }
    let fallback = fallback_name.filter(|v| fsv_py::truthy(v));
    match (fallback, id_value) {
        (Some(f), Some(id)) => format!("{} ({})", fsv_py::py_str(f), fsv_py::py_str(id)),
        (Some(f), None) => fsv_py::py_str(f),
        _ => id_value
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_else(|| "-".into()),
    }
}

/// The classic `fsv <resource> get` panel. `flow` feeds the change state diagram.
pub fn detail_panel(
    item: &Value,
    res: &Resource,
    schema: Option<&Value>,
    flow: Option<&Value>,
) -> Vec<Block> {
    let schema = schema_or_empty(schema);
    let mut out = Vec::new();
    let subject = item.get("subject").map(fsv_py::py_str).unwrap_or_default();
    out.push(Block::Rule(format!(
        "[bold cyan]{}[/]  {subject}",
        resource::format_id(item, res)
    )));
    let sid = item.get("status").cloned().unwrap_or(Value::Null);
    let status = truthy_str(item.get("status_name"))
        .unwrap_or_else(|| choice_label("status", &sid, &schema));
    out.push(Block::Markup(format!(
        "[green]status[/]    {status}  (id={})",
        fsv_py::py_str(&sid)
    )));
    if res.name == "changes" {
        out.extend(state_diagram(item, flow, 3));
        let ct = choice_label(
            "change_type",
            item.get("change_type").unwrap_or(&Value::Null),
            &schema,
        );
        out.push(Block::Markup(format!("type      {ct}")));
        out.push(Block::Markup(format!(
            "risk      {}",
            lookup(RISK, item.get("risk")).unwrap_or("-")
        )));
    }
    if res.name == "tickets" {
        let typ = item
            .get("type")
            .map(fsv_py::py_str)
            .unwrap_or_else(|| "-".into());
        out.push(Block::Markup(format!("type      {typ}")));
        if let Some(dept) = truthy_str(obj_get(
            item.get("department").filter(|v| fsv_py::truthy(v)),
            "name",
        )) {
            out.push(Block::Markup(format!("department {dept}")));
        }
    }
    out.push(Block::Markup(format!(
        "priority  {}",
        lookup(PRIORITY, item.get("priority")).unwrap_or("-")
    )));
    if item.get("impact").is_some_and(|v| !v.is_null()) {
        out.push(Block::Markup(format!(
            "impact    {}",
            lookup(IMPACT, item.get("impact")).unwrap_or("-")
        )));
    }
    let get = |k: &str| item.get(k).unwrap_or(&Value::Null);
    let has = |k: &str| item.get(k).is_some_and(fsv_py::truthy);
    if has("planned_start_date") || has("planned_end_date") {
        out.push(Block::Markup(format!(
            "planned   {}  →  {}",
            fmt_dt(get("planned_start_date")),
            fmt_dt(get("planned_end_date"))
        )));
    }
    if has("created_at") || has("updated_at") {
        out.push(Block::Markup(format!(
            "created   {}  updated {}",
            fmt_dt(get("created_at")),
            fmt_dt(get("updated_at"))
        )));
    }
    let agent_obj = item
        .get("agent")
        .filter(|v| fsv_py::truthy(v))
        .or_else(|| item.get("responder"));
    let agent_id = item
        .get("agent_id")
        .filter(|v| fsv_py::truthy(v))
        .or_else(|| item.get("responder_id"));
    out.push(Block::Markup(format!(
        "requester {}  agent {}  group {}",
        party_display(item.get("requester"), item.get("requester_id"), None),
        party_display(agent_obj, agent_id, item.get("owner_name")),
        party_display(
            item.get("group"),
            item.get("group_id"),
            item.get("group_name")
        ),
    )));
    let description = strip_html(item.get("description").and_then(Value::as_str));
    if !description.is_empty() {
        out.push(Block::Blank);
        out.push(Block::Rule("[dim]description[/]".into()));
        out.push(Block::Markup(description));
    }
    if let Some(cf) = item
        .get("custom_fields")
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_object)
    {
        out.push(Block::Blank);
        out.push(Block::Rule("[dim]custom_fields[/]".into()));
        let mut keys: Vec<(&String, &Value)> =
            cf.iter().filter(|(_, v)| !blank_answer(v)).collect();
        keys.sort_by(|a, b| a.0.cmp(b.0));
        let mut grouped: Vec<(&str, Vec<(String, &Value)>)> = Vec::new();
        for (k, v) in keys {
            let label = truthy_str(field(k, &schema).and_then(|f| f.get("label")))
                .unwrap_or_else(|| k.clone());
            let cat = custom_field_category(k, &schema);
            match grouped.iter_mut().find(|(c, _)| *c == cat) {
                Some((_, rows)) => rows.push((label, v)),
                None => grouped.push((cat, vec![(label, v)])),
            }
        }
        let mut first = true;
        for category in CUSTOM_FIELD_CATEGORY_ORDER {
            let Some((_, rows)) = grouped.iter().find(|(c, _)| *c == category) else {
                continue;
            };
            if !first {
                out.push(Block::Blank);
            }
            first = false;
            out.push(Block::Markup(format!("  [bold]{category}[/]")));
            for (label, v) in rows {
                out.push(Block::Markup(format!(
                    "    [cyan]{label}[/]: {}",
                    escape(&fmt_value(v))
                )));
            }
        }
    }
    if let Some(pf) = item
        .get("planning_fields")
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_object)
    {
        out.push(Block::Blank);
        out.push(Block::Rule("[dim]planning_fields[/]".into()));
        for (k, v) in pf {
            if let Some(text) = truthy_str(obj_get(Some(v), "description_text")) {
                let cut: String = text.chars().take(200).collect();
                out.push(Block::Markup(format!("  [cyan]{k}[/]: {cut}")));
            }
        }
    }
    out
}

// --------------------------------------------------------------------------
// requested items

fn field_value_text(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let text = items
                .iter()
                .map(field_value_text)
                .collect::<Vec<_>>()
                .join(", ");
            if text.is_empty() { "-".into() } else { text }
        }
        Value::Object(m) => match ["name", "value", "label", "email"]
            .iter()
            .find_map(|k| m.get(*k).filter(|v| fsv_py::truthy(v)))
        {
            Some(v) => fsv_py::py_str(v),
            None => fsv_py::Dumps::default().ensure_ascii(false).dump(value),
        },
        other => {
            let text = pystr::strip(&fsv_py::py_str(other)).to_string();
            if text.is_empty() { "-".into() } else { text }
        }
    }
}

/// Port of fsv.render._requested_item_field_defs: the field, then its
/// `nested_fields` depth-first.
fn field_defs(field: &Value) -> Vec<&Value> {
    let mut defs = vec![field];
    for nested in field
        .get("nested_fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        defs.extend(field_defs(nested));
    }
    defs
}

/// `_requested_item_rows`: `(label, value)` answers in form order, then any
/// answered field the form does not describe (by name).
pub fn requested_item_rows(item: &Value) -> Vec<(String, String)> {
    let empty = Map::new();
    let values = item
        .get("custom_fields")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let defs: Vec<&Value> = obj_get(item.get("item"), "custom_fields")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    let mut add_field = |f: &Value, rows: &mut Vec<(String, String)>| {
        let (Some(name), Some(label)) = (truthy_str(f.get("name")), truthy_str(f.get("label")))
        else {
            return;
        };
        if seen.contains(&name) {
            return;
        }
        let Some(value) = values.get(&name) else {
            return;
        };
        if blank_answer(value) {
            return;
        }
        rows.push((label, field_value_text(value)));
        seen.push(name);
    };

    for f in defs {
        add_field(f, &mut rows);
        let selected = f
            .get("name")
            .map(fsv_py::py_str)
            .and_then(|n| values.get(&n))
            .filter(|v| fsv_py::truthy(v));
        let sections = f.get("sections").filter(|v| fsv_py::truthy(v));
        if let (Some(selected), Some(sections)) = (selected, sections) {
            for section in sections.as_array().into_iter().flatten() {
                if section.get("name").is_none_or(|n| !same(n, selected)) {
                    continue;
                }
                for sf in section
                    .get("fields")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    for nested in field_defs(sf) {
                        add_field(nested, &mut rows);
                    }
                }
                break;
            }
        } else {
            for nested in f
                .get("nested_fields")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                for child in field_defs(nested) {
                    add_field(child, &mut rows);
                }
            }
        }
    }
    for (name, value) in values {
        if seen.contains(name) || blank_answer(value) {
            continue;
        }
        rows.push((name.clone(), field_value_text(value)));
    }
    rows
}

/// `views.requested_item_json`.
pub fn requested_item_json(item: &Value) -> Value {
    let meta = item.get("item").filter(|v| fsv_py::truthy(v));
    let name = ["name", "short_description"]
        .iter()
        .find_map(|k| obj_get(meta, k).filter(|v| fsv_py::truthy(v)))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    let stage = match item.get("stage") {
        Some(s @ Value::Object(_)) => json!(name_of(s)),
        Some(s) if fsv_py::truthy(s) => s.clone(),
        _ => json!(""),
    };
    let answers: Vec<Value> = requested_item_rows(item)
        .into_iter()
        .map(|(k, v)| json!({"label": k, "value": v}))
        .collect();
    json!({
        "name": name,
        "stage": stage,
        "quantity": item.get("quantity").cloned().unwrap_or(Value::Null),
        "answers": answers,
    })
}

/// `requested_items_panel`: each requested item, its description and answers.
pub fn requested_items_panel(items: &[Value]) -> Vec<Block> {
    let mut out = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let idx = idx + 1;
        let meta = item.get("item").filter(|v| fsv_py::truthy(v));
        let item_name = ["name", "short_description"]
            .iter()
            .find_map(|k| obj_get(meta, k).filter(|v| fsv_py::truthy(v)))
            .map(fsv_py::py_str)
            .unwrap_or_else(|| format!("requested item {idx}"));
        out.push(Block::Blank);
        out.push(Block::Rule(format!(
            "[dim]requested_item {idx}[/]  {}",
            escape(&item_name)
        )));
        let stage = match item.get("stage") {
            Some(Value::Object(s)) => s
                .get("name")
                .filter(|v| fsv_py::truthy(v))
                .map(fsv_py::py_str),
            other => other.filter(|v| fsv_py::truthy(v)).map(fsv_py::py_str),
        };
        out.push(Block::Markup(format!(
            "stage     {}",
            stage.unwrap_or_else(|| "-".into())
        )));
        let description = obj_get(meta, "description")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        let description = pystr::strip(&description);
        if !description.is_empty() {
            out.push(Block::Blank);
            out.push(Block::Rule("[dim]requested_item description[/]".into()));
            out.push(Block::Markup(description.to_string()));
        }
        let rows = requested_item_rows(item);
        if !rows.is_empty() {
            out.push(Block::Blank);
            out.push(Block::Rule("[dim]requested_item fields[/]".into()));
            for (label, value) in rows {
                out.push(Block::Markup(format!(
                    "[cyan]{}[/]  {}",
                    escape(&label),
                    escape(&value)
                )));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fsv_core::resource::{CHANGES, TICKETS};

    fn extra() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/views_extra.json")).expect("fixture")
    }

    /// Names in `_requested_item_field_defs` order (`gen_fsv_views_extra.py`).
    #[test]
    fn field_defs_matches_python() {
        for row in extra()["field_defs"].as_array().expect("rows") {
            let names: Vec<&Value> = field_defs(&row[0])
                .into_iter()
                .map(|d| &d["name"])
                .collect();
            assert_eq!(json!(names), row[1], "{row}");
        }
    }

    #[test]
    fn requested_item_answers_follow_form_then_extras() {
        let item = json!({
            "item": {"name": "Laptop", "custom_fields": [
                {"name": "model", "label": "Model", "sections": [
                    {"name": "Mac", "fields": [{"name": "chip", "label": "Chip"}]},
                    {"name": "PC", "fields": [{"name": "gpu", "label": "GPU"}]}
                ]},
                {"name": "bag", "label": "Bag", "nested_fields": [{"name": "colour", "label": "Colour"}]},
                {"name": "count", "label": "Count"}
            ]},
            "custom_fields": {"model": "Mac", "chip": "M4", "gpu": "none", "bag": true, "colour": {"name": "Red"},
                              "count": 0, "extra": ["a", "b"]},
            "stage": {"name": "Fulfilment"},
            "quantity": 2
        });
        let rows = requested_item_rows(&item);
        assert_eq!(
            rows,
            [
                ("Model".to_string(), "Mac".to_string()),
                ("Chip".into(), "M4".into()),
                ("Bag".into(), "True".into()),
                ("Colour".into(), "Red".into()),
                ("gpu".into(), "none".into()),
                ("extra".into(), "a, b".into()),
            ]
        );
        let j = requested_item_json(&item);
        assert_eq!(j["name"], "Laptop");
        assert_eq!(j["stage"], "Fulfilment");
        assert_eq!(j["answers"].as_array().unwrap().len(), 6);
    }

    #[test]
    fn classic_list_columns_and_rows() {
        let schema =
            json!({"fields": [{"name": "status", "choices": [{"id": 2, "value": "Open"}]}]});
        let items = vec![
            json!({"id": 9, "status": 2, "priority": 3, "subject": "Disk", "type": "Incident", "requester": {"name": "Ann"}}),
        ];
        let t = list_table(&items, &TICKETS, Some(&schema));
        let headers: Vec<&str> = t.columns.iter().map(|c| c.header.as_str()).collect();
        assert_eq!(
            headers,
            ["ID", "Status", "Type", "Pri", "Subject", "Requester"]
        );
        assert_eq!(
            t.rows[0],
            ["INC-9", "Open (2)", "Incident", "High", "Disk", "Ann"]
        );
        let rows = list_rows(&items, &TICKETS, Some(&schema));
        assert_eq!(
            rows[0].keys().collect::<Vec<_>>(),
            [
                "id",
                "status",
                "status_id",
                "priority",
                "subject",
                "requester",
                "type"
            ]
        );
    }

    #[test]
    fn classic_detail_lines() {
        let item = json!({"display_id": 4, "subject": "Patch", "status": 1, "status_name": "Open", "priority": 2,
                          "requester": {"name": "Ann", "id": 3}, "group_name": "Ops", "group_id": 8,
                          "custom_fields": {"cf_b": "x", "cf_a": 0}});
        let text = blocks_text(&detail_panel(&item, &CHANGES, None, None), 60);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains("CHN-4  Patch"), "{text}");
        assert_eq!(lines[1], "status    Open  (id=1)");
        assert_eq!(lines[2], "type      ");
        assert_eq!(lines[3], "risk      -");
        assert_eq!(lines[4], "priority  Medium");
        assert_eq!(lines[5], "requester Ann (3)  agent -  group Ops (8)");
        assert!(text.contains("  other\n    cf_b: x\n"), "{text}");
    }

    #[test]
    fn state_diagram_window() {
        let change = json!({"state_flow_id": 1, "status": 3, "state_traversal": [1, 2], "human_display_id": "CHN-1"});
        let flow = json!({"state_list": [
            {"id": 1, "value": "Open"}, {"id": 2, "value": "Plan"}, {"id": 3, "value": "Build"},
            {"id": 4, "value": "Test"}, {"id": 5, "value": "Done"}
        ]});
        let text = blocks_text(&state_diagram(&change, Some(&flow), 1), 80);
        assert_eq!(
            text,
            "state flow: 40% complete  (2/5)\n… → ✓ Plan →  ▶ Build  → Test → …\n  use `fsv changes state CHN-1` for full flow\n"
        );
    }

    #[test]
    fn left_rule_fills_width() {
        let lines = blocks_lines(
            &[Block::LeftRule("[bold]Notes[/]".into()), Block::DimRule],
            12,
            false,
        );
        assert_eq!(lines, ["Notes ──────", "────────────"]);
    }
}
