//! `FsvApp` methods driven one at a time. Expected text, labels, choices
//! and request arguments are Python's (`tui/app.py`, the cited method), or
//! the output of the Python helper run on the same input.

use std::sync::Mutex;

use fsv_core::Error;
use fsv_core::resource::Resource;

use super::*;
use crate::backend::{BResult, Failure, TASK_NOTIFY_BEFORE, TASK_STATUS_LABELS, TabInfo};
use crate::modals::{ConfirmModal, FailureModal, ModalResult};

/// `views.list_meta({"meta": {"total_count": 3}}, 3 items, 1, 100)` is
/// `(3, False)`; without the site crate the total is unknown.
const SITE_TOTAL: Option<i64> = if cfg!(feature = "site") {
    Some(3)
} else {
    None
};

#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<String>>,
    rows: usize,
    views: Vec<View>,
    headers: Vec<Value>,
    fails: Vec<&'static str>,
    network: bool,
    defaults: Vec<String>,
    tasks: Mutex<Vec<Value>>,
    tickets: Mutex<Vec<Value>>,
    search_rows: usize,
    task_result: Option<Value>,
    open_ok: bool,
    /// Empty means the platform stand-in (`TASK_STATUS_LABELS`).
    status_labels: Vec<String>,
    /// Empty means the platform stand-in (`TASK_NOTIFY_BEFORE`).
    notify_labels: Vec<String>,
}

impl Mock {
    fn new(rows: usize) -> Mock {
        Mock {
            rows,
            open_ok: true,
            ..Default::default()
        }
    }
    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }
    fn calls(&self, prefix: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with(prefix))
            .cloned()
            .collect()
    }
    fn all(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn fail(&self, what: &str) -> BResult<()> {
        if self.fails.contains(&what) {
            Err(Failure::value(format!("{what} denied")))
        } else {
            Ok(())
        }
    }
}

fn plan(review: Value) -> ReviewPlan {
    ReviewPlan {
        hash: format!("h-{review}"),
        review,
        public: false,
        target_id: json!(11),
        raw: Value::Null,
    }
}

fn schema() -> Value {
    json!({"fields": [
        {"name": "status", "label": "Status", "choices": [
            {"id": 1, "value": "Open"}, {"id": 2, "value": "Pending"}]},
        {"name": "priority", "label": "Priority"}
    ]})
}

impl Backend for Mock {
    fn default_where(&self, entity: &str) -> Vec<String> {
        if entity == "tickets" {
            Vec::new()
        } else {
            self.defaults.clone()
        }
    }
    fn copy_to_clipboard(&self, text: &str) -> Result<(), String> {
        self.log(format!("copy {text}"));
        self.fail("copy").map_err(|e| e.to_string())
    }
    fn open_url(&self, url: &str) -> bool {
        self.log(format!("open {url}"));
        self.open_ok
    }
    fn item_url(&self, res: Res, id: &Value) -> String {
        format!("https://acme.test/a/{}/{id}", res.api_path)
    }
    fn load_schema(&self, res: Res) -> BResult<Value> {
        self.log(format!("schema {}", res.name));
        if self.fails.contains(&"session") {
            return Err(Failure::from(Error::Session("expired".into())));
        }
        if self.fails.contains(&"schema") && res.name == "problems" {
            return Err(Failure::value("no problems module"));
        }
        Ok(schema())
    }
    fn current_user(&self) -> BResult<Value> {
        self.log("me".into());
        Ok(json!({"id": 7, "name": "Me"}))
    }
    fn user_tz(&self, me: &Value) -> Option<TimeZone> {
        self.log(format!("tz {}", me["id"]));
        Some(TimeZone::UTC)
    }
    fn load_views(&self, res: Res) -> BResult<Vec<View>> {
        self.log(format!("views {}", res.name));
        Ok(self.views.clone())
    }
    fn ticket_headers(&self) -> BResult<Vec<Value>> {
        self.log("headers".into());
        Ok(self.headers.clone())
    }
    fn view_settings(&self, res: Res, view: &View) -> BResult<Value> {
        self.log(format!("view-settings {} {}", res.name, view.id));
        Ok(json!({"order_by": "updated_at", "order_type": null}))
    }
    fn build_query_hash(&self, _res: Res, _schema: &Value, filters: &[String]) -> BResult<String> {
        self.fail("query")?;
        Ok(filters.join("&"))
    }
    fn list_page(&self, req: &ListRequest) -> BResult<(Vec<Value>, Value)> {
        self.log(format!(
            "list {} page={} q={:?} view={:?} order={} {}",
            req.res.name, req.page, req.query_hash, req.filter_name, req.order_by, req.order_type
        ));
        self.fail("list")?;
        let rows = (1..=self.rows)
            .map(|i| json!({"id": i, "display_id": i, "status": 1, "priority": 1, "subject": format!("row {i}")}))
            .collect();
        Ok((rows, json!({"meta": {"total_count": self.rows}})))
    }
    fn search_items(
        &self,
        term: &str,
        entity: Option<&str>,
        sort: &str,
        page: i64,
    ) -> BResult<SearchResult> {
        self.log(format!("search {term} {entity:?} {sort} {page}"));
        self.fail("search")?;
        let res = resource_for(entity.unwrap_or("tickets"));
        let items = (1..=self.search_rows)
            .map(|i| {
                let id = (page - 1) * 100 + i as i64;
                Item::new(res, json!({"id": id, "display_id": id, "subject": format!("hit {id}"), "status": "Open", "_agent": "Ann", "_group": "Ops"}))
            })
            .collect();
        Ok((
            items,
            vec![
                ("tickets".into(), 40),
                ("problems".into(), 0),
                ("changes".into(), 2),
            ],
        ))
    }
    fn load_detail(&self, res: Res, id: &Value, _schema: Option<&Value>) -> BResult<DetailPage> {
        self.log(format!("detail {} {id}", res.name));
        self.fail("detail")?;
        let tab = |n: &str, l: &str| TabInfo {
            name: n.into(),
            label: l.into(),
            count: None,
        };
        Ok(DetailPage {
            res,
            record: json!({"id": id, "display_id": id, "status": 1, "priority": 1, "subject": "row", "stats": {}}),
            tabs: vec![
                tab("details", "Details"),
                tab("notes", "Notes"),
                tab("tasks", "Tasks"),
            ],
            badge: "Open".into(),
            due: "Fri 9:00".into(),
            tz: None,
            schema: None,
            extra: Value::Null,
        })
    }
    fn load_tab(&self, res: Res, record: &Value, tab: &str) -> BResult<TabData> {
        self.log(format!("tab {} {} {tab}", res.name, record["id"]));
        self.fail("tab")?;
        let mut t = TabData {
            name: tab.into(),
            label: site::tab_label(tab),
            data: json!(self.tasks.lock().unwrap().clone()),
            ..Default::default()
        };
        t.meta.insert("complete".into(), json!(true));
        t.parts.insert(
            "Tickets".into(),
            json!(self.tickets.lock().unwrap().clone()),
        );
        t.parts_meta
            .insert("Tickets".into(), json!({"complete": true}));
        Ok(t)
    }
    fn completion_network(&self) -> bool {
        self.network
    }
    fn remote_user_values(&self, kind: &str, prefix: &str) -> BResult<Vec<String>> {
        self.log(format!("remote {kind} {prefix}"));
        Ok((1..=10).map(|i| format!("{prefix}{i}")).collect())
    }
    fn lookup_choices(&self, link: &str, prefix: &str) -> BResult<Vec<Value>> {
        self.log(format!("lookup {link} {prefix}"));
        Ok(vec![
            json!({"name": "Zed", "email": "zed@x.io"}),
            json!({"name": "Alpha", "email": "al@x.io"}),
        ])
    }
    fn update_item(&self, res: Res, id: &Value, body: &Map<String, Value>) -> BResult<Value> {
        self.log(format!(
            "update {} {id} {}",
            res.name,
            Value::Object(body.clone())
        ));
        self.fail("write")?;
        Ok(json!({}))
    }
    fn add_note(&self, res: Res, id: &Value, body: &str, public: bool) -> BResult<Value> {
        self.log(format!("note {} {id} {body} {public}", res.name));
        self.fail("write")?;
        Ok(json!({}))
    }
    fn ticket_note_plan(&self, ticket_id: &Value, body: &str, public: bool) -> BResult<ReviewPlan> {
        self.log(format!("ticket-note-plan {ticket_id} {body} {public}"));
        self.fail("ticket-note")?;
        Ok(plan(json!({"body": body})))
    }
    fn change_task_plan(
        &self,
        change_id: &Value,
        title: &str,
        owner: Option<&str>,
        options: &[(String, String)],
    ) -> BResult<ReviewPlan> {
        self.log(format!(
            "task-plan {change_id} {title} {owner:?} {options:?}"
        ));
        self.fail("task-plan")?;
        Ok(plan(json!({"title": title})))
    }
    fn task_create_choices(&self, key: &str) -> BResult<Vec<String>> {
        self.log(format!("task-choices {key}"));
        self.fail("task-choices")?;
        let custom = if key == "status" {
            &self.status_labels
        } else {
            &self.notify_labels
        };
        if !custom.is_empty() {
            return Ok(custom.clone());
        }
        let labels: &[&str] = if key == "status" {
            TASK_STATUS_LABELS
        } else {
            TASK_NOTIFY_BEFORE
        };
        Ok(labels.iter().map(|s| (*s).to_string()).collect())
    }
    fn send_change_task(&self, p: &ReviewPlan, read_only: bool) -> BResult<Value> {
        self.log(format!("task-send {} {read_only}", p.hash));
        Ok(self
            .task_result
            .clone()
            .unwrap_or_else(|| json!({"task_id": 501, "verified": true})))
    }
    fn search_change_tickets(&self, query: &str) -> BResult<Vec<Value>> {
        self.log(format!("ticket-search {query}"));
        Ok(vec![
            json!({"id": 900, "display_id": 90, "subject": "Acme access"}),
        ])
    }
    fn download_choices(&self, _item: &Item) -> BResult<(Vec<Value>, Vec<String>)> {
        self.fail("downloads")?;
        Ok((vec![json!({"id": 70, "name": "report.pdf"})], Vec::new()))
    }
    fn task_for_edit(&self, change_id: &Value, task_id: &Value) -> BResult<(Value, Vec<Value>)> {
        self.log(format!("task-edit {change_id} {task_id}"));
        self.fail("task-edit")?;
        Ok((json!({"title": "Old", "status": 1}), Vec::new()))
    }
    fn association_plan(
        &self,
        change_id: &Value,
        ticket_ids: &[Value],
        unlink: bool,
    ) -> BResult<ReviewPlan> {
        self.log(format!("assoc-plan {change_id} {ticket_ids:?} {unlink}"));
        Ok(plan(json!({"change_id": change_id, "unlink": unlink})))
    }
    fn send_association(&self, p: &ReviewPlan) -> BResult<Value> {
        self.log(format!("assoc-send {}", p.hash));
        Ok(json!({"verified": true}))
    }
    fn approval_request_plan(
        &self,
        change_id: &Value,
        name: &str,
        message: &str,
        recipients: &[String],
        rule: &str,
    ) -> BResult<ReviewPlan> {
        self.log(format!(
            "approval-plan {change_id} {name} {message} {recipients:?} {rule}"
        ));
        Ok(plan(json!({"name": name})))
    }
    fn approval_targets(&self, kind: &str, change_id: &Value) -> BResult<Vec<Value>> {
        self.log(format!("approval-targets {kind} {change_id}"));
        self.fail("approvals")?;
        Ok(vec![json!({"id": 31})])
    }
    fn approval_action_plan(
        &self,
        change_id: &Value,
        target_id: &Value,
        operation: &str,
        remark: Option<&str>,
    ) -> BResult<ReviewPlan> {
        self.log(format!(
            "approval-action {change_id} {target_id} {operation} {remark:?}"
        ));
        Ok(plan(json!({"op": operation})))
    }
    fn update_task(
        &self,
        change_id: &Value,
        task_id: &Value,
        body: &Map<String, Value>,
    ) -> BResult<()> {
        self.log(format!(
            "task-update {change_id} {task_id} {}",
            Value::Object(body.clone())
        ));
        Ok(())
    }
    fn delete_task(&self, change_id: &Value, task_id: &Value) -> BResult<()> {
        self.log(format!("task-delete {change_id} {task_id}"));
        Ok(())
    }
    fn ticket_property_plan(
        &self,
        ticket_id: &Value,
        name: &str,
        value: &Value,
    ) -> BResult<ReviewPlan> {
        self.log(format!("property-plan {ticket_id} {name} {value}"));
        self.fail("property")?;
        Ok(plan(json!({"name": name, "value": value})))
    }
    fn send_ticket_property(&self, p: &ReviewPlan) -> BResult<Value> {
        self.log(format!("property-send {}", p.hash));
        Ok(json!({"verified": true}))
    }
    fn task_mutation_plan(
        &self,
        kind: &str,
        change_id: &Value,
        task_id: &Value,
        body: &Map<String, Value>,
    ) -> BResult<ReviewPlan> {
        self.log(format!(
            "task-mutation-plan {kind} {change_id} {task_id} {}",
            Value::Object(body.clone())
        ));
        Ok(plan(json!({"kind": kind})))
    }
    fn send_task_mutation(&self, p: &ReviewPlan) -> BResult<Value> {
        self.log(format!("task-mutation-send {}", p.hash));
        Ok(json!({"verified": true}))
    }
    fn catalog(&self) -> BResult<(Vec<Value>, Vec<Value>)> {
        self.fail("catalog")?;
        Ok((
            vec![
                json!({"display_id": 4, "name": "Laptop", "category_id": 1}),
                json!({"display_id": null, "name": "Hidden", "category_id": 1}),
                json!({"display_id": 5, "name": "Badge", "category_id": 2}),
            ],
            vec![json!({"id": 1, "name": "Hardware"}), json!({"id": 2})],
        ))
    }
    crate::backend_stubs!(
        send_ticket_note,
        download,
        send_approval,
        ticket_property_fields,
        known_error_plan,
        send_known_error,
        planning_definitions,
        planning_row,
        planning_text_plan,
        send_planning_text,
        ticket_conversation_plan,
        send_ticket_conversation,
        asset_candidates,
        open_form
    );
}

struct TestPages;

impl Pages for TestPages {
    fn lifecycle_markup(&self, page: &DetailPage, context: usize) -> String {
        format!("flow {} ctx={context}", page.record["id"])
    }
    fn reporter_markup(&self, _page: &DetailPage) -> String {
        "reported by Ann".into()
    }
    fn main_lines(&self, _page: &DetailPage) -> Vec<String> {
        vec!["main".into()]
    }
    fn notes_lines(&self, _page: &DetailPage) -> Vec<String> {
        vec!["note".into()]
    }
    fn side_lines(&self, _page: &DetailPage) -> Vec<String> {
        vec!["side".into()]
    }
    fn tab_lines(
        &self,
        data: &TabData,
        tz: Option<&TimeZone>,
        schema: Option<&Value>,
    ) -> Vec<String> {
        crate::BasicPages.tab_lines(data, tz, schema)
    }
}

fn start(mock: Mock, read_only: bool) -> (App, Arc<Mock>) {
    let mock = Arc::new(mock);
    let mut app = App::new_inline(mock.clone(), Arc::new(TestPages), read_only);
    app.mount();
    app.settle();
    app.advance(DETAIL_DEBOUNCE);
    (app, mock)
}

fn app(rows: usize) -> (App, Arc<Mock>) {
    start(Mock::new(rows), false)
}

fn change(id: i64) -> Item {
    Item::new(
        &CHANGES,
        json!({"id": id, "display_id": id, "subject": "Acme change"}),
    )
}

/// The app with change 11 selected (detail loaded).
fn on_change(mock: Mock) -> (App, Arc<Mock>) {
    let (mut a, m) = start(mock, false);
    a.switch_entity("changes");
    a.advance(ENTITY_DEBOUNCE);
    a.advance(DETAIL_DEBOUNCE);
    assert_eq!(a.selected.as_ref().map(|s| s.res.name), Some("changes"));
    (a, m)
}

fn top(a: &App) -> &Modal {
    &a.modals.last().expect("a modal is open").0
}

fn picker_labels(a: &App) -> (String, Vec<String>) {
    match top(a) {
        Modal::Picker(p) => (
            p.title.clone(),
            p.options.iter().map(|o| o.1.clone()).collect(),
        ),
        m => panic!("not a picker: {m:?}"),
    }
}

fn confirm(a: &App) -> &ConfirmModal {
    match top(a) {
        Modal::Confirm(c) => c,
        m => panic!("not a confirm: {m:?}"),
    }
}

fn failure(a: &App) -> &FailureModal {
    match top(a) {
        Modal::Failure(f) | Modal::Outcome(f) => f,
        m => panic!("not a failure: {m:?}"),
    }
}

fn value_title(a: &App) -> String {
    match top(a) {
        Modal::Value(v) => v.title.clone(),
        m => panic!("not a value screen: {m:?}"),
    }
}

/// Answer the top modal as its Python callback would receive it.
fn answer(a: &mut App, result: ModalResult) {
    let (_, cont) = a.modals.pop().expect("a modal is open");
    a.resume(cont, result);
    a.settle();
}

fn toasts(a: &App) -> Vec<(String, Severity)> {
    a.toasts
        .iter()
        .map(|t| (t.message.clone(), t.severity))
        .collect()
}

fn last_toast(a: &App) -> String {
    a.toasts
        .last()
        .map(|t| t.message.clone())
        .unwrap_or_default()
}

fn markup(a: &App) -> String {
    match &a.detail.content {
        Content::Markup(m) => m.clone(),
        Content::Plain(p, _) => p.clone(),
    }
}

fn saved(id: &str, name: &str, raw: Value) -> View {
    View {
        id: id.into(),
        name: name.into(),
        system: false,
        marked_default: false,
        raw,
    }
}

// ── list loading ────────────────────────────────────────────

#[test]
fn fetch_list_loads_meta_once_then_one_page() {
    let mut mock = Mock::new(3);
    mock.views = vec![saved("v1", "Mine", json!({"id": 1}))];
    let mock = Arc::new(mock);
    let a = App::new_inline(mock.clone(), Arc::new(TestPages), false);
    let msg = a.list_job(false).fetch_list(mock.as_ref());
    assert_eq!(
        mock.all(),
        vec![
            "schema tickets",
            "schema changes",
            "schema problems",
            "me",
            "tz 7",
            "views tickets",
            "headers",
            "view-settings tickets v1",
            "list tickets page=1 q=None view=Some(\"v1\") order=updated_at desc",
        ]
    );
    let Msg::List {
        page,
        loaded,
        outcome,
        ..
    } = msg
    else {
        panic!("not a list message")
    };
    assert_eq!(page, 1);
    assert_eq!(loaded.sort.as_deref(), Some("updated_at"));
    assert_eq!(
        outcome.map(|o| (o.0.len(), o.1, o.2)),
        Ok((3, SITE_TOTAL, false))
    );
}

#[test]
fn fetch_list_filters_drop_the_view_and_schema_errors_fall_back() {
    let mut mock = Mock::new(1);
    mock.views = vec![saved("v1", "Mine", json!({"id": 1}))];
    mock.fails = vec!["schema"];
    let mock = Arc::new(mock);
    let mut a = App::new_inline(mock.clone(), Arc::new(TestPages), false);
    a.active_filters = vec!["status=Open".into(), "priority=High".into()];
    let Msg::List { loaded, .. } = a.list_job(false).fetch_list(mock.as_ref()) else {
        panic!()
    };
    assert!(
        loaded
            .schemas
            .contains(&("problems", json!({"fields": []})))
    );
    assert_eq!(
        mock.calls("list"),
        vec![
            "list tickets page=1 q=Some(\"status=Open&priority=High\") view=None order=created_at desc"
        ]
    );
    assert!(mock.calls("view-settings").is_empty());
}

#[test]
fn fetch_list_session_error_stops_before_listing() {
    let mut mock = Mock::new(1);
    mock.fails = vec!["session"];
    let mock = Arc::new(mock);
    let a = App::new_inline(mock.clone(), Arc::new(TestPages), false);
    let Msg::List { outcome, .. } = a.list_job(false).fetch_list(mock.as_ref()) else {
        panic!()
    };
    assert!(outcome.is_err());
    assert!(mock.calls("list").is_empty());
}

#[test]
fn load_list_accept_list_and_populate_list_fill_the_table() {
    let (mut a, m) = app(3);
    assert_eq!(m.calls("list").len(), 1);
    assert_eq!(a.rows.len(), 3);
    assert_eq!(a.rows[0].cells[0].0.text, "001");
    assert_eq!((a.total, a.list_total, a.has_more), (3, SITE_TOTAL, false));
    let epoch = a.list_epoch;
    a.accept_list(
        epoch + 1,
        "tickets",
        1,
        vec![json!({"id": 9})],
        None,
        false,
        false,
    );
    assert_eq!(a.items.len(), 3, "stale epoch is dropped");
    a.accept_list(
        epoch,
        "tickets",
        3,
        vec![json!({"id": 9})],
        None,
        false,
        true,
    );
    assert_eq!(a.items.len(), 3, "append out of order is dropped");
    a.accept_list(
        epoch,
        "tickets",
        2,
        vec![
            json!({"id": 3, "display_id": 3}),
            json!({"id": 4, "display_id": 4}),
        ],
        None,
        true,
        true,
    );
    assert_eq!(a.items.len(), 4, "duplicates on append are skipped");
    assert_eq!(
        (a.page, a.total, a.has_more),
        (2, 5, true),
        "lower bound counts before de-duplication"
    );
    a.load_list(true);
    a.settle();
    assert_eq!(
        m.calls("list").last().map(|c| c.contains("page=3")),
        Some(true)
    );
}

#[test]
fn populate_list_has_more_defaults_to_a_full_page_and_empty_clears() {
    let (mut a, _) = app(0);
    assert_eq!(markup(&a), "[dim]no rows[/]");
    a.per_page = 2;
    let items: Vec<Item> = (1..=2)
        .map(|i| Item::new(&TICKETS, json!({"id": i, "display_id": i})))
        .collect();
    a.populate_list(items, 2, false, None);
    assert!(a.has_more);
    assert_eq!(a.rows.len(), 2);
}

#[cfg(feature = "site")]
#[test]
fn list_columns_follow_ticket_headers() {
    let headers = vec![
        json!({"name": "status", "label": "Status &amp; more", "selected": true, "position": 2}),
        json!({"name": "display_id", "label": "ID", "selected": true, "position": 0}),
        json!({"name": "priority", "label": "Priority", "selected": true, "position": 3}),
        json!({"name": "x", "label": "X", "selected": false, "position": 1}),
    ];
    let sch = json!({"fields": [{"name": "status", "label": "State"}, {"name": "priority", "label": "Urgency"}, {"name": "subject", "label": "Title"}]});
    let cols = |res, h: Option<&[Value]>| -> Vec<(String, String)> {
        site::list_columns(res, Some(&sch), h)
            .into_iter()
            .map(|c| (c.key, c.label))
            .collect()
    };
    let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
        v.iter()
            .map(|(k, l)| (k.to_string(), l.to_string()))
            .collect()
    };
    assert_eq!(
        cols(&TICKETS, Some(&headers)),
        pairs(&[
            ("subject", "Title"),
            ("status", "Status & more"),
            ("priority", "Priority")
        ])
    );
    assert_eq!(
        cols(&CHANGES, None),
        pairs(&[
            ("subject", "Title"),
            ("requester", "Requester"),
            ("status", "State"),
            ("priority", "Urgency"),
            ("assigned_to", "Assigned to"),
            ("department", "Department"),
        ])
    );
    assert_eq!(site::sort_label("priority_id", Some(&sch)), "Urgency");
    assert_eq!(site::sort_label("due_by_x", None), "Due by x");
    assert_eq!(
        site::list_meta(&json!({"meta": {"total_count": 1}}), 2, 1, 100),
        (None, false)
    );
}

#[test]
fn list_failed_toasts_only_for_the_current_list() {
    let (mut a, _) = app(1);
    a.loading_more = true;
    a.list_failed(a.list_epoch + 1, "tickets", "boom".into());
    assert!(a.toasts.is_empty() && a.loading_more);
    a.list_failed(a.list_epoch, "tickets", "boom".into());
    assert!(!a.loading_more);
    let t = &a.toasts[0];
    assert_eq!(
        (t.message.as_str(), t.title.as_deref(), t.severity),
        (
            "boom",
            Some("List load failed; reload or retry"),
            Severity::Error
        )
    );
}

#[test]
fn load_more_and_reload_list() {
    let (mut a, m) = app(2);
    a.load_more();
    assert!(!a.loading_more, "no more pages: nothing to load");
    a.has_more = true;
    a.load_more();
    assert!(a.loading_more);
    a.load_more();
    a.settle();
    assert_eq!(m.calls("list").len(), 2, "one append while loading");
    a.focus = Focus::Detail;
    a.filter_error = Some("bad".into());
    let epoch = a.list_epoch;
    a.reload_list("[dim]reloading...[/]");
    assert_eq!(a.list_epoch, epoch + 1);
    assert_eq!((a.focus, a.filter_error.clone()), (Focus::List, None));
    assert_eq!(markup(&a), "[dim]reloading...[/]");
    a.settle();
    assert_eq!(a.items.len(), 2);
}

// ── detail ──────────────────────────────────────────────────

#[test]
fn show_detail_page_for_and_tab_names() {
    let (mut a, _) = app(2);
    let sel = a.selected.clone().expect("row 1 selected");
    let page = a.page_for(Some(&sel)).expect("page loaded");
    assert_eq!(page.tab_names(), vec!["details", "notes", "tasks"]);
    assert_eq!(a.tab_names(None), vec!["details", "notes", "tasks"]);
    assert_eq!(
        a.tab_names(Some(&CHANGES)),
        detail_tab_labels("changes").to_vec()
    );
    a.detail_tab_idx = 9;
    let key = sel.key();
    a.pending_detail_key = Some(key.clone());
    a.show_detail(sel.clone(), &key);
    assert_eq!(a.detail_tab_idx, 0);
    a.show_detail(change(5), "changes:5");
    assert_eq!(
        a.selected.as_ref().map(Item::key),
        Some(key),
        "stale key ignored"
    );
    assert!(a.page_for(None).is_none());
}

#[test]
fn current_tab_name_defaults_to_details() {
    let (mut a, _) = app(0);
    assert_eq!(a.current_tab_name(None), "details");
    a.detail_tab_idx = 99;
    assert_eq!(a.current_tab_name(Some(&PROBLEMS)), "activity");
}

#[test]
fn detail_error_only_for_the_pending_row() {
    let (mut a, _) = app(1);
    a.pending_detail_key = Some("tickets:1".into());
    a.detail_error("tickets:2", "nope".into());
    assert_ne!(markup(&a), "could not load: nope");
    a.detail_error("tickets:1", "nope".into());
    assert_eq!(markup(&a), "could not load: nope");
    assert!(matches!(&a.detail.content, Content::Plain(_, s) if s.fg == Some(Color::Red)));
}

#[test]
fn load_detail_errors_without_cache_show_the_message() {
    let mut mock = Mock::new(1);
    mock.fails = vec!["detail"];
    let (a, _) = start(mock, false);
    assert_eq!(markup(&a), "could not load: detail denied");
}

#[test]
fn trigger_state_flow_render_bar_tabs_and_content() {
    let (mut a, _) = app(1);
    assert_eq!(a.detail.state_flow, "flow 1 ctx=3");
    assert_eq!(a.detail.reporter, "reported by Ann");
    let bar: Vec<&str> = a.detail.bar.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(bar, vec!["#INC-1 ", "row  ", "Open", ", due Fri 9:00"]);
    assert_eq!(
        a.detail.tabs,
        "[bold reverse] 1:Details [/]  [dim]2:Notes[/]  [dim]3:Tasks[/]"
    );
    assert_eq!(markup(&a), "main\n\nnote");
    assert_eq!(a.detail.side, Some(vec!["side".to_string()]));

    let sel = Item::new(
        &CHANGES,
        json!({"id": 5, "display_id": 5, "subject": "Acme", "priority": 2}),
    );
    a.trigger_state_flow(&sel);
    assert_eq!(
        (a.detail.state_flow.as_str(), a.detail.reporter.as_str()),
        ("", "")
    );
    a.render_detail_bar(&sel);
    let bar: Vec<&str> = a.detail.bar.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(bar, vec!["CHN-5 Acme  ", "Medium"]);
    let t = Item::new(
        &TICKETS,
        json!({"id": 6, "display_id": 6, "status": 2, "subject": "x"}),
    );
    a.render_detail_bar(&t);
    let bar: Vec<&str> = a.detail.bar.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(bar[0], "[Pending]");
    a.selected = Some(sel.clone());
    a.detail_tab_idx = 1;
    a.render_detail_tabs(&CHANGES);
    assert!(
        a.detail
            .tabs
            .starts_with("[dim]1:details[/]  [bold reverse] 2:notes [/]")
    );
}

#[test]
fn render_detail_bar_hides_due_on_resolved_tickets() {
    let (mut a, _) = app(1);
    let key = "tickets:1".to_string();
    if let Some(p) = a.details.get_mut(&key) {
        p.record["stats"] = json!({"resolved_at": "2026-01-01"});
    }
    let item = a.selected.clone().unwrap();
    a.render_detail_bar(&item);
    assert!(!a.detail.bar.iter().any(|(t, _)| t.starts_with(", due")));
}

#[test]
fn render_detail_content_loads_sub_tabs_and_render_sub_content_guards() {
    let (mut a, m) = app(1);
    let sel = a.selected.clone().unwrap();
    a.detail_tab_idx = 2;
    a.render_detail_content(&sel);
    assert_eq!(a.pending_sub_key.as_deref(), Some("tickets:1:tasks"));
    assert_eq!(markup(&a), "[dim]loading tasks...[/]");
    assert_eq!(a.detail.side, None);
    a.settle();
    assert_eq!(m.calls("tab").len(), 1);
    assert_eq!(markup(&a), "\n  [dim]no tasks[/]\n");
    a.load_sub_resource("tasks".into(), "tickets:1:tasks".into());
    a.settle();
    assert_eq!(m.calls("tab").len(), 1, "cached");
    let data = TabData {
        label: "Tasks".into(),
        data: json!([1, 2]),
        ..Default::default()
    };
    a.render_sub_content("tasks", &data, &TICKETS, "tickets:9:tasks");
    assert_eq!(markup(&a), "\n  [dim]no tasks[/]\n", "stale key ignored");
    a.render_sub_content("tasks", &data, &TICKETS, "tickets:1:tasks");
    assert_eq!(markup(&a), "\n  2 Tasks\n");
}

#[test]
fn load_sub_resource_error_is_a_missing_note() {
    let mut mock = Mock::new(1);
    mock.fails = vec!["tab"];
    let (mut a, _) = start(mock, false);
    a.step_detail_tab(1);
    a.settle();
    assert_eq!(markup(&a), "\n  [dim]tab denied: unavailable[/]\n");
}

#[test]
fn step_detail_tab_wraps() {
    let (mut a, _) = app(1);
    a.step_detail_tab(-1);
    assert_eq!(a.detail_tab_idx, 2);
    a.step_detail_tab(1);
    assert_eq!(a.detail_tab_idx, 0);
}

#[test]
fn panes_toggle_focus_and_resize() {
    let (mut a, _) = app(1);
    assert_eq!(a.active_pane(), Pane::List);
    a.toggle_pane();
    assert_eq!((a.active_pane(), a.focus), (Pane::Detail, Focus::Detail));
    a.focus_pane(Pane::List);
    assert_eq!(a.active_pane(), Pane::List);
    for _ in 0..10 {
        a.resize_panes(1);
    }
    assert_eq!((a.list_fr, a.detail_fr), (17, 3));
    a.resize_panes(-20);
    assert_eq!((a.list_fr, a.detail_fr), (3, 17));
}

#[test]
fn clear_and_pending_detail_reset_side_and_scroll() {
    let (mut a, _) = app(1);
    assert_eq!(a.selected_key().as_deref(), Some("tickets:1"));
    a.detail.scroll = 7;
    a.show_side(Some(vec!["x".into()]));
    a.set_pending_detail("tickets:2", "[dim]loading details...[/]");
    assert_eq!((a.detail.scroll, a.detail.side.clone()), (0, None));
    assert_eq!(a.selected_key(), None);
    assert_eq!(a.pending_detail_key.as_deref(), Some("tickets:2"));
    a.detail.scroll = 4;
    a.scroll_detail_home();
    assert_eq!(a.detail.scroll, 0);
    a.detail.scroll = 4;
    a.reset_detail_scroll();
    assert_eq!(a.detail.scroll, 0);
    a.clear_detail("[dim]select row[/]");
    assert_eq!(a.pending_detail_key, None);
    assert_eq!(markup(&a), "[dim]select row[/]");
}

#[test]
fn find_item_and_item_key() {
    let (a, _) = app(2);
    let it = a.find_item("INC-2").expect("display id lookup");
    assert_eq!(it.key(), "tickets:2");
    assert!(a.find_item("INC-9").is_none());
    assert_eq!(Item::new(&CHANGES, json!({"id": 4})).key(), "changes:4");
}

#[test]
fn row_highlighted_uses_the_cache_and_pages_at_the_bottom() {
    let (mut a, m) = app(2);
    a.cursor = 1;
    a.row_highlighted();
    a.advance(DETAIL_DEBOUNCE);
    a.cursor = 0;
    a.row_highlighted();
    assert_eq!(
        a.selected_key().as_deref(),
        Some("tickets:1"),
        "cached record shown at once"
    );
    a.advance(DETAIL_DEBOUNCE);
    assert_eq!(m.calls("detail").len(), 2);
    a.has_more = true;
    a.cursor = 1;
    a.row_highlighted();
    assert!(a.loading_more);
}

// ── header / bars ───────────────────────────────────────────

#[test]
fn view_header_names_range_total_and_sort() {
    let (mut a, _) = app(3);
    assert_eq!(
        a.view_header(),
        "[bold]All tickets[/][dim] · 1-3 of 3 · Sort by: Created Date[/]"
    );
    a.active_filters = vec!["status=Open".into()];
    a.list_total = None;
    a.has_more = true;
    assert_eq!(
        a.view_header(),
        "[bold]Filtered[/][dim] · 1-3 of 3+ · Sort by: Created Date[/]"
    );
    a.items.clear();
    a.has_more = false;
    assert!(a.view_header().contains(" · 0 of 0 · "));
}

#[test]
fn render_header_filter_bar_status_and_key_hints() {
    let (mut a, _) = app(3);
    assert!(a.header_markup().starts_with(
        "[bold cyan]fsv[/] :: freshservice  [bold reverse] TICKETS [/] PROBLEMS  CHANGES    "
    ));
    assert!(a.header_markup().ends_with("  [dim]v views  ? help[/]"));
    assert_eq!(
        a.filter_bar_markup(),
        "[dim]/ to filter[/]  [dim]focus=list  sel=INC-1  3 rows[/]"
    );
    assert_eq!(
        a.status_parts(),
        (
            "TICKETS > INC-1 > details    ".into(),
            "pane=list · 3 rows".into()
        )
    );
    a.filter_error = Some("bad".into());
    a.loading_more = true;
    assert_eq!(
        a.filter_bar_markup(),
        "[red]filter error: bad[/]  [dim]focus=list  sel=INC-1  3 rows[/]  [yellow]loading more…[/]"
    );
    assert_eq!(a.status_parts().1, "pane=list · loading more…");
    assert!(a.key_hints_markup().contains("[bold]r[/] reload/reply  "));
    a.read_only = true;
    let hints = a.key_hints_markup();
    assert!(hints.contains("[bold]o[/] browser  [bold]r[/] reload  [bold]y[/] yank"));
}

// ── filter input ────────────────────────────────────────────

#[test]
fn open_apply_and_cancel_filter_input() {
    let (mut a, m) = app(1);
    a.active_filters = vec!["status=Open".into()];
    a.open_filter_input();
    assert_eq!(
        (a.focus, a.filter.input.value()),
        (Focus::Filter, "status=Open")
    );
    a.close_filter();
    assert_eq!((a.focus, a.filter.visible), (Focus::List, false));
    a.apply_filter(r#"status=Open subject="a b""#);
    assert_eq!(a.active_filters, vec!["status=Open", "subject=a b"]);
    assert_eq!(a.filter.history[0], r#"status=Open subject="a b""#);
    assert_eq!(markup(&a), "[dim]filtering...[/]");
    a.settle();
    assert!(
        m.calls("list")
            .last()
            .unwrap()
            .contains("q=Some(\"status=Open&subject=a b\")")
    );
    a.apply_filter("   ");
    assert!(a.active_filters.is_empty());
}

#[test]
fn filter_input_events_submit_and_change() {
    let (mut a, _) = app(1);
    a.open_filter_input();
    a.filter.input.set_value("sta");
    a.on_input_changed();
    assert_eq!(a.filter.suggestions, vec!["status"]);
    a.filter.input.set_value("status=Open");
    a.on_input_submitted(Focus::Filter);
    assert_eq!(a.active_filters, vec!["status=Open"]);
}

#[test]
fn maybe_schedule_lookup_and_run_network_lookup() {
    let mut mock = Mock::new(1);
    mock.network = true;
    let (mut a, m) = start(mock, false);
    a.schemas.insert(
        "tickets",
        json!({"fields": [
            {"name": "requester", "field_type": "default_requester"},
            {"name": "site", "field_type": "custom_lookup", "lookup_config": {"link": "/sites"}}]}),
    );
    a.open_filter_input();
    a.type_text("requester=al");
    a.advance(LOOKUP_DEBOUNCE);
    assert_eq!(m.calls("remote"), vec!["remote requesters al"]);
    assert_eq!(a.filter.suggestions.len(), 8, "first 8 remote values");
    a.run_network_lookup(LookupRequest {
        key: "stale".into(),
        prefix: "x".into(),
        kind: "agents".into(),
        link: None,
    });
    a.settle();
    assert_eq!(m.calls("remote").len(), 1, "stale key never fetched");
    a.filter.input.set_value("");
    a.type_text("site=al");
    a.advance(LOOKUP_DEBOUNCE);
    assert_eq!(m.calls("lookup"), vec!["lookup /sites al"]);
    assert_eq!(a.filter.suggestions, vec!["al@x.io", "Zed"]);
}

#[test]
fn maybe_schedule_lookup_needs_network_a_field_and_two_chars() {
    let mut mock = Mock::new(1);
    mock.network = true;
    let (mut a, m) = start(mock, false);
    a.schemas.insert(
        "tickets",
        json!({"fields": [{"name": "requester", "field_type": "default_requester"}]}),
    );
    a.filter.input.set_value("requester=a");
    a.maybe_schedule_lookup();
    a.filter.input.set_value("requester=al ");
    a.maybe_schedule_lookup();
    a.advance(LOOKUP_DEBOUNCE);
    assert!(
        m.calls("remote").is_empty(),
        "short prefix / trailing space"
    );
    a.filter.input.set_value("requester=al");
    a.maybe_schedule_lookup();
    a.filter.input.set_value("requester=a");
    a.maybe_schedule_lookup();
    a.advance(LOOKUP_DEBOUNCE);
    assert!(
        m.calls("remote").is_empty(),
        "shrinking below 2 stops the timer"
    );
    a.filter.input.set_value("requester=al");
    a.maybe_schedule_lookup();
    a.advance(LOOKUP_DEBOUNCE);
    assert!(
        m.calls("remote").is_empty(),
        "same lookup key is not rescheduled"
    );
    a.filter.input.set_value("requester=ali");
    a.maybe_schedule_lookup();
    a.advance(LOOKUP_DEBOUNCE);
    assert_eq!(m.calls("remote"), vec!["remote requesters ali"]);

    let (mut b, mb) = app(1);
    b.schemas = a.schemas.clone();
    b.filter.input.set_value("requester=al");
    b.maybe_schedule_lookup();
    b.advance(LOOKUP_DEBOUNCE);
    assert!(mb.calls("remote").is_empty(), "completion network off");
}

// ── search ──────────────────────────────────────────────────

#[test]
fn load_search_loads_missing_schemas_then_searches() {
    let mut mock = Mock::new(1);
    mock.search_rows = 2;
    let mock = Arc::new(mock);
    let mut a = App::new_inline(mock.clone(), Arc::new(TestPages), false);
    a.search_mode = true;
    a.search_term = "vpn".into();
    a.search_page = 3;
    a.load_search(false);
    a.settle();
    assert_eq!(
        mock.all(),
        vec![
            "schema tickets",
            "schema changes",
            "schema problems",
            "search vpn None relevance 3",
        ]
    );
    assert_eq!(a.rows.len(), 2);
    a.load_search(true);
    a.settle();
    assert_eq!(mock.calls("schema").len(), 3, "schemas are loaded once");
    assert_eq!(a.items.len(), 4);
}

fn searching(rows: usize) -> (App, Arc<Mock>) {
    let mut mock = Mock::new(2);
    mock.search_rows = rows;
    let (mut a, m) = start(mock, false);
    a.open_search_input();
    assert_eq!((a.focus, a.search_input.value()), (Focus::Search, ""));
    a.search_input.set_value("vpn");
    a.on_input_submitted(Focus::Search);
    a.settle();
    (a, m)
}

#[test]
fn apply_search_load_search_and_populate_search() {
    let (mut a, m) = searching(3);
    assert!(a.search_mode);
    assert_eq!(m.calls("search"), vec!["search vpn None relevance 1"]);
    assert_eq!(a.total, 42, "sum of totals when not narrowed");
    assert!(!a.search_has_more);
    assert_eq!(
        a.table_columns,
        vec!["", "ID", "STATUS", "SUBJECT", "AGENT", "GROUP"]
    );
    let cells: Vec<&str> = a.rows[0].cells.iter().map(|c| c.0.text.as_str()).collect();
    assert_eq!(cells, vec!["001", "INC-1", "Open", "hit 1", "Ann", "Ops"]);
    a.populate_search(Vec::new(), Vec::new(), false, a.search_seq - 1);
    assert_eq!(a.rows.len(), 3, "stale seq dropped");
    a.open_search_input();
    assert_eq!(a.search_input.value(), "vpn");
    a.cancel_search_input();
    assert_eq!((a.focus, a.search_input_visible), (Focus::List, false));
}

#[test]
fn apply_search_empty_term_exits_or_refocuses() {
    let (mut a, m) = app(1);
    a.focus = Focus::Search;
    a.apply_search("  ");
    assert_eq!((a.focus, a.search_mode), (Focus::List, false));
    let (mut s, _) = searching(1);
    s.apply_search("");
    assert!(!s.search_mode);
    s.settle();
    assert!(!m.calls("list").is_empty());
}

#[test]
fn step_search_entity_narrow_cache_and_restore() {
    let (mut a, m) = searching(30);
    a.step_search_entity(1);
    a.settle();
    assert_eq!(a.search_entity, Some("tickets"));
    assert_eq!((a.total, a.search_has_more), (40, true));
    a.step_search_entity(-1);
    assert_eq!(a.search_entity, None);
    assert_eq!(a.total, 42, "restored from the cache");
    assert_eq!(m.calls("search").len(), 2);
    a.step_search_entity(1);
    assert_eq!(m.calls("search").len(), 2, "tickets restored too");
    a.search_narrow_entity("changes");
    a.settle();
    assert_eq!(
        m.calls("search").last().unwrap(),
        "search vpn Some(\"changes\") relevance 1"
    );
    a.save_search_cache();
    a.restore_search_cache(Some("tickets"));
    assert_eq!((a.search_entity, a.rows.len()), (Some("tickets"), 30));
    a.search_mode = false;
    a.search_narrow_entity("problems");
    assert_eq!(a.search_entity, Some("tickets"), "not in search mode");
}

#[test]
fn search_load_more_and_cycle_search_sort() {
    let (mut a, m) = searching(30);
    a.search_narrow_entity("tickets");
    a.settle();
    a.search_load_more();
    a.settle();
    assert_eq!(
        m.calls("search").last().unwrap(),
        "search vpn Some(\"tickets\") relevance 2"
    );
    assert_eq!(a.rows.len(), 60);
    a.search_has_more = false;
    a.search_load_more();
    assert_eq!(a.search_page, 2);
    for want in ["created", "modified", "relevance"] {
        a.cycle_search_sort();
        assert_eq!(a.search_sort, want);
    }
    a.settle();
    assert_eq!(a.search_page, 1);
}

#[test]
fn exit_search_mode_restores_default_filters() {
    let (mut a, _) = searching(1);
    a.entity = "changes";
    a.exit_search_mode();
    assert!(!a.search_mode && a.search_term.is_empty());
    assert!(a.active_filters.is_empty());
    assert_eq!(a.get_default_filters("tickets"), Vec::<String>::new());
}

// ── entities / views ────────────────────────────────────────

#[test]
fn switch_entity_debounces_and_applies_defaults() {
    let mut mock = Mock::new(1);
    mock.defaults = vec!["status=Open".into()];
    let (mut a, m) = start(mock, false);
    let epoch = a.list_epoch;
    a.switch_entity("tickets");
    assert_eq!(a.list_epoch, epoch, "same entity is a no-op");
    a.switch_entity("problems");
    assert_eq!(a.active_filters, vec!["status=Open"]);
    a.advance(Duration::from_millis(100));
    a.switch_entity("changes");
    a.advance(ENTITY_DEBOUNCE);
    assert_eq!(m.calls("list problems").len(), 0);
    assert_eq!(m.calls("list changes").len(), 1);
    a.next_entity();
    assert_eq!(a.entity, "tickets");
    a.prev_entity();
    assert_eq!(a.entity, "changes");
}

#[test]
fn pick_view_offers_saved_views() {
    let mut mock = Mock::new(1);
    mock.views = vec![
        saved("v1", "Mine", Value::Null),
        saved("v2", "Team", Value::Null),
    ];
    let (mut a, m) = start(mock, false);
    a.pick_view();
    assert_eq!(
        picker_labels(&a),
        ("Tickets views".into(), vec!["Mine".into(), "Team".into()])
    );
    answer(&mut a, ModalResult::Choice(json!(1)));
    assert_eq!(a.view.get("tickets").map(|v| v.id.as_str()), Some("v2"));
    assert!(
        m.calls("list")
            .last()
            .unwrap()
            .contains("view=Some(\"v2\")")
    );
    a.search_mode = true;
    a.pick_view();
    assert!(a.modals.is_empty());
}

#[test]
fn pick_view_without_views_warns() {
    let (mut a, _) = app(1);
    a.pick_view();
    assert_eq!(
        toasts(&a),
        vec![(
            "No saved views on this account for tickets".into(),
            Severity::Warning
        )]
    );
}

// ── forms ───────────────────────────────────────────────────

#[test]
fn open_form_create_choice_and_launch_create() {
    let (mut a, _) = app(1);
    a.open_form("new");
    assert_eq!(
        picker_labels(&a),
        (
            "Create".into(),
            vec!["Change".into(), "Incident".into(), "Service Request".into()]
        )
    );
    a.modals.clear();
    a.create_choice("change");
    assert_eq!(last_toast(&a), "form UI not available");
    a.selected = None;
    a.open_form("edit");
    assert_eq!(last_toast(&a), "Select a change to edit");
    a.selected = Some(change(3));
    a.open_form("edit");
    assert_eq!(last_toast(&a), "form UI not available");
    a.read_only = true;
    a.open_form("new");
    assert_eq!(last_toast(&a), "Read-only mode: write actions are disabled");
    let n = a.toasts.len();
    a.launch_create("incident", None);
    a.create_choice("incident");
    assert_eq!(a.toasts.len(), n);
}

#[test]
fn load_catalog_and_pick_catalog() {
    let (mut a, _) = app(1);
    a.create_choice("sr");
    a.settle();
    assert_eq!(
        picker_labels(&a),
        (
            "Service catalog — category".into(),
            vec!["All categories".into(), "Hardware".into(), "2".into()]
        )
    );
    answer(&mut a, ModalResult::Choice(json!("all")));
    assert_eq!(
        picker_labels(&a),
        (
            "Service catalog — search items".into(),
            vec!["#4 Laptop · Hardware".into(), "#5 Badge · ".into()]
        )
    );
    a.modals.clear();
    a.pick_catalog(Vec::new(), Vec::new());
    assert_eq!(picker_labels(&a).1, vec!["All categories"]);

    let mut mock = Mock::new(1);
    mock.fails = vec!["catalog"];
    let (mut b, _) = start(mock, false);
    b.load_catalog();
    b.settle();
    assert_eq!(failure(&b).message, "Catalog unavailable: catalog denied");
}

#[test]
fn show_failure_remembers_the_last_failure() {
    let (mut a, _) = app(1);
    a.show_failure("boom".into());
    assert_eq!(a.write_failure.as_deref(), Some("boom"));
    assert_eq!(failure(&a).message, "boom");
}

// ── write actions ───────────────────────────────────────────

#[test]
fn reply_possible_write_key_and_target_label() {
    let (mut a, _) = app(1);
    assert!(a.reply_possible());
    assert_eq!(a.target_label(&a.selected.clone().unwrap()), "INC-1");
    a.read_only = true;
    assert!(!a.reply_possible());
    a.write_key("s");
    assert_eq!(last_toast(&a), "Read-only mode: write actions are disabled");
    a.read_only = false;
    a.selected = None;
    a.write_key("n");
    assert_eq!(last_toast(&a), "No row selected");
}

#[test]
fn start_status_confirms_then_do_write_once() {
    let (mut a, m) = app(1);
    let item = a.selected.clone().unwrap();
    a.start_status(item);
    answer(&mut a, ModalResult::Status(json!(2), "Pending".into()));
    assert_eq!(confirm(&a).title, "Review exact ticket property");
    assert_eq!(m.calls("property-plan"), vec!["property-plan 1 status 2"]);
    answer(&mut a, ModalResult::Bool(true));
    a.settle();
    assert_eq!(m.calls("update"), Vec::<String>::new());
    assert_eq!(m.calls("property-send").len(), 1);
    assert_eq!(last_toast(&a), "Ticket property verified");
}

#[test]
fn start_status_unavailable_and_without_choices() {
    let (mut a, _) = app(1);
    a.start_status(Item::new(&PROBLEMS, json!({"id": 1})));
    assert_eq!(
        last_toast(&a),
        "Status change is not available for problems"
    );
    a.schemas.insert("changes", json!({"fields": []}));
    a.start_status(change(1));
    assert_eq!(last_toast(&a), "No status choices in schema");
}

#[test]
fn start_note_and_review_ticket_note() {
    let (mut a, m) = app(1);
    a.start_note(Item::new(&PROBLEMS, json!({"id": 1})), true);
    assert_eq!(last_toast(&a), "Not available for this item type");
    let t = a.selected.clone().unwrap();
    a.start_note(t.clone(), false);
    assert!(matches!(top(&a), Modal::Note(n) if n.target == "INC-1"));
    answer(&mut a, ModalResult::Note("hello".into(), false));
    assert_eq!(confirm(&a).title, "Review exact ticket note");
    assert_eq!(confirm(&a).lines, vec!["{\n  \"body\": \"hello\"\n}"]);
    assert_eq!(
        m.calls("ticket-note-plan"),
        vec!["ticket-note-plan 1 hello false"]
    );

    let mut mock = Mock::new(1);
    mock.fails = vec!["ticket-note"];
    let (mut b, _) = start(mock, false);
    b.review_ticket_note(t, "x".into(), false);
    assert_eq!(failure(&b).message, "ticket-note denied");
}

#[test]
fn do_write_messages_and_failures() {
    let (mut a, m) = on_change(Mock::new(1));
    let c = a.selected.clone().unwrap();
    a.do_write(WriteKind::Note("n1".into(), true), c.clone());
    assert!(
        failure(&a)
            .message
            .contains("Legacy TUI send path disabled")
    );
    a.modals.clear();
    a.do_write(WriteKind::Status(json!(2), "Pending".into()), c);
    assert!(failure(&a).message.contains("Nothing sent."));
    assert!(m.calls("note").is_empty());
    assert!(m.calls("update").is_empty());
}

#[test]
fn after_write_evicts_caches_and_reloads_the_selected_row() {
    let (mut a, m) = app(2);
    let sel = a.selected.clone().unwrap();
    a.sub_cache
        .insert("tickets:1:notes".into(), TabData::default());
    a.sub_cache
        .insert("tickets:2:notes".into(), TabData::default());
    a.after_write(&TICKETS, sel, "Done");
    assert_eq!(last_toast(&a), "Done");
    assert!(!a.detail_cache.contains_key("tickets:1"));
    assert_eq!(
        a.sub_cache.keys().collect::<Vec<_>>(),
        vec!["tickets:2:notes"]
    );
    a.settle();
    assert_eq!(m.calls("detail tickets 1").len(), 2);
    a.after_write(&TICKETS, Item::new(&TICKETS, json!({"id": 2})), "Other");
    a.settle();
    assert_eq!(m.calls("detail tickets 2").len(), 0);
}

#[test]
fn open_browser_and_yank() {
    let (mut a, m) = app(1);
    a.open_browser();
    a.yank();
    assert_eq!(m.calls("open"), vec!["open https://acme.test/a/tickets/1"]);
    assert_eq!(last_toast(&a), "Copied INC-1");
    let mut mock = Mock::new(1);
    mock.open_ok = false;
    mock.fails = vec!["copy"];
    let (mut b, _) = start(mock, false);
    b.open_browser();
    assert_eq!(last_toast(&b), "Browser open failed");
    b.yank();
    assert_eq!(last_toast(&b), "Copy failed: copy denied");
    b.selected = None;
    b.open_browser();
    b.yank();
    assert_eq!(
        toasts(&b)[2..],
        [
            ("No row selected".to_string(), Severity::Warning),
            ("No row selected".to_string(), Severity::Warning)
        ]
    );
}

// ── `a` actions ─────────────────────────────────────────────

#[test]
fn action_menu_choices_and_guards() {
    let (mut a, _) = app(1);
    a.action_menu();
    assert_eq!(
        picker_labels(&a),
        (
            "Selected record actions".into(),
            vec!["Download selected attachment".into()]
        )
    );
    a.modals.clear();
    a.selected = Some(change(1));
    a.action_menu();
    assert_eq!(
        picker_labels(&a).1,
        vec![
            "Add task",
            "Update selected task",
            "Delete selected task",
            "Link initiating ticket",
            "Unlink initiating ticket",
            "Request approval",
            "Respond to approval",
            "Cancel approval group",
            "List approval groups",
            "Download selected attachment",
            "Edit native planning/analysis text",
        ]
    );
    a.modals.clear();
    a.selected = Some(Item::new(&PROBLEMS, json!({"id": 4, "display_id": 4})));
    a.action_menu();
    assert_eq!(
        picker_labels(&a).1,
        vec![
            "Download selected attachment",
            "Edit native planning/analysis text",
            "Mark as known error",
        ]
    );
    a.modals.clear();
    a.selected = None;
    a.action_menu();
    assert_eq!(last_toast(&a), "No row selected");
    a.read_only = true;
    a.action_menu();
    assert_eq!(last_toast(&a), "Read-only mode: actions are disabled");
}

#[test]
fn prepare_action_routes_each_kind() {
    let (mut a, m) = app(1);
    let c = change(11);
    a.prepare_action("task-add", c.clone());
    assert_eq!(
        value_title(&a),
        "New change task title — Open; reminder Never"
    );
    a.modals.clear();
    a.prepare_action("approval-request", c.clone());
    assert_eq!(value_title(&a), "Approval group name");
    a.modals.clear();
    a.prepare_action("link", c.clone());
    assert_eq!(value_title(&a), "Search initiating tickets (ID or subject)");
    a.modals.clear();
    a.prepare_action("approval-groups", c.clone());
    a.settle();
    assert_eq!(
        m.calls("approval-targets"),
        vec!["approval-targets approval-groups 11"]
    );
    a.modals.clear();
    a.compose_change_task(Item::new(&TICKETS, json!({"id": 1})));
    assert!(a.modals.is_empty(), "tasks are changes-only");
    a.read_only = true;
    a.prepare_action("task-add", c);
    assert!(a.modals.is_empty());
}

#[test]
fn change_task_options_prepare_review_and_send() {
    let (mut a, m) = on_change(Mock::new(1));
    let c = a.selected.clone().unwrap();
    a.compose_change_task(c);
    answer(&mut a, ModalResult::Text("Acme task".into()));
    assert_eq!(
        picker_labels(&a),
        (
            "Task owner".into(),
            vec![
                "No owner (default)".into(),
                "Explicit owner user ID or email".into()
            ]
        )
    );
    answer(&mut a, ModalResult::Choice(json!("none")));
    let (title, labels) = picker_labels(&a);
    assert_eq!(
        title,
        "Task optional fields — defaults: the form's first status / reminder; others omitted"
    );
    assert_eq!(
        labels,
        vec![
            "Review exact task",
            "Group ID or exact name: default / omitted",
            "Status: default / omitted",
            "Note (plain text): default / omitted",
            "Notify Before: default / omitted",
            "Planned start (ISO with offset): default / omitted",
            "Planned end (ISO with offset): default / omitted",
            "Planned effort (1h30m or 01:30): default / omitted",
            "Clear optional fields",
        ]
    );
    answer(&mut a, ModalResult::Choice(json!("status")));
    assert_eq!(
        picker_labels(&a),
        (
            "Task status".into(),
            vec![
                "Open".into(),
                "In Progress".into(),
                "Completed".into(),
                "Blocked".into()
            ]
        )
    );
    answer(&mut a, ModalResult::Choice(json!("In Progress")));
    assert_eq!(picker_labels(&a).1[2], "Status: In Progress");
    answer(&mut a, ModalResult::Choice(json!("note")));
    assert_eq!(value_title(&a), "Note (plain text)");
    answer(&mut a, ModalResult::Text("Draft".into()));
    answer(&mut a, ModalResult::Choice(json!("review")));
    assert_eq!(
        m.calls("task-plan"),
        vec![r#"task-plan 1 Acme task None [("status", "In Progress"), ("note", "Draft")]"#]
    );
    assert_eq!(confirm(&a).title, "Review exact new change task");
    assert_eq!(confirm(&a).lines, vec!["{\n  \"title\": \"Acme task\"\n}"]);
    answer(&mut a, ModalResult::Bool(true));
    assert_eq!(m.calls("task-send").len(), 1);
    assert_eq!(last_toast(&a), "Task 501 verified");
}

#[test]
fn change_task_status_picker_uses_runtime_labels() {
    let mock = Mock {
        status_labels: vec!["Todo".into(), "Doing".into(), "Done".into()],
        notify_labels: vec!["None".into(), "10 Min".into()],
        ..Mock::new(1)
    };
    let (mut a, m) = on_change(mock);
    let c = a.selected.clone().unwrap();
    a.compose_change_task(c);
    answer(&mut a, ModalResult::Text("Acme task".into()));
    answer(&mut a, ModalResult::Choice(json!("none")));
    answer(&mut a, ModalResult::Choice(json!("status")));
    assert_eq!(
        picker_labels(&a),
        (
            "Task status".into(),
            vec!["Todo".into(), "Doing".into(), "Done".into()]
        )
    );
    assert_eq!(m.calls("task-choices"), vec!["task-choices status"]);
    answer(&mut a, ModalResult::Choice(json!("Todo")));
    answer(&mut a, ModalResult::Choice(json!("notify_before")));
    assert_eq!(
        picker_labels(&a).1,
        vec!["None".to_string(), "10 Min".to_string()]
    );
}

#[test]
fn change_task_clear_explicit_owner_and_failures() {
    let (mut a, m) = app(1);
    let c = change(11);
    a.task_titled(c.clone(), "T".into());
    answer(&mut a, ModalResult::Choice(json!("explicit")));
    assert_eq!(
        value_title(&a),
        "Explicit task owner user ID or exact email (no implicit self)"
    );
    answer(&mut a, ModalResult::Text("agent@acme.test".into()));
    answer(&mut a, ModalResult::Choice(json!("notify_before")));
    assert_eq!(
        picker_labels(&a).1,
        vec![
            "Never",
            "15 Minutes",
            "30 Minutes",
            "45 Minutes",
            "1 Hour",
            "2 Hours"
        ]
    );
    answer(&mut a, ModalResult::Choice(json!("45 Minutes")));
    answer(&mut a, ModalResult::Choice(json!("clear")));
    assert_eq!(picker_labels(&a).1[4], "Notify Before: default / omitted");
    answer(&mut a, ModalResult::Choice(json!("review")));
    assert_eq!(
        m.calls("task-plan"),
        vec!["task-plan 11 T Some(\"agent@acme.test\") []"]
    );

    let mut mock = Mock::new(1);
    mock.fails = vec!["task-plan"];
    mock.task_result = Some(
        json!({"task_id": 9, "verified": true, "unverified_fields": ["status"], "warning": "Do not resend"}),
    );
    let (mut b, _) = start(mock, false);
    b.prepare_change_task(c.clone(), "T".into(), None, Vec::new());
    b.settle();
    assert!(failure(&b).message.starts_with("task-plan denied"));
    b.modals.clear();
    b.send_change_task(c.clone(), plan(json!({})));
    b.settle();
    assert_eq!(
        failure(&b).message,
        "Task ID 9 submitted; readback incomplete or unverified.\nDo not resend\nDo not resend."
    );
    b.modals.clear();
    b.read_only = true;
    b.review_change_task(c.clone(), plan(json!({})));
    b.change_task_options(c, "T".into(), None, Vec::new());
    assert!(b.modals.is_empty());
}

#[test]
fn approval_request_compose_prepare_review_send_and_result() {
    let (mut a, m) = on_change(Mock::new(1));
    let c = a.selected.clone().unwrap();
    a.compose_approval_request(c.clone());
    answer(&mut a, ModalResult::Text("CAB".into()));
    assert_eq!(value_title(&a), "Approval message");
    answer(&mut a, ModalResult::Text("please".into()));
    assert_eq!(
        value_title(&a),
        "Explicit approver IDs or exact emails, comma separated"
    );
    answer(&mut a, ModalResult::Text("a@x.io, 42 ".into()));
    let (title, rules) = picker_labels(&a);
    assert_eq!(title, "Approval rule");
    assert_eq!(
        rules,
        site::approval_rules()
            .iter()
            .map(|(_, l)| l.to_string())
            .collect::<Vec<_>>()
    );
    let key = site::approval_rules()[0].0;
    answer(&mut a, ModalResult::Choice(json!(key)));
    assert_eq!(
        m.calls("approval-plan"),
        vec![format!(
            "approval-plan 1 CAB please [\"a@x.io\", \"42\"] {key}"
        )]
    );
    assert_eq!(confirm(&a).title, "Review exact approval action");
    assert_eq!(confirm(&a).lines.len(), 1);
    a.prepare_approval_request(c.clone(), "n".into(), "m".into(), "1".into(), key.into());
    a.settle();
    a.modals.clear();
    a.approval_result(c, json!({"verified": true}));
    assert_eq!(
        last_toast(&a),
        "Approval submitted; inspect verification details"
    );
    assert_eq!(failure(&a).title, "Approval result — do not resend");
    assert_eq!(failure(&a).message, "{\n  \"verified\": true\n}");
}

#[test]
fn approval_targets_pick_action_and_review() {
    let (mut a, m) = app(1);
    let c = change(11);
    a.load_approval_targets("approval-respond".into(), c.clone());
    a.settle();
    assert_eq!(
        picker_labels(&a),
        ("Approval records".into(), vec!["{\"id\": 31}".into()])
    );
    a.modals.clear();
    a.pick_approval_target("approval-cancel".into(), c.clone(), Vec::new());
    assert_eq!(last_toast(&a), "No approval records or groups");
    a.pick_approval_target("approval-cancel".into(), c.clone(), vec![json!({"id": 32})]);
    assert_eq!(picker_labels(&a).0, "Approval groups");
    answer(&mut a, ModalResult::Choice(json!({"id": 32})));
    assert_eq!(
        m.calls("approval-action"),
        vec!["approval-action 11 32 cancel None"]
    );
    assert_eq!(
        confirm(&a).lines,
        vec![
            "{\n  \"op\": \"cancel\"\n}".to_string(),
            "Selected inventory row: {\n  \"id\": 32\n}".into()
        ]
    );
    a.modals.clear();
    a.prepare_approval_action(c.clone(), json!({"id": 33}), "reject", Some("no"));
    assert_eq!(
        m.calls("approval-action")[1],
        "approval-action 11 33 reject Some(\"no\")"
    );
    a.modals.clear();
    a.review_approval(c, plan(json!({})), None);
    assert_eq!(confirm(&a).lines.len(), 1);

    let mut mock = Mock::new(1);
    mock.fails = vec!["approvals"];
    let (mut b, _) = start(mock, false);
    b.load_approval_targets("approval-groups".into(), change(11));
    b.settle();
    assert_eq!(
        failure(&b).message,
        "Approval inventory unavailable: approvals denied"
    );
}

#[test]
fn load_and_pick_action_targets() {
    let mock = Mock::new(1);
    *mock.tasks.lock().unwrap() = vec![json!({"id": 600, "title": "Acme task", "status": 1})];
    let (mut a, m) = start(mock, false);
    let c = change(11);
    a.load_action_targets("task-delete".into(), c.clone(), String::new());
    a.settle();
    assert_eq!(m.calls("tab"), vec!["tab changes 11 tasks"]);
    assert_eq!(
        picker_labels(&a),
        ("task-delete".into(), vec!["600: Acme task ".into()])
    );
    a.modals.clear();
    a.load_action_targets("link".into(), c.clone(), "Acme".into());
    a.settle();
    assert_eq!(m.calls("ticket-search"), vec!["ticket-search Acme"]);
    assert_eq!(picker_labels(&a).1, vec!["90: Acme access "]);
    a.modals.clear();
    a.load_action_targets("download".into(), c.clone(), String::new());
    a.settle();
    assert_eq!(picker_labels(&a).1, vec!["70: report.pdf request"]);
    a.modals.clear();
    a.pick_action_target(
        "unlink".into(),
        c.clone(),
        vec![json!({"id": 1, "subject": "S"})],
        vec!["page two denied".into()],
    );
    assert_eq!(
        picker_labels(&a).0,
        "unlink — partial inventory; inspect warnings"
    );
    let t = a.toasts.last().unwrap();
    assert_eq!(
        (t.message.as_str(), t.expires),
        ("page two denied", a.now() + Duration::from_secs(15))
    );
    a.modals.clear();
    a.pick_action_target("unlink".into(), c, Vec::new(), vec!["w1".into()]);
    assert_eq!(failure(&a).message, "No available targets.\nw1");
}

#[test]
fn load_action_targets_errors_are_failures() {
    let mut mock = Mock::new(1);
    mock.fails = vec!["tab", "downloads"];
    let (mut a, _) = start(mock, false);
    a.load_action_targets("task-update".into(), change(11), String::new());
    a.settle();
    assert_eq!(
        failure(&a).message,
        "Could not load action targets: tab denied"
    );
    a.modals.clear();
    a.load_action_targets("download".into(), change(11), String::new());
    a.settle();
    assert_eq!(
        failure(&a).message,
        "Could not load action targets: downloads denied"
    );
}

#[test]
fn load_task_editor_edit_task_field_and_confirm_action() {
    let (mut a, m) = app(1);
    let c = change(11);
    let target = json!({"id": 600, "title": "Acme task"});
    a.load_task_editor(c.clone(), target.clone());
    a.settle();
    assert_eq!(m.calls("task-edit"), vec!["task-edit 11 600"]);
    assert_eq!(
        picker_labels(&a),
        (
            "Update task 600 — choose one changed field".into(),
            vec![
                "Title".into(),
                "Status".into(),
                "Group ID".into(),
                "Agent ID".into()
            ]
        )
    );
    answer(&mut a, ModalResult::Choice(json!("title")));
    assert_eq!(value_title(&a), "Task 600: title");
    answer(&mut a, ModalResult::Text("Old".into()));
    assert_eq!(last_toast(&a), "No change; nothing sent");
    a.edit_task_field(
        c.clone(),
        target.clone(),
        json!({"owner_id": 1}),
        Vec::new(),
    );
    answer(&mut a, ModalResult::Choice(json!("owner_id")));
    answer(&mut a, ModalResult::Text("x".into()));
    assert_eq!(failure(&a).message, "Enter a positive numeric ID");
    a.modals.clear();
    a.edit_task_field(c, target, json!({"owner_id": 1}), Vec::new());
    answer(&mut a, ModalResult::Choice(json!("owner_id")));
    answer(&mut a, ModalResult::Text("5".into()));
    assert_eq!(confirm(&a).title, "Review exact task-update");
    assert!(confirm(&a).lines[0].contains("task-update"));
    assert_eq!(
        m.calls("task-mutation-plan"),
        vec!["task-mutation-plan task-update 11 600 {\"owner_id\":5}"]
    );

    let mut mock = Mock::new(1);
    mock.fails = vec!["task-edit"];
    let (mut b, _) = start(mock, false);
    b.load_task_editor(change(11), json!({"id": 1}));
    b.settle();
    assert_eq!(failure(&b).message, "Task unavailable: task-edit denied");
}

#[test]
fn confirm_action_link_reviews_the_association_plan() {
    let (mut a, m) = app(1);
    let target = json!({"id": 900, "display_id": 90, "subject": "Acme access", "_inventory_warnings": ["partial"]});
    a.confirm_action("link".into(), change(11), target, Map::new());
    assert_eq!(
        m.calls("assoc-plan"),
        vec!["assoc-plan 11 [Number(900)] false"]
    );
    assert_eq!(
        confirm(&a).lines,
        vec![
            "Record: CHN-11 (ID 11)",
            "Target: 900 — Acme access",
            "Action: link",
            "{}",
            "Initiating-ticket relationship only.",
            "{\n  \"change_id\": 11,\n  \"unlink\": false\n}",
            "partial",
        ]
    );
    a.modals.clear();
    a.confirm_action(
        "download".into(),
        change(11),
        json!({"id": 70, "name": "r.pdf"}),
        Map::new(),
    );
    assert_eq!(
        confirm(&a).lines[4],
        "Source: request. Existing files are skipped; no overwrite."
    );
}

#[test]
fn execute_action_sends_once_and_reads_back() {
    let mock = Mock::new(1);
    *mock.tasks.lock().unwrap() = vec![json!({"id": 600, "status": 3})];
    let (mut a, m) = start(mock, false);
    let c = change(11);
    let mut body = Map::new();
    body.insert("status".into(), json!(3));
    a.execute_action(
        "task-update".into(),
        c.clone(),
        json!({"id": 600}),
        body.clone(),
        None,
    );
    a.settle();
    assert!(failure(&a).message.contains("exact reviewed task plan"));
    assert!(m.calls("task-update").is_empty());
    a.modals.clear();
    a.execute_action(
        "task-delete".into(),
        c.clone(),
        json!({"id": 600}),
        Map::new(),
        None,
    );
    a.settle();
    assert!(failure(&a).message.contains("exact reviewed task plan"));
    assert!(m.calls("task-delete").is_empty());
    a.modals.clear();
    let reviewed = plan(json!({"change_id": 11, "unlink": false}));
    a.execute_action(
        "link".into(),
        c,
        json!({"id": 900}),
        Map::new(),
        Some(reviewed),
    );
    a.settle();
    assert_eq!(m.calls("assoc-send").len(), 1);
    assert_eq!(last_toast(&a), "link complete inventory verified");
}

#[test]
fn prepare_download_collision_and_unsafe_names() {
    let dir = std::env::temp_dir().join(format!("fsv-tui-unit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("taken.txt"), b"x").unwrap();
    let d = dir.to_string_lossy().to_string();
    let (mut a, _) = app(1);
    a.prepare_download(
        change(11),
        json!({"id": 70, "name": "taken.txt"}),
        d.clone(),
        None,
    );
    assert_eq!(
        picker_labels(&a),
        (
            format!(
                "File exists: {} — no overwrite",
                dir.join("taken.txt").display()
            ),
            vec!["Skip download".into(), "Choose another filename".into()]
        )
    );
    answer(&mut a, ModalResult::Choice(json!("rename")));
    assert_eq!(value_title(&a), "Save as filename (no directories)");
    answer(&mut a, ModalResult::Text("new.txt".into()));
    assert_eq!(
        confirm(&a).lines[3],
        format!("{{\"destination\": \"{d}\"}}")
    );
    a.modals.clear();
    a.prepare_download(
        change(11),
        json!({"id": 70}),
        d.clone(),
        Some("../x".into()),
    );
    assert_eq!(
        failure(&a).message,
        "unsafe file name '../x': contains '..'"
    );
    a.modals.clear();
    a.prepare_download(change(11), json!({"id": 71}), d, None);
    assert_eq!(confirm(&a).lines[1], "Target: 71 — attachment-71");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ── keys / framework seams ──────────────────────────────────

#[test]
fn handle_key_routes_modal_inputs_then_main_keys() {
    let (mut a, _) = app(2);
    a.handle_key(&keys::parse("question_mark"));
    assert!(matches!(top(&a), Modal::Help));
    a.handle_key(&keys::parse("j"));
    assert!(a.modals.is_empty());
    a.handle_key(&keys::parse("j"));
    assert_eq!(a.cursor, 1);
    a.handle_key(&keys::parse("f"));
    a.handle_key(&keys::parse("escape"));
    assert_eq!((a.focus, a.search_mode), (Focus::List, false));
}

#[test]
fn backend_trait_defaults_are_unavailable_not_panics() {
    struct Bare;
    impl Backend for Bare {
        crate::backend_stubs!(
            load_schema,
            current_user,
            load_views,
            ticket_headers,
            view_settings,
            build_query_hash,
            list_page,
            search_items,
            load_detail,
            load_tab,
            remote_user_values,
            lookup_choices,
            update_item,
            add_note,
            ticket_note_plan,
            send_ticket_note,
            change_task_plan,
            send_change_task,
            search_change_tickets,
            download_choices,
            download,
            task_for_edit,
            association_plan,
            send_association,
            approval_request_plan,
            approval_targets,
            approval_action_plan,
            send_approval,
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
    let b = Bare;
    assert!(!b.completion_network());
    assert!(b.user_tz(&json!({"time_zone": "Bangkok"})).is_none());
    assert_eq!(
        b.view_settings(&TICKETS, &saved("v", "V", Value::Null))
            .ok(),
        Some(Value::Null)
    );
    assert_eq!(
        b.send_change_task(&plan(json!({})), false)
            .map_err(|e| e.to_string())
            .err()
            .as_deref(),
        Some("change tasks is not available in this build")
    );
    let _: &Resource = &TICKETS;
}

#[test]
fn reviewed_write_defaults_are_unavailable() {
    struct Bare;
    impl Backend for Bare {
        crate::backend_stubs!(
            load_schema,
            current_user,
            load_views,
            ticket_headers,
            view_settings,
            build_query_hash,
            list_page,
            search_items,
            load_detail,
            load_tab,
            remote_user_values,
            lookup_choices,
            update_item,
            add_note,
            ticket_note_plan,
            send_ticket_note,
            change_task_plan,
            send_change_task,
            search_change_tickets,
            download_choices,
            download,
            task_for_edit,
            association_plan,
            send_association,
            approval_request_plan,
            approval_targets,
            approval_action_plan,
            send_approval,
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
    let p = plan(json!({}));
    let msgs = [
        Bare.ticket_property_fields(&json!(1))
            .err()
            .map(|e| e.to_string()),
        Bare.send_known_error(&p).err().map(|e| e.to_string()),
        Bare.planning_definitions("changes")
            .err()
            .map(|e| e.to_string()),
        Bare.send_ticket_conversation(&p, true)
            .err()
            .map(|e| e.to_string()),
    ];
    assert_eq!(
        msgs.map(Option::unwrap_or_default),
        [
            "ticket property edits is not available in this build",
            "known-error marks is not available in this build",
            "planning text is not available in this build",
            "ticket conversations is not available in this build",
        ]
        .map(String::from)
    );
}

/// `test_read_only_blocks_ticket_options_sender` and
/// `test_readonly_property_picker_never_prepares`: no picker, no review.
#[test]
fn read_only_never_prepares_reviewed_writes() {
    let (mut a, m) = start(Mock::new(1), true);
    let t = Item::new(&TICKETS, json!({"id": 123}));
    let p = Item::new(&PROBLEMS, json!({"id": 5}));
    a.ticket_conversation_options(t.clone(), "Acme".into(), true, true, Default::default());
    a.prepare_ticket_conversation(t.clone(), "Acme".into(), true, true, Default::default());
    a.load_ticket_property_fields(t.clone());
    a.prepare_ticket_property(t, "subject".into(), json!("Acme"));
    a.prepare_known_error(p.clone());
    a.load_planning_sections(p);
    a.settle();
    assert!(a.modals.is_empty());
    assert!(m.calls("detail").len() <= 1);
}

/// A `FormScreen` that is already done with the given action.
struct DoneForm {
    action: &'static str,
    sent: Arc<Mutex<Vec<String>>>,
}

impl FormView for DoneForm {
    fn tick(&mut self, _now: Duration) {}
    fn handle_key(&mut self, _key: &KeyEvent) {}
    fn render(
        &mut self,
        _area: crate::backend::Rect,
        _buf: &mut crate::backend::Buffer,
    ) -> Option<crate::backend::Position> {
        None
    }
    fn done(&self) -> bool {
        true
    }
    fn finish(self: Box<Self>) -> FormExit {
        match self.action {
            "save_draft" => FormExit::Draft("/tmp/acme-draft.yaml".into()),
            "submit" => FormExit::Submit(Box::new(DoneSend(self.sent))),
            _ => FormExit::Cancelled,
        }
    }
}

struct DoneSend(Arc<Mutex<Vec<String>>>);

impl FormSend for DoneSend {
    fn send(self: Box<Self>) -> BResult<Value> {
        self.0.lock().unwrap().push("send".into());
        Ok(json!({"created": {"id": 99, "display_id": 12}}))
    }
}

fn opened(
    action: &'static str,
    mode: &str,
    item: Option<Item>,
) -> (OpenedForm, Arc<Mutex<Vec<String>>>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let view = Box::new(DoneForm {
        action,
        sent: sent.clone(),
    });
    (
        OpenedForm {
            view,
            mode: mode.into(),
            item,
        },
        sent,
    )
}

/// `_build_form` failure toasts "Form unavailable: …"; `_push_form`'s
/// `done` toasts a draft path or hands a submit to `_send_form`, which
/// toasts `form_result` for a create and reloads the change for an edit.
#[test]
fn build_form_push_form_and_send_form() {
    let (mut a, _) = app(1);
    a.build_form("new", None, None);
    a.settle();
    assert_eq!(
        failure(&a).message,
        "Form unavailable: the form UI is not available in this build"
    );
    a.modals.clear();

    let (f, sent) = opened("save_draft", "incident", None);
    a.push_form(f);
    assert!(!a.form_open());
    assert_eq!(last_toast(&a), "Draft saved: /tmp/acme-draft.yaml");
    assert!(sent.lock().unwrap().is_empty());

    let (f, sent) = opened("cancel", "new", None);
    let n = a.toasts.len();
    a.push_form(f);
    a.settle();
    assert_eq!((a.toasts.len(), sent.lock().unwrap().len()), (n, 0));

    let (f, sent) = opened("submit", "new", None);
    a.push_form(f);
    a.settle();
    assert_eq!(*sent.lock().unwrap(), ["send"]);
    assert!(toasts(&a).contains(&("Change created CHN-12".into(), Severity::Information)));

    let (f, sent) = opened("submit", "edit", Some(change(3)));
    a.push_form(f);
    a.settle();
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(last_toast(&a), "Change updated");

    a.read_only = true;
    let (f, sent) = opened("submit", "new", None);
    a.push_form(f);
    a.settle();
    assert!(sent.lock().unwrap().is_empty());
}

/// `_send_form` directly: a create toasts the new id; read-only never sends.
#[test]
fn send_form_sends_once_unless_read_only() {
    let (mut a, _) = app(1);
    let sent = Arc::new(Mutex::new(Vec::new()));
    a.send_form(Box::new(DoneSend(sent.clone())), "incident".into(), None);
    a.settle();
    assert_eq!(*sent.lock().unwrap(), ["send"]);
    assert!(toasts(&a).contains(&("Incident created INC-12".into(), Severity::Information)));
    a.read_only = true;
    a.send_form(Box::new(DoneSend(sent.clone())), "new".into(), None);
    a.settle();
    assert_eq!(sent.lock().unwrap().len(), 1);
}

fn review_plan() -> crate::backend::ReviewPlan {
    crate::backend::ReviewPlan {
        review: json!({}),
        hash: "h".into(),
        public: false,
        target_id: json!(11),
        raw: json!({}),
    }
}

/// `_prepare_change_status` is `exclusive=True`: the later review replaces the earlier one.
#[test]
fn exclusive_status_review_keeps_only_the_latest() {
    let (mut a, _) = app(1);
    let before = a.modals.len();
    let item = change(11);
    a.prepare_change_status(item.clone(), json!(2));
    a.prepare_change_status(item, json!(3));
    a.settle();
    assert_eq!(a.modals.len(), before + 1);
}

/// `_send_change_status` is not exclusive: both sends still report.
#[test]
fn nonexclusive_status_sends_both_apply() {
    let (mut a, _) = app(1);
    let before = a.modals.len();
    let item = change(11);
    a.send_change_status(item.clone(), review_plan());
    a.send_change_status(item, review_plan());
    a.settle();
    assert_eq!(a.modals.len(), before + 2);
}
