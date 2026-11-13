//! Freshservice search mode.

use super::*;

impl App {
    pub(super) fn open_search_input(&mut self) {
        let v = if self.search_mode {
            self.search_term.clone()
        } else {
            String::new()
        };
        self.search_input.set_value(&v);
        self.search_input_visible = true;
        self.focus = Focus::Search;
    }

    pub(super) fn reset_search_rows(&mut self) {
        self.items.clear();
        self.total = 0;
        self.page = 1;
        self.has_more = false;
        self.detail_cache.clear();
        self.sub_cache.clear();
        self.clear_table();
    }

    pub(super) fn apply_search(&mut self, raw: &str) {
        self.search_input_visible = false;
        let term = raw.trim().to_string();
        if term.is_empty() {
            if self.search_mode {
                self.exit_search_mode();
            } else {
                self.focus = Focus::List;
            }
            return;
        }
        self.search_seq += 1;
        self.search_cache.clear();
        self.search_mode = true;
        self.search_term = term;
        self.search_entity = None;
        self.search_sort = "relevance";
        self.search_page = 1;
        self.search_has_more = false;
        self.active_filters.clear();
        self.filter_error = None;
        self.reset_search_rows();
        self.focus_pane(Pane::List);
        self.clear_detail("[dim]searching...[/]");
        self.load_search(false);
    }

    pub(super) fn exit_search_mode(&mut self) {
        self.search_seq += 1;
        self.search_cache.clear();
        self.search_mode = false;
        self.search_term.clear();
        self.search_totals.clear();
        self.search_entity = None;
        self.search_sort = "relevance";
        self.search_page = 1;
        self.search_has_more = false;
        self.active_filters = self.get_default_filters(self.entity);
        self.filter_error = None;
        self.reload_list("[dim]loading rows...[/]");
    }

    pub(super) fn step_search_entity(&mut self, delta: isize) {
        let tabs: [Option<&'static str>; 4] =
            [None, Some("tickets"), Some("problems"), Some("changes")];
        let idx = tabs
            .iter()
            .position(|t| *t == self.search_entity)
            .unwrap_or(0) as isize;
        let new = tabs[(idx + delta).rem_euclid(4) as usize];
        if new == self.search_entity {
            return;
        }
        self.save_search_cache();
        if self.search_cache.contains_key(&new) {
            self.restore_search_cache(new);
            return;
        }
        match new {
            None => {
                self.search_seq += 1;
                self.search_entity = None;
                self.search_page = 1;
                self.search_has_more = false;
                self.reset_search_rows();
                self.clear_detail("[dim]searching...[/]");
                self.load_search(false);
            }
            Some(e) => self.search_narrow_entity(e),
        }
    }

    pub(super) fn search_narrow_entity(&mut self, entity: &'static str) {
        if !self.search_mode || self.search_term.is_empty() {
            return;
        }
        self.search_seq += 1;
        self.search_entity = Some(entity);
        self.search_page = 1;
        self.search_has_more = false;
        self.reset_search_rows();
        self.clear_detail("[dim]searching...[/]");
        self.load_search(false);
    }

    pub(super) fn save_search_cache(&mut self) {
        if !self.items.is_empty() {
            self.search_cache.insert(
                self.search_entity,
                (
                    self.items.clone(),
                    self.search_totals.clone(),
                    self.search_page,
                    self.search_has_more,
                ),
            );
        }
    }

    pub(super) fn search_total(
        &self,
        totals: &[(String, i64)],
        entity: Option<&str>,
    ) -> Option<i64> {
        entity.and_then(|e| totals.iter().find(|(k, _)| k == e).map(|(_, c)| *c))
    }

    pub(super) fn search_row(idx: usize, item: &Item) -> Row {
        let cells = vec![
            TableCell::plain(format!("{:03}", idx + 1)),
            TableCell::plain(item.display_id()),
            TableCell::plain(
                item.get("status")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
            ),
            TableCell::plain(cut(
                &item
                    .get("subject")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
                80,
            )),
            TableCell::plain(
                item.get("_agent")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
            ),
            TableCell::plain(
                item.get("_group")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
            ),
        ];
        Row {
            key: item.display_id(),
            cells: cells.into_iter().map(|c| (c, 0)).collect(),
        }
    }

    pub(super) fn search_columns(&mut self) {
        self.table_columns = ["", "ID", "STATUS", "SUBJECT", "AGENT", "GROUP"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    }

    pub(super) fn restore_search_cache(&mut self, entity: Option<&'static str>) {
        let Some((items, totals, page, more)) = self.search_cache.get(&entity).cloned() else {
            return;
        };
        self.search_seq += 1;
        self.search_entity = entity;
        self.search_page = page;
        self.search_has_more = more;
        self.items = items;
        self.total = self
            .search_total(&totals, entity)
            .unwrap_or_else(|| totals.iter().map(|(_, c)| c).sum());
        self.search_totals = totals;
        self.clear_table();
        self.search_columns();
        self.rows = self
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| App::search_row(i, it))
            .collect();
        if self.items.is_empty() {
            self.clear_detail("[dim]no results[/]");
        } else {
            self.move_cursor(0, true);
            self.clear_detail("[dim]loading details...[/]");
        }
    }

    pub(super) fn load_search(&mut self, append: bool) {
        let seq = self.search_seq;
        let have: Vec<&'static str> = self.schemas.keys().copied().collect();
        let (term, entity, sort, page) = (
            self.search_term.clone(),
            self.search_entity,
            self.search_sort,
            self.search_page,
        );
        self.spawn_group("list", true, move |b| {
            let mut schemas = Vec::new();
            let outcome = (|| -> Result<SearchResult, String> {
                for r in ALL_RESOURCES {
                    if !have.contains(&r.name) {
                        schemas.push((r.name, b.load_schema(r).map_err(|e| e.to_string())?));
                    }
                }
                b.search_items(&term, entity, sort, page)
                    .map_err(|e| e.to_string())
            })();
            Msg::Search {
                seq,
                append,
                schemas,
                outcome,
            }
        });
    }

    pub(super) fn populate_search(
        &mut self,
        items: Vec<Item>,
        totals: Vec<(String, i64)>,
        append: bool,
        seq: u64,
    ) {
        if seq != self.search_seq {
            return;
        }
        self.loading_more = false;
        match self.search_total(&totals, self.search_entity) {
            Some(t) => {
                self.total = t;
                self.search_has_more = items.len() == 30;
            }
            None => {
                self.total = totals.iter().map(|(_, c)| c).sum();
                self.search_has_more = false;
            }
        }
        self.search_totals = totals;
        let offset = if append {
            self.items.len()
        } else {
            self.clear_table();
            self.search_columns();
            self.items.clear();
            0
        };
        let had = !items.is_empty();
        for (i, it) in items.iter().enumerate() {
            let did = it.display_id();
            if self.rows.iter().any(|r| r.key == did) {
                continue;
            }
            self.rows.push(App::search_row(offset + i, it));
        }
        self.items.extend(items);
        if !append && had {
            self.clear_detail("[dim]loading details...[/]");
            self.move_cursor(0, true);
        } else if !append {
            self.clear_detail("[dim]no results[/]");
        }
    }

    pub(super) fn search_load_more(&mut self) {
        if !self.search_has_more || self.loading_more {
            return;
        }
        self.loading_more = true;
        self.search_page += 1;
        self.load_search(true);
    }

    pub(super) fn cycle_search_sort(&mut self) {
        self.search_seq += 1;
        self.search_cache.clear();
        let order = ["relevance", "created", "modified"];
        let idx = order
            .iter()
            .position(|o| *o == self.search_sort)
            .unwrap_or(0);
        self.search_sort = order[(idx + 1) % 3];
        self.search_page = 1;
        self.search_has_more = false;
        self.items.clear();
        self.total = 0;
        self.detail_cache.clear();
        self.sub_cache.clear();
        self.clear_table();
        self.clear_detail("[dim]searching...[/]");
        self.load_search(false);
    }
}
