//! The `/` filter input and its completions.

use super::*;

impl App {
    pub(super) fn with_source<R>(
        &mut self,
        f: impl FnOnce(&mut FilterState, &Source<'_>) -> R,
    ) -> R {
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

    pub(super) fn open_filter_input(&mut self) {
        let current = self.active_filters.join(" ");
        self.with_source(|st, src| st.open(&current, src));
        self.focus = Focus::Filter;
        self.maybe_schedule_lookup();
    }

    /// `FsvApp._cancel_filter_input`; applying a filter closes it the same way.
    pub(super) fn close_filter(&mut self) {
        self.filter.close();
        self.stop_timer(Slot::Lookup);
        self.focus = Focus::List;
    }

    pub(super) fn apply_filter(&mut self, raw: &str) {
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
    pub(super) fn maybe_schedule_lookup(&mut self) {
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

    pub(super) fn run_network_lookup(&mut self, req: LookupRequest) {
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

    pub(super) fn filter_key(&mut self, ev: &KeyEvent, key: &str) {
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
    pub(super) fn on_input_submitted(&mut self, input: Focus) {
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
    pub(super) fn on_input_changed(&mut self) {
        self.with_source(|st, src| st.typed(src));
        self.maybe_schedule_lookup();
    }

    pub(super) fn cancel_search_input(&mut self) {
        self.search_input_visible = false;
        self.focus = Focus::List;
    }
}
