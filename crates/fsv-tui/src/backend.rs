//! Service seams the TUI calls (`fsv.service`, `fsv.views` loaders,
//! `fsv.pages` renderers, writers, config).
//!
//! fsv-api has no loaders or writers yet and fsv-views / fsv-formui are
//! stubs, so the TUI binds to these traits. Every method has a default that
//! reports the feature as unavailable; workers turn that into a toast or a
//! failure screen, never a panic. A real implementation lives with the
//! binary once the service crates land; tests substitute recorders.

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
    fn load_schema(&self, res: Res) -> BResult<Value> {
        Err(Failure::unavailable("schema loading"))
    }
    fn current_user(&self) -> BResult<Value> {
        Ok(Value::Null)
    }
    fn user_tz(&self, me: &Value) -> Option<TimeZone> {
        None
    }
    fn load_views(&self, res: Res) -> BResult<Vec<View>> {
        Ok(Vec::new())
    }
    fn ticket_headers(&self) -> BResult<Vec<Value>> {
        Ok(Vec::new())
    }
    fn view_settings(&self, res: Res, view: &View) -> BResult<Value> {
        Ok(Value::Null)
    }
    /// `query.build_query_hash_from_schema`; a `Value` error is a filter error.
    fn build_query_hash(&self, res: Res, schema: &Value, filters: &[String]) -> BResult<String> {
        Err(Failure::unavailable("filtering"))
    }
    /// `service.list_page` → `(items, raw response)`.
    fn list_page(&self, req: &ListRequest) -> BResult<(Vec<Value>, Value)> {
        Err(Failure::unavailable("record lists"))
    }
    fn search_items(
        &self,
        term: &str,
        entity: Option<&str>,
        sort: &str,
        page: i64,
    ) -> BResult<SearchResult> {
        Err(Failure::unavailable("search"))
    }
    fn load_detail(&self, res: Res, id: &Value, schema: Option<&Value>) -> BResult<DetailPage> {
        Err(Failure::unavailable("record details"))
    }
    fn load_tab(&self, res: Res, record: &Value, tab: &str) -> BResult<TabData> {
        Err(Failure::unavailable("detail tabs"))
    }

    // ── filter completion ───────────────────────────────────
    fn completion_network(&self) -> bool {
        false
    }
    fn cached_group_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// `completion._remote_user_values(kind, prefix)` values.
    fn remote_user_values(&self, kind: &str, prefix: &str) -> BResult<Vec<String>> {
        Ok(Vec::new())
    }
    fn lookup_choices(&self, link: &str, prefix: &str) -> BResult<Vec<Value>> {
        Ok(Vec::new())
    }

    // ── writes ──────────────────────────────────────────────
    fn update_item(&self, res: Res, id: &Value, body: &Map<String, Value>) -> BResult<Value> {
        Err(Failure::unavailable("record updates"))
    }
    fn add_note(&self, res: Res, id: &Value, body: &str, public: bool) -> BResult<Value> {
        Err(Failure::unavailable("notes"))
    }
    fn add_reply(&self, res: Res, id: &Value, body: &str) -> BResult<Value> {
        Err(Failure::unavailable("replies"))
    }
    fn ticket_note_plan(&self, ticket_id: &Value, body: &str, public: bool) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("ticket notes"))
    }
    fn send_ticket_note(
        &self,
        plan: &ReviewPlan,
        public_confirmed: bool,
        read_only: bool,
    ) -> BResult<Value> {
        Err(Failure::unavailable("ticket notes"))
    }
    /// `change_tasks.build_plan` after resolving owner, group and status.
    fn change_task_plan(
        &self,
        change_id: &Value,
        title: &str,
        owner: Option<&str>,
        options: &[(String, String)],
    ) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("change tasks"))
    }
    fn send_change_task(&self, plan: &ReviewPlan, read_only: bool) -> BResult<Value> {
        Err(Failure::unavailable("change tasks"))
    }
    fn search_change_tickets(&self, query: &str) -> BResult<Vec<Value>> {
        Err(Failure::unavailable("ticket search"))
    }
    /// `actions.download_choices` → `(rows, warnings)`.
    fn download_choices(&self, item: &Item) -> BResult<(Vec<Value>, Vec<String>)> {
        Err(Failure::unavailable("attachment downloads"))
    }
    fn download(&self, attachment: &Value, destination: &str) -> BResult<Value> {
        Err(Failure::unavailable("attachment downloads"))
    }
    /// `create.get_task_for_edit` plus `task_status.fields_from`.
    fn task_for_edit(&self, change_id: &Value, task_id: &Value) -> BResult<(Value, Vec<Value>)> {
        Err(Failure::unavailable("task editing"))
    }
    fn association_plan(
        &self,
        change_id: &Value,
        ticket_ids: &[Value],
        unlink: bool,
    ) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("associations"))
    }
    fn send_association(&self, plan: &ReviewPlan) -> BResult<Value> {
        Err(Failure::unavailable("associations"))
    }
    /// `change_approvals.resolve_approvers` + `build_plan` (a request plan).
    fn approval_request_plan(
        &self,
        change_id: &Value,
        name: &str,
        message: &str,
        recipients: &[String],
        rule: &str,
    ) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("approval requests"))
    }
    /// `approval-respond` → approval records; otherwise approval groups.
    fn approval_targets(&self, kind: &str, change_id: &Value) -> BResult<Vec<Value>> {
        Err(Failure::unavailable("approval inventory"))
    }
    /// `change_approval_actions.build_plan` (approve / reject / cancel).
    fn approval_action_plan(
        &self,
        change_id: &Value,
        target_id: &Value,
        operation: &str,
        remark: Option<&str>,
    ) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("approval actions"))
    }
    /// Sends a request or action plan once; the result carries `verified`.
    fn send_approval(&self, plan: &ReviewPlan, read_only: bool) -> BResult<Value> {
        Err(Failure::unavailable("approval actions"))
    }
    fn update_task(
        &self,
        change_id: &Value,
        task_id: &Value,
        body: &Map<String, Value>,
    ) -> BResult<()> {
        Err(Failure::unavailable("task updates"))
    }
    fn delete_task(&self, change_id: &Value, task_id: &Value) -> BResult<()> {
        Err(Failure::unavailable("task deletes"))
    }

    // ── reviewed single-purpose writes ──────────────────────
    /// `load_meta("ticket", "edit", fresh=True, module_id=ticket_id)` filtered
    /// to the fields `e` offers on a ticket.
    fn ticket_property_fields(&self, ticket_id: &Value) -> BResult<Vec<PropertyField>> {
        Err(Failure::unavailable("ticket property edits"))
    }
    /// `ticket_properties.build_plan(ticket_id, answers={name: value})`.
    fn ticket_property_plan(
        &self,
        ticket_id: &Value,
        name: &str,
        value: &Value,
    ) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("ticket property edits"))
    }
    /// `ticket_properties.send(plan, reviewed_hash=plan.hash)`.
    fn send_ticket_property(&self, plan: &ReviewPlan) -> BResult<Value> {
        Err(Failure::unavailable("ticket property edits"))
    }
    /// `problem_known_error.build_plan(problem_id)`.
    fn known_error_plan(&self, problem_id: &Value) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("known-error marks"))
    }
    /// `problem_known_error.send(plan, reviewed_hash=plan.hash)`.
    fn send_known_error(&self, plan: &ReviewPlan) -> BResult<Value> {
        Err(Failure::unavailable("known-error marks"))
    }
    /// `planning_text.definitions(module)`; `module` is `changes` or `problems`.
    fn planning_definitions(&self, module: &str) -> BResult<Vec<Value>> {
        Err(Failure::unavailable("planning text"))
    }
    /// `match_planning_row({"name": section}, planning_text.inventory(module, id))`.
    fn planning_row(&self, module: &str, id: &Value, section: &str) -> BResult<Option<Value>> {
        Err(Failure::unavailable("planning text"))
    }
    /// `planning_text.build_plan(module, id, section, description)`.
    fn planning_text_plan(
        &self,
        module: &str,
        id: &Value,
        section: &str,
        description: &str,
    ) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("planning text"))
    }
    /// `planning_text.send(plan, reviewed_hash=plan.hash)`.
    fn send_planning_text(&self, plan: &ReviewPlan) -> BResult<Value> {
        Err(Failure::unavailable("planning text"))
    }
    /// `ticket_conversation.recipients` for cc / notify / bcc, then
    /// `build_plan`. `raw` must be the plan's payload: the TUI reads
    /// `private` from it to decide on the public-consent step.
    fn ticket_conversation_plan(
        &self,
        ticket_id: &Value,
        body: &str,
        req: &ConversationRequest,
    ) -> BResult<ReviewPlan> {
        Err(Failure::unavailable("ticket conversations"))
    }
    /// `ticket_conversation.send(plan, reviewed_hash=plan.hash, public_confirmed=...)`.
    fn send_ticket_conversation(
        &self,
        plan: &ReviewPlan,
        public_confirmed: bool,
    ) -> BResult<Value> {
        Err(Failure::unavailable("ticket conversations"))
    }

    // ── create forms ────────────────────────────────────────
    /// `import fsv.formui` succeeded; without it `N` / `e` toast
    /// "form UI not available".
    fn forms_available(&self) -> bool {
        false
    }
    /// `_form_session` / `ticket_session` / `sr_session` wrapped in a
    /// `FormScreen` (runs on a worker).
    fn open_form(&self, req: &FormRequest) -> BResult<Box<dyn FormView>> {
        Err(Failure::unavailable("the form UI"))
    }
    fn catalog(&self) -> BResult<(Vec<Value>, Vec<Value>)> {
        Err(Failure::unavailable("service catalog"))
    }
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
