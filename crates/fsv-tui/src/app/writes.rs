//! Write actions.

use super::*;

impl App {
    pub(super) fn reply_possible(&self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|s| !self.read_only && actions::can_reply(s.res))
    }

    pub(super) fn write_key(&mut self, key: &str) {
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

    pub(super) fn start_status(&mut self, item: Item) {
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

    pub(super) fn start_note(&mut self, item: Item, reply: bool) {
        if item.res.name == "changes" && !reply {
            self.show_failure(
                "Change-note visibility and a shared reviewed CLI contract are unavailable. Use the web form; the legacy TUI note sender is disabled.".to_string(),
            );
            return;
        }
        let ok = if reply {
            actions::can_reply(item.res)
        } else {
            actions::can_note(item.res)
        };
        if !ok {
            self.notify("Not available for this item type", Severity::Warning);
            return;
        }
        let mut title = self.target_label(&item);
        if item.res.name == "problems" {
            title.push_str(" — visibility/notifications not proven");
        }
        let modal = NoteModal::new(&title, reply, item.res.name == "tickets")
            .with_ticket_options(item.res.name == "tickets");
        self.push(Modal::Note(modal), Cont::NoteComposed { item, reply });
    }

    pub(super) fn target_label(&self, item: &Item) -> String {
        item.display_id()
    }

    pub(super) fn review_ticket_note(&mut self, item: Item, body: String, public: bool) {
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

    pub(super) fn send_ticket_note(&mut self, plan: ReviewPlan, public_confirmed: bool) {
        if self.read_only {
            return;
        }
        let read_only = self.read_only;
        self.spawn_group("write", false, move |b| {
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

    pub(super) fn do_write(&mut self, kind: WriteKind, item: Item) {
        match &kind {
            WriteKind::Status(id, label) => {
                let _ = (id, label, &item);
            }
            WriteKind::Note(body, public) => {
                let _ = (body, public, &item);
            }
        }
        if self.read_only {
            return;
        }
        self.show_failure(
            "Legacy TUI send path disabled; prepare and confirm an exact shared reviewed plan. Nothing sent.".to_string(),
        );
    }

    pub(super) fn after_write(&mut self, res: Res, item: Item, msg: &str) {
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

    pub(super) fn action_menu(&mut self) {
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

    pub(super) fn prepare_action(&mut self, kind: &str, item: Item) {
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
    pub(super) fn compose_change_task(&mut self, item: Item) {
        if self.read_only || item.res.name != "changes" {
            return;
        }
        self.push(
            Modal::value("New change task title — Open; reminder Never", ""),
            Cont::TaskTitle { item },
        );
    }

    pub(super) fn task_titled(&mut self, item: Item, title: String) {
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

    pub(super) fn review_change_task(&mut self, item: Item, plan: ReviewPlan) {
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
    pub(super) fn compose_approval_request(&mut self, item: Item) {
        self.push(
            Modal::value("Approval group name", ""),
            Cont::ApprovalName { item },
        );
    }

    pub(super) fn approval_result(&mut self, item: Item, result: Value) {
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

    pub(super) fn prepare_approval_request(
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
        self.spawn_group("approval-review", true, move |b| {
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

    pub(super) fn load_approval_targets(&mut self, kind: String, item: Item) {
        if self.read_only {
            return;
        }
        self.spawn_group("approval-read", true, move |b| {
            Msg::Ui(vec![match b.approval_targets(&kind, &item.ref_id()) {
                Ok(rows) => Ui::ApprovalTargets { kind, item, rows },
                Err(e) => Ui::Failure(format!("Approval inventory unavailable: {e}")),
            }])
        });
    }

    pub(super) fn pick_approval_target(&mut self, kind: String, item: Item, rows: Vec<Value>) {
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

    pub(super) fn prepare_approval_action(
        &mut self,
        item: Item,
        row: Value,
        op: &str,
        remark: Option<&str>,
    ) {
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

    pub(super) fn review_approval(&mut self, item: Item, plan: ReviewPlan, before: Option<Value>) {
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

    pub(super) fn send_approval(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        let read_only = self.read_only;
        self.spawn_group("write", false, move |b| {
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
    pub(super) fn prepare_download(
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

    pub(super) fn change_task_options(
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

    pub(super) fn prepare_change_task(
        &mut self,
        item: Item,
        title: String,
        owner: Option<String>,
        values: Vec<(String, String)>,
    ) {
        if self.read_only {
            return;
        }
        self.spawn_group("task-create-review", true, move |b| {
            let ui = match b.change_task_plan(&item.ref_id(), &title, owner.as_deref(), &values) {
                Ok(plan) => Ui::TaskPlan(item, plan),
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
            };
            Msg::Ui(vec![ui])
        });
    }

    pub(super) fn send_change_task(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        let read_only = self.read_only;
        self.spawn_group("write", false, move |b| {
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

    pub(super) fn load_action_targets(&mut self, kind: String, item: Item, query: String) {
        self.spawn_group("action-read", true, move |b| {
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

    pub(super) fn pick_action_target(
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

    pub(super) fn load_task_editor(&mut self, item: Item, target: Value) {
        self.spawn_group("task-read", true, move |b| {
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

    pub(super) fn edit_task_field(
        &mut self,
        item: Item,
        target: Value,
        current: Value,
        fields: Vec<Value>,
    ) {
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

    pub(super) fn confirm_action(
        &mut self,
        kind: String,
        item: Item,
        target: Value,
        body: Map<String, Value>,
    ) {
        if self.read_only {
            return;
        }
        if kind == "task-update" || kind == "task-delete" {
            self.prepare_task_mutation(kind, item, target, body);
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

    pub(super) fn execute_action(
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
        self.spawn_group("write", false, move |b| {
            Msg::Ui(vec![execute_action_job(
                b, &kind, item, &target, &body, plan,
            )])
        });
    }

    pub(super) fn prepare_task_mutation(
        &mut self,
        kind: String,
        item: Item,
        target: Value,
        body: Map<String, Value>,
    ) {
        if self.read_only {
            return;
        }
        let task_id = target.get("id").cloned().unwrap_or(Value::Null);
        self.spawn_group("task-review", true, move |b| {
            let ui = match b.task_mutation_plan(&kind, &item.ref_id(), &task_id, &body) {
                Ok(plan) => Ui::Push(
                    Box::new(Modal::confirm(
                        &format!("Review exact {kind}"),
                        vec![fsv_py::pretty(&plan.review)],
                    )),
                    Cont::TaskMutationReviewed { item, plan },
                ),
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
            };
            Msg::Ui(vec![ui])
        });
    }

    pub(super) fn send_task_mutation(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        self.spawn_group("write", false, move |b| {
            outcome_msg(
                b.send_task_mutation(&plan),
                "Task mutation outcome — do not resend",
                &CHANGES,
                item,
                "Exact task mutation verified",
            )
        });
    }

    pub(super) fn prepare_problem_note(&mut self, item: Item, body: String) {
        if self.read_only {
            return;
        }
        self.spawn_group("note-review", true, move |b| {
            let ui = match b.problem_note_plan(&item.ref_id(), &body) {
                Ok(plan) => Ui::Push(
                    Box::new(Modal::confirm(
                        "Review exact problem note — visibility not proven",
                        vec![
                            fsv_py::pretty(&plan.review),
                            "Body-only native note. Privacy and notifications are not established; inspect before confirming.".into(),
                        ],
                    )),
                    Cont::ProblemNoteReviewed { item, plan },
                ),
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
            };
            Msg::Ui(vec![ui])
        });
    }

    pub(super) fn send_problem_note(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        self.spawn_group("write", false, move |b| {
            outcome_msg(
                b.send_problem_note(&plan),
                "Problem note outcome — do not resend",
                &PROBLEMS,
                item,
                "Exact problem note verified",
            )
        });
    }

    pub(super) fn prepare_change_status(&mut self, item: Item, status: Value) {
        if self.read_only {
            return;
        }
        self.spawn_group("status-review", true, move |b| {
            let ui = match b.change_status_plan(&item.ref_id(), &status) {
                Ok(plan) => Ui::Push(
                    Box::new(Modal::confirm(
                        "Review exact change status",
                        vec![fsv_py::Dumps::default().indent(2).dump(&plan.review)],
                    )),
                    Cont::ChangeStatusReviewed { item, plan },
                ),
                Err(e) => Ui::Failure(actions::failure_detail(&e)),
            };
            Msg::Ui(vec![ui])
        });
    }

    pub(super) fn send_change_status(&mut self, item: Item, plan: ReviewPlan) {
        if self.read_only {
            return;
        }
        self.spawn_group("write", false, move |b| {
            outcome_msg(
                b.send_change_status(&plan),
                "Form outcome — do not resend",
                &CHANGES,
                item,
                "Change properties verified",
            )
        });
    }
}

fn outcome_msg(
    result: Result<Value, crate::backend::Failure>,
    title: &str,
    res: Res,
    item: Item,
    verified: &str,
) -> Msg {
    match result {
        Err(e) => Msg::Ui(vec![Ui::Failure(actions::failure_detail(&e))]),
        Ok(r) => {
            let mut ui = vec![Ui::Outcome {
                title: title.into(),
                message: fsv_py::Dumps::default().indent(2).dump(&r),
            }];
            if r.get("verified").is_some_and(fsv_py::truthy) {
                ui.push(Ui::AfterWrite(res, item, verified.into()));
            }
            Msg::Ui(ui)
        }
    }
}
