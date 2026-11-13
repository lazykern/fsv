//! Create/edit forms and the service catalog.

use super::*;

impl App {
    pub(super) fn open_form(&mut self, mode: &str) {
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

    pub(super) fn create_choice(&mut self, kind: &str) {
        if self.read_only {
            return;
        }
        if kind == "sr" {
            self.load_catalog();
        } else {
            self.launch_create(kind, None);
        }
    }

    pub(super) fn launch_create(&mut self, kind: &str, item: Option<Value>) {
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
    pub(super) fn build_form(&mut self, mode: &str, data: Option<Value>, item: Option<Item>) {
        let record = self
            .page_for(item.as_ref())
            .map(|p| p.record.clone())
            .or_else(|| data.clone());
        let req = FormRequest {
            mode: mode.to_string(),
            item: data,
            record,
        };
        self.spawn_group("form", true, move |b| {
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
    pub(super) fn push_form(&mut self, opened: OpenedForm) {
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
    pub(super) fn check_form_done(&mut self) {
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
    pub(super) fn send_form(&mut self, send: Box<dyn FormSend>, mode: String, item: Option<Item>) {
        if self.read_only {
            return;
        }
        self.spawn_group("write", false, move |_| {
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

    pub(super) fn load_catalog(&mut self) {
        self.spawn_group("catalog", true, |b| {
            Msg::Ui(vec![match b.catalog() {
                Ok((items, cats)) => Ui::Catalog(items, cats),
                Err(e) => Ui::Failure(format!("Catalog unavailable: {e}")),
            }])
        });
    }

    pub(super) fn pick_catalog(&mut self, items: Vec<Value>, categories: Vec<Value>) {
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
