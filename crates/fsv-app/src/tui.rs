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
use fsv_api::conversations::{self, ConversationArgs, ConversationPlan, MailArgs};
use fsv_api::formmeta::{ClientResolver, DiskStore, MetaStore};
use fsv_api::notes::{self, BodyFormat, NotePlan};
use fsv_api::planning_text;
use fsv_api::problem_known_error;
use fsv_api::properties::{self, AnsweredPropertyPlan, PropertyArgs, PropertyPlan};
use fsv_api::reviewed_write::WritePlan;
use fsv_api::service::{self as svc, Api, Journal, ListArgs};
use fsv_api::tasks::{self, TaskArgs, TaskPlan};
use fsv_api::{QueryError, asset_picker, integrated_assets, site};
use fsv_core::{Client, Error, Result};
use fsv_forms::Kind;
use fsv_forms::spec::REQUESTED_FOR_TYPE;
use fsv_tui::backend::{
    AssetCandidates, AssetCandidatesRequest, BResult, ConversationRequest, DetailPage, Failure,
    FormRequest, FormView, Item, ListRequest, Missing, PropertyField, ReadOnly, Res, ReviewPlan,
    SearchResult, TabData, TabInfo, View,
};
use fsv_tui::{Backend, BasicPages, Pages};
use fsv_views::{Detail, DetailInput, SessionError};

use crate::form::FormDeps;
use crate::source::ClientSource;
use crate::writer::change_id;

pub mod desktop;
pub mod files;
pub mod forms;

pub use desktop::{Browser, Clipboard};

/// `schema.load(res, client)`: the list/detail schema of one resource.
pub type SchemaLoader = Arc<dyn Fn(Res) -> Result<Value> + Send + Sync>;

/// `schema.load(res, client)` ([`fsv_api::schema::load_in_quiet`]) cached
/// under the client's profile paths for its tenant.
///
/// The stale-cache warning is dropped: Python prints it to stderr while the
/// Textual app is running, which captures stderr and never draws it.
pub fn schema_loader(client: Arc<Client>) -> SchemaLoader {
    Arc::new(move |res: Res| {
        let key = fsv_core::config::cache_domain_key(Some(&client.domain()));
        let paths = client.ctx().paths.clone();
        fsv_api::schema::load_in_quiet(res, &client, false, &paths, &key).map(|(doc, _)| doc)
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
    /// What `N` / `e` forms talk to; `None`: [`FormDeps::live`] per form.
    form_deps: Option<forms::DepsFactory>,
    /// Reviewed conversation plans by hash (the most recent
    /// [`PLAN_CACHE_SIZE`]): their [`ReviewPlan::raw`] is only the payload.
    conversations: Mutex<IndexMap<String, ConversationPlan>>,
    /// `schema.load(CHANGES, force=True)` for planning definitions.
    change_schema: Arc<ChangeSchemaFn>,
}

type ChangeSchemaFn = dyn Fn(&dyn Api) -> Result<Value> + Send + Sync;

const PLAN_CACHE_SIZE: usize = 16;

/// `_load_ticket_property_fields`' core ticket fields.
const PROPERTY_CORE: &[&str] = &[
    "subject",
    "description",
    "requester",
    "requested_for",
    "group",
    "agent",
    "status",
    "priority",
    "urgency",
    "impact",
    "source",
    "category",
    "ticket_type",
];

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
        recipient_context_json: str_at(raw, "recipient_context_json"),
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
            form_deps: None,
            conversations: Mutex::new(IndexMap::new()),
            change_schema: Arc::new(planning_text::live_change_schema),
        }
    }

    pub fn with_form_deps(mut self, deps: forms::DepsFactory) -> LiveBackend {
        self.form_deps = Some(deps);
        self
    }

    /// Planning definitions read the change schema through `schema`.
    pub fn with_change_schema(
        mut self,
        schema: impl Fn(&dyn Api) -> Result<Value> + Send + Sync + 'static,
    ) -> LiveBackend {
        self.change_schema = Arc::new(schema);
        self
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

    /// `ClientResolver(c)` for lookup answers.
    fn resolver(&self) -> Arc<dyn fsv_forms::LookupResolver> {
        Arc::new(ClientResolver::new(self.client.clone()))
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
        let env = std::env::var("FSV_COMPLETION_NETWORK").ok();
        fsv_completion::completion_network(env.as_deref(), &Value::Object(self.config.clone()))
    }

    fn keep_conversation(&self, p: ConversationPlan) {
        let mut g = self.conversations.lock().unwrap_or_else(|p| p.into_inner());
        g.insert(p.hash(), p);
        while g.len() > PLAN_CACHE_SIZE {
            g.shift_remove_index(0);
        }
    }

    fn conversation(&self, hash: &str) -> Option<ConversationPlan> {
        self.conversations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(hash)
            .cloned()
    }
}

/// A property plan as the TUI reviews it; [`ReviewPlan::raw`] carries its
/// fields, answers included, for the send.
pub fn property_review(a: &AnsweredPropertyPlan) -> ReviewPlan {
    let p = &a.plan;
    ReviewPlan {
        review: Value::Object(a.review()),
        hash: a.hash(),
        public: false,
        target_id: json!(p.ticket_id),
        raw: json!({"kind": "ticket_property", "ticket_id": p.ticket_id, "tenant": p.tenant, "body_json": p.body_json,
                    "before_json": p.before_json, "record_hash": p.record_hash, "answers_json": a.answers_json,
                    "metadata_hash": a.metadata_hash, "acknowledged_json": a.acknowledged_json}),
    }
}

fn property_plan(raw: &Value) -> AnsweredPropertyPlan {
    AnsweredPropertyPlan {
        plan: PropertyPlan {
            ticket_id: i64_at(raw, "ticket_id"),
            tenant: str_at(raw, "tenant"),
            body_json: str_at(raw, "body_json"),
            before_json: str_at(raw, "before_json"),
            record_hash: str_at(raw, "record_hash"),
        },
        answers_json: opt_str(raw, "answers_json").unwrap_or_else(|| "{}".into()),
        metadata_hash: str_at(raw, "metadata_hash"),
        acknowledged_json: opt_str(raw, "acknowledged_json").unwrap_or_else(|| "[]".into()),
    }
}

/// A `reviewed_write.WritePlan` as the TUI reviews it; [`ReviewPlan::raw`]
/// carries its fields for the send.
pub fn write_review(p: &WritePlan) -> ReviewPlan {
    ReviewPlan {
        review: Value::Object(p.review()),
        hash: p.hash(),
        public: false,
        target_id: json!(p.target),
        raw: json!({"kind": "reviewed_write", "action": p.action, "target": p.target, "tenant": p.tenant,
                    "method": p.method, "path": p.path, "payload_json": p.payload_json,
                    "context_hash": p.context_hash}),
    }
}

/// The [`WritePlan`] behind [`write_review`] when its action is `action`.
fn write_plan(raw: &Value, action: impl Fn(&str) -> bool) -> BResult<WritePlan> {
    if raw.get("kind").and_then(Value::as_str) != Some("reviewed_write")
        || !action(&str_at(raw, "action"))
    {
        return Err(Failure::value("not a plan for this action"));
    }
    Ok(WritePlan {
        action: str_at(raw, "action"),
        target: i64_at(raw, "target"),
        tenant: str_at(raw, "tenant"),
        method: str_at(raw, "method"),
        path: str_at(raw, "path"),
        payload_json: str_at(raw, "payload_json"),
        context_hash: str_at(raw, "context_hash"),
    })
}

/// `TaskPlan` fields as [`ReviewPlan::raw`] carries them.
fn task_raw(p: &TaskPlan) -> Value {
    json!({"kind": "change_task", "change_id": p.change_id, "title": p.title, "owner_id": p.owner_id,
           "tenant": p.tenant, "group_id": p.group_id, "status_label": p.status_label, "status_id": p.status_id,
           "note": p.note, "notify_before": p.notify_before, "planned_start": p.planned_start,
           "planned_end": p.planned_end, "planned_effort": p.planned_effort,
           "custom_fields_json": p.custom_fields_json, "fields_json": p.fields_json})
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
        custom_fields_json: raw
            .get("custom_fields_json")
            .and_then(Value::as_str)
            .unwrap_or("{}")
            .to_string(),
        fields_json: str_at(raw, "fields_json"),
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

fn form_value(value: &Value) -> fsv_forms::Value {
    match value {
        Value::Null => fsv_forms::Value::Null,
        Value::Bool(v) => fsv_forms::Value::Bool(*v),
        Value::Number(n) if n.as_i64().is_some() => fsv_forms::Value::Int(n.as_i64().unwrap_or(0)),
        Value::Number(n) if n.as_f64().is_some() => {
            fsv_forms::Value::Float(n.as_f64().unwrap_or(0.0))
        }
        Value::String(s) => fsv_forms::Value::Str(s.clone()),
        other => fsv_forms::Value::Json(other.clone()),
    }
}

fn change_status_session(
    backend: &LiveBackend,
    target: &Value,
    status: &Value,
) -> BResult<fsv_formui::FormSession> {
    let factory = backend.form_deps.clone().unwrap_or_else(|| {
        let client = backend.client.clone();
        Arc::new(move || crate::form::FormDeps::live(client.clone()))
    });
    let mut options = crate::form::ChangeForm::new(fsv_formui::FormMode::Edit);
    options.target = Some(target.clone());
    let mut session = crate::form::change_session(&factory()?, options)?;
    let set = session.set("status", form_value(status));
    if !set.error.is_empty() {
        return Err(Failure::value(set.error));
    }
    Ok(session)
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
        fsv_views::source::ticket_headers(&self.src()).map_err(session)
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

    /// `app._prepare_ticket_conversation`: CC and notify resolved to exact
    /// users, BCC looked up as a check only, then `build_plan`.
    fn ticket_conversation_plan(
        &self,
        ticket_id: &Value,
        body: &str,
        req: &ConversationRequest,
    ) -> BResult<ReviewPlan> {
        let cc = conversations::recipients(self.api(), &req.cc)?;
        let notify = conversations::recipients(self.api(), &req.notify)?;
        conversations::recipients(self.api(), &req.bcc)?;
        let tid = s(ticket_id);
        let args = ConversationArgs {
            kind: if req.reply { "reply" } else { "note" },
            cc: &cc,
            public: req.public,
            notify: &notify,
            format: BodyFormat::Plain,
            set_status: req.set_status.as_deref(),
        };
        let mail = MailArgs {
            bcc: &req.bcc,
            sender: req.sender.as_deref().filter(|v| !v.is_empty()),
        };
        let p = conversations::build_plan_mail(
            self.api(),
            self.store.as_ref(),
            &tid,
            body,
            &args,
            &mail,
        )?;
        let payload = p.payload();
        let plan = ReviewPlan {
            review: Value::Object(p.review()),
            hash: p.hash(),
            public: p.kind == "reply" || payload.get("private") == Some(&Value::Bool(false)),
            target_id: json!(p.ticket_id),
            raw: payload,
        };
        self.keep_conversation(p);
        Ok(plan)
    }

    /// `ticket_conversation.send(plan, reviewed_hash=plan.hash, public_confirmed=...)`.
    fn send_ticket_conversation(
        &self,
        plan: &ReviewPlan,
        public_confirmed: bool,
    ) -> BResult<Value> {
        let Some(p) = self.conversation(&plan.hash) else {
            return Err(Failure::value(
                "the conversation plan was not built by this session; review it again",
            ));
        };
        Ok(conversations::send(
            &p,
            self.api(),
            self.journal.as_ref(),
            self.store.as_ref(),
            Some(&plan.hash),
            public_confirmed,
        )?)
    }

    /// `app._load_ticket_property_fields`: the fresh edit form's writable
    /// core and custom inputs.
    fn ticket_property_fields(&self, ticket_id: &Value) -> BResult<Vec<PropertyField>> {
        let meta = fsv_api::formmeta::load_meta(
            self.api(),
            self.store.as_ref(),
            "ticket",
            "edit",
            true,
            ticket_id,
        )?;
        let spec = meta.spec();
        Ok(spec
            .fields()
            .iter()
            .filter(|f| {
                (f.sent || f.field_type == REQUESTED_FOR_TYPE)
                    && !f.synthetic
                    && f.is_input()
                    && f.kind != Kind::Unsupported
                    && (f.is_custom || PROPERTY_CORE.contains(&f.api_name.as_str()))
            })
            .map(|f| PropertyField {
                name: f.api_name.clone(),
                label: f.label.clone(),
                choices: (f.kind == Kind::Dropdown).then(|| {
                    f.choices
                        .iter()
                        .filter(|ch| ch.visible)
                        .map(|ch| (json!(ch.value), ch.label.clone()))
                        .collect()
                }),
            })
            .collect())
    }

    /// `ticket_properties.build_plan(ticket_id, answers={name: value})`.
    fn ticket_property_plan(
        &self,
        ticket_id: &Value,
        name: &str,
        value: &Value,
    ) -> BResult<ReviewPlan> {
        let mut answers = Map::new();
        answers.insert(name.to_string(), value.clone());
        let args = PropertyArgs {
            answers: Some(&answers),
            ..PropertyArgs::default()
        };
        let p = properties::build_plan_with(
            self.api(),
            self.store.as_ref(),
            &s(ticket_id),
            &args,
            Some(self.resolver()),
        )?;
        Ok(property_review(&p))
    }

    /// `ticket_properties.send(plan, reviewed_hash=plan.hash)`.
    fn send_ticket_property(&self, plan: &ReviewPlan) -> BResult<Value> {
        if plan.raw.get("kind").and_then(Value::as_str) != Some("ticket_property") {
            return Err(Failure::value("not a ticket property plan"));
        }
        Ok(properties::send_with(
            &property_plan(&plan.raw),
            self.api(),
            self.journal.as_ref(),
            self.store.as_ref(),
            Some(&plan.hash),
            Some(self.resolver()),
        )?)
    }

    /// `problem_known_error.build_plan(problem_id)`.
    fn known_error_plan(&self, problem_id: &Value) -> BResult<ReviewPlan> {
        let p = problem_known_error::build_plan(self.api(), &s(problem_id))?;
        Ok(write_review(&p))
    }

    /// `problem_known_error.send(plan, reviewed_hash=plan.hash)`.
    fn send_known_error(&self, plan: &ReviewPlan) -> BResult<Value> {
        let p = write_plan(&plan.raw, |a| a == "problem_known_error_mark")?;
        Ok(problem_known_error::send(
            &p,
            self.api(),
            self.journal.as_ref(),
            Some(&plan.hash),
        )?)
    }

    /// `planning_text.definitions(module)`.
    fn planning_definitions(&self, module: &str) -> BResult<Vec<Value>> {
        Ok(planning_text::definitions(
            self.api(),
            module,
            self.change_schema.as_ref(),
        )?)
    }

    /// `match_planning_row({"name": section}, planning_text.inventory(module, id))`.
    fn planning_row(&self, module: &str, id: &Value, section: &str) -> BResult<Option<Value>> {
        let rows = planning_text::inventory(self.api(), module, id_of(id)?)?;
        Ok(fsv_api::create::match_planning_row(
            &json!({"name": section}),
            &rows,
        )?)
    }

    /// `planning_text.build_plan(module, id, section, description)`.
    fn planning_text_plan(
        &self,
        module: &str,
        id: &Value,
        section: &str,
        description: &str,
    ) -> BResult<ReviewPlan> {
        let p = planning_text::build_plan(
            self.api(),
            module,
            &s(id),
            section,
            description,
            self.change_schema.as_ref(),
        )?;
        let mut review = write_review(&p.plan);
        review.review = Value::Object(p.review());
        review.hash = p.hash();
        review.raw["before"] = json!(p.before);
        Ok(review)
    }

    /// `planning_text.send(plan, reviewed_hash=plan.hash)`.
    fn send_planning_text(&self, plan: &ReviewPlan) -> BResult<Value> {
        let p = planning_text::PlanningTextPlan {
            plan: write_plan(&plan.raw, |a| a.ends_with("_planning_text"))?,
            before: str_at(&plan.raw, "before"),
        };
        Ok(planning_text::send(
            &p,
            self.api(),
            self.journal.as_ref(),
            Some(&plan.hash),
            self.change_schema.as_ref(),
        )?)
    }

    /// `integrated_assets.search` page 1, as picker rows (`display_id` is
    /// `name.id`, `name` is the label, `ci_type_name` is `type`).
    fn inventory_search(&self, query: &str, kind: &str) -> BResult<AssetCandidates> {
        let raw = integrated_assets::search(self.api(), query, kind, 1)?;
        let data = raw
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut assets = Vec::with_capacity(data.len());
        for row in data {
            let name = row.get("name").cloned().unwrap_or(Value::Null);
            let ident = name.get("id").cloned().unwrap_or(Value::Null);
            let display_id = fsv_tui::backend::positive_id(&ident)?;
            let mut shaped = Map::new();
            shaped.insert("display_id".into(), json!(display_id));
            shaped.insert(
                "name".into(),
                name.get("label").cloned().unwrap_or(Value::Null),
            );
            shaped.insert(
                "ci_type_name".into(),
                row.get("type").cloned().unwrap_or(Value::Null),
            );
            shaped.insert("inventory".into(), row);
            assets.push(shaped);
        }
        Ok(AssetCandidates {
            change_id: 0,
            page: 1,
            scope: "single search page; not a complete inventory".into(),
            assets,
            mutation: "none".into(),
            warning: String::new(),
        })
    }

    fn associated_inventory(
        &self,
        module: &str,
        target: &Value,
        kind: &str,
    ) -> BResult<Vec<Value>> {
        Ok(integrated_assets::inventory(
            self.api(),
            module,
            target,
            kind,
        )?)
    }

    fn inventory_association_plan(
        &self,
        module: &str,
        target: &Value,
        ident: &Value,
        kind: &str,
        remove: bool,
    ) -> BResult<ReviewPlan> {
        let plan = integrated_assets::build_plan(
            self.api(),
            module,
            target,
            std::slice::from_ref(ident),
            kind,
            remove,
        )?;
        Ok(write_review(&plan))
    }

    fn send_inventory_association(
        &self,
        plan: &ReviewPlan,
        module: &str,
        ident: &Value,
        kind: &str,
        remove: bool,
    ) -> BResult<Value> {
        let write = write_plan(&plan.raw, |action| {
            action == "integrated_asset_add" || action == "integrated_asset_remove"
        })?;
        Ok(integrated_assets::send(
            &write,
            self.api(),
            self.journal.as_ref(),
            module,
            std::slice::from_ref(ident),
            kind,
            remove,
            Some(&plan.hash),
        )?)
    }

    /// `asset_picker.candidates(c, change, query, page=page)`.
    fn asset_candidates(&self, req: &AssetCandidatesRequest) -> BResult<AssetCandidates> {
        let got = asset_picker::candidates(self.api(), &req.change, &req.query, req.page)?;
        Ok(AssetCandidates {
            change_id: got.change_id,
            page: got.page,
            scope: got.scope,
            assets: got.assets,
            mutation: got.mutation,
            warning: got.warning,
        })
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
            let conv = fsv_views::source::load_conversations(&self.src(), &rid, true, 3)
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

    fn task_mutation_plan(
        &self,
        kind: &str,
        change_id: &Value,
        task_id: &Value,
        body: &Map<String, Value>,
    ) -> BResult<ReviewPlan> {
        if kind == "task-delete" {
            let plan = fsv_api::change_task_delete::build_plan(self.api(), change_id, task_id)?;
            return Ok(write_review(&plan));
        }
        let mut inputs = Map::new();
        for (key, value) in body {
            let name = if key == "owner_id" || key == "group_id" {
                key.trim_end_matches("_id").to_string()
            } else {
                key.clone()
            };
            inputs.insert(name, Value::String(fsv_py::py_str(value)));
        }
        let plan =
            fsv_api::change_task_update::build_plan(self.api(), change_id, task_id, &inputs)?;
        let mut review = write_review(&plan.write);
        review.hash = plan.hash();
        review.review = Value::Object(plan.review());
        if let Some(raw) = review.raw.as_object_mut() {
            raw.insert("task_id".into(), json!(plan.task_id));
            raw.insert("inputs_json".into(), json!(plan.inputs_json));
        }
        Ok(review)
    }

    fn send_task_mutation(&self, plan: &ReviewPlan) -> BResult<Value> {
        let action = str_at(&plan.raw, "action");
        if action == "change_task_delete" {
            let write = write_plan(&plan.raw, |name| name == "change_task_delete")?;
            return Ok(fsv_api::change_task_delete::send(
                &write,
                self.api(),
                self.journal.as_ref(),
                Some(&plan.hash),
            )?);
        }
        let write = write_plan(&plan.raw, |name| name == "change_task_update")?;
        let update = fsv_api::change_task_update::TaskUpdatePlan {
            write,
            task_id: i64_at(&plan.raw, "task_id"),
            inputs_json: str_at(&plan.raw, "inputs_json"),
        };
        Ok(fsv_api::change_task_update::send(
            &update,
            self.api(),
            self.journal.as_ref(),
            Some(&plan.hash),
        )?)
    }

    fn problem_note_plan(&self, problem_id: &Value, body: &str) -> BResult<ReviewPlan> {
        let plan =
            fsv_api::problem_workflows::child_plan(self.api(), &s(problem_id), "note", body)?;
        let mut review = write_review(&plan.plan);
        review.hash = plan.hash();
        review.review = Value::Object(plan.review());
        if let Some(raw) = review.raw.as_object_mut() {
            raw.insert("inputs_json".into(), json!(plan.inputs_json));
        }
        Ok(review)
    }

    fn send_problem_note(&self, plan: &ReviewPlan) -> BResult<Value> {
        let write = write_plan(&plan.raw, |name| name == "problem_note")?;
        let child = fsv_api::problem_workflows::ProblemPlan {
            plan: write,
            inputs_json: str_at(&plan.raw, "inputs_json"),
            warnings_json: "[]".into(),
        };
        Ok(fsv_api::problem_workflows::send(
            &child,
            self.api(),
            self.journal.as_ref(),
            self.store.as_ref(),
            Some(&plan.hash),
            Some(self.resolver()),
        )?)
    }

    fn change_status_plan(&self, target: &Value, status: &Value) -> BResult<ReviewPlan> {
        let mut session = change_status_session(self, target, status)?;
        let plan = session.build_plan(None).map_err(Failure::value)?;
        if !plan.ok() {
            return Err(Failure::value(plan.errors.join("; ")));
        }
        Ok(ReviewPlan {
            review: json!({
                "plan_hash": plan.hash,
                "errors": plan.errors,
                "payload": plan.payload,
            }),
            hash: plan.hash,
            public: false,
            target_id: target.clone(),
            raw: json!({"kind": "change_status", "target": target, "status": status}),
        })
    }

    fn send_change_status(&self, plan: &ReviewPlan) -> BResult<Value> {
        if plan.raw.get("kind").and_then(Value::as_str) != Some("change_status") {
            return Err(Failure::value("not a change status plan"));
        }
        let target = plan.raw.get("target").cloned().unwrap_or(Value::Null);
        let status = plan.raw.get("status").cloned().unwrap_or(Value::Null);
        let mut session = change_status_session(self, &target, &status)?;
        let fresh = session.build_plan(None).map_err(Failure::value)?;
        if !fresh.ok() || fresh.hash != plan.hash {
            return Err(Failure::value(
                "Change tenant or exact reviewed payload changed; review again",
            ));
        }
        Ok(session.send(&fresh, false)?)
    }

    /// `catalog.list_items(client)` then `catalog.list_categories(client)`.
    fn forms_available(&self) -> bool {
        true
    }

    fn open_form(&self, req: &FormRequest) -> BResult<Box<dyn FormView>> {
        let deps = self.form_deps.clone().unwrap_or_else(|| {
            let client = self.client.clone();
            Arc::new(move || FormDeps::live(client.clone()))
        });
        Ok(Box::new(forms::open(deps, req.clone())?))
    }

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
    let read_only = fsv_tui::actions::resolve_read_only(read_only, &backend.read_only_setting());
    let backend: Arc<dyn Backend> = if read_only {
        Arc::new(ReadOnly(backend))
    } else {
        Arc::new(backend)
    };
    fsv_tui::launch(read_only, backend, pages)?;
    Ok(())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_actions;
#[cfg(test)]
mod tests_reviewed;
