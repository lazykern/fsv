//! Glue shared by `change_cli` / `ticket_cli`: the requester, the
//! in-terminal form and a plan reviewed either way.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use serde_json::{Value as Json, json};

use fsv_api::service::Api;
use fsv_app::Submitted;
use fsv_core::review::Review;
use fsv_core::term::{self, Console, err};
use fsv_core::{Error, Result};
use fsv_flows::change::{Plan, flow_exit};
use fsv_flows::drafts::Draft;
use fsv_forms::LookupValue;
use fsv_formui::kit::theme::Theme;
use fsv_formui::{FormResult, FormSession};

/// `change_cli._Requester`: --requester > draft `requester:` > config
/// `defaults.requester` > (terminal) a prompt.
pub struct Requester<'a> {
    api: &'a dyn Api,
    flag: Option<String>,
    tty: bool,
    cache: RefCell<HashMap<String, LookupValue>>,
    prompted: RefCell<Option<String>>,
    config_key: &'static str,
    /// `typer.prompt(text, default="", show_default=False)`.
    line: &'a Line<'a>,
}

type Pick<'f> = dyn Fn(&[LookupValue]) -> LookupValue + 'f;
type Line<'f> = dyn Fn(&str) -> Result<String> + 'f;

fn nonempty(s: &Option<String>) -> Option<String> {
    s.clone().filter(|v| !v.is_empty())
}

fn prompt_line(text: &str) -> Result<String> {
    term::prompt(text, Some(""), false)
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
            line: &prompt_line,
        }
    }

    #[cfg(test)]
    fn answering(self, line: &'a Line<'a>) -> Self {
        Requester { line, ..self }
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
        let v = (self.line)("Requester (email)")?;
        *self.prompted.borrow_mut() = Some(v).filter(|s| !s.is_empty());
        Ok(())
    }

    /// `cli_sr._Requester.ask`: an empty answer stops.
    pub fn ask_sr(&self) -> Result<()> {
        let v = (self.line)("Requester (email; approvals route via their manager)")?;
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
            match pick_one(cands, |text, default| {
                term::prompt_int(text, Some(default), true)
            }) {
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

/// Port of fsv.change_cli._pick
///
/// `ask(text, default)` is `typer.prompt(text, type=int, default=default)`.
/// `None`: cancelled (Python's `FlowExit(2, "no requester chosen")`, raised
/// by the caller). Candidates go through Rich markup unescaped, as in Python.
fn pick_one(
    cands: &[LookupValue],
    ask: impl FnOnce(&str, i64) -> Result<i64>,
) -> Result<Option<LookupValue>> {
    let shown = &cands[..cands.len().min(20)];
    err().print("several requesters match; candidates:");
    for (i, c) in shown.iter().enumerate() {
        err().print(&format!("  {}. {}", i + 1, c));
    }
    let n = ask("Pick a number (0 to cancel)", 0)?;
    if n >= 1 && (n as usize) <= shown.len() {
        return Ok(Some(shown[n as usize - 1].clone()));
    }
    Ok(None)
}

/// The reviewed plan on submit, else the form's exit (`fsv_app::form::finish`);
/// the theme is config `forms.theme` (`FormApp.__init__`).
/// Port of fsv.change_cli._run_form.
pub fn run_form(session: FormSession) -> Result<Submitted> {
    let cfg = fsv_core::config::load_config();
    let theme = cfg
        .get("forms")
        .and_then(|f| f.get("theme"))
        .filter(|v| fsv_py::truthy(v))
        .map(|v| Theme::by_name(&fsv_py::py_str(v)))
        .unwrap_or_default();
    fsv_app::form::finish(open_form(session, theme)?)
}

#[cfg(not(test))]
fn open_form(session: FormSession, theme: Theme) -> std::io::Result<FormResult> {
    fsv_formui::run_form(session, theme)
}

#[cfg(test)]
type Opener = Box<dyn FnMut(FormSession, Theme) -> std::io::Result<FormResult>>;

#[cfg(test)]
thread_local! {
    static OPEN_FORM: RefCell<Option<Opener>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn open_form(session: FormSession, theme: Theme) -> std::io::Result<FormResult> {
    OPEN_FORM.with(|o| (o.borrow_mut().as_mut().expect("test form opener"))(session, theme))
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
    /// files, target or risks.
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
                    "risks": [],
                    "requires_confirmation": false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{UNITS, isolated, tenant};
    use fsv_formui::Action;

    fn cands() -> Vec<LookupValue> {
        let mut v: Vec<LookupValue> = (1..=25)
            .map(|i| LookupValue {
                id: json!(i),
                display: format!("Ann {i}"),
                email: Some(format!("ann{i}@acme.com")),
            })
            .collect();
        v[1] = LookupValue {
            id: json!(2),
            display: "[b]Bo[/b]".into(),
            email: None,
        };
        v[2] = LookupValue {
            id: json!(3),
            display: String::new(),
            email: None,
        };
        v
    }

    /// Expected values from Python `change_cli._pick` with `typer.prompt`
    /// patched to answer `n`, on the same 25 candidates.
    #[test]
    fn pick_one_matches_python() {
        let cands = cands();
        for (n, want) in [
            (3, Some(3)),
            (1, Some(1)),
            (20, Some(20)),
            (0, None),
            (21, None),
            (-1, None),
        ] {
            let mut asked = None;
            let (got, text) = fsv_core::term::capture(|| {
                pick_one(&cands, |t, d| {
                    asked = Some((t.to_string(), d));
                    Ok(n)
                })
                .unwrap()
            });
            assert_eq!(asked, Some(("Pick a number (0 to cancel)".to_string(), 0)));
            assert_eq!(got.map(|c| c.id), want.map(|i| json!(i)), "n={n}");
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), 21);
            assert_eq!(
                lines[..4],
                [
                    "several requesters match; candidates:",
                    "  1. Ann 1 <ann1@acme.com>",
                    "  2. Bo",
                    "  3. 3",
                ]
            );
            assert_eq!(lines[20], "  20. Ann 20 <ann20@acme.com>");
        }
    }

    fn opt(v: &Json) -> Option<String> {
        v.as_str().map(str::to_string)
    }

    fn lookup_json(v: &Option<LookupValue>) -> Json {
        v.as_ref().map_or(
            Json::Null,
            |c| json!({"id": c.id, "display": c.display, "email": c.email, "str": c.to_string()}),
        )
    }

    fn exit_json(e: &Error) -> Json {
        match e {
            Error::Exit { code, message } => json!({"exit": code, "message": message}),
            other => json!({"error": other.to_string()}),
        }
    }

    /// `units.json` `requester.value`: `change_cli._Requester.value` and
    /// `cli_sr._Requester.value` over flag, draft, prompt and config.
    #[test]
    fn requester_value_matches_python() {
        let env = isolated();
        let t = tenant("units");
        for case in UNITS["requester"]["value"].as_array().unwrap() {
            let sr = case["sr"].as_bool().unwrap();
            let key = if sr { "sr_requester" } else { "requester" };
            let cfg = &case["config"];
            env.config(if cfg.is_null() {
                json!({})
            } else {
                json!({"defaults": {key: cfg}})
            });
            let flag = opt(&case["flag"]);
            let r = if sr {
                Requester::sr(&*t.client, flag, false)
            } else {
                Requester::new(&*t.client, flag, false)
            };
            *r.prompted.borrow_mut() = opt(&case["prompted"]);
            let draft = Draft {
                requester: opt(&case["draft"]),
                ..Draft::default()
            };
            assert_eq!(r.value(&draft), opt(&case["want"]), "{case}");
        }
        assert!(t.requests().is_empty());
    }

    /// `units.json` `requester.ask`: `change_cli._Requester.ask` keeps the
    /// answer as typed; an empty one is no answer.
    #[test]
    fn requester_ask_matches_python() {
        let t = tenant("units");
        for case in UNITS["requester"]["ask"].as_array().unwrap() {
            let answer = case["answer"].as_str().unwrap();
            let asked = RefCell::new(Vec::new());
            let line = |text: &str| -> Result<String> {
                asked.borrow_mut().push(text.to_string());
                Ok(answer.to_string())
            };
            let r = Requester::new(&*t.client, None, false).answering(&line);
            r.ask().unwrap();
            assert_eq!(*r.prompted.borrow(), opt(&case["prompted"]), "{case}");
            drop(r);
            assert_eq!(asked.into_inner(), [case["asked"].as_str().unwrap()]);
        }
    }

    /// `units.json` `requester.resolve`: `_Requester.resolve` twice per flag
    /// against the requester autocomplete; only a match is cached.
    #[test]
    fn requester_resolve_matches_python() {
        let _env = isolated();
        for case in UNITS["requester"]["resolve"].as_array().unwrap() {
            let t = tenant("units");
            let r = Requester::new(&*t.client, opt(&case["flag"]), false);
            for key in ["want", "again"] {
                let (hit, problem) = r.resolve(&Draft::default()).unwrap();
                assert_eq!(
                    json!([lookup_json(&hit), problem]),
                    case[key],
                    "{key} {case}"
                );
            }
            assert_eq!(json!(t.requests()), case["requests"], "{case}");
        }
    }

    /// `units.json` `run_form`: `change_cli._run_form` over each form outcome.
    #[test]
    fn run_form_matches_python() {
        let env = isolated();
        env.config(json!({"forms": {"theme": "textual-light"}}));
        let t = tenant("form");
        let deps = fsv_app::FormDeps::live(t.client.clone()).unwrap();
        let session = || {
            fsv_app::change_session(&deps, fsv_app::ChangeForm::new(fsv_formui::FormMode::New))
                .unwrap()
        };
        for case in UNITS["run_form"].as_array().unwrap() {
            let action = match case["action"].as_str().unwrap() {
                "save_draft" => Action::SaveDraft,
                "cancel" => Action::Cancel,
                _ => Action::Submit,
            };
            let mut s = session();
            let plan = (!case["plan"].is_null()).then(|| s.build_plan(None).unwrap());
            let hash = plan.as_ref().map(|p| p.hash.clone());
            let saved_path = opt(&case["saved_path"]).map(Into::into);
            let mut result = Some(FormResult {
                action,
                plan,
                saved_path,
                session: s,
            });
            OPEN_FORM.with(|o| {
                *o.borrow_mut() = Some(Box::new(move |_, theme| {
                    assert_eq!(theme, Theme::by_name("textual-light"));
                    Ok(result.take().unwrap())
                }))
            });
            let (got, text) = fsv_core::term::capture(|| run_form(session()));
            let want = &case["want"];
            match got {
                Ok(sent) => {
                    assert_eq!(want["ok"], "PLAN");
                    assert_eq!(Some(sent.plan.hash), hash);
                }
                Err(e) => {
                    let mut w = want.clone();
                    let w = w.as_object_mut().unwrap();
                    w.remove("out");
                    w.remove("err");
                    assert_eq!(exit_json(&e), json!(w), "{case}");
                }
            }
            assert_eq!(
                (want["out"].as_str(), want["err"].as_str()),
                (Some(""), Some(""))
            );
            assert_eq!(text, "");
        }
        OPEN_FORM.with(|o| *o.borrow_mut() = None);
    }
}
