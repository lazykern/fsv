//! [`fsv_tui::Backend`] and [`fsv_tui::Pages`] over the live client,
//! fsv-api and fsv-views, and the `fsv tui` entry point.
//!
//! A loaded [`Detail`] stays in a [`DetailCache`] shared by the backend and
//! the renderer; [`DetailPage::extra`] carries its key. Write plans travel
//! through the TUI as JSON ([`ReviewPlan::raw`]) and are rebuilt for the
//! send, which re-checks the reviewed hash.

use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

use fsv_api::Directory;
use fsv_api::approvals::{self, ActionPlan, ApprovalPlan};
use fsv_api::associations::{self, AssociationPlan, SendOpts};
use fsv_api::formmeta::{DiskStore, MetaStore};
use fsv_api::notes::{self, BodyFormat, NotePlan};
use fsv_api::service::{self as svc, Api, Journal, ListArgs};
use fsv_api::tasks::{self, TaskArgs, TaskPlan};
use fsv_api::{QueryError, site};
use fsv_core::{Client, Error, Result};
use fsv_tui::backend::{
    BResult, DetailPage, Failure, Item, ListRequest, Missing, Res, ReviewPlan, SearchResult,
    TabData, TabInfo, View,
};
use fsv_tui::{Backend, BasicPages, Pages};
use fsv_views::{Detail, DetailInput, SessionError};

use crate::source::ClientSource;
use crate::writer::change_id;

pub mod desktop;
pub mod files;

pub use desktop::{Browser, Clipboard};

/// `schema.load(res, client)`: the list/detail schema of one resource.
pub type SchemaLoader = Arc<dyn Fn(Res) -> Result<Value> + Send + Sync>;

/// `schema.load(res, client)` ([`fsv_api::schema::load_in`]) cached under
/// the client's profile paths for its tenant.
pub fn schema_loader(client: Arc<Client>) -> SchemaLoader {
    Arc::new(move |res: Res| {
        let key = fsv_core::config::cache_domain_key(Some(&client.domain()));
        let paths = client.ctx().paths.clone();
        fsv_api::schema::load_in(res, &client, false, &paths, &key)
    })
}

/// Includes `views.load_detail` asks for on a ticket.
pub const TICKET_DETAIL_INCLUDES: &str = "requester,stats,phone,feedback,ticket_status,responder";

const DETAIL_CACHE_SIZE: usize = 64;

type Details = (u64, IndexMap<String, Arc<Detail>>);

/// Loaded detail pages by key (the most recent [`DETAIL_CACHE_SIZE`]).
#[derive(Clone, Default)]
pub struct DetailCache {
    inner: Arc<Mutex<Details>>,
}

impl DetailCache {
    /// Keep `d`; its key for [`DetailPage::extra`].
    pub fn put(&self, d: Detail) -> String {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.0 += 1;
        let key = format!("{}:{}", d.resource.name, g.0);
        g.1.insert(key.clone(), Arc::new(d));
        while g.1.len() > DETAIL_CACHE_SIZE {
            g.1.shift_remove_index(0);
        }
        key
    }

    /// The detail behind a page this backend loaded.
    pub fn get(&self, page: &DetailPage) -> Option<Arc<Detail>> {
        let key = page.extra.get("detail_key")?.as_str()?;
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .1
            .get(key)
            .cloned()
    }
}

/// The filter builder's people and group lookups over the client.
pub struct ClientDirectory(pub Arc<dyn Api + Send + Sync>);

impl Directory for ClientDirectory {
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        extra: &[(&str, &str)],
    ) -> std::result::Result<Vec<Value>, QueryError> {
        let params = (!extra.is_empty()).then(|| {
            extra
                .iter()
                .map(|(k, v)| (k.to_string(), json!(v)))
                .collect::<Map<String, Value>>()
        });
        self.0
            .autocomplete(kind, query, params)
            .map_err(|e| QueryError(e.to_string()))
    }

    fn agent_groups(&self) -> std::result::Result<Value, QueryError> {
        self.0
            .int_get("bootstrap/agents_groups", None)
            .map_err(|e| QueryError(e.to_string()))
    }
}

/// [`Backend`] over the live client.
pub struct LiveBackend {
    pub client: Arc<Client>,
    pub schema: SchemaLoader,
    pub journal: Arc<dyn Journal + Send + Sync>,
    /// Edit-form business rules (`views.load_edit_rules`' cache).
    pub store: Arc<dyn MetaStore + Send + Sync>,
    /// `config.load_config()`.
    pub config: Map<String, Value>,
    pub details: DetailCache,
    /// Yank (`pbcopy`).
    pub clipboard: Clipboard,
    /// Open in the browser (`webbrowser.open`).
    pub browser: Browser,
    me: Mutex<Option<Value>>,
}

fn session(e: SessionError) -> Failure {
    Failure::from(Error::Session(e.0))
}

fn id_of(v: &Value) -> BResult<i64> {
    change_id(v).map_err(Failure::from)
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn s(v: &Value) -> String {
    fsv_py::py_str(v)
}

fn str_at(raw: &Value, key: &str) -> String {
    raw.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn i64_at(raw: &Value, key: &str) -> i64 {
    raw.get(key).and_then(Value::as_i64).unwrap_or_default()
}

fn note_plan(raw: &Value) -> NotePlan {
    NotePlan {
        ticket_id: i64_at(raw, "ticket_id"),
        body_html: str_at(raw, "body_html"),
        public: raw.get("public").is_some_and(fsv_py::truthy),
        notify_emails: raw
            .get("notify_emails")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(s).collect())
            .unwrap_or_default(),
        tenant: str_at(raw, "tenant"),
    }
}

fn approval_plan(raw: &Value) -> ApprovalPlan {
    ApprovalPlan {
        change_id: i64_at(raw, "change_id"),
        name: str_at(raw, "name"),
        message: str_at(raw, "message"),
        approver_ids: raw
            .get("approver_ids")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default(),
        tenant: str_at(raw, "tenant"),
        rule: str_at(raw, "rule"),
    }
}

fn action_plan(raw: &Value) -> ActionPlan {
    ActionPlan {
        change_id: i64_at(raw, "change_id"),
        target_id: i64_at(raw, "target_id"),
        operation: str_at(raw, "operation"),
        remark: raw
            .get("remark")
            .and_then(Value::as_str)
            .map(str::to_string),
        tenant: str_at(raw, "tenant"),
    }
}

/// A views tab as the TUI holds it.
pub fn tui_tab(td: fsv_views::TabData) -> TabData {
    TabData {
        name: td.name,
        label: td.label,
        data: td.data.unwrap_or(Value::Null),
        missing: td.missing.map(|m| Missing {
            what: m.what,
            status: m.status,
        }),
        parts: td.parts,
        notes: td.notes,
        meta: td.meta,
        parts_meta: td.parts_meta,
    }
}

/// The TUI's tab back as a views tab (for `pages.tab_lines`).
pub fn views_tab(td: &TabData) -> fsv_views::TabData {
    fsv_views::TabData {
        name: td.name.clone(),
        label: td.label.clone(),
        data: (!(td.data.is_null() && td.missing.is_some())).then(|| td.data.clone()),
        missing: td
            .missing
            .as_ref()
            .map(|m| site::Missing::new(m.what.clone(), m.status)),
        parts: td.parts.clone(),
        notes: td.notes.clone(),
        meta: td.meta.clone(),
        parts_meta: td.parts_meta.clone(),
    }
}

impl LiveBackend {
    /// The current profile's schema cache, journal, rules cache and config.
    pub fn live(client: Arc<Client>) -> LiveBackend {
        LiveBackend::new(client.clone(), schema_loader(client))
    }

    /// [`LiveBackend::live`] with `schema` loading the schemas.
    pub fn new(client: Arc<Client>, schema: SchemaLoader) -> LiveBackend {
        LiveBackend {
            client,
            schema,
            journal: Arc::new(svc::DiskJournal),
            store: Arc::new(DiskStore::current()),
            config: fsv_core::config::load_config(),
            details: DetailCache::default(),
            clipboard: Arc::new(desktop::pbcopy),
            browser: Arc::new(desktop::open_in_browser),
            me: Mutex::new(None),
        }
    }

    pub fn with_clipboard(mut self, clipboard: Clipboard) -> LiveBackend {
        self.clipboard = clipboard;
        self
    }

    pub fn with_browser(mut self, browser: Browser) -> LiveBackend {
        self.browser = browser;
        self
    }

    pub fn with_journal(mut self, journal: Arc<dyn Journal + Send + Sync>) -> LiveBackend {
        self.journal = journal;
        self
    }

    pub fn with_store(mut self, store: Arc<dyn MetaStore + Send + Sync>) -> LiveBackend {
        self.store = store;
        self
    }

    pub fn with_config(mut self, config: Map<String, Value>) -> LiveBackend {
        self.config = config;
        self
    }

    /// The renderer sharing this backend's loaded pages.
    pub fn pages(&self) -> LivePages {
        LivePages {
            details: self.details.clone(),
        }
    }

    fn api(&self) -> &dyn Api {
        self.client.as_ref()
    }

    fn src(&self) -> ClientSource {
        ClientSource::new(self.client.clone())
    }

    fn domain(&self) -> String {
        self.client.domain()
    }

    fn sends<'a>(hash: &'a str, read_only: bool) -> SendOpts<'a> {
        SendOpts {
            reviewed_hash: Some(hash),
            read_only,
            dry_run: false,
        }
    }

    fn paths(&self) -> &fsv_core::config::Paths {
        &self.client.ctx().paths
    }

    /// The filter history file (`filters_cache_path("tui_history")`).
    pub fn history_path(&self) -> std::path::PathBuf {
        files::history_path(self.paths(), &self.domain())
    }

    /// `views.load_detail(c, res, id, schema=...)`; the properties load last,
    /// as in Python (no extra tabs are asked for).
    fn detail(&self, res: Res, id: &Value, schema: Option<&Value>) -> BResult<Detail> {
        let includes = (res.name == "tickets").then_some(TICKET_DETAIL_INCLUDES);
        let record = svc::get_item(self.api(), res, id_of(id)?, includes)?;
        let schema = match schema {
            Some(s) => s.clone(),
            None => (self.schema)(res)?,
        };
        let user = self.current_user()?;
        let tz = self.user_tz(&user).unwrap_or_else(TimeZone::system);
        let input = DetailInput {
            record,
            schema: Some(schema),
            user,
            tz,
            item_id: id.clone(),
            all_conversations: false,
            tabs: &[],
        };
        let mut d = fsv_views::load_detail(&mut self.src(), res, input, None).map_err(session)?;
        let api: Arc<dyn Api + Send + Sync> = self.client.clone();
        (d.properties, d.properties_note) = fsv_api::views::load_properties(
            &api,
            self.store.as_ref(),
            res,
            &d.record,
            Some(&d.schema),
            Some(&d.tz),
            Some(&d.user),
        )?;
        Ok(d)
    }

    /// `completion._completion_network()`.
    fn network_on(&self) -> bool {
        if std::env::var("FSV_COMPLETION_NETWORK").as_deref() == Ok("1") {
            return true;
        }
        match self.config.get("completion").and_then(|c| c.get("network")) {
            Some(Value::String(v)) => matches!(v.as_str(), "on" | "true" | "1"),
            Some(Value::Bool(b)) => *b,
            Some(Value::Number(n)) => n.as_f64() == Some(1.0),
            _ => false,
        }
    }
}

/// `TaskPlan` fields as [`ReviewPlan::raw`] carries them.
fn task_raw(p: &TaskPlan) -> Value {
    json!({"kind": "change_task", "change_id": p.change_id, "title": p.title, "owner_id": p.owner_id,
           "tenant": p.tenant, "group_id": p.group_id, "status_label": p.status_label, "status_id": p.status_id,
           "note": p.note, "notify_before": p.notify_before, "planned_start": p.planned_start,
           "planned_end": p.planned_end, "planned_effort": p.planned_effort})
}

fn opt_str(raw: &Value, key: &str) -> Option<String> {
    raw.get(key).and_then(Value::as_str).map(str::to_string)
}

fn task_plan(raw: &Value) -> TaskPlan {
    TaskPlan {
        change_id: i64_at(raw, "change_id"),
        title: str_at(raw, "title"),
        owner_id: raw.get("owner_id").and_then(Value::as_i64),
        tenant: str_at(raw, "tenant"),
        group_id: raw.get("group_id").and_then(Value::as_i64),
        status_label: str_at(raw, "status_label"),
        status_id: i64_at(raw, "status_id"),
        note: opt_str(raw, "note"),
        notify_before: str_at(raw, "notify_before"),
        planned_start: opt_str(raw, "planned_start"),
        planned_end: opt_str(raw, "planned_end"),
        planned_effort: opt_str(raw, "planned_effort"),
    }
}

fn association_raw(p: &AssociationPlan) -> Value {
    json!({"kind": "association", "change_id": p.change_id, "ticket_ids": p.ticket_ids,
           "unlink": p.unlink, "tenant": p.tenant})
}

fn association_plan(raw: &Value) -> AssociationPlan {
    AssociationPlan {
        change_id: i64_at(raw, "change_id"),
        ticket_ids: raw
            .get("ticket_ids")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default(),
        unlink: raw.get("unlink").is_some_and(fsv_py::truthy),
        tenant: str_at(raw, "tenant"),
    }
}

/// `_completion_case`: the candidate in the typed prefix's case.
fn completion_case(candidate: &str, query: &str) -> String {
    if candidate.starts_with(query) {
        return candidate.to_string();
    }
    let folded = fsv_core::pystr::casefold(candidate);
    if folded.starts_with(&fsv_core::pystr::casefold(query)) {
        let rest: String = candidate.chars().skip(query.chars().count()).collect();
        return format!("{query}{rest}");
    }
    candidate.to_string()
}

/// `completion._remote_user_values` rows → `(value, detail)`.
pub fn user_values(rows: &[Value], query: &str) -> Vec<(String, String)> {
    let text = |row: &Value, k: &str| {
        row.get(k)
            .filter(|v| fsv_py::truthy(v))
            .map(s)
            .unwrap_or_default()
    };
    let first = |row: &Value, keys: &[&str]| {
        keys.iter()
            .map(|k| text(row, k))
            .find(|v| !v.is_empty())
            .unwrap_or_default()
    };
    let q = fsv_core::pystr::casefold(query);
    let mut out = Vec::new();
    for row in rows.iter().take(10) {
        let candidates = ["email", "details", "value", "name"].map(|k| text(row, k));
        let mut value = candidates
            .iter()
            .find(|x| !x.is_empty() && fsv_core::pystr::casefold(x).starts_with(&q))
            .cloned()
            .unwrap_or_default();
        if value.is_empty() {
            value = first(row, &["value", "name", "email"]);
        }
        let detail = first(row, &["value", "name", "email", "details", "id", "user_id"]);
        if !value.is_empty() {
            out.push((completion_case(&value, query), detail));
        }
    }
    out
}

impl Backend for LiveBackend {
    fn read_only_setting(&self) -> Value {
        match self.config.get("tui") {
            Some(Value::Object(t)) => t.get("read_only").cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    fn default_where(&self, entity: &str) -> Vec<String> {
        fsv_core::config::get_default_where(entity)
    }

    fn item_url(&self, res: Res, id: &Value) -> String {
        format!("https://{}/a/{}/{}", self.domain(), res.api_path, s(id))
    }

    fn load_schema(&self, res: Res) -> BResult<Value> {
        Ok((self.schema)(res)?)
    }

    /// `views.current_user`: once per backend; `{}` when the role may not
    /// read it (a session error still fails).
    fn current_user(&self) -> BResult<Value> {
        let mut g = self.me.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(me) = g.as_ref() {
            return Ok(me.clone());
        }
        let mut me = match self.client.me() {
            Ok(Value::Object(m)) => m,
            Ok(_) => Map::new(),
            Err(e @ Error::Session(_)) => return Err(e.into()),
            Err(_) => Map::new(),
        };
        if !me.is_empty() && !me.get("name").is_some_and(fsv_py::truthy) {
            let name = ["first_name", "last_name"]
                .iter()
                .map(|k| {
                    me.get(*k)
                        .filter(|v| fsv_py::truthy(v))
                        .map(s)
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ");
            me.insert("name".into(), json!(name.trim()));
        }
        let me = Value::Object(me);
        *g = Some(me.clone());
        Ok(me)
    }

    /// `views.user_tz`: `FSV_TZ`, else the profile zone, else the system.
    fn user_tz(&self, me: &Value) -> Option<TimeZone> {
        let user = me
            .as_object()
            .filter(|m| m.get("time_zone").is_some_and(fsv_py::truthy));
        Some(
            fsv_forms::tz::resolve_timezone(None, user)
                .map(|(z, _)| z.tz)
                .unwrap_or_else(|_| TimeZone::system()),
        )
    }

    /// `views.load_views`; a hidden list is no views.
    fn load_views(&self, res: Res) -> BResult<Vec<View>> {
        let (views, _missing) = fsv_api::views::load_views(self.api(), res)?;
        Ok(views
            .into_iter()
            .map(|v| View {
                id: v.id,
                name: v.name,
                system: v.system,
                marked_default: v.marked_default,
                raw: v.raw,
            })
            .collect())
    }

    fn ticket_headers(&self) -> BResult<Vec<Value>> {
        let got =
            fsv_api::views::optional_get(self.api(), "tickets/headers", None, "list headers")?;
        Ok(match got {
            Ok(Value::Object(m)) => m
                .get("headers")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter(|h| h.is_object()).cloned().collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        })
    }

    /// `views.view_settings`.
    fn view_settings(&self, res: Res, view: &View) -> BResult<Value> {
        let saved = site::SavedView {
            id: view.id.clone(),
            name: view.name.clone(),
            system: view.system,
            marked_default: view.marked_default,
            raw: view.raw.clone(),
        };
        Ok(fsv_api::views::view_settings(self.api(), res, &saved)?)
    }

    fn build_query_hash(&self, res: Res, schema: &Value, filters: &[String]) -> BResult<String> {
        let dir = ClientDirectory(self.client.clone());
        fsv_api::build_query_hash_from_schema(Some(&dir), res.name, schema, filters)
            .map(Option::unwrap_or_default)
            .map_err(|e| Failure::value(e.0))
    }

    fn list_page(&self, req: &ListRequest) -> BResult<(Vec<Value>, Value)> {
        let args = ListArgs {
            page: req.page,
            per_page: req.per_page,
            filter_name: req.filter_name.as_deref(),
            order_by: Some(&req.order_by),
            order_type: Some(&req.order_type),
            query_hash: req.query_hash.as_deref().filter(|q| !q.is_empty()),
            or_grouping: false,
        };
        Ok(svc::list_page(self.api(), req.res, &args)?)
    }

    fn search_items(
        &self,
        term: &str,
        entity: Option<&str>,
        sort: &str,
        page: i64,
    ) -> BResult<SearchResult> {
        let (rows, totals) = svc::search_items(
            self.api(),
            term,
            entity,
            Some(sort).filter(|s| !s.is_empty()),
            page,
        )?;
        let items = rows
            .into_iter()
            .filter_map(|r| {
                let res = r
                    .get("_resource")
                    .and_then(Value::as_str)
                    .and_then(fsv_core::resource::lookup)?;
                Some(Item::new(res, r))
            })
            .collect();
        Ok((
            items,
            totals
                .into_iter()
                .map(|(k, v)| (k, v.as_i64().unwrap_or_default()))
                .collect(),
        ))
    }

    /// `views.load_detail(c, res, id, schema=...)`.
    fn load_detail(&self, res: Res, id: &Value, schema: Option<&Value>) -> BResult<DetailPage> {
        let d = self.detail(res, id, schema)?;
        let page = DetailPage {
            res,
            record: d.record.clone(),
            tabs: d
                .tabs
                .iter()
                .map(|t| TabInfo {
                    name: t.name.clone(),
                    label: t.label.clone(),
                    count: t.count,
                })
                .collect(),
            badge: d.badge(),
            due: d.due_text(),
            tz: Some(d.tz.clone()),
            schema: Some(d.schema.clone()),
            extra: json!({"notes": d.notes()}),
        };
        let key = self.details.put(d);
        Ok(DetailPage {
            extra: json!({"detail_key": key, "notes": page.extra["notes"]}),
            ..page
        })
    }

    fn load_tab(&self, res: Res, record: &Value, tab: &str) -> BResult<TabData> {
        Ok(tui_tab(
            fsv_views::load_tab(&mut self.src(), res, record, tab).map_err(session)?,
        ))
    }

    fn update_item(&self, res: Res, id: &Value, body: &Map<String, Value>) -> BResult<Value> {
        Ok(svc::update_item(
            self.api(),
            res,
            id_of(id)?,
            &Value::Object(body.clone()),
        )?)
    }

    fn add_note(&self, res: Res, id: &Value, body: &str, public: bool) -> BResult<Value> {
        Ok(svc::add_note(self.api(), res, id_of(id)?, body, public)?)
    }

    fn add_reply(&self, res: Res, id: &Value, body: &str) -> BResult<Value> {
        Ok(svc::add_reply(self.api(), res, id_of(id)?, body)?)
    }

    /// `ticket_notes.build_plan(id, body, public=..., notify_emails=[], body_format="plain")`.
    fn ticket_note_plan(&self, ticket_id: &Value, body: &str, public: bool) -> BResult<ReviewPlan> {
        let p = notes::build_plan(
            &s(ticket_id),
            body,
            public,
            &[],
            BodyFormat::Plain,
            &self.domain(),
        )?;
        Ok(ReviewPlan {
            review: Value::Object(p.review()),
            hash: p.hash(),
            public: p.public,
            target_id: json!(p.ticket_id),
            raw: json!({"kind": "ticket_note", "ticket_id": p.ticket_id, "body_html": p.body_html, "public": p.public,
                        "notify_emails": p.notify_emails, "tenant": p.tenant}),
        })
    }

    fn send_ticket_note(
        &self,
        plan: &ReviewPlan,
        public_confirmed: bool,
        read_only: bool,
    ) -> BResult<Value> {
        let p = note_plan(&plan.raw);
        Ok(notes::send(
            &p,
            self.api(),
            self.journal.as_ref(),
            Self::sends(&plan.hash, read_only),
            public_confirmed,
        )?)
    }

    /// `app._prepare_change_task`: validate locally first, then resolve the
    /// owner, group and status and plan again with the real ids.
    fn change_task_plan(
        &self,
        change_id: &Value,
        title: &str,
        owner: Option<&str>,
        options: &[(String, String)],
    ) -> BResult<ReviewPlan> {
        let mut values: IndexMap<&str, &str> = IndexMap::new();
        for (k, v) in options {
            values.insert(k.as_str(), v.as_str());
        }
        let group = values.shift_remove("group");
        let status = values.shift_remove("status").unwrap_or("Open");
        let mut args = TaskArgs::default();
        for (k, v) in &values {
            match *k {
                "note" => args.note = Some(v),
                "notify_before" => args.notify_before = v,
                "planned_start" => args.planned_start = Some(v),
                "planned_end" => args.planned_end = Some(v),
                "planned_effort" => args.planned_effort = Some(v),
                other => {
                    return Err(Failure::value(format!(
                        "build_plan() got an unexpected keyword argument '{other}'"
                    )));
                }
            }
        }
        let cid = s(change_id);
        let tenant = self.domain();
        tasks::status_label(status)?;
        tasks::build_plan(
            &cid,
            title,
            &TaskArgs {
                status_id: Some(1),
                ..args.clone()
            },
            &tenant,
        )?;
        let owner_id = tasks::resolve_owner(self.api(), owner)?;
        let group_id = tasks::resolve_group(self.api(), group)?;
        let status_id = tasks::resolve_status(self.api(), status)?;
        let p = tasks::build_plan(
            &cid,
            title,
            &TaskArgs {
                owner_id,
                group_id,
                status,
                status_id: Some(status_id),
                ..args
            },
            &tenant,
        )?;
        Ok(ReviewPlan {
            review: Value::Object(p.review()),
            hash: p.hash(),
            public: false,
            target_id: json!(p.change_id),
            raw: task_raw(&p),
        })
    }

    /// `change_tasks.send(plan, reviewed_hash=plan.hash, read_only=...)`.
    fn send_change_task(&self, plan: &ReviewPlan, read_only: bool) -> BResult<Value> {
        if plan.raw.get("kind").and_then(Value::as_str) != Some("change_task") {
            return Err(Failure::value("not a change task plan"));
        }
        Ok(tasks::send(
            &task_plan(&plan.raw),
            self.api(),
            self.journal.as_ref(),
            Self::sends(&plan.hash, read_only),
        )?)
    }

    /// `create.search_change_tickets(query)`.
    fn search_change_tickets(&self, query: &str) -> BResult<Vec<Value>> {
        Ok(fsv_api::create::search_change_tickets(self.api(), query)?)
    }

    /// `actions.download_choices`: a fresh detail's attachments, requested
    /// item files and cloud files, plus planning attachments (changes) or
    /// every conversation's attachments (tickets).
    fn download_choices(&self, item: &Item) -> BResult<(Vec<Value>, Vec<String>)> {
        let res = item.res;
        let rid = item.ref_id();
        let d = self.detail(res, &rid, None)?;
        let mut warnings = d.notes();
        let items = d.requested_items.clone().unwrap_or_default();
        let mut rows = files::attachment_rows(&d.record, &items);
        rows.extend(files::sourced(d.record.get("cloud_files"), "cloud file"));
        for row in &mut rows {
            if let Value::Object(o) = row {
                o.entry("source").or_insert_with(|| json!("request"));
            }
        }
        if res.name == "changes" {
            match self
                .api()
                .int_get(&format!("changes/{}/planning-fields", s(&rid)), None)
            {
                Ok(data) => {
                    let entries = data
                        .get("change_planning_fields")
                        .cloned()
                        .unwrap_or(json!([]));
                    for entry in entries.as_array().into_iter().flatten() {
                        let label = ["label", "name"]
                            .iter()
                            .find_map(|k| entry.get(*k).filter(|v| fsv_py::truthy(v)))
                            .map(s)
                            .unwrap_or_else(|| "None".into());
                        rows.extend(files::sourced(
                            entry.get("attachments"),
                            &format!("planning: {label}"),
                        ));
                    }
                }
                Err(e) => warnings.push(format!("Planning attachment inventory unavailable: {e}")),
            }
        }
        if res.name == "tickets" {
            let conv = fsv_views::source::load_conversations(&mut self.src(), &rid, true, 3)
                .map_err(session)?;
            if let Some(m) = &conv.missing {
                warnings.push(m.note());
            }
            if conv.meta.get("complete") == Some(&Value::Bool(false))
                || conv.meta.get("has_more").is_some_and(fsv_py::truthy)
            {
                warnings.push("Conversation attachment inventory is partial".into());
            }
            for cv in &conv.rows {
                let source = format!(
                    "conversation {}",
                    cv.get("id").map(s).unwrap_or_else(|| "None".into())
                );
                rows.extend(files::sourced(cv.get("attachments"), &source));
            }
        }
        Ok((rows, warnings))
    }

    /// `actions.download_selected`: `cli_sr._download(..., force=False)`.
    fn download(&self, attachment: &Value, destination: &str) -> BResult<Value> {
        Ok(files::download(self.api(), attachment, destination)?)
    }

    /// `create.get_task_for_edit` plus `task_status.fields_from`.
    fn task_for_edit(&self, change_id: &Value, task_id: &Value) -> BResult<(Value, Vec<Value>)> {
        let current =
            fsv_api::create::get_task_for_edit(self.api(), id_of(change_id)?, id_of(task_id)?)?;
        let data = self.api().int_get(
            "change_task_fields",
            Some(obj(json!({"include_deleted_choices": "true"}))),
        )?;
        let fields = fsv_api::fields_from_payload(&data)
            .map_err(|_| Failure::value("unreadable runtime task-status metadata"))?;
        Ok((Value::Object(current), fields))
    }

    /// `change_associations.build_plan(change_id, ids, unlink=...)`.
    fn association_plan(
        &self,
        change_id: &Value,
        ticket_ids: &[Value],
        unlink: bool,
    ) -> BResult<ReviewPlan> {
        let p = associations::build_plan(&s(change_id), ticket_ids, unlink, &self.domain())?;
        Ok(ReviewPlan {
            review: Value::Object(p.review()),
            hash: p.hash(),
            public: false,
            target_id: json!(p.change_id),
            raw: association_raw(&p),
        })
    }

    /// `change_associations.send(plan, client, reviewed_hash=plan.hash)`.
    fn send_association(&self, plan: &ReviewPlan) -> BResult<Value> {
        if plan.raw.get("kind").and_then(Value::as_str) != Some("association") {
            return Err(Failure::value("not an association plan"));
        }
        Ok(associations::send(
            &association_plan(&plan.raw),
            self.api(),
            self.journal.as_ref(),
            Self::sends(&plan.hash, false),
        )?)
    }

    /// `create.update_task(change_id, task_id, body)`.
    fn update_task(
        &self,
        change_id: &Value,
        task_id: &Value,
        body: &Map<String, Value>,
    ) -> BResult<()> {
        fsv_api::create::update_task(self.api(), id_of(change_id)?, id_of(task_id)?, body)?;
        Ok(())
    }

    /// `create.delete_task(change_id, task_id)`.
    fn delete_task(&self, change_id: &Value, task_id: &Value) -> BResult<()> {
        fsv_api::create::delete_task(self.api(), id_of(change_id)?, id_of(task_id)?)?;
        Ok(())
    }

    /// `catalog.list_items(client)` then `catalog.list_categories(client)`.
    fn catalog(&self) -> BResult<(Vec<Value>, Vec<Value>)> {
        let items = fsv_api::catalog::list_items(self.api(), false, fsv_api::catalog::MAX_PAGES)?;
        let categories = fsv_api::catalog::list_categories(self.api())?;
        Ok((items, categories))
    }

    /// `catalog.resolve_category`; its "no/several categories" error is no
    /// categories (the TUI only passes ids from its own picker).
    fn resolve_category(&self, categories: &[Value], text: &str) -> Vec<Value> {
        fsv_api::catalog::resolve_category(categories, text).unwrap_or_default()
    }

    /// `catalog.filter_items(items, category_ids=ids)` (ids compared as text).
    fn filter_catalog_items(&self, items: &[Value], category_ids: Option<&[Value]>) -> Vec<Value> {
        fsv_api::catalog::filter_items(items, category_ids, None, &[])
    }

    // ── filter completion ───────────────────────────────────
    fn completion_network(&self) -> bool {
        self.network_on()
    }

    fn cached_group_names(&self) -> Vec<String> {
        files::cached_group_names(&files::groups_path(self.paths(), &self.domain()))
    }

    /// `completion._remote_user_values(kind, prefix)` values; any failure is
    /// no suggestions.
    fn remote_user_values(&self, kind: &str, prefix: &str) -> BResult<Vec<String>> {
        let query = fsv_core::pystr::strip(prefix);
        if !self.network_on() || query.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let params = (kind == "requesters").then(|| obj(json!({"all_users": "true"})));
        let Ok(rows) = self.api().autocomplete(kind, query, params) else {
            return Ok(Vec::new());
        };
        Ok(user_values(&rows, query)
            .into_iter()
            .map(|(v, _)| v)
            .collect())
    }

    /// `client.lookup_choices(link, prefix)`.
    fn lookup_choices(&self, link: &str, prefix: &str) -> BResult<Vec<Value>> {
        Ok(self.api().lookup_choices(link, prefix)?)
    }

    // ── settings ────────────────────────────────────────────
    fn load_filter_history(&self) -> Vec<String> {
        files::load_history(&self.history_path())
    }

    fn save_filter_history(&self, history: &[String]) {
        files::save_history(self.paths(), &self.history_path(), history);
    }

    fn copy_to_clipboard(&self, text: &str) -> std::result::Result<(), String> {
        (self.clipboard)(text)
    }

    fn open_url(&self, url: &str) -> bool {
        (self.browser)(url)
    }

    /// `change_approvals.resolve_approvers` + `build_plan`.
    fn approval_request_plan(
        &self,
        change_id: &Value,
        name: &str,
        message: &str,
        recipients: &[String],
        rule: &str,
    ) -> BResult<ReviewPlan> {
        let ids = approvals::resolve_approvers(self.api(), recipients)?;
        let p = approvals::build_plan(&s(change_id), name, message, &ids, rule, &self.domain())?;
        Ok(ReviewPlan {
            review: Value::Object(p.review()),
            hash: p.hash(),
            public: false,
            target_id: json!(p.change_id),
            raw: json!({"kind": "approval_request", "change_id": p.change_id, "name": p.name, "message": p.message,
                        "approver_ids": p.approver_ids, "tenant": p.tenant, "rule": p.rule}),
        })
    }

    fn approval_targets(&self, kind: &str, change_id: &Value) -> BResult<Vec<Value>> {
        if kind == "approval-respond" {
            return Ok(approvals::inventory(self.api(), id_of(change_id)?)?);
        }
        Ok(approvals::get_change_approval_groups(
            self.api(),
            change_id,
        )?)
    }

    fn approval_action_plan(
        &self,
        change_id: &Value,
        target_id: &Value,
        operation: &str,
        remark: Option<&str>,
    ) -> BResult<ReviewPlan> {
        let p = approvals::build_action_plan(
            &s(change_id),
            target_id,
            operation,
            remark,
            &self.domain(),
        )?;
        Ok(ReviewPlan {
            review: Value::Object(p.review()),
            hash: p.hash(),
            public: false,
            target_id: json!(p.target_id),
            raw: json!({"kind": "approval_action", "change_id": p.change_id, "target_id": p.target_id, "operation": p.operation,
                        "remark": p.remark, "tenant": p.tenant}),
        })
    }

    fn send_approval(&self, plan: &ReviewPlan, read_only: bool) -> BResult<Value> {
        let opts = Self::sends(&plan.hash, read_only);
        Ok(match plan.raw.get("kind").and_then(Value::as_str) {
            Some("approval_request") => approvals::send(
                &approval_plan(&plan.raw),
                self.api(),
                self.journal.as_ref(),
                opts,
            )?,
            Some("approval_action") => approvals::send_action(
                &action_plan(&plan.raw),
                self.api(),
                self.journal.as_ref(),
                opts,
            )?,
            _ => return Err(Failure::value("not an approval plan")),
        })
    }
}

/// `pages.py` over the details [`LiveBackend`] loaded; a page it did not
/// load renders with [`BasicPages`].
pub struct LivePages {
    pub details: DetailCache,
}

impl Pages for LivePages {
    fn lifecycle_markup(&self, page: &DetailPage, context: usize) -> String {
        match self.details.get(page) {
            Some(d) => fsv_views::pages::lifecycle_markup(d.lifecycle.as_ref(), Some(context)),
            None => BasicPages.lifecycle_markup(page, context),
        }
    }

    fn reporter_markup(&self, page: &DetailPage) -> String {
        match self.details.get(page) {
            Some(d) => fsv_views::pages::reporter_markup(&d),
            None => BasicPages.reporter_markup(page),
        }
    }

    fn main_lines(&self, page: &DetailPage) -> Vec<String> {
        match self.details.get(page) {
            Some(d) => fsv_views::pages::main_lines(&d),
            None => BasicPages.main_lines(page),
        }
    }

    fn notes_lines(&self, page: &DetailPage) -> Vec<String> {
        match self.details.get(page) {
            Some(d) => fsv_views::pages::notes_lines(&d),
            None => BasicPages.notes_lines(page),
        }
    }

    fn side_lines(&self, page: &DetailPage) -> Vec<String> {
        match self.details.get(page) {
            Some(d) => fsv_views::pages::side_lines(&d),
            None => BasicPages.side_lines(page),
        }
    }

    fn tab_lines(
        &self,
        data: &TabData,
        tz: Option<&TimeZone>,
        schema: Option<&Value>,
    ) -> Vec<String> {
        fsv_views::pages::tab_lines(&views_tab(data), tz, schema)
    }

    fn tab_bar(&self, page: &DetailPage, active: &str, numbered: bool) -> String {
        match self.details.get(page) {
            Some(d) => fsv_views::pages::tab_bar(&d, active, numbered),
            None => BasicPages.tab_bar(page, active, numbered),
        }
    }
}

/// `fsv tui [--read-only]` on the signed-in profile.
pub fn run_tui(read_only: bool) -> Result<()> {
    let backend = LiveBackend::live(fsv_core::get_client()?);
    let pages = Arc::new(backend.pages());
    fsv_tui::launch(read_only, Arc::new(backend), pages)?;
    Ok(())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_actions;
