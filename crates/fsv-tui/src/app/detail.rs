//! The detail pane: record, tabs and sub-resources.

use super::*;

impl App {
    pub(super) fn load_detail(&mut self, item: Item, key: String) {
        let cache_key = item.key();
        if let (Some(rec), true) = (
            self.detail_cache.get(&cache_key).cloned(),
            self.details.contains_key(&cache_key),
        ) {
            self.show_detail(Item::new(item.res, rec), &key);
            return;
        }
        let schema = self.schemas.get(item.res.name).cloned();
        self.spawn_group("detail", true, move |b| {
            let outcome = b
                .load_detail(item.res, &item.id(), schema.as_ref())
                .map_err(|e| actions::clean_error(&e));
            Msg::Detail { item, key, outcome }
        });
    }

    pub(super) fn detail_error(&mut self, key: &str, message: String) {
        if self.pending_detail_key.as_deref() != Some(key) {
            return;
        }
        self.detail.content = Content::Plain(
            format!("could not load: {message}"),
            Style::default().fg(Color::Red),
        );
    }

    pub(super) fn page_for(&self, item: Option<&Item>) -> Option<&DetailPage> {
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

    pub(super) fn show_detail(&mut self, item: Item, key: &str) {
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

    pub(super) fn focus_pane(&mut self, pane: Pane) {
        self.focus = if pane == Pane::Detail {
            Focus::Detail
        } else {
            Focus::List
        };
    }

    pub(super) fn toggle_pane(&mut self) {
        self.focus_pane(if self.active_pane() == Pane::List {
            Pane::Detail
        } else {
            Pane::List
        });
    }

    pub(super) fn selected_key(&self) -> Option<String> {
        self.selected.as_ref().map(Item::key)
    }

    /// The properties side panel; `None` hides it.
    pub(super) fn show_side(&mut self, lines: Option<Vec<String>>) {
        self.detail.side = lines;
    }

    pub(super) fn scroll_detail_home(&mut self) {
        self.detail.scroll = 0;
    }

    /// Python defers this until after the refresh; the next draw is that.
    pub(super) fn reset_detail_scroll(&mut self) {
        self.scroll_detail_home();
    }

    pub(super) fn clear_detail(&mut self, message: &str) {
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

    pub(super) fn set_pending_detail(&mut self, key: &str, message: &str) {
        self.selected = None;
        self.pending_detail_key = Some(key.to_string());
        self.pending_sub_key = None;
        self.detail.bar.clear();
        self.detail.tabs.clear();
        self.show_side(None);
        self.detail.content = Content::Markup(message.to_string());
        self.reset_detail_scroll();
    }

    pub(super) fn trigger_state_flow(&mut self, item: &Item) {
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

    pub(super) fn render_detail_bar(&mut self, item: &Item) {
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

    pub(super) fn render_detail_tabs(&mut self, res: Res) {
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

    pub(super) fn render_detail_content(&mut self, item: &Item) {
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

    pub(super) fn load_sub_resource(&mut self, tab: String, sub_key: String) {
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
        self.spawn_group("sub", true, move |b| {
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

    pub(super) fn render_sub_content(
        &mut self,
        tab: &str,
        data: &TabData,
        res: Res,
        sub_key: &str,
    ) {
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

    pub(super) fn set_detail_tab(&mut self, idx: usize) {
        let Some(sel) = self.selected.clone() else {
            return;
        };
        self.detail_tab_idx = idx;
        self.render_detail_tabs(sel.res);
        self.render_detail_content(&sel);
    }

    pub(super) fn step_detail_tab(&mut self, delta: isize) {
        let Some(sel) = &self.selected else { return };
        let n = self.tab_names(Some(sel.res)).len().max(1) as isize;
        let next = (self.detail_tab_idx as isize + delta).rem_euclid(n) as usize;
        self.set_detail_tab(next);
    }

    pub(super) fn detail_scroll(&mut self, delta: i32) {
        let max = self.detail_max_scroll.get() as i32;
        self.detail.scroll = (self.detail.scroll as i32 + delta).clamp(0, max.max(0)) as u16;
    }
}
