//! Freshservice record types and display IDs (`resources.py`).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::value::{get, truthy};

/// Static description of a record type.
#[derive(Debug, PartialEq, Eq)]
pub struct Resource {
    /// Canonical key (`changes`).
    pub name: &'static str,
    /// URL path segment.
    pub api_path: &'static str,
    /// JSON list field.
    pub list_key: &'static str,
    /// JSON detail field.
    pub item_key: &'static str,
    /// v2 schema endpoint.
    pub form_fields_path: &'static str,
    /// JSON key in the schema response.
    pub form_fields_key: &'static str,
    /// `/api/_/<this>` saved filters endpoint.
    pub filters_path: Option<&'static str>,
    pub display_prefixes: &'static [&'static str],
    pub sub_resources: &'static [&'static str],
    /// CSV for `?include=` on the list endpoint.
    pub list_include: &'static str,
    pub has_notes: bool,
}

impl Resource {
    /// `https://<domain>/a/<api_path>`.
    pub fn portal_url(&self) -> String {
        format!("https://{}/a/{}", crate::config::domain(), self.api_path)
    }
}

pub static CHANGES: Resource = Resource {
    name: "changes",
    api_path: "changes",
    list_key: "changes",
    item_key: "change",
    form_fields_path: "change_form_fields",
    form_fields_key: "change_fields",
    filters_path: Some("change-filters"),
    display_prefixes: &["CHN"],
    sub_resources: &["notes", "tasks", "activities", "approvals", "time_entries"],
    list_include: "change_status,agent,requester,group,department",
    has_notes: true,
};

pub static TICKETS: Resource = Resource {
    name: "tickets",
    api_path: "tickets",
    list_key: "tickets",
    item_key: "ticket",
    form_fields_path: "ticket_form_fields",
    form_fields_key: "ticket_fields",
    filters_path: Some("ticket_filters"),
    display_prefixes: &["INC", "SR"],
    sub_resources: &["conversations", "tasks", "activities", "time_entries"],
    list_include: "stats,responder,requester,ticket_states,ticket_status,group",
    has_notes: false,
};

pub static PROBLEMS: Resource = Resource {
    name: "problems",
    api_path: "problems",
    list_key: "problems",
    item_key: "problem",
    form_fields_path: "problem_form_fields",
    form_fields_key: "problem_fields",
    filters_path: Some("problem-filters"),
    display_prefixes: &["PRB"],
    sub_resources: &["notes", "tasks", "activities", "time_entries"],
    list_include: "problem_status,agent,requester,group",
    has_notes: true,
};

/// All resources in registry order.
pub static REGISTRY: [&Resource; 3] = [&CHANGES, &TICKETS, &PROBLEMS];

/// `REGISTRY[name]`.
pub fn lookup(name: &str) -> Option<&'static Resource> {
    REGISTRY.iter().copied().find(|r| r.name == name)
}

/// Accept `INC-123`, `SR-123`, `CHN-123`, `PRB-123`, or bare `123`.
pub fn parse_id(s: &str, resource: &Resource) -> Result<i64> {
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:([A-Z]+)-)?(\d+)\n?$").expect("regex"));
    let s = crate::pystr::strip(s);
    let caps = ID.captures(s).ok_or_else(|| Error::value(format!("invalid id: {s}")))?;
    if let Some(prefix) = caps.get(1) {
        let prefix = prefix.as_str().to_uppercase();
        if !resource.display_prefixes.contains(&prefix.as_str()) {
            return Err(Error::value(format!(
                "prefix {prefix} not valid for {} (expect {})",
                resource.name,
                resource.display_prefixes.join(" or ")
            )));
        }
    }
    let num = caps.get(2).map_or("", |m| m.as_str());
    crate::pystr::parse_decimal(num).ok_or_else(|| Error::value(format!("invalid id: {s}")))
}

/// Display ID (`human_display_id`, else `<PREFIX>-<display_id|id>`).
pub fn format_id(item: &Value, resource: &Resource) -> String {
    if let Some(hid) = get(item, "human_display_id").filter(|v| truthy(v)) {
        return fsv_py::py_str(hid);
    }
    let did = crate::value::first_truthy(item, &["display_id"]).or_else(|| get(item, "id")).unwrap_or(&Value::Null);
    format!("{}-{}", resource.display_prefixes[0], fsv_py::py_str(did))
}
