//! Every key binding `app.py` documents (HelpScreen + `on_key`), driven
//! through `App` with inline workers, a manual clock and a recording backend.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fsv_core::resource::Resource;
use fsv_tui::app::{App, Focus};
use fsv_tui::backend::{
    BResult, DetailPage, Failure, Item, ListRequest, ReviewPlan, SearchResult, TabData, TabInfo,
    View,
};
use fsv_tui::modals::Modal;
use fsv_tui::{Backend, BasicPages, view};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
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

impl Backend for Mock {
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
    fn add_reply(&self, res: Res, id: &Value, body: &str) -> BResult<Value> {
        self.log(format!("reply {} {id} {body}", res.name));
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
    assert!(matches!(top(&a), Some(Modal::Note(_))));
    a.type_text("qoy");
    a.press("ctrl+s");
    assert!(screen(&a).contains("Confirm private note"));
    assert!(m.calls("note").is_empty());
    a.press("y");
    a.settle();
    assert_eq!(
        m.calls("note"),
        vec!["note changes 1 qoy public=false".to_string()]
    );
}

#[test]
fn n_ctrl_t_makes_a_public_note_and_escape_sends_nothing() {
    let (mut a, m) = app(3);
    to_changes(&mut a);
    a.press("n");
    a.type_text("hi");
    a.press("ctrl+t");
    a.press("ctrl+s");
    assert!(screen(&a).contains("Confirm public note"));
    a.press("escape");
    a.settle();
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
    assert!(screen(&a).contains("Confirm reply"));
    a.press("y");
    a.settle();
    assert_eq!(m.calls("reply"), vec!["reply tickets 1 thanks".to_string()]);
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
    to_changes(&mut a);
    a.press("n");
    a.type_text("x");
    a.press("ctrl+s");
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
        assert_eq!(labels.len(), 10);
        assert_eq!(labels[5], "  Request approval");
        assert_eq!(labels[9], "  Download selected attachment");
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
    let (mut a, _) = app(3);
    a.press("e");
    assert!(toasts(&a).iter().any(|t| t == "Select a change to edit"));
    to_changes(&mut a);
    a.press("e");
    assert!(toasts(&a).iter().any(|t| t == "form UI not available"));
}
