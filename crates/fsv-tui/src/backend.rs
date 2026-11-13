//! Service seams the TUI calls (`fsv.service`, `fsv.views` loaders,
//! `fsv.pages` renderers, writers, config).
//!
//! The TUI binds to these traits; `fsv_app::LiveBackend` implements them
//! over the service crates and tests substitute recorders. Network methods
//! have no defaults (test doubles opt into stubs with [`backend_stubs!`]);
//! a failure becomes a toast or a failure screen, never a panic.
//! [`ReadOnly`] refuses every write at the backend.

use std::fmt;
use std::time::Duration;

use fsv_core::Error;
use fsv_core::resource::Resource;
use fsv_core::term::escape;
use jiff::tz::TimeZone;
use serde_json::{Map, Value};

/// The types [`FormView`] is written in, for implementors.
pub use crossterm::event::KeyEvent;
pub use ratatui::buffer::Buffer;
pub use ratatui::layout::{Position, Rect};

pub type Res = &'static Resource;

/// A failed service call plus the partial-outcome attributes Python's
/// `failure_detail` prints (`created_id`, `steps`, `uploaded`, `error`).
#[derive(Debug)]
pub struct Failure {
    pub error: Box<Error>,
    pub outcome: Vec<(String, String)>,
}

impl Failure {
    pub fn unavailable(what: &str) -> Failure {
        Failure::from(Error::other(format!(
            "{what} is not available in this build"
        )))
    }

    pub fn read_only(what: &str) -> Failure {
        Failure::from(Error::other(format!(
            "read-only mode: writes are disabled ({what})"
        )))
    }

    pub fn value(msg: impl Into<String>) -> Failure {
        Failure::from(Error::value(msg))
    }

    pub fn is_session(&self) -> bool {
        matches!(*self.error, Error::Session(_))
    }

    pub fn is_value(&self) -> bool {
        matches!(*self.error, Error::Value(_))
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Failure {
            error: Box::new(error),
            outcome: Vec::new(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

pub type BResult<T> = Result<T, Failure>;

/// A record with the resource it came from (Python's `item["_resource"]`).
#[derive(Clone, Debug)]
pub struct Item {
    pub res: Res,
    pub data: Value,
}

impl Item {
    pub fn new(res: Res, data: Value) -> Item {
        Item { res, data }
    }

    pub fn id(&self) -> Value {
        self.data.get("id").cloned().unwrap_or(Value::Null)
    }

    pub fn id_str(&self) -> String {
        fsv_py::py_str(&self.id())
    }

    /// `item.get("display_id") or item["id"]`.
    pub fn ref_id(&self) -> Value {
        match self.data.get("display_id") {
            Some(v) if fsv_py::truthy(v) => v.clone(),
            _ => self.id(),
        }
    }

    pub fn display_id(&self) -> String {
        fsv_core::resource::format_id(&self.data, self.res)
    }

    /// `FsvApp._item_key`: `<resource>:<id>`, the detail / sub cache key.
    pub fn key(&self) -> String {
        format!("{}:{}", self.res.name, self.id_str())
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    pub fn str(&self, key: &str) -> String {
        match self.data.get(key) {
            Some(Value::Null) | None => String::new(),
            Some(v) => fsv_py::py_str(v),
        }
    }
}

/// One saved list view (`views.SavedView`).
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub id: String,
    pub name: String,
    pub system: bool,
    pub marked_default: bool,
    pub raw: Value,
}

/// A section the tenant did not provide (`views.Missing`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub what: String,
    pub status: Option<i64>,
}

impl Missing {
    pub fn new(what: impl Into<String>) -> Missing {
        Missing {
            what: what.into(),
            status: None,
        }
    }

    pub fn note(&self) -> String {
        match self.status {
            Some(403) => format!("{}: not permitted for your role (403)", self.what),
            Some(404) => format!("{}: not available on this account (404)", self.what),
            Some(s) if s != 0 => format!("{}: unavailable ({s})", self.what),
            _ => format!("{}: unavailable", self.what),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabInfo {
    pub name: String,
    pub label: String,
    pub count: Option<i64>,
}

/// A loaded detail page (`views.Detail`). `extra` carries whatever the
/// [`Pages`] renderer needs beyond the record.
#[derive(Clone, Debug)]
pub struct DetailPage {
    pub res: Res,
    pub record: Value,
    pub tabs: Vec<TabInfo>,
    pub badge: String,
    pub due: String,
    pub tz: Option<TimeZone>,
    pub schema: Option<Value>,
    pub extra: Value,
}

impl DetailPage {
    pub fn tab_names(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.name.clone()).collect()
    }
}

/// One non-details tab (`views.TabData`).
#[derive(Clone, Debug, Default)]
pub struct TabData {
    pub name: String,
    pub label: String,
    pub data: Value,
    pub missing: Option<Missing>,
    pub parts: Map<String, Value>,
    pub notes: Vec<String>,
    pub meta: Map<String, Value>,
    pub parts_meta: Map<String, Value>,
}

/// A reviewed write plan (ticket note, change task, association). `review`
/// is what the confirm screen shows; `hash` is passed back on send.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewPlan {
    pub review: Value,
    pub hash: String,
    pub public: bool,
    pub target_id: Value,
    pub raw: Value,
}

#[derive(Clone, Debug)]
pub struct ListRequest {
    pub res: Res,
    pub page: i64,
    pub per_page: i64,
    pub query_hash: Option<String>,
    pub filter_name: Option<String>,
    pub order_by: String,
    pub order_type: String,
}

/// Search results: items and per-entity totals in server order.
pub type SearchResult = (Vec<Item>, Vec<(String, i64)>);

pub const TASK_STATUS_LABELS: &[&str] = &["Open", "In Progress", "Completed", "Blocked"];
pub const TASK_NOTIFY_BEFORE: &[&str] = &[
    "Never",
    "15 Minutes",
    "30 Minutes",
    "45 Minutes",
    "1 Hour",
    "2 Hours",
];

/// One editable ticket/SR property (`FieldSpec` after `_load_ticket_property_fields`'
/// filter). `choices` is `Some` for a dropdown: its visible `(value, label)` pairs.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyField {
    pub name: String,
    pub label: String,
    pub choices: Option<Vec<(Value, String)>>,
}

/// `_prepare_ticket_conversation`'s keyword arguments, after the TUI split
/// the comma-separated option strings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConversationRequest {
    /// `kind="reply"`, else `"note"`.
    pub reply: bool,
    pub public: bool,
    pub cc: Vec<String>,
    pub notify: Vec<String>,
    pub bcc: Vec<String>,
    pub sender: Option<String>,
    /// `Some("Pending")` when the Pending toggle is on.
    pub set_status: Option<String>,
}

/// `asset_picker.candidates(client, target, query, page=page)`'s arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetCandidatesRequest {
    /// The change's `display_id or id`, unparsed (`candidates` runs
    /// `positive_id(parse_id(str(target), CHANGES))`).
    pub change: Value,
    pub query: String,
    /// The TUI always asks for page 1.
    pub page: i64,
}

/// `asset_picker.candidates`' result dict, one search page.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetCandidates {
    pub change_id: i64,
    pub page: i64,
    /// `"single search page; not a complete inventory"`.
    pub scope: String,
    /// The `assets` rows as returned, each with a unique positive `display_id`.
    pub assets: Vec<Map<String, Value>>,
    /// `"none"`.
    pub mutation: String,
    pub warning: String,
}

impl AssetCandidates {
    /// Port of fsv.asset_picker.select.
    pub fn select(&self, display_id: &Value) -> BResult<&Map<String, Value>> {
        let display_id = positive_id(display_id)?;
        let mut rows = self.assets.iter().filter(|row| {
            row.get("display_id").and_then(|v| positive_id(v).ok()) == Some(display_id)
        });
        match (rows.next(), rows.next()) {
            (Some(row), None) => Ok(row),
            _ => Err(Failure::value(
                "exact asset display ID is not unique on this search page",
            )),
        }
    }
}

/// Port of fsv.watchers.positive_id: a positive decimal int or digit string,
/// never a bool. Like `int()`, any Unicode decimal digits count; a
/// `str.isdigit` character that is not one (`²`) is `int()`'s `ValueError`.
pub fn positive_id(value: &Value) -> BResult<i64> {
    if let Value::String(s) = value
        && !s.is_ascii()
        && fsv_core::pystr::is_digit(s)
    {
        let ascii: Option<String> = s
            .chars()
            .map(|c| fsv_core::pystr::decimal_value(c).and_then(|d| char::from_digit(d, 10)))
            .collect();
        let Some(ascii) = ascii else {
            return Err(Failure::value(format!(
                "invalid literal for int() with base 10: {}",
                fsv_py::py_repr_str(s)
            )));
        };
        return positive_id(&Value::String(ascii));
    }
    let s = fsv_py::py_str(value);
    match value {
        Value::Bool(_) => None,
        _ if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) => None,
        _ => s.parse::<i64>().ok().filter(|&n| n > 0),
    }
    .ok_or_else(|| Failure::value("expected a positive numeric identity"))
}

/// The form `N` / `e` open (`_build_form(mode, item, ...)`).
#[derive(Clone, Debug)]
pub struct FormRequest {
    /// `new` (a change), `edit` (the selected change), `incident` or `sr`.
    pub mode: String,
    /// The selected change's row (`edit`) or the picked catalog item (`sr`).
    pub item: Option<Value>,
    /// `edit`: the loaded detail record, else the list row.
    pub record: Option<Value>,
}

/// An open full-screen form (`fsv.formui.FormScreen`) the TUI drives with
/// keys, ticks and draws until it is done.
pub trait FormView: Send {
    fn tick(&mut self, now: Duration);
    fn handle_key(&mut self, key: &KeyEvent);
    fn render(&mut self, area: Rect, buf: &mut Buffer) -> Option<Position>;
    fn done(&self) -> bool;
    /// Text the form asked to put on the clipboard.
    fn take_clipboard(&mut self) -> Option<String> {
        None
    }
    fn finish(self: Box<Self>) -> FormExit;
}

/// How a form ended (`FormScreen`'s result `action`).
pub enum FormExit {
    Cancelled,
    /// `save_draft`, with the saved path.
    Draft(String),
    /// `submit` with a reviewed plan, ready for `session.send(plan)`.
    Submit(Box<dyn FormSend>),
}

/// A submitted form's send (`session.send(result.plan)`).
pub trait FormSend: Send {
    fn send(self: Box<Self>) -> BResult<Value>;
}

#[allow(unused_variables)]
pub trait Backend: Send + Sync {
    // ── settings ────────────────────────────────────────────
    /// `config.tui.read_only` (raw value).
    fn read_only_setting(&self) -> Value {
        Value::Null
    }
    fn default_where(&self, entity: &str) -> Vec<String> {
        Vec::new()
    }
    fn load_filter_history(&self) -> Vec<String> {
        Vec::new()
    }
    fn save_filter_history(&self, history: &[String]) {}
    fn copy_to_clipboard(&self, text: &str) -> Result<(), String> {
        Err("clipboard not available".into())
    }
    fn open_url(&self, url: &str) -> bool {
        false
    }
    fn item_url(&self, res: Res, id: &Value) -> String {
        format!(
            "https://{}/a/{}/{}",
            fsv_core::config::domain(),
            res.api_path,
            fsv_py::py_str(id)
        )
    }

    // ── loaders ─────────────────────────────────────────────
    fn load_schema(&self, res: Res) -> BResult<Value>;
    fn current_user(&self) -> BResult<Value>;
    fn user_tz(&self, me: &Value) -> Option<TimeZone> {
        None
    }
    fn load_views(&self, res: Res) -> BResult<Vec<View>>;
    fn ticket_headers(&self) -> BResult<Vec<Value>>;
    fn view_settings(&self, res: Res, view: &View) -> BResult<Value>;
    /// `query.build_query_hash_from_schema`; a `Value` error is a filter error.
    fn build_query_hash(&self, res: Res, schema: &Value, filters: &[String]) -> BResult<String>;
    /// `service.list_page` → `(items, raw response)`.
    fn list_page(&self, req: &ListRequest) -> BResult<(Vec<Value>, Value)>;
    fn search_items(
        &self,
        term: &str,
        entity: Option<&str>,
        sort: &str,
        page: i64,
    ) -> BResult<SearchResult>;
    fn load_detail(&self, res: Res, id: &Value, schema: Option<&Value>) -> BResult<DetailPage>;
    fn load_tab(&self, res: Res, record: &Value, tab: &str) -> BResult<TabData>;

    // ── filter completion ───────────────────────────────────
    fn completion_network(&self) -> bool {
        false
    }
    fn cached_group_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// `completion._remote_user_values(kind, prefix)` values.
    fn remote_user_values(&self, kind: &str, prefix: &str) -> BResult<Vec<String>>;
    fn lookup_choices(&self, link: &str, prefix: &str) -> BResult<Vec<Value>>;

    // ── writes ──────────────────────────────────────────────
    fn update_item(&self, res: Res, id: &Value, body: &Map<String, Value>) -> BResult<Value>;
    fn add_note(&self, res: Res, id: &Value, body: &str, public: bool) -> BResult<Value>;
    fn ticket_note_plan(&self, ticket_id: &Value, body: &str, public: bool) -> BResult<ReviewPlan>;
    fn send_ticket_note(
        &self,
        plan: &ReviewPlan,
        public_confirmed: bool,
        read_only: bool,
    ) -> BResult<Value>;
    /// `change_tasks.build_plan` after resolving owner, group and status.
    fn change_task_plan(
        &self,
        change_id: &Value,
        title: &str,
        owner: Option<&str>,
        options: &[(String, String)],
    ) -> BResult<ReviewPlan>;
    /// `_change_task_choices`: runtime status or reminder labels.
    /// `key` is `status` or `notify_before`.
    fn task_create_choices(&self, _key: &str) -> BResult<Vec<String>> {
        Err(Failure::unavailable("task choices"))
    }
    fn send_change_task(&self, plan: &ReviewPlan, read_only: bool) -> BResult<Value>;
    fn search_change_tickets(&self, query: &str) -> BResult<Vec<Value>>;
    /// `actions.download_choices` → `(rows, warnings)`.
    fn download_choices(&self, item: &Item) -> BResult<(Vec<Value>, Vec<String>)>;
    fn download(&self, attachment: &Value, destination: &str) -> BResult<Value>;
    /// `create.get_task_for_edit` plus `task_status.fields_from`.
    fn task_for_edit(&self, change_id: &Value, task_id: &Value) -> BResult<(Value, Vec<Value>)>;
    fn association_plan(
        &self,
        change_id: &Value,
        ticket_ids: &[Value],
        unlink: bool,
    ) -> BResult<ReviewPlan>;
    fn send_association(&self, plan: &ReviewPlan) -> BResult<Value>;
    /// `change_approvals.resolve_approvers` + `build_plan` (a request plan).
    fn approval_request_plan(
        &self,
        change_id: &Value,
        name: &str,
        message: &str,
        recipients: &[String],
        rule: &str,
    ) -> BResult<ReviewPlan>;
    /// `approval-respond` → approval records; otherwise approval groups.
    fn approval_targets(&self, kind: &str, change_id: &Value) -> BResult<Vec<Value>>;
    /// `change_approval_actions.build_plan` (approve / reject / cancel).
    fn approval_action_plan(
        &self,
        change_id: &Value,
        target_id: &Value,
        operation: &str,
        remark: Option<&str>,
    ) -> BResult<ReviewPlan>;
    /// Sends a request or action plan once; the result carries `verified`.
    fn send_approval(&self, plan: &ReviewPlan, read_only: bool) -> BResult<Value>;
    fn update_task(
        &self,
        change_id: &Value,
        task_id: &Value,
        body: &Map<String, Value>,
    ) -> BResult<()>;
    fn delete_task(&self, change_id: &Value, task_id: &Value) -> BResult<()>;

    // ── reviewed single-purpose writes ──────────────────────
    /// `load_meta("ticket", "edit", fresh=True, module_id=ticket_id)` filtered
    /// to the fields `e` offers on a ticket.
    fn ticket_property_fields(&self, ticket_id: &Value) -> BResult<Vec<PropertyField>>;
    /// `ticket_properties.build_plan(ticket_id, answers={name: value})`.
    fn ticket_property_plan(
        &self,
        ticket_id: &Value,
        name: &str,
        value: &Value,
    ) -> BResult<ReviewPlan>;
    /// `ticket_properties.send(plan, reviewed_hash=plan.hash)`.
    fn send_ticket_property(&self, plan: &ReviewPlan) -> BResult<Value>;
    /// `problem_known_error.build_plan(problem_id)`.
    fn known_error_plan(&self, problem_id: &Value) -> BResult<ReviewPlan>;
    /// `problem_known_error.send(plan, reviewed_hash=plan.hash)`.
    fn send_known_error(&self, plan: &ReviewPlan) -> BResult<Value>;
    /// `planning_text.definitions(module)`; `module` is `changes` or `problems`.
    fn planning_definitions(&self, module: &str) -> BResult<Vec<Value>>;
    /// `match_planning_row({"name": section}, planning_text.inventory(module, id))`.
    fn planning_row(&self, module: &str, id: &Value, section: &str) -> BResult<Option<Value>>;
    /// `planning_text.build_plan(module, id, section, description)`.
    fn planning_text_plan(
        &self,
        module: &str,
        id: &Value,
        section: &str,
        description: &str,
    ) -> BResult<ReviewPlan>;
    /// `planning_text.send(plan, reviewed_hash=plan.hash)`.
    fn send_planning_text(&self, plan: &ReviewPlan) -> BResult<Value>;
    /// `ticket_conversation.recipients` for cc / notify / bcc, then
    /// `build_plan`. `raw` must be the plan's payload: the TUI reads
    /// `private` from it to decide on the public-consent step.
    fn ticket_conversation_plan(
        &self,
        ticket_id: &Value,
        body: &str,
        req: &ConversationRequest,
    ) -> BResult<ReviewPlan>;
    /// `ticket_conversation.send(plan, reviewed_hash=plan.hash, public_confirmed=...)`.
    fn send_ticket_conversation(&self, plan: &ReviewPlan, public_confirmed: bool)
    -> BResult<Value>;

    // ── read-only pickers ───────────────────────────────────
    /// `asset_picker.candidates(c, req.change, req.query, page=req.page)`:
    /// one GET of `/itam/core/device/api/`; never associates.
    fn asset_candidates(&self, req: &AssetCandidatesRequest) -> BResult<AssetCandidates>;
    /// `integrated_assets.search` page 1, shaped as picker rows.
    fn inventory_search(&self, query: &str, kind: &str) -> BResult<AssetCandidates> {
        let _ = (query, kind);
        Err(Failure::unavailable("integrated inventory"))
    }
    /// `integrated_assets.inventory` for one ticket, problem, or change.
    fn associated_inventory(
        &self,
        module: &str,
        target: &Value,
        kind: &str,
    ) -> BResult<Vec<Value>> {
        let _ = (module, target, kind);
        Err(Failure::unavailable("integrated inventory"))
    }
    /// `integrated_assets.build_plan` for one device or service id.
    fn inventory_association_plan(
        &self,
        module: &str,
        target: &Value,
        ident: &Value,
        kind: &str,
        remove: bool,
    ) -> BResult<ReviewPlan> {
        let _ = (module, target, ident, kind, remove);
        Err(Failure::unavailable("integrated inventory"))
    }
    /// `integrated_assets.send` once, with the reviewed hash.
    fn send_inventory_association(
        &self,
        plan: &ReviewPlan,
        module: &str,
        ident: &Value,
        kind: &str,
        remove: bool,
    ) -> BResult<Value> {
        let _ = (plan, module, ident, kind, remove);
        Err(Failure::unavailable("integrated inventory"))
    }

    /// `change_task_update.build_plan` or `change_task_delete.build_plan`.
    fn task_mutation_plan(
        &self,
        kind: &str,
        change_id: &Value,
        task_id: &Value,
        body: &Map<String, Value>,
    ) -> BResult<ReviewPlan> {
        let _ = (kind, change_id, task_id, body);
        Err(Failure::unavailable("task mutations"))
    }
    /// Send the reviewed task plan once.
    fn send_task_mutation(&self, plan: &ReviewPlan) -> BResult<Value> {
        let _ = plan;
        Err(Failure::unavailable("task mutations"))
    }
    /// `problem_workflows.child_plan(..., kind="note")`.
    fn problem_note_plan(&self, problem_id: &Value, body: &str) -> BResult<ReviewPlan> {
        let _ = (problem_id, body);
        Err(Failure::unavailable("problem notes"))
    }
    fn send_problem_note(&self, plan: &ReviewPlan) -> BResult<Value> {
        let _ = plan;
        Err(Failure::unavailable("problem notes"))
    }
    /// Edit-form change session with one status value.
    fn change_status_plan(&self, target: &Value, status: &Value) -> BResult<ReviewPlan> {
        let _ = (target, status);
        Err(Failure::unavailable("change status"))
    }
    fn send_change_status(&self, plan: &ReviewPlan) -> BResult<Value> {
        let _ = plan;
        Err(Failure::unavailable("change status"))
    }

    // ── create forms ────────────────────────────────────────
    /// `import fsv.formui` succeeded; without it `N` / `e` toast
    /// "form UI not available".
    fn forms_available(&self) -> bool {
        false
    }
    /// `_form_session` / `ticket_session` / `sr_session` wrapped in a
    /// `FormScreen` (runs on a worker).
    fn open_form(&self, req: &FormRequest) -> BResult<Box<dyn FormView>>;
    fn catalog(&self) -> BResult<(Vec<Value>, Vec<Value>)>;
    fn resolve_category(&self, categories: &[Value], text: &str) -> Vec<Value> {
        categories
            .iter()
            .filter(|c| c.get("id").map(fsv_py::py_str).as_deref() == Some(text))
            .cloned()
            .collect()
    }
    fn filter_catalog_items(&self, items: &[Value], category_ids: Option<&[Value]>) -> Vec<Value> {
        match category_ids {
            None => items.to_vec(),
            Some(ids) => items
                .iter()
                .filter(|i| i.get("category_id").is_some_and(|c| ids.contains(c)))
                .cloned()
                .collect(),
        }
    }
}

/// A backend that refuses every write: the reads go to `B`, each write
/// method fails with "read-only mode: …" before reaching it, and forms
/// are unavailable. `fsv_app::run_tui` wraps its backend in it when
/// read-only mode is on, so a missed UI check cannot write.
pub struct ReadOnly<B>(pub B);

impl<B: Backend> Backend for ReadOnly<B> {
    fn read_only_setting(&self) -> Value {
        self.0.read_only_setting()
    }
    fn default_where(&self, entity: &str) -> Vec<String> {
        self.0.default_where(entity)
    }
    fn load_filter_history(&self) -> Vec<String> {
        self.0.load_filter_history()
    }
    fn save_filter_history(&self, history: &[String]) {
        self.0.save_filter_history(history)
    }
    fn copy_to_clipboard(&self, text: &str) -> Result<(), String> {
        self.0.copy_to_clipboard(text)
    }
    fn open_url(&self, url: &str) -> bool {
        self.0.open_url(url)
    }
    fn item_url(&self, res: Res, id: &Value) -> String {
        self.0.item_url(res, id)
    }
    fn load_schema(&self, res: Res) -> BResult<Value> {
        self.0.load_schema(res)
    }
    fn current_user(&self) -> BResult<Value> {
        self.0.current_user()
    }
    fn user_tz(&self, me: &Value) -> Option<TimeZone> {
        self.0.user_tz(me)
    }
    fn load_views(&self, res: Res) -> BResult<Vec<View>> {
        self.0.load_views(res)
    }
    fn ticket_headers(&self) -> BResult<Vec<Value>> {
        self.0.ticket_headers()
    }
    fn view_settings(&self, res: Res, view: &View) -> BResult<Value> {
        self.0.view_settings(res, view)
    }
    fn build_query_hash(&self, res: Res, schema: &Value, filters: &[String]) -> BResult<String> {
        self.0.build_query_hash(res, schema, filters)
    }
    fn list_page(&self, req: &ListRequest) -> BResult<(Vec<Value>, Value)> {
        self.0.list_page(req)
    }
    fn search_items(
        &self,
        term: &str,
        entity: Option<&str>,
        sort: &str,
        page: i64,
    ) -> BResult<SearchResult> {
        self.0.search_items(term, entity, sort, page)
    }
    fn load_detail(&self, res: Res, id: &Value, schema: Option<&Value>) -> BResult<DetailPage> {
        self.0.load_detail(res, id, schema)
    }
    fn load_tab(&self, res: Res, record: &Value, tab: &str) -> BResult<TabData> {
        self.0.load_tab(res, record, tab)
    }
    fn completion_network(&self) -> bool {
        self.0.completion_network()
    }
    fn cached_group_names(&self) -> Vec<String> {
        self.0.cached_group_names()
    }
    fn remote_user_values(&self, kind: &str, prefix: &str) -> BResult<Vec<String>> {
        self.0.remote_user_values(kind, prefix)
    }
    fn lookup_choices(&self, link: &str, prefix: &str) -> BResult<Vec<Value>> {
        self.0.lookup_choices(link, prefix)
    }
    fn update_item(&self, _res: Res, _id: &Value, _body: &Map<String, Value>) -> BResult<Value> {
        Err(Failure::read_only("record updates"))
    }
    fn add_note(&self, _res: Res, _id: &Value, _body: &str, _public: bool) -> BResult<Value> {
        Err(Failure::read_only("notes"))
    }
    fn ticket_note_plan(&self, ticket_id: &Value, body: &str, public: bool) -> BResult<ReviewPlan> {
        self.0.ticket_note_plan(ticket_id, body, public)
    }
    fn send_ticket_note(
        &self,
        _plan: &ReviewPlan,
        _public_confirmed: bool,
        _read_only: bool,
    ) -> BResult<Value> {
        Err(Failure::read_only("ticket notes"))
    }
    fn change_task_plan(
        &self,
        change_id: &Value,
        title: &str,
        owner: Option<&str>,
        options: &[(String, String)],
    ) -> BResult<ReviewPlan> {
        self.0.change_task_plan(change_id, title, owner, options)
    }
    fn task_create_choices(&self, key: &str) -> BResult<Vec<String>> {
        self.0.task_create_choices(key)
    }
    fn send_change_task(&self, _plan: &ReviewPlan, _read_only: bool) -> BResult<Value> {
        Err(Failure::read_only("change tasks"))
    }
    fn search_change_tickets(&self, query: &str) -> BResult<Vec<Value>> {
        self.0.search_change_tickets(query)
    }
    fn download_choices(&self, item: &Item) -> BResult<(Vec<Value>, Vec<String>)> {
        self.0.download_choices(item)
    }
    fn download(&self, attachment: &Value, destination: &str) -> BResult<Value> {
        self.0.download(attachment, destination)
    }
    fn task_for_edit(&self, change_id: &Value, task_id: &Value) -> BResult<(Value, Vec<Value>)> {
        self.0.task_for_edit(change_id, task_id)
    }
    fn association_plan(
        &self,
        change_id: &Value,
        ticket_ids: &[Value],
        unlink: bool,
    ) -> BResult<ReviewPlan> {
        self.0.association_plan(change_id, ticket_ids, unlink)
    }
    fn send_association(&self, _plan: &ReviewPlan) -> BResult<Value> {
        Err(Failure::read_only("associations"))
    }
    fn approval_request_plan(
        &self,
        change_id: &Value,
        name: &str,
        message: &str,
        recipients: &[String],
        rule: &str,
    ) -> BResult<ReviewPlan> {
        self.0
            .approval_request_plan(change_id, name, message, recipients, rule)
    }
    fn approval_targets(&self, kind: &str, change_id: &Value) -> BResult<Vec<Value>> {
        self.0.approval_targets(kind, change_id)
    }
    fn approval_action_plan(
        &self,
        change_id: &Value,
        target_id: &Value,
        operation: &str,
        remark: Option<&str>,
    ) -> BResult<ReviewPlan> {
        self.0
            .approval_action_plan(change_id, target_id, operation, remark)
    }
    fn send_approval(&self, _plan: &ReviewPlan, _read_only: bool) -> BResult<Value> {
        Err(Failure::read_only("approvals"))
    }
    fn update_task(
        &self,
        _change_id: &Value,
        _task_id: &Value,
        _body: &Map<String, Value>,
    ) -> BResult<()> {
        Err(Failure::read_only("task updates"))
    }
    fn delete_task(&self, _change_id: &Value, _task_id: &Value) -> BResult<()> {
        Err(Failure::read_only("task deletes"))
    }
    fn ticket_property_fields(&self, ticket_id: &Value) -> BResult<Vec<PropertyField>> {
        self.0.ticket_property_fields(ticket_id)
    }
    fn ticket_property_plan(
        &self,
        ticket_id: &Value,
        name: &str,
        value: &Value,
    ) -> BResult<ReviewPlan> {
        self.0.ticket_property_plan(ticket_id, name, value)
    }
    fn send_ticket_property(&self, _plan: &ReviewPlan) -> BResult<Value> {
        Err(Failure::read_only("ticket property edits"))
    }
    fn known_error_plan(&self, problem_id: &Value) -> BResult<ReviewPlan> {
        self.0.known_error_plan(problem_id)
    }
    fn send_known_error(&self, _plan: &ReviewPlan) -> BResult<Value> {
        Err(Failure::read_only("known-error marks"))
    }
    fn planning_definitions(&self, module: &str) -> BResult<Vec<Value>> {
        self.0.planning_definitions(module)
    }
    fn planning_row(&self, module: &str, id: &Value, section: &str) -> BResult<Option<Value>> {
        self.0.planning_row(module, id, section)
    }
    fn planning_text_plan(
        &self,
        module: &str,
        id: &Value,
        section: &str,
        description: &str,
    ) -> BResult<ReviewPlan> {
        self.0.planning_text_plan(module, id, section, description)
    }
    fn send_planning_text(&self, _plan: &ReviewPlan) -> BResult<Value> {
        Err(Failure::read_only("planning text"))
    }
    fn ticket_conversation_plan(
        &self,
        ticket_id: &Value,
        body: &str,
        req: &ConversationRequest,
    ) -> BResult<ReviewPlan> {
        self.0.ticket_conversation_plan(ticket_id, body, req)
    }
    fn send_ticket_conversation(
        &self,
        _plan: &ReviewPlan,
        _public_confirmed: bool,
    ) -> BResult<Value> {
        Err(Failure::read_only("ticket conversations"))
    }
    fn asset_candidates(&self, req: &AssetCandidatesRequest) -> BResult<AssetCandidates> {
        self.0.asset_candidates(req)
    }
    fn inventory_search(&self, query: &str, kind: &str) -> BResult<AssetCandidates> {
        self.0.inventory_search(query, kind)
    }
    fn associated_inventory(
        &self,
        module: &str,
        target: &Value,
        kind: &str,
    ) -> BResult<Vec<Value>> {
        self.0.associated_inventory(module, target, kind)
    }
    fn inventory_association_plan(
        &self,
        _module: &str,
        _target: &Value,
        _ident: &Value,
        _kind: &str,
        _remove: bool,
    ) -> BResult<ReviewPlan> {
        Err(Failure::read_only("inventory associations"))
    }
    fn send_inventory_association(
        &self,
        _plan: &ReviewPlan,
        _module: &str,
        _ident: &Value,
        _kind: &str,
        _remove: bool,
    ) -> BResult<Value> {
        Err(Failure::read_only("inventory associations"))
    }
    fn forms_available(&self) -> bool {
        false
    }
    fn open_form(&self, _req: &FormRequest) -> BResult<Box<dyn FormView>> {
        Err(Failure::read_only("forms"))
    }
    fn catalog(&self) -> BResult<(Vec<Value>, Vec<Value>)> {
        self.0.catalog()
    }
    fn resolve_category(&self, categories: &[Value], text: &str) -> Vec<Value> {
        self.0.resolve_category(categories, text)
    }
    fn filter_catalog_items(&self, items: &[Value], category_ids: Option<&[Value]>) -> Vec<Value> {
        self.0.filter_catalog_items(items, category_ids)
    }
}

/// Stub bodies for test doubles: `backend_stubs!(list_page, load_detail)`
/// inside an `impl Backend` writes those methods as "unavailable" (or
/// empty) answers. Real backends implement every network method.
#[doc(hidden)]
#[macro_export]
macro_rules! backend_stubs {
    (@one load_schema) => {
        #[allow(unused_variables)]
        fn load_schema(&self, res: $crate::backend::Res) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("schema loading"))
        }
    };
    (@one current_user) => {
        #[allow(unused_variables)]
        fn current_user(&self) -> $crate::backend::BResult<::serde_json::Value> {
            Ok(::serde_json::Value::Null)
        }
    };
    (@one load_views) => {
        #[allow(unused_variables)]
        fn load_views(&self, res: $crate::backend::Res) -> $crate::backend::BResult<Vec<$crate::backend::View>> {
            Ok(Vec::new())
        }
    };
    (@one ticket_headers) => {
        #[allow(unused_variables)]
        fn ticket_headers(&self) -> $crate::backend::BResult<Vec<::serde_json::Value>> {
            Ok(Vec::new())
        }
    };
    (@one view_settings) => {
        #[allow(unused_variables)]
        fn view_settings(&self, res: $crate::backend::Res, view: &$crate::backend::View) -> $crate::backend::BResult<::serde_json::Value> {
            Ok(::serde_json::Value::Null)
        }
    };
    (@one build_query_hash) => {
        #[allow(unused_variables)]
        fn build_query_hash(&self, res: $crate::backend::Res, schema: &::serde_json::Value, filters: &[String]) -> $crate::backend::BResult<String> {
            Err($crate::backend::Failure::unavailable("filtering"))
        }
    };
    (@one list_page) => {
        #[allow(unused_variables)]
        fn list_page(&self, req: &$crate::backend::ListRequest) -> $crate::backend::BResult<(Vec<::serde_json::Value>, ::serde_json::Value)> {
            Err($crate::backend::Failure::unavailable("record lists"))
        }
    };
    (@one search_items) => {
        #[allow(unused_variables)]
        fn search_items(
            &self,
            term: &str,
            entity: Option<&str>,
            sort: &str,
            page: i64,
        ) -> $crate::backend::BResult<$crate::backend::SearchResult> {
            Err($crate::backend::Failure::unavailable("search"))
        }
    };
    (@one load_detail) => {
        #[allow(unused_variables)]
        fn load_detail(&self, res: $crate::backend::Res, id: &::serde_json::Value, schema: Option<&::serde_json::Value>) -> $crate::backend::BResult<$crate::backend::DetailPage> {
            Err($crate::backend::Failure::unavailable("record details"))
        }
    };
    (@one load_tab) => {
        #[allow(unused_variables)]
        fn load_tab(&self, res: $crate::backend::Res, record: &::serde_json::Value, tab: &str) -> $crate::backend::BResult<$crate::backend::TabData> {
            Err($crate::backend::Failure::unavailable("detail tabs"))
        }
    };
    (@one remote_user_values) => {
        #[allow(unused_variables)]
        fn remote_user_values(&self, kind: &str, prefix: &str) -> $crate::backend::BResult<Vec<String>> {
            Ok(Vec::new())
        }
    };
    (@one lookup_choices) => {
        #[allow(unused_variables)]
        fn lookup_choices(&self, link: &str, prefix: &str) -> $crate::backend::BResult<Vec<::serde_json::Value>> {
            Ok(Vec::new())
        }
    };
    (@one update_item) => {
        #[allow(unused_variables)]
        fn update_item(&self, res: $crate::backend::Res, id: &::serde_json::Value, body: &::serde_json::Map<String, ::serde_json::Value>) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("record updates"))
        }
    };
    (@one add_note) => {
        #[allow(unused_variables)]
        fn add_note(&self, res: $crate::backend::Res, id: &::serde_json::Value, body: &str, public: bool) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("notes"))
        }
    };
    (@one ticket_note_plan) => {
        #[allow(unused_variables)]
        fn ticket_note_plan(&self, ticket_id: &::serde_json::Value, body: &str, public: bool) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("ticket notes"))
        }
    };
    (@one send_ticket_note) => {
        #[allow(unused_variables)]
        fn send_ticket_note(
            &self,
            plan: &$crate::backend::ReviewPlan,
            public_confirmed: bool,
            read_only: bool,
        ) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("ticket notes"))
        }
    };
    (@one change_task_plan) => {
        #[allow(unused_variables)]
        fn change_task_plan(
            &self,
            change_id: &::serde_json::Value,
            title: &str,
            owner: Option<&str>,
            options: &[(String, String)],
        ) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("change tasks"))
        }
    };
    (@one send_change_task) => {
        #[allow(unused_variables)]
        fn send_change_task(&self, plan: &$crate::backend::ReviewPlan, read_only: bool) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("change tasks"))
        }
    };
    (@one search_change_tickets) => {
        #[allow(unused_variables)]
        fn search_change_tickets(&self, query: &str) -> $crate::backend::BResult<Vec<::serde_json::Value>> {
            Err($crate::backend::Failure::unavailable("ticket search"))
        }
    };
    (@one download_choices) => {
        #[allow(unused_variables)]
        fn download_choices(&self, item: &$crate::backend::Item) -> $crate::backend::BResult<(Vec<::serde_json::Value>, Vec<String>)> {
            Err($crate::backend::Failure::unavailable("attachment downloads"))
        }
    };
    (@one download) => {
        #[allow(unused_variables)]
        fn download(&self, attachment: &::serde_json::Value, destination: &str) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("attachment downloads"))
        }
    };
    (@one task_for_edit) => {
        #[allow(unused_variables)]
        fn task_for_edit(&self, change_id: &::serde_json::Value, task_id: &::serde_json::Value) -> $crate::backend::BResult<(::serde_json::Value, Vec<::serde_json::Value>)> {
            Err($crate::backend::Failure::unavailable("task editing"))
        }
    };
    (@one association_plan) => {
        #[allow(unused_variables)]
        fn association_plan(
            &self,
            change_id: &::serde_json::Value,
            ticket_ids: &[::serde_json::Value],
            unlink: bool,
        ) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("associations"))
        }
    };
    (@one send_association) => {
        #[allow(unused_variables)]
        fn send_association(&self, plan: &$crate::backend::ReviewPlan) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("associations"))
        }
    };
    (@one approval_request_plan) => {
        #[allow(unused_variables)]
        fn approval_request_plan(
            &self,
            change_id: &::serde_json::Value,
            name: &str,
            message: &str,
            recipients: &[String],
            rule: &str,
        ) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("approval requests"))
        }
    };
    (@one approval_targets) => {
        #[allow(unused_variables)]
        fn approval_targets(&self, kind: &str, change_id: &::serde_json::Value) -> $crate::backend::BResult<Vec<::serde_json::Value>> {
            Err($crate::backend::Failure::unavailable("approval inventory"))
        }
    };
    (@one approval_action_plan) => {
        #[allow(unused_variables)]
        fn approval_action_plan(
            &self,
            change_id: &::serde_json::Value,
            target_id: &::serde_json::Value,
            operation: &str,
            remark: Option<&str>,
        ) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("approval actions"))
        }
    };
    (@one send_approval) => {
        #[allow(unused_variables)]
        fn send_approval(&self, plan: &$crate::backend::ReviewPlan, read_only: bool) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("approval actions"))
        }
    };
    (@one update_task) => {
        #[allow(unused_variables)]
        fn update_task(
            &self,
            change_id: &::serde_json::Value,
            task_id: &::serde_json::Value,
            body: &::serde_json::Map<String, ::serde_json::Value>,
        ) -> $crate::backend::BResult<()> {
            Err($crate::backend::Failure::unavailable("task updates"))
        }
    };
    (@one delete_task) => {
        #[allow(unused_variables)]
        fn delete_task(&self, change_id: &::serde_json::Value, task_id: &::serde_json::Value) -> $crate::backend::BResult<()> {
            Err($crate::backend::Failure::unavailable("task deletes"))
        }
    };
    (@one ticket_property_fields) => {
        #[allow(unused_variables)]
        fn ticket_property_fields(&self, ticket_id: &::serde_json::Value) -> $crate::backend::BResult<Vec<$crate::backend::PropertyField>> {
            Err($crate::backend::Failure::unavailable("ticket property edits"))
        }
    };
    (@one ticket_property_plan) => {
        #[allow(unused_variables)]
        fn ticket_property_plan(
            &self,
            ticket_id: &::serde_json::Value,
            name: &str,
            value: &::serde_json::Value,
        ) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("ticket property edits"))
        }
    };
    (@one send_ticket_property) => {
        #[allow(unused_variables)]
        fn send_ticket_property(&self, plan: &$crate::backend::ReviewPlan) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("ticket property edits"))
        }
    };
    (@one known_error_plan) => {
        #[allow(unused_variables)]
        fn known_error_plan(&self, problem_id: &::serde_json::Value) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("known-error marks"))
        }
    };
    (@one send_known_error) => {
        #[allow(unused_variables)]
        fn send_known_error(&self, plan: &$crate::backend::ReviewPlan) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("known-error marks"))
        }
    };
    (@one planning_definitions) => {
        #[allow(unused_variables)]
        fn planning_definitions(&self, module: &str) -> $crate::backend::BResult<Vec<::serde_json::Value>> {
            Err($crate::backend::Failure::unavailable("planning text"))
        }
    };
    (@one planning_row) => {
        #[allow(unused_variables)]
        fn planning_row(&self, module: &str, id: &::serde_json::Value, section: &str) -> $crate::backend::BResult<Option<::serde_json::Value>> {
            Err($crate::backend::Failure::unavailable("planning text"))
        }
    };
    (@one planning_text_plan) => {
        #[allow(unused_variables)]
        fn planning_text_plan(
            &self,
            module: &str,
            id: &::serde_json::Value,
            section: &str,
            description: &str,
        ) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("planning text"))
        }
    };
    (@one send_planning_text) => {
        #[allow(unused_variables)]
        fn send_planning_text(&self, plan: &$crate::backend::ReviewPlan) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("planning text"))
        }
    };
    (@one ticket_conversation_plan) => {
        #[allow(unused_variables)]
        fn ticket_conversation_plan(
            &self,
            ticket_id: &::serde_json::Value,
            body: &str,
            req: &$crate::backend::ConversationRequest,
        ) -> $crate::backend::BResult<$crate::backend::ReviewPlan> {
            Err($crate::backend::Failure::unavailable("ticket conversations"))
        }
    };
    (@one send_ticket_conversation) => {
        #[allow(unused_variables)]
        fn send_ticket_conversation(
            &self,
            plan: &$crate::backend::ReviewPlan,
            public_confirmed: bool,
        ) -> $crate::backend::BResult<::serde_json::Value> {
            Err($crate::backend::Failure::unavailable("ticket conversations"))
        }
    };
    (@one asset_candidates) => {
        #[allow(unused_variables)]
        fn asset_candidates(&self, req: &$crate::backend::AssetCandidatesRequest) -> $crate::backend::BResult<$crate::backend::AssetCandidates> {
            Err($crate::backend::Failure::unavailable("asset candidates"))
        }
    };
    (@one open_form) => {
        #[allow(unused_variables)]
        fn open_form(&self, req: &$crate::backend::FormRequest) -> $crate::backend::BResult<Box<dyn $crate::backend::FormView>> {
            Err($crate::backend::Failure::unavailable("the form UI"))
        }
    };
    (@one catalog) => {
        #[allow(unused_variables)]
        fn catalog(&self) -> $crate::backend::BResult<(Vec<::serde_json::Value>, Vec<::serde_json::Value>)> {
            Err($crate::backend::Failure::unavailable("service catalog"))
        }
    };
    ($($name:ident),* $(,)?) => {
        $( $crate::backend_stubs!(@one $name); )*
    };
}

/// `pages.py` renderers (markup strings) the TUI shows for a loaded page.
pub trait Pages: Send + Sync {
    fn lifecycle_markup(&self, page: &DetailPage, context: usize) -> String;
    fn reporter_markup(&self, page: &DetailPage) -> String;
    fn main_lines(&self, page: &DetailPage) -> Vec<String>;
    fn notes_lines(&self, page: &DetailPage) -> Vec<String>;
    fn side_lines(&self, page: &DetailPage) -> Vec<String>;
    fn tab_lines(
        &self,
        data: &TabData,
        tz: Option<&TimeZone>,
        schema: Option<&Value>,
    ) -> Vec<String>;

    /// `pages.tab_bar`.
    fn tab_bar(&self, page: &DetailPage, active: &str, numbered: bool) -> String {
        page.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut label = match t.count {
                    Some(c) if c != 0 => format!("{} ({c})", t.label),
                    _ => t.label.clone(),
                };
                if numbered {
                    label = format!("{}:{label}", i + 1);
                }
                if t.name == active {
                    format!("[bold reverse] {} [/]", escape(&label))
                } else {
                    format!("[dim]{}[/]", escape(&label))
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    }
}

/// Minimal renderer used until fsv-views ports `pages.py`: description,
/// notes, and the tab's missing note or row count.
pub struct BasicPages;

impl Pages for BasicPages {
    fn lifecycle_markup(&self, _page: &DetailPage, _context: usize) -> String {
        String::new()
    }

    fn reporter_markup(&self, _page: &DetailPage) -> String {
        String::new()
    }

    fn main_lines(&self, page: &DetailPage) -> Vec<String> {
        let mut out = vec!["[bold]Description[/]".to_string()];
        let desc = page
            .record
            .get("description_text")
            .or_else(|| page.record.get("description"))
            .map(crate::format::strip_html)
            .unwrap_or_default();
        if desc.is_empty() {
            out.push("  [dim]No description[/]".into());
        } else {
            out.extend(desc.lines().map(|l| {
                if l.is_empty() {
                    String::new()
                } else {
                    format!("  {}", escape(l))
                }
            }));
        }
        out
    }

    fn notes_lines(&self, page: &DetailPage) -> Vec<String> {
        page.extra
            .get("notes")
            .and_then(Value::as_array)
            .map(|n| {
                n.iter()
                    .map(|n| format!("[dim]· {}[/]", escape(&fsv_py::py_str(n))))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn side_lines(&self, page: &DetailPage) -> Vec<String> {
        vec![format!("[cyan]Status[/]  {}", escape(&page.badge))]
    }

    fn tab_lines(
        &self,
        data: &TabData,
        _tz: Option<&TimeZone>,
        _schema: Option<&Value>,
    ) -> Vec<String> {
        if let Some(m) = &data.missing {
            return vec![format!("  [dim]{}[/]", escape(&m.note()))];
        }
        let n = data.data.as_array().map_or(0, Vec::len);
        let mut out: Vec<String> = data
            .notes
            .iter()
            .map(|n| format!("  [dim]{}[/]", escape(n)))
            .collect();
        out.push(if n == 0 {
            format!("  [dim]no {}[/]", escape(&data.label.to_lowercase()))
        } else {
            format!("  {n} {}", escape(&data.label))
        });
        out
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;
    use fsv_core::resource::{CHANGES, TICKETS};
    use serde_json::json;
    use std::sync::Mutex;

    /// Records reads; its writes would succeed, so a refusal can only come
    /// from `ReadOnly`.
    #[derive(Default)]
    struct Inner {
        log: Mutex<Vec<String>>,
    }

    impl Backend for Inner {
        fn load_detail(&self, res: Res, id: &Value, _: Option<&Value>) -> BResult<DetailPage> {
            self.log
                .lock()
                .unwrap()
                .push(format!("detail {} {id}", res.name));
            Ok(DetailPage {
                res,
                record: json!({"id": id}),
                tabs: Vec::new(),
                badge: String::new(),
                due: String::new(),
                tz: None,
                schema: None,
                extra: Value::Null,
            })
        }
        fn add_note(&self, _: Res, _: &Value, _: &str, _: bool) -> BResult<Value> {
            self.log.lock().unwrap().push("note".into());
            Ok(json!({}))
        }
        fn send_ticket_note(&self, _: &ReviewPlan, _: bool, _: bool) -> BResult<Value> {
            self.log.lock().unwrap().push("ticket-note".into());
            Ok(json!({}))
        }
        fn send_change_task(&self, _: &ReviewPlan, _: bool) -> BResult<Value> {
            self.log.lock().unwrap().push("task".into());
            Ok(json!({}))
        }
        fn send_approval(&self, _: &ReviewPlan, _: bool) -> BResult<Value> {
            self.log.lock().unwrap().push("approval".into());
            Ok(json!({}))
        }
        fn forms_available(&self) -> bool {
            true
        }
        crate::backend_stubs!(
            load_schema,
            current_user,
            load_views,
            ticket_headers,
            view_settings,
            build_query_hash,
            list_page,
            search_items,
            load_tab,
            remote_user_values,
            lookup_choices,
            update_item,
            ticket_note_plan,
            change_task_plan,
            search_change_tickets,
            download_choices,
            download,
            task_for_edit,
            association_plan,
            send_association,
            approval_request_plan,
            approval_targets,
            approval_action_plan,
            update_task,
            delete_task,
            ticket_property_fields,
            ticket_property_plan,
            send_ticket_property,
            known_error_plan,
            send_known_error,
            planning_definitions,
            planning_row,
            planning_text_plan,
            send_planning_text,
            ticket_conversation_plan,
            send_ticket_conversation,
            asset_candidates,
            open_form,
            catalog
        );
    }

    #[test]
    fn read_only_refuses_writes_and_passes_reads() {
        let ro = ReadOnly(Inner::default());
        let plan = ReviewPlan {
            review: json!({}),
            hash: "h".into(),
            public: false,
            target_id: json!(1),
            raw: Value::Null,
        };
        let refused = [
            ro.add_note(&CHANGES, &json!(1), "x", false).unwrap_err(),
            ro.send_ticket_note(&plan, true, false).unwrap_err(),
            ro.send_change_task(&plan, false).unwrap_err(),
            ro.send_approval(&plan, false).unwrap_err(),
            ro.update_item(&CHANGES, &json!(1), &Map::new())
                .unwrap_err(),
            ro.open_form(&FormRequest {
                mode: "new".into(),
                item: None,
                record: None,
            })
            .err()
            .unwrap(),
        ];
        for e in refused {
            assert!(
                e.to_string()
                    .starts_with("read-only mode: writes are disabled ("),
                "{e}"
            );
        }
        assert!(!ro.forms_available());
        assert!(ro.0.log.lock().unwrap().is_empty());
        let page = ro.load_detail(&TICKETS, &json!(7), None).unwrap();
        assert_eq!(page.record, json!({"id": 7}));
        assert_eq!(*ro.0.log.lock().unwrap(), ["detail tickets 7"]);
    }
}
