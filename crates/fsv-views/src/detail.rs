//! One record's site page: tab contents and the assembled [`Detail`]
//! (`views.TabData`, `load_tab`, `Detail`, `load_detail`).

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

use fsv_api::QueryError;
use fsv_api::service::Api;
use fsv_api::site::{
    self, Lifecycle, Missing, PlanningEntry, PlanningOutcome, Prop, Tab, canonical_tab,
    html_to_text, tab_label,
};
use fsv_core::resource::{self, Resource};

use crate::render::requested_item_json;
use crate::source::{
    AssociationReader, FetchError, MAX_DETAIL_PAGES, SessionError, Source, collect_pages,
    get_requested_items, load_conversations, obj, optional_get, requested_for, rows_of, v2_read,
    workspace_get,
};

/// One tab's content as the web loads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabData {
    pub name: String,
    pub label: String,
    pub data: Option<Value>,
    pub missing: Option<Missing>,
    /// Sub-lists in server order (associations, devops, related tickets, risk survey).
    pub parts: Map<String, Value>,
    pub notes: Vec<String>,
    pub meta: Map<String, Value>,
    pub parts_meta: Map<String, Value>,
}

impl TabData {
    pub fn new(name: &str) -> Self {
        TabData {
            name: name.to_string(),
            label: tab_label(name),
            ..TabData::default()
        }
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("name".into(), json!(self.name));
        out.insert("label".into(), json!(self.label));
        if !self.meta.is_empty() {
            out.insert("meta".into(), Value::Object(self.meta.clone()));
        }
        if !self.parts_meta.is_empty() {
            out.insert("parts_meta".into(), Value::Object(self.parts_meta.clone()));
        }
        if let Some(miss) = &self.missing
            && let Value::Object(m) = miss.to_json()
        {
            out.extend(m);
        }
        if let Some(data) = &self.data {
            out.insert("data".into(), data.clone());
        }
        if !self.parts.is_empty() {
            out.insert("parts".into(), Value::Object(self.parts.clone()));
        }
        if !self.notes.is_empty() {
            out.insert("notes".into(), json!(self.notes));
        }
        Value::Object(out)
    }
}

fn rid_of(record: &Value) -> String {
    let rid = ["display_id", "id"]
        .iter()
        .find_map(|k| record.get(*k).filter(|v| fsv_py::truthy(v)));
    rid.map(fsv_py::py_str).unwrap_or_else(|| "None".into())
}

fn simple(
    src: &dyn Api,
    td: &mut TabData,
    path: &str,
    params: Value,
    key: &str,
) -> Result<(), SessionError> {
    let got = collect_pages(
        src,
        path,
        key,
        &obj(params),
        &td.label.clone(),
        MAX_DETAIL_PAGES,
    )?;
    td.meta = got.meta;
    let pages = td.meta.get("pages").is_some_and(fsv_py::truthy);
    td.missing = if pages { None } else { got.failure };
    td.data = if td.missing.is_none() {
        Some(Value::Array(got.rows))
    } else {
        None
    };
    if let Some(p) = td.meta.get("partial").filter(|v| fsv_py::truthy(v)) {
        td.notes.push(fsv_py::py_str(p));
    }
    Ok(())
}

type PartSpec<'a> = (&'a str, String, Value, &'a str);

/// `_parts`: several lists under one tab; the tab is missing only when every list failed.
fn parts(src: &dyn Api, td: &mut TabData, specs: Vec<PartSpec<'_>>) -> Result<(), SessionError> {
    let mut failures: Vec<Missing> = Vec::new();
    for (label, path, params, key) in specs {
        let got = collect_pages(src, &path, key, &obj(params), label, MAX_DETAIL_PAGES)?;
        if let Some(p) = got.meta.get("partial").filter(|v| fsv_py::truthy(v)) {
            td.notes.push(fsv_py::py_str(p));
        }
        let pages = got.meta.get("pages").is_some_and(fsv_py::truthy);
        td.parts_meta
            .insert(label.to_string(), Value::Object(got.meta));
        if let Some(miss) = got.failure
            && !pages
        {
            if miss.status != Some(404) {
                td.notes.push(miss.note());
            }
            failures.push(miss);
            continue;
        }
        td.parts.insert(label.to_string(), Value::Array(got.rows));
    }
    if td.parts.is_empty()
        && let Some(first) = failures.first()
    {
        td.missing = Some(Missing::new(td.label.clone(), first.status));
    }
    Ok(())
}

/// `load_tab`: one site tab through the endpoints the web calls.
pub fn load_tab(
    src: &mut dyn Source,
    res: &Resource,
    record: &Value,
    name: &str,
) -> Result<TabData, SessionError> {
    tab(src.api(), res, record, name)
}

fn tab(src: &dyn Api, res: &Resource, record: &Value, name: &str) -> Result<TabData, SessionError> {
    let name = canonical_tab(name);
    let base = format!("{}/{}", res.api_path, rid_of(record));
    let mut td = TabData::new(&name);
    let label = td.label.clone();
    let ws = record.get("workspace_id");
    match name.as_str() {
        "notes" => simple(
            src,
            &mut td,
            &format!("{base}/notes"),
            json!({"include": "user", "page": 1, "per_page": 30}),
            "notes",
        )?,
        "tasks" => simple(
            src,
            &mut td,
            &format!("{base}/tasks"),
            json!({"page": 1, "per_page": 100}),
            "tasks",
        )?,
        "approvals" => simple(
            src,
            &mut td,
            &format!("{base}/approvals"),
            json!({}),
            "approvals",
        )?,
        "activity" => simple(
            src,
            &mut td,
            &format!("{base}/activities"),
            json!({"page": 1, "per_page": 30}),
            "activities",
        )?,
        "time_entries" => simple(
            src,
            &mut td,
            &format!("{base}/time_entries"),
            json!({}),
            "time_entries",
        )?,
        "assets" if res.name == "tickets" => simple(
            src,
            &mut td,
            &format!("{base}/associated_assets"),
            json!({}),
            "associated_assets",
        )?,
        "assets" => match optional_get(src, &format!("{base}/assets"), None, &label)? {
            Err(miss) => td.missing = Some(miss),
            Ok(data) => {
                td.data = Some(Value::Array(first_rows(
                    &data,
                    &["associated_assets", "assets"],
                )))
            }
        },
        "impacted_services" => {
            let params = obj(json!({"filter": "impacted_services"}));
            match optional_get(src, &format!("{base}/assets"), Some(&params), &label)? {
                Err(miss) => td.missing = Some(miss),
                Ok(data) => {
                    td.data = Some(Value::Array(first_rows(
                        &data,
                        &[
                            "associated_impacted_services",
                            "associated_assets",
                            "assets",
                        ],
                    )))
                }
            }
        }
        "associations" => {
            if res.name == "changes" {
                let specs = vec![
                    (
                        "Releases",
                        format!("{base}/releases"),
                        json!({"per_page": 50}),
                        "releases",
                    ),
                    (
                        "Tickets",
                        format!("{base}/tickets"),
                        json!({"per_page": 50}),
                        "tickets",
                    ),
                    (
                        "Archived tickets",
                        format!("{base}/tickets"),
                        json!({"filter": "archived", "per_page": 50}),
                        "tickets",
                    ),
                    (
                        "Tickets caused by this change",
                        format!("{base}/tickets"),
                        json!({"filter": "associated_tickets_caused", "per_page": 50}),
                        "tickets",
                    ),
                    (
                        "Problems",
                        format!("{base}/problems"),
                        json!({"per_page": 50}),
                        "problems",
                    ),
                ];
                parts(&AssociationReader(src), &mut td, specs)?;
            } else {
                let specs = if res.name == "tickets" {
                    vec![
                        (
                            "Changes",
                            format!("{base}/changes"),
                            json!({"change_type": "change"}),
                            "changes",
                        ),
                        (
                            "Changes that caused this",
                            format!("{base}/changes"),
                            json!({"change_type": "change_cause"}),
                            "changes",
                        ),
                        (
                            "Problems",
                            format!("{base}/problems"),
                            json!({}),
                            "problems",
                        ),
                    ]
                } else {
                    vec![
                        ("Changes", format!("{base}/changes"), json!({}), "changes"),
                        ("Tickets", format!("{base}/tickets"), json!({}), "tickets"),
                    ]
                };
                parts(src, &mut td, specs)?;
            }
        }
        "devops" => {
            let specs = vec![
                (
                    "Work items",
                    format!("{base}/work-items"),
                    json!({"page": 1, "per_page": 5}),
                    "work_items",
                ),
                (
                    "Pull requests",
                    format!("{base}/pull-requests"),
                    json!({"page": 1, "per_page": 5}),
                    "pull_requests",
                ),
                (
                    "Pipeline runs",
                    format!("{base}/pipeline-runs"),
                    json!({"page": 1, "per_page": 5}),
                    "pipeline_runs",
                ),
            ];
            parts(src, &mut td, specs)?;
        }
        "risk" => match optional_get(src, &format!("{base}/risk-analysis"), None, &label)? {
            Err(miss) => td.missing = Some(miss),
            Ok(data) => {
                td.data = Some(if data.is_object() { data } else { json!({}) });
                let mut params = obj(json!({"module": "risk"}));
                if let Some(ws) = ws.filter(|v| !v.is_null() && v.as_str() != Some("")) {
                    params.insert("workspace_id".into(), ws.clone());
                }
                if let Ok(survey) =
                    optional_get(src, &format!("{base}/survey"), Some(&params), "risk survey")?
                    && fsv_py::truthy(&survey)
                {
                    td.parts.insert("survey".into(), survey);
                }
            }
        },
        "related_tickets" => {
            let params = obj(json!({"successor_type": "active"}));
            match optional_get(src, &format!("{base}/mappings"), Some(&params), &label)? {
                Err(miss) => td.missing = Some(miss),
                Ok(data) if data.is_object() => {
                    let parent = match data.get("predecessor_ticket") {
                        Some(p) if p.is_object() => vec![p.clone()],
                        _ => Vec::new(),
                    };
                    td.parts.insert("Parent".into(), Value::Array(parent));
                    td.parts.insert(
                        "Child tickets".into(),
                        Value::Array(rows_of(&data, "successor_tickets")),
                    );
                }
                Ok(_) => {}
            }
        }
        "resolution_notes" => resolution(src, &mut td, record, &base)?,
        _ => td.missing = Some(Missing::new(label, None)),
    }
    Ok(td)
}

fn first_rows(data: &Value, keys: &[&str]) -> Vec<Value> {
    keys.iter()
        .map(|k| rows_of(data, k))
        .find(|rows| !rows.is_empty())
        .unwrap_or_default()
}

fn resolution(
    src: &dyn Api,
    td: &mut TabData,
    record: &Value,
    base: &str,
) -> Result<(), SessionError> {
    let stats = record.get("stats").filter(|v| fsv_py::truthy(v));
    let mut info = Map::new();
    for k in ["resolved_at", "closed_at"] {
        if let Some(v) = stats.and_then(|s| s.get(k)).filter(|v| fsv_py::truthy(v)) {
            info.insert(k.into(), v.clone());
        }
    }
    let html = ["resolution_notes", "resolution_notes_html"]
        .iter()
        .find_map(|k| record.get(*k).filter(|v| fsv_py::truthy(v)))
        .cloned()
        .unwrap_or(Value::String(String::new()));
    let mut text = html_to_text(&html);
    if text.is_empty() {
        let data = match v2_read(src, &format!("{base}/resolution_note")) {
            Ok(data) => data,
            Err(FetchError::Session(msg)) => return Err(SessionError(msg)),
            Err(FetchError::Status(Some(404))) => {
                td.meta.insert("absent".into(), json!(true));
                Value::Null
            }
            Err(FetchError::Status(status)) => {
                td.missing = Some(Missing::new(td.label.clone(), status));
                Value::Null
            }
        };
        if let Some(map) = data.as_object() {
            let note = map.get("resolution_note").unwrap_or(&data);
            match note {
                Value::Object(n) => {
                    let body = ["body", "description", "body_text"]
                        .iter()
                        .find_map(|k| n.get(*k).filter(|v| fsv_py::truthy(v)))
                        .cloned()
                        .unwrap_or(Value::String(String::new()));
                    text = html_to_text(&body);
                    if !["body", "description", "body_text"]
                        .iter()
                        .any(|k| n.contains_key(*k))
                    {
                        td.missing = Some(Missing::new(td.label.clone(), None));
                    }
                }
                Value::String(s) => text = html_to_text(&Value::String(s.clone())),
                Value::Null if map.contains_key("resolution_note") => {
                    td.meta.insert("absent".into(), json!(true));
                }
                _ => td.missing = Some(Missing::new(td.label.clone(), None)),
            }
        } else if !data.is_null() {
            td.missing = Some(Missing::new(td.label.clone(), None));
        }
    }
    if !text.is_empty() {
        info.insert("note".into(), Value::String(text));
    }
    td.data = Some(Value::Object(info));
    Ok(())
}

/// Everything the site's detail page shows for one record.
#[derive(Debug, Clone)]
pub struct Detail {
    pub resource: &'static Resource,
    pub record: Value,
    pub schema: Value,
    pub user: Value,
    pub tz: TimeZone,
    pub tabs: Vec<Tab>,
    pub tabs_missing: Option<Missing>,
    pub lifecycle: Option<Lifecycle>,
    pub lifecycle_missing: Option<Missing>,
    pub planning: Option<Vec<PlanningEntry>>,
    pub planning_missing: Option<Missing>,
    pub properties: Vec<Prop>,
    pub properties_note: Option<String>,
    pub requested_items: Option<Vec<Value>>,
    pub requested_items_missing: Option<Missing>,
    pub conversations: Option<Vec<Value>>,
    pub conversations_meta: Map<String, Value>,
    pub conversations_missing: Option<Missing>,
    pub requested_for: Option<String>,
    /// Loaded tabs in request order.
    pub tab_data: Vec<TabData>,
    /// Clock for "due in"; `None` is the real clock.
    pub now: Option<Timestamp>,
}

impl Detail {
    pub fn new(
        resource: &'static Resource,
        record: Value,
        schema: Value,
        user: Value,
        tz: TimeZone,
        tabs: Vec<Tab>,
    ) -> Self {
        Detail {
            resource,
            record,
            schema,
            user,
            tz,
            tabs,
            tabs_missing: None,
            lifecycle: None,
            lifecycle_missing: None,
            planning: None,
            planning_missing: None,
            properties: Vec::new(),
            properties_note: None,
            requested_items: None,
            requested_items_missing: None,
            conversations: None,
            conversations_meta: Map::new(),
            conversations_missing: None,
            requested_for: None,
            tab_data: Vec::new(),
            now: None,
        }
    }

    pub fn id(&self) -> String {
        resource::format_id(&self.record, self.resource)
    }

    pub fn subject(&self) -> String {
        self.record
            .get("subject")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default()
    }

    pub fn badge(&self) -> String {
        site::status_badge(self.resource, &self.record, Some(&self.schema))
    }

    pub fn me_id(&self) -> Option<&Value> {
        self.user.get("id")
    }

    pub fn due_text(&self) -> String {
        site::fmt_when(
            self.record.get("due_by").unwrap_or(&Value::Null),
            Some(&self.tz),
            true,
        )
    }

    pub fn tab_names(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.name.clone()).collect()
    }

    pub fn tab(&self, name: &str) -> Option<&TabData> {
        self.tab_data.iter().find(|t| t.name == name)
    }

    /// Replace a loaded tab in place, or append it.
    pub fn set_tab(&mut self, td: TabData) {
        match self.tab_data.iter_mut().find(|t| t.name == td.name) {
            Some(slot) => *slot = td,
            None => self.tab_data.push(td),
        }
    }

    pub fn reporter(&self) -> Map<String, Value> {
        let rf = self.requested_for.as_deref().filter(|s| !s.is_empty());
        site::reporter(
            self.resource,
            &self.record,
            Some(&self.schema),
            Some(&self.tz),
            rf,
        )
    }

    /// The right-hand card above Properties.
    pub fn summary(&self) -> Vec<(String, String)> {
        site::summary(
            self.resource,
            &self.record,
            Some(&self.schema),
            Some(&self.tz),
            self.now.unwrap_or_else(Timestamp::now),
        )
    }

    pub fn description(&self) -> String {
        site::description_text(&self.record)
    }

    pub fn attachments(&self) -> Vec<Value> {
        site::attachments(&self.record)
    }

    pub fn notes(&self) -> Vec<String> {
        let mut out: Vec<String> = [
            &self.tabs_missing,
            &self.lifecycle_missing,
            &self.planning_missing,
            &self.requested_items_missing,
            &self.conversations_missing,
        ]
        .into_iter()
        .flatten()
        .map(Missing::note)
        .collect();
        if let Some(note) = self.properties_note.as_ref().filter(|s| !s.is_empty()) {
            out.push(note.clone());
        }
        out
    }

    pub fn to_json(&self) -> Value {
        let mut details = Map::new();
        details.insert("description".into(), json!(self.description()));
        details.insert("attachments".into(), json!(self.attachments()));
        if let Some(planning) = &self.planning {
            details.insert(
                "planning".into(),
                json!(
                    planning
                        .iter()
                        .map(PlanningEntry::to_json)
                        .collect::<Vec<_>>()
                ),
            );
        }
        if let Some(items) = &self.requested_items {
            details.insert(
                "requested_items".into(),
                json!(items.iter().map(requested_item_json).collect::<Vec<_>>()),
            );
        }
        if let Some(convs) = &self.conversations {
            let rows: Vec<Value> = convs
                .iter()
                .map(|cv| site::conversation_json(cv, Some(&self.tz)))
                .collect();
            details.insert("conversations".into(), json!(rows));
            details.insert(
                "conversations_total".into(),
                self.conversations_meta
                    .get("count")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id()));
        out.insert("subject".into(), json!(self.subject()));
        out.insert("status".into(), json!(self.badge()));
        out.insert(
            "due_by".into(),
            self.record.get("due_by").cloned().unwrap_or(Value::Null),
        );
        out.insert("reporter".into(), Value::Object(self.reporter()));
        out.insert(
            "tabs".into(),
            json!(self.tabs.iter().map(Tab::to_json).collect::<Vec<_>>()),
        );
        out.insert(
            "lifecycle".into(),
            self.lifecycle
                .as_ref()
                .map_or(Value::Null, Lifecycle::to_json),
        );
        out.insert("details".into(), Value::Object(details));
        out.insert(
            "summary".into(),
            json!(
                self.summary()
                    .iter()
                    .map(|(k, v)| json!({"label": k, "value": v}))
                    .collect::<Vec<_>>()
            ),
        );
        out.insert(
            "properties".into(),
            json!(
                self.properties
                    .iter()
                    .map(Prop::to_json)
                    .collect::<Vec<_>>()
            ),
        );
        out.insert("notes".into(), json!(self.notes()));
        if !self.tab_data.is_empty() {
            let tabs: Map<String, Value> = self
                .tab_data
                .iter()
                .map(|t| (t.name.clone(), t.to_json()))
                .collect();
            out.insert("tab_data".into(), Value::Object(tabs));
        }
        Value::Object(out)
    }

    /// `views.resolve_tab`.
    pub fn resolve_tab(&self, text: &str) -> Result<String, QueryError> {
        site::resolve_tab(&self.tabs, text, &self.id(), self.tabs_missing.is_some())
    }
}

/// What the caller already has: the record, its schema, and the signed-in agent.
pub struct DetailInput<'a> {
    pub record: Value,
    /// `None` is an empty form (`{"fields": []}`).
    pub schema: Option<Value>,
    /// `current_user`; `{}` when the role may not read it.
    pub user: Value,
    /// `user_tz(user)`.
    pub tz: TimeZone,
    pub item_id: Value,
    pub all_conversations: bool,
    pub tabs: &'a [String],
}

/// Form-evaluated properties (`load_properties`) or `None` to show fields as stored.
pub type Properties = Option<(Vec<Prop>, Option<String>)>;

/// `load_detail` after the record and schema are loaded.
///
/// `properties` comes from the form engine; without it the fields are shown
/// in schema order with the same note Python prints when rules cannot run.
pub fn load_detail(
    src: &mut dyn Source,
    res: &'static Resource,
    input: DetailInput<'_>,
    properties: Properties,
) -> Result<Detail, SessionError> {
    detail(src.api(), res, input, properties)
}

fn detail(
    src: &dyn Api,
    res: &'static Resource,
    input: DetailInput<'_>,
    properties: Properties,
) -> Result<Detail, SessionError> {
    let rec = input.record;
    let sch = input
        .schema
        .filter(fsv_py::truthy)
        .unwrap_or_else(|| json!({"fields": []}));
    let rid = ["display_id", "id"]
        .iter()
        .find_map(|k| rec.get(*k).filter(|v| fsv_py::truthy(v)))
        .cloned()
        .unwrap_or(input.item_id);
    let rid_s = fsv_py::py_str(&rid);

    let got = optional_get(src, &format!("{}/{rid_s}/tabs", res.api_path), None, "tabs")?;
    let mut raw_tab_names: Vec<String> = Vec::new();
    let (tab_list, tabs_miss) = match got {
        Ok(data) if data.is_array() || data.is_object() => {
            let rows = match &data {
                Value::Array(items) => items.clone(),
                other => other
                    .get("tabs")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            };
            raw_tab_names = rows
                .iter()
                .filter(|t| t.is_object())
                .map(|t| {
                    t.get("name")
                        .map(fsv_py::py_str)
                        .unwrap_or_else(|| "None".into())
                })
                .collect();
            (site::parse_tabs(&data), None)
        }
        other => {
            let mut tabs = vec![Tab {
                name: "details".into(),
                label: tab_label("details"),
                count: None,
            }];
            tabs.extend(site::fallback_tabs(res.name).iter().map(|n| Tab {
                name: (*n).to_string(),
                label: tab_label(n),
                count: None,
            }));
            (tabs, other.err())
        }
    };
    let mut d = Detail::new(res, rec, sch, input.user, input.tz, tab_list);
    d.tabs_missing = tabs_miss;

    if res.name == "changes" {
        (d.lifecycle, d.lifecycle_missing) = load_lifecycle(src, &d.record)?;
        (d.planning, d.planning_missing) = load_planning(src, &d.record, &d.schema, "changes")?;
    } else if res.name == "problems" {
        (d.planning, d.planning_missing) = load_planning(src, &d.record, &d.schema, "problems")?;
    }
    if res.name == "tickets" {
        d.requested_for = requested_for(src, &d.record)?;
        if raw_tab_names.iter().any(|n| n == "requested_items") || d.tabs_missing.is_some() {
            match get_requested_items(src, &rid) {
                Ok(items) => d.requested_items = Some(items),
                Err(FetchError::Session(msg)) => return Err(SessionError(msg)),
                Err(FetchError::Status(status)) => {
                    let miss = Missing::new("requested items", status);
                    if !(miss.status == Some(404) && d.tabs_missing.is_some()) {
                        d.requested_items_missing = Some(miss);
                    }
                }
            }
        }
        let convs = load_conversations(src, &rid, input.all_conversations, 3)?;
        d.conversations = if convs.missing.is_some() && convs.rows.is_empty() {
            None
        } else {
            Some(convs.rows)
        };
        d.conversations_meta = convs.meta;
        d.conversations_missing = convs.missing;
    }
    (d.properties, d.properties_note) = match properties {
        Some(props) => props,
        None if d.schema.get("fields").is_none_or(|f| !fsv_py::truthy(f)) => {
            (Vec::new(), Some("no form fields".into()))
        }
        None => (
            site::flat_properties(&d.record, Some(&d.schema)),
            Some("form rules could not be applied; fields shown as stored".into()),
        ),
    };
    for name in input.tabs {
        let cname = canonical_tab(name);
        if cname == "details" {
            continue;
        }
        let td = tab(src, res, &d.record, &cname)?;
        d.set_tab(td);
    }
    Ok(d)
}

/// `load_lifecycle`: `(None, None)` when the account has no flow.
pub fn load_lifecycle(
    src: &dyn Api,
    record: &Value,
) -> Result<(Option<Lifecycle>, Option<Missing>), SessionError> {
    let Some(sfid) = record.get("state_flow_id").filter(|v| fsv_py::truthy(v)) else {
        return Ok((None, None));
    };
    let params = obj(json!({"include": "state_info"}));
    let path = format!("change_management/{}", fsv_py::py_str(sfid));
    match workspace_get(
        src,
        &path,
        Some(&params),
        record.get("workspace_id"),
        "lifecycle",
    )? {
        Err(miss) => Ok((None, Some(miss))),
        Ok(data) => Ok((site::lifecycle_from(record, &data), None)),
    }
}

/// `load_planning`: planning fields (problems: the native analysis
/// sections) in form order; `(None, None)` when the account defines none.
pub fn load_planning(
    src: &dyn Api,
    record: &Value,
    schema: &Value,
    module: &str,
) -> Result<(Option<Vec<PlanningEntry>>, Option<Missing>), SessionError> {
    let defs = if module == "problems" {
        fsv_api::planning_text::PROBLEM_SECTIONS
            .iter()
            .map(|(_, key, title)| json!({"name": key, "label": title}))
            .collect()
    } else {
        site::change_planning_defs(Some(schema))
    };
    if defs.is_empty() {
        return Ok((None, None));
    }
    let path = format!("{module}/{}/planning-fields", rid_of(record));
    let data = match optional_get(src, &path, None, "planning")? {
        Err(miss) => return Ok((None, Some(miss))),
        Ok(data) => data,
    };
    let miss = |why: &str| Some(Missing::new(why, None));
    Ok(match site::planning_rows(&defs, &data, module) {
        PlanningOutcome::Entries(rows) => (Some(rows), None),
        PlanningOutcome::NoDefinitions => (None, None),
        PlanningOutcome::Unavailable => (None, miss("planning rows unavailable")),
        PlanningOutcome::Unidentified => (None, miss("unidentified planning rows")),
        PlanningOutcome::Ambiguous(_) => (None, miss("ambiguous planning rows")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::fake::Canned;
    use fsv_core::resource::{CHANGES, TICKETS};

    #[test]
    fn notes_tab_reads_one_page() {
        let mut src = Canned {
            int: vec![(
                "changes/7/notes".into(),
                Ok(json!({"notes": [{"id": 1, "body": "<p>hi</p>"}], "meta": {"has_more": false}})),
            )],
            ..Default::default()
        };
        let td = load_tab(&mut src, &CHANGES, &json!({"display_id": 7}), "notes").unwrap();
        assert_eq!(td.label, "Notes");
        assert!(td.missing.is_none());
        assert_eq!(td.data.unwrap().as_array().unwrap().len(), 1);
    }

    #[test]
    fn unavailable_tab_is_missing() {
        let mut src = Canned::default();
        let td = load_tab(&mut src, &CHANGES, &json!({"display_id": 7}), "tasks").unwrap();
        assert_eq!(
            td.missing.unwrap().note(),
            "Tasks: not available on this account (404)"
        );
        assert!(td.data.is_none());
        let td = load_tab(&mut src, &CHANGES, &json!({"display_id": 7}), "bogus").unwrap();
        assert_eq!(td.missing.unwrap().note(), "Bogus: unavailable");
    }

    #[test]
    fn resolution_from_v2_and_404() {
        let mut src = Canned {
            v2: vec![(
                "tickets/3/resolution_note".into(),
                Ok(json!({"resolution_note": {"body": "<b>Fixed</b>"}})),
            )],
            ..Default::default()
        };
        let rec = json!({"id": 3, "stats": {"resolved_at": "2026-01-01T00:00:00Z"}});
        let td = load_tab(&mut src, &TICKETS, &rec, "resolution").unwrap();
        assert_eq!(
            td.data.unwrap(),
            json!({"resolved_at": "2026-01-01T00:00:00Z", "note": "Fixed"})
        );
        let td = load_tab(&mut src, &TICKETS, &json!({"id": 4}), "resolution").unwrap();
        assert_eq!(td.meta.get("absent"), Some(&json!(true)));
        assert!(td.missing.is_none());
    }

    #[test]
    fn detail_falls_back_to_known_tabs() {
        let mut src = Canned::default();
        let input = DetailInput {
            record: json!({"display_id": 5, "subject": "Disk"}),
            schema: None,
            user: json!({}),
            tz: TimeZone::UTC,
            item_id: json!(5),
            all_conversations: false,
            tabs: &[],
        };
        let d = load_detail(&mut src, &CHANGES, input, None).unwrap();
        assert_eq!(
            d.tab_names(),
            ["details", "notes", "tasks", "approvals", "activity"]
        );
        assert_eq!(
            d.notes(),
            [
                "tabs: not available on this account (404)",
                "no form fields"
            ]
        );
        assert_eq!(d.resolve_tab("risk").unwrap(), "risk");
    }
}
