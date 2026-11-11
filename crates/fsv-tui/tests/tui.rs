use std::sync::{Arc, Mutex};
use std::time::Duration;

use fsv_core::resource::Resource;
use fsv_tui::app::{App, Focus};
use fsv_tui::backend::{BResult, DetailPage, Failure, ListRequest, View};
use fsv_tui::modals::Modal;
use fsv_tui::{Backend, BasicPages, view};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Map, Value, json};

#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<String>>,
    history: Mutex<Vec<String>>,
}

impl Mock {
    fn log(&self, s: String) {
        if let Ok(mut c) = self.calls.lock() {
            c.push(s);
        }
    }
    fn calls(&self, prefix: &str) -> Vec<String> {
        self.calls
            .lock()
            .map(|c| {
                c.iter()
                    .filter(|s| s.starts_with(prefix))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn status_schema() -> Value {
    json!({"fields": [{"name": "status", "choices": [
        {"id": 1, "value": "Open"}, {"id": 2, "value": "Pending"}, {"id": 3, "value": "Resolved"}
    ]}]})
}

impl Backend for Mock {
    fn read_only_setting(&self) -> Value {
        Value::Bool(false)
    }
    fn load_filter_history(&self) -> Vec<String> {
        self.history.lock().map(|h| h.clone()).unwrap_or_default()
    }
    fn save_filter_history(&self, history: &[String]) {
        if let Ok(mut h) = self.history.lock() {
            *h = history.to_vec();
        }
    }
    fn load_schema(&self, _res: &'static Resource) -> BResult<Value> {
        Ok(status_schema())
    }
    fn current_user(&self) -> BResult<Value> {
        Ok(json!({"id": 7, "name": "Me"}))
    }
    fn load_views(&self, _res: &'static Resource) -> BResult<Vec<View>> {
        Ok(Vec::new())
    }
    fn ticket_headers(&self) -> BResult<Vec<Value>> {
        Ok(Vec::new())
    }
    fn build_query_hash(
        &self,
        _res: &'static Resource,
        _schema: &Value,
        filters: &[String],
    ) -> BResult<String> {
        Ok(filters.join("&"))
    }
    fn list_page(&self, req: &ListRequest) -> BResult<(Vec<Value>, Value)> {
        self.log(format!("list {} q={:?}", req.res.name, req.query_hash));
        let rows = (1..=3).map(|i| json!({"id": i, "display_id": i, "status": 1, "priority": 2, "subject": format!("{} row {i}", req.res.name), "requester": {"name": "Alice"}})).collect();
        Ok((rows, json!({"meta": {"total_count": 3}})))
    }
    fn load_detail(
        &self,
        res: &'static Resource,
        id: &Value,
        _schema: Option<&Value>,
    ) -> BResult<DetailPage> {
        self.log(format!("detail {} {id}", res.name));
        Ok(DetailPage {
            res,
            record: json!({"id": id, "display_id": id, "status": 1, "subject": "row"}),
            tabs: Vec::new(),
            badge: String::new(),
            due: String::new(),
            tz: None,
            schema: None,
            extra: Value::Null,
        })
    }
    fn update_item(
        &self,
        res: &'static Resource,
        id: &Value,
        body: &Map<String, Value>,
    ) -> BResult<Value> {
        self.log(format!(
            "update {} {id} {}",
            res.name,
            Value::Object(body.clone())
        ));
        Ok(json!({}))
    }
    fn add_note(
        &self,
        _res: &'static Resource,
        _id: &Value,
        _body: &str,
        _public: bool,
    ) -> BResult<Value> {
        Err(Failure::unavailable("notes"))
    }
}

fn app(read_only: bool) -> (App, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    let mut app = App::new_inline(mock.clone(), Arc::new(BasicPages), read_only);
    app.mount();
    app.settle();
    app.advance(Duration::from_millis(200));
    assert!(app.selected.is_some());
    (app, mock)
}

fn screen(app: &App) -> String {
    let mut term = match Terminal::new(TestBackend::new(120, 36)) {
        Ok(t) => t,
        Err(e) => panic!("terminal: {e}"),
    };
    if let Err(e) = term.draw(|f| view::draw(app, f)) {
        panic!("draw: {e}");
    }
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

#[test]
fn list_screen_renders_and_navigates() {
    let (mut app, mock) = app(false);
    let s = screen(&app);
    assert!(s.contains("TICKETS"), "{s}");
    assert!(s.contains("tickets row 1"), "{s}");
    assert!(s.contains("tickets row 3"), "{s}");
    assert_eq!(app.rows.len(), 3);
    assert_eq!(app.cursor, 0);

    app.press("j");
    app.press("j");
    assert_eq!(app.cursor, 2);
    app.press("k");
    assert_eq!(app.cursor, 1);
    app.press("G");
    assert_eq!(app.cursor, 2);
    app.press("g");
    app.press("g");
    assert_eq!(app.cursor, 0);
    app.advance(Duration::from_millis(200));
    assert!(!mock.calls("detail tickets").is_empty());

    app.press("g");
    app.press("c");
    app.advance(Duration::from_millis(250));
    assert_eq!(app.entity, "changes");
    let s = screen(&app);
    assert!(s.contains("CHANGES"), "{s}");
    assert!(s.contains("changes row 2"), "{s}");
}

#[test]
fn filter_input_and_history() {
    let (mut app, mock) = app(false);
    app.press("slash");
    assert_eq!(app.focus, Focus::Filter);
    assert!(app.filter.visible);
    app.type_text("status:open");
    app.press("enter");
    app.settle();
    assert_eq!(app.active_filters, vec!["status:open".to_string()]);
    assert!(
        mock.calls("list tickets")
            .iter()
            .any(|c| c.contains("status:open"))
    );

    app.press("slash");
    app.press("ctrl+u");
    app.type_text("priority:high");
    app.press("enter");
    app.settle();

    app.press("slash");
    app.press("ctrl+u");
    app.press("ctrl+p");
    assert_eq!(app.filter.input.value(), "priority:high");
    app.press("ctrl+p");
    assert_eq!(app.filter.input.value(), "status:open");
    app.press("ctrl+n");
    assert_eq!(app.filter.input.value(), "priority:high");
    app.press("escape");
    assert!(!app.filter.visible);
}

#[test]
fn status_modal_confirms_one_write() {
    let (mut app, mock) = app(false);
    eprintln!(
        "sel={:?} toasts={:?}",
        app.selected.as_ref().map(|s| s.key()),
        app.toasts.iter().map(|t| &t.message).collect::<Vec<_>>()
    );
    app.press("s");
    assert!(
        matches!(app.modals.last(), Some((Modal::Status(_), _))),
        "{:?}",
        app.toasts.iter().map(|t| &t.message).collect::<Vec<_>>()
    );
    let s = screen(&app);
    assert!(s.contains("Change status"), "{s}");
    assert!(s.contains("Resolved"), "{s}");

    app.press("down");
    app.press("enter");
    assert!(matches!(app.modals.last(), Some((Modal::Confirm(_), _))));
    let s = screen(&app);
    assert!(s.contains("->"), "{s}");
    app.press("y");
    app.settle();
    assert!(app.modals.is_empty());
    let updates = mock.calls("update");
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(updates[0].contains("\"status\""), "{updates:?}");
}

#[test]
fn status_modal_escape_writes_nothing() {
    let (mut app, mock) = app(false);
    app.press("s");
    app.press("escape");
    app.settle();
    assert!(app.modals.is_empty());
    assert!(mock.calls("update").is_empty());
}

#[test]
fn read_only_blocks_writes() {
    let (mut app, mock) = app(true);
    app.press("s");
    app.settle();
    assert!(app.modals.is_empty());
    assert!(mock.calls("update").is_empty());
    assert!(!app.toasts.is_empty());
}

#[test]
fn help_modal_opens_and_closes() {
    let (mut app, _) = app(false);
    app.press("question_mark");
    assert!(matches!(app.modals.last(), Some((Modal::Help, _))));
    app.press("escape");
    assert!(app.modals.is_empty());
}
