//! The read-only change asset/CI candidate picker (`i`): search one page,
//! pick a candidate by display ID, show it. Nothing is associated, so it
//! also runs in read-only mode.

use fsv_py::{Dumps, py_str, truthy};
use serde_json::{Value, json};

use super::{App, Cont, Msg, Severity, Ui};
use crate::backend::{AssetCandidates, AssetCandidatesRequest, Item};
use crate::modals::{Modal, ModalResult};

pub const SEARCH_PROMPT: &str = "Read-only asset search — selection does not associate";
pub const PICKER_TITLE: &str = "Asset candidates — page 1 only; read-only";
pub const EMPTY_TITLE: &str = "Asset candidates — read-only";
pub const EMPTY_MESSAGE: &str = "No candidates on search page 1. No association was attempted.";
pub const SELECTED_TITLE: &str = "Selected asset — no association";

/// A modal's continuation inside the asset picker.
#[derive(Clone, Debug)]
pub enum Step {
    Search { item: Item },
    Pick { result: AssetCandidates },
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
        let Some(item) = self.selected.clone().filter(|s| s.res.name == "changes") else {
            self.notify(
                "Select a change to browse asset candidates",
                Severity::Warning,
            );
            return;
        };
        self.push(
            Modal::value(SEARCH_PROMPT, ""),
            Cont::Assets(Step::Search { item }),
        );
    }

    pub(super) fn resume_assets(&mut self, step: Step, result: ModalResult) {
        match (step, result) {
            (Step::Search { item }, ModalResult::Text(query)) => {
                self.load_asset_inventory(item, query)
            }
            (Step::Pick { result }, ModalResult::Choice(value)) => {
                let modal = match result.select(&value) {
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
            _ => {}
        }
    }

    /// Port of fsv.tui.app.FsvApp._load_asset_inventory.
    fn load_asset_inventory(&mut self, item: Item, query: String) {
        self.spawn(move |b| {
            let req = AssetCandidatesRequest {
                change: item.ref_id(),
                query,
                page: 1,
            };
            Msg::Ui(vec![match b.asset_candidates(&req) {
                Err(e) => unavailable(format!("Asset search unavailable: {e}")),
                Ok(result) if result.assets.is_empty() => Ui::Outcome {
                    title: EMPTY_TITLE.into(),
                    message: EMPTY_MESSAGE.into(),
                },
                Ok(result) => Ui::Push(
                    Box::new(Modal::choices(PICKER_TITLE, asset_choices(&result))),
                    Cont::Assets(Step::Pick { result }),
                ),
            }])
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use fsv_core::resource::{CHANGES, TICKETS};
    use serde_json::Map;

    use super::*;
    use crate::backend::{BResult, Backend, BasicPages, Failure};

    /// `asset_picker.candidates` answers: `rows`, or a failure.
    #[derive(Default)]
    struct Assets {
        calls: Mutex<Vec<String>>,
        rows: Vec<Value>,
        fail: bool,
    }

    impl Backend for Assets {
        fn asset_candidates(&self, req: &AssetCandidatesRequest) -> BResult<AssetCandidates> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{} {:?} {}", req.change, req.query, req.page));
            if self.fail {
                return Err(Failure::value("asset candidate response is unavailable"));
            }
            Ok(AssetCandidates {
                change_id: 11,
                page: req.page,
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
    }

    fn app(b: Assets) -> (App, Arc<Assets>) {
        let b = Arc::new(b);
        (App::new_inline(b.clone(), Arc::new(BasicPages), true), b)
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

    /// `_asset_inventory_prompt`: a change opens the read-only search
    /// prompt; anything else (or nothing) is a warning toast.
    #[test]
    fn asset_inventory_prompt_needs_a_change() {
        let (mut a, b) = app(Assets::default());
        a.asset_inventory_prompt();
        assert!(a.modals.is_empty());
        assert_eq!(
            a.toasts.last().map(|t| (t.message.as_str(), t.severity)),
            Some(("Select a change to browse asset candidates", Severity::Warning))
        );
        a.selected = Some(Item::new(&TICKETS, json!({"id": 5})));
        a.asset_inventory_prompt();
        assert!(a.modals.is_empty());
        assert_eq!(a.toasts.len(), 2);

        a.selected = Some(change());
        a.asset_inventory_prompt();
        match a.modals.last() {
            Some((Modal::Value(v), Cont::Assets(Step::Search { item }))) => {
                assert_eq!(v.title, SEARCH_PROMPT);
                assert_eq!(item.ref_id(), json!(11));
            }
            m => panic!("not the search prompt: {m:?}"),
        }
        assert!(calls(&b).is_empty());
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
        a.load_asset_inventory(change(), "Globex".into());
        a.settle();
        assert_eq!(calls(&b), [r#"11 "Globex" 1"#]);
        match a.modals.last() {
            Some((Modal::Picker(p), Cont::Assets(Step::Pick { result }))) => {
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
        a.load_asset_inventory(change(), "none".into());
        a.settle();
        match top(&a) {
            Modal::Outcome(o) => {
                assert_eq!((o.title.as_str(), o.message.as_str()), (EMPTY_TITLE, EMPTY_MESSAGE));
            }
            m => panic!("not an outcome: {m:?}"),
        }

        let (mut a, _) = app(Assets {
            fail: true,
            ..Default::default()
        });
        a.load_asset_inventory(change(), "x".into());
        a.settle();
        match top(&a) {
            Modal::Failure(f) => assert_eq!(
                f.message,
                "Asset search unavailable: asset candidate response is unavailable"
            ),
            m => panic!("not a failure: {m:?}"),
        }
    }
}
