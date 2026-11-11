//! `FsvApp`: state, workers, timers, the key handler and the write flows.
//!
//! Workers run on threads (or inline in tests) and send a [`Msg`] back; the
//! staleness guards are Python's (list epoch + entity, search seq, pending
//! detail / sub keys, completion key). A worker error toasts or opens the
//! failure screen; it never ends the app.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use fsv_core::resource::{CHANGES, PROBLEMS, TICKETS};
use fsv_core::term::escape;
use fsv_py::Dumps;
use jiff::tz::TimeZone;
use ratatui::style::{Color, Modifier, Style};
use serde_json::{Map, Value, json};

use crate::actions;
use crate::backend::{
    Backend, DetailPage, Failure, FormExit, FormRequest, FormSend, FormView, Item, ListRequest,
    Missing, Pages, Res, ReviewPlan, SearchResult, TASK_NOTIFY_BEFORE, TASK_STATUS_LABELS, TabData,
    View,
};
use crate::filter::{self, FilterState, Lookup, LookupRequest, Source};
use crate::format::{self, clip, cut, resolve_priority, resolve_status};
use crate::input::{InputEvent, LineInput};
use crate::keys;
use crate::modals::{Modal, ModalResult, NoteModal, Outcome, PickerModal, StatusModal};
use crate::site::{self, Column};

pub const ENTITY_TABS: [&str; 3] = ["tickets", "problems", "changes"];
const ALL_RESOURCES: [Res; 3] = [&TICKETS, &CHANGES, &PROBLEMS];
const DETAIL_DEBOUNCE: Duration = Duration::from_millis(150);
const ENTITY_DEBOUNCE: Duration = Duration::from_millis(200);
const LOOKUP_DEBOUNCE: Duration = Duration::from_millis(300);
const TOAST_SECS: u64 = 5;

/// `_resource_for_entity`, with the callers' `or TICKETS` fallback.
pub fn resource_for(name: &str) -> Res {
    match name {
        "changes" => &CHANGES,
        "problems" => &PROBLEMS,
        _ => &TICKETS,
    }
}

pub fn detail_tab_labels(res: &str) -> &'static [&'static str] {
    match res {
        "changes" => &[
            "details",
            "notes",
            "tasks",
            "assets",
            "associations",
            "approvals",
            "activity",
        ],
        "tickets" => &[
            "details",
            "conversations",
            "tasks",
            "assets",
            "associations",
            "approvals",
            "activity",
            "resolution",
        ],
        "problems" => &[
            "details",
            "notes",
            "tasks",
            "assets",
            "associations",
            "activity",
        ],
        _ => &["details"],
    }
}

fn entity_key(key: &str) -> Option<&'static str> {
    match key {
        "t" => Some("tickets"),
        "p" => Some("problems"),
        "c" => Some("changes"),
        _ => None,
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// Python f-string of `d.get(k, default)`.
fn fget(v: &Value, key: &str, default: &str) -> String {
    match v.get(key) {
        None => default.to_string(),
        Some(x) => fsv_py::py_str(x),
    }
}

fn first_of(v: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter()
        .find_map(|k| v.get(*k).filter(|x| fsv_py::truthy(x)).cloned())
}

fn json_line(v: &Value) -> String {
    Dumps::default().ensure_ascii(false).dump(v)
}

fn json_pretty(v: &Value) -> String {
    Dumps::default().ensure_ascii(false).indent(2).dump(v)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    List,
    Detail,
}

impl Pane {
    pub fn as_str(self) -> &'static str {
        match self {
            Pane::List => "list",
            Pane::Detail => "detail",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    Detail,
    Filter,
    Search,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Information,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub title: Option<String>,
    pub message: String,
    pub severity: Severity,
    pub expires: Duration,
}

#[derive(Clone, Debug)]
pub struct TableCell {
    pub text: String,
    pub style: Style,
}

impl TableCell {
    fn plain(t: impl Into<String>) -> TableCell {
        TableCell {
            text: t.into(),
            style: Style::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub key: String,
    /// Subject cells carry a dim-cyan `#ID ` prefix of this many chars.
    pub cells: Vec<(TableCell, usize)>,
}

#[derive(Clone, Debug)]
pub enum Content {
    Markup(String),
    Plain(String, Style),
}

#[derive(Clone, Debug)]
pub struct DetailView {
    pub bar: Vec<(String, Style)>,
    pub state_flow: String,
    pub reporter: String,
    pub tabs: String,
    pub content: Content,
    pub side: Option<Vec<String>>,
    pub scroll: u16,
}

impl Default for DetailView {
    fn default() -> Self {
        DetailView {
            bar: Vec::new(),
            state_flow: String::new(),
            reporter: String::new(),
            tabs: String::new(),
            content: Content::Markup(String::new()),
            side: None,
            scroll: 0,
        }
    }
}

/// What a confirmed / picked modal continues with (Python's callbacks).
#[derive(Clone, Debug)]
pub enum Cont {
    None,
    ViewPicked {
        res: Res,
        views: Vec<View>,
    },
    StatusPicked {
        item: Item,
    },
    StatusConfirmed {
        item: Item,
        id: Value,
        label: String,
    },
    NoteComposed {
        item: Item,
        reply: bool,
    },
    NoteConfirmed {
        item: Item,
        body: String,
        public: bool,
        reply: bool,
    },
    TicketNoteReviewed {
        plan: ReviewPlan,
    },
    TicketNotePublic {
        plan: ReviewPlan,
    },
    ActionMenu {
        item: Item,
    },
    LinkQuery {
        item: Item,
    },
    TaskTitle {
        item: Item,
    },
    TaskOwnerMode {
        item: Item,
        title: String,
    },
    TaskOwner {
        item: Item,
        title: String,
    },
    TaskOptions {
        item: Item,
        title: String,
        owner: Option<String>,
        values: Vec<(String, String)>,
    },
    TaskOptionValue {
        item: Item,
        title: String,
        owner: Option<String>,
        values: Vec<(String, String)>,
        key: String,
    },
    TaskReviewed {
        item: Item,
        plan: ReviewPlan,
    },
    ActionTarget {
        kind: String,
        item: Item,
    },
    TaskField {
        item: Item,
        target: Value,
        current: Value,
        fields: Vec<Value>,
    },
    TaskFieldValue {
        item: Item,
        target: Value,
        current: Value,
        fields: Vec<Value>,
        field: String,
    },
    DownloadDest {
        item: Item,
        target: Value,
    },
    DownloadCollision {
        item: Item,
        target: Value,
        directory: String,
        name: String,
    },
    DownloadRename {
        item: Item,
        target: Value,
        directory: String,
    },
    ApprovalName {
        item: Item,
    },
    ApprovalMessage {
        item: Item,
        name: String,
    },
    ApprovalRecipients {
        item: Item,
        name: String,
        message: String,
    },
    ApprovalRule {
        item: Item,
        name: String,
        message: String,
        recipients: String,
    },
    ApprovalTarget {
        kind: String,
        item: Item,
    },
    ApprovalOp {
        item: Item,
        row: Value,
    },
    ApprovalRemark {
        item: Item,
        row: Value,
        op: String,
    },
    ApprovalReviewed {
        item: Item,
        plan: ReviewPlan,
    },
    ActionConfirmed {
        kind: String,
        item: Item,
        target: Value,
        body: Map<String, Value>,
        plan: Option<ReviewPlan>,
    },
    /// Ticket property, known-error, planning text and ticket conversation flows.
    Reviewed(reviewed::Step),
    /// Read-only asset candidate search (`i`).
    Assets(assets::Step),
    CreateChoice,
    CatalogCategory {
        items: Vec<Value>,
        categories: Vec<Value>,
    },
    CatalogItem,
}

/// A form a worker opened (`_push_form(screen_cls, session, mode, item)`).
pub struct OpenedForm {
    pub view: Box<dyn FormView>,
    pub mode: String,
    pub item: Option<Item>,
}

impl std::fmt::Debug for OpenedForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedForm")
            .field("mode", &self.mode)
            .field("item", &self.item)
            .finish_non_exhaustive()
    }
}

/// The form on screen; its clock starts when it opens.
pub(crate) struct ActiveForm {
    pub(crate) view: RefCell<Box<dyn FormView>>,
    mode: String,
    item: Option<Item>,
    start: Duration,
}

/// UI-thread follow-ups a worker asks for (`call_from_thread`).
#[derive(Debug)]
pub enum Ui {
    Failure(String),
    Toast {
        message: String,
        severity: Severity,
        title: Option<String>,
        secs: u64,
    },
    AfterWrite(Res, Item, String),
    TicketNoteVerified {
        ticket_id: Value,
        note_id: Value,
    },
    Catalog(Vec<Value>, Vec<Value>),
    TaskPlan(Item, ReviewPlan),
    Targets {
        kind: String,
        item: Item,
        rows: Vec<Value>,
        warnings: Vec<String>,
    },
    TaskEditor {
        item: Item,
        target: Value,
        current: Value,
        fields: Vec<Value>,
    },
    ApprovalReview {
        item: Item,
        plan: ReviewPlan,
        before: Option<Value>,
    },
    ApprovalTargets {
        kind: String,
        item: Item,
        rows: Vec<Value>,
    },
    ApprovalResult {
        item: Item,
        result: Value,
    },
    Form(OpenedForm),
    ReloadList,
    /// `call_from_thread(push_screen, modal, callback)`.
    Push(Box<Modal>, Cont),
    /// `OutcomeScreen(title, message)`.
    Outcome {
        title: String,
        message: String,
    },
}

#[derive(Debug, Default)]
pub struct Loaded {
    pub schemas: Vec<(&'static str, Value)>,
    pub me: Option<(Value, Option<TimeZone>)>,
    pub views: Option<(&'static str, Vec<View>)>,
    pub headers: Option<Vec<Value>>,
    pub sort: Option<String>,
}

type ListOutcome = Result<(Vec<Value>, Option<i64>, bool), String>;

#[derive(Debug)]
pub enum Msg {
    List {
        epoch: u64,
        entity: &'static str,
        page: i64,
        append: bool,
        loaded: Loaded,
        outcome: ListOutcome,
    },
    Detail {
        item: Item,
        key: String,
        outcome: Result<DetailPage, String>,
    },
    Sub {
        tab: String,
        key: String,
        res: Res,
        data: TabData,
    },
    Search {
        seq: u64,
        append: bool,
        schemas: Vec<(&'static str, Value)>,
        outcome: Result<SearchResult, String>,
    },
    Lookup {
        key: String,
        values: Vec<String>,
    },
    Ui(Vec<Ui>),
}

type Job = Box<dyn FnOnce(&dyn Backend) -> Msg + Send>;

enum Exec {
    Inline(VecDeque<Job>),
    Threads(mpsc::Sender<Msg>, mpsc::Receiver<Msg>),
}

pub enum Clock {
    Real(Instant),
    Manual(Duration),
}

impl Clock {
    fn now(&self) -> Duration {
        match self {
            Clock::Real(start) => start.elapsed(),
            Clock::Manual(d) => *d,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Detail,
    Entity,
    Lookup,
}

enum TimerKind {
    Detail(Item, String),
    Entity,
    Lookup(LookupRequest),
}

struct Timer {
    due: Duration,
    slot: Slot,
    kind: TimerKind,
}

type SearchCacheEntry = (Vec<Item>, Vec<(String, i64)>, i64, bool);

pub struct App {
    backend: Arc<dyn Backend>,
    pages: Arc<dyn Pages>,
    exec: Exec,
    clock: Clock,
    timers: Vec<Timer>,
    pub read_only: bool,
    pub entity: &'static str,
    pub detail_tab_idx: usize,
    pub items: Vec<Item>,
    pub total: i64,
    pub schemas: HashMap<&'static str, Value>,
    pub selected: Option<Item>,
    detail_cache: HashMap<String, Value>,
    details: HashMap<String, DetailPage>,
    sub_cache: HashMap<String, TabData>,
    g_pending: Option<Pane>,
    pending_detail_key: Option<String>,
    pending_sub_key: Option<String>,
    pub list_fr: u16,
    pub detail_fr: u16,
    pub page: i64,
    pub list_epoch: u64,
    pub write_failure: Option<String>,
    pub per_page: i64,
    pub has_more: bool,
    pub loading_more: bool,
    pub active_filters: Vec<String>,
    pub filter_error: Option<String>,
    pub filter: FilterState,
    pub search_input: LineInput,
    pub search_input_visible: bool,
    pub search_mode: bool,
    pub search_term: String,
    pub search_totals: Vec<(String, i64)>,
    pub search_entity: Option<&'static str>,
    pub search_sort: &'static str,
    pub search_page: i64,
    pub search_has_more: bool,
    pub search_seq: u64,
    search_cache: HashMap<Option<&'static str>, SearchCacheEntry>,
    pub views: HashMap<&'static str, Vec<View>>,
    pub view: HashMap<&'static str, View>,
    headers: HashMap<&'static str, Vec<Value>>,
    pub me: Option<Value>,
    tz: Option<TimeZone>,
    pub list_total: Option<i64>,
    pub list_sort: String,
    columns: Vec<Column>,
    pub table_columns: Vec<String>,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub focus: Focus,
    pub detail: DetailView,
    pub modals: Vec<(Modal, Cont)>,
    pub toasts: Vec<Toast>,
    pub quit: bool,
    pub(crate) form: Option<ActiveForm>,
    pub(crate) detail_max_scroll: std::cell::Cell<u16>,
    pub(crate) table_offset: std::cell::Cell<usize>,
    pub(crate) table_height: std::cell::Cell<usize>,
}

impl App {
    /// Threaded workers and the wall clock (the real terminal).
    pub fn new(backend: Arc<dyn Backend>, pages: Arc<dyn Pages>, read_only: bool) -> App {
        let (tx, rx) = mpsc::channel();
        App::build(
            backend,
            pages,
            read_only,
            Exec::Threads(tx, rx),
            Clock::Real(Instant::now()),
        )
    }

    /// Inline workers (run by [`App::settle`]) and a manual clock, for tests.
    pub fn new_inline(backend: Arc<dyn Backend>, pages: Arc<dyn Pages>, read_only: bool) -> App {
        App::build(
            backend,
            pages,
            read_only,
            Exec::Inline(VecDeque::new()),
            Clock::Manual(Duration::ZERO),
        )
    }

    fn build(
        backend: Arc<dyn Backend>,
        pages: Arc<dyn Pages>,
        read_only: bool,
        exec: Exec,
        clock: Clock,
    ) -> App {
        let read_only = actions::resolve_read_only(read_only, &backend.read_only_setting());
        let filters = backend.default_where("tickets");
        let history = backend.load_filter_history();
        App {
            backend,
            pages,
            exec,
            clock,
            timers: Vec::new(),
            read_only,
            entity: "tickets",
            detail_tab_idx: 0,
            items: Vec::new(),
            total: 0,
            schemas: HashMap::new(),
            selected: None,
            detail_cache: HashMap::new(),
            details: HashMap::new(),
            sub_cache: HashMap::new(),
            g_pending: None,
            pending_detail_key: None,
            pending_sub_key: None,
            list_fr: 11,
            detail_fr: 9,
            page: 1,
            list_epoch: 0,
            write_failure: None,
            per_page: 100,
            has_more: false,
            loading_more: false,
            active_filters: filters,
            filter_error: None,
            filter: FilterState::new(history),
            search_input: LineInput::new("search freshservice..."),
            search_input_visible: false,
            search_mode: false,
            search_term: String::new(),
            search_totals: Vec::new(),
            search_entity: None,
            search_sort: "relevance",
            search_page: 1,
            search_has_more: false,
            search_seq: 0,
            search_cache: HashMap::new(),
            views: HashMap::new(),
            view: HashMap::new(),
            headers: HashMap::new(),
            me: None,
            tz: None,
            list_total: None,
            list_sort: "created_at".into(),
            columns: Vec::new(),
            table_columns: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            focus: Focus::List,
            detail: DetailView::default(),
            modals: Vec::new(),
            toasts: Vec::new(),
            quit: false,
            form: None,
            detail_max_scroll: std::cell::Cell::new(u16::MAX),
            table_offset: std::cell::Cell::new(0),
            table_height: std::cell::Cell::new(10),
        }
    }

    /// `on_mount`: detail placeholder and the first list load.
    pub fn mount(&mut self) {
        self.clear_detail("[dim]loading rows...[/]");
        self.load_list(false);
    }

    pub fn pages(&self) -> &dyn Pages {
        self.pages.as_ref()
    }
}

const TASK_FIELDS: [(&str, &str); 7] = [
    ("group", "Group ID or exact name"),
    ("status", "Status"),
    ("note", "Note (plain text)"),
    ("notify_before", "Notify Before"),
    ("planned_start", "Planned start (ISO with offset)"),
    ("planned_end", "Planned end (ISO with offset)"),
    ("planned_effort", "Planned effort (1h30m or 01:30)"),
];

#[derive(Debug)]
enum WriteKind {
    Status(Value, String),
    Note(String, bool),
    Reply(String),
}

/// What a list worker needs from the app, captured on the UI thread.
struct ListJob {
    epoch: u64,
    entity: &'static str,
    page: i64,
    append: bool,
    have: Vec<&'static str>,
    need_me: bool,
    need_views: bool,
    need_headers: bool,
    filters: Vec<String>,
    schema: Option<Value>,
    cur_view: Option<View>,
    per_page: i64,
}

impl ListJob {
    /// Schemas, the signed-in agent, saved views, ticket headers, the query
    /// hash and view sort, then one `list_page`.
    fn fetch_list(self, b: &dyn Backend) -> Msg {
        let res = resource_for(self.entity);
        let mut loaded = Loaded::default();
        let outcome = (|| -> ListOutcome {
            for r in ALL_RESOURCES {
                if !self.have.contains(&r.name) {
                    match b.load_schema(r) {
                        Ok(s) => loaded.schemas.push((r.name, s)),
                        Err(e) if e.is_session() => return Err(e.to_string()),
                        Err(_) => loaded.schemas.push((r.name, json!({"fields": []}))),
                    }
                }
            }
            if self.need_me {
                let me = b.current_user().map_err(|e| e.to_string())?;
                let tz = b.user_tz(&me);
                loaded.me = Some((me, tz));
            }
            let mut view = self.cur_view;
            if self.need_views {
                let views = b.load_views(res).map_err(|e| e.to_string())?;
                if view.is_none() {
                    view = site::default_view(&views).cloned();
                }
                loaded.views = Some((res.name, views));
            }
            if self.need_headers {
                loaded.headers = Some(b.ticket_headers().map_err(|e| e.to_string())?);
            }
            let mut query_hash = None;
            if !self.filters.is_empty() {
                let sch = self
                    .schema
                    .or_else(|| {
                        loaded
                            .schemas
                            .iter()
                            .find(|(n, _)| *n == res.name)
                            .map(|(_, s)| s.clone())
                    })
                    .unwrap_or_else(|| json!({"fields": []}));
                query_hash = Some(
                    b.build_query_hash(res, &sch, &self.filters)
                        .map_err(|e| e.to_string())?,
                );
            }
            let view = if self.filters.is_empty() { view } else { None };
            let (mut order_by, mut order_type) = ("created_at".to_string(), "desc".to_string());
            if let Some(v) = view.as_ref().filter(|v| fsv_py::truthy(&v.raw)) {
                let settings = b.view_settings(res, v).map_err(|e| e.to_string())?;
                if let Some(ob) = settings.get("order_by").filter(|x| fsv_py::truthy(x)) {
                    order_by = fsv_py::py_str(ob);
                    order_type = settings
                        .get("order_type")
                        .filter(|x| fsv_py::truthy(x))
                        .map_or("desc".into(), fsv_py::py_str);
                }
            }
            loaded.sort = Some(order_by.clone());
            let req = ListRequest {
                res,
                page: self.page,
                per_page: self.per_page,
                query_hash,
                filter_name: view.map(|v| v.id),
                order_by,
                order_type,
            };
            let (items, data) = b.list_page(&req).map_err(|e| e.to_string())?;
            let (total, more) = site::list_meta(&data, items.len(), self.page, self.per_page);
            Ok((items, total, more))
        })();
        Msg::List {
            epoch: self.epoch,
            entity: self.entity,
            page: self.page,
            append: self.append,
            loaded,
            outcome,
        }
    }
}

fn run_job(job: Job, backend: &dyn Backend) -> Msg {
    match catch_unwind(AssertUnwindSafe(|| job(backend))) {
        Ok(m) => m,
        Err(p) => {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "worker failed".into());
            Msg::Ui(vec![Ui::Toast {
                message: what,
                severity: Severity::Error,
                title: Some("Worker failed".into()),
                secs: TOAST_SECS,
            }])
        }
    }
}

/// `_execute_action`: one send, then a readback that never re-sends.
fn execute_action_job(
    b: &dyn Backend,
    kind: &str,
    item: Item,
    target: &Value,
    body: &Map<String, Value>,
    plan: Option<ReviewPlan>,
) -> Ui {
    let res = item.res;
    let cid = item.ref_id();
    let tid = target.get("id").cloned().unwrap_or(Value::Null);
    if kind == "download" {
        let dest = body
            .get("destination")
            .map(fsv_py::py_str)
            .unwrap_or_default();
        return match actions::download_selected(b, target, &dest) {
            Err(e) => Ui::Failure(actions::failure_detail(&e)),
            Ok(r)
                if matches!(
                    r.get("status").and_then(Value::as_str),
                    Some("error" | "rejected")
                ) =>
            {
                Ui::Failure(actions::failure_detail(&Failure::value(
                    Dumps::default().dump(&r),
                )))
            }
            Ok(r) => Ui::Toast {
                message: Dumps::default().dump(&r),
                severity: Severity::Information,
                title: Some("Download result".into()),
                secs: 15,
            },
        };
    }
    match actions::apply_change_action(b, kind, &cid, target, body, plan.as_ref()) {
        Err(e) => return Ui::Failure(actions::failure_detail(&e)),
        Ok(Some(r)) => {
            return if r.is_object() && r.get("verified").is_some_and(fsv_py::truthy) {
                Ui::AfterWrite(res, item, format!("{kind} complete inventory verified"))
            } else {
                Ui::Failure(actions::failure_detail(&Failure::value(
                    "Association outcome unknown/unverified; do not resubmit",
                )))
            };
        }
        Ok(None) => {}
    }
    let readback = (|| -> Result<(), String> {
        let tasks = kind.starts_with("task-");
        let tab = b
            .load_tab(
                &CHANGES,
                &item.data,
                if tasks { "tasks" } else { "associations" },
            )
            .map_err(|e| e.to_string())?;
        let (rows, _, complete) = actions::target_inventory(&tab, tasks)?;
        let found = rows
            .iter()
            .find(|r| r.get("id").map(fsv_py::py_str) == Some(fsv_py::py_str(&tid)));
        let removing = kind == "task-delete" || kind == "unlink";
        if removing && (found.is_some() || !complete) {
            return Err("Removal is not verified in a complete list".into());
        }
        if !removing && found.is_none_or(|f| body.iter().any(|(k, v)| f.get(k) != Some(v))) {
            return Err("Exact target state was not verified".into());
        }
        Ok(())
    })();
    match readback {
        Ok(()) => Ui::AfterWrite(res, item, format!("{kind} verified")),
        Err(e) => Ui::Failure(format!(
            "Write sent once; readback unverified: {e}. Do not resend; inspect the record."
        )),
    }
}

mod assets;
mod bars;
mod cont;
mod detail;
mod entities;
mod filter_input;
mod forms;
mod key_handler;
mod list;
mod reviewed;
mod runtime;
mod search;
mod writes;

#[cfg(test)]
mod tests;
