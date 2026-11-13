//! Saved views, list columns, row cells, page headers, and detail tabs.
//! Port of the pure half of `views.py` (fetching stays with the caller).

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use fsv_core::pystr;
use fsv_core::resource::{self, Resource};

use crate::QueryError;
use crate::site::format::{fmt_when, is_date_only, parse_dt};

const TICKET_STATES: &[(&str, &str)] = &[
    ("new", "New"),
    ("elapsed", "Response Due"),
    ("overdue", "Overdue"),
];

const SORT_LABELS: &[(&str, &str)] = &[
    ("created_at", "Created Date"),
    ("updated_at", "Last Modified"),
    ("due_by", "Due by"),
    ("fr_due_by", "First response due"),
    ("display_id", "ID"),
];

const COLUMN_KEYS: &[(&str, &[&str])] = &[
    (
        "changes",
        &[
            "subject",
            "requester",
            "status",
            "priority",
            "assigned_to",
            "department",
        ],
    ),
    (
        "tickets",
        &[
            "subject",
            "requester",
            "state",
            "status",
            "priority",
            "assigned_to",
        ],
    ),
    (
        "problems",
        &[
            "subject",
            "requester",
            "status",
            "priority",
            "assigned_to",
            "department",
        ],
    ),
];

const CHROME_LABELS: &[(&str, &str)] = &[
    ("subject", "Subject"),
    ("requester", "Requester"),
    ("state", "State"),
    ("status", "Status"),
    ("priority", "Priority"),
    ("assigned_to", "Assigned to"),
    ("department", "Department"),
];

const HEADER_KEYS: &[(&str, &str)] = &[
    ("ticket_subject", "subject"),
    ("subject", "subject"),
    ("requester", "requester"),
    ("state", "state"),
    ("status", "status"),
    ("priority", "priority"),
    ("assigned_to", "assigned_to"),
    ("department", "department"),
];

const TAB_LABELS: &[(&str, &str)] = &[
    ("details", "Details"),
    ("devops", "Devops"),
    ("risk", "Risk"),
    ("notes", "Notes"),
    ("tasks", "Tasks"),
    ("assets", "Assets"),
    ("impacted_services", "Impacted services"),
    ("associations", "Associations"),
    ("approvals", "Approvals"),
    ("activity", "Activity"),
    ("related_tickets", "Related tickets"),
    ("resolution_notes", "Resolution"),
    ("time_entries", "Time entries"),
];

const TAB_ALIASES: &[(&str, &str)] = &[
    ("impacted", "impacted_services"),
    ("impacted-services", "impacted_services"),
    ("related", "related_tickets"),
    ("related-tickets", "related_tickets"),
    ("resolution", "resolution_notes"),
    ("activities", "activity"),
    ("detail", "details"),
];

const DETAILS_PARTS: &[&str] = &["requested_items", "details", "conversations"];

const FALLBACK_TABS: &[(&str, &[&str])] = &[
    ("changes", &["notes", "tasks", "approvals", "activity"]),
    (
        "tickets",
        &["tasks", "approvals", "activity", "resolution_notes"],
    ),
    ("problems", &["notes", "tasks", "activity"]),
];

/// A section the tenant did not provide (feature off, role, plan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub what: String,
    pub status: Option<i64>,
}

impl Missing {
    pub fn new(what: impl Into<String>, status: Option<i64>) -> Self {
        Self {
            what: what.into(),
            status,
        }
    }

    /// `403`/`404` wording from `Missing.note`. Status `0` is falsy, like Python.
    ///
    /// Port of `views.Missing.note`.
    pub fn note(&self) -> String {
        match self.status {
            Some(403) => format!("{}: not permitted for your role (403)", self.what),
            Some(404) => format!("{}: not available on this account (404)", self.what),
            Some(status) if status != 0 => format!("{}: unavailable ({status})", self.what),
            _ => format!("{}: unavailable", self.what),
        }
    }

    /// Port of `views.Missing.to_json`.
    pub fn to_json(&self) -> Value {
        json!({"unavailable": true, "status": self.status, "note": self.note()})
    }
}

/// One saved list view, in server order.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedView {
    pub id: String,
    pub name: String,
    pub system: bool,
    pub marked_default: bool,
    pub raw: Value,
}

impl SavedView {
    /// Port of `views.SavedView.to_json`.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "system": self.system,
            "default": self.marked_default,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListPage {
    pub resource_name: String,
    pub items: Vec<Value>,
    pub columns: Vec<Column>,
    pub rows: Vec<Map<String, Value>>,
    pub view_name: Option<String>,
    pub page: i64,
    pub per_page: i64,
    pub total: Option<i64>,
    pub has_next: bool,
    pub order_by: String,
    pub order_type: String,
    pub sort: String,
    pub notes: Vec<String>,
}

impl ListPage {
    /// Port of `views.ListPage.title`.
    pub fn title(&self) -> String {
        self.view_name
            .clone()
            .unwrap_or_else(|| format!("All {}", self.resource_name))
    }

    /// `<view> · 31-33 of 33 · Sort by: Created Date`
    ///
    /// Port of `views.ListPage.header`.
    pub fn header(&self) -> String {
        let n = self.items.len() as i64;
        let end = (self.page - 1) * self.per_page + n;
        let rng = if n > 0 {
            let lo = (self.page - 1) * self.per_page + 1;
            format!("{lo}-{end}")
        } else {
            "0".to_string()
        };
        let of = match self.total {
            Some(total) if total >= end => format!(" of {total}"),
            _ if self.has_next => format!(" of {end}+"),
            _ => format!(" of {end}"),
        };
        format!("{} · {rng}{of} · Sort by: {}", self.title(), self.sort)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub name: String,
    pub label: String,
    pub count: Option<i64>,
}

impl Tab {
    /// Port of `views.Tab.to_json`.
    pub fn to_json(&self) -> Value {
        json!({"name": self.name, "label": self.label, "count": self.count})
    }
}

pub fn fallback_tabs(resource_name: &str) -> &'static [&'static str] {
    FALLBACK_TABS
        .iter()
        .find(|(n, _)| *n == resource_name)
        .map(|(_, tabs)| *tabs)
        .unwrap_or(&[])
}

/// Rows inside a filters/views payload, server order preserved.
///
/// Port of `views._view_list`.
pub fn view_list(data: &Value) -> Vec<&Map<String, Value>> {
    if let Some(items) = data.as_array() {
        return items.iter().filter_map(Value::as_object).collect();
    }
    let Some(obj) = data.as_object() else {
        return Vec::new();
    };
    for (k, v) in obj {
        if let Some(items) = v.as_array()
            && (k.ends_with("filters") || k.ends_with("views"))
        {
            return items.iter().filter_map(Value::as_object).collect();
        }
    }
    Vec::new()
}

pub fn parse_saved_views(data: &Value) -> Vec<SavedView> {
    let mut out = Vec::new();
    for v in view_list(data) {
        let Some(vid) = v
            .get("id")
            .filter(|id| !id.is_null() && id.as_str() != Some(""))
        else {
            continue;
        };
        let marked = v.get("default").is_some_and(fsv_py::truthy);
        let system = marked && v.get("user_id").is_none_or(|id| id.is_null());
        let name = match v.get("name").filter(|v| fsv_py::truthy(v)) {
            Some(n) => fsv_py::py_str(n),
            None => fsv_py::py_str(vid),
        };
        out.push(SavedView {
            id: fsv_py::py_str(vid),
            name,
            system,
            marked_default: marked,
            raw: Value::Object(v.clone()),
        });
    }
    out
}

/// Port of `views.default_view`.
pub fn default_view(views: &[SavedView]) -> Option<&SavedView> {
    views.iter().find(|v| v.marked_default).or(views.first())
}

/// Port of `views.find_view`.
pub fn find_view(views: &[SavedView], text: &str) -> Result<SavedView, QueryError> {
    let want = fold(text);
    if let Some(v) = views.iter().find(|v| fold(&v.id) == want) {
        return Ok(v.clone());
    }
    let exact: Vec<&SavedView> = views.iter().filter(|v| fold(&v.name) == want).collect();
    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    let mut pref: Vec<&SavedView> = if exact.is_empty() {
        views
            .iter()
            .filter(|v| fold(&v.name).starts_with(&want))
            .collect()
    } else {
        exact
    };
    if pref.len() == 1 {
        return Ok(pref[0].clone());
    }
    if pref.is_empty() {
        pref = if want.is_empty() {
            Vec::new()
        } else {
            views
                .iter()
                .filter(|v| fold(&v.name).contains(&want))
                .collect()
        };
        if pref.len() == 1 {
            return Ok(pref[0].clone());
        }
    }
    let shown: Vec<&SavedView> = if pref.is_empty() {
        views.iter().collect()
    } else {
        pref.to_vec()
    };
    let names = shown
        .iter()
        .take(12)
        .map(|v| fsv_py::py_repr_str(&v.name))
        .collect::<Vec<_>>()
        .join(", ");
    if !pref.is_empty() {
        return Err(QueryError(format!(
            "view {} is ambiguous: {names}",
            fsv_py::py_repr_str(text)
        )));
    }
    let msg = if views.is_empty() {
        format!("no view named {}", fsv_py::py_repr_str(text))
    } else {
        format!(
            "no view named {}; views: {names}",
            fsv_py::py_repr_str(text)
        )
    };
    Err(QueryError(msg))
}

/// Port of `views.list_columns`.
pub fn list_columns(
    res: &Resource,
    schema: Option<&Value>,
    headers: Option<&[Value]>,
) -> Vec<Column> {
    if let Some(headers) = headers.filter(|h| !h.is_empty()) {
        let mut picked: Vec<&Value> = headers
            .iter()
            .filter(|h| {
                h.get("selected").is_some_and(fsv_py::truthy)
                    && h.get("name").and_then(Value::as_str) != Some("display_id")
            })
            .collect();
        picked.sort_by_key(|h| position_key(h));
        let mut cols = Vec::new();
        let mut seen = BTreeSet::new();
        for h in picked {
            let raw_name = h
                .get("name")
                .map(fsv_py::py_str)
                .unwrap_or_else(|| "None".to_string());
            let key = HEADER_KEYS
                .iter()
                .find(|(n, _)| *n == raw_name)
                .map(|(_, k)| *k)
                .unwrap_or(raw_name.as_str())
                .to_string();
            if !seen.insert(key.clone()) {
                continue;
            }
            let label = h
                .get("label")
                .filter(|v| fsv_py::truthy(v))
                .map(fsv_py::py_str)
                .unwrap_or_else(|| key.clone());
            cols.push(Column {
                key,
                label: pystr::html_unescape(&label).into_owned(),
            });
        }
        if !cols.is_empty() {
            if !seen.contains("subject") {
                let label =
                    schema_label(schema, "subject").unwrap_or_else(|| "Subject".to_string());
                cols.insert(
                    0,
                    Column {
                        key: "subject".to_string(),
                        label,
                    },
                );
            }
            return cols;
        }
    }
    let keys = COLUMN_KEYS
        .iter()
        .find(|(n, _)| *n == res.name)
        .map(|(_, k)| *k)
        .unwrap_or(COLUMN_KEYS[0].1);
    keys.iter()
        .map(|key| {
            let label = if matches!(
                *key,
                "subject" | "requester" | "status" | "priority" | "department"
            ) {
                schema_label(schema, key)
            } else {
                None
            };
            Column {
                key: (*key).to_string(),
                label: label.unwrap_or_else(|| chrome_label(key)),
            }
        })
        .collect()
}

/// Port of `views.choice_name`.
pub fn choice_name(schema: Option<&Value>, field_name: &str, value: Option<&Value>) -> String {
    let Some(value) = value.filter(|v| !v.is_null() && v.as_str() != Some("")) else {
        return String::new();
    };
    let want = fsv_py::py_str(value);
    for f in fields(schema) {
        if f.get("name").and_then(Value::as_str) != Some(field_name) {
            continue;
        }
        let Some(choices) = f.get("choices").and_then(Value::as_array) else {
            continue;
        };
        for ch in choices.iter().filter_map(Value::as_object) {
            let id = py_get_str(ch.get("id"));
            let ch_value = py_get_str(ch.get("value"));
            if id == want || ch_value == want {
                let text = ch
                    .get("value")
                    .filter(|v| fsv_py::truthy(v))
                    .or_else(|| ch.get("name").filter(|v| fsv_py::truthy(v)))
                    .or_else(|| ch.get("label").filter(|v| fsv_py::truthy(v)))
                    .map(fsv_py::py_str)
                    .unwrap_or_else(|| want.clone());
                return crate::schema::humanize_key(&pystr::html_unescape(&text));
            }
        }
    }
    String::new()
}

/// Port of `views.status_name`.
pub fn status_name(item: &Value, schema: Option<&Value>) -> String {
    for key in [
        "change_status",
        "ticket_status",
        "problem_status",
        "summary_status",
    ] {
        let n = obj_name(item.get(key));
        if !n.is_empty() {
            return n;
        }
    }
    let named = item
        .get("status_name")
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    if !named.is_empty() {
        return named;
    }
    let choice = choice_name(schema, "status", item.get("status"));
    if !choice.is_empty() {
        return choice;
    }
    match item.get("status") {
        Some(v) if !v.is_null() => fsv_py::py_str(v),
        _ => String::new(),
    }
}

/// Port of `views.priority_name`.
pub fn priority_name(item: &Value, schema: Option<&Value>) -> String {
    let v = item.get("priority");
    let named = item
        .get("priority_name")
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    if !named.is_empty() {
        return named;
    }
    let choice = choice_name(schema, "priority", v);
    if !choice.is_empty() {
        return choice;
    }
    // runtime choices first; the static platform labels only without them
    crate::schema::display_label("priority", v, schema.filter(|s| s.is_object()), "")
}

/// Port of `views.department_name`.
pub fn department_name(item: &Value, schema: Option<&Value>) -> String {
    let named = obj_name(item.get("department"));
    if !named.is_empty() {
        return named;
    }
    let direct = item
        .get("department_name")
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    if !direct.is_empty() {
        return direct;
    }
    choice_name(schema, "department", item.get("department_id"))
}

/// Port of `views.requester_name`.
pub fn requester_name(item: &Value) -> String {
    let named = obj_name(item.get("requester"));
    if !named.is_empty() {
        return named;
    }
    item.get("requester_name")
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

/// `(id, display name)`. `id` is `None` when the record has no agent id.
///
/// Port of `views.agent_of`.
pub fn agent_of(item: &Value) -> (Option<Value>, String) {
    for key in ["agent", "responder", "owner"] {
        let Some(obj) = item.get(key).and_then(Value::as_object) else {
            continue;
        };
        let name_ok = obj.get("name").is_some_and(fsv_py::truthy);
        let id_set = obj.get("id").is_some_and(|v| !v.is_null());
        if !(name_ok || id_set) {
            continue;
        }
        let id = if obj.contains_key("id") {
            obj.get("id").filter(|v| !v.is_null()).cloned()
        } else {
            first_truthy(item, &["agent_id", "responder_id"]).cloned()
        };
        return (id, obj_name(Some(&Value::Object(obj.clone()))));
    }
    let id = first_truthy(item, &["agent_id", "responder_id"]).cloned();
    let nm = ["owner_name", "responder_name", "agent_name"]
        .iter()
        .find_map(|k| item.get(*k).filter(|v| fsv_py::truthy(v)))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    (id, nm)
}

/// Port of `views.group_name_of`.
pub fn group_name_of(item: &Value, schema: Option<&Value>) -> String {
    let named = obj_name(item.get("group"));
    if !named.is_empty() {
        return named;
    }
    let direct = item
        .get("group_name")
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    if !direct.is_empty() {
        return direct;
    }
    choice_name(schema, "group", item.get("group_id"))
}

/// Port of `views.assigned_to`.
pub fn assigned_to(item: &Value, me_id: Option<&Value>, schema: Option<&Value>) -> String {
    let (aid, mut agent) = agent_of(item);
    if !agent.is_empty()
        && let (Some(me), Some(aid)) = (me_id.filter(|v| !v.is_null()), aid.as_ref())
        && fsv_py::py_str(aid) == fsv_py::py_str(me)
    {
        agent.push_str(" (Me)");
    }
    let grp = group_name_of(item, schema);
    let grp = pystr::strip(&grp);
    if !grp.is_empty() {
        return format!(
            "{grp} / {}",
            if agent.is_empty() {
                "None"
            } else {
                agent.as_str()
            }
        );
    }
    if agent.is_empty() {
        "None".to_string()
    } else {
        agent
    }
}

/// Port of `views.ticket_state`.
pub fn ticket_state(item: &Value) -> String {
    let raw = item.get("state");
    let st = match raw {
        Some(Value::Object(map)) => map
            .get("name")
            .filter(|v| fsv_py::truthy(v))
            .or_else(|| map.get("state").filter(|v| fsv_py::truthy(v))),
        other => other,
    };
    let Some(st) = st.filter(|v| fsv_py::truthy(v)) else {
        return String::new();
    };
    let text = fsv_py::py_str(st);
    TICKET_STATES
        .iter()
        .find(|(k, _)| *k == text)
        .map(|(_, v)| (*v).to_string())
        .unwrap_or_else(|| py_capitalize(&text.replace('_', " ")))
}

/// Port of `views.cell`.
pub fn cell(
    item: &Value,
    key: &str,
    _res: &Resource,
    schema: Option<&Value>,
    me_id: Option<&Value>,
    tz: Option<&jiff::tz::TimeZone>,
) -> String {
    match key {
        "subject" => {
            return item
                .get("subject")
                .filter(|v| fsv_py::truthy(v))
                .map(fsv_py::py_str)
                .unwrap_or_default();
        }
        "requester" => return requester_name(item),
        "status" => return status_name(item, schema),
        "priority" => return priority_name(item, schema),
        "assigned_to" => return assigned_to(item, me_id, schema),
        "department" => return department_name(item, schema),
        "state" => return ticket_state(item),
        "group" | "group_id" => return group_name_of(item, schema),
        "agent" | "responder" | "agent_id" | "responder_id" => return agent_of(item).1,
        _ => {}
    }
    let mut raw = item.get(key).filter(|v| !v.is_null());
    if raw.is_none() {
        raw = item
            .get("custom_fields")
            .filter(|v| fsv_py::truthy(v))
            .and_then(|c| c.get(key))
            .filter(|v| !v.is_null());
    }
    let Some(raw) = raw else {
        return String::new();
    };
    if let Some(text) = raw.as_str()
        && parse_dt(raw).is_some()
        && (text.contains('T') || is_date_only(raw))
    {
        return fmt_when(raw, tz, true);
    }
    if !matches!(raw, Value::Array(_) | Value::Object(_)) {
        let label = choice_name(schema, key, Some(raw));
        if !label.is_empty() {
            return label;
        }
    }
    match raw {
        Value::Object(_) => obj_name(Some(raw)),
        Value::Array(items) => items
            .iter()
            .map(|x| {
                let n = obj_name(Some(x));
                if n.is_empty() { fsv_py::py_str(x) } else { n }
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::Bool(true) => "Yes".to_string(),
        Value::Bool(false) => "No".to_string(),
        other => fsv_py::py_str(other),
    }
}

/// Port of `views.list_row`.
pub fn list_row(
    item: &Value,
    res: &Resource,
    columns: &[Column],
    schema: Option<&Value>,
    me_id: Option<&Value>,
    tz: Option<&jiff::tz::TimeZone>,
) -> Map<String, Value> {
    let mut row = Map::new();
    row.insert(
        "id".to_string(),
        Value::String(resource::format_id(item, res)),
    );
    for col in columns {
        row.insert(
            col.key.clone(),
            Value::String(cell(item, &col.key, res, schema, me_id, tz)),
        );
    }
    row
}

/// Port of `views.sort_label`.
pub fn sort_label(order_by: Option<&str>, schema: Option<&Value>) -> String {
    let Some(order_by) = order_by.filter(|s| !s.is_empty()) else {
        return "Created Date".to_string();
    };
    if let Some((_, label)) = SORT_LABELS.iter().find(|(k, _)| *k == order_by) {
        return (*label).to_string();
    }
    let base = order_by.strip_suffix("_id").unwrap_or(order_by);
    schema_label(schema, base)
        .or_else(|| schema_label(schema, order_by))
        .unwrap_or_else(|| py_capitalize(&base.replace('_', " ")))
}

/// Port of `views.list_meta`.
pub fn list_meta(data: &Value, item_count: usize, page: i64, per_page: i64) -> (Option<i64>, bool) {
    let empty = Map::new();
    let data_map = data.as_object().unwrap_or(&empty);
    let meta = data_map
        .get("meta")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut total = meta
        .get("total_count")
        .or_else(|| meta.get("count"))
        .or_else(|| data_map.get("total"))
        .filter(|v| !v.is_null())
        .and_then(py_int);
    let floor = (item_count as i64) + (page - 1) * per_page;
    if total.is_some_and(|t| t < floor) {
        total = None;
    }
    // `meta.get` treats a missing key and null the same. The data-level
    // lookup is `data.get("has_next", data.get("has_more"))`: an explicit
    // null `has_next` stays unset and does not consult `has_more`.
    let explicit = first_present(meta, &["has_next", "has_more"]).or_else(|| {
        if data_map.contains_key("has_next") {
            data_map.get("has_next").filter(|v| !v.is_null())
        } else {
            data_map.get("has_more").filter(|v| !v.is_null())
        }
    });
    let has_next = if let Some(v) = explicit {
        fsv_py::truthy(v)
    } else if data_map.contains_key("next_page_url") || meta.contains_key("next_page_url") {
        data_map
            .get("next_page_url")
            .or_else(|| meta.get("next_page_url"))
            .is_some_and(fsv_py::truthy)
    } else {
        match total {
            Some(t) => (page - 1) * per_page + (item_count as i64) < t,
            None => (item_count as i64) >= per_page,
        }
    };
    (total, has_next)
}

/// Port of `views.canonical_tab`.
pub fn canonical_tab(name: &str) -> String {
    let n = pystr::strip(name).to_lowercase().replace(' ', "_");
    if let Some((_, v)) = TAB_ALIASES.iter().find(|(k, _)| *k == n) {
        return (*v).to_string();
    }
    let dashed = n.replace('_', "-");
    if let Some((_, v)) = TAB_ALIASES.iter().find(|(k, _)| *k == dashed) {
        return (*v).to_string();
    }
    n
}

pub fn is_known_tab(name: &str) -> bool {
    TAB_LABELS.iter().any(|(k, _)| *k == name)
}

/// Port of `views.tab_label`.
pub fn tab_label(name: &str) -> String {
    TAB_LABELS
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| (*v).to_string())
        .unwrap_or_else(|| py_capitalize(&name.replace('_', " ")))
}

/// Port of `views.parse_tabs`.
pub fn parse_tabs(data: &Value) -> Vec<Tab> {
    let rows = if data.is_array() {
        data.as_array()
    } else {
        data.as_object()
            .and_then(|m| m.get("tabs"))
            .and_then(Value::as_array)
    };
    let mut tabs = vec![Tab {
        name: "details".to_string(),
        label: tab_label("details"),
        count: None,
    }];
    for t in rows.into_iter().flatten() {
        let Some(obj) = t.as_object() else { continue };
        let Some(name) = obj
            .get("name")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
        else {
            continue;
        };
        if DETAILS_PARTS.contains(&name.as_str()) {
            continue;
        }
        let count = obj.get("count").and_then(as_py_int);
        tabs.push(Tab {
            name: name.clone(),
            label: tab_label(&name),
            count,
        });
    }
    tabs
}

/// Port of `views._fold`.
fn fold(s: &str) -> String {
    pystr::casefold(&pystr::collapse_ws(s))
}

fn chrome_label(key: &str) -> String {
    CHROME_LABELS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| (*v).to_string())
        .unwrap_or_else(|| key.to_string())
}

/// Port of `views._schema_label`.
pub(crate) fn schema_label(schema: Option<&Value>, name: &str) -> Option<String> {
    for f in fields(schema) {
        if f.get("name").and_then(Value::as_str) == Some(name)
            && let Some(label) = f.get("label").filter(|v| fsv_py::truthy(v))
        {
            return Some(pystr::html_unescape(&fsv_py::py_str(label)).into_owned());
        }
    }
    None
}

fn fields(schema: Option<&Value>) -> Vec<&Map<String, Value>> {
    schema
        .and_then(|s| s.get("fields"))
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_object).collect())
        .unwrap_or_default()
}

fn position_key(h: &Value) -> i64 {
    h.get("position")
        .filter(|v| fsv_py::truthy(v))
        .and_then(py_int)
        .unwrap_or_default()
}

pub(crate) fn obj_name(obj: Option<&Value>) -> String {
    let Some(map) = obj.and_then(Value::as_object) else {
        return String::new();
    };
    for key in ["name", "value", "email"] {
        if let Some(v) = map.get(key).filter(|v| fsv_py::truthy(v)) {
            return pystr::strip(&fsv_py::py_str(v)).to_string();
        }
    }
    String::new()
}

fn py_get_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".to_string(),
        Some(v) => fsv_py::py_str(v),
    }
}

fn first_truthy<'a>(item: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|k| item.get(*k).filter(|v| fsv_py::truthy(v)))
}

fn first_present<'a>(map: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|k| map.get(*k).filter(|v| !v.is_null()))
}

fn as_py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok())),
        _ => None,
    }
}

/// `int(v)` for a JSON number or a base-10 string. Anything else is `None`
/// (`int("30.5")` fails in Python too).
fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok()))
            .or_else(|| n.as_f64().map(|f| f as i64)),
        // `int(" 1")` raises; do not trim.
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub(crate) fn py_capitalize(s: &str) -> String {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out: String = first.to_uppercase().collect();
    out.extend(chars.flat_map(|c| c.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fsv_core::resource::CHANGES;
    use serde_json::json;

    fn oracle() -> Value {
        serde_json::from_str(include_str!("../testdata/coverage_oracle.json")).unwrap()
    }

    fn rows(v: &Value) -> &Vec<Value> {
        v.as_array().unwrap()
    }

    fn cols(c: &[Column]) -> Value {
        json!(c.iter().map(|c| [&c.key, &c.label]).collect::<Vec<_>>())
    }

    #[test]
    fn small_types_to_json_match_python() {
        let o = oracle();
        let v = &o["views"];
        for case in rows(&v["missing"]) {
            let m = Missing::new("tasks", case["status"].as_i64());
            assert_eq!(m.note(), case["note"].as_str().unwrap(), "{case}");
            assert_eq!(m.to_json(), case["json"], "{case}");
        }
        let sv = SavedView {
            id: "7".into(),
            name: "Mine".into(),
            system: false,
            marked_default: true,
            raw: json!({}),
        };
        assert_eq!(sv.to_json(), v["saved_view"]);
        let tabs = [
            Tab {
                name: "notes".into(),
                label: "Notes".into(),
                count: Some(3),
            },
            Tab {
                name: "tasks".into(),
                label: "Tasks".into(),
                count: None,
            },
        ];
        assert_eq!(
            json!(tabs.iter().map(Tab::to_json).collect::<Vec<_>>()),
            v["tab"]
        );
    }

    #[test]
    fn view_list_fold_and_labels_match_python() {
        let o = oracle();
        let v = &o["views"];
        let schema = Some(&o["schema"]);
        for case in rows(&v["view_list"]) {
            let got: Vec<Value> = view_list(&case[0])
                .into_iter()
                .map(|m| Value::Object(m.clone()))
                .collect();
            assert_eq!(json!(got), case[1], "{}", case[0]);
        }
        for case in rows(&v["fold"]) {
            assert_eq!(fold(case[0].as_str().unwrap()), case[1], "{}", case[0]);
        }
        for case in rows(&v["schema_label"]) {
            assert_eq!(
                json!(schema_label(schema, case[0].as_str().unwrap())),
                case[1],
                "{}",
                case[0]
            );
        }
        assert_eq!(json!(schema_label(None, "status")), v["schema_label_none"]);
        for case in rows(&v["sort_label"]) {
            assert_eq!(
                sort_label(case[0].as_str(), schema),
                case[1].as_str().unwrap(),
                "{}",
                case[0]
            );
        }
    }

    #[test]
    fn list_columns_match_python() {
        let o = oracle();
        let schema = Some(&o["schema"]);
        let want = &o["views"]["list_columns"];
        let headers = json!([
            {"name": "display_id", "label": "ID", "selected": true, "position": 0},
            {"name": "requester", "label": "Req &amp; X", "selected": true, "position": 3},
            {"name": "status", "label": "Status", "selected": true, "position": 2},
            {"name": "priority", "label": "Prio", "selected": false, "position": 1},
            {"name": "status", "label": "Again", "selected": true, "position": 4},
            {"name": "cf_x", "selected": true},
        ]);
        let mut with_subject = rows(&headers).clone();
        with_subject.push(
            json!({"name": "ticket_subject", "label": "Title", "selected": true, "position": 1}),
        );
        let unselected = [json!({"name": "x", "selected": false})];
        let tickets = &fsv_core::resource::TICKETS;
        type Case<'a> = (
            &'a str,
            &'a Resource,
            Option<&'a Value>,
            Option<&'a [Value]>,
        );
        let cases: [Case; 6] = [
            ("changes_default", &CHANGES, None, None),
            ("changes_schema", &CHANGES, schema, None),
            ("tickets_default", tickets, None, None),
            ("tickets_headers", tickets, schema, Some(rows(&headers))),
            (
                "tickets_headers_subject",
                tickets,
                schema,
                Some(&with_subject),
            ),
            ("tickets_unselected", tickets, None, Some(&unselected)),
        ];
        for (name, res, schema, headers) in cases {
            assert_eq!(
                cols(&list_columns(res, schema, headers)),
                want[name],
                "{name}"
            );
        }
    }

    #[test]
    fn record_names_match_python() {
        let o = oracle();
        let v = &o["views"];
        let schema = Some(&o["schema"]);
        for case in rows(&v["choice_name"]) {
            let value = (!case[1].is_null()).then_some(&case[1]);
            assert_eq!(
                choice_name(schema, case[0].as_str().unwrap(), value),
                case[2].as_str().unwrap(),
                "{case}"
            );
        }
        for case in rows(&v["status_name"]) {
            assert_eq!(status_name(&case[0], schema), case[1], "{}", case[0]);
        }
        for case in rows(&v["department_name"]) {
            assert_eq!(department_name(&case[0], schema), case[1], "{}", case[0]);
        }
        for case in rows(&v["requester_name"]) {
            assert_eq!(requester_name(&case[0]), case[1], "{}", case[0]);
        }
        for case in rows(&v["agent_of"]) {
            let (id, name) = agent_of(&case[0]);
            assert_eq!(json!([id, name]), case[1], "{}", case[0]);
        }
        for case in rows(&v["group_name_of"]) {
            assert_eq!(group_name_of(&case[0], schema), case[1], "{}", case[0]);
        }
    }

    #[test]
    fn list_row_matches_python() {
        let o = oracle();
        let v = &o["views"];
        let schema = Some(&o["schema"]);
        let columns = list_columns(&CHANGES, schema, None);
        assert_eq!(cols(&columns), v["list_row_columns"]);
        for case in rows(&v["list_row"]) {
            let row = list_row(&case[0], &CHANGES, &columns, schema, Some(&json!(21)), None);
            assert_eq!(Value::Object(row), case[1], "{}", case[0]);
        }
    }

    #[test]
    fn list_meta_trusts_explicit_flags() {
        assert_eq!(
            list_meta(&json!({"meta": {"has_next": true}}), 1, 1, 30),
            (None, true)
        );
        assert_eq!(
            list_meta(&json!({"meta": {"has_next": false}}), 30, 1, 30),
            (None, false)
        );
        assert_eq!(
            list_meta(&json!({"next_page_url": "?page=2"}), 1, 1, 30),
            (None, true)
        );
        assert_eq!(
            list_meta(&json!({"meta": {"total_count": 30}}), 30, 1, 30),
            (Some(30), false)
        );
        // Explicit null has_next does not fall through to has_more.
        assert_eq!(
            list_meta(&json!({"has_next": null, "has_more": true}), 1, 1, 30),
            (None, false)
        );
        assert_eq!(
            list_meta(&json!({"has_more": true}), 1, 1, 30),
            (None, true)
        );
    }

    #[test]
    fn name_strip_and_bool_priority_match_python() {
        assert_eq!(obj_name(Some(&json!({"name": "\u{1c}Ada\u{1c}"}))), "Ada");
        assert_eq!(priority_name(&json!({"priority": true}), None), "Low");
        assert_eq!(priority_name(&json!({"priority": false}), None), "False");
        assert_eq!(canonical_tab("\u{1c}Details"), "details");
    }

    #[test]
    fn later_page_header_without_a_total() {
        let page = |n: usize, has_next: bool, total: Option<i64>| {
            ListPage {
                resource_name: CHANGES.name.to_string(),
                items: vec![json!({}); n],
                columns: vec![],
                rows: vec![],
                view_name: None,
                page: 2,
                per_page: 30,
                total,
                has_next,
                order_by: "created_at".into(),
                order_type: "desc".into(),
                sort: "Created Date".into(),
                notes: vec![],
            }
            .header()
        };
        assert!(
            page(3, false, None).contains("31-33 of 33 ·"),
            "{}",
            page(3, false, None)
        );
        assert!(page(30, true, None).contains("31-60 of 60+ ·"));
        assert!(page(30, true, Some(75)).contains("31-60 of 75 ·"));
    }

    /// `ListPage.title`: the view name, else `All <resource>`.
    #[test]
    fn list_page_title_is_view_or_all() {
        let mut page = ListPage {
            resource_name: CHANGES.name.to_string(),
            items: vec![],
            columns: vec![],
            rows: vec![],
            view_name: None,
            page: 1,
            per_page: 30,
            total: None,
            has_next: false,
            order_by: "created_at".into(),
            order_type: "desc".into(),
            sort: "Created Date".into(),
            notes: vec![],
        };
        assert_eq!(page.title(), "All changes");
        page.view_name = Some("Mine".into());
        assert_eq!(page.title(), "Mine");
    }

    #[test]
    fn find_view_id_prefix_and_ambiguous() {
        let views = parse_saved_views(&json!({"filters": [
            {"id": 1, "name": "All Changes", "default": true},
            {"id": 2, "name": "My Open", "default": false, "user_id": 9},
            {"id": 3, "name": "My Closed", "default": false, "user_id": 9}
        ]}));
        assert!(views[0].system && views[0].marked_default);
        assert!(!views[1].system);
        assert_eq!(default_view(&views).unwrap().id, "1");
        assert_eq!(find_view(&views, "2").unwrap().name, "My Open");
        assert_eq!(find_view(&views, "my o").unwrap().name, "My Open");
        let err = find_view(&views, "my").unwrap_err().0;
        assert!(err.contains("ambiguous"), "{err}");
        let missing = find_view(&views, "nope").unwrap_err().0;
        assert!(missing.starts_with("no view named 'nope'"), "{missing}");
    }

    #[test]
    fn assigned_marks_me_and_ticket_state() {
        let item = json!({
            "group": {"name": "Network"},
            "agent": {"id": 7, "name": "Ada"},
            "state": "overdue",
            "priority": 4
        });
        assert_eq!(
            assigned_to(&item, Some(&json!(7)), None),
            "Network / Ada (Me)"
        );
        assert_eq!(assigned_to(&item, Some(&json!(8)), None), "Network / Ada");
        assert_eq!(ticket_state(&item), "Overdue");
        assert_eq!(priority_name(&item, None), "Urgent");
        assert_eq!(
            ticket_state(&json!({"state": "waiting_on_customer"})),
            "Waiting on customer"
        );
    }

    #[test]
    fn tabs_skip_detail_parts_and_alias() {
        let tabs = parse_tabs(&json!({"tabs": [
            {"name": "notes", "count": 2},
            {"name": "conversations"},
            {"name": "details"}
        ]}));
        assert_eq!(tabs[0].name, "details");
        assert_eq!(tabs[1].name, "notes");
        assert_eq!(tabs[1].count, Some(2));
        assert_eq!(tabs.len(), 2);
        assert_eq!(canonical_tab("Related Tickets"), "related_tickets");
        assert_eq!(tab_label("time_entries"), "Time entries");
    }
}
