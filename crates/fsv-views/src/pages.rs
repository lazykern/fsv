//! `pages.py`: site list and detail data rendered the way the web lays it
//! out. Every function returns Rich markup (or lines of it); all values
//! coming from the server are escaped.

use std::cmp::Ordering;

use jiff::tz::TimeZone;
use serde_json::{Map, Value};

use fsv_api::site::{self, Lifecycle, ListPage};
use fsv_core::pystr;
use fsv_core::term::{Column, Console, Overflow, Table, escape};

use crate::detail::{Detail, TabData};
use crate::render::{Block, print_blocks, requested_item_json};
use crate::source::name_of;

#[cfg(test)]
mod tests;

pub const DIM_EMPTY: &str = "[dim]--[/]";

/// `_e`: `None` is empty, anything else is `str()`-ed and escaped.
pub fn e(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        other => escape(&fsv_py::py_str(other)),
    }
}

fn es(text: &str) -> String {
    escape(text)
}

fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(fsv_py::truthy)
}

fn first_truthy<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|k| v.get(*k).filter(|x| fsv_py::truthy(x)))
}

fn str_or_empty(v: Option<&Value>) -> String {
    v.filter(|x| fsv_py::truthy(x))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// `str.capitalize()`.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

/// `isinstance(v, int)` (bools included) as an integer.
fn py_int_exact(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

pub fn section(title: &str) -> String {
    format!("[bold]{}[/]", es(title))
}

// --------------------------------------------------------------------------
// lists

/// `list_table`: the saved-view columns; subject carries the id underneath.
pub fn list_table(page: &ListPage) -> Table {
    let mut t = Table::new().show_lines(false).pad_edge(false).expand(false);
    for col in &page.columns {
        let mut c = Column::new(col.label.clone()).overflow(Overflow::Fold);
        if col.key == "status" {
            c = c.style("green");
        }
        if col.key == "subject" {
            c = c.ratio(3);
        }
        t.add_column(c);
    }
    for row in &page.rows {
        let mut cells = Vec::new();
        for col in &page.columns {
            let val = row.get(&col.key).map(fsv_py::py_str).unwrap_or_default();
            if col.key == "subject" {
                let id = row.get("id").map(e).unwrap_or_default();
                cells.push(format!("{}\n[dim cyan]#{id}[/]", es(&val)));
            } else if col.key == "assigned_to" && val.ends_with("(Me)") {
                cells.push(format!("{}[bold](Me)[/]", es(&val[..val.len() - 4])));
            } else if val.is_empty() {
                cells.push(DIM_EMPTY.to_string());
            } else {
                cells.push(es(&val));
            }
        }
        t.add_row(cells);
    }
    t
}

pub fn list_header(page: &ListPage) -> String {
    let title = page.title();
    let rest: String = page.header().chars().skip(char_len(&title) + 3).collect();
    format!("[bold]{}[/][dim] · {}[/]", es(&title), es(&rest))
}

// --------------------------------------------------------------------------
// detail: header, lifecycle, reporter, tab bar

pub fn header_lines(d: &Detail) -> Vec<String> {
    let mut out = vec![format!(
        "[bold cyan]#{}[/]  [bold]{}[/]",
        es(&d.id()),
        es(&d.subject())
    )];
    let mut status = format!("[bold green]{}[/]", es(&d.badge()));
    let due = d.due_text();
    let stats = d.record.get("stats");
    let closed = (truthy(stats.and_then(|s| s.get("resolved_at")))
        || truthy(stats.and_then(|s| s.get("closed_at"))))
        && d.resource.name == "tickets";
    if !due.is_empty() && !closed {
        status += &format!(", due {}", es(&due));
    }
    out.push(status);
    out
}

/// `✓ Open › [Planning] › Awaiting Approval › …`; `context` keeps only that
/// many states either side of the current one.
pub fn lifecycle_markup(lc: Option<&Lifecycle>, context: Option<usize>) -> String {
    let Some(lc) = lc.filter(|lc| !lc.states.is_empty()) else {
        return String::new();
    };
    let states = &lc.states;
    let cur = states.iter().position(|s| s.mark == "current").unwrap_or(0);
    let (mut lo, mut hi) = (0, states.len() - 1);
    if let Some(c) = context {
        lo = cur.saturating_sub(c);
        hi = (states.len() - 1).min(cur + c);
    }
    let mut parts: Vec<String> = if lo > 0 {
        vec!["[dim]…[/]".into()]
    } else {
        Vec::new()
    };
    for s in &states[lo..=hi] {
        let name = es(&s.name);
        parts.push(match s.mark.as_str() {
            "current" => format!("[bold black on green] {name} [/]"),
            "done" => format!("[green]✓ {name}[/]"),
            _ => format!("[dim]{name}[/]"),
        });
    }
    if hi < states.len() - 1 {
        parts.push("[dim]…[/]".into());
    }
    parts.join(" [dim]›[/] ")
}

/// The lifecycle bar; `""` when the account has none.
pub fn status_line(d: &Detail) -> String {
    lifecycle_markup(d.lifecycle.as_ref(), None)
}

pub fn reporter_markup(d: &Detail) -> String {
    let r = d.reporter();
    if !truthy(r.get("name")) && !truthy(r.get("created_at")) {
        return String::new();
    }
    let name = r
        .get("name")
        .filter(|v| fsv_py::truthy(v))
        .map(e)
        .unwrap_or_else(|| "Someone".into());
    let mut text = format!(
        "[bold]{name}[/] reported {}",
        r.get("relative").map(e).unwrap_or_default()
    );
    if let Some(when) = r.get("when").filter(|v| fsv_py::truthy(v)) {
        text += &format!(" [dim]({})[/]", e(when));
    }
    if let Some(via) = r.get("via").filter(|v| fsv_py::truthy(v)) {
        text += &format!(" via [bold]{}[/]", e(via));
    }
    if let Some(rf) = r.get("request_for").filter(|v| fsv_py::truthy(v)) {
        text += &format!("   [dim]Request for:[/] {}", e(rf));
    }
    text
}

pub fn tab_bar(d: &Detail, active: &str, numbered: bool) -> String {
    let mut parts = Vec::new();
    for (i, t) in d.tabs.iter().enumerate() {
        let mut label = match t.count.filter(|c| *c != 0) {
            Some(c) => format!("{} ({c})", t.label),
            None => t.label.clone(),
        };
        if numbered {
            label = format!("{}:{label}", i + 1);
        }
        parts.push(if t.name == active {
            format!("[bold reverse] {} [/]", es(&label))
        } else {
            format!("[dim]{}[/]", es(&label))
        });
    }
    parts.join("  ")
}

// --------------------------------------------------------------------------
// detail: Details tab (main column + right column)

/// `_indent`.
pub fn indent(text: &str, pad: &str) -> Vec<String> {
    let out: Vec<String> = site::py_splitlines(text)
        .into_iter()
        .map(|ln| {
            if ln.is_empty() {
                String::new()
            } else {
                format!("{pad}{}", es(ln))
            }
        })
        .collect();
    if out.is_empty() {
        vec![pad.to_string()]
    } else {
        out
    }
}

pub fn main_lines(d: &Detail) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(items) = &d.requested_items {
        out.push(section("Requested items"));
        if items.is_empty() {
            out.push("  [dim]none[/]".into());
        }
        for item in items {
            let j = requested_item_json(item);
            let qty = match j
                .get("quantity")
                .filter(|q| !q.is_boolean())
                .and_then(Value::as_i64)
            {
                Some(q) if q > 1 => format!(" × {q}"),
                _ => String::new(),
            };
            let name = j
                .get("name")
                .filter(|v| fsv_py::truthy(v))
                .map(e)
                .unwrap_or_else(|| "Requested item".into());
            let stage = j
                .get("stage")
                .filter(|v| fsv_py::truthy(v))
                .map(e)
                .unwrap_or_else(|| "-".into());
            out.push(format!("  [bold]{name}[/]{qty}   [dim]Stage:[/] {stage}"));
            for a in j
                .get("answers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                out.push(format!(
                    "    [cyan]{}[/]  {}",
                    e(&a["label"]),
                    e(&a["value"])
                ));
            }
        }
        out.push(String::new());
    } else if let Some(miss) = &d.requested_items_missing {
        out.push(format!("[dim]{}[/]", es(&miss.note())));
        out.push(String::new());
    }
    out.push(section("Description"));
    let desc = d.description();
    if desc.is_empty() {
        out.push("  [dim]No description[/]".into());
    } else {
        out.extend(indent(&desc, "  "));
    }
    let atts = d.attachments();
    if !atts.is_empty() {
        out.push(String::new());
        out.push(section("Attachments"));
        for a in &atts {
            let size = match a.get("size").filter(|v| fsv_py::truthy(v)) {
                Some(s) => format!(" [dim]({})[/]", human_size(s)),
                None => String::new(),
            };
            out.push(format!(
                "  📎 {}{size}",
                a.get("name").map(e).unwrap_or_default()
            ));
        }
    }
    if let Some(planning) = &d.planning {
        out.push(String::new());
        out.push(section("Planning"));
        for p in planning {
            let help = if !p.help.is_empty() && p.help != p.label {
                format!("  [dim]{}[/]", es(&p.help))
            } else {
                String::new()
            };
            out.push(format!("  [cyan]{}[/]{help}", es(&p.label)));
            if p.empty() {
                out.push("    [dim]empty[/]".into());
                continue;
            }
            if !p.text.is_empty() {
                out.extend(indent(&p.text, "    "));
            }
            for f in &p.files {
                out.push(format!("    📎 {}", es(f)));
            }
        }
    } else if let Some(miss) = &d.planning_missing {
        out.push(String::new());
        out.push(format!("[dim]{}[/]", es(&miss.note())));
    }
    if let Some(convs) = &d.conversations {
        out.push(String::new());
        let total = d.conversations_meta.get("count");
        let shown = convs.len() as i64;
        let title = match total.filter(|v| fsv_py::truthy(v)) {
            Some(t) => format!("Conversations ({})", fsv_py::py_str(t)),
            None => "Conversations".into(),
        };
        out.push(section(&title));
        if let Some(t) = py_int_exact(total).filter(|t| *t > shown) {
            out.push(format!(
                "  [dim]… {} earlier (use --all-conversations)[/]",
                t - shown
            ));
        }
        if convs.is_empty() {
            out.push("  [dim]no conversations[/]".into());
        }
        for cv in convs {
            out.extend(conversation_lines(cv, Some(&d.tz)));
        }
        if let Some(miss) = &d.conversations_missing {
            out.push(format!("  [yellow]incomplete: {}[/]", es(&miss.note())));
        }
    } else if let Some(miss) = &d.conversations_missing {
        out.push(String::new());
        out.push(format!("[dim]{}[/]", es(&miss.note())));
    }
    out
}

pub fn conversation_lines(cv: &Value, tz: Option<&TimeZone>) -> Vec<String> {
    let j = site::conversation_json(cv, tz);
    let kind = j["kind"].as_str().unwrap_or_default();
    let mark = if fsv_py::truthy(&j["private"]) {
        "[yellow]🔒 private[/]"
    } else {
        "[green]public[/]"
    };
    let when = site::fmt_when(&j["created_at"], tz, false);
    let author = Some(&j["author"])
        .filter(|v| fsv_py::truthy(v))
        .map(e)
        .unwrap_or_else(|| "Someone".into());
    let mut out = vec![format!(
        "  [bold]{author}[/] {} · {mark} · {} [dim]({})[/]",
        es(kind),
        e(&j["relative"]),
        es(&when)
    )];
    let files = j["attachments"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let body = j["body"].as_str().unwrap_or_default();
    if !body.is_empty() {
        out.extend(indent(body, "    "));
    } else {
        out.push("    [dim](empty)[/]".into());
    }
    for f in files {
        out.push(format!("    📎 {}", e(f)));
    }
    out.push(String::new());
    out
}

/// `float(n)`: numbers, bools, and numeric text.
fn py_float(n: &Value) -> Option<f64> {
    match n {
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Number(x) => x.as_f64(),
        Value::String(s) => {
            let t = pystr::strip(s);
            let t =
                if t.contains('_') && !t.starts_with('_') && !t.ends_with('_') && !t.contains("__")
                {
                    t.replace('_', "")
                } else {
                    t.to_string()
                };
            t.parse().ok()
        }
        _ => None,
    }
}

/// `f"{x:.{places}f}"`: half-even, and a negative value that rounds to zero
/// keeps the minus (`-0`, `-0.0`).
fn fmt_fixed(x: f64, places: usize) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x.is_sign_positive() {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let scale = 10f64.powi(places as i32);
    let scaled = x * scale;
    if !scaled.is_finite() {
        return format!("{x:.places$}");
    }
    let rounded = fsv_core::pystr::round_half_even(scaled);
    let neg = if rounded == 0.0 {
        x.is_sign_negative()
    } else {
        rounded.is_sign_negative()
    };
    let digits = format!("{:.0}", rounded.abs());
    let body = if places == 0 {
        digits
    } else if digits.len() <= places {
        format!("0.{digits:0>places$}")
    } else {
        let split = digits.len() - places;
        format!("{}.{}", &digits[..split], &digits[split..])
    };
    if neg { format!("-{body}") } else { body }
}

/// `_human_size`: `512 B`, `1.5 KB`, … up to GB.
pub fn human_size(n: &Value) -> String {
    let Some(mut size) = py_float(n) else {
        return fsv_py::py_str(n);
    };
    for unit in ["B", "KB", "MB", "GB"] {
        if size < 1024.0 || unit == "GB" {
            return if unit == "B" {
                format!("{} {unit}", fmt_fixed(size, 0))
            } else {
                format!("{} {unit}", fmt_fixed(size, 1))
            };
        }
        size /= 1024.0;
    }
    fsv_py::py_str(n)
}

pub fn side_lines(d: &Detail) -> Vec<String> {
    let mut out = Vec::new();
    let rows = d.summary();
    let lw = rows.iter().map(|(k, _)| char_len(k)).max().unwrap_or(6) + 2;
    for (label, value) in &rows {
        let v = if value.is_empty() {
            DIM_EMPTY.to_string()
        } else {
            es(value)
        };
        out.push(format!(
            "[cyan]{}[/]{}{v}",
            es(label),
            " ".repeat(lw - char_len(label))
        ));
    }
    out.push(String::new());
    out.push(section("Properties"));
    if d.properties.is_empty() {
        out.push("  [dim]no fields[/]".into());
    }
    let width = 28.min(
        d.properties
            .iter()
            .map(|p| char_len(&p.label))
            .max()
            .unwrap_or(8)
            + 2,
    );
    for p in &d.properties {
        let star = if p.required { "[red]*[/]" } else { " " };
        let label = if char_len(&p.label) <= width {
            p.label.clone()
        } else {
            p.label.chars().take(width - 1).collect::<String>() + "…"
        };
        let pad = " ".repeat(1.max(width.saturating_sub(char_len(&label))));
        let v = if p.value.is_empty() {
            DIM_EMPTY.to_string()
        } else {
            es(&p.value)
        };
        out.push(format!("  [cyan]{}[/]{star}{pad}{v}", es(&label)));
    }
    out
}

pub fn notes_lines(d: &Detail) -> Vec<String> {
    d.notes()
        .iter()
        .map(|n| format!("[dim]· {}[/]", es(n)))
        .collect()
}

// --------------------------------------------------------------------------
// other tabs

/// `_who`.
pub fn who(obj: Option<&Value>) -> String {
    obj.map(name_of).unwrap_or_default()
}

/// `_status_text`.
pub fn status_text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(m @ Value::Object(_)) => str_or_empty(first_truthy(m, &["name", "value", "id"])),
        Some(other) => fsv_py::py_str(other),
    }
}

type TabRenderer = fn(&TabData, Option<&TimeZone>, Option<&Value>) -> Vec<String>;

fn renderer(name: &str) -> TabRenderer {
    match name {
        "notes" => notes_tab,
        "tasks" => tasks_tab,
        "approvals" => approvals_tab,
        "assets" | "impacted_services" => assets_tab,
        "activity" => activity_tab,
        "associations" | "devops" | "related_tickets" => parts_tab,
        "risk" => risk_tab,
        "resolution_notes" => resolution_tab,
        _ => generic_tab,
    }
}

pub fn tab_lines(td: &TabData, tz: Option<&TimeZone>, schema: Option<&Value>) -> Vec<String> {
    if let Some(miss) = &td.missing
        && td.data.is_none()
        && td.parts.is_empty()
    {
        return vec![format!("[dim]{}[/]", es(&miss.note()))];
    }
    let mut out = renderer(&td.name)(td, tz, schema);
    if let Some(miss) = &td.missing {
        out.push(format!("[dim]{}[/]", es(&miss.note())));
    }
    out.extend(td.notes.iter().map(|n| format!("[dim]· {}[/]", es(n))));
    out
}

fn data_rows(td: &TabData) -> &[Value] {
    td.data
        .as_ref()
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn data_obj(td: &TabData) -> Map<String, Value> {
    td.data
        .as_ref()
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

pub fn notes_tab(td: &TabData, tz: Option<&TimeZone>, _schema: Option<&Value>) -> Vec<String> {
    let rows = data_rows(td);
    if rows.is_empty() {
        return vec!["[dim]no notes[/]".into()];
    }
    let mut out = Vec::new();
    for n in rows {
        let lock = if truthy(n.get("private")) {
            "[yellow]🔒 private[/]"
        } else {
            "[green]public[/]"
        };
        let mut who_ = who(n.get("user"));
        if who_.is_empty() {
            who_ = str_or_empty(n.get("user_id"));
        }
        let who_ = if who_.is_empty() {
            "Someone".to_string()
        } else {
            who_
        };
        let created = n.get("created_at").unwrap_or(&Value::Null);
        out.push(format!(
            "[bold]{}[/] added a note · {lock} · {} [dim]({})[/]",
            es(&who_),
            es(&site::relative(created, None)),
            es(&site::fmt_when(created, tz, false))
        ));
        let html = n
            .get("body")
            .filter(|v| fsv_py::truthy(v))
            .cloned()
            .unwrap_or(Value::String(String::new()));
        let mut body = site::html_to_text(&html);
        if body.is_empty() {
            body = str_or_empty(n.get("body_text"));
        }
        out.extend(indent(&body, "  "));
        out.push(String::new());
    }
    out
}

pub fn tasks_tab(td: &TabData, tz: Option<&TimeZone>, _schema: Option<&Value>) -> Vec<String> {
    let rows = data_rows(td);
    if rows.is_empty() {
        return vec!["[dim]no tasks[/]".into()];
    }
    let mut out = Vec::new();
    for t in rows {
        let mut st = str_or_empty(t.get("status_name"));
        if st.is_empty() {
            st = status_text(t.get("status_info"));
        }
        if st.is_empty() {
            st = fsv_api::display(t.get("status").unwrap_or(&Value::Null), None);
        }
        let tid = match first_truthy(t, &["human_display_id"]) {
            Some(h) => fsv_py::py_str(h),
            None => {
                let id = first_truthy(t, &["display_id"]).or_else(|| t.get("id"));
                format!(
                    "#{}",
                    id.map(fsv_py::py_str).unwrap_or_else(|| "None".into())
                )
            }
        };
        let title = t
            .get("title")
            .filter(|v| fsv_py::truthy(v))
            .map(e)
            .unwrap_or_else(|| "-".into());
        out.push(format!("○ [bold]{title}[/] [dim]{}[/]", es(&tid)));
        let mut bits = vec![st];
        let mut grp = who(t.get("group"));
        if grp.is_empty() {
            grp = str_or_empty(t.get("group_name"));
        }
        let mut agent = who(t.get("agent"));
        if agent.is_empty() {
            agent = str_or_empty(t.get("owner_name"));
        }
        if !grp.is_empty() || !agent.is_empty() {
            let or_none = |s: String| if s.is_empty() { "None".to_string() } else { s };
            bits.push(format!("{} / {}", or_none(grp), or_none(agent)));
        }
        if let Some(due) = t.get("due_date").filter(|v| fsv_py::truthy(v)) {
            bits.push(format!("due {}", site::fmt_when(due, tz, true)));
        }
        let joined = bits
            .into_iter()
            .filter(|b| !b.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        out.push(format!("    [dim]{}[/]", es(&joined)));
    }
    out
}

/// Python's `(str(type(x)), x)` sort key for approval levels.
fn level_type(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "<class 'bool'>",
        Value::Number(n) if n.is_f64() => "<class 'float'>",
        Value::Number(_) => "<class 'int'>",
        Value::String(_) => "<class 'str'>",
        Value::Null => "<class 'NoneType'>",
        Value::Array(_) => "<class 'list'>",
        Value::Object(_) => "<class 'dict'>",
    }
}

fn level_num(v: &Value) -> Option<f64> {
    match v {
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn level_eq(a: &Value, b: &Value) -> bool {
    match (level_num(a), level_num(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn level_cmp(a: &Value, b: &Value) -> Ordering {
    level_type(a)
        .cmp(level_type(b))
        .then_with(|| match (level_num(a), level_num(b)) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => match (a.as_str(), b.as_str()) {
                (Some(x), Some(y)) => x.cmp(y),
                _ => Ordering::Equal,
            },
        })
}

pub fn approvals_tab(td: &TabData, _tz: Option<&TimeZone>, _schema: Option<&Value>) -> Vec<String> {
    let rows = data_rows(td);
    if rows.is_empty() {
        return vec!["[dim]no approvals requested[/]".into()];
    }
    let mut levels: Vec<(Value, Vec<&Value>)> = Vec::new();
    for a in rows {
        let lvl = first_truthy(a, &["level_id", "level"])
            .cloned()
            .unwrap_or(Value::from(1));
        match levels.iter_mut().find(|(k, _)| level_eq(k, &lvl)) {
            Some((_, group)) => group.push(a),
            None => levels.push((lvl, vec![a])),
        }
    }
    levels.sort_by(|a, b| level_cmp(&a.0, &b.0));
    let mut out = Vec::new();
    for (lvl, group) in &levels {
        out.push(section(&format!("Level {}", fsv_py::py_str(lvl))));
        for a in group {
            let mut st = status_text(a.get("status"));
            if st.is_empty() {
                st = str_or_empty(a.get("approval_status"));
            }
            let icon = match st.to_lowercase().as_str() {
                "approved" => "[green]✓[/]",
                "rejected" => "[red]✗[/]",
                _ => "[yellow]◷[/]",
            };
            let mut who_ = who(first_truthy(a, &["member"]).or_else(|| a.get("approver")));
            if who_.is_empty() {
                who_ = str_or_empty(a.get("member_id"));
            }
            let when = first_truthy(a, &["updated_at"])
                .or_else(|| a.get("created_at"))
                .unwrap_or(&Value::Null);
            out.push(format!(
                "  {icon} [bold]{}[/]  {}  [dim]{}[/]",
                es(&who_),
                es(&capitalize(&st)),
                es(&site::relative(when, None))
            ));
            let remarks = a
                .get("remark")
                .filter(|v| fsv_py::truthy(v))
                .and_then(Value::as_array);
            for r in remarks.into_iter().flatten() {
                if let Some(data) = r
                    .as_object()
                    .and_then(|m| m.get("data"))
                    .filter(|v| fsv_py::truthy(v))
                {
                    out.push(format!("      [dim]“{}”[/]", e(data)));
                }
            }
        }
    }
    out
}

pub fn assets_tab(td: &TabData, _tz: Option<&TimeZone>, _schema: Option<&Value>) -> Vec<String> {
    let rows = data_rows(td);
    if rows.is_empty() {
        return vec!["[dim]none associated[/]".into()];
    }
    let mut out = Vec::new();
    for a in rows {
        let it = a.get("config_item").filter(|v| v.is_object()).unwrap_or(a);
        let typ = match first_truthy(it, &["ci_type_name"]) {
            Some(v) => fsv_py::py_str(v),
            None => {
                let w = who(it.get("ci_type"));
                if w.is_empty() {
                    str_or_empty(it.get("asset_type_name"))
                } else {
                    w
                }
            }
        };
        let aid = first_truthy(it, &["display_id"])
            .or_else(|| it.get("id"))
            .unwrap_or(&Value::Null);
        let name = first_truthy(it, &["name", "display_name"])
            .map(e)
            .unwrap_or_else(|| "-".into());
        out.push(format!(
            "[bold]{name}[/]  [dim]{} · #{}[/]",
            es(&typ),
            e(aid)
        ));
    }
    out
}

/// `(day heading, HH:MM)` for an activity row, in `tz` (the value's own offset when `None`).
fn activity_when(value: &Value, tz: Option<&TimeZone>) -> Option<(String, String)> {
    let when = site::parse_dt(value)?;
    let dt = match tz {
        Some(tz) => when.to_zoned(tz.clone()).datetime(),
        None => {
            let text = fsv_py::py_str(value).replace('Z', "+00:00");
            fsv_core::output::parse_isoformat(&text)?.0
        }
    };
    let day = format!(
        "{}, {} {}",
        dt.strftime("%a"),
        dt.day(),
        dt.strftime("%b %Y")
    );
    Some((day, dt.strftime("%H:%M").to_string()))
}

pub fn activity_tab(td: &TabData, tz: Option<&TimeZone>, _schema: Option<&Value>) -> Vec<String> {
    let rows = data_rows(td);
    if rows.is_empty() {
        return vec!["[dim]no activity[/]".into()];
    }
    let mut out = Vec::new();
    let mut day: Option<String> = None;
    for a in rows {
        let local = activity_when(a.get("created_at").unwrap_or(&Value::Null), tz);
        let d = local
            .as_ref()
            .map_or_else(|| "?".to_string(), |(d, _)| d.clone());
        if day.as_deref() != Some(d.as_str()) {
            out.push(section(&d));
            day = Some(d);
        }
        let mut actor = who(a.get("actor"));
        if actor.is_empty() {
            actor = "System".into();
        }
        let content = es(&site::one_line(a.get("content").unwrap_or(&Value::Null)));
        out.push(match &local {
            Some((_, hm)) => format!("  [dim]{hm}[/]  [bold]{}[/] {content}", es(&actor)),
            None => format!("  [bold]{}[/] {content}", es(&actor)),
        });
        let subs = a
            .get("sub_contents")
            .filter(|v| fsv_py::truthy(v))
            .and_then(Value::as_array);
        for s in subs.into_iter().flatten() {
            out.push(format!("         [dim]· {}[/]", es(&site::one_line(s))));
        }
    }
    out
}

/// `_item_line`.
pub fn item_line(x: &Value) -> String {
    let did = first_truthy(x, &["human_display_id", "display_id"])
        .or_else(|| x.get("id"))
        .unwrap_or(&Value::Null);
    let mut st = str_or_empty(x.get("status_name"));
    if st.is_empty() {
        st = status_text(x.get("status"));
    }
    let subject = str_or_empty(first_truthy(x, &["subject", "name", "title"]));
    let tail = if st.is_empty() {
        String::new()
    } else {
        format!("  [dim]{}[/]", es(&st))
    };
    format!("  [cyan]#{}[/] {}{tail}", e(did), es(&subject))
}

pub fn parts_tab(td: &TabData, _tz: Option<&TimeZone>, _schema: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    for (label, rows) in &td.parts {
        if label == "survey" {
            continue;
        }
        let rows = rows.as_array().map(Vec::as_slice).unwrap_or(&[]);
        out.push(section(&format!("{label} ({})", rows.len())));
        if rows.is_empty() {
            out.push("  [dim]none[/]".into());
        }
        out.extend(rows.iter().map(item_line));
    }
    if out.is_empty() {
        return vec!["[dim]nothing associated[/]".into()];
    }
    out
}

pub fn risk_tab(td: &TabData, _tz: Option<&TimeZone>, schema: Option<&Value>) -> Vec<String> {
    let data = data_obj(td);
    let mut out = Vec::new();
    let score = data.get("overall_score").filter(|v| !v.is_null());
    out.push(format!(
        "[cyan]Overall score[/]  {}",
        score.map(e).unwrap_or_else(|| DIM_EMPTY.into())
    ));
    let computed = data
        .get("computed_risk")
        .filter(|v| !v.is_null() && v.as_str() != Some(""));
    out.push(format!(
        "[cyan]Computed risk[/]  {}",
        computed
            .map(|c| es(&status_text(Some(c))))
            .unwrap_or_else(|| DIM_EMPTY.into())
    ));
    let factors = data
        .get("basic_risk_awareness")
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_array);
    if let Some(factors) = factors.filter(|f| !f.is_empty()) {
        out.push(String::new());
        out.push(section(&format!("{} factors considered", factors.len())));
        for f in factors.iter().filter(|f| f.is_object()) {
            let mut name = site::choice_name(schema, "", None);
            if name.is_empty() {
                name = capitalize(&str_or_empty(f.get("name")).replace('_', " "));
            }
            let op = str_or_empty(f.get("operator"));
            out.push(format!(
                "  {} {} [bold]{}[/]",
                es(&name),
                es(&op),
                f.get("value").map(e).unwrap_or_default()
            ));
        }
    }
    if let Some(survey) = td
        .parts
        .get("survey")
        .and_then(Value::as_object)
        .filter(|s| !s.is_empty())
    {
        out.push(String::new());
        out.push(section("Risk assessment"));
        for (k, v) in survey {
            if matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                out.push(format!("  [cyan]{}[/]  {}", es(k), e(v)));
            }
        }
    }
    out
}

pub fn resolution_tab(td: &TabData, tz: Option<&TimeZone>, _schema: Option<&Value>) -> Vec<String> {
    let data = data_obj(td);
    let mut out = Vec::new();
    if let Some(v) = data.get("resolved_at").filter(|v| fsv_py::truthy(v)) {
        out.push(format!(
            "[cyan]Resolved[/]  {}",
            es(&site::fmt_when(v, tz, true))
        ));
    }
    if let Some(v) = data.get("closed_at").filter(|v| fsv_py::truthy(v)) {
        out.push(format!(
            "[cyan]Closed[/]  {}",
            es(&site::fmt_when(v, tz, true))
        ));
    }
    if let Some(note) = data.get("note").filter(|v| fsv_py::truthy(v)) {
        out.push(String::new());
        out.push(section("Resolution note"));
        out.extend(indent(&fsv_py::py_str(note), "  "));
    }
    if truthy(td.meta.get("absent")) {
        out.push("[dim]no resolution note (404)[/]".into());
    }
    if out.is_empty() && td.missing.is_none() {
        out.push("[dim]no resolution yet[/]".into());
    }
    out
}

pub fn generic_tab(td: &TabData, tz: Option<&TimeZone>, schema: Option<&Value>) -> Vec<String> {
    if !td.parts.is_empty() {
        return parts_tab(td, tz, schema);
    }
    match td.data.as_ref().filter(|v| fsv_py::truthy(v)) {
        None => vec!["[dim]nothing here[/]".into()],
        Some(Value::Array(rows)) => {
            let out: Vec<String> = rows
                .iter()
                .filter(|r| r.is_object())
                .map(item_line)
                .collect();
            if out.is_empty() {
                vec!["[dim]nothing here[/]".into()]
            } else {
                out
            }
        }
        Some(other) => vec![e(other)],
    }
}

// --------------------------------------------------------------------------
// CLI printing

/// What `print_detail` prints, as blocks (the Details grid is laid out at print width).
pub fn detail_blocks(d: &Detail, tab: Option<&str>, all_tabs: bool) -> Vec<Block> {
    let mut out: Vec<Block> = header_lines(d).into_iter().map(Block::Markup).collect();
    let bar = status_line(d);
    if !bar.is_empty() {
        out.push(Block::Markup(bar));
    }
    let rep = reporter_markup(d);
    if !rep.is_empty() {
        out.push(Block::Markup(rep));
    }
    out.push(Block::Blank);
    let active = tab.unwrap_or("details");
    out.push(Block::Markup(tab_bar(d, active, false)));
    out.push(Block::DimRule);
    if active == "details" || all_tabs {
        out.push(Block::Columns {
            main: main_lines(d).join("\n"),
            side: side_lines(d).join("\n"),
        });
    }
    let names: Vec<String> = if all_tabs {
        d.tabs
            .iter()
            .filter(|t| t.name != "details")
            .map(|t| t.name.clone())
            .collect()
    } else if active == "details" {
        Vec::new()
    } else {
        vec![active.to_string()]
    };
    for name in &names {
        let Some(td) = d.tab(name) else { continue };
        if all_tabs {
            out.push(Block::Blank);
            out.push(Block::LeftRule(format!("[bold]{}[/]", es(&td.label))));
        }
        out.extend(
            tab_lines(td, Some(&d.tz), Some(&d.schema))
                .into_iter()
                .map(Block::Markup),
        );
    }
    let shown: Vec<String> = [
        &d.requested_items_missing,
        &d.planning_missing,
        &d.conversations_missing,
    ]
    .into_iter()
    .flatten()
    .map(|m| m.note())
    .collect();
    let notes: Vec<String> = d
        .notes()
        .into_iter()
        .filter(|n| !shown.contains(n))
        .map(|n| format!("[dim]· {}[/]", es(&n)))
        .collect();
    if !notes.is_empty() {
        out.push(Block::Blank);
        out.extend(notes.into_iter().map(Block::Markup));
    }
    out
}

pub fn print_detail(console: &Console, d: &Detail, tab: Option<&str>, all_tabs: bool) {
    print_blocks(console, &detail_blocks(d, tab, all_tabs));
}
