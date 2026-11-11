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

    // ── runtime ─────────────────────────────────────────────

    fn spawn(&mut self, job: impl FnOnce(&dyn Backend) -> Msg + Send + 'static) {
        match &mut self.exec {
            Exec::Inline(q) => q.push_back(Box::new(job)),
            Exec::Threads(tx, _) => {
                let tx = tx.clone();
                let backend = Arc::clone(&self.backend);
                std::thread::spawn(move || {
                    let msg = run_job(Box::new(job), backend.as_ref());
                    let _ = tx.send(msg);
                });
            }
        }
    }

    fn set_timer(&mut self, slot: Slot, delay: Duration, kind: TimerKind) {
        self.stop_timer(slot);
        let due = self.clock.now() + delay;
        self.timers.push(Timer { due, slot, kind });
    }

    fn stop_timer(&mut self, slot: Slot) {
        self.timers.retain(|t| t.slot != slot);
    }

    /// Deliver finished workers, fire due timers, expire toasts.
    pub fn tick(&mut self) {
        let mut msgs = Vec::new();
        if let Exec::Threads(_, rx) = &self.exec {
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
        }
        for m in msgs {
            self.on_msg(m);
        }
        self.fire_timers();
        let now = self.clock.now();
        self.toasts.retain(|t| t.expires > now);
        if let Some(form) = &self.form {
            let clip = {
                let mut view = form.view.borrow_mut();
                view.tick(now.saturating_sub(form.start));
                view.take_clipboard()
            };
            if let Some(text) = clip {
                let _ = self.backend.copy_to_clipboard(&text);
            }
        }
        self.check_form_done();
    }

    fn fire_timers(&mut self) {
        let now = self.clock.now();
        let mut due: Vec<Timer> = Vec::new();
        let mut i = 0;
        while i < self.timers.len() {
            if self.timers[i].due <= now {
                due.push(self.timers.remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by_key(|t| t.due);
        for t in due {
            match t.kind {
                TimerKind::Detail(item, key) => self.load_detail(item, key),
                TimerKind::Entity => self.reload_list("[dim]loading rows...[/]"),
                TimerKind::Lookup(req) => self.run_network_lookup(req),
            }
        }
    }

    /// Tests: run queued inline workers and due timers until quiet.
    pub fn settle(&mut self) {
        for _ in 0..1000 {
            self.fire_timers();
            let job = match &mut self.exec {
                Exec::Inline(q) => q.pop_front(),
                Exec::Threads(..) => None,
            };
            let Some(job) = job else { break };
            let backend = Arc::clone(&self.backend);
            let msg = run_job(job, backend.as_ref());
            self.on_msg(msg);
        }
    }

    /// Tests: move the manual clock forward and settle.
    pub fn advance(&mut self, d: Duration) {
        if let Clock::Manual(now) = &mut self.clock {
            *now += d;
        }
        self.settle();
    }

    pub fn now(&self) -> Duration {
        self.clock.now()
    }

    pub fn notify(&mut self, message: impl Into<String>, severity: Severity) {
        self.notify_full(message.into(), severity, None, TOAST_SECS);
    }

    fn notify_full(
        &mut self,
        message: String,
        severity: Severity,
        title: Option<String>,
        secs: u64,
    ) {
        let expires = self.clock.now() + Duration::from_secs(secs);
        self.toasts.push(Toast {
            title,
            message,
            severity,
            expires,
        });
    }

    fn push(&mut self, modal: Modal, cont: Cont) {
        self.modals.push((modal, cont));
    }

    fn show_failure(&mut self, message: String) {
        self.write_failure = Some(message.clone());
        self.push(Modal::failure(&message), Cont::None);
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::List {
                epoch,
                entity,
                page,
                append,
                loaded,
                outcome,
            } => {
                self.merge_loaded(loaded);
                match outcome {
                    Ok((items, total, more)) => {
                        self.accept_list(epoch, entity, page, items, total, more, append)
                    }
                    Err(e) => self.list_failed(epoch, entity, e),
                }
            }
            Msg::Detail { item, key, outcome } => {
                let cache_key = item.key();
                match outcome {
                    Ok(page) => {
                        self.detail_cache.insert(cache_key, page.record.clone());
                        let record = page.record.clone();
                        self.details.insert(item.key(), page);
                        self.show_detail(Item::new(item.res, record), &key);
                    }
                    Err(e) => match self.detail_cache.get(&cache_key).cloned() {
                        Some(cached) => self.show_detail(Item::new(item.res, cached), &key),
                        None => self.detail_error(&key, e),
                    },
                }
            }
            Msg::Sub {
                tab,
                key,
                res,
                data,
            } => {
                self.sub_cache.insert(key.clone(), data.clone());
                self.render_sub_content(&tab, &data, res, &key);
            }
            Msg::Search {
                seq,
                append,
                schemas,
                outcome,
            } => {
                for (n, s) in schemas {
                    self.schemas.entry(n).or_insert(s);
                }
                match outcome {
                    Ok((items, totals)) => self.populate_search(items, totals, append, seq),
                    Err(e) => {
                        if seq == self.search_seq {
                            self.loading_more = false;
                            self.notify_full(
                                e,
                                Severity::Error,
                                Some("Search failed".into()),
                                TOAST_SECS,
                            );
                        }
                    }
                }
            }
            Msg::Lookup { key, values } => self.filter.apply_network(&key, values),
            Msg::Ui(list) => {
                for u in list {
                    self.on_ui(u);
                }
            }
        }
    }

    fn on_ui(&mut self, u: Ui) {
        match u {
            Ui::Failure(m) => self.show_failure(m),
            Ui::Toast {
                message,
                severity,
                title,
                secs,
            } => self.notify_full(message, severity, title, secs),
            Ui::AfterWrite(res, item, msg) => self.after_write(res, item, &msg),
            Ui::TicketNoteVerified { ticket_id, note_id } => {
                let mut item =
                    Item::new(&TICKETS, json!({"id": ticket_id, "display_id": ticket_id}));
                if let Some(sel) = &self.selected
                    && sel.res.name == "tickets"
                    && sel.ref_id() == ticket_id
                {
                    item = sel.clone();
                }
                self.after_write(
                    &TICKETS,
                    item,
                    &format!("Note {} verified", fsv_py::py_str(&note_id)),
                );
            }
            Ui::Catalog(items, categories) => self.pick_catalog(items, categories),
            Ui::TaskPlan(item, plan) => self.review_change_task(item, plan),
            Ui::Targets {
                kind,
                item,
                rows,
                warnings,
            } => self.pick_action_target(kind, item, rows, warnings),
            Ui::TaskEditor {
                item,
                target,
                current,
                fields,
            } => self.edit_task_field(item, target, current, fields),
            Ui::ApprovalReview { item, plan, before } => self.review_approval(item, plan, before),
            Ui::ApprovalTargets { kind, item, rows } => self.pick_approval_target(kind, item, rows),
            Ui::ApprovalResult { item, result } => self.approval_result(item, result),
            Ui::Form(opened) => self.push_form(opened),
            Ui::ReloadList => self.reload_list("[dim]reloading...[/]"),
            Ui::Push(modal, cont) => self.push(*modal, cont),
            Ui::Outcome { title, message } => {
                self.push(Modal::outcome(&title, &message), Cont::None)
            }
        }
    }

    // ── list loading ────────────────────────────────────────

    pub fn load_list(&mut self, append: bool) {
        let job = self.list_job(append);
        self.spawn(move |b| job.fetch_list(b));
    }

    fn list_job(&self, append: bool) -> ListJob {
        let res = resource_for(self.entity);
        ListJob {
            epoch: self.list_epoch,
            entity: self.entity,
            page: if append { self.page + 1 } else { 1 },
            append,
            have: self.schemas.keys().copied().collect(),
            need_me: self.me.is_none(),
            need_views: !self.views.contains_key(res.name),
            need_headers: res.name == "tickets" && !self.headers.contains_key(res.name),
            filters: self.active_filters.clone(),
            schema: self.schemas.get(res.name).cloned(),
            cur_view: self.view.get(res.name).cloned(),
            per_page: self.per_page,
        }
    }

    fn merge_loaded(&mut self, loaded: Loaded) {
        for (n, s) in loaded.schemas {
            self.schemas.entry(n).or_insert(s);
        }
        if let Some((me, tz)) = loaded.me
            && !me.is_null()
        {
            self.me = Some(me);
            self.tz = tz;
        }
        if let Some((name, views)) = loaded.views {
            if !self.view.contains_key(name)
                && let Some(v) = site::default_view(&views)
            {
                self.view.insert(name, v.clone());
            }
            self.views.insert(name, views);
        }
        if let Some(h) = loaded.headers {
            self.headers.insert("tickets", h);
        }
        if let Some(s) = loaded.sort {
            self.list_sort = s;
        }
    }

    fn list_failed(&mut self, epoch: u64, entity: &str, message: String) {
        if (epoch, entity) != (self.list_epoch, self.entity) {
            return;
        }
        self.loading_more = false;
        self.notify_full(
            message,
            Severity::Error,
            Some("List load failed; reload or retry".into()),
            TOAST_SECS,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_list(
        &mut self,
        epoch: u64,
        entity: &str,
        page: i64,
        items: Vec<Value>,
        total: Option<i64>,
        more: bool,
        append: bool,
    ) {
        if (epoch, entity) != (self.list_epoch, self.entity) || self.search_mode {
            return;
        }
        if append && page != self.page + 1 {
            return;
        }
        self.page = page;
        self.list_total = total;
        let res = resource_for(entity);
        let items: Vec<Item> = items.into_iter().map(|v| Item::new(res, v)).collect();
        let lower = if append {
            self.items.len() + items.len()
        } else {
            items.len()
        } as i64;
        self.populate_list(items, total.unwrap_or(lower), append, Some(more));
    }

    fn populate_list(
        &mut self,
        items: Vec<Item>,
        total: i64,
        append: bool,
        has_more: Option<bool>,
    ) {
        self.loading_more = false;
        self.total = total;
        self.has_more = has_more.unwrap_or(items.len() as i64 == self.per_page);
        let mut seen: Vec<String> = if append {
            self.items.iter().map(Item::key).collect()
        } else {
            Vec::new()
        };
        let mut unique = Vec::new();
        for it in items {
            let k = it.key();
            if !seen.contains(&k) {
                seen.push(k);
                unique.push(it);
            }
        }
        let res_now = resource_for(self.entity);
        let offset = if append {
            self.items.len()
        } else {
            self.columns = if self.search_mode {
                Vec::new()
            } else {
                site::list_columns(
                    res_now,
                    self.schemas.get(res_now.name),
                    self.headers.get(res_now.name).map(Vec::as_slice),
                )
            };
            self.rows.clear();
            self.cursor = 0;
            self.table_columns = if self.columns.is_empty() {
                ["", "ID", "STATUS", "PRI", "SUBJECT", "REQUESTER"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                std::iter::once(String::new())
                    .chain(self.columns.iter().map(|c| c.label.clone()))
                    .collect()
            };
            0
        };
        let me_id = self.me.as_ref().and_then(|m| m.get("id")).cloned();
        let empty = json!({"fields": []});
        for (idx, item) in unique.iter().enumerate() {
            let did = item.display_id();
            let sel = format!("{:03}", offset + idx + 1);
            let schema = self.schemas.get(item.res.name).unwrap_or(&empty);
            let mut cells = vec![(TableCell::plain(sel), 0)];
            if !self.columns.is_empty() {
                let row = site::list_row(
                    &item.data,
                    item.res,
                    &self.columns,
                    Some(schema),
                    me_id.as_ref(),
                    self.tz.as_ref(),
                );
                for col in &self.columns {
                    let val = row
                        .get(&col.key)
                        .map(|v| {
                            if let Value::String(s) = v {
                                s.clone()
                            } else {
                                fsv_py::py_str(v)
                            }
                        })
                        .unwrap_or_default();
                    let cell = match col.key.as_str() {
                        "subject" => {
                            let prefix = format!("#{did} ");
                            let n = prefix.chars().count();
                            (TableCell::plain(prefix + &clip(&val, 60)), n)
                        }
                        "status" => (
                            TableCell {
                                text: clip(&val, 24),
                                style: Style::default().fg(Color::Green),
                            },
                            0,
                        ),
                        "assigned_to" if val.ends_with(" (Me)") && val.chars().count() > 40 => {
                            let base: String = val.chars().take(val.chars().count() - 5).collect();
                            (TableCell::plain(clip(&base, 35) + " (Me)"), 0)
                        }
                        k => {
                            let w = if k == "assigned_to" { 40 } else { 34 };
                            let c = clip(&val, w);
                            if c.is_empty() {
                                (
                                    TableCell {
                                        text: "--".into(),
                                        style: Style::default().add_modifier(Modifier::DIM),
                                    },
                                    0,
                                )
                            } else {
                                (TableCell::plain(c), 0)
                            }
                        }
                    };
                    cells.push(cell);
                }
            } else {
                let requester = item
                    .get("requester")
                    .filter(|r| r.is_object())
                    .and_then(|r| r.get("name"))
                    .filter(|n| fsv_py::truthy(n))
                    .map(fsv_py::py_str)
                    .unwrap_or_default();
                cells.push((TableCell::plain(did.clone()), 0));
                cells.push((TableCell::plain(resolve_status(&item.data, schema)), 0));
                cells.push((TableCell::plain(resolve_priority(&item.data)), 0));
                cells.push((TableCell::plain(cut(&item.str("subject"), 80)), 0));
                cells.push((TableCell::plain(requester), 0));
            }
            self.rows.push(Row { key: did, cells });
        }
        let had = !unique.is_empty();
        if append {
            self.items.extend(unique);
        } else {
            self.items = unique;
        }
        if !append && had {
            self.clear_detail("[dim]loading details...[/]");
            self.move_cursor(0, true);
        } else if !append {
            self.clear_detail("[dim]no rows[/]");
        }
    }

    fn load_more(&mut self) {
        if self.loading_more || !self.has_more {
            return;
        }
        self.loading_more = true;
        self.load_list(true);
    }

    pub fn reload_list(&mut self, message: &str) {
        self.list_epoch += 1;
        self.list_total = None;
        self.items.clear();
        self.total = 0;
        self.page = 1;
        self.has_more = false;
        self.loading_more = false;
        self.g_pending = None;
        self.filter_error = None;
        self.detail_cache.clear();
        self.sub_cache.clear();
        self.clear_table();
        self.focus_pane(Pane::List);
        self.clear_detail(message);
        self.load_list(false);
    }

    fn clear_table(&mut self) {
        self.rows.clear();
        self.table_columns.clear();
        self.cursor = 0;
        self.table_offset.set(0);
    }

    // ── table cursor ────────────────────────────────────────

    fn move_cursor(&mut self, row: usize, force: bool) {
        if self.rows.is_empty() {
            return;
        }
        let row = row.min(self.rows.len() - 1);
        let changed = row != self.cursor;
        self.cursor = row;
        if changed || force {
            self.row_highlighted();
        }
    }

    fn cursor_step(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize;
        self.move_cursor(next, false);
    }

    fn find_item(&self, did: &str) -> Option<Item> {
        self.items.iter().find(|i| i.display_id() == did).cloned()
    }

    /// `FsvApp.on_data_table_row_highlighted`: show the cached record or a
    /// placeholder, debounce the detail load, and page at the last row.
    fn row_highlighted(&mut self) {
        let Some(did) = self.rows.get(self.cursor).map(|r| r.key.clone()) else {
            return;
        };
        let Some(item) = self.find_item(&did) else {
            return;
        };
        self.detail_tab_idx = 0;
        let key = item.key();
        if let Some(cached) = self.detail_cache.get(&key).cloned() {
            self.pending_detail_key = Some(key.clone());
            self.pending_sub_key = None;
            self.show_detail(Item::new(item.res, cached), &key);
        } else {
            self.set_pending_detail(&key, "[dim]loading details...[/]");
        }
        self.set_timer(Slot::Detail, DETAIL_DEBOUNCE, TimerKind::Detail(item, key));
        if self.cursor + 1 == self.rows.len() && !self.loading_more {
            if self.search_mode && self.search_has_more {
                self.search_load_more();
            } else if self.has_more {
                self.load_more();
            }
        }
    }

    // ── detail ──────────────────────────────────────────────

    fn load_detail(&mut self, item: Item, key: String) {
        let cache_key = item.key();
        if let (Some(rec), true) = (
            self.detail_cache.get(&cache_key).cloned(),
            self.details.contains_key(&cache_key),
        ) {
            self.show_detail(Item::new(item.res, rec), &key);
            return;
        }
        let schema = self.schemas.get(item.res.name).cloned();
        self.spawn(move |b| {
            let outcome = b
                .load_detail(item.res, &item.id(), schema.as_ref())
                .map_err(|e| actions::clean_error(&e));
            Msg::Detail { item, key, outcome }
        });
    }

    fn detail_error(&mut self, key: &str, message: String) {
        if self.pending_detail_key.as_deref() != Some(key) {
            return;
        }
        self.detail.content = Content::Plain(
            format!("could not load: {message}"),
            Style::default().fg(Color::Red),
        );
    }

    fn page_for(&self, item: Option<&Item>) -> Option<&DetailPage> {
        item.and_then(|i| self.details.get(&i.key()))
    }

    pub fn tab_names(&self, res: Option<Res>) -> Vec<String> {
        if let Some(page) = self.page_for(self.selected.as_ref())
            && res.is_none_or(|r| r.name == page.res.name)
        {
            return page.tab_names();
        }
        let res = res
            .or(self.selected.as_ref().map(|s| s.res))
            .unwrap_or(&TICKETS);
        detail_tab_labels(res.name)
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    pub fn current_tab_name(&self, res: Option<Res>) -> String {
        if res.is_none() && self.selected.is_none() {
            return "details".into();
        }
        let tabs = self.tab_names(res);
        tabs.get(self.detail_tab_idx.min(tabs.len().saturating_sub(1)))
            .cloned()
            .unwrap_or_else(|| "details".into())
    }

    fn show_detail(&mut self, item: Item, key: &str) {
        if self.pending_detail_key.as_deref() != Some(key) {
            return;
        }
        let res = item.res;
        self.selected = Some(item.clone());
        self.pending_sub_key = None;
        if self.detail_tab_idx >= self.tab_names(Some(res)).len() {
            self.detail_tab_idx = 0;
        }
        self.render_detail_bar(&item);
        self.trigger_state_flow(&item);
        self.render_detail_tabs(res);
        self.render_detail_content(&item);
    }

    pub fn active_pane(&self) -> Pane {
        if self.focus == Focus::Detail {
            Pane::Detail
        } else {
            Pane::List
        }
    }

    fn focus_pane(&mut self, pane: Pane) {
        self.focus = if pane == Pane::Detail {
            Focus::Detail
        } else {
            Focus::List
        };
    }

    fn toggle_pane(&mut self) {
        self.focus_pane(if self.active_pane() == Pane::List {
            Pane::Detail
        } else {
            Pane::List
        });
    }

    fn selected_key(&self) -> Option<String> {
        self.selected.as_ref().map(Item::key)
    }

    /// The properties side panel; `None` hides it.
    fn show_side(&mut self, lines: Option<Vec<String>>) {
        self.detail.side = lines;
    }

    fn scroll_detail_home(&mut self) {
        self.detail.scroll = 0;
    }

    /// Python defers this until after the refresh; the next draw is that.
    fn reset_detail_scroll(&mut self) {
        self.scroll_detail_home();
    }

    fn clear_detail(&mut self, message: &str) {
        self.selected = None;
        self.pending_detail_key = None;
        self.pending_sub_key = None;
        self.detail = DetailView {
            content: Content::Markup(message.to_string()),
            ..DetailView::default()
        };
        self.show_side(None);
        self.reset_detail_scroll();
    }

    fn set_pending_detail(&mut self, key: &str, message: &str) {
        self.selected = None;
        self.pending_detail_key = Some(key.to_string());
        self.pending_sub_key = None;
        self.detail.bar.clear();
        self.detail.tabs.clear();
        self.show_side(None);
        self.detail.content = Content::Markup(message.to_string());
        self.reset_detail_scroll();
    }

    fn trigger_state_flow(&mut self, item: &Item) {
        let (flow, rep) = match self.page_for(Some(item)) {
            Some(page) => (
                self.pages.lifecycle_markup(page, 3),
                self.pages.reporter_markup(page),
            ),
            None => (String::new(), String::new()),
        };
        self.detail.state_flow = flow;
        self.detail.reporter = rep;
    }

    fn render_detail_bar(&mut self, item: &Item) {
        let did = item.display_id();
        let subject = item
            .get("subject")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        let mut bar: Vec<(String, Style)> = Vec::new();
        if let Some(page) = self.page_for(Some(item)) {
            bar.push((
                format!("#{did} "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            bar.push((
                format!("{}  ", cut(&subject, 90)),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            bar.push((
                page.badge.clone(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
            let stats = page.record.get("stats").cloned().unwrap_or(Value::Null);
            let done = ["resolved_at", "closed_at"]
                .iter()
                .any(|k| stats.get(*k).is_some_and(fsv_py::truthy));
            if !page.due.is_empty() && (item.res.name != "tickets" || !done) {
                bar.push((
                    format!(", due {}", page.due),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
        } else {
            let schema = self
                .schemas
                .get(item.res.name)
                .cloned()
                .unwrap_or_else(|| json!({"fields": []}));
            if item.res.name != "changes" {
                bar.push((
                    format!("[{}]", resolve_status(&item.data, &schema)),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ));
                bar.push(("  ".into(), Style::default()));
            }
            bar.push((format!("{did} {}  ", cut(&subject, 80)), Style::default()));
            bar.push((
                resolve_priority(&item.data),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
        self.detail.bar = bar;
    }

    fn render_detail_tabs(&mut self, res: Res) {
        let active = self.current_tab_name(Some(res));
        if let Some(page) = self
            .page_for(self.selected.as_ref())
            .filter(|p| p.res.name == res.name)
        {
            self.detail.tabs = self.pages.tab_bar(page, &active, true);
            return;
        }
        self.detail.tabs = detail_tab_labels(res.name)
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if i == self.detail_tab_idx {
                    format!("[bold reverse] {}:{t} [/]", i + 1)
                } else {
                    format!("[dim]{}:{t}[/]", i + 1)
                }
            })
            .collect::<Vec<_>>()
            .join("  ");
    }

    fn render_detail_content(&mut self, item: &Item) {
        let tab = self.current_tab_name(Some(item.res));
        if tab == "details" {
            self.pending_sub_key = None;
            if let Some(page) = self.page_for(Some(item)) {
                let mut lines = self.pages.main_lines(page);
                lines.push(String::new());
                lines.extend(self.pages.notes_lines(page));
                let side = self.pages.side_lines(page);
                self.detail.content = Content::Markup(lines.join("\n"));
                self.show_side(Some(side));
            } else {
                let schema = self
                    .schemas
                    .get(item.res.name)
                    .cloned()
                    .unwrap_or_else(|| json!({"fields": []}));
                self.detail.content =
                    Content::Markup(format::format_details(&item.data, item.res, &schema));
                self.show_side(None);
            }
            self.reset_detail_scroll();
        } else {
            self.show_side(None);
            let sub_key = format!("{}:{tab}", item.key());
            self.pending_sub_key = Some(sub_key.clone());
            self.detail.content = Content::Markup(format!("[dim]loading {tab}...[/]"));
            self.reset_detail_scroll();
            self.load_sub_resource(tab, sub_key);
        }
    }

    fn load_sub_resource(&mut self, tab: String, sub_key: String) {
        let Some(item) = self.selected.clone() else {
            return;
        };
        let res = item.res;
        let cache_key = format!("{}:{tab}", item.key());
        if let Some(data) = self.sub_cache.get(&cache_key).cloned() {
            self.render_sub_content(&tab, &data, res, &sub_key);
            return;
        }
        let record = self
            .page_for(Some(&item))
            .map_or_else(|| item.data.clone(), |p| p.record.clone());
        self.spawn(move |b| {
            let data = b.load_tab(res, &record, &tab).unwrap_or_else(|e| TabData {
                name: tab.clone(),
                label: site::tab_label(&tab),
                missing: Some(Missing::new(actions::clean_error(&e))),
                ..TabData::default()
            });
            Msg::Sub {
                tab,
                key: sub_key,
                res,
                data,
            }
        });
    }

    fn render_sub_content(&mut self, tab: &str, data: &TabData, res: Res, sub_key: &str) {
        let _ = tab;
        if self.pending_sub_key.as_deref() != Some(sub_key) {
            return;
        }
        let Some(current) = self.selected_key() else {
            return;
        };
        if format!("{current}:{}", self.current_tab_name(Some(res))) != sub_key {
            return;
        }
        let page = self.page_for(self.selected.as_ref());
        let tz = page.and_then(|p| p.tz.as_ref()).or(self.tz.as_ref());
        let schema = page.and_then(|p| p.schema.as_ref());
        let mut lines = vec![String::new()];
        lines.extend(self.pages.tab_lines(data, tz, schema));
        lines.push(String::new());
        self.detail.content = Content::Markup(lines.join("\n"));
        self.reset_detail_scroll();
    }

    fn set_detail_tab(&mut self, idx: usize) {
        let Some(sel) = self.selected.clone() else {
            return;
        };
        self.detail_tab_idx = idx;
        self.render_detail_tabs(sel.res);
        self.render_detail_content(&sel);
    }

    fn step_detail_tab(&mut self, delta: isize) {
        let Some(sel) = &self.selected else { return };
        let n = self.tab_names(Some(sel.res)).len().max(1) as isize;
        let next = (self.detail_tab_idx as isize + delta).rem_euclid(n) as usize;
        self.set_detail_tab(next);
    }

    fn detail_scroll(&mut self, delta: i32) {
        let max = self.detail_max_scroll.get() as i32;
        self.detail.scroll = (self.detail.scroll as i32 + delta).clamp(0, max.max(0)) as u16;
    }

    // ── header / bars ───────────────────────────────────────

    pub fn view_header(&self) -> String {
        let res = resource_for(self.entity);
        let view = if self.active_filters.is_empty() {
            self.view.get(res.name)
        } else {
            None
        };
        let name = match view {
            Some(v) => v.name.clone(),
            None if !self.active_filters.is_empty() => "Filtered".into(),
            None => format!("All {}", res.name),
        };
        let rows = self.items.len();
        let rng = if rows > 0 {
            format!("1-{rows}")
        } else {
            "0".into()
        };
        let of = match self.list_total {
            Some(t) => format!(" of {t}"),
            None => format!(" of {rows}{}", if self.has_more { "+" } else { "" }),
        };
        let sort = site::sort_label(&self.list_sort, self.schemas.get(res.name));
        format!(
            "[bold]{}[/][dim] · {rng}{of} · Sort by: {}[/]",
            escape(&name),
            escape(&sort)
        )
    }

    fn count_text(&self) -> String {
        let rows = self.items.len() as i64;
        if self.total > rows {
            format!("{rows}/{}", self.total)
        } else {
            rows.to_string()
        }
    }

    /// `FsvApp._render_header`: entity tabs or search tabs with counts.
    pub fn header_markup(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.search_mode {
            parts.push(format!(
                "[bold cyan]fsv[/] :: [yellow bold]search[/] [yellow]{}[/]  ",
                escape(&self.search_term)
            ));
            parts.push(if self.search_entity.is_none() {
                "[bold reverse] ALL [/] ".into()
            } else {
                "ALL  ".into()
            });
            for tab in ENTITY_TABS {
                let label = tab.to_uppercase();
                let cnt = self
                    .search_totals
                    .iter()
                    .find(|(k, _)| k == tab)
                    .map_or(String::new(), |(_, c)| format!("({c})"));
                parts.push(if Some(tab) == self.search_entity {
                    format!("[bold reverse] {label}{cnt} [/] ")
                } else {
                    format!("{label}{cnt}  ")
                });
            }
            let sort = match self.search_sort {
                "created" => "date created",
                "modified" => "last modified",
                s => s,
            };
            parts.push(format!(
                "  [dim]{} shown  sort:{sort}  Esc exit[/]",
                self.count_text()
            ));
        } else {
            parts.push("[bold cyan]fsv[/] :: freshservice  ".into());
            for tab in ENTITY_TABS {
                let label = tab.to_uppercase();
                parts.push(if tab == self.entity {
                    format!("[bold reverse] {label} [/] ")
                } else {
                    format!("{label}  ")
                });
            }
            parts.push(format!("  {}", self.view_header()));
            parts.push("  [dim]v views  ? help[/]".into());
        }
        parts.concat()
    }

    /// `FsvApp._render_filter_bar`.
    pub fn filter_bar_markup(&self) -> String {
        let sel = self.selected.as_ref().map_or("-".into(), Item::display_id);
        let loading = if self.loading_more {
            "  [yellow]loading more…[/]"
        } else {
            ""
        };
        let part = if self.search_mode {
            format!(
                "[yellow]search: {}[/]  [dim]h/l narrow  s sort  Esc exit  f new search[/]",
                escape(&self.search_term)
            )
        } else if let Some(e) = &self.filter_error {
            format!("[red]filter error: {}[/]", escape(e))
        } else if !self.active_filters.is_empty() {
            format!(
                "[yellow]filter: {}[/]",
                escape(&self.active_filters.join(" "))
            )
        } else {
            "[dim]/ to filter[/]".into()
        };
        format!(
            "{part}  [dim]focus={}  sel={}  {} rows[/]{loading}",
            self.active_pane().as_str(),
            escape(&sel),
            self.count_text()
        )
    }

    /// `FsvApp._render_status`: `(label, dim suffix)`.
    pub fn status_parts(&self) -> (String, String) {
        let mut label = if self.search_mode {
            let mut l = format!("SEARCH: {}", self.search_term);
            if let Some(e) = self.search_entity {
                l += &format!(" > {}", e.to_uppercase());
            }
            l
        } else {
            self.entity.to_uppercase()
        };
        if let Some(sel) = &self.selected {
            label += &format!(
                " > {} > {}",
                sel.display_id(),
                self.current_tab_name(Some(sel.res))
            );
        }
        let suffix = if self.loading_more {
            "loading more…".to_string()
        } else {
            format!("{} rows", self.count_text())
        };
        (
            format!("{label}    "),
            format!("pane={} · {suffix}", self.active_pane().as_str()),
        )
    }

    /// `FsvApp._render_key_hints`.
    pub fn key_hints_markup(&self) -> String {
        let write = if self.read_only {
            ""
        } else {
            "[bold]s[/] status  [bold]n[/] note  [bold]N[/]/[bold]e[/] create/edit  [bold]a[/] actions  [bold]![/] failure  "
        };
        let rkey = if self.read_only {
            "[bold]r[/] reload  "
        } else {
            "[bold]r[/] reload/reply  "
        };
        format!(
            "[bold]Tab[/] pane  [bold]j/k[/] move or scroll  [bold]J/K[/] ×5  [bold]G/gg[/] ↕  [bold]h/l[/] entity/tab  [bold]gw/gt/gp/gc[/] entity  [bold]1-9[/] detail tab  [bold]v[/] views  [bold]\\[][/bold] resize  [bold]f[/] search  [bold]/[/] filter  [bold]o[/] browser  {write}{rkey}[bold]y[/] yank  [bold]q[/]/[bold]Ctrl+C[/] quit  [bold]?[/] help"
        )
    }

    // ── filter input ────────────────────────────────────────

    fn with_source<R>(&mut self, f: impl FnOnce(&mut FilterState, &Source<'_>) -> R) -> R {
        let fields: Vec<Value> = self
            .schemas
            .get(self.entity)
            .and_then(|s| s.get("fields"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let backend = Arc::clone(&self.backend);
        let groups = move || backend.cached_group_names();
        let src = Source {
            fields: &fields,
            items: &self.items,
            groups: &groups,
        };
        f(&mut self.filter, &src)
    }

    fn open_filter_input(&mut self) {
        let current = self.active_filters.join(" ");
        self.with_source(|st, src| st.open(&current, src));
        self.focus = Focus::Filter;
        self.maybe_schedule_lookup();
    }

    /// `FsvApp._cancel_filter_input`; applying a filter closes it the same way.
    fn close_filter(&mut self) {
        self.filter.close();
        self.stop_timer(Slot::Lookup);
        self.focus = Focus::List;
    }

    fn apply_filter(&mut self, raw: &str) {
        self.close_filter();
        if !raw.trim().is_empty() {
            self.active_filters = filter::shlex_split(raw)
                .unwrap_or_else(|| raw.split_whitespace().map(str::to_string).collect());
            self.filter.remember(raw.trim());
            self.backend.save_filter_history(&self.filter.history);
        } else {
            self.active_filters.clear();
        }
        self.reload_list("[dim]filtering...[/]");
    }

    /// `FsvApp._maybe_schedule_network_lookup`.
    fn maybe_schedule_lookup(&mut self) {
        if !self.backend.completion_network() {
            return;
        }
        let fields: Vec<Value> = self
            .schemas
            .get(self.entity)
            .and_then(|s| s.get("fields"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        match filter::network_lookup(
            self.filter.input.value(),
            &fields,
            self.filter.tab_base.is_some(),
            &self.filter.lookup_key,
        ) {
            Lookup::Nothing => {}
            Lookup::Cancel => self.stop_timer(Slot::Lookup),
            Lookup::Schedule(req) => {
                self.filter.lookup_key = req.key.clone();
                self.set_timer(Slot::Lookup, LOOKUP_DEBOUNCE, TimerKind::Lookup(req));
            }
        }
    }

    fn run_network_lookup(&mut self, req: LookupRequest) {
        if req.key != self.filter.lookup_key {
            return;
        }
        self.spawn(move |b| {
            let values = match (req.kind.as_str(), &req.link) {
                ("lookup", Some(link)) => b
                    .lookup_choices(link, &req.prefix)
                    .map(|rows| filter::lookup_values(rows, &req.prefix)),
                _ => b
                    .remote_user_values(&req.kind, &req.prefix)
                    .map(|v| v.into_iter().take(8).collect()),
            };
            Msg::Lookup {
                key: req.key,
                values: values.unwrap_or_default(),
            }
        });
    }

    fn filter_key(&mut self, ev: &KeyEvent, key: &str) {
        match key {
            "escape" => {
                self.close_filter();
                return;
            }
            "tab" => {
                self.with_source(|st, src| st.complete(src));
                return;
            }
            "up" | "ctrl+p" => {
                self.with_source(|st, src| st.history_navigate(1, src));
                self.maybe_schedule_lookup();
                return;
            }
            "down" | "ctrl+n" => {
                self.with_source(|st, src| st.history_navigate(-1, src));
                self.maybe_schedule_lookup();
                return;
            }
            _ => {}
        }
        match self.filter.input.handle(ev) {
            InputEvent::Submitted => self.on_input_submitted(Focus::Filter),
            InputEvent::Changed => self.on_input_changed(),
            _ => {}
        }
    }

    /// Enter in the filter or search input.
    fn on_input_submitted(&mut self, input: Focus) {
        match input {
            Focus::Filter => {
                let raw = self.filter.input.value().to_string();
                self.apply_filter(&raw);
            }
            Focus::Search => {
                let raw = self.search_input.value().to_string();
                self.apply_search(&raw);
            }
            _ => {}
        }
    }

    /// Typing in the filter input; code-set values (Tab, history) never get
    /// here, which is what Python's tab / history locks are for.
    fn on_input_changed(&mut self) {
        self.with_source(|st, src| st.typed(src));
        self.maybe_schedule_lookup();
    }

    fn cancel_search_input(&mut self) {
        self.search_input_visible = false;
        self.focus = Focus::List;
    }

    // ── search ──────────────────────────────────────────────

    fn open_search_input(&mut self) {
        let v = if self.search_mode {
            self.search_term.clone()
        } else {
            String::new()
        };
        self.search_input.set_value(&v);
        self.search_input_visible = true;
        self.focus = Focus::Search;
    }

    fn reset_search_rows(&mut self) {
        self.items.clear();
        self.total = 0;
        self.page = 1;
        self.has_more = false;
        self.detail_cache.clear();
        self.sub_cache.clear();
        self.clear_table();
    }

    fn apply_search(&mut self, raw: &str) {
        self.search_input_visible = false;
        let term = raw.trim().to_string();
        if term.is_empty() {
            if self.search_mode {
                self.exit_search_mode();
            } else {
                self.focus = Focus::List;
            }
            return;
        }
        self.search_seq += 1;
        self.search_cache.clear();
        self.search_mode = true;
        self.search_term = term;
        self.search_entity = None;
        self.search_sort = "relevance";
        self.search_page = 1;
        self.search_has_more = false;
        self.active_filters.clear();
        self.filter_error = None;
        self.reset_search_rows();
        self.focus_pane(Pane::List);
        self.clear_detail("[dim]searching...[/]");
        self.load_search(false);
    }

    fn exit_search_mode(&mut self) {
        self.search_seq += 1;
        self.search_cache.clear();
        self.search_mode = false;
        self.search_term.clear();
        self.search_totals.clear();
        self.search_entity = None;
        self.search_sort = "relevance";
        self.search_page = 1;
        self.search_has_more = false;
        self.active_filters = self.get_default_filters(self.entity);
        self.filter_error = None;
        self.reload_list("[dim]loading rows...[/]");
    }

    fn step_search_entity(&mut self, delta: isize) {
        let tabs: [Option<&'static str>; 4] =
            [None, Some("tickets"), Some("problems"), Some("changes")];
        let idx = tabs
            .iter()
            .position(|t| *t == self.search_entity)
            .unwrap_or(0) as isize;
        let new = tabs[(idx + delta).rem_euclid(4) as usize];
        if new == self.search_entity {
            return;
        }
        self.save_search_cache();
        if self.search_cache.contains_key(&new) {
            self.restore_search_cache(new);
            return;
        }
        match new {
            None => {
                self.search_seq += 1;
                self.search_entity = None;
                self.search_page = 1;
                self.search_has_more = false;
                self.reset_search_rows();
                self.clear_detail("[dim]searching...[/]");
                self.load_search(false);
            }
            Some(e) => self.search_narrow_entity(e),
        }
    }

    fn search_narrow_entity(&mut self, entity: &'static str) {
        if !self.search_mode || self.search_term.is_empty() {
            return;
        }
        self.search_seq += 1;
        self.search_entity = Some(entity);
        self.search_page = 1;
        self.search_has_more = false;
        self.reset_search_rows();
        self.clear_detail("[dim]searching...[/]");
        self.load_search(false);
    }

    fn save_search_cache(&mut self) {
        if !self.items.is_empty() {
            self.search_cache.insert(
                self.search_entity,
                (
                    self.items.clone(),
                    self.search_totals.clone(),
                    self.search_page,
                    self.search_has_more,
                ),
            );
        }
    }

    fn search_total(&self, totals: &[(String, i64)], entity: Option<&str>) -> Option<i64> {
        entity.and_then(|e| totals.iter().find(|(k, _)| k == e).map(|(_, c)| *c))
    }

    fn search_row(idx: usize, item: &Item) -> Row {
        let cells = vec![
            TableCell::plain(format!("{:03}", idx + 1)),
            TableCell::plain(item.display_id()),
            TableCell::plain(
                item.get("status")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
            ),
            TableCell::plain(cut(
                &item
                    .get("subject")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
                80,
            )),
            TableCell::plain(
                item.get("_agent")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
            ),
            TableCell::plain(
                item.get("_group")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
            ),
        ];
        Row {
            key: item.display_id(),
            cells: cells.into_iter().map(|c| (c, 0)).collect(),
        }
    }

    fn search_columns(&mut self) {
        self.table_columns = ["", "ID", "STATUS", "SUBJECT", "AGENT", "GROUP"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    }

    fn restore_search_cache(&mut self, entity: Option<&'static str>) {
        let Some((items, totals, page, more)) = self.search_cache.get(&entity).cloned() else {
            return;
        };
        self.search_seq += 1;
        self.search_entity = entity;
        self.search_page = page;
        self.search_has_more = more;
        self.items = items;
        self.total = self
            .search_total(&totals, entity)
            .unwrap_or_else(|| totals.iter().map(|(_, c)| c).sum());
        self.search_totals = totals;
        self.clear_table();
        self.search_columns();
        self.rows = self
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| App::search_row(i, it))
            .collect();
        if self.items.is_empty() {
            self.clear_detail("[dim]no results[/]");
        } else {
            self.move_cursor(0, true);
            self.clear_detail("[dim]loading details...[/]");
        }
    }

    fn load_search(&mut self, append: bool) {
        let seq = self.search_seq;
        let have: Vec<&'static str> = self.schemas.keys().copied().collect();
        let (term, entity, sort, page) = (
            self.search_term.clone(),
            self.search_entity,
            self.search_sort,
            self.search_page,
        );
        self.spawn(move |b| {
            let mut schemas = Vec::new();
            let outcome = (|| -> Result<SearchResult, String> {
                for r in ALL_RESOURCES {
                    if !have.contains(&r.name) {
                        schemas.push((r.name, b.load_schema(r).map_err(|e| e.to_string())?));
                    }
                }
                b.search_items(&term, entity, sort, page)
                    .map_err(|e| e.to_string())
            })();
            Msg::Search {
                seq,
                append,
                schemas,
                outcome,
            }
        });
    }

    fn populate_search(
        &mut self,
        items: Vec<Item>,
        totals: Vec<(String, i64)>,
        append: bool,
        seq: u64,
    ) {
        if seq != self.search_seq {
            return;
        }
        self.loading_more = false;
        match self.search_total(&totals, self.search_entity) {
            Some(t) => {
                self.total = t;
                self.search_has_more = items.len() == 30;
            }
            None => {
                self.total = totals.iter().map(|(_, c)| c).sum();
                self.search_has_more = false;
            }
        }
        self.search_totals = totals;
        let offset = if append {
            self.items.len()
        } else {
            self.clear_table();
            self.search_columns();
            self.items.clear();
            0
        };
        let had = !items.is_empty();
        for (i, it) in items.iter().enumerate() {
            let did = it.display_id();
            if self.rows.iter().any(|r| r.key == did) {
                continue;
            }
            self.rows.push(App::search_row(offset + i, it));
        }
        self.items.extend(items);
        if !append && had {
            self.clear_detail("[dim]loading details...[/]");
            self.move_cursor(0, true);
        } else if !append {
            self.clear_detail("[dim]no results[/]");
        }
    }

    fn search_load_more(&mut self) {
        if !self.search_has_more || self.loading_more {
            return;
        }
        self.loading_more = true;
        self.search_page += 1;
        self.load_search(true);
    }

    fn cycle_search_sort(&mut self) {
        self.search_seq += 1;
        self.search_cache.clear();
        let order = ["relevance", "created", "modified"];
        let idx = order
            .iter()
            .position(|o| *o == self.search_sort)
            .unwrap_or(0);
        self.search_sort = order[(idx + 1) % 3];
        self.search_page = 1;
        self.search_has_more = false;
        self.items.clear();
        self.total = 0;
        self.detail_cache.clear();
        self.sub_cache.clear();
        self.clear_table();
        self.clear_detail("[dim]searching...[/]");
        self.load_search(false);
    }

    // ── entities / views ────────────────────────────────────

    fn switch_entity(&mut self, name: &'static str) {
        if name == self.entity {
            return;
        }
        self.entity = name;
        self.detail_tab_idx = 0;
        self.list_epoch += 1;
        self.has_more = false;
        self.loading_more = false;
        self.active_filters = self.get_default_filters(name);
        self.filter_error = None;
        self.set_timer(Slot::Entity, ENTITY_DEBOUNCE, TimerKind::Entity);
    }

    /// `config.get_default_where(entity)`; the backend maps errors to `[]`.
    fn get_default_filters(&self, entity: &str) -> Vec<String> {
        self.backend.default_where(entity)
    }

    fn step_entity(&mut self, delta: isize) {
        let idx = ENTITY_TABS
            .iter()
            .position(|e| *e == self.entity)
            .unwrap_or(0) as isize;
        self.switch_entity(ENTITY_TABS[(idx + delta).rem_euclid(3) as usize]);
    }

    fn next_entity(&mut self) {
        self.step_entity(1);
    }

    fn prev_entity(&mut self) {
        self.step_entity(-1);
    }

    fn pick_view(&mut self) {
        if self.search_mode {
            return;
        }
        let res = resource_for(self.entity);
        let views = self.views.get(res.name).cloned().unwrap_or_default();
        if views.is_empty() {
            self.notify(
                format!("No saved views on this account for {}", res.name),
                Severity::Warning,
            );
            return;
        }
        let current = self.view.get(res.name);
        let options = views
            .iter()
            .enumerate()
            .map(|(i, v)| (Value::from(i), v.name.clone(), current == Some(v), v.system))
            .collect();
        let title = format!("{} views", capitalize(res.name));
        self.push(
            Modal::Picker(PickerModal::views(&title, options)),
            Cont::ViewPicked { res, views },
        );
    }

    // ── keys ────────────────────────────────────────────────

    pub fn press(&mut self, name: &str) {
        self.handle_key(&keys::parse(name));
    }

    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.handle_key(&KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
    }

    /// `FsvApp.on_key`: ctrl+c, then the open modal, the search or filter
    /// input, and finally the main-pane keys.
    pub fn handle_key(&mut self, ev: &KeyEvent) {
        let key = keys::name(ev);
        if key == "ctrl+c" {
            self.quit = true;
            return;
        }
        if self.form.is_some() && self.modals.is_empty() {
            if key == "ctrl+q" {
                self.quit = true;
                return;
            }
            if let Some(form) = &self.form {
                form.view.borrow_mut().handle_key(ev);
            }
            self.check_form_done();
            return;
        }
        if let Some((modal, _)) = self.modals.last_mut() {
            match modal.handle(ev) {
                Outcome::Stay => {}
                Outcome::Warn(m) => self.notify(m, Severity::Warning),
                Outcome::Close(result) => {
                    if let Some((_, cont)) = self.modals.pop() {
                        self.resume(cont, result);
                    }
                }
            }
            return;
        }
        match self.focus {
            Focus::Search => {
                if key == "escape" {
                    self.cancel_search_input();
                } else if self.search_input.handle(ev) == InputEvent::Submitted {
                    self.on_input_submitted(Focus::Search);
                }
                return;
            }
            Focus::Filter => {
                self.filter_key(ev, &key);
                return;
            }
            _ => {}
        }
        self.main_key(&key);
    }

    fn main_key(&mut self, key: &str) {
        let pane = self.active_pane();
        if let Some(pending) = self.g_pending.take() {
            if key == "g" {
                if pending == Pane::Detail {
                    self.scroll_detail_home();
                } else {
                    self.move_cursor(0, false);
                }
            } else if let Some(e) = entity_key(key) {
                if self.search_mode {
                    self.search_narrow_entity(e);
                } else {
                    self.switch_entity(e);
                }
            }
            return;
        }
        match key {
            "g" => self.g_pending = Some(pane),
            "f" => self.open_search_input(),
            "escape" if self.search_mode => self.exit_search_mode(),
            "s" if self.search_mode => self.cycle_search_sort(),
            "slash" => {
                if !self.search_mode {
                    self.open_filter_input();
                }
            }
            "tab" => self.toggle_pane(),
            "ctrl+j" => self.focus_pane(Pane::Detail),
            "ctrl+k" => self.focus_pane(Pane::List),
            "question_mark" => self.push(Modal::Help, Cont::None),
            "j" | "down" => {
                if pane == Pane::Detail {
                    self.detail_scroll(1)
                } else {
                    self.cursor_step(1)
                }
            }
            "k" | "up" => {
                if pane == Pane::Detail {
                    self.detail_scroll(-1)
                } else {
                    self.cursor_step(-1)
                }
            }
            "J" | "right_curly_bracket" => {
                for _ in 0..5 {
                    if pane == Pane::Detail {
                        self.detail_scroll(1)
                    } else {
                        self.cursor_step(1)
                    }
                }
            }
            "K" | "left_curly_bracket" => {
                for _ in 0..5 {
                    if pane == Pane::Detail {
                        self.detail_scroll(-1)
                    } else {
                        self.cursor_step(-1)
                    }
                }
            }
            "G" => {
                if pane == Pane::Detail {
                    self.detail.scroll = self.detail_max_scroll.get();
                } else if !self.rows.is_empty() {
                    let last = self.rows.len() - 1;
                    self.move_cursor(last, false);
                }
            }
            "h" | "left" => {
                if pane == Pane::Detail {
                    self.step_detail_tab(-1)
                } else if self.search_mode {
                    self.step_search_entity(-1)
                } else {
                    self.prev_entity()
                }
            }
            "l" | "right" => {
                if pane == Pane::Detail {
                    self.step_detail_tab(1)
                } else if self.search_mode {
                    self.step_search_entity(1)
                } else {
                    self.next_entity()
                }
            }
            "o" => self.open_browser(),
            "a" => self.action_menu(),
            "exclamation_mark" => {
                if let Some(m) = self.write_failure.clone() {
                    self.push(Modal::failure(&m), Cont::None);
                }
            }
            "s" | "n" => self.write_key(key),
            "r" if pane == Pane::Detail && self.reply_possible() => self.write_key(key),
            "r" => {
                if self.search_mode {
                    let t = self.search_term.clone();
                    self.apply_search(&t);
                } else {
                    self.reload_list("[dim]reloading...[/]");
                }
            }
            "y" => self.yank(),
            "q" => self.quit = true,
            "left_square_bracket" => self.resize_panes(-1),
            "right_square_bracket" => self.resize_panes(1),
            "v" => self.pick_view(),
            "N" => self.open_form("new"),
            "e" => self.open_form("edit"),
            d if d.len() == 1 && matches!(d.as_bytes()[0], b'1'..=b'9') => {
                let idx = (d.as_bytes()[0] - b'1') as usize;
                if let Some(sel) = &self.selected
                    && idx < self.tab_names(Some(sel.res)).len()
                {
                    self.set_detail_tab(idx);
                }
            }
            "pagedown" | "pageup" | "home" | "end" => {
                let page = self.table_height.get().max(1) as isize;
                if pane == Pane::Detail {
                    match key {
                        "pagedown" => self.detail_scroll(page as i32),
                        "pageup" => self.detail_scroll(-(page as i32)),
                        "home" => self.detail.scroll = 0,
                        _ => self.detail.scroll = self.detail_max_scroll.get(),
                    }
                } else {
                    match key {
                        "pagedown" => self.cursor_step(page),
                        "pageup" => self.cursor_step(-page),
                        "home" => self.move_cursor(0, false),
                        _ => {
                            let last = self.rows.len().saturating_sub(1);
                            self.move_cursor(last, false)
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn resize_panes(&mut self, delta: i32) {
        self.list_fr = (self.list_fr as i32 + delta).clamp(3, 17) as u16;
        self.detail_fr = (self.detail_fr as i32 - delta).clamp(3, 17) as u16;
    }

    fn open_browser(&mut self) {
        let Some(sel) = self.selected.clone() else {
            self.notify("No row selected", Severity::Warning);
            return;
        };
        let url = self.backend.item_url(sel.res, &sel.id());
        if !self.backend.open_url(&url) {
            self.notify("Browser open failed", Severity::Warning);
        }
    }

    fn yank(&mut self) {
        let Some(sel) = self.selected.clone() else {
            self.notify("No row selected", Severity::Warning);
            return;
        };
        let did = sel.display_id();
        match self.backend.copy_to_clipboard(&did) {
            Ok(()) => self.notify(format!("Copied {did}"), Severity::Information),
            Err(e) => self.notify(format!("Copy failed: {e}"), Severity::Warning),
        }
    }

    // ── modal continuations ─────────────────────────────────

    fn resume(&mut self, cont: Cont, result: ModalResult) {
        use ModalResult as R;
        match (cont, result) {
            (Cont::None, _) => {}
            (Cont::ViewPicked { res, views }, R::Choice(i)) => {
                let Some(view) = i.as_u64().and_then(|i| views.get(i as usize)).cloned() else {
                    return;
                };
                let msg = format!("[dim]loading {}...[/]", escape(&view.name));
                self.view.insert(res.name, view);
                self.active_filters.clear();
                self.reload_list(&msg);
            }
            (Cont::StatusPicked { item }, R::Status(id, label)) => {
                let schema = self
                    .schemas
                    .get(item.res.name)
                    .cloned()
                    .unwrap_or_else(|| json!({"fields": []}));
                let plan = actions::status_plan(item.res, &item, &schema, &id, &label);
                self.push(
                    Modal::confirm(&plan.title, plan.lines),
                    Cont::StatusConfirmed { item, id, label },
                );
            }
            (Cont::StatusConfirmed { item, id, label }, R::Bool(true)) => {
                self.do_write(WriteKind::Status(id, label), item)
            }
            (Cont::NoteComposed { item, reply }, R::NoteOptions(body, public))
                if item.res.name == "tickets" =>
            {
                self.ticket_conversation_options(item, body, public, reply, Default::default())
            }
            (Cont::NoteComposed { item, reply }, R::Note(body, public)) => {
                if item.res.name == "tickets" && reply {
                    self.prepare_ticket_conversation(item, body, public, reply, Default::default());
                    return;
                }
                if item.res.name == "tickets" && !reply {
                    self.review_ticket_note(item, body, public);
                    return;
                }
                let plan = actions::note_plan(public, reply);
                let mut lines = plan.lines.clone();
                lines.push(String::new());
                lines.push(format!("> {body}"));
                self.push(
                    Modal::confirm(&plan.title, lines),
                    Cont::NoteConfirmed {
                        item,
                        body,
                        public,
                        reply,
                    },
                );
            }
            (
                Cont::NoteConfirmed {
                    item,
                    body,
                    public,
                    reply,
                },
                R::Bool(true),
            ) => self.do_write(
                if reply {
                    WriteKind::Reply(body)
                } else {
                    WriteKind::Note(body, public)
                },
                item,
            ),
            (Cont::TicketNoteReviewed { plan }, R::Bool(true)) => {
                if plan.public {
                    let lines = vec![
                        "This is a public note visible to the customer, not a reply.".to_string(),
                        "No explicit notification recipients. Tenant workflows may still notify people.".to_string(),
                    ];
                    self.push(
                        Modal::confirm("Confirm PUBLIC visibility", lines),
                        Cont::TicketNotePublic { plan },
                    );
                } else {
                    self.send_ticket_note(plan, false);
                }
            }
            (Cont::TicketNotePublic { plan }, R::Bool(true)) => self.send_ticket_note(plan, true),
            (Cont::ActionMenu { item }, R::Choice(kind)) => {
                self.prepare_action(&fsv_py::py_str(&kind), item)
            }
            (Cont::LinkQuery { item }, R::Text(q)) => {
                self.load_action_targets("link".into(), item, q)
            }
            (Cont::TaskTitle { item }, R::Text(title)) => self.task_titled(item, title),
            (Cont::TaskOwnerMode { item, title }, R::Choice(mode)) => {
                if self.read_only {
                    return;
                }
                if mode == "none" {
                    self.change_task_options(item, title, None, Vec::new());
                } else {
                    self.push(
                        Modal::value(
                            "Explicit task owner user ID or exact email (no implicit self)",
                            "",
                        ),
                        Cont::TaskOwner { item, title },
                    );
                }
            }
            (Cont::TaskOwner { item, title }, R::Text(owner)) => {
                self.change_task_options(item, title, Some(owner), Vec::new())
            }
            (
                Cont::TaskOptions {
                    item,
                    title,
                    owner,
                    values,
                },
                R::Choice(key),
            ) => {
                if self.read_only {
                    return;
                }
                let key = fsv_py::py_str(&key);
                match key.as_str() {
                    "review" => self.prepare_change_task(item, title, owner, values),
                    "clear" => self.change_task_options(item, title, owner, Vec::new()),
                    "status" | "notify_before" => {
                        let (labels, t) = if key == "status" {
                            (TASK_STATUS_LABELS, "Task status")
                        } else {
                            (TASK_NOTIFY_BEFORE, "Notify Before")
                        };
                        let choices = labels.iter().map(|l| (json!(l), l.to_string())).collect();
                        self.push(
                            Modal::choices(t, choices),
                            Cont::TaskOptionValue {
                                item,
                                title,
                                owner,
                                values,
                                key,
                            },
                        );
                    }
                    _ => {
                        let label = TASK_FIELDS
                            .iter()
                            .find(|(k, _)| *k == key)
                            .map_or("", |(_, l)| l);
                        let cur = values
                            .iter()
                            .find(|(k, _)| *k == key)
                            .map_or("", |(_, v)| v.as_str())
                            .to_string();
                        self.push(
                            Modal::value(label, &cur),
                            Cont::TaskOptionValue {
                                item,
                                title,
                                owner,
                                values,
                                key,
                            },
                        );
                    }
                }
            }
            (
                Cont::TaskOptionValue {
                    item,
                    title,
                    owner,
                    mut values,
                    key,
                },
                R::Text(v) | R::Choice(Value::String(v)),
            ) => {
                if self.read_only {
                    return;
                }
                match values.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = v,
                    None => values.push((key, v)),
                }
                self.change_task_options(item, title, owner, values);
            }
            (Cont::TaskReviewed { item, plan }, R::Bool(true)) => self.send_change_task(item, plan),
            (Cont::ActionTarget { kind, item }, R::Choice(target)) => {
                if self.read_only {
                    return;
                }
                match kind.as_str() {
                    "task-update" => self.load_task_editor(item, target),
                    "download" => self.push(
                        Modal::value("Download destination directory", "."),
                        Cont::DownloadDest { item, target },
                    ),
                    _ => self.confirm_action(kind, item, target, Map::new()),
                }
            }
            (
                Cont::TaskField {
                    item,
                    target,
                    current,
                    fields,
                },
                R::Choice(field),
            ) => {
                let field = fsv_py::py_str(&field);
                let title = format!("Task {}: {field}", fget(&target, "id", ""));
                let cur = current
                    .get(&field)
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default();
                self.push(
                    Modal::value(&title, &cur),
                    Cont::TaskFieldValue {
                        item,
                        target,
                        current,
                        fields,
                        field,
                    },
                );
            }
            (
                Cont::TaskFieldValue {
                    item,
                    target,
                    current,
                    fields,
                    field,
                },
                R::Text(value),
            ) => match actions::task_patch(&current, &field, &value, &fields) {
                Err(e) => self.show_failure(e),
                Ok(body) if body.is_empty() => {
                    self.notify("No change; nothing sent", Severity::Information)
                }
                Ok(body) => {
                    let before: Map<String, Value> = body
                        .keys()
                        .map(|k| (k.clone(), current.get(k).cloned().unwrap_or(Value::Null)))
                        .collect();
                    let mut t = target;
                    if let Value::Object(o) = &mut t {
                        o.insert("_before".into(), Value::Object(before));
                    }
                    self.confirm_action("task-update".into(), item, t, body);
                }
            },
            (Cont::DownloadDest { item, target }, R::Text(path)) => {
                self.prepare_download(item, target, path, None)
            }
            (
                Cont::DownloadCollision {
                    item,
                    target,
                    directory,
                    name,
                },
                R::Choice(action),
            ) => {
                if action == "rename" && !self.read_only {
                    self.push(
                        Modal::value("Save as filename (no directories)", &name),
                        Cont::DownloadRename {
                            item,
                            target,
                            directory,
                        },
                    );
                }
            }
            (
                Cont::DownloadRename {
                    item,
                    target,
                    directory,
                },
                R::Text(name),
            ) => self.prepare_download(item, target, directory, Some(name)),
            (Cont::ApprovalName { item }, R::Text(name)) => {
                if self.read_only {
                    return;
                }
                self.push(
                    Modal::value("Approval message", ""),
                    Cont::ApprovalMessage { item, name },
                );
            }
            (Cont::ApprovalMessage { item, name }, R::Text(message)) => {
                if self.read_only {
                    return;
                }
                self.push(
                    Modal::value("Explicit approver IDs or exact emails, comma separated", ""),
                    Cont::ApprovalRecipients {
                        item,
                        name,
                        message,
                    },
                );
            }
            (
                Cont::ApprovalRecipients {
                    item,
                    name,
                    message,
                },
                R::Text(recipients),
            ) => {
                if self.read_only {
                    return;
                }
                let rules = site::approval_rules()
                    .into_iter()
                    .map(|(k, l)| (json!(k), l.to_string()))
                    .collect();
                self.push(
                    Modal::choices("Approval rule", rules),
                    Cont::ApprovalRule {
                        item,
                        name,
                        message,
                        recipients,
                    },
                );
            }
            (
                Cont::ApprovalRule {
                    item,
                    name,
                    message,
                    recipients,
                },
                R::Choice(rule),
            ) => self.prepare_approval_request(
                item,
                name,
                message,
                recipients,
                fsv_py::py_str(&rule),
            ),
            (Cont::ApprovalTarget { kind, item }, R::Choice(row)) => {
                if self.read_only {
                    return;
                }
                match kind.as_str() {
                    "approval-groups" => self.push(
                        Modal::outcome("Approval group details (read only)", &json_pretty(&row)),
                        Cont::None,
                    ),
                    "approval-cancel" => self.prepare_approval_action(item, row, "cancel", None),
                    _ => {
                        let ops = vec![
                            (json!("approve"), "Approve".to_string()),
                            (json!("reject"), "Reject".to_string()),
                        ];
                        self.push(
                            Modal::choices("Approval response", ops),
                            Cont::ApprovalOp { item, row },
                        );
                    }
                }
            }
            (Cont::ApprovalOp { item, row }, R::Choice(op)) => {
                if self.read_only {
                    return;
                }
                let op = fsv_py::py_str(&op);
                self.push(
                    Modal::value_with(
                        "Remark (required for reject; optional for approve)",
                        "",
                        op == "approve",
                    ),
                    Cont::ApprovalRemark { item, row, op },
                );
            }
            (Cont::ApprovalRemark { item, row, op }, R::Text(remark)) => {
                let remark = (!remark.is_empty()).then_some(remark);
                self.prepare_approval_action(item, row, &op, remark.as_deref());
            }
            (Cont::ApprovalReviewed { item, plan }, R::Bool(true)) => {
                self.send_approval(item, plan)
            }
            (
                Cont::ActionConfirmed {
                    kind,
                    item,
                    target,
                    body,
                    plan,
                },
                R::Bool(true),
            ) => self.execute_action(kind, item, target, body, plan),
            (Cont::CreateChoice, R::Choice(kind)) => self.create_choice(&fsv_py::py_str(&kind)),
            (Cont::CatalogCategory { items, categories }, R::Choice(category)) => {
                if self.read_only {
                    return;
                }
                let ids: Option<Vec<Value>> = if category == "all" {
                    None
                } else {
                    Some(
                        self.backend
                            .resolve_category(&categories, &fsv_py::py_str(&category))
                            .iter()
                            .filter_map(|c| c.get("id").cloned())
                            .collect(),
                    )
                };
                let rows = self.backend.filter_catalog_items(&items, ids.as_deref());
                let name_of = |cid: Option<&Value>| {
                    categories
                        .iter()
                        .find(|c| c.get("id") == cid)
                        .map_or(String::new(), |c| fget(c, "name", ""))
                };
                let choices = rows
                    .iter()
                    .filter(|r| r.get("display_id").is_some_and(|d| !d.is_null()))
                    .map(|r| {
                        (
                            r.clone(),
                            format!(
                                "#{} {} · {}",
                                fget(r, "display_id", "None"),
                                fget(r, "name", ""),
                                name_of(r.get("category_id"))
                            ),
                        )
                    })
                    .collect();
                self.push(
                    Modal::choices("Service catalog — search items", choices),
                    Cont::CatalogItem,
                );
            }
            (Cont::CatalogItem, R::Choice(row)) => self.launch_create("sr", Some(row)),
            (Cont::Reviewed(step), result) => self.resume_reviewed(step, result),
            _ => {}
        }
    }

    // ── write actions ───────────────────────────────────────

    fn reply_possible(&self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|s| !self.read_only && actions::can_reply(s.res))
    }

    fn write_key(&mut self, key: &str) {
        if self.read_only {
            self.notify(
                "Read-only mode: write actions are disabled",
                Severity::Warning,
            );
            return;
        }
        let Some(sel) = self.selected.clone() else {
            self.notify("No row selected", Severity::Warning);
            return;
        };
        match key {
            "s" => self.start_status(sel),
            "n" => self.start_note(sel, false),
            _ => self.start_note(sel, true),
        }
    }

    fn start_status(&mut self, item: Item) {
        let res = item.res;
        if !actions::can_change_status(res) {
            self.notify(
                format!("Status change is not available for {}", res.name),
                Severity::Warning,
            );
            return;
        }
        let schema = self
            .schemas
            .get(res.name)
            .cloned()
            .unwrap_or_else(|| json!({"fields": []}));
        let choices = actions::status_choices(&schema);
        if choices.is_empty() {
            self.notify("No status choices in schema", Severity::Warning);
            return;
        }
        let current = resolve_status(&item.data, &schema);
        self.push(
            Modal::Status(StatusModal::new(choices, &current)),
            Cont::StatusPicked { item },
        );
    }

    fn start_note(&mut self, item: Item, reply: bool) {
        let ok = if reply {
            actions::can_reply(item.res)
        } else {
            actions::can_note(item.res)
        };
        if !ok {
            self.notify("Not available for this item type", Severity::Warning);
            return;
        }
        let modal = NoteModal::new(&self.target_label(&item), reply, true)
            .with_ticket_options(item.res.name == "tickets");
        self.push(Modal::Note(modal), Cont::NoteComposed { item, reply });
    }

    fn target_label(&self, item: &Item) -> String {
        item.display_id()
    }

    fn review_ticket_note(&mut self, item: Item, body: String, public: bool) {
        if self.read_only {
            return;
        }
        let plan = match self.backend.ticket_note_plan(&item.ref_id(), &body, public) {
            Ok(p) => p,
            Err(e) => {
                self.show_failure(e.to_string());
                return;
            }
        };
        let review = Dumps::default()
            .ensure_ascii(false)
            .indent(2)
            .dump(&plan.review);
        self.push(
            Modal::confirm("Review exact ticket note", vec![review]),
            Cont::TicketNoteReviewed { plan },
        );
    }

    fn send_ticket_note(&mut self, plan: ReviewPlan, public_confirmed: bool) {
        if self.read_only {
            return;
        }
        let read_only = self.read_only;
        self.spawn(move |b| {
            let ui = match b.send_ticket_note(&plan, public_confirmed, read_only) {
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
                Ok(r) if !r.get("verified").is_some_and(fsv_py::truthy) => Ui::Failure(format!(
                    "Note ID {} submitted; readback unverified.\n{}\nDo not resend.",
                    fget(&r, "note_id", "None"),
                    fget(&r, "warning", "")
                )),
                Ok(r) => Ui::TicketNoteVerified {
                    ticket_id: plan.target_id.clone(),
                    note_id: r.get("note_id").cloned().unwrap_or(Value::Null),
                },
            };
            Msg::Ui(vec![ui])
        });
    }

    fn do_write(&mut self, kind: WriteKind, item: Item) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            let res = item.res;
            let id = item.id();
            let outcome = match &kind {
                WriteKind::Status(new_id, label) => actions::apply_status(b, res, &id, new_id)
                    .map(|_| format!("Status set to {label}")),
                WriteKind::Reply(body) => {
                    actions::apply_reply(b, res, &id, body).map(|_| "Reply sent".to_string())
                }
                WriteKind::Note(body, public) => actions::apply_note(b, res, &id, body, *public)
                    .map(|_| {
                        if *public {
                            "Public note added".into()
                        } else {
                            "Private note added".into()
                        }
                    }),
            };
            Msg::Ui(vec![match outcome {
                Ok(msg) => Ui::AfterWrite(res, item, msg),
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
            }])
        });
    }

    fn after_write(&mut self, res: Res, item: Item, msg: &str) {
        self.notify(msg.to_string(), Severity::Information);
        let prefix = format!("{}:{}", res.name, item.id_str());
        self.detail_cache.remove(&prefix);
        self.sub_cache
            .retain(|k, _| !k.starts_with(&format!("{prefix}:")));
        if let Some(sel) = &self.selected
            && sel.id() == item.id()
            && sel.res.name == res.name
        {
            let key = item.key();
            self.pending_detail_key = Some(key.clone());
            self.load_detail(item, key);
        }
    }

    fn action_menu(&mut self) {
        if self.read_only {
            self.notify("Read-only mode: actions are disabled", Severity::Warning);
            return;
        }
        let Some(item) = self.selected.clone() else {
            self.notify("No row selected", Severity::Warning);
            return;
        };
        let mut choices: Vec<(Value, String)> = Vec::new();
        if item.res.name == "changes" {
            for (k, l) in [
                ("task-add", "Add task"),
                ("task-update", "Update selected task"),
                ("task-delete", "Delete selected task"),
                ("link", "Link initiating ticket"),
                ("unlink", "Unlink initiating ticket"),
                ("approval-request", "Request approval"),
                ("approval-respond", "Respond to approval"),
                ("approval-cancel", "Cancel approval group"),
                ("approval-groups", "List approval groups"),
            ] {
                choices.push((json!(k), l.to_string()));
            }
        }
        choices.push((json!("download"), "Download selected attachment".into()));
        if matches!(item.res.name, "changes" | "problems") {
            choices.push((
                json!("planning-text"),
                "Edit native planning/analysis text".into(),
            ));
        }
        if item.res.name == "problems" {
            choices.push((json!("known-error"), "Mark as known error".into()));
        }
        self.push(
            Modal::choices("Selected record actions", choices),
            Cont::ActionMenu { item },
        );
    }

    fn prepare_action(&mut self, kind: &str, item: Item) {
        if self.read_only {
            return;
        }
        match kind {
            "known-error" => self.prepare_known_error(item),
            "planning-text" => self.load_planning_sections(item),
            "task-add" => self.compose_change_task(item),
            "approval-request" => self.compose_approval_request(item),
            "approval-respond" | "approval-cancel" | "approval-groups" => {
                self.load_approval_targets(kind.to_string(), item)
            }
            "link" => self.push(
                Modal::value("Search initiating tickets (ID or subject)", ""),
                Cont::LinkQuery { item },
            ),
            _ => self.load_action_targets(kind.to_string(), item, String::new()),
        }
    }

    /// Title → owner mode → optional fields (see `task_titled`).
    fn compose_change_task(&mut self, item: Item) {
        if self.read_only || item.res.name != "changes" {
            return;
        }
        self.push(
            Modal::value("New change task title — Open; reminder Never", ""),
            Cont::TaskTitle { item },
        );
    }

    fn task_titled(&mut self, item: Item, title: String) {
        if self.read_only {
            return;
        }
        let choices = vec![
            (json!("none"), "No owner (default)".to_string()),
            (
                json!("explicit"),
                "Explicit owner user ID or email".to_string(),
            ),
        ];
        self.push(
            Modal::choices("Task owner", choices),
            Cont::TaskOwnerMode { item, title },
        );
    }

    fn review_change_task(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        let review = fsv_py::pretty(&plan.review);
        self.push(
            Modal::confirm("Review exact new change task", vec![review]),
            Cont::TaskReviewed { item, plan },
        );
    }

    /// Name → message → recipients → rule, then `prepare_approval_request`.
    fn compose_approval_request(&mut self, item: Item) {
        self.push(
            Modal::value("Approval group name", ""),
            Cont::ApprovalName { item },
        );
    }

    fn approval_result(&mut self, item: Item, result: Value) {
        self.after_write(
            &CHANGES,
            item,
            "Approval submitted; inspect verification details",
        );
        self.push(
            Modal::outcome("Approval result — do not resend", &json_pretty(&result)),
            Cont::None,
        );
    }

    fn prepare_approval_request(
        &mut self,
        item: Item,
        name: String,
        message: String,
        recipients: String,
        rule: String,
    ) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            let values: Vec<String> = recipients
                .split(',')
                .map(|v| v.trim().to_string())
                .collect();
            Msg::Ui(vec![match b.approval_request_plan(
                &item.ref_id(),
                &name,
                &message,
                &values,
                &rule,
            ) {
                Ok(plan) => Ui::ApprovalReview {
                    item,
                    plan,
                    before: None,
                },
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
            }])
        });
    }

    fn load_approval_targets(&mut self, kind: String, item: Item) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            Msg::Ui(vec![match b.approval_targets(&kind, &item.ref_id()) {
                Ok(rows) => Ui::ApprovalTargets { kind, item, rows },
                Err(e) => Ui::Failure(format!("Approval inventory unavailable: {e}")),
            }])
        });
    }

    fn pick_approval_target(&mut self, kind: String, item: Item, rows: Vec<Value>) {
        if self.read_only {
            return;
        }
        if rows.is_empty() {
            self.notify("No approval records or groups", Severity::Warning);
            return;
        }
        let choices = rows
            .into_iter()
            .map(|r| {
                let label = json_line(&r);
                (r, label)
            })
            .collect();
        let title = if kind == "approval-respond" {
            "Approval records"
        } else {
            "Approval groups"
        };
        self.push(
            Modal::choices(title, choices),
            Cont::ApprovalTarget { kind, item },
        );
    }

    fn prepare_approval_action(&mut self, item: Item, row: Value, op: &str, remark: Option<&str>) {
        if self.read_only {
            return;
        }
        let rid = row.get("id").cloned().unwrap_or(Value::Null);
        match self
            .backend
            .approval_action_plan(&item.ref_id(), &rid, op, remark)
        {
            Ok(plan) => self.review_approval(item, plan, Some(row)),
            Err(e) => self.show_failure(actions::failure_detail(&e)),
        }
    }

    fn review_approval(&mut self, item: Item, plan: ReviewPlan, before: Option<Value>) {
        if self.read_only {
            return;
        }
        let mut lines = vec![json_pretty(&plan.review)];
        if let Some(b) = &before {
            lines.push(format!("Selected inventory row: {}", json_pretty(b)));
        }
        self.push(
            Modal::confirm("Review exact approval action", lines),
            Cont::ApprovalReviewed { item, plan },
        );
    }

    fn send_approval(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        let read_only = self.read_only;
        self.spawn(move |b| {
            Msg::Ui(vec![match b.send_approval(&plan, read_only) {
                Ok(r) if r.get("verified").is_some_and(fsv_py::truthy) => {
                    Ui::ApprovalResult { item, result: r }
                }
                Ok(_) => Ui::Failure(actions::failure_detail(&Failure::value(
                    "Approval outcome unverified; do not resend; inspect the record",
                ))),
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
            }])
        });
    }

    /// Resolve collisions before review; never overwrite an existing file.
    fn prepare_download(
        &mut self,
        item: Item,
        target: Value,
        directory: String,
        name: Option<String>,
    ) {
        if self.read_only {
            return;
        }
        let name = name.unwrap_or_else(|| match target.get("name").filter(|v| fsv_py::truthy(v)) {
            Some(n) => fsv_py::py_str(n),
            None => format!("attachment-{}", fget(&target, "id", "")),
        });
        let path = match actions::safe_target(&directory, &name) {
            Ok(p) => p,
            Err(e) => {
                self.show_failure(e);
                return;
            }
        };
        if path.exists() {
            let choices = vec![
                (json!("skip"), "Skip download".to_string()),
                (json!("rename"), "Choose another filename".to_string()),
            ];
            self.push(
                Modal::choices(
                    &format!("File exists: {} — no overwrite", path.display()),
                    choices,
                ),
                Cont::DownloadCollision {
                    item,
                    target,
                    directory,
                    name,
                },
            );
            return;
        }
        let mut target = target;
        if let Value::Object(o) = &mut target {
            o.insert("name".into(), Value::String(name));
        }
        let mut body = Map::new();
        body.insert("destination".into(), Value::String(directory));
        self.confirm_action("download".into(), item, target, body);
    }

    fn change_task_options(
        &mut self,
        item: Item,
        title: String,
        owner: Option<String>,
        values: Vec<(String, String)>,
    ) {
        if self.read_only {
            return;
        }
        let mut choices = vec![(json!("review"), "Review exact task".to_string())];
        for (k, label) in TASK_FIELDS {
            let v = values
                .iter()
                .find(|(key, _)| key == k)
                .map_or("default / omitted", |(_, v)| v.as_str());
            choices.push((json!(k), format!("{label}: {v}")));
        }
        choices.push((json!("clear"), "Clear optional fields".into()));
        self.push(
            Modal::choices(
                "Task optional fields — defaults Open / Never; others omitted",
                choices,
            ),
            Cont::TaskOptions {
                item,
                title,
                owner,
                values,
            },
        );
    }

    fn prepare_change_task(
        &mut self,
        item: Item,
        title: String,
        owner: Option<String>,
        values: Vec<(String, String)>,
    ) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            let ui = match b.change_task_plan(&item.ref_id(), &title, owner.as_deref(), &values) {
                Ok(plan) => Ui::TaskPlan(item, plan),
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
            };
            Msg::Ui(vec![ui])
        });
    }

    fn send_change_task(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        let read_only = self.read_only;
        self.spawn(move |b| {
            let ui = match b.send_change_task(&plan, read_only) {
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
                Ok(r) if !r.get("verified").is_some_and(fsv_py::truthy) || r.get("unverified_fields").is_some_and(fsv_py::truthy) => Ui::Failure(format!(
                    "Task ID {} submitted; readback incomplete or unverified.\n{}\nDo not resend.",
                    fget(&r, "task_id", "None"),
                    fget(&r, "warning", "")
                )),
                Ok(r) => Ui::AfterWrite(&CHANGES, item, format!("Task {} verified", fget(&r, "task_id", "None"))),
            };
            Msg::Ui(vec![ui])
        });
    }

    fn load_action_targets(&mut self, kind: String, item: Item, query: String) {
        self.spawn(move |b| {
            let outcome = (|| -> Result<(Vec<Value>, Vec<String>), String> {
                match kind.as_str() {
                    "download" => actions::download_choices(b, &item).map_err(|e| e.to_string()),
                    "link" => b
                        .search_change_tickets(&query)
                        .map(|r| (r, Vec::new()))
                        .map_err(|e| e.to_string()),
                    k => {
                        let tasks = k.starts_with("task-");
                        let tab = b
                            .load_tab(
                                &CHANGES,
                                &item.data,
                                if tasks { "tasks" } else { "associations" },
                            )
                            .map_err(|e| e.to_string())?;
                        let (rows, warnings, _) = actions::target_inventory(&tab, tasks)?;
                        Ok((rows, warnings))
                    }
                }
            })();
            Msg::Ui(vec![match outcome {
                Ok((rows, warnings)) => Ui::Targets {
                    kind,
                    item,
                    rows,
                    warnings,
                },
                Err(e) => Ui::Failure(format!("Could not load action targets: {e}")),
            }])
        });
    }

    fn pick_action_target(
        &mut self,
        kind: String,
        item: Item,
        rows: Vec<Value>,
        warnings: Vec<String>,
    ) {
        if self.read_only {
            return;
        }
        if rows.is_empty() {
            self.show_failure(format!("No available targets.\n{}", warnings.join("\n")));
            return;
        }
        let mut choices = Vec::new();
        for row in rows {
            let mut row = row;
            if !warnings.is_empty()
                && let Value::Object(o) = &mut row
            {
                o.insert("_inventory_warnings".into(), json!(warnings));
            }
            let identity = first_of(&row, &["human_display_id", "display_id"])
                .or_else(|| row.get("id").cloned())
                .unwrap_or(Value::Null);
            let label = first_of(&row, &["title", "subject", "name", "filename"])
                .map_or("attachment".into(), |v| fsv_py::py_str(&v));
            let text = format!(
                "{}: {label} {}",
                fsv_py::py_str(&identity),
                fget(&row, "source", "")
            );
            choices.push((row, text));
        }
        let title = if warnings.is_empty() {
            kind.clone()
        } else {
            format!("{kind} — partial inventory; inspect warnings")
        };
        if !warnings.is_empty() {
            self.notify_full(warnings.join("\n"), Severity::Warning, None, 15);
        }
        self.push(
            Modal::choices(&title, choices),
            Cont::ActionTarget { kind, item },
        );
    }

    fn load_task_editor(&mut self, item: Item, target: Value) {
        self.spawn(move |b| {
            let tid = target.get("id").cloned().unwrap_or(Value::Null);
            Msg::Ui(vec![match b.task_for_edit(&item.ref_id(), &tid) {
                Ok((current, fields)) => Ui::TaskEditor {
                    item,
                    target,
                    current,
                    fields,
                },
                Err(e) => Ui::Failure(format!("Task unavailable: {e}")),
            }])
        });
    }

    fn edit_task_field(&mut self, item: Item, target: Value, current: Value, fields: Vec<Value>) {
        let choices = [
            ("title", "Title"),
            ("status", "Status"),
            ("group_id", "Group ID"),
            ("owner_id", "Agent ID"),
        ]
        .iter()
        .map(|(k, l)| (json!(k), l.to_string()))
        .collect();
        let title = format!(
            "Update task {} — choose one changed field",
            fget(&target, "id", "")
        );
        self.push(
            Modal::choices(&title, choices),
            Cont::TaskField {
                item,
                target,
                current,
                fields,
            },
        );
    }

    fn confirm_action(
        &mut self,
        kind: String,
        item: Item,
        target: Value,
        body: Map<String, Value>,
    ) {
        if self.read_only {
            return;
        }
        let tlabel = first_of(&target, &["title", "subject"])
            .map_or_else(|| fget(&target, "name", ""), |v| fsv_py::py_str(&v));
        let tid = first_of(&target, &["human_display_id"])
            .or_else(|| target.get("id").cloned())
            .unwrap_or(Value::Null);
        let safety = if kind.starts_with("task-") {
            "Assignment/status changes may notify people. Deletion cannot be undone.".to_string()
        } else if kind == "link" || kind == "unlink" {
            "Initiating-ticket relationship only.".to_string()
        } else {
            format!(
                "Source: {}. Existing files are skipped; no overwrite.",
                fget(&target, "source", "request")
            )
        };
        let mut lines = vec![
            format!("Record: {} (ID {})", item.display_id(), item.id_str()),
            format!("Target: {} — {tlabel}", fsv_py::py_str(&tid)),
            format!("Action: {kind}"),
            json_line(&Value::Object(body.clone())),
            safety,
        ];
        if let Some(before) = target.get("_before") {
            lines.insert(3, format!("Before: {}", json_line(before)));
        }
        let mut plan = None;
        if kind == "link" || kind == "unlink" {
            let tid = target.get("id").cloned().unwrap_or(Value::Null);
            match self
                .backend
                .association_plan(&item.ref_id(), &[tid], kind == "unlink")
            {
                Ok(p) => {
                    lines.push(fsv_py::pretty(&p.review));
                    plan = Some(p);
                }
                Err(e) => {
                    self.show_failure(e.to_string());
                    return;
                }
            }
        }
        if let Some(w) = target.get("_inventory_warnings").and_then(Value::as_array) {
            lines.extend(w.iter().map(fsv_py::py_str));
        }
        self.push(
            Modal::confirm(&format!("Confirm {kind}"), lines),
            Cont::ActionConfirmed {
                kind,
                item,
                target,
                body,
                plan,
            },
        );
    }

    fn execute_action(
        &mut self,
        kind: String,
        item: Item,
        target: Value,
        body: Map<String, Value>,
        plan: Option<ReviewPlan>,
    ) {
        if self.read_only {
            return;
        }
        self.spawn(move |b| {
            Msg::Ui(vec![execute_action_job(
                b, &kind, item, &target, &body, plan,
            )])
        });
    }

    // ── forms ───────────────────────────────────────────────

    fn open_form(&mut self, mode: &str) {
        if self.read_only {
            self.notify(
                "Read-only mode: write actions are disabled",
                Severity::Warning,
            );
            return;
        }
        if mode == "new" {
            let choices = vec![
                (json!("change"), "Change".to_string()),
                (json!("incident"), "Incident".to_string()),
                (json!("sr"), "Service Request".to_string()),
            ];
            self.push(Modal::choices("Create", choices), Cont::CreateChoice);
            return;
        }
        if let Some(item) = self.selected.clone().filter(|s| s.res.name == "tickets") {
            self.load_ticket_property_fields(item);
            return;
        }
        if self
            .selected
            .as_ref()
            .is_none_or(|s| s.res.name != "changes")
        {
            self.notify("Select a change to edit", Severity::Warning);
            return;
        }
        if !self.backend.forms_available() {
            self.notify("form UI not available", Severity::Warning);
            return;
        }
        self.notify_full("opening the form…".into(), Severity::Information, None, 2);
        let item = self.selected.clone();
        self.build_form(mode, item.as_ref().map(|i| i.data.clone()), item);
    }

    fn create_choice(&mut self, kind: &str) {
        if self.read_only {
            return;
        }
        if kind == "sr" {
            self.load_catalog();
        } else {
            self.launch_create(kind, None);
        }
    }

    fn launch_create(&mut self, kind: &str, item: Option<Value>) {
        if self.read_only {
            return;
        }
        if !self.backend.forms_available() {
            self.notify("form UI not available", Severity::Warning);
            return;
        }
        let mode = if kind == "change" { "new" } else { kind };
        self.build_form(mode, item, None);
    }

    /// `_build_form`: open the session on a worker; `edit` starts from the
    /// loaded detail record (`_page_for(item)`), else the row.
    fn build_form(&mut self, mode: &str, data: Option<Value>, item: Option<Item>) {
        let record = self
            .page_for(item.as_ref())
            .map(|p| p.record.clone())
            .or_else(|| data.clone());
        let req = FormRequest {
            mode: mode.to_string(),
            item: data,
            record,
        };
        self.spawn(move |b| {
            Msg::Ui(vec![match b.open_form(&req) {
                Ok(view) => Ui::Form(OpenedForm {
                    view,
                    mode: req.mode,
                    item,
                }),
                Err(e) => Ui::Failure(format!("Form unavailable: {e}")),
            }])
        });
    }

    /// `_push_form`.
    fn push_form(&mut self, opened: OpenedForm) {
        self.form = Some(ActiveForm {
            view: RefCell::new(opened.view),
            mode: opened.mode,
            item: opened.item,
            start: self.clock.now(),
        });
        self.check_form_done();
    }

    /// Is a form on screen?
    pub fn form_open(&self) -> bool {
        self.form.is_some()
    }

    /// `_push_form`'s `done` callback once the form finished.
    fn check_form_done(&mut self) {
        if !self.form.as_ref().is_some_and(|f| f.view.borrow().done()) {
            return;
        }
        let Some(form) = self.form.take() else {
            return;
        };
        match form.view.into_inner().finish() {
            FormExit::Cancelled => {}
            FormExit::Draft(path) => {
                self.notify(format!("Draft saved: {path}"), Severity::Information)
            }
            FormExit::Submit(send) => self.send_form(send, form.mode, form.item),
        }
    }

    /// `_send_form`: send once; an edit reloads the change, a create toasts
    /// the new id and reloads the list.
    /// Port of fsv.tui.app.FsvApp._send_form.
    fn send_form(&mut self, send: Box<dyn FormSend>, mode: String, item: Option<Item>) {
        if self.read_only {
            return;
        }
        self.spawn(move |_| {
            Msg::Ui(match send.send() {
                Err(e) => vec![Ui::Failure(actions::failure_detail(&e))],
                Ok(out) => {
                    let (label, done) = actions::form_result(&mode, &out);
                    match item {
                        Some(item) if mode == "edit" => {
                            vec![Ui::AfterWrite(item.res, item, "Change updated".into())]
                        }
                        _ => vec![
                            Ui::Toast {
                                message: format!("{label} created {done}").trim().to_string(),
                                severity: Severity::Information,
                                title: None,
                                secs: TOAST_SECS,
                            },
                            Ui::ReloadList,
                        ],
                    }
                }
            })
        });
    }

    fn load_catalog(&mut self) {
        self.spawn(|b| {
            Msg::Ui(vec![match b.catalog() {
                Ok((items, cats)) => Ui::Catalog(items, cats),
                Err(e) => Ui::Failure(format!("Catalog unavailable: {e}")),
            }])
        });
    }

    fn pick_catalog(&mut self, items: Vec<Value>, categories: Vec<Value>) {
        let mut choices = vec![(json!("all"), "All categories".to_string())];
        for c in &categories {
            let id = c.get("id").cloned().unwrap_or(Value::Null);
            let name = c
                .get("name")
                .map_or_else(|| fsv_py::py_str(&id), fsv_py::py_str);
            choices.push((id, name));
        }
        self.push(
            Modal::choices("Service catalog — category", choices),
            Cont::CatalogCategory { items, categories },
        );
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

mod reviewed;

#[cfg(test)]
mod tests;
