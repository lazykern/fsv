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
use fsv_api::associations::SendOpts;
use fsv_api::notes::{self, BodyFormat, NotePlan};
use fsv_api::service::{self as svc, Api, Journal, ListArgs};
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

/// `schema.load(res, client)`: the list/detail schema of one resource
/// (lives in fsv-cli; injected).
pub type SchemaLoader = Arc<dyn Fn(Res) -> Result<Value> + Send + Sync>;

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
    /// `config.load_config()`.
    pub config: Map<String, Value>,
    pub details: DetailCache,
    me: Mutex<Option<Value>>,
}

fn session(e: SessionError) -> Failure {
    Failure::from(Error::Session(e.0))
}

fn id_of(v: &Value) -> BResult<i64> {
    change_id(v).map_err(Failure::from)
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
    /// The current profile's journal and config.
    pub fn new(client: Arc<Client>, schema: SchemaLoader) -> LiveBackend {
        LiveBackend {
            client,
            schema,
            journal: Arc::new(svc::DiskJournal),
            config: fsv_core::config::load_config(),
            details: DetailCache::default(),
            me: Mutex::new(None),
        }
    }

    pub fn with_journal(mut self, journal: Arc<dyn Journal + Send + Sync>) -> LiveBackend {
        self.journal = journal;
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

    fn load_views(&self, res: Res) -> BResult<Vec<View>> {
        let Some(path) = res.filters_path else {
            return Ok(Vec::new());
        };
        let data = match fsv_views::source::optional_get(&mut self.src(), path, None, "saved views")
            .map_err(session)?
        {
            Ok(d) => d,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(site::parse_saved_views(&data)
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
        let got = fsv_views::source::optional_get(
            &mut self.src(),
            "tickets/headers",
            None,
            "list headers",
        )
        .map_err(session)?;
        Ok(match got {
            Ok(Value::Object(m)) => m
                .get("headers")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter(|h| h.is_object()).cloned().collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        })
    }

    fn view_settings(&self, res: Res, view: &View) -> BResult<Value> {
        let Some(path) = res.filters_path else {
            return Ok(json!({}));
        };
        let got = fsv_views::source::optional_get(
            &mut self.src(),
            &format!("{path}/{}", view.id),
            None,
            "view settings",
        )
        .map_err(session)?;
        Ok(match got {
            Ok(Value::Object(m)) => m
                .values()
                .find(|v| v.is_object())
                .cloned()
                .unwrap_or(Value::Object(m)),
            _ => json!({}),
        })
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
        let d = fsv_views::load_detail(&mut self.src(), res, input, None).map_err(session)?;
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

/// `fsv tui [--read-only]` on the signed-in profile. `schema` loads a
/// resource's schema (`schema.load`, owned by the CLI).
pub fn run_tui(read_only: bool, schema: SchemaLoader) -> Result<()> {
    let backend = LiveBackend::new(fsv_core::get_client()?, schema);
    let pages = Arc::new(backend.pages());
    fsv_tui::launch(read_only, Arc::new(backend), pages)?;
    Ok(())
}

#[cfg(test)]
mod tests;
