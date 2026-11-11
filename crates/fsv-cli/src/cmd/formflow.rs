//! Glue shared by `change_cli` / `ticket_cli`: the requester, the
//! in-terminal form and a plan reviewed either way.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use serde_json::{Value as Json, json};

use fsv_api::service::Api;
use fsv_app::Submitted;
use fsv_core::review::Review;
use fsv_core::term::{self, Console, err, escape};
use fsv_core::{Error, Result};
use fsv_flows::change::{Plan, flow_exit};
use fsv_flows::drafts::Draft;
use fsv_forms::LookupValue;
use fsv_formui::FormSession;
use fsv_formui::kit::theme::Theme;

/// `change_cli._Requester`: --requester > draft `requester:` > config
/// `defaults.requester` > (terminal) a prompt.
pub struct Requester<'a> {
    api: &'a dyn Api,
    flag: Option<String>,
    tty: bool,
    cache: RefCell<HashMap<String, LookupValue>>,
    prompted: RefCell<Option<String>>,
    config_key: &'static str,
}

type Pick<'f> = dyn Fn(&[LookupValue]) -> LookupValue + 'f;

fn nonempty(s: &Option<String>) -> Option<String> {
    s.clone().filter(|v| !v.is_empty())
}

impl<'a> Requester<'a> {
    pub fn new(api: &'a dyn Api, flag: Option<String>, tty: bool) -> Self {
        Requester {
            api,
            flag,
            tty,
            cache: RefCell::new(HashMap::new()),
            prompted: RefCell::new(None),
            config_key: "requester",
        }
    }

    /// `cli_sr._Requester`: the default is config `defaults.sr_requester`.
    pub fn sr(api: &'a dyn Api, flag: Option<String>, tty: bool) -> Self {
        Requester {
            config_key: "sr_requester",
            ..Requester::new(api, flag, tty)
        }
    }

    /// Port of fsv.change_cli._Requester.value, fsv.cli_sr._Requester.value.
    pub fn value(&self, draft: &Draft) -> Option<String> {
        if let Some(v) = nonempty(&self.flag)
            .or_else(|| nonempty(&draft.requester))
            .or_else(|| nonempty(&self.prompted.borrow()))
        {
            return Some(v);
        }
        let cfg = fsv_core::config::load_config();
        cfg.get("defaults")
            .and_then(|d| d.get(self.config_key))
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .filter(|s| !s.is_empty())
    }

    /// Port of fsv.change_cli._Requester.ask.
    pub fn ask(&self) -> Result<()> {
        let v = term::prompt("Requester (email)", Some(""), false)?;
        *self.prompted.borrow_mut() = Some(v).filter(|s| !s.is_empty());
        Ok(())
    }

    /// `cli_sr._Requester.ask`: an empty answer stops.
    pub fn ask_sr(&self) -> Result<()> {
        let v = term::prompt(
            "Requester (email; approvals route via their manager)",
            Some(""),
            false,
        )?;
        let v = fsv_core::pystr::strip(&v).to_string();
        if v.is_empty() {
            return Err(flow_exit(2, "no requester given; nothing was sent"));
        }
        *self.prompted.borrow_mut() = Some(v);
        Ok(())
    }

    /// `(requester, problem)`; a `ValueError` from the lookup is the problem.
    /// Port of fsv.change_cli._Requester.resolve, fsv.cli_sr._Requester.resolve.
    pub fn resolve(&self, draft: &Draft) -> Result<(Option<LookupValue>, Option<String>)> {
        let Some(v) = self.value(draft) else {
            return Ok((None, None));
        };
        if let Some(hit) = self.cache.borrow().get(&v) {
            return Ok((Some(hit.clone()), None));
        }
        let cancelled = Cell::new(false);
        let failed: RefCell<Option<Error>> = RefCell::new(None);
        let pick = |cands: &[LookupValue]| -> LookupValue {
            match pick_one(cands) {
                Ok(Some(c)) => c,
                Ok(None) => {
                    cancelled.set(true);
                    cands[0].clone()
                }
                Err(e) => {
                    *failed.borrow_mut() = Some(e);
                    cands[0].clone()
                }
            }
        };
        let picker: Option<&Pick<'_>> = if self.tty { Some(&pick) } else { None };
        let found = fsv_api::formmeta::resolve_requester(self.api, &v, picker);
        if let Some(e) = failed.into_inner() {
            return Err(e);
        }
        if cancelled.get() {
            return Err(flow_exit(2, "no requester chosen"));
        }
        match found {
            Ok(r) => {
                self.cache.borrow_mut().insert(v, r.clone());
                Ok((Some(r), None))
            }
            Err(Error::Value(m)) => Ok((None, Some(m))),
            Err(e) => Err(e),
        }
    }
}

/// `change_cli._pick`; `None`: cancelled.
fn pick_one(cands: &[LookupValue]) -> Result<Option<LookupValue>> {
    let shown = &cands[..cands.len().min(20)];
    err().print("several requesters match; candidates:");
    for (i, c) in shown.iter().enumerate() {
        err().print(&format!("  {}. {}", i + 1, escape(&c.to_string())));
    }
    let n = term::prompt_int("Pick a number (0 to cancel)", Some(0), true)?;
    if n >= 1 && (n as usize) <= shown.len() {
        return Ok(Some(shown[n as usize - 1].clone()));
    }
    Ok(None)
}

/// `_run_form`: the reviewed plan on submit, else the form's exit.
pub fn run_form(session: FormSession) -> Result<Submitted> {
    let cfg = fsv_core::config::load_config();
    let theme = cfg
        .get("forms")
        .and_then(|f| f.get("theme"))
        .filter(|v| fsv_py::truthy(v))
        .map(|v| Theme::by_name(&fsv_py::py_str(v)))
        .unwrap_or_default();
    fsv_app::run_form(session, theme)
}

/// A plan built by the CLI or reviewed in the form.
pub enum Reviewed {
    Flow(Box<Plan>),
    Form(Box<Submitted>),
}

impl Reviewed {
    pub fn ok(&self) -> bool {
        self.errors().is_empty()
    }

    pub fn errors(&self) -> &[String] {
        match self {
            Reviewed::Flow(p) => &p.errors,
            Reviewed::Form(s) => &s.plan.errors,
        }
    }

    pub fn hash(&self) -> &str {
        match self {
            Reviewed::Flow(p) => &p.hash,
            Reviewed::Form(s) => &s.plan.hash,
        }
    }

    pub fn review(&self) -> &Review {
        match self {
            Reviewed::Flow(p) => &p.review,
            Reviewed::Form(s) => &s.plan.review,
        }
    }

    pub fn payload(&self) -> Option<Json> {
        match self {
            Reviewed::Flow(p) => p.payload.clone().map(Json::Object),
            Reviewed::Form(s) => s.plan.payload.clone(),
        }
    }

    pub fn render(&self, console: &Console) {
        self.review().render(console);
    }

    /// `Plan.to_json()`; a form plan without its built plan carries no
    /// files or target.
    pub fn to_json(&self, action: &str) -> Json {
        match self {
            Reviewed::Flow(p) => p.to_json(),
            Reviewed::Form(s) => {
                if let Some(built) = carried::<Plan>(&s.plan) {
                    return built.to_json();
                }
                let p = &s.plan;
                json!({
                    "plan_hash": if p.hash.is_empty() { Json::Null } else { json!(p.hash) },
                    "action": action,
                    "target": Json::Null,
                    "payload": if p.ok() { p.payload.clone().unwrap_or(Json::Null) } else { Json::Null },
                    "after": {"attachments": [], "planning": []},
                    "review": p.review.to_dict(),
                    "warnings": p.review.warnings,
                    "errors": p.errors,
                })
            }
        }
    }

    /// `flow.expect(plan, expected)`.
    pub fn expect(&self, expected: Option<&str>) -> Result<()> {
        match self {
            Reviewed::Flow(p) => fsv_flows::change::expect(p, expected),
            Reviewed::Form(s) => expect_hash(&s.plan.hash, &s.plan.errors, expected),
        }
    }
}

/// The builder's own plan a form plan carries.
pub fn carried<T: 'static>(plan: &fsv_formui::Plan) -> Option<&T> {
    plan.detail.as_ref().and_then(|d| d.downcast_ref::<T>())
}

pub fn expect_hash(hash: &str, errors: &[String], expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected.filter(|e| !e.is_empty()) else {
        return Ok(());
    };
    if hash.is_empty() {
        return Err(flow_exit(
            2,
            format!(
                "--expect-plan given but the plan is incomplete: {}",
                errors.join("; ")
            ),
        ));
    }
    if !fsv_forms::plan::plan_matches(expected, hash) {
        return Err(flow_exit(
            4,
            format!(
                "plan changed: expected {expected}, computed {hash}. Review again with --dry-run."
            ),
        ));
    }
    Ok(())
}

/// `json.dumps(v, indent=2, default=str, ensure_ascii=False)`.
pub fn pretty(v: &Json) -> String {
    fsv_py::Dumps::default()
        .indent(2)
        .ensure_ascii(false)
        .dump(v)
}

/// A `ValueError` from draft loading is a usage exit.
pub fn draft_error(e: Error) -> Error {
    match e {
        Error::Value(m) => flow_exit(2, m),
        other => other,
    }
}

/// `EditorAbort` leaves with exit 1. Port of fsv.change_cli._abort_editor.
pub fn editor_abort(e: Error) -> Error {
    match e {
        Error::EditorAbort(m) => flow_exit(1, m),
        other => other,
    }
}
