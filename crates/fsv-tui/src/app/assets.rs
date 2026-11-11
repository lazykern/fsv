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
