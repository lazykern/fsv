//! Entity tabs and saved views.

use super::*;

impl App {
    pub(super) fn switch_entity(&mut self, name: &'static str) {
        if name == self.entity {
            return;
        }
        self.entity = name;
        self.detail_tab_idx = 0;
        self.list_epoch += 1;
        self.has_more = false;
        self.loading_more = false;
        self.active_filters = self.get_default_filters(name);
        self.filter_error = None;
        self.set_timer(Slot::Entity, ENTITY_DEBOUNCE, TimerKind::Entity);
    }

    /// `config.get_default_where(entity)`; the backend maps errors to `[]`.
    pub(super) fn get_default_filters(&self, entity: &str) -> Vec<String> {
        self.backend.default_where(entity)
    }

    pub(super) fn step_entity(&mut self, delta: isize) {
        let idx = ENTITY_TABS
            .iter()
            .position(|e| *e == self.entity)
            .unwrap_or(0) as isize;
        self.switch_entity(ENTITY_TABS[(idx + delta).rem_euclid(3) as usize]);
    }

    pub(super) fn next_entity(&mut self) {
        self.step_entity(1);
    }

    pub(super) fn prev_entity(&mut self) {
        self.step_entity(-1);
    }

    /// Port of fsv.tui.app.FsvApp._pick_view.
    pub(super) fn pick_view(&mut self) {
        if self.search_mode {
            return;
        }
        let res = resource_for(self.entity);
        let views = self.views.get(res.name).cloned().unwrap_or_default();
        if views.is_empty() {
            self.notify(
                format!("No saved views on this account for {}", res.name),
                Severity::Warning,
            );
            return;
        }
        let current = self.view.get(res.name);
        let options = views
            .iter()
            .enumerate()
            .map(|(i, v)| (Value::from(i), v.name.clone(), current == Some(v), v.system))
            .collect();
        let title = format!("{} views", capitalize(res.name));
        self.push(
            Modal::Picker(PickerModal::views(&title, options)),
            Cont::ViewPicked { res, views },
        );
    }
}
