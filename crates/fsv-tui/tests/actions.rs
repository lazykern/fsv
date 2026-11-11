//! The `a` menu's change-task and initiating-ticket actions, end to end:
//! keys in, rendered 120x36 screen and backend calls out. The mock keeps
//! the task / association inventory so readback sees the write
//! (`FsvApp._execute_action`, `_send_change_task`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fsv_tui::app::App;
use fsv_tui::backend::{BResult, DetailPage, ListRequest, ReviewPlan, TabData, TabInfo};
use fsv_tui::modals::Modal;
use fsv_tui::{Backend, BasicPages, view};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Map, Value, json};

#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<String>>,
    tasks: Mutex<Vec<Value>>,
    tickets: Mutex<Vec<Value>>,
}

impl Mock {
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
}

fn plan(review: Value, raw: Value) -> ReviewPlan {
    ReviewPlan {
        hash: "h1".into(),
        review,
        public: false,
        target_id: Value::Null,
        raw,
    }
}

impl Backend for Mock {
    fn load_schema(&self, _res: fsv_tui::backend::Res) -> BResult<Value> {
        Ok(json!({"fields": []}))
    }
    fn list_page(&self, req: &ListRequest) -> BResult<(Vec<Value>, Value)> {
        let base = if req.res.name == "changes" { 10 } else { 0 };
        let rows = (1..=2)
            .map(|i| json!({"id": base + i, "display_id": base + i, "subject": format!("{} row {i}", req.res.name)}))
            .collect();
        Ok((rows, json!({"meta": {"total_count": 2}})))
    }
    fn load_detail(
        &self,
        res: fsv_tui::backend::Res,
        id: &Value,
        _schema: Option<&Value>,
    ) -> BResult<DetailPage> {
        self.log(format!("detail {} {id}", res.name));
        let tab = |n: &str, l: &str| TabInfo {
            name: n.into(),
            label: l.into(),
            count: None,
        };
        Ok(DetailPage {
            res,
            record: json!({"id": id, "display_id": id, "subject": "Acme change"}),
            tabs: vec![
                tab("details", "Details"),
                tab("tasks", "Tasks"),
                tab("associations", "Associations"),
            ],
            badge: "Open".into(),
            due: String::new(),
            tz: None,
            schema: None,
            extra: Value::Null,
        })
    }
    fn load_tab(&self, res: fsv_tui::backend::Res, record: &Value, tab: &str) -> BResult<TabData> {
        self.log(format!("tab {} {} {tab}", res.name, record["id"]));
        let mut t = TabData {
            name: tab.into(),
            label: tab.into(),
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
        Ok(plan(
            json!({"change_id": change_id, "title": title}),
            json!({"title": title}),
        ))
    }
    fn send_change_task(&self, p: &ReviewPlan, read_only: bool) -> BResult<Value> {
        self.log(format!("task-send {} {read_only}", p.hash));
        self.tasks
            .lock()
            .unwrap()
            .push(json!({"id": 600, "title": p.raw["title"], "status": 1}));
        Ok(json!({"task_id": 600, "verified": true}))
    }
    fn task_for_edit(&self, change_id: &Value, task_id: &Value) -> BResult<(Value, Vec<Value>)> {
        self.log(format!("task-edit {change_id} {task_id}"));
        let tasks = self.tasks.lock().unwrap();
        let t = tasks.iter().find(|t| &t["id"] == task_id).cloned();
        Ok((t.unwrap_or(Value::Null), Vec::new()))
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
        for t in self.tasks.lock().unwrap().iter_mut() {
            if &t["id"] == task_id {
                for (k, v) in body {
                    t[k] = v.clone();
                }
            }
        }
        Ok(())
    }
    fn delete_task(&self, change_id: &Value, task_id: &Value) -> BResult<()> {
        self.log(format!("task-delete {change_id} {task_id}"));
        self.tasks.lock().unwrap().retain(|t| &t["id"] != task_id);
        Ok(())
    }
    fn search_change_tickets(&self, query: &str) -> BResult<Vec<Value>> {
        self.log(format!("ticket-search {query}"));
        Ok(vec![
            json!({"id": 900, "display_id": 90, "subject": "Acme access"}),
        ])
    }
    fn association_plan(
        &self,
        change_id: &Value,
        ticket_ids: &[Value],
        unlink: bool,
    ) -> BResult<ReviewPlan> {
        self.log(format!("assoc-plan {change_id} {ticket_ids:?} {unlink}"));
        Ok(plan(
            json!({"change_id": change_id, "ticket_ids": ticket_ids, "unlink": unlink}),
            json!({"ids": ticket_ids, "unlink": unlink}),
        ))
    }
    fn send_association(&self, p: &ReviewPlan) -> BResult<Value> {
        self.log(format!("assoc-send {}", p.raw));
        let mut tickets = self.tickets.lock().unwrap();
        if p.raw["unlink"] == json!(true) {
            tickets.retain(|t| !p.raw["ids"].as_array().unwrap().contains(&t["id"]));
        } else {
            tickets.push(json!({"id": 900, "display_id": 90, "subject": "Acme access"}));
        }
        Ok(json!({"verified": true}))
    }
}

fn change_app() -> (App, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    let mut app = App::new_inline(mock.clone(), Arc::new(BasicPages), false);
    app.mount();
    app.settle();
    app.advance(Duration::from_millis(200));
    app.press("g");
    app.press("c");
    app.advance(Duration::from_millis(250));
    app.advance(Duration::from_millis(200));
    assert_eq!(
        app.selected.as_ref().map(|s| s.key()).as_deref(),
        Some("changes:11")
    );
    (app, mock)
}

fn screen(app: &App) -> String {
    let mut term = Terminal::new(TestBackend::new(120, 36)).unwrap();
    term.draw(|f| view::draw(app, f)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn menu_pick(a: &mut App, filter: &str) {
    a.press("a");
    assert!(screen(a).contains("Selected record actions"));
    a.type_text(filter);
    a.press("enter");
    a.settle();
}

fn last_toast(a: &App) -> String {
    a.toasts
        .last()
        .map(|t| t.message.clone())
        .unwrap_or_default()
}

fn add_task(a: &mut App) {
    menu_pick(a, "Add task");
    let s = screen(a);
    assert!(
        s.contains("New change task title — Open; reminder Never"),
        "{s}"
    );
    a.type_text("Acme rollout");
    a.press("enter");
    let s = screen(a);
    assert!(s.contains("Task owner"), "{s}");
    assert!(s.contains("No owner (default)"), "{s}");
    a.press("enter");
    let s = screen(a);
    assert!(
        s.contains("Task optional fields — defaults Open / Never"),
        "{s}"
    );
    a.press("enter");
    a.settle();
    let s = screen(a);
    assert!(s.contains("Review exact new change task"), "{s}");
    assert!(s.contains("\"title\": \"Acme rollout\""), "{s}");
    a.press("y");
    a.settle();
}

#[test]
fn a_menu_task_add_reviews_sends_once_and_reloads() {
    let (mut a, m) = change_app();
    add_task(&mut a);
    assert_eq!(
        m.calls("task-plan"),
        vec!["task-plan 11 Acme rollout None []"]
    );
    assert_eq!(m.calls("task-send"), vec!["task-send h1 false"]);
    assert!(a.modals.is_empty());
    assert_eq!(last_toast(&a), "Task 600 verified");
    assert!(screen(&a).contains("Task 600 verified"));
    assert_eq!(m.calls("detail changes 11").len(), 2, "detail reloaded");
}

#[test]
fn a_menu_task_add_cancel_on_review_sends_nothing() {
    let (mut a, m) = change_app();
    menu_pick(&mut a, "Add task");
    a.type_text("Acme rollout");
    a.press("enter");
    a.press("enter");
    a.press("enter");
    a.settle();
    a.press("escape");
    a.settle();
    assert!(a.modals.is_empty());
    assert!(m.calls("task-send").is_empty());
}

#[test]
fn a_menu_task_update_changes_one_field_and_verifies_readback() {
    let (mut a, m) = change_app();
    add_task(&mut a);
    menu_pick(&mut a, "Update selected task");
    let s = screen(&a);
    assert!(s.contains("task-update"), "{s}");
    assert!(s.contains("600: Acme rollout"), "{s}");
    a.press("enter");
    a.settle();
    assert_eq!(m.calls("task-edit"), vec!["task-edit 11 600"]);
    let s = screen(&a);
    assert!(
        s.contains("Update task 600 — choose one changed field"),
        "{s}"
    );
    a.press("enter");
    let s = screen(&a);
    assert!(s.contains("Task 600: title"), "{s}");
    a.press("ctrl+u");
    a.type_text("Acme rollout v2");
    a.press("enter");
    let s = screen(&a);
    assert!(s.contains("Confirm task-update"), "{s}");
    assert!(s.contains("Action: task-update"), "{s}");
    a.press("y");
    a.settle();
    assert_eq!(
        m.calls("task-update"),
        vec!["task-update 11 600 {\"title\":\"Acme rollout v2\"}"]
    );
    assert_eq!(last_toast(&a), "task-update verified");
    assert_eq!(m.tasks.lock().unwrap()[0]["title"], "Acme rollout v2");
}

#[test]
fn a_menu_task_delete_verifies_removal() {
    let (mut a, m) = change_app();
    add_task(&mut a);
    menu_pick(&mut a, "Delete selected task");
    a.press("enter");
    let s = screen(&a);
    assert!(s.contains("Confirm task-delete"), "{s}");
    assert!(s.contains("Target: 600 — Acme rollout"), "{s}");
    let rows: Vec<&str> = s.lines().collect();
    let at = |y: usize| rows[y].chars().skip(31).take(58).collect::<String>();
    assert_eq!(
        at(10).trim_end(),
        "Assignment/status changes may notify people. Deletion"
    );
    assert_eq!(at(11).trim_end(), "cannot be undone.");
    assert_eq!(at(31).trim_end(), "y confirm · Esc cancel");
    a.press("y");
    a.settle();
    assert_eq!(m.calls("task-delete"), vec!["task-delete 11 600"]);
    assert!(m.tasks.lock().unwrap().is_empty());
    assert_eq!(last_toast(&a), "task-delete verified");
    assert!(screen(&a).contains("task-delete verified"));
}

#[test]
fn a_menu_link_then_unlink_initiating_ticket() {
    let (mut a, m) = change_app();
    menu_pick(&mut a, "Link initiating ticket");
    let s = screen(&a);
    assert!(
        s.contains("Search initiating tickets (ID or subject)"),
        "{s}"
    );
    a.type_text("Acme");
    a.press("enter");
    a.settle();
    assert_eq!(m.calls("ticket-search"), vec!["ticket-search Acme"]);
    assert!(screen(&a).contains("90: Acme access"));
    a.press("enter");
    a.settle();
    assert_eq!(
        m.calls("assoc-plan"),
        vec!["assoc-plan 11 [Number(900)] false"]
    );
    let s = screen(&a);
    assert!(s.contains("Confirm link"), "{s}");
    assert!(s.contains("Initiating-ticket relationship only."), "{s}");
    a.press("y");
    a.settle();
    // `apply_change_action` rebuilds the plan and sends only if it equals the reviewed one.
    assert_eq!(
        m.calls("assoc-plan"),
        vec![
            "assoc-plan 11 [Number(900)] false",
            "assoc-plan 11 [Number(900)] false"
        ]
    );
    assert_eq!(m.calls("assoc-send").len(), 1);
    assert_eq!(last_toast(&a), "link complete inventory verified");
    assert_eq!(m.tickets.lock().unwrap().len(), 1);

    menu_pick(&mut a, "Unlink initiating ticket");
    let s = screen(&a);
    assert!(s.contains("unlink"), "{s}");
    assert!(s.contains("Acme access"), "{s}");
    a.press("enter");
    a.settle();
    assert_eq!(
        m.calls("assoc-plan")[2..],
        ["assoc-plan 11 [Number(900)] true"]
    );
    assert!(screen(&a).contains("Confirm unlink"));
    a.press("y");
    a.settle();
    assert_eq!(m.calls("assoc-plan").len(), 4);
    assert_eq!(m.calls("assoc-send").len(), 2);
    assert!(m.tickets.lock().unwrap().is_empty());
    assert_eq!(last_toast(&a), "unlink complete inventory verified");
}

#[test]
fn a_menu_unlink_without_associations_reports_no_targets() {
    let (mut a, m) = change_app();
    menu_pick(&mut a, "Unlink initiating ticket");
    assert!(matches!(
        a.modals.last(),
        Some((Modal::Failure(_), _)) | Some((Modal::Outcome(_), _))
    ));
    assert!(screen(&a).contains("No available targets."));
    assert!(m.calls("assoc-plan").is_empty());
}

/// `HelpScreen`: width 76, max-height 90%, overflow hidden — at 120x36
/// Textual clips the help at y=2..=33 and never scrolls; any key closes.
#[test]
fn help_is_clipped_on_a_36_row_terminal() {
    let (mut a, _) = change_app();
    a.press("question_mark");
    let s = screen(&a);
    let lines: Vec<&str> = s.lines().collect();
    let col = |l: &str, x: usize| l.chars().nth(x).unwrap_or(' ');
    assert_eq!(col(lines[2], 22), '┏', "{s}");
    assert_eq!(col(lines[33], 22), '┗', "{s}");
    assert_eq!(col(lines[2], 97), '┓', "{s}");
    assert!(s.contains("fsv help"), "{s}");
    assert!(s.contains("default view)"), "{s}");
    assert!(s.contains("empty input clears active filter"), "{s}");
    assert!(!s.contains("Press any key to close."), "{s}");
    a.press("j");
    assert!(a.modals.is_empty());
}
