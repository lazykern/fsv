//! Runtime: workers, timers, messages and the UI updates they carry.

use super::*;

impl App {
    /// Textual `@work(group=..., exclusive=...)`.
    ///
    /// An exclusive start bumps the group, so an older result from that group
    /// is dropped. A non-exclusive start keeps the current generation: it does
    /// not cancel anyone, and an exclusive start that lands first drops it.
    pub(super) fn spawn_group(
        &mut self,
        group: &'static str,
        exclusive: bool,
        job: impl FnOnce(&dyn Backend) -> Msg + Send + 'static,
    ) {
        let generation = {
            let slot = self.worker_gen.entry(group).or_insert(0);
            if exclusive {
                *slot = slot.saturating_add(1);
            }
            *slot
        };
        self.spawn(move |b| Msg::Grouped {
            group,
            generation,
            inner: Box::new(job(b)),
        });
    }

    pub(super) fn spawn(&mut self, job: impl FnOnce(&dyn Backend) -> Msg + Send + 'static) {
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

    pub(super) fn set_timer(&mut self, slot: Slot, delay: Duration, kind: TimerKind) {
        self.stop_timer(slot);
        let due = self.clock.now() + delay;
        self.timers.push(Timer { due, slot, kind });
    }

    pub(super) fn stop_timer(&mut self, slot: Slot) {
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

    pub(super) fn fire_timers(&mut self) {
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

    pub(super) fn notify_full(
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

    pub(super) fn push(&mut self, modal: Modal, cont: Cont) {
        self.modals.push((modal, cont));
    }

    pub(super) fn show_failure(&mut self, message: String) {
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
            Msg::Grouped {
                group,
                generation,
                inner,
            } => {
                if self.worker_gen.get(group).copied() == Some(generation) {
                    self.on_msg(*inner);
                }
            }
        }
    }

    pub(super) fn on_ui(&mut self, u: Ui) {
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
}
