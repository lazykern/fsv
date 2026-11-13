//! Device and service associations (`i`): browse one search page, or review
//! one add or remove before sending. Read-only mode only browses devices.

use fsv_py::{Dumps, py_str, truthy};
use serde_json::{Value, json};

use super::{App, Cont, Msg, Severity, Ui};
use crate::actions;
use crate::backend::{AssetCandidates, Item, ReviewPlan};
use crate::modals::{Modal, ModalResult};

pub const SEARCH_PROMPT: &str = "Read-only asset search — selection does not associate";
pub const PICKER_TITLE: &str = "Asset candidates — page 1 only; read-only";
pub const EMPTY_TITLE: &str = "Asset candidates — read-only";
pub const EMPTY_MESSAGE: &str = "No candidates on search page 1. No association was attempted.";
pub const SELECTED_TITLE: &str = "Selected asset — no association";
pub const MENU_TITLE: &str = "Inventory associations";
pub const REVIEW_TITLE: &str = "Review exact inventory association";
pub const REMOVE_TITLE: &str = "Remove exact associated item — selection prepares review";
pub const ASSOCIATED_TITLE: &str = "Associated inventory";
pub const ASSOCIATED_EMPTY: &str = "No associated items. Nothing sent.";
pub const OUTCOME_TITLE: &str = "Inventory association outcome — do not resend";

/// A modal's continuation inside the inventory flow.
#[derive(Clone, Debug)]
pub enum Step {
    Menu {
        item: Item,
    },
    Search {
        item: Item,
        kind: String,
        add: bool,
    },
    Pick {
        item: Item,
        kind: String,
        add: bool,
        result: AssetCandidates,
    },
    Remove {
        item: Item,
        kind: String,
    },
    Confirm {
        module: String,
        kind: String,
        ident: Value,
        remove: bool,
        plan: ReviewPlan,
    },
}

pub fn inventory_menu() -> Vec<(Value, String)> {
    vec![
        (
            json!("browse:devices"),
            "Browse devices/CI — no association".into(),
        ),
        (
            json!("add:devices"),
            "Add device/CI — review before sending".into(),
        ),
        (
            json!("remove:devices"),
            "Remove associated device/CI — review before sending".into(),
        ),
        (
            json!("add:services"),
            "Add business service — review before sending".into(),
        ),
        (
            json!("remove:services"),
            "Remove associated business service — review before sending".into(),
        ),
    ]
}

fn search_title(kind: &str, add: bool) -> String {
    if add {
        format!("Search {kind} — selection prepares review, not a send")
    } else {
        SEARCH_PROMPT.into()
    }
}

fn picker_title(add: bool) -> String {
    if add {
        "Asset candidates — page 1 only; selection prepares review".into()
    } else {
        PICKER_TITLE.into()
    }
}

/// `f"[{row['display_id']}] {row.get('name') or '-'} — {row.get('ci_type_name') or '-'}"`,
/// keyed by the row's `display_id`.
pub fn asset_choices(result: &AssetCandidates) -> Vec<(Value, String)> {
    let or_dash = |v: Option<&Value>| v.filter(|v| truthy(v)).map_or("-".to_string(), py_str);
    result
        .assets
        .iter()
        .map(|row| {
            let id = row.get("display_id").cloned().unwrap_or(Value::Null);
            let label = format!(
                "[{}] {} — {}",
                py_str(&id),
                or_dash(row.get("name")),
                or_dash(row.get("ci_type_name"))
            );
            (id, label)
        })
        .collect()
}

fn unavailable(message: String) -> Ui {
    Ui::Push(Box::new(Modal::failure(&message)), Cont::None)
}

impl App {
    /// Port of fsv.tui.app.FsvApp._asset_inventory_prompt.
    pub(super) fn asset_inventory_prompt(&mut self) {
        let Some(item) = self
            .selected
            .clone()
            .filter(|s| matches!(s.res.name, "tickets" | "changes" | "problems"))
        else {
            self.notify("Select a ticket, SR, problem or change", Severity::Warning);
            return;
        };
        if self.read_only {
            self.asset_search_prompt(item, "devices".into(), false);
            return;
        }
        self.push(
            Modal::choices(MENU_TITLE, inventory_menu()),
            Cont::Assets(Step::Menu { item }),
        );
    }

    pub(super) fn resume_assets(&mut self, step: Step, result: ModalResult) {
        match (step, result) {
            (Step::Menu { item }, ModalResult::Choice(value)) => {
                let Some((action, kind)) = value.as_str().and_then(|s| s.split_once(':')) else {
                    return;
                };
                let kind = kind.to_string();
                if action == "remove" {
                    self.load_associated_inventory(item, kind);
                } else {
                    self.asset_search_prompt(item, kind, action == "add");
                }
            }
            (Step::Search { item, kind, add }, ModalResult::Text(query)) => {
                self.load_asset_inventory(item, query, kind, add)
            }
            (
                Step::Pick {
                    item,
                    kind,
                    add,
                    result,
                },
                ModalResult::Choice(value),
            ) => {
                let modal = match result.select(&value) {
                    Ok(row) if add => {
                        let ident = row.get("display_id").cloned().unwrap_or(Value::Null);
                        self.prepare_inventory_association(item, kind, ident, false);
                        return;
                    }
                    Ok(row) => Modal::outcome(
                        SELECTED_TITLE,
                        &Dumps::default().indent(2).dump(&json!({
                            "selected": row,
                            "scope": result.scope,
                            "mutation": "none",
                        })),
                    ),
                    Err(e) => Modal::failure(&format!("Asset search unavailable: {e}")),
                };
                self.push(modal, Cont::None);
            }
            (Step::Remove { item, kind }, ModalResult::Choice(value)) => {
                self.prepare_inventory_association(item, kind, value, true);
            }
            (
                Step::Confirm {
                    module,
                    kind,
                    ident,
                    remove,
                    plan,
                },
                ModalResult::Bool(true),
            ) => {
                self.send_inventory_association(module, kind, ident, remove, plan);
            }
            _ => {}
        }
    }

    /// Port of fsv.tui.app.FsvApp._asset_search_prompt.
    fn asset_search_prompt(&mut self, item: Item, kind: String, add: bool) {
        if add && self.read_only {
            return;
        }
        let title = search_title(&kind, add);
        self.push(
            Modal::value(&title, ""),
            Cont::Assets(Step::Search { item, kind, add }),
        );
    }

    /// Port of fsv.tui.app.FsvApp._load_asset_inventory.
    fn load_asset_inventory(&mut self, item: Item, query: String, kind: String, add: bool) {
        if add && self.read_only {
            return;
        }
        self.spawn_group("asset-inventory", true, move |b| {
            Msg::Ui(vec![match b.inventory_search(&query, &kind) {
                Err(e) => unavailable(format!("Asset search unavailable: {e}")),
                Ok(result) if result.assets.is_empty() => Ui::Outcome {
                    title: EMPTY_TITLE.into(),
                    message: EMPTY_MESSAGE.into(),
                },
                Ok(result) => Ui::Push(
                    Box::new(Modal::choices(&picker_title(add), asset_choices(&result))),
                    Cont::Assets(Step::Pick {
                        item,
                        kind,
                        add,
                        result,
                    }),
                ),
            }])
        });
    }

    /// Port of fsv.tui.app.FsvApp._load_associated_inventory.
    fn load_associated_inventory(&mut self, item: Item, kind: String) {
        if self.read_only {
            return;
        }
        let module = item.res.name.to_string();
        let target = item.ref_id();
        let item_for_pick = item.clone();
        self.spawn_group("asset-inventory", true, move |b| {
            Msg::Ui(vec![
                match b.associated_inventory(&module, &target, &kind) {
                    Err(e) => unavailable(format!("Association inventory unavailable: {e}")),
                    Ok(rows) if rows.is_empty() => Ui::Outcome {
                        title: ASSOCIATED_TITLE.into(),
                        message: ASSOCIATED_EMPTY.into(),
                    },
                    Ok(rows) => {
                        let choices = rows
                            .iter()
                            .map(|row| {
                                let id = row.get("item_id").cloned().unwrap_or(Value::Null);
                                let name = row
                                    .get("name")
                                    .filter(|v| truthy(v))
                                    .or_else(|| row.get("item_name").filter(|v| truthy(v)))
                                    .map(py_str)
                                    .unwrap_or_else(|| kind.clone());
                                (id.clone(), format!("[{}] {name}", py_str(&id)))
                            })
                            .collect();
                        Ui::Push(
                            Box::new(Modal::choices(REMOVE_TITLE, choices)),
                            Cont::Assets(Step::Remove {
                                item: item_for_pick,
                                kind,
                            }),
                        )
                    }
                },
            ])
        });
    }

    /// Port of fsv.tui.app.FsvApp._prepare_inventory_association.
    fn prepare_inventory_association(
        &mut self,
        item: Item,
        kind: String,
        ident: Value,
        remove: bool,
    ) {
        if self.read_only {
            return;
        }
        let module = item.res.name.to_string();
        let target = item.ref_id();
        self.spawn_group("asset-review", true, move |b| {
            Msg::Ui(vec![
                match b.inventory_association_plan(&module, &target, &ident, &kind, remove) {
                    Err(e) => unavailable(format!("Association review blocked: {e}")),
                    Ok(plan) => Ui::Push(
                        Box::new(Modal::confirm(
                            REVIEW_TITLE,
                            vec![Dumps::default().indent(2).dump(&plan.review)],
                        )),
                        Cont::Assets(Step::Confirm {
                            module,
                            kind,
                            ident,
                            remove,
                            plan,
                        }),
                    ),
                },
            ])
        });
    }

    /// Port of fsv.tui.app.FsvApp._send_inventory_association.
    fn send_inventory_association(
        &mut self,
        module: String,
        kind: String,
        ident: Value,
        remove: bool,
        plan: ReviewPlan,
    ) {
        if self.read_only {
            return;
        }
        self.spawn_group("write", true, move |b| {
            Msg::Ui(vec![
                match b.send_inventory_association(&plan, &module, &ident, &kind, remove) {
                    Err(e) => Ui::Failure(actions::failure_detail(&e)),
                    Ok(result) => Ui::Outcome {
                        title: OUTCOME_TITLE.into(),
                        message: Dumps::default().indent(2).dump(&result),
                    },
                },
            ])
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use fsv_core::resource::{CHANGES, TICKETS};
    use serde_json::Map;

    use super::*;
    use crate::backend::{BResult, Backend, BasicPages, Failure, ReviewPlan};

    /// `integrated_assets.search` answers: `rows`, or a failure.
    #[derive(Default)]
    struct Assets {
        calls: Mutex<Vec<String>>,
        rows: Vec<Value>,
        associated: Vec<Value>,
        fail: bool,
    }

    impl Backend for Assets {
        fn inventory_search(&self, query: &str, kind: &str) -> BResult<AssetCandidates> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("search {kind} {query:?}"));
            if self.fail {
                return Err(Failure::value("asset candidate response is unavailable"));
            }
            Ok(AssetCandidates {
                change_id: 11,
                page: 1,
                scope: "single search page; not a complete inventory".into(),
                assets: self
                    .rows
                    .iter()
                    .map(|r| r.as_object().cloned().unwrap_or_else(Map::new))
                    .collect(),
                mutation: "none".into(),
                warning: String::new(),
            })
        }
        fn associated_inventory(
            &self,
            module: &str,
            target: &Value,
            kind: &str,
        ) -> BResult<Vec<Value>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("inventory {module} {} {kind}", py_str(target)));
            Ok(self.associated.clone())
        }
        fn inventory_association_plan(
            &self,
            module: &str,
            target: &Value,
            ident: &Value,
            kind: &str,
            remove: bool,
        ) -> BResult<ReviewPlan> {
            self.calls.lock().unwrap().push(format!(
                "plan {module} {} {kind} {} {remove}",
                py_str(target),
                py_str(ident)
            ));
            Ok(ReviewPlan {
                review: json!({"plan_hash": "abc", "ident": ident}),
                hash: "abc".into(),
                public: false,
                target_id: target.clone(),
                raw: Value::Null,
            })
        }
        fn send_inventory_association(
            &self,
            plan: &ReviewPlan,
            module: &str,
            ident: &Value,
            kind: &str,
            remove: bool,
        ) -> BResult<Value> {
            self.calls.lock().unwrap().push(format!(
                "send {} {module} {kind} {} {remove}",
                plan.hash,
                py_str(ident)
            ));
            Ok(json!({"verified": true}))
        }
        crate::backend_stubs!(
            load_schema,
            current_user,
            load_views,
            ticket_headers,
            view_settings,
            build_query_hash,
            list_page,
            search_items,
            load_detail,
            load_tab,
            remote_user_values,
            lookup_choices,
            update_item,
            add_note,
            ticket_note_plan,
            send_ticket_note,
            change_task_plan,
            send_change_task,
            search_change_tickets,
            download_choices,
            download,
            task_for_edit,
            association_plan,
            send_association,
            approval_request_plan,
            approval_targets,
            approval_action_plan,
            send_approval,
            update_task,
            delete_task,
            ticket_property_fields,
            ticket_property_plan,
            send_ticket_property,
            known_error_plan,
            send_known_error,
            planning_definitions,
            planning_row,
            planning_text_plan,
            send_planning_text,
            ticket_conversation_plan,
            send_ticket_conversation,
            asset_candidates,
            open_form,
            catalog
        );
    }

    fn app(b: Assets) -> (App, Arc<Assets>) {
        open(b, true)
    }

    fn open(b: Assets, read_only: bool) -> (App, Arc<Assets>) {
        let b = Arc::new(b);
        (
            App::new_inline(b.clone(), Arc::new(BasicPages), read_only),
            b,
        )
    }

    fn calls(b: &Assets) -> Vec<String> {
        b.calls.lock().unwrap().clone()
    }

    fn top(a: &App) -> &Modal {
        &a.modals.last().expect("a modal").0
    }

    fn change() -> Item {
        Item::new(&CHANGES, json!({"id": 11, "display_id": 11}))
    }

    /// `_asset_inventory_prompt`: a ticket, problem, or change opens the
    /// menu, and read-only mode opens the device search. Anything else warns.
    #[test]
    fn asset_inventory_prompt_needs_a_record() {
        let (mut a, b) = app(Assets::default());
        a.asset_inventory_prompt();
        assert!(a.modals.is_empty());
        assert_eq!(
            a.toasts.last().map(|t| (t.message.as_str(), t.severity)),
            Some(("Select a ticket, SR, problem or change", Severity::Warning))
        );
        a.selected = Some(Item::new(&TICKETS, json!({"id": 5, "display_id": 5})));
        a.asset_inventory_prompt();
        match a.modals.last() {
            Some((Modal::Value(v), Cont::Assets(Step::Search { kind, add, .. }))) => {
                assert_eq!(v.title, SEARCH_PROMPT);
                assert_eq!(kind, "devices");
                assert!(!add);
            }
            m => panic!("not the read-only search: {m:?}"),
        }
        assert!(calls(&b).is_empty());

        let (mut a, _) = open(Assets::default(), false);
        a.selected = Some(change());
        a.asset_inventory_prompt();
        match a.modals.last() {
            Some((Modal::Picker(p), Cont::Assets(Step::Menu { item }))) => {
                assert_eq!(p.title, MENU_TITLE);
                assert_eq!(p.options.len(), 5);
                assert_eq!(item.ref_id(), json!(11));
            }
            m => panic!("not the menu: {m:?}"),
        }
    }

    /// `_load_asset_inventory`: one page-1 search for the change's display
    /// ID; the choices, an empty-page outcome, or the failure screen.
    #[test]
    fn load_asset_inventory_shows_choices_empty_or_failure() {
        let (mut a, b) = app(Assets {
            rows: vec![
                json!({"id": 9000007001_i64, "display_id": 701, "name": "Globex laptop", "ci_type_name": "Computer"}),
                json!({"id": 9000007002_i64, "display_id": 702, "name": null}),
            ],
            ..Default::default()
        });
        a.load_asset_inventory(change(), "Globex".into(), "devices".into(), false);
        a.settle();
        assert_eq!(calls(&b), [r#"search devices "Globex""#]);
        match a.modals.last() {
            Some((Modal::Picker(p), Cont::Assets(Step::Pick { result, add, .. }))) => {
                assert!(!add);
                assert_eq!(p.title, PICKER_TITLE);
                assert_eq!(
                    p.options.iter().map(|o| o.1.as_str()).collect::<Vec<_>>(),
                    ["[701] Globex laptop — Computer", "[702] - — -"]
                );
                assert_eq!(result.assets.len(), 2);
            }
            m => panic!("not the picker: {m:?}"),
        }

        let (mut a, _) = app(Assets::default());
        a.load_asset_inventory(change(), "none".into(), "devices".into(), false);
        a.settle();
        match top(&a) {
            Modal::Outcome(o) => {
                assert_eq!(
                    (o.title.as_str(), o.message.as_str()),
                    (EMPTY_TITLE, EMPTY_MESSAGE)
                );
            }
            m => panic!("not an outcome: {m:?}"),
        }

        let (mut a, _) = app(Assets {
            fail: true,
            ..Default::default()
        });
        a.load_asset_inventory(change(), "x".into(), "devices".into(), false);
        a.settle();
        match top(&a) {
            Modal::Failure(f) => assert_eq!(
                f.message,
                "Asset search unavailable: asset candidate response is unavailable"
            ),
            m => panic!("not a failure: {m:?}"),
        }
    }

    /// Add reviews one id before send. An empty association list sends nothing.
    #[test]
    fn add_reviews_before_send_and_empty_remove_does_not() {
        let (mut a, b) = open(
            Assets {
                rows: vec![
                    json!({"display_id": 81, "name": "Acme item", "ci_type_name": "Computer"}),
                ],
                ..Default::default()
            },
            false,
        );
        a.selected = Some(Item::new(
            &TICKETS,
            json!({"id": 9000000123_i64, "display_id": 123}),
        ));
        a.asset_inventory_prompt();
        a.resume_assets(
            Step::Menu {
                item: a.selected.clone().unwrap(),
            },
            ModalResult::Choice(json!("add:devices")),
        );
        match a.modals.last() {
            Some((Modal::Value(v), Cont::Assets(Step::Search { add, kind, .. }))) => {
                assert!(add);
                assert_eq!(kind, "devices");
                assert_eq!(
                    v.title,
                    "Search devices — selection prepares review, not a send"
                );
            }
            m => panic!("not the add search: {m:?}"),
        }
        a.load_asset_inventory(
            a.selected.clone().unwrap(),
            "Acme".into(),
            "devices".into(),
            true,
        );
        a.settle();
        let step = match a.modals.last() {
            Some((_, Cont::Assets(step))) => step.clone(),
            m => panic!("not a pick: {m:?}"),
        };
        a.resume_assets(step, ModalResult::Choice(json!(81)));
        a.settle();
        match top(&a) {
            Modal::Confirm(c) => assert_eq!(c.title, REVIEW_TITLE),
            m => panic!("not a review: {m:?}"),
        }
        assert!(
            calls(&b)
                .iter()
                .any(|c| c.starts_with("plan tickets 123 devices 81 false"))
        );
        let confirm = match a.modals.last() {
            Some((_, Cont::Assets(step))) => step.clone(),
            m => panic!("not a confirm step: {m:?}"),
        };
        a.resume_assets(confirm, ModalResult::Bool(true));
        a.settle();
        match top(&a) {
            Modal::Outcome(o) => assert_eq!(o.title, OUTCOME_TITLE),
            m => panic!("not an outcome: {m:?}"),
        }
        assert!(
            calls(&b)
                .iter()
                .any(|c| c.starts_with("send abc tickets devices 81 false"))
        );

        let (mut a, b) = open(Assets::default(), false);
        a.load_associated_inventory(change(), "services".into());
        a.settle();
        match top(&a) {
            Modal::Outcome(o) => assert_eq!(
                (o.title.as_str(), o.message.as_str()),
                (ASSOCIATED_TITLE, ASSOCIATED_EMPTY)
            ),
            m => panic!("not empty: {m:?}"),
        }
        assert_eq!(calls(&b), ["inventory changes 11 services".to_string()]);
    }
}
