//! Every key binding `app.py` documents (HelpScreen + `on_key`), driven
//! through `App` with inline workers, a manual clock and a recording backend.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use fsv_core::resource::Resource;
use fsv_tui::app::{App, Focus};
use fsv_tui::backend::{
    AssetCandidates, AssetCandidatesRequest, BResult, ConversationRequest, DetailPage, Failure,
    FormExit, FormRequest, FormSend, FormView, Item, ListRequest, PropertyField, ReviewPlan,
    SearchResult, TabData, TabInfo, View,
};
use fsv_tui::modals::Modal;
use fsv_tui::{Backend, BasicPages, view};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::widgets::{Paragraph, Widget};
use serde_json::{Map, Value, json};

type Res = &'static Resource;

#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<String>>,
    history: Mutex<Vec<String>>,
    rows: usize,
    views: Vec<View>,
    note_fails: bool,
    approval_unverified: bool,
    forms: bool,
    form_fails: bool,
    send_fails: bool,
    sent: Arc<Mutex<Vec<String>>>,
    planning_attachments: bool,
    no_planning: bool,
    unverified: bool,
    no_assets: bool,
    assets_fail: bool,
}

/// A stand-in `FormScreen`: `s` submits, `d` saves a draft, `escape`
/// cancels, `c` asks for a clipboard copy; it shows what it was opened with.
struct FakeForm {
    req: FormRequest,
    exit: Option<char>,
    clip: Option<String>,
    keys: Vec<String>,
    send_fails: bool,
    sent: Arc<Mutex<Vec<String>>>,
}

impl FormView for FakeForm {
    fn tick(&mut self, _now: Duration) {}
    fn handle_key(&mut self, key: &KeyEvent) {
        match key.code {
            KeyCode::Char(c @ ('s' | 'd')) => self.exit = Some(c),
            KeyCode::Esc => self.exit = Some('x'),
            KeyCode::Char('c') => self.clip = Some("copied".into()),
            KeyCode::Char(c) => self.keys.push(c.to_string()),
            _ => {}
        }
    }
    fn render(&mut self, area: Rect, buf: &mut Buffer) -> Option<Position> {
        let field = |v: &Option<Value>, k: &str| {
            v.as_ref()
                .and_then(|v| v.get(k))
                .map_or("-".to_string(), |v| v.to_string())
        };
        let text = format!(
            "FORM {} item={} record={} typed={}",
            self.req.mode,
            field(&self.req.item, "display_id"),
            field(&self.req.record, "subject"),
            self.keys.join("")
        );
        Paragraph::new(text).render(area, buf);
        None
    }
    fn done(&self) -> bool {
        self.exit.is_some()
    }
    fn take_clipboard(&mut self) -> Option<String> {
        self.clip.take()
    }
    fn finish(self: Box<Self>) -> FormExit {
        match self.exit {
            Some('s') => FormExit::Submit(Box::new(FakeSend {
                mode: self.req.mode,
                fails: self.send_fails,
                sent: self.sent,
            })),
            Some('d') => FormExit::Draft("/tmp/fsv-draft.json".into()),
            _ => FormExit::Cancelled,
        }
    }
}

struct FakeSend {
    mode: String,
    fails: bool,
    sent: Arc<Mutex<Vec<String>>>,
}

impl FormSend for FakeSend {
    fn send(self: Box<Self>) -> BResult<Value> {
        self.sent.lock().unwrap().push(self.mode.clone());
        if self.fails {
            return Err(Failure::value("send denied"));
        }
        Ok(match self.mode.as_str() {
            "new" => json!({"created": {"id": 99, "display_id": 12}}),
            "incident" => json!({"result": {"ticket": {"human_display_id": "INC-7", "id": 70}}}),
            "sr" => json!({"result": {"id": 5}}),
            _ => json!({"result": {"change": {"display_id": 3}}}),
        })
    }
}

impl Mock {
    fn new(rows: usize) -> Mock {
        Mock {
            rows,
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
            .filter(|s| s.starts_with(prefix))
            .cloned()
            .collect()
    }
}

fn plan(review: Value, public: bool) -> ReviewPlan {
    ReviewPlan {
        hash: format!("h-{review}"),
        review,
        public,
        target_id: json!(1),
        raw: Value::Null,
    }
}

impl Mock {
    fn asset_rows(&self) -> BResult<AssetCandidates> {
        if self.assets_fail {
            return Err(Failure::value("asset candidate response is unavailable"));
        }
        let rows = if self.no_assets {
            vec![]
        } else {
            vec![
                json!({"id": 9000007001_i64, "display_id": 701, "name": "Globex laptop", "ci_type_name": "Computer"}),
                json!({"id": 9000007002_i64, "display_id": 702, "name": null}),
            ]
        };
        Ok(AssetCandidates {
            change_id: 1,
            page: 1,
            scope: "single search page; not a complete inventory".into(),
            assets: rows
                .into_iter()
                .map(|r| r.as_object().unwrap().clone())
                .collect(),
            mutation: "none".into(),
            warning:
                "Selection only. Asset/CI add-remove variants remain unsupported by this picker."
                    .into(),
        })
    }
}

impl Backend for Mock {
    fn inventory_search(&self, query: &str, kind: &str) -> BResult<AssetCandidates> {
        self.log(format!("assets {kind} {query:?}"));
        self.asset_rows()
    }
    fn asset_candidates(&self, req: &AssetCandidatesRequest) -> BResult<AssetCandidates> {
        self.log(format!(
            "assets {} {:?} {}",
            req.change, req.query, req.page
        ));
        let mut rows = self.asset_rows()?;
        rows.page = req.page;
        Ok(rows)
    }
    fn forms_available(&self) -> bool {
        self.forms
    }
    fn open_form(&self, req: &FormRequest) -> BResult<Box<dyn FormView>> {
        self.log(format!("open_form {}", req.mode));
        if self.form_fails {
            return Err(Failure::value("no change form"));
        }
        Ok(Box::new(FakeForm {
            req: req.clone(),
            exit: None,
            clip: None,
            keys: Vec::new(),
            send_fails: self.send_fails,
            sent: self.sent.clone(),
        }))
    }
    fn catalog(&self) -> BResult<(Vec<Value>, Vec<Value>)> {
        if !self.forms {
            return Err(Failure::value("catalog denied"));
        }
        Ok((
            vec![json!({"id": 40, "display_id": 4, "name": "Laptop", "category_id": 1})],
            vec![json!({"id": 1, "name": "Hardware"})],
        ))
    }
    fn load_filter_history(&self) -> Vec<String> {
        self.history.lock().unwrap().clone()
    }
    fn save_filter_history(&self, history: &[String]) {
        *self.history.lock().unwrap() = history.to_vec();
    }
    fn copy_to_clipboard(&self, text: &str) -> Result<(), String> {
        self.log(format!("copy {text}"));
        Ok(())
    }
    fn open_url(&self, url: &str) -> bool {
        self.log(format!("open {url}"));
        true
    }
    fn item_url(&self, res: Res, id: &Value) -> String {
        format!("https://example.test/a/{}/{id}", res.api_path)
    }
    fn load_schema(&self, _res: Res) -> BResult<Value> {
        Ok(json!({"fields": [
            {"name": "status", "label": "Status", "field_type": "default_status", "choices": [
                {"id": 1, "value": "Open"}, {"id": 2, "value": "Pending"}, {"id": 3, "value": "Resolved"}]},
            {"name": "priority", "label": "Priority", "choices": [{"id": 1, "value": "Low"}]}
        ]}))
    }
    fn current_user(&self) -> BResult<Value> {
        Ok(json!({"id": 7, "name": "Me"}))
    }
    fn load_views(&self, _res: Res) -> BResult<Vec<View>> {
        Ok(self.views.clone())
    }
    fn build_query_hash(&self, _res: Res, _schema: &Value, filters: &[String]) -> BResult<String> {
        Ok(filters.join("&"))
    }
    fn list_page(&self, req: &ListRequest) -> BResult<(Vec<Value>, Value)> {
        self.log(format!(
            "list {} q={:?} view={:?}",
            req.res.name, req.query_hash, req.filter_name
        ));
        let rows = (1..=self.rows)
            .map(|i| json!({"id": i, "display_id": i, "status": 1, "priority": 1, "subject": format!("{} row {i}", req.res.name)}))
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
        let mut items = Vec::new();
        for (res, n) in [
            (&fsv_core::resource::TICKETS, 2),
            (&fsv_core::resource::CHANGES, 2),
        ] {
            if entity.is_none_or(|e| e == res.name) {
                for i in 1..=n {
                    items.push(Item::new(res, json!({"id": i, "display_id": i, "subject": format!("hit {} {i}", res.name)})));
                }
            }
        }
        Ok((
            items,
            vec![
                ("tickets".into(), 2),
                ("problems".into(), 0),
                ("changes".into(), 2),
            ],
        ))
    }
    fn load_detail(&self, res: Res, id: &Value, _schema: Option<&Value>) -> BResult<DetailPage> {
        self.log(format!("detail {} {id}", res.name));
        let desc: Vec<String> = (1..=100).map(|i| format!("line {i}")).collect();
        let tab = |n: &str, l: &str| TabInfo {
            name: n.into(),
            label: l.into(),
            count: None,
        };
        Ok(DetailPage {
            res,
            record: json!({"id": id, "display_id": id, "status": 1, "subject": "row", "description_text": desc.join("\n")}),
            tabs: vec![
                tab("details", "Details"),
                tab("notes", "Notes"),
                tab("tasks", "Tasks"),
            ],
            badge: String::new(),
            due: String::new(),
            tz: None,
            schema: None,
            extra: Value::Null,
        })
    }
    fn load_tab(&self, _res: Res, _record: &Value, tab: &str) -> BResult<TabData> {
        self.log(format!("tab {tab}"));
        let mut t = TabData {
            name: tab.into(),
            label: tab.into(),
            data: json!([]),
            ..Default::default()
        };
        t.meta.insert("complete".into(), json!(true));
        Ok(t)
    }
    fn update_item(&self, res: Res, id: &Value, body: &Map<String, Value>) -> BResult<Value> {
        self.log(format!(
            "update {} {id} {}",
            res.name,
            Value::Object(body.clone())
        ));
        Ok(json!({}))
    }
    fn add_note(&self, res: Res, id: &Value, body: &str, public: bool) -> BResult<Value> {
        if self.note_fails {
            return Err(Failure::value("server down"));
        }
        self.log(format!("note {} {id} {body} public={public}", res.name));
        Ok(json!({}))
    }
    fn ticket_note_plan(&self, ticket_id: &Value, body: &str, public: bool) -> BResult<ReviewPlan> {
        self.log(format!("ticket-note-plan {ticket_id} {body} {public}"));
        Ok(plan(json!({"body": body}), public))
    }
    fn send_ticket_note(
        &self,
        p: &ReviewPlan,
        public_confirmed: bool,
        _read_only: bool,
    ) -> BResult<Value> {
        if self.note_fails {
            return Err(Failure::value("server down"));
        }
        self.log(format!("ticket-note-send {} {public_confirmed}", p.hash));
        Ok(json!({"verified": true, "note_id": 11}))
    }
    fn download_choices(&self, _item: &Item) -> BResult<(Vec<Value>, Vec<String>)> {
        Ok((vec![json!({"id": 70, "name": "report.pdf"})], Vec::new()))
    }
    fn download(&self, attachment: &Value, destination: &str) -> BResult<Value> {
        self.log(format!("download {} {destination}", attachment["name"]));
        Ok(json!({"status": "downloaded"}))
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
            "approval-request {change_id} {name} {message} {recipients:?} {rule}"
        ));
        Ok(plan(json!({"name": name, "rule": rule}), false))
    }
    fn approval_targets(&self, kind: &str, change_id: &Value) -> BResult<Vec<Value>> {
        self.log(format!("approval-targets {kind} {change_id}"));
        Ok(vec![json!({"id": 31, "status": "requested"})])
    }
    fn approval_action_plan(
        &self,
        change_id: &Value,
        target_id: &Value,
        operation: &str,
        remark: Option<&str>,
    ) -> BResult<ReviewPlan> {
        self.log(format!(
            "approval-plan {change_id} {target_id} {operation} {remark:?}"
        ));
        Ok(plan(json!({"op": operation}), false))
    }
    fn send_approval(&self, p: &ReviewPlan, _read_only: bool) -> BResult<Value> {
        self.log(format!("send-approval {}", p.hash));
        Ok(json!({"verified": !self.approval_unverified, "id": 31}))
    }
    fn ticket_property_fields(&self, ticket_id: &Value) -> BResult<Vec<PropertyField>> {
        self.log(format!("property-fields {ticket_id}"));
        Ok(vec![
            PropertyField {
                name: "subject".into(),
                label: "Subject".into(),
                choices: None,
            },
            PropertyField {
                name: "priority".into(),
                label: "Priority".into(),
                choices: Some(vec![(json!(1), "Low".into()), (json!(4), "Urgent".into())]),
            },
        ])
    }
    fn ticket_property_plan(
        &self,
        ticket_id: &Value,
        name: &str,
        value: &Value,
    ) -> BResult<ReviewPlan> {
        self.log(format!("property-plan {ticket_id} {name} {value}"));
        Ok(plan(
            json!({"before": {name: "old"}, "after": {name: value}, "plan_hash": "p1"}),
            false,
        ))
    }
    fn send_ticket_property(&self, p: &ReviewPlan) -> BResult<Value> {
        self.log(format!("property-send {}", p.hash));
        Ok(json!({"status": "updated", "verified": !self.unverified}))
    }
    fn known_error_plan(&self, problem_id: &Value) -> BResult<ReviewPlan> {
        self.log(format!("known-error-plan {problem_id}"));
        Ok(plan(
            json!({"method": "PUT", "payload": {"known_error": true}}),
            false,
        ))
    }
    fn send_known_error(&self, p: &ReviewPlan) -> BResult<Value> {
        self.log(format!("known-error-send {}", p.hash));
        Ok(json!({"verified": !self.unverified}))
    }
    fn planning_definitions(&self, module: &str) -> BResult<Vec<Value>> {
        self.log(format!("planning-defs {module}"));
        if self.no_planning {
            return Ok(Vec::new());
        }
        Ok(vec![
            json!({"name": "acme_plan", "label": "Acme plan", "id": 1}),
        ])
    }
    fn planning_row(&self, module: &str, id: &Value, section: &str) -> BResult<Option<Value>> {
        self.log(format!("planning-row {module} {id} {section}"));
        let attachments = if self.planning_attachments {
            json!([{"id": 1}])
        } else {
            json!([])
        };
        Ok(Some(
            json!({"description": "old text", "attachments": attachments}),
        ))
    }
    fn planning_text_plan(
        &self,
        module: &str,
        id: &Value,
        section: &str,
        description: &str,
    ) -> BResult<ReviewPlan> {
        self.log(format!(
            "planning-plan {module} {id} {section} {description:?}"
        ));
        Ok(plan(
            json!({"section": section, "description": description, "plan_hash": "q1"}),
            false,
        ))
    }
    fn send_planning_text(&self, p: &ReviewPlan) -> BResult<Value> {
        self.log(format!("planning-send {}", p.hash));
        Ok(json!({"verified": !self.unverified}))
    }
    fn ticket_conversation_plan(
        &self,
        ticket_id: &Value,
        body: &str,
        req: &ConversationRequest,
    ) -> BResult<ReviewPlan> {
        self.log(format!("conversation-plan {ticket_id} {body} {req:?}"));
        let payload = if req.reply {
            json!({"body": body, "cc_emails": req.cc})
        } else {
            json!({"body": body, "private": !req.public, "notify_emails": req.notify})
        };
        let mut p = plan(json!({"payload": payload, "plan_hash": "c1"}), false);
        p.raw = payload;
        Ok(p)
    }
    fn send_ticket_conversation(&self, p: &ReviewPlan, public_confirmed: bool) -> BResult<Value> {
        self.log(format!("conversation-send {} {public_confirmed}", p.hash));
        Ok(json!({"conversation_id": 456, "verified": !self.unverified}))
    }
    fsv_tui::backend_stubs!(
        ticket_headers,
        view_settings,
        remote_user_values,
        lookup_choices,
        change_task_plan,
        send_change_task,
        search_change_tickets,
        task_for_edit,
        association_plan,
        send_association,
        update_task,
        delete_task
    );
}

fn start(mock: Mock, read_only: bool) -> (App, Arc<Mock>) {
    let mock = Arc::new(mock);
    let mut app = App::new_inline(mock.clone(), Arc::new(BasicPages), read_only);
    app.mount();
    app.settle();
    app.advance(Duration::from_millis(200));
    (app, mock)
}

fn app(rows: usize) -> (App, Arc<Mock>) {
    start(Mock::new(rows), false)
}

fn to_changes(app: &mut App) {
    app.press("g");
    app.press("c");
    app.advance(Duration::from_millis(250));
    app.advance(Duration::from_millis(200));
    assert_eq!(app.entity, "changes");
    assert!(
        app.selected
            .as_ref()
            .is_some_and(|s| s.res.name == "changes")
    );
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

fn top(app: &App) -> Option<&Modal> {
    app.modals.last().map(|(m, _)| m)
}

fn toasts(app: &App) -> Vec<String> {
    app.toasts.iter().map(|t| t.message.clone()).collect()
}

// ── App ─────────────────────────────────────────────────────

#[test]
fn ctrl_c_quits_even_with_a_modal_open() {
    let (mut a, _) = app(3);
    a.press("question_mark");
    a.press("ctrl+c");
    assert!(a.quit);
}

#[test]
fn q_quits() {
    let (mut a, _) = app(3);
    a.press("q");
    assert!(a.quit);
}

#[test]
fn question_mark_opens_help_and_any_key_closes() {
    let (mut a, _) = app(3);
    a.press("question_mark");
    let s = screen(&a);
    assert!(s.contains("fsv help") && s.contains("switch pane"), "{s}");
    a.press("x");
    assert!(a.modals.is_empty());
}

#[test]
fn r_in_list_pane_reloads() {
    let (mut a, m) = app(3);
    let before = m.calls("list tickets").len();
    a.press("r");
    a.settle();
    assert_eq!(m.calls("list tickets").len(), before + 1);
}

// ── Navigation ──────────────────────────────────────────────

#[test]
fn down_up_arrows_move_the_cursor() {
    let (mut a, _) = app(3);
    a.press("down");
    a.press("down");
    assert_eq!(a.cursor, 2);
    a.press("down");
    assert_eq!(a.cursor, 2);
    a.press("up");
    assert_eq!(a.cursor, 1);
}

#[test]
fn shift_j_k_and_curly_brackets_move_by_five() {
    let (mut a, _) = app(12);
    a.press("J");
    assert_eq!(a.cursor, 5);
    a.press("right_curly_bracket");
    assert_eq!(a.cursor, 10);
    a.press("K");
    assert_eq!(a.cursor, 5);
    a.press("left_curly_bracket");
    assert_eq!(a.cursor, 0);
}

#[test]
fn tab_and_ctrl_j_k_switch_panes() {
    let (mut a, _) = app(3);
    assert_eq!(a.focus, Focus::List);
    a.press("tab");
    assert_eq!(a.focus, Focus::Detail);
    a.press("tab");
    assert_eq!(a.focus, Focus::List);
    a.press("ctrl+j");
    assert_eq!(a.focus, Focus::Detail);
    a.press("ctrl+k");
    assert_eq!(a.focus, Focus::List);
}

#[test]
fn detail_pane_j_k_shift_j_g_and_gg_scroll() {
    let (mut a, _) = app(3);
    a.press("tab");
    screen(&a);
    a.press("k");
    assert_eq!(a.detail.scroll, 0);
    a.press("j");
    assert_eq!(a.detail.scroll, 1);
    a.press("J");
    assert_eq!(a.detail.scroll, 6);
    a.press("K");
    assert_eq!(a.detail.scroll, 1);
    a.press("G");
    assert!(a.detail.scroll > 6, "{}", a.detail.scroll);
    assert_eq!(a.cursor, 0);
    a.press("g");
    a.press("g");
    assert_eq!(a.detail.scroll, 0);
}

#[test]
fn g_t_p_c_jump_to_entities() {
    let (mut a, m) = app(3);
    for (k, e) in [("p", "problems"), ("c", "changes"), ("t", "tickets")] {
        a.press("g");
        a.press(k);
        a.advance(Duration::from_millis(250));
        assert_eq!(a.entity, e);
        assert!(screen(&a).contains(&format!("{e} row 1")));
    }
    assert_eq!(m.calls("list problems").len(), 1);
}

#[test]
fn g_then_other_key_is_swallowed() {
    let (mut a, _) = app(3);
    a.press("g");
    a.press("j");
    assert_eq!(a.cursor, 0);
    a.press("j");
    assert_eq!(a.cursor, 1);
}

#[test]
fn h_l_and_arrows_cycle_entities_in_list_pane() {
    let (mut a, _) = app(3);
    a.press("l");
    a.advance(Duration::from_millis(250));
    assert_eq!(a.entity, "problems");
    a.press("right");
    a.advance(Duration::from_millis(250));
    assert_eq!(a.entity, "changes");
    a.press("l");
    a.advance(Duration::from_millis(250));
    assert_eq!(a.entity, "tickets");
    a.press("h");
    a.advance(Duration::from_millis(250));
    assert_eq!(a.entity, "changes");
    a.press("left");
    a.advance(Duration::from_millis(250));
    assert_eq!(a.entity, "problems");
}

#[test]
fn h_l_step_detail_tabs_and_digits_jump() {
    let (mut a, m) = app(3);
    a.press("tab");
    a.press("l");
    assert_eq!(a.detail_tab_idx, 1);
    a.settle();
    assert_eq!(m.calls("tab notes").len(), 1);
    a.press("h");
    assert_eq!(a.detail_tab_idx, 0);
    a.press("h");
    assert_eq!(a.detail_tab_idx, 2);
    a.press("2");
    assert_eq!(a.detail_tab_idx, 1);
    a.press("9");
    assert_eq!(a.detail_tab_idx, 1);
    a.press("1");
    assert_eq!(a.detail_tab_idx, 0);
    assert_eq!(a.entity, "tickets");
}

#[test]
fn digits_switch_tabs_from_the_list_pane() {
    let (mut a, _) = app(3);
    a.press("3");
    assert_eq!(a.detail_tab_idx, 2);
    assert_eq!(a.focus, Focus::List);
}

#[test]
fn brackets_resize_panes_with_clamp() {
    let (mut a, _) = app(3);
    assert_eq!((a.list_fr, a.detail_fr), (11, 9));
    a.press("left_square_bracket");
    assert_eq!((a.list_fr, a.detail_fr), (10, 10));
    for _ in 0..20 {
        a.press("right_square_bracket");
    }
    assert_eq!((a.list_fr, a.detail_fr), (17, 3));
    screen(&a);
}

#[test]
fn paging_keys_move_by_page_and_to_ends() {
    let (mut a, _) = app(30);
    screen(&a);
    a.press("end");
    assert_eq!(a.cursor, 29);
    a.press("home");
    assert_eq!(a.cursor, 0);
    a.press("pagedown");
    assert!(a.cursor > 1);
    a.press("pageup");
    assert_eq!(a.cursor, 0);
}

// ── Views ───────────────────────────────────────────────────

fn saved(id: &str, name: &str, system: bool, default: bool) -> View {
    View {
        id: id.into(),
        name: name.into(),
        system,
        marked_default: default,
        raw: json!({"id": id}),
    }
}

#[test]
fn v_picks_a_saved_view_by_name() {
    let mock = Mock {
        rows: 3,
        views: vec![
            saved("1", "All tickets", true, true),
            saved("2", "My team queue", false, false),
        ],
        ..Default::default()
    };
    let (mut a, m) = start(mock, false);
    a.press("v");
    let s = screen(&a);
    assert!(s.contains("Tickets views") && s.contains("(custom)"), "{s}");
    a.type_text("team");
    a.press("enter");
    a.settle();
    assert_eq!(
        a.view.get("tickets").map(|v| v.name.as_str()),
        Some("My team queue")
    );
    assert!(
        m.calls("list tickets")
            .last()
            .is_some_and(|c| c.contains("Some(\"2\")"))
    );
}

#[test]
fn v_without_views_warns() {
    let (mut a, _) = app(3);
    a.press("v");
    assert!(a.modals.is_empty());
    assert!(toasts(&a).iter().any(|t| t.contains("No saved views")));
}

// ── Search ──────────────────────────────────────────────────

fn search(a: &mut App, term: &str) {
    a.press("f");
    assert_eq!(a.focus, Focus::Search);
    a.type_text(term);
    a.press("enter");
    a.settle();
    assert!(a.search_mode);
}

#[test]
fn f_searches_and_escape_exits_search_mode() {
    let (mut a, m) = app(3);
    search(&mut a, "vpn");
    assert_eq!(m.calls("search vpn None relevance 1").len(), 1);
    assert_eq!(a.rows.len(), 4);
    assert!(screen(&a).contains("hit changes 2"));
    a.press("escape");
    a.settle();
    assert!(!a.search_mode);
    assert_eq!(a.rows.len(), 3);
}

#[test]
fn escape_in_search_input_cancels_without_searching() {
    let (mut a, m) = app(3);
    a.press("f");
    a.type_text("x");
    a.press("escape");
    assert_eq!(a.focus, Focus::List);
    assert!(!a.search_input_visible && !a.search_mode);
    assert!(m.calls("search").is_empty());
}

#[test]
fn search_h_l_narrow_and_g_entity_keys() {
    let (mut a, m) = app(3);
    search(&mut a, "vpn");
    a.press("l");
    a.settle();
    assert_eq!(a.search_entity, Some("tickets"));
    assert_eq!(m.calls("search vpn Some(\"tickets\")").len(), 1);
    assert_eq!(a.rows.len(), 2);
    a.press("h");
    a.settle();
    assert_eq!(a.search_entity, None);
    assert_eq!(a.rows.len(), 4);
    assert_eq!(m.calls("search vpn None").len(), 1, "restored from cache");
    a.press("g");
    a.press("c");
    a.settle();
    assert_eq!(a.search_entity, Some("changes"));
    assert_eq!(a.entity, "tickets");
}

#[test]
fn search_s_cycles_sort_and_r_reruns() {
    let (mut a, m) = app(3);
    search(&mut a, "vpn");
    for want in ["created", "modified", "relevance"] {
        a.press("s");
        a.settle();
        assert_eq!(a.search_sort, want);
    }
    assert!(
        a.modals.is_empty(),
        "s in search mode is not a status write"
    );
    a.press("r");
    a.settle();
    assert_eq!(m.calls("search vpn None relevance 1").len(), 3);
}

#[test]
fn slash_and_v_do_nothing_in_search_mode() {
    let (mut a, _) = app(3);
    search(&mut a, "vpn");
    a.press("slash");
    assert_eq!(a.focus, Focus::List);
    assert!(!a.filter.visible);
    a.press("v");
    assert!(a.modals.is_empty());
}

// ── Filter ──────────────────────────────────────────────────

#[test]
fn filter_tab_completes_and_arrows_browse_history() {
    let (mut a, m) = app(3);
    a.press("slash");
    a.type_text("stat");
    a.press("tab");
    assert_eq!(a.filter.input.value(), "status=");
    a.press("tab");
    assert_eq!(a.filter.input.value(), "status=Open");
    a.press("enter");
    a.settle();
    assert_eq!(a.active_filters, vec!["status=Open".to_string()]);
    assert!(
        m.calls("list tickets")
            .last()
            .is_some_and(|c| c.contains("status=Open"))
    );

    a.press("slash");
    a.press("ctrl+u");
    a.type_text("priority=Low");
    a.press("enter");
    a.settle();
    assert_eq!(
        *m.history.lock().unwrap(),
        vec!["priority=Low", "status=Open"]
    );

    a.press("slash");
    a.press("ctrl+u");
    a.press("up");
    assert_eq!(a.filter.input.value(), "priority=Low");
    a.press("up");
    assert_eq!(a.filter.input.value(), "status=Open");
    a.press("down");
    assert_eq!(a.filter.input.value(), "priority=Low");
    a.press("escape");
    assert!(!a.filter.visible);
    assert_eq!(a.focus, Focus::List);
}

#[test]
fn filter_enter_on_empty_input_clears_the_filter() {
    let (mut a, _) = app(3);
    a.press("slash");
    a.type_text("status=Open");
    a.press("enter");
    a.settle();
    a.press("slash");
    a.press("ctrl+u");
    a.press("enter");
    a.settle();
    assert!(a.active_filters.is_empty());
}

#[test]
fn filter_keys_do_not_reach_the_main_handler() {
    let (mut a, _) = app(3);
    a.press("slash");
    a.type_text("qjs");
    assert!(!a.quit);
    assert_eq!(a.cursor, 0);
    assert!(a.modals.is_empty());
    assert_eq!(a.filter.input.value(), "qjs");
}

// ── Selection ───────────────────────────────────────────────

#[test]
fn y_copies_and_o_opens_the_selected_item() {
    let (mut a, m) = app(3);
    let did = a.selected.as_ref().unwrap().display_id();
    a.press("y");
    assert_eq!(m.calls("copy"), vec![format!("copy {did}")]);
    assert!(toasts(&a).contains(&format!("Copied {did}")));
    a.press("o");
    assert_eq!(
        m.calls("open"),
        vec!["open https://example.test/a/tickets/1".to_string()]
    );
}

#[test]
fn y_and_o_without_selection_warn() {
    let (mut a, m) = app(0);
    a.press("y");
    a.press("o");
    assert!(m.calls("copy").is_empty() && m.calls("open").is_empty());
    assert_eq!(
        toasts(&a)
            .iter()
            .filter(|t| *t == "No row selected")
            .count(),
        2
    );
}

// ── Write actions ───────────────────────────────────────────

#[test]
fn n_private_note_on_change_confirms_then_sends_once() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    a.press("n");
    assert!(screen(&a).contains("Change-note visibility"));
    assert!(m.calls("note").is_empty());
}

#[test]
fn n_ctrl_t_makes_a_public_note_and_escape_sends_nothing() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    a.press("n");
    assert!(screen(&a).contains("Change-note visibility"));
    a.press("escape");
    assert!(m.calls("note").is_empty());
    assert!(a.modals.is_empty());
}

#[test]
fn n_ticket_note_goes_through_the_reviewed_sender() {
    let (mut a, m) = app(3);
    a.press("n");
    a.type_text("hello");
    a.press("ctrl+s");
    a.settle();
    assert!(screen(&a).contains("Review exact ticket note"));
    a.press("y");
    a.settle();
    assert_eq!(m.calls("ticket-note-send").len(), 1);
    assert!(m.calls("note ").is_empty());
    assert!(toasts(&a).contains(&"Note 11 verified".to_string()));
}

#[test]
fn r_in_detail_pane_replies_to_a_ticket() {
    let (mut a, m) = app(3);
    a.press("tab");
    a.press("r");
    let s = screen(&a);
    assert!(s.contains("Add reply") && s.contains("PUBLIC reply"), "{s}");
    a.type_text("thanks");
    a.press("ctrl+s");
    a.settle();
    assert!(screen(&a).contains("Review exact ticket conversation"));
    assert_eq!(
        m.calls("conversation-plan"),
        [format!(
            "conversation-plan 1 thanks {:?}",
            ConversationRequest {
                reply: true,
                public: true,
                ..Default::default()
            }
        )]
    );
    a.press("y");
    assert!(screen(&a).contains("Confirm PUBLIC mail/visibility"));
    assert!(m.calls("conversation-send").is_empty());
    a.press("y");
    a.settle();
    let sends = m.calls("conversation-send");
    assert!(sends.len() == 1 && sends[0].ends_with(" true"), "{sends:?}");
    assert!(m.calls("reply").is_empty());
    assert!(screen(&a).contains("Conversation result — do not resend"));
    assert!(toasts(&a).contains(&"Exact conversation verified".to_string()));
}

#[test]
fn r_in_detail_pane_on_a_change_reloads_instead() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    a.press("tab");
    let before = m.calls("list changes").len();
    a.press("r");
    a.settle();
    assert!(a.modals.is_empty());
    assert_eq!(m.calls("list changes").len(), before + 1);
}

#[test]
fn exclamation_mark_reopens_the_last_failure() {
    let mock = Mock {
        rows: 3,
        note_fails: true,
        ..Default::default()
    };
    let (mut a, _) = start(mock, false);
    a.press("exclamation_mark");
    assert!(a.modals.is_empty());
    a.press("n");
    a.type_text("x");
    a.press("ctrl+s");
    a.settle();
    a.press("y");
    a.settle();
    assert!(matches!(top(&a), Some(Modal::Failure(_))));
    assert!(screen(&a).contains("server down"));
    a.press("escape");
    assert!(a.modals.is_empty());
    a.press("exclamation_mark");
    assert!(matches!(top(&a), Some(Modal::Failure(_))));
    assert!(
        a.write_failure
            .as_deref()
            .is_some_and(|w| w.contains("No automatic retry"))
    );
}

#[test]
fn read_only_blocks_every_write_key() {
    let (mut a, m) = start(Mock::new(3), true);
    a.press("tab");
    for k in ["s", "n", "r", "a", "N", "e"] {
        a.press(k);
        a.settle();
        assert!(a.modals.is_empty(), "{k}");
    }
    assert!(
        m.calls("update").is_empty() && m.calls("note").is_empty() && m.calls("reply").is_empty()
    );
    assert!(toasts(&a).iter().any(|t| t.contains("Read-only mode")));
}

#[test]
fn s_on_a_problem_is_not_available() {
    let (mut a, _) = app(3);
    a.press("g");
    a.press("p");
    a.advance(Duration::from_millis(250));
    a.advance(Duration::from_millis(200));
    a.press("s");
    assert!(a.modals.is_empty());
    assert!(
        toasts(&a)
            .iter()
            .any(|t| t == "Status change is not available for problems")
    );
}

// ── a: selected record actions ──────────────────────────────

fn menu_pick(a: &mut App, filter: &str) {
    a.press("a");
    assert!(screen(a).contains("Selected record actions"));
    a.type_text(filter);
    a.press("enter");
    a.settle();
}

#[test]
fn a_menu_lists_change_and_ticket_actions() {
    let (mut a, _) = app(3);
    a.press("a");
    if let Some(Modal::Picker(p)) = top(&a) {
        assert_eq!(p.labels(), vec!["  Download selected attachment"]);
    } else {
        panic!("no menu");
    }
    a.press("escape");
    to_changes(&mut a);
    a.press("a");
    if let Some(Modal::Picker(p)) = top(&a) {
        let labels = p.labels();
        assert_eq!(labels.len(), 11);
        assert_eq!(labels[5], "  Request approval");
        assert_eq!(labels[9], "  Download selected attachment");
        assert_eq!(labels[10], "  Edit native planning/analysis text");
    } else {
        panic!("no menu");
    }
}

#[test]
fn a_approval_groups_shows_read_only_details() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    menu_pick(&mut a, "list approval");
    assert_eq!(
        m.calls("approval-targets"),
        vec!["approval-targets approval-groups 1".to_string()]
    );
    assert!(screen(&a).contains("Approval groups"));
    a.press("enter");
    assert!(matches!(top(&a), Some(Modal::Outcome(_))));
    assert!(screen(&a).contains("Approval group details (read only)"));
    assert!(m.calls("send-approval").is_empty());
}

#[test]
fn a_approval_respond_approve_with_empty_remark_sends_once() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    menu_pick(&mut a, "respond");
    assert!(screen(&a).contains("Approval records"));
    a.press("enter");
    assert!(screen(&a).contains("Approval response"));
    a.press("enter");
    a.press("enter");
    assert_eq!(
        m.calls("approval-plan"),
        vec!["approval-plan 1 31 approve None".to_string()]
    );
    let s = screen(&a);
    assert!(
        s.contains("Review exact approval action") && s.contains("Selected inventory row"),
        "{s}"
    );
    a.press("y");
    a.settle();
    assert_eq!(m.calls("send-approval").len(), 1);
    assert!(screen(&a).contains("Approval result"));
    assert!(toasts(&a).iter().any(|t| t.contains("Approval submitted")));
}

#[test]
fn a_approval_reject_requires_remark_and_escape_sends_nothing() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    menu_pick(&mut a, "respond");
    a.press("enter");
    a.type_text("reject");
    a.press("enter");
    a.press("enter");
    assert!(
        matches!(top(&a), Some(Modal::Value(_))),
        "blank remark stays for reject"
    );
    a.type_text("no");
    a.press("enter");
    assert_eq!(
        m.calls("approval-plan"),
        vec!["approval-plan 1 31 reject Some(\"no\")".to_string()]
    );
    a.press("escape");
    a.settle();
    assert!(m.calls("send-approval").is_empty());
}

#[test]
fn a_approval_cancel_reviews_then_sends() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    menu_pick(&mut a, "cancel approval");
    a.press("enter");
    assert_eq!(
        m.calls("approval-plan"),
        vec!["approval-plan 1 31 cancel None".to_string()]
    );
    a.press("y");
    a.settle();
    assert_eq!(m.calls("send-approval").len(), 1);
}

#[test]
fn a_approval_request_collects_name_message_recipients_and_rule() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    menu_pick(&mut a, "request approval");
    assert!(screen(&a).contains("Approval group name"));
    a.type_text("CAB");
    a.press("enter");
    a.type_text("please");
    a.press("enter");
    a.type_text("a@x.test, 5");
    a.press("enter");
    assert!(screen(&a).contains("Approval rule"));
    a.type_text("major");
    a.press("enter");
    a.settle();
    assert_eq!(
        m.calls("approval-request"),
        vec!["approval-request 1 CAB please [\"a@x.test\", \"5\"] majority".to_string()]
    );
    a.press("y");
    a.settle();
    assert_eq!(m.calls("send-approval").len(), 1);
}

#[test]
fn a_unverified_approval_is_a_persistent_failure() {
    let mock = Mock {
        rows: 3,
        approval_unverified: true,
        ..Default::default()
    };
    let (mut a, m) = start(mock, false);
    to_changes(&mut a);
    menu_pick(&mut a, "cancel approval");
    a.press("enter");
    a.press("y");
    a.settle();
    assert_eq!(m.calls("send-approval").len(), 1);
    assert!(matches!(top(&a), Some(Modal::Failure(_))));
    assert!(
        a.write_failure
            .as_deref()
            .is_some_and(|w| w.contains("do not resend"))
    );
}

fn download_to(a: &mut App, dir: &str) {
    menu_pick(a, "download");
    a.press("enter");
    assert!(screen(a).contains("Download destination directory"));
    a.press("ctrl+u");
    a.type_text(dir);
    a.press("enter");
}

#[test]
fn a_download_reviews_then_downloads_once() {
    let dir = std::env::temp_dir().join(format!("fsv-tui-dl-new-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut a, m) = app(3);
    download_to(&mut a, &dir.to_string_lossy());
    assert!(screen(&a).contains("Confirm download"));
    a.press("y");
    a.settle();
    assert_eq!(m.calls("download").len(), 1);
    assert!(m.calls("download")[0].starts_with("download \"report.pdf\""));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_download_collision_offers_skip_or_rename() {
    let dir = std::env::temp_dir().join(format!("fsv-tui-dl-dup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("report.pdf"), b"x").unwrap();
    let d = dir.to_string_lossy().to_string();
    let (mut a, m) = app(3);

    download_to(&mut a, &d);
    match top(&a) {
        Some(Modal::Picker(p)) => assert!(
            p.title.starts_with("File exists: ") && p.title.ends_with("report.pdf — no overwrite"),
            "{}",
            p.title
        ),
        _ => panic!("no collision picker"),
    }
    a.type_text("skip");
    a.press("enter");
    assert!(a.modals.is_empty());

    download_to(&mut a, &d);
    a.type_text("another");
    a.press("enter");
    assert!(screen(&a).contains("Save as filename"));
    a.press("ctrl+u");
    a.type_text("copy.pdf");
    a.press("enter");
    a.press("y");
    a.settle();
    assert_eq!(m.calls("download").len(), 1);
    assert!(m.calls("download")[0].starts_with("download \"copy.pdf\""));

    download_to(&mut a, &d);
    a.type_text("another");
    a.press("enter");
    a.press("ctrl+u");
    a.type_text("../x");
    a.press("enter");
    assert!(matches!(top(&a), Some(Modal::Failure(_))));
    assert_eq!(m.calls("download").len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Forms ───────────────────────────────────────────────────

#[test]
fn shift_n_opens_the_create_chooser() {
    let (mut a, _) = app(3);
    a.press("N");
    if let Some(Modal::Picker(p)) = top(&a) {
        assert_eq!(
            p.labels(),
            vec!["  Change", "  Incident", "  Service Request"]
        );
    } else {
        panic!("no chooser");
    }
    a.press("escape");
    assert!(a.modals.is_empty());
    a.press("N");
    a.press("enter");
    assert!(toasts(&a).iter().any(|t| t == "form UI not available"));
    a.press("N");
    a.type_text("service");
    a.press("enter");
    a.settle();
    assert!(screen(&a).contains("Catalog unavailable"));
}

#[test]
fn e_edits_only_a_selected_change() {
    let (mut a, _) = app(0);
    a.press("e");
    assert!(toasts(&a).iter().any(|t| t == "Select a change to edit"));
    let (mut a, _) = app(3);
    to_changes(&mut a);
    a.press("e");
    assert!(toasts(&a).iter().any(|t| t == "form UI not available"));
}

fn forms() -> (App, Arc<Mock>) {
    start(
        Mock {
            forms: true,
            ..Mock::new(3)
        },
        false,
    )
}

fn sent(m: &Mock) -> Vec<String> {
    m.sent.lock().unwrap().clone()
}

/// `_create_choice("change")` -> `_build_form("new")`; submit sends once,
/// toasts `form_result` ("Change created CHN-12") and reloads the list.
#[test]
fn n_change_opens_the_form_and_a_submit_sends_and_reloads() {
    let (mut a, m) = forms();
    let lists = m.calls("list").len();
    a.press("N");
    a.press("enter");
    a.settle();
    assert_eq!(m.calls("open_form"), ["open_form new"]);
    assert!(a.form_open());
    assert!(screen(&a).contains("FORM new item=- record=-"));
    a.press("q");
    a.press("j");
    assert!(!a.quit);
    assert!(screen(&a).contains("typed=qj"));
    a.press("s");
    assert!(!a.form_open());
    a.settle();
    assert_eq!(sent(&m), ["new"]);
    assert!(toasts(&a).contains(&"Change created CHN-12".to_string()));
    assert_eq!(m.calls("list").len(), lists + 1);
    assert!(!screen(&a).contains("FORM"));
}

#[test]
fn n_incident_draft_toasts_the_path_and_sends_nothing() {
    let (mut a, m) = forms();
    a.press("N");
    a.type_text("incident");
    a.press("enter");
    a.settle();
    assert_eq!(m.calls("open_form"), ["open_form incident"]);
    a.press("d");
    a.settle();
    assert!(toasts(&a).contains(&"Draft saved: /tmp/fsv-draft.json".to_string()));
    assert!(sent(&m).is_empty());
    assert!(!a.form_open());
}

/// `_launch_create("sr", row)`: the picked catalog row opens `sr_session`;
/// a cancelled form sends nothing and says nothing.
#[test]
fn n_service_request_opens_the_picked_catalog_item() {
    let (mut a, m) = forms();
    a.press("N");
    a.type_text("service");
    a.press("enter");
    a.settle();
    a.press("enter");
    a.press("enter");
    a.settle();
    assert_eq!(m.calls("open_form"), ["open_form sr"]);
    assert!(screen(&a).contains("FORM sr item=4"));
    let before = toasts(&a);
    a.press("escape");
    a.settle();
    assert!(!a.form_open());
    assert!(sent(&m).is_empty());
    assert_eq!(toasts(&a), before);

    let (mut c, _) = forms();
    c.press("N");
    c.type_text("service");
    c.press("enter");
    c.settle();
    c.press("enter");
    c.press("enter");
    c.settle();
    c.press("s");
    c.settle();
    assert!(toasts(&c).contains(&"Service Request created ID 5".to_string()));
}

/// `e`: "opening the form…", the form edits the loaded detail record, and a
/// send toasts "Change updated" and reloads the change.
#[test]
fn e_edits_the_loaded_change_and_reloads_it() {
    let (mut a, m) = forms();
    to_changes(&mut a);
    a.press("e");
    assert!(toasts(&a).contains(&"opening the form…".to_string()));
    a.settle();
    assert_eq!(m.calls("open_form"), ["open_form edit"]);
    assert!(screen(&a).contains(r#"FORM edit item=1 record="row""#));
    let details = m.calls("detail changes").len();
    a.press("s");
    a.settle();
    assert_eq!(sent(&m), ["edit"]);
    assert!(toasts(&a).contains(&"Change updated".to_string()));
    assert_eq!(m.calls("detail changes").len(), details + 1);
}

#[test]
fn form_failures_open_the_failure_screen() {
    let (mut a, _) = start(
        Mock {
            forms: true,
            form_fails: true,
            ..Mock::new(3)
        },
        false,
    );
    a.press("N");
    a.press("enter");
    a.settle();
    assert!(!a.form_open());
    assert!(screen(&a).contains("Form unavailable: no change form"));

    let (mut b, m) = start(
        Mock {
            forms: true,
            send_fails: true,
            ..Mock::new(3)
        },
        false,
    );
    b.press("N");
    b.press("enter");
    b.settle();
    b.press("s");
    b.settle();
    assert_eq!(sent(&m), ["new"]);
    assert_eq!(
        b.write_failure.as_deref(),
        Some(
            "send denied\nNo automatic retry. Check the exact record and journal before another send; outcome may be unknown."
        )
    );
}

#[test]
fn form_copy_goes_to_the_clipboard_and_ctrl_q_quits() {
    let (mut a, m) = forms();
    a.press("N");
    a.press("enter");
    a.settle();
    a.press("c");
    a.tick();
    assert_eq!(m.calls("copy"), ["copy copied"]);
    a.press("ctrl+q");
    assert!(a.quit);
}

// ── Reviewed writes (properties, known error, planning, conversations) ──

fn to_problems(app: &mut App) {
    app.press("g");
    app.press("p");
    app.advance(Duration::from_millis(250));
    app.advance(Duration::from_millis(200));
    assert_eq!(app.entity, "problems");
}

fn picker(app: &App) -> (String, Vec<String>) {
    match top(app) {
        Some(Modal::Picker(p)) => (p.title.clone(), p.labels()),
        m => panic!("not a picker: {m:?}"),
    }
}

fn confirm(app: &App) -> (String, Vec<String>) {
    match top(app) {
        Some(Modal::Confirm(c)) => (c.title.clone(), c.lines.clone()),
        m => panic!("not a confirm: {m:?}"),
    }
}

fn outcome(app: &App) -> (String, String) {
    match top(app) {
        Some(Modal::Outcome(f)) => (f.title.clone(), f.message.clone()),
        m => panic!("not an outcome: {m:?}"),
    }
}

fn value_title(app: &App) -> String {
    match top(app) {
        Some(Modal::Value(v)) => v.title.clone(),
        m => panic!("not a value screen: {m:?}"),
    }
}

fn pick(app: &mut App, text: &str) {
    app.type_text(text);
    app.press("enter");
    app.settle();
}

/// `test_tui_property_field_picker_review_and_outcome`, both `cancel` cases.
#[test]
fn e_on_a_ticket_reviews_one_property_and_sends_once() {
    for cancel in [true, false] {
        let (mut a, m) = app(3);
        a.press("e");
        a.settle();
        assert_eq!(m.calls("property-fields"), ["property-fields 1"]);
        assert_eq!(
            picker(&a),
            (
                "Edit ticket/SR property — one field per review".into(),
                vec!["  Subject".into(), "  Priority".into()]
            )
        );
        pick(&mut a, "Subject");
        assert_eq!(
            value_title(&a),
            "Subject — exact value/name/email; comma-separated for multiple values"
        );
        pick(&mut a, "Globex maintenance");
        assert_eq!(
            m.calls("property-plan"),
            [r#"property-plan 1 subject "Globex maintenance""#]
        );
        let (title, lines) = confirm(&a);
        assert_eq!(title, "Review exact ticket property");
        assert!(
            lines[0].contains("Globex maintenance")
                && lines[0].contains("before")
                && lines[0].contains("plan_hash"),
            "{lines:?}"
        );
        assert!(m.calls("property-send").is_empty());
        a.press(if cancel { "escape" } else { "y" });
        a.settle();
        assert_eq!(m.calls("property-send").len(), usize::from(!cancel));
        if cancel {
            assert!(a.modals.is_empty());
        } else {
            let (t, msg) = outcome(&a);
            assert_eq!(t, "Property result — do not resend");
            assert!(msg.contains("\"verified\": true"), "{msg}");
            assert!(toasts(&a).contains(&"Ticket property verified".to_string()));
        }
    }
}

#[test]
fn e_on_a_ticket_dropdown_picks_a_visible_choice() {
    let (mut a, m) = app(3);
    a.press("e");
    a.settle();
    pick(&mut a, "Priority");
    assert_eq!(
        picker(&a),
        ("Priority".into(), vec!["  Low".into(), "  Urgent".into()])
    );
    pick(&mut a, "Urgent");
    assert_eq!(m.calls("property-plan"), ["property-plan 1 priority 4"]);
    assert_eq!(confirm(&a).0, "Review exact ticket property");
}

#[test]
fn unverified_property_shows_the_outcome_without_reloading() {
    let (mut a, m) = start(
        Mock {
            unverified: true,
            ..Mock::new(3)
        },
        false,
    );
    a.press("e");
    a.settle();
    pick(&mut a, "Subject");
    pick(&mut a, "x");
    let details = m.calls("detail").len();
    a.press("y");
    a.settle();
    assert_eq!(outcome(&a).0, "Property result — do not resend");
    assert!(!toasts(&a).contains(&"Ticket property verified".to_string()));
    assert_eq!(m.calls("detail").len(), details);
}

/// `test_tui_known_error_review_and_cancel`, both `cancel` cases.
#[test]
fn a_known_error_on_a_problem_reviews_then_sends_once() {
    for cancel in [true, false] {
        let (mut a, m) = app(3);
        to_problems(&mut a);
        a.press("a");
        assert_eq!(
            picker(&a).1,
            [
                "  Download selected attachment",
                "  Edit native planning/analysis text",
                "  Mark as known error"
            ]
        );
        pick(&mut a, "known error");
        assert_eq!(m.calls("known-error-plan"), ["known-error-plan 1"]);
        let (title, lines) = confirm(&a);
        assert_eq!(title, "Review known-error mark");
        assert!(lines[0].contains("\"known_error\": true"), "{lines:?}");
        assert_eq!(
            lines[1],
            "This does not resolve/close the problem or publish a knowledge article. Reversal is not implemented."
        );
        a.press(if cancel { "escape" } else { "y" });
        a.settle();
        assert_eq!(m.calls("known-error-send").len(), usize::from(!cancel));
        if !cancel {
            assert_eq!(outcome(&a).0, "Known-error result — do not resend");
            assert!(toasts(&a).contains(&"Known-error state verified".to_string()));
        }
    }
}

/// `test_tui_planning_picker_editor_review_and_outcome`, both `cancel` cases;
/// the stored text preloads and is reviewed and sent like
/// `test_tui_native_existing_content_preloads_and_reviews_edit` (the PUT
/// itself is checked in fsv-app `planning_text_edits_stored_text_with_a_put`).
#[test]
fn a_planning_text_picks_a_section_edits_reviews_and_sends_once() {
    for cancel in [true, false] {
        let (mut a, m) = app(3);
        to_changes(&mut a);
        a.press("a");
        pick(&mut a, "native planning");
        assert_eq!(m.calls("planning-defs"), ["planning-defs changes"]);
        assert_eq!(
            picker(&a),
            (
                "Native planning/analysis section".into(),
                vec!["  Acme plan".into()]
            )
        );
        a.press("enter");
        a.settle();
        assert_eq!(
            m.calls("planning-row"),
            ["planning-row changes 1 acme_plan"]
        );
        assert!(matches!(top(&a), Some(Modal::Note(n)) if n.planning));
        let s = screen(&a);
        assert!(
            s.contains("Edit planning text: acme_plan")
                && s.contains("Text/HTML saved verbatim; no attachment changes")
                && s.contains("old text")
                && s.contains("Review")
                && s.contains("Ctrl+S review · Esc cancel")
                && !s.contains("Preview:"),
            "{s}"
        );
        a.press("ctrl+t");
        a.press("enter");
        a.type_text("second line");
        a.press("ctrl+s");
        a.settle();
        assert_eq!(
            m.calls("planning-plan"),
            [r#"planning-plan changes 1 acme_plan "old text\nsecond line""#]
        );
        let (title, lines) = confirm(&a);
        assert_eq!(title, "Review exact planning text");
        assert!(lines[0].contains("acme_plan") && lines[0].contains("plan_hash"));
        assert!(m.calls("planning-send").is_empty());
        a.press(if cancel { "escape" } else { "y" });
        a.settle();
        assert_eq!(m.calls("planning-send").len(), usize::from(!cancel));
        if !cancel {
            assert_eq!(outcome(&a).0, "Planning result — do not resend");
            assert!(toasts(&a).contains(&"Planning text verified".to_string()));
        }
    }
}

#[test]
fn a_planning_text_refuses_attachments_and_empty_metadata() {
    let (mut a, m) = start(
        Mock {
            planning_attachments: true,
            ..Mock::new(3)
        },
        false,
    );
    to_problems(&mut a);
    a.press("a");
    pick(&mut a, "native planning");
    assert_eq!(m.calls("planning-defs"), ["planning-defs problems"]);
    a.press("enter");
    a.settle();
    assert!(matches!(top(&a), Some(Modal::Failure(_))));
    assert!(screen(&a).contains("planning attachments are present or unavailable"));

    let (mut b, _) = start(
        Mock {
            no_planning: true,
            ..Mock::new(3)
        },
        false,
    );
    to_changes(&mut b);
    b.press("a");
    pick(&mut b, "native planning");
    assert!(matches!(top(&b), Some(Modal::Failure(_))));
    assert!(screen(&b).contains("No native planning sections in current metadata"));
}

/// `test_ticket_options_recipients_review_and_public_consent`, all four
/// `(reply, cancel)` cases.
#[test]
fn ctrl_o_ticket_options_add_recipients_review_and_ask_public_consent() {
    for (reply, cancel) in [(true, true), (true, false), (false, true), (false, false)] {
        let (mut a, m) = app(3);
        if reply {
            a.press("tab");
        }
        a.press(if reply { "r" } else { "n" });
        assert!(matches!(top(&a), Some(Modal::Note(n)) if n.ticket_options));
        a.type_text("Acme update");
        a.press("ctrl+o");
        let (title, labels) = picker(&a);
        if reply {
            assert_eq!(title, "Reply recipient/status options");
            assert_eq!(
                labels,
                [
                    "  Review exact reply",
                    "  Status: unchanged (toggle Pending)",
                    "  CC emails",
                    "  BCC emails (no CC/status combination)",
                    "  Configured sender (required for BCC)"
                ]
            );
        } else {
            assert_eq!(title, "Note notification/status options");
            assert_eq!(
                labels,
                [
                    "  Review exact note",
                    "  Status: unchanged (toggle Pending)",
                    "  Notify-to emails"
                ]
            );
        }
        pick(
            &mut a,
            if reply {
                "CC emails"
            } else {
                "Notify-to emails"
            },
        );
        assert_eq!(
            value_title(&a),
            "Comma-separated plain emails (empty clears)"
        );
        pick(&mut a, "person@example.com");
        assert!(matches!(top(&a), Some(Modal::Picker(_))));
        a.press("enter");
        a.settle();
        let (title, lines) = confirm(&a);
        assert_eq!(title, "Review exact ticket conversation");
        let review = lines.join("\n");
        assert!(review.contains("person@example.com") && review.contains("plan_hash"));
        assert!(review.contains(if reply {
            "\"cc_emails\""
        } else {
            "\"notify_emails\""
        }));
        assert!(m.calls("conversation-send").is_empty());
        a.press(if cancel { "escape" } else { "y" });
        if !cancel && reply {
            let (title, lines) = confirm(&a);
            assert_eq!(title, "Confirm PUBLIC mail/visibility");
            assert_eq!(
                lines,
                [
                    "Review every recipient. This public conversation may send email and trigger workflows."
                ]
            );
            assert!(m.calls("conversation-send").is_empty());
            a.press("y");
        }
        a.settle();
        let sends = m.calls("conversation-send");
        assert_eq!(sends.len(), usize::from(!cancel));
        if !cancel {
            assert!(sends[0].ends_with(if reply { " true" } else { " false" }));
            let (t, msg) = outcome(&a);
            assert_eq!(t, "Conversation result — do not resend");
            assert!(msg.contains("456"));
            a.press("escape");
        }
    }
}

#[test]
fn ticket_options_toggle_pending_set_sender_and_clear_with_empty() {
    let (mut a, m) = app(3);
    a.press("tab");
    a.press("r");
    a.type_text("Acme");
    a.press("ctrl+o");
    pick(&mut a, "Status");
    assert!(
        picker(&a)
            .1
            .contains(&"  Status: Pending (toggle Pending)".to_string())
    );
    pick(&mut a, "Configured sender");
    assert_eq!(value_title(&a), "Configured support email");
    pick(&mut a, "help@acme.test");
    pick(&mut a, "CC emails");
    pick(&mut a, "a@acme.test");
    pick(&mut a, "CC emails");
    for _ in 0.."a@acme.test".len() {
        a.press("backspace");
    }
    a.press("enter");
    pick(&mut a, "Review exact reply");
    assert_eq!(
        m.calls("conversation-plan"),
        [format!(
            "conversation-plan 1 Acme {:?}",
            ConversationRequest {
                reply: true,
                public: true,
                sender: Some("help@acme.test".into()),
                set_status: Some("Pending".into()),
                ..Default::default()
            }
        )]
    );
}

#[test]
fn ctrl_o_needs_text_and_a_ticket() {
    let (mut a, m) = app(3);
    a.press("n");
    a.press("ctrl+o");
    assert!(matches!(top(&a), Some(Modal::Note(_))));
    assert!(toasts(&a).contains(&"Note is empty".to_string()));
    let (mut b, _) = app(3);
    to_changes(&mut b);
    b.press("n");
    assert!(screen(&b).contains("Change-note visibility"));
    assert!(m.calls("conversation-plan").is_empty());
}

// ── Read-only asset candidate picker (`i`) ──────────────────

/// `test_asset_picker.py::test_pilot_readonly_search_selection_and_cancel`,
/// all four modes, in read-only mode.
#[test]
fn i_on_a_change_searches_page_1_and_picks_without_associating() {
    for mode in ["select", "cancel-search", "cancel-picker", "empty"] {
        let mock = Mock {
            no_assets: mode == "empty",
            ..Mock::new(3)
        };
        let (mut a, m) = start(mock, true);
        to_changes(&mut a);
        a.press("i");
        assert_eq!(
            value_title(&a),
            "Read-only asset search — selection does not associate",
            "{mode}"
        );
        if mode == "cancel-search" {
            a.press("escape");
            a.settle();
            assert!(a.modals.is_empty());
            assert!(m.calls("assets").is_empty());
            continue;
        }
        pick(&mut a, "Globex");
        assert_eq!(m.calls("assets"), [r#"assets devices "Globex""#], "{mode}");
        if mode == "empty" {
            assert_eq!(
                outcome(&a),
                (
                    "Asset candidates — read-only".into(),
                    "No candidates on search page 1. No association was attempted.".into()
                )
            );
            continue;
        }
        assert_eq!(
            picker(&a),
            (
                "Asset candidates — page 1 only; read-only".into(),
                vec![
                    "  [701] Globex laptop — Computer".into(),
                    "  [702] - — -".into()
                ]
            )
        );
        let s = screen(&a);
        assert!(
            s.contains("Asset candidates") && s.contains("page 1") && s.contains("read-only"),
            "{s}"
        );
        assert!(!s.to_lowercase().contains("saved view"), "{s}");
        a.press(if mode == "cancel-picker" {
            "escape"
        } else {
            "enter"
        });
        a.settle();
        if mode == "cancel-picker" {
            assert!(a.modals.is_empty());
            continue;
        }
        let (title, message) = outcome(&a);
        assert_eq!(title, "Selected asset — no association");
        assert_eq!(
            message,
            r#"{
  "selected": {
    "id": 9000007001,
    "display_id": 701,
    "name": "Globex laptop",
    "ci_type_name": "Computer"
  },
  "scope": "single search page; not a complete inventory",
  "mutation": "none"
}"#
        );
        assert_eq!(m.calls("assets").len(), 1);
    }
}

#[test]
fn i_needs_a_record_and_a_search_term() {
    let (mut a, m) = start(Mock::new(3), true);
    a.selected = None;
    a.press("i");
    assert!(a.modals.is_empty());
    assert!(toasts(&a).contains(&"Select a ticket, SR, problem or change".to_string()));
    to_changes(&mut a);
    a.press("i");
    a.press("enter");
    a.settle();
    assert_eq!(
        value_title(&a),
        "Read-only asset search — selection does not associate"
    );
    assert!(m.calls("assets").is_empty());
}

#[test]
fn i_search_failure_opens_the_failure_screen() {
    let mock = Mock {
        assets_fail: true,
        ..Mock::new(3)
    };
    let (mut a, m) = start(mock, false);
    to_changes(&mut a);
    a.press("i");
    pick(&mut a, "Browse");
    pick(&mut a, "Globex");
    assert_eq!(m.calls("assets").len(), 1);
    match top(&a) {
        Some(Modal::Failure(f)) => assert_eq!(
            f.message,
            "Asset search unavailable: asset candidate response is unavailable"
        ),
        m => panic!("not a failure: {m:?}"),
    }
}

#[test]
fn help_lists_the_asset_picker_key() {
    assert!(
        fsv_tui::modals::HELP_TEXT.contains(
            "  [cyan]i[/]            browse or review device/CI and service associations\n"
        )
    );
}

/// `asset_picker.select`: an exact, unique positive display ID.
#[test]
fn asset_select_needs_an_exact_unique_display_id() {
    let row = |id: Value| {
        json!({"id": 9000007001_i64, "display_id": id})
            .as_object()
            .unwrap()
            .clone()
    };
    let mut result = AssetCandidates {
        change_id: 123,
        page: 1,
        scope: String::new(),
        assets: vec![row(json!(701)), row(json!("702"))],
        mutation: "none".into(),
        warning: String::new(),
    };
    assert_eq!(result.select(&json!(701)).unwrap()["display_id"], 701);
    assert_eq!(result.select(&json!("702")).unwrap()["display_id"], "702");
    let not_unique = "exact asset display ID is not unique on this search page";
    assert_eq!(
        result
            .select(&json!(9000007001_i64))
            .unwrap_err()
            .to_string(),
        not_unique
    );
    for bad in [json!(true), json!(0), json!(-1), json!("x"), json!(701.0)] {
        assert_eq!(
            result.select(&bad).unwrap_err().to_string(),
            "expected a positive numeric identity",
            "{bad}"
        );
    }
    result.assets.push(row(json!(701)));
    assert_eq!(
        result.select(&json!(701)).unwrap_err().to_string(),
        not_unique
    );
}
