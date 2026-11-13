//! List loading and the table cursor.

use super::*;

impl App {
    pub fn load_list(&mut self, append: bool) {
        let job = self.list_job(append);
        self.spawn_group("list", true, move |b| job.fetch_list(b));
    }

    pub(super) fn list_job(&self, append: bool) -> ListJob {
        let res = resource_for(self.entity);
        ListJob {
            epoch: self.list_epoch,
            entity: self.entity,
            page: if append { self.page + 1 } else { 1 },
            append,
            have: self.schemas.keys().copied().collect(),
            need_me: self.me.is_none(),
            need_views: !self.views.contains_key(res.name),
            need_headers: res.name == "tickets" && !self.headers.contains_key(res.name),
            filters: self.active_filters.clone(),
            schema: self.schemas.get(res.name).cloned(),
            cur_view: self.view.get(res.name).cloned(),
            per_page: self.per_page,
        }
    }

    pub(super) fn merge_loaded(&mut self, loaded: Loaded) {
        for (n, s) in loaded.schemas {
            self.schemas.entry(n).or_insert(s);
        }
        if let Some((me, tz)) = loaded.me
            && !me.is_null()
        {
            self.me = Some(me);
            self.tz = tz;
        }
        if let Some((name, views)) = loaded.views {
            if !self.view.contains_key(name)
                && let Some(v) = site::default_view(&views)
            {
                self.view.insert(name, v.clone());
            }
            self.views.insert(name, views);
        }
        if let Some(h) = loaded.headers {
            self.headers.insert("tickets", h);
        }
        if let Some(s) = loaded.sort {
            self.list_sort = s;
        }
    }

    pub(super) fn list_failed(&mut self, epoch: u64, entity: &str, message: String) {
        if (epoch, entity) != (self.list_epoch, self.entity) {
            return;
        }
        self.loading_more = false;
        self.notify_full(
            message,
            Severity::Error,
            Some("List load failed; reload or retry".into()),
            TOAST_SECS,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn accept_list(
        &mut self,
        epoch: u64,
        entity: &str,
        page: i64,
        items: Vec<Value>,
        total: Option<i64>,
        more: bool,
        append: bool,
    ) {
        if (epoch, entity) != (self.list_epoch, self.entity) || self.search_mode {
            return;
        }
        if append && page != self.page + 1 {
            return;
        }
        self.page = page;
        self.list_total = total;
        let res = resource_for(entity);
        let items: Vec<Item> = items.into_iter().map(|v| Item::new(res, v)).collect();
        let lower = if append {
            self.items.len() + items.len()
        } else {
            items.len()
        } as i64;
        self.populate_list(items, total.unwrap_or(lower), append, Some(more));
    }

    pub(super) fn populate_list(
        &mut self,
        items: Vec<Item>,
        total: i64,
        append: bool,
        has_more: Option<bool>,
    ) {
        self.loading_more = false;
        self.total = total;
        self.has_more = has_more.unwrap_or(items.len() as i64 == self.per_page);
        let mut seen: Vec<String> = if append {
            self.items.iter().map(Item::key).collect()
        } else {
            Vec::new()
        };
        let mut unique = Vec::new();
        for it in items {
            let k = it.key();
            if !seen.contains(&k) {
                seen.push(k);
                unique.push(it);
            }
        }
        let res_now = resource_for(self.entity);
        let offset = if append {
            self.items.len()
        } else {
            self.columns = if self.search_mode {
                Vec::new()
            } else {
                site::list_columns(
                    res_now,
                    self.schemas.get(res_now.name),
                    self.headers.get(res_now.name).map(Vec::as_slice),
                )
            };
            self.rows.clear();
            self.cursor = 0;
            self.table_columns = if self.columns.is_empty() {
                ["", "ID", "STATUS", "PRI", "SUBJECT", "REQUESTER"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                std::iter::once(String::new())
                    .chain(self.columns.iter().map(|c| c.label.clone()))
                    .collect()
            };
            0
        };
        let me_id = self.me.as_ref().and_then(|m| m.get("id")).cloned();
        let empty = json!({"fields": []});
        for (idx, item) in unique.iter().enumerate() {
            let did = item.display_id();
            let sel = format!("{:03}", offset + idx + 1);
            let schema = self.schemas.get(item.res.name).unwrap_or(&empty);
            let mut cells = vec![(TableCell::plain(sel), 0)];
            if !self.columns.is_empty() {
                let row = site::list_row(
                    &item.data,
                    item.res,
                    &self.columns,
                    Some(schema),
                    me_id.as_ref(),
                    self.tz.as_ref(),
                );
                for col in &self.columns {
                    let val = row
                        .get(&col.key)
                        .map(|v| {
                            if let Value::String(s) = v {
                                s.clone()
                            } else {
                                fsv_py::py_str(v)
                            }
                        })
                        .unwrap_or_default();
                    let cell = match col.key.as_str() {
                        "subject" => {
                            let prefix = format!("#{did} ");
                            let n = prefix.chars().count();
                            (TableCell::plain(prefix + &clip(&val, 60)), n)
                        }
                        "status" => (
                            TableCell {
                                text: clip(&val, 24),
                                style: Style::default().fg(Color::Green),
                            },
                            0,
                        ),
                        "assigned_to" if val.ends_with(" (Me)") && val.chars().count() > 40 => {
                            let base: String = val.chars().take(val.chars().count() - 5).collect();
                            (TableCell::plain(clip(&base, 35) + " (Me)"), 0)
                        }
                        k => {
                            let w = if k == "assigned_to" { 40 } else { 34 };
                            let c = clip(&val, w);
                            if c.is_empty() {
                                (
                                    TableCell {
                                        text: "--".into(),
                                        style: Style::default().add_modifier(Modifier::DIM),
                                    },
                                    0,
                                )
                            } else {
                                (TableCell::plain(c), 0)
                            }
                        }
                    };
                    cells.push(cell);
                }
            } else {
                let requester = item
                    .get("requester")
                    .filter(|r| r.is_object())
                    .and_then(|r| r.get("name"))
                    .filter(|n| fsv_py::truthy(n))
                    .map(fsv_py::py_str)
                    .unwrap_or_default();
                cells.push((TableCell::plain(did.clone()), 0));
                cells.push((TableCell::plain(resolve_status(&item.data, schema)), 0));
                cells.push((TableCell::plain(resolve_priority(&item.data)), 0));
                cells.push((TableCell::plain(cut(&item.str("subject"), 80)), 0));
                cells.push((TableCell::plain(requester), 0));
            }
            self.rows.push(Row { key: did, cells });
        }
        let had = !unique.is_empty();
        if append {
            self.items.extend(unique);
        } else {
            self.items = unique;
        }
        if !append && had {
            self.clear_detail("[dim]loading details...[/]");
            self.move_cursor(0, true);
        } else if !append {
            self.clear_detail("[dim]no rows[/]");
        }
    }

    pub(super) fn load_more(&mut self) {
        if self.loading_more || !self.has_more {
            return;
        }
        self.loading_more = true;
        self.load_list(true);
    }

    pub fn reload_list(&mut self, message: &str) {
        self.list_epoch += 1;
        self.list_total = None;
        self.items.clear();
        self.total = 0;
        self.page = 1;
        self.has_more = false;
        self.loading_more = false;
        self.g_pending = None;
        self.filter_error = None;
        self.detail_cache.clear();
        self.sub_cache.clear();
        self.clear_table();
        self.focus_pane(Pane::List);
        self.clear_detail(message);
        self.load_list(false);
    }

    pub(super) fn clear_table(&mut self) {
        self.rows.clear();
        self.table_columns.clear();
        self.cursor = 0;
        self.table_offset.set(0);
    }

    // ── table cursor ────────────────────────────────────────

    pub(super) fn move_cursor(&mut self, row: usize, force: bool) {
        if self.rows.is_empty() {
            return;
        }
        let row = row.min(self.rows.len() - 1);
        let changed = row != self.cursor;
        self.cursor = row;
        if changed || force {
            self.row_highlighted();
        }
    }

    pub(super) fn cursor_step(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize;
        self.move_cursor(next, false);
    }

    pub(super) fn find_item(&self, did: &str) -> Option<Item> {
        self.items.iter().find(|i| i.display_id() == did).cloned()
    }

    /// `FsvApp.on_data_table_row_highlighted`: show the cached record or a
    /// placeholder, debounce the detail load, and page at the last row.
    pub(super) fn row_highlighted(&mut self) {
        let Some(did) = self.rows.get(self.cursor).map(|r| r.key.clone()) else {
            return;
        };
        let Some(item) = self.find_item(&did) else {
            return;
        };
        self.detail_tab_idx = 0;
        let key = item.key();
        if let Some(cached) = self.detail_cache.get(&key).cloned() {
            self.pending_detail_key = Some(key.clone());
            self.pending_sub_key = None;
            self.show_detail(Item::new(item.res, cached), &key);
        } else {
            self.set_pending_detail(&key, "[dim]loading details...[/]");
        }
        self.set_timer(Slot::Detail, DETAIL_DEBOUNCE, TimerKind::Detail(item, key));
        if self.cursor + 1 == self.rows.len() && !self.loading_more {
            if self.search_mode && self.search_has_more {
                self.search_load_more();
            } else if self.has_more {
                self.load_more();
            }
        }
    }
}
