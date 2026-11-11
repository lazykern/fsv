//! The key handler.

use super::*;

impl App {
    pub fn press(&mut self, name: &str) {
        self.handle_key(&keys::parse(name));
    }

    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.handle_key(&KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
    }

    /// `FsvApp.on_key`: ctrl+c, then the open modal, the search or filter
    /// input, and finally the main-pane keys.
    pub fn handle_key(&mut self, ev: &KeyEvent) {
        let key = keys::name(ev);
        if key == "ctrl+c" {
            self.quit = true;
            return;
        }
        if self.form.is_some() && self.modals.is_empty() {
            if key == "ctrl+q" {
                self.quit = true;
                return;
            }
            if let Some(form) = &self.form {
                form.view.borrow_mut().handle_key(ev);
            }
            self.check_form_done();
            return;
        }
        if let Some((modal, _)) = self.modals.last_mut() {
            match modal.handle(ev) {
                Outcome::Stay => {}
                Outcome::Warn(m) => self.notify(m, Severity::Warning),
                Outcome::Close(result) => {
                    if let Some((_, cont)) = self.modals.pop() {
                        self.resume(cont, result);
                    }
                }
            }
            return;
        }
        match self.focus {
            Focus::Search => {
                if key == "escape" {
                    self.cancel_search_input();
                } else if self.search_input.handle(ev) == InputEvent::Submitted {
                    self.on_input_submitted(Focus::Search);
                }
                return;
            }
            Focus::Filter => {
                self.filter_key(ev, &key);
                return;
            }
            _ => {}
        }
        self.main_key(&key);
    }

    pub(super) fn main_key(&mut self, key: &str) {
        let pane = self.active_pane();
        if let Some(pending) = self.g_pending.take() {
            if key == "g" {
                if pending == Pane::Detail {
                    self.scroll_detail_home();
                } else {
                    self.move_cursor(0, false);
                }
            } else if let Some(e) = entity_key(key) {
                if self.search_mode {
                    self.search_narrow_entity(e);
                } else {
                    self.switch_entity(e);
                }
            }
            return;
        }
        match key {
            "g" => self.g_pending = Some(pane),
            "f" => self.open_search_input(),
            "escape" if self.search_mode => self.exit_search_mode(),
            "s" if self.search_mode => self.cycle_search_sort(),
            "slash" => {
                if !self.search_mode {
                    self.open_filter_input();
                }
            }
            "tab" => self.toggle_pane(),
            "ctrl+j" => self.focus_pane(Pane::Detail),
            "ctrl+k" => self.focus_pane(Pane::List),
            "question_mark" => self.push(Modal::Help, Cont::None),
            "j" | "down" => {
                if pane == Pane::Detail {
                    self.detail_scroll(1)
                } else {
                    self.cursor_step(1)
                }
            }
            "k" | "up" => {
                if pane == Pane::Detail {
                    self.detail_scroll(-1)
                } else {
                    self.cursor_step(-1)
                }
            }
            "J" | "right_curly_bracket" => {
                for _ in 0..5 {
                    if pane == Pane::Detail {
                        self.detail_scroll(1)
                    } else {
                        self.cursor_step(1)
                    }
                }
            }
            "K" | "left_curly_bracket" => {
                for _ in 0..5 {
                    if pane == Pane::Detail {
                        self.detail_scroll(-1)
                    } else {
                        self.cursor_step(-1)
                    }
                }
            }
            "G" => {
                if pane == Pane::Detail {
                    self.detail.scroll = self.detail_max_scroll.get();
                } else if !self.rows.is_empty() {
                    let last = self.rows.len() - 1;
                    self.move_cursor(last, false);
                }
            }
            "h" | "left" => {
                if pane == Pane::Detail {
                    self.step_detail_tab(-1)
                } else if self.search_mode {
                    self.step_search_entity(-1)
                } else {
                    self.prev_entity()
                }
            }
            "l" | "right" => {
                if pane == Pane::Detail {
                    self.step_detail_tab(1)
                } else if self.search_mode {
                    self.step_search_entity(1)
                } else {
                    self.next_entity()
                }
            }
            "o" => self.open_browser(),
            "i" => self.asset_inventory_prompt(),
            "a" => self.action_menu(),
            "exclamation_mark" => {
                if let Some(m) = self.write_failure.clone() {
                    self.push(Modal::failure(&m), Cont::None);
                }
            }
            "s" | "n" => self.write_key(key),
            "r" if pane == Pane::Detail && self.reply_possible() => self.write_key(key),
            "r" => {
                if self.search_mode {
                    let t = self.search_term.clone();
                    self.apply_search(&t);
                } else {
                    self.reload_list("[dim]reloading...[/]");
                }
            }
            "y" => self.yank(),
            "q" => self.quit = true,
            "left_square_bracket" => self.resize_panes(-1),
            "right_square_bracket" => self.resize_panes(1),
            "v" => self.pick_view(),
            "N" => self.open_form("new"),
            "e" => self.open_form("edit"),
            d if d.len() == 1 && matches!(d.as_bytes()[0], b'1'..=b'9') => {
                let idx = (d.as_bytes()[0] - b'1') as usize;
                if let Some(sel) = &self.selected
                    && idx < self.tab_names(Some(sel.res)).len()
                {
                    self.set_detail_tab(idx);
                }
            }
            "pagedown" | "pageup" | "home" | "end" => {
                let page = self.table_height.get().max(1) as isize;
                if pane == Pane::Detail {
                    match key {
                        "pagedown" => self.detail_scroll(page as i32),
                        "pageup" => self.detail_scroll(-(page as i32)),
                        "home" => self.detail.scroll = 0,
                        _ => self.detail.scroll = self.detail_max_scroll.get(),
                    }
                } else {
                    match key {
                        "pagedown" => self.cursor_step(page),
                        "pageup" => self.cursor_step(-page),
                        "home" => self.move_cursor(0, false),
                        _ => {
                            let last = self.rows.len().saturating_sub(1);
                            self.move_cursor(last, false)
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn resize_panes(&mut self, delta: i32) {
        self.list_fr = (self.list_fr as i32 + delta).clamp(3, 17) as u16;
        self.detail_fr = (self.detail_fr as i32 - delta).clamp(3, 17) as u16;
    }

    pub(super) fn open_browser(&mut self) {
        let Some(sel) = self.selected.clone() else {
            self.notify("No row selected", Severity::Warning);
            return;
        };
        let url = self.backend.item_url(sel.res, &sel.id());
        if !self.backend.open_url(&url) {
            self.notify("Browser open failed", Severity::Warning);
        }
    }

    pub(super) fn yank(&mut self) {
        let Some(sel) = self.selected.clone() else {
            self.notify("No row selected", Severity::Warning);
            return;
        };
        let did = sel.display_id();
        match self.backend.copy_to_clipboard(&did) {
            Ok(()) => self.notify(format!("Copied {did}"), Severity::Information),
            Err(e) => self.notify(format!("Copy failed: {e}"), Severity::Warning),
        }
    }
}
