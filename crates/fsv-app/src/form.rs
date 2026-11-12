//! The form's [`FormBackend`] over the fsv-flows plan builders and send
//! paths, the session constructors of `formui/session.py`
//! (`change_session`, `ticket_session`, `sr_session`) and the CLI's
//! `_run_form`.
//!
//! The builder's own plan (`change::Plan`, `catalog::SRPlan`) rides in
//! [`fsv_formui::Plan::detail`], so save and send use the reviewed plan, as
//! Python's session keeps the plan object.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value as Json, json};

use fsv_api::catalog::{self, ItemMeta, SRPlan};
use fsv_api::formmeta::{DiskStore, MetaStore};
use fsv_api::service::{Api, Journal};
use fsv_core::review::Review;
use fsv_core::{Client, Error, Result};
use fsv_flows::change::{self, ChangeWriter, CreateOptions, ExecError, UpdateOptions, flow_exit};
use fsv_flows::drafts::{Draft, PlanningSpec};
use fsv_flows::meta::{
    ClientResolver, FormMeta, MetaSource, Resolver, existing_from_record, name_existing,
};
use fsv_flows::ticket::{self, TicketOptions};
use fsv_forms::{Field, FormError, LookupResolver, LookupValue, Value, Values};
use fsv_formui::files::{self, Paths};
use fsv_formui::kit::theme::Theme;
use fsv_formui::session::{TICKET_TITLE, change_title, default_requester, sr_title};
use fsv_formui::{
    Action, ApiFailure, FormApi, FormBackend, FormKind, FormMode, FormResult, FormSession,
    PlanRequest, SessionInit,
};

use crate::meta::LiveMeta;
use crate::writer::{LiveWriter, change_id};

/// `status_label` of `change_session` (status id -> shown name).
pub type StatusLabel = Arc<dyn Fn(&Json) -> String + Send + Sync>;

/// What the form sessions talk to. [`FormDeps::live`] is the signed-in
/// profile; tests swap in memory journals and stores.
#[derive(Clone)]
pub struct FormDeps {
    pub api: Arc<dyn Api + Send + Sync>,
    pub form_api: Arc<dyn FormApi>,
    pub meta: Arc<dyn MetaSource + Send + Sync>,
    pub store: Arc<dyn MetaStore + Send + Sync>,
    pub plan_journal: Arc<dyn fsv_api::service::Journal + Send + Sync>,
    pub journal: Arc<dyn Journal + Send + Sync>,
    pub writer: Arc<dyn ChangeWriter + Send + Sync>,
    pub resolver: Arc<ClientResolver>,
    /// `config.load_config()` (defaults.requester, forms.max_file_mb).
    pub config: Map<String, Json>,
    pub paths: Paths,
    /// `--attach-flow`; `None`: config `changes.attach_flow`.
    pub attach_flow: Option<String>,
}

impl FormDeps {
    /// The current profile: disk cache, disk journals, `$HOME` and the
    /// working directory.
    pub fn live(client: Arc<Client>) -> Result<FormDeps> {
        let api: Arc<dyn Api + Send + Sync> = client.clone();
        let store: Arc<dyn MetaStore + Send + Sync> = Arc::new(DiskStore::current());
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Ok(FormDeps {
            form_api: client,
            meta: Arc::new(LiveMeta::new(api.clone(), store.clone())),
            store,
            plan_journal: Arc::new(fsv_api::service::DiskJournal),
            journal: Arc::new(fsv_api::service::DiskJournal),
            writer: Arc::new(LiveWriter::new(api.clone())),
            resolver: Arc::new(ClientResolver::new(api.clone())),
            config: fsv_core::config::load_config(),
            paths: Paths::new(home, std::env::current_dir()?),
            attach_flow: None,
            api,
        })
    }
}

/// The metadata a session was opened with.
#[derive(Clone)]
pub enum Meta {
    /// A change or incident form.
    Form(Arc<FormMeta>),
    /// A service-catalog item.
    Item(Arc<ItemMeta>),
}

/// [`FormBackend`] over fsv-flows (changes, incidents) and fsv-api's
/// catalog (service requests).
#[derive(Clone)]
pub struct LiveForm {
    pub deps: FormDeps,
    pub meta: Meta,
    pub status_label: Option<StatusLabel>,
}

/// The form's resolver (`fsv_formui::Resolver`) over the flows'
/// [`ClientResolver`], so the form and the plan builders share its caches.
pub struct FormResolver(pub Arc<ClientResolver>);

impl fsv_formui::Resolver for FormResolver {
    fn lookup(
        &self,
        entity: &str,
        query: &str,
        field: &Field,
    ) -> std::result::Result<Vec<LookupValue>, ApiFailure> {
        LookupResolver::lookup(self.0.as_ref(), entity, query, field).map_err(|e| {
            match e.downcast::<Error>() {
                Ok(e) => ApiFailure::from(*e),
                Err(e) => ApiFailure::new(None, e.to_string()),
            }
        })
    }
}

/// A form answer as the flows' drafts hold it (`Value::from_json` reads it
/// back): a lookup becomes its id (else its email or display text), dates
/// their ISO text.
pub fn answer_json(v: &Value) -> Json {
    match v {
        Value::Lookup(l) if !l.id.is_null() => l.id.clone(),
        Value::Lookup(l) => json!(
            l.email
                .clone()
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| l.display.clone())
        ),
        Value::List(items) => Json::Array(items.iter().map(answer_json).collect()),
        other => other.to_json().unwrap_or(Json::Null),
    }
}

fn answers(d: &fsv_formui::Draft) -> Map<String, Json> {
    d.answers
        .iter()
        .map(|(k, v)| (k.clone(), answer_json(v)))
        .collect()
}

/// The form's draft as a change/incident draft.
pub fn flows_draft(d: &fsv_formui::Draft) -> Draft {
    Draft {
        answers: answers(d),
        requester: d.requester.clone(),
        description_html: d.description_html.clone(),
        attachments: d.attachments.clone(),
        planning: d
            .planning
            .iter()
            .map(|(k, p)| {
                (
                    k.clone(),
                    PlanningSpec {
                        files: p.files.clone(),
                        description_html: p.description_html.clone(),
                    },
                )
            })
            .collect(),
        ack: d.ack.clone(),
        ..Draft::default()
    }
}

/// The form's draft as a service-request draft.
pub fn sr_draft(d: &fsv_formui::Draft) -> catalog::Draft {
    catalog::Draft {
        answers: answers(d),
        description_html: d.description_html.clone(),
        attachments: d.attachments.clone(),
        planning: d
            .planning
            .iter()
            .map(|(k, p)| {
                (
                    k.clone(),
                    json!({"files": p.files, "description_html": p.description_html}),
                )
            })
            .collect(),
        ack: d.ack.clone(),
    }
}

/// `FormSession.build_plan`'s pending-input block on a builder's plan.
fn block_on(errors: &mut Vec<String>, review: &mut Review, hash: &mut String, pending: &[String]) {
    let new: Vec<String> = pending
        .iter()
        .filter(|e| !errors.contains(e))
        .cloned()
        .collect();
    if new.is_empty() {
        return;
    }
    errors.extend(new.iter().cloned());
    for e in new {
        if !review.errors.contains(&e) {
            review.errors.push(e);
        }
    }
    hash.clear();
    review.plan_hash.clear();
}

/// A form send that failed.
#[derive(Debug)]
pub enum SendError {
    Failed(Error),
    /// The change was written, then a later step failed (`PartialFailure`).
    Partial(Box<change::PartialFailure>),
    /// A service request's upload or place-request failed (`SendFailure`).
    Sr(Box<catalog::SendFailure>),
}

impl From<catalog::Executed> for SendError {
    fn from(e: catalog::Executed) -> Self {
        match e {
            catalog::Executed::Refused(e) => SendError::Failed(e),
            catalog::Executed::Failed(f) => SendError::Sr(Box::new(f)),
        }
    }
}

impl From<Error> for SendError {
    fn from(e: Error) -> Self {
        SendError::Failed(e)
    }
}

impl From<ExecError> for SendError {
    fn from(e: ExecError) -> Self {
        match e {
            ExecError::Failed(e) => SendError::Failed(e),
            ExecError::Partial(p) => SendError::Partial(p),
        }
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Failed(e) => e.fmt(f),
            SendError::Partial(p) => p.fmt(f),
            SendError::Sr(e) => e.fmt(f),
        }
    }
}

impl SendError {
    pub fn exit_code(&self) -> i32 {
        match self {
            SendError::Failed(e) => e.exit_code(),
            SendError::Partial(p) => p.exit_code(),
            SendError::Sr(e) => e.cause.exit_code(),
        }
    }

    /// The exception `FormSession.send` raises: a partial failure keeps its
    /// message and exit code.
    pub fn into_error(self) -> Error {
        match self {
            SendError::Failed(e) => e,
            SendError::Partial(p) => Error::exit(p.exit_code(), p.to_string()),
            SendError::Sr(e) => Error::exit(e.cause.exit_code(), e.to_string()),
        }
    }

    /// The attributes `tui.actions.failure_detail` lists (`created_id`,
    /// `steps`, `uploaded`, `error`); empty for a plain failure.
    pub fn outcome(&self) -> Vec<(String, String)> {
        match self {
            SendError::Failed(_) => Vec::new(),
            SendError::Sr(e) => vec![
                (
                    "uploaded".into(),
                    fsv_py::py_str(&Json::Array(e.uploaded.clone())),
                ),
                ("error".into(), e.cause.to_string()),
            ],
            SendError::Partial(p) => vec![
                ("created_id".into(), fsv_py::py_str(&p.created_id)),
                (
                    "steps".into(),
                    fsv_py::py_str(&Json::Array(p.steps.clone())),
                ),
                (
                    "uploaded".into(),
                    fsv_py::py_str(&Json::Array(p.uploaded.clone())),
                ),
                ("error".into(), p.error.to_string()),
            ],
        }
    }

    /// `change_cli._report_partial(e, what)`: a partial failure prints its
    /// lines to stderr and exits 5. `cli_sr._send`'s `except SendFailure`:
    /// the uploaded ids go to stderr, then `{step} failed: ...` exits with
    /// the cause's code. Any other failure is returned as is.
    pub fn report(self, what: &str) -> Error {
        match self {
            SendError::Failed(e) => e,
            SendError::Partial(p) => report_partial(&p, what),
            SendError::Sr(e) => {
                if let Some(note) = e.uploaded_note() {
                    fsv_core::term::err().print(&note);
                }
                e.into_error()
            }
        }
    }
}

/// The stderr lines (markup) of `change_cli._report_partial`.
pub fn partial_lines(e: &change::PartialFailure, what: &str) -> Vec<String> {
    let id = fsv_py::py_str(&e.created_id);
    let mut lines = vec![format!(
        "[red]error[/red]: {what} {id} was written, but a later step failed: {}",
        e.error
    )];
    if !e.uploaded.is_empty() {
        let ids: Vec<String> = e
            .uploaded
            .iter()
            .map(|u| {
                format!(
                    "{} ({})",
                    fsv_py::py_str(u.get("id").unwrap_or(&Json::Null)),
                    fsv_py::py_str(u.get("name").unwrap_or(&Json::Null))
                )
            })
            .collect();
        lines.push(format!("uploaded attachment ids: {}", ids.join(", ")));
    }
    lines.extend(e.steps.iter().map(|s| format!("  {}", fsv_py::py_repr(s))));
    lines
}

/// `change_cli._report_partial`: print [`partial_lines`] to stderr; the
/// `FlowExit(5, "partial failure; {what} id {id}")` to raise.
pub fn report_partial(e: &change::PartialFailure, what: &str) -> Error {
    let err = fsv_core::term::err();
    for line in partial_lines(e, what) {
        err.print(&line);
    }
    flow_exit(
        5,
        format!(
            "partial failure; {what} id {}",
            fsv_py::py_str(&e.created_id)
        ),
    )
}

/// The form metadata changed since the review and the rebuilt plan differs.
/// Nothing was written.
pub struct Changed {
    pub kind: FormKind,
    /// `review_diff` lines.
    pub diff: Vec<String>,
    /// The rebuilt plan; [`Submitted::send_confirmed`] sends it.
    pub plan: fsv_formui::Plan,
}

impl Changed {
    pub fn ok(&self) -> bool {
        self.plan.errors.is_empty()
    }

    /// `FormSession.send`'s refusal (exit 4 with the diff).
    pub fn refusal(&self) -> Error {
        let what = match self.kind {
            FormKind::Sr => "the item form changed since the review",
            _ => "form metadata changed since the review",
        };
        flow_exit(4, format!("{what}: {}", self.diff.join("; ")))
    }

    /// The stderr lines (markup) the CLI prints before showing the new plan.
    pub fn lines(&self) -> Vec<String> {
        let (head, esc) = match self.kind {
            FormKind::Sr => (
                "the catalog item or its rules changed since the review; the plan is now different:",
                true,
            ),
            _ => (
                "form metadata changed since the review; the plan is now different:",
                false,
            ),
        };
        let mut lines = vec![format!("[yellow]{head}[/yellow]")];
        lines.extend(self.diff.iter().map(|l| {
            if esc {
                format!("  {}", fsv_core::term::escape(l))
            } else {
                format!("  {l}")
            }
        }));
        lines
    }

    /// The confirm question for the updated plan.
    pub fn question(&self) -> &'static str {
        match self.kind {
            FormKind::Sr => "Place the updated request?",
            _ => "Send the updated plan?",
        }
    }

    /// The exit when the updated plan is refused at the prompt.
    pub fn cancelled(&self) -> Error {
        match self.kind {
            FormKind::Sr => flow_exit(1, "cancelled; nothing was uploaded or sent"),
            _ => flow_exit(1, "cancelled; nothing was sent"),
        }
    }
}

/// What [`Submitted::send_checked`] ended with.
pub enum SendOutcome {
    /// The writes ran; the send result.
    Sent(Json),
    /// Nothing was sent: show [`Changed::lines`] and the new review, then
    /// [`Submitted::send_confirmed`] or [`Changed::refusal`].
    Changed(Box<Changed>),
}

/// The CLI options of `change_cli._send` / `cli_sr._send` /
/// `ticket_flow.send` for a plan reviewed in the form.
#[derive(Clone, Copy, Debug)]
pub struct SendOptions<'a> {
    /// `flow.interactive() and not no_input`.
    pub tty: bool,
    pub yes: bool,
    pub expect: Option<&'a str>,
    pub allow_duplicate: bool,
    /// `--ack`.
    pub ack: bool,
    /// `changes update --duplicate`.
    pub duplicate: &'a str,
}

impl Default for SendOptions<'_> {
    fn default() -> Self {
        SendOptions {
            tty: false,
            yes: false,
            expect: None,
            allow_duplicate: false,
            ack: false,
            duplicate: "append",
        }
    }
}

struct SendCtl<'a> {
    allow: bool,
    expect: Option<&'a str>,
    duplicate: &'a str,
    /// `cli_sr._send`'s recipient-identity re-check before the freshen.
    identity_check: bool,
}

const SESSION_CTL: SendCtl<'static> = SendCtl {
    allow: false,
    expect: None,
    duplicate: "append",
    identity_check: false,
};

/// The builder's plan and the form that built it, in
/// [`fsv_formui::Plan::detail`].
struct Carried<P> {
    plan: P,
    form: LiveForm,
}

/// The builder's plan a reviewed [`fsv_formui::Plan`] carries.
fn carried<T: Any + Send + Sync>(plan: &fsv_formui::Plan) -> Option<&T> {
    plan.detail
        .as_deref()
        .and_then(|d| d.downcast_ref::<Carried<T>>())
        .map(|c| &c.plan)
}

/// The form that built a reviewed plan.
fn carrier(plan: &fsv_formui::Plan) -> Option<&LiveForm> {
    let d = plan.detail.as_deref()?;
    d.downcast_ref::<Carried<change::Plan>>()
        .map(|c| &c.form)
        .or_else(|| d.downcast_ref::<Carried<SRPlan>>().map(|c| &c.form))
}

fn not_reviewed() -> Error {
    Error::value("the plan was not built by this form; review it again")
}

/// `flow.expect` on a service-request plan.
fn expect_sr(plan: &SRPlan, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected.filter(|e| !e.is_empty()) else {
        return Ok(());
    };
    if plan.hash.is_empty() {
        return Err(flow_exit(
            2,
            format!(
                "--expect-plan given but the plan is incomplete: {}",
                plan.errors.join("; ")
            ),
        ));
    }
    if !fsv_forms::plan::plan_matches(expected, &plan.hash) {
        return Err(flow_exit(
            4,
            format!(
                "plan changed: expected {expected}, computed {}. Review again with --dry-run.",
                plan.hash
            ),
        ));
    }
    Ok(())
}

impl LiveForm {
    pub fn new(deps: FormDeps, meta: Meta) -> LiveForm {
        LiveForm {
            deps,
            meta,
            status_label: None,
        }
    }

    fn resolver(&self) -> Arc<dyn Resolver> {
        self.deps.resolver.clone()
    }

    /// `FormSession.build_plan(ack, meta)` for a change or incident.
    pub fn form_plan(&self, req: &PlanRequest<'_>, meta: Arc<FormMeta>) -> Result<change::Plan> {
        let draft = flows_draft(req.draft);
        let mut plan = match (req.kind, req.mode) {
            (FormKind::Change, FormMode::New) => {
                let opts = CreateOptions {
                    resolver: Some(self.resolver()),
                    requester: req.requester.cloned(),
                    ack_all: req.ack_all,
                    ..CreateOptions::default()
                };
                change::build_create_plan(&meta, &draft, &opts)?
            }
            (FormKind::Change, FormMode::Edit) => {
                let target = change_id(req.target.unwrap_or(&Json::Null))?;
                let record = req.record.as_object().cloned().unwrap_or_default();
                let label = self.status_label.clone();
                let label_fn = move |v: &Json| label.as_ref().map(|f| f(v)).unwrap_or_default();
                let opts = UpdateOptions {
                    resolver: Some(self.resolver()),
                    ack_all: req.ack_all,
                    status_label: self
                        .status_label
                        .as_ref()
                        .map(|_| &label_fn as &dyn Fn(&Json) -> String),
                    hints_path: None,
                };
                change::build_update_plan(&meta, &draft, &record, target, &opts)?
            }
            (FormKind::Ticket, _) => {
                let opts = TicketOptions {
                    resolver: Some(self.resolver()),
                    requester: req.requester.cloned(),
                    ack_all: req.ack_all,
                    cc_emails: Vec::new(),
                };
                ticket::build_create_plan(&meta, &draft, &opts)?
            }
            (FormKind::Sr, _) => {
                return Err(Error::value(
                    "a service-request form needs catalog item metadata",
                ));
            }
        };
        block_on(
            &mut plan.errors,
            &mut plan.review,
            &mut plan.hash,
            req.pending_errors,
        );
        Ok(plan)
    }

    /// `catalog.build_sr_plan(meta, draft, resolver=..., requester=..., ack_all=...)`.
    pub fn item_plan(&self, req: &PlanRequest<'_>, meta: &ItemMeta) -> Result<SRPlan> {
        let resolver: Arc<dyn LookupResolver> = self.deps.resolver.clone();
        let domain = self.deps.api.domain();
        let mut plan = catalog::build_sr_plan(
            meta,
            &sr_draft(req.draft),
            Some(resolver),
            req.requester,
            req.ack_all,
            &catalog::Recipients::default(),
            &domain,
        )?;
        block_on(
            &mut plan.errors,
            &mut plan.review,
            &mut plan.hash,
            req.pending_errors,
        );
        Ok(plan)
    }

    fn wrap_change(&self, p: change::Plan) -> fsv_formui::Plan {
        fsv_formui::Plan {
            errors: p.errors.clone(),
            hash: p.hash.clone(),
            review: p.review.clone(),
            payload: p.payload.clone().map(Json::Object),
            detail: Some(Arc::new(Carried {
                plan: p,
                form: self.clone(),
            })),
        }
    }

    fn wrap_item(&self, p: SRPlan) -> fsv_formui::Plan {
        fsv_formui::Plan {
            errors: p.errors.clone(),
            hash: p.hash.clone(),
            review: p.review.clone(),
            payload: p.payload.clone().map(Json::Object),
            detail: Some(Arc::new(Carried {
                plan: p,
                form: self.clone(),
            })),
        }
    }

    /// Duplicate check, freshness re-check, then the writes; a changed plan
    /// comes back unsent.
    fn send_with(
        &self,
        req: &PlanRequest<'_>,
        reviewed: &fsv_formui::Plan,
        ctl: &SendCtl<'_>,
    ) -> std::result::Result<SendOutcome, SendError> {
        match &self.meta {
            Meta::Form(_) => self.send_change(req, reviewed, ctl),
            Meta::Item(_) => self.send_item(req, reviewed, ctl),
        }
    }

    fn send_change(
        &self,
        req: &PlanRequest<'_>,
        reviewed: &fsv_formui::Plan,
        ctl: &SendCtl<'_>,
    ) -> std::result::Result<SendOutcome, SendError> {
        let plan = carried::<change::Plan>(reviewed)
            .cloned()
            .ok_or_else(not_reviewed)?;
        let mut rebuild = |m: Arc<FormMeta>| self.form_plan(req, m);
        let d = &self.deps;
        if req.kind == FormKind::Ticket {
            return Ok(SendOutcome::Sent(ticket::send(
                plan,
                d.api.as_ref(),
                d.meta.as_ref(),
                &mut rebuild,
                d.plan_journal.as_ref(),
                ctl.expect,
                ctl.allow,
            )?));
        }
        change::check_duplicate(&plan, ctl.allow, d.plan_journal.as_ref())?;
        let (new, diff) = change::freshen(plan, d.meta.as_ref(), &mut rebuild)?;
        if let Some(diff) = diff {
            return Ok(SendOutcome::Changed(Box::new(Changed {
                kind: req.kind,
                diff,
                plan: self.wrap_change(new),
            })));
        }
        Ok(SendOutcome::Sent(self.execute_change(&new, ctl)?))
    }

    fn execute_change(
        &self,
        plan: &change::Plan,
        ctl: &SendCtl<'_>,
    ) -> std::result::Result<Json, SendError> {
        let d = &self.deps;
        change::expect(plan, ctl.expect)?;
        Ok(change::execute(
            plan,
            d.writer.as_ref(),
            d.plan_journal.as_ref(),
            ctl.duplicate,
            d.attach_flow.as_deref(),
        )?)
    }

    fn send_item(
        &self,
        req: &PlanRequest<'_>,
        reviewed: &fsv_formui::Plan,
        ctl: &SendCtl<'_>,
    ) -> std::result::Result<SendOutcome, SendError> {
        let plan = carried::<SRPlan>(reviewed)
            .cloned()
            .ok_or_else(not_reviewed)?;
        let d = &self.deps;
        catalog::duplicate_check(&plan, ctl.allow, d.journal.as_ref())?;
        if ctl.identity_check && !plan.identity.is_empty() {
            let checked = self.item_plan(req, &plan.meta)?;
            if !checked.ok() || checked.hash != plan.hash {
                return Err(flow_exit(
                    4,
                    "recipient identities changed since review; review again",
                )
                .into());
            }
        }
        let (new, diff) = catalog::freshen(plan, d.api.as_ref(), d.store.as_ref(), &mut |m| {
            self.item_plan(req, &m)
        })?;
        if let Some(diff) = diff {
            return Ok(SendOutcome::Changed(Box::new(Changed {
                kind: req.kind,
                diff,
                plan: self.wrap_item(new),
            })));
        }
        Ok(SendOutcome::Sent(self.execute_item(&new, ctl)?))
    }

    fn execute_item(
        &self,
        plan: &SRPlan,
        ctl: &SendCtl<'_>,
    ) -> std::result::Result<Json, SendError> {
        let d = &self.deps;
        expect_sr(plan, ctl.expect)?;
        Ok(catalog::execute_detailed(
            plan,
            d.api.as_ref(),
            d.journal.as_ref(),
            &catalog::upload,
            ctl.allow,
        )?)
    }

    /// Send a [`Changed::plan`] the user confirmed: duplicate check, then
    /// the writes, with no second freshness check.
    fn send_changed(
        &self,
        changed: &Changed,
        ctl: &SendCtl<'_>,
    ) -> std::result::Result<Json, SendError> {
        let d = &self.deps;
        if let Some(p) = carried::<change::Plan>(&changed.plan) {
            change::check_duplicate(p, ctl.allow, d.plan_journal.as_ref())?;
            return self.execute_change(p, ctl);
        }
        let p = carried::<SRPlan>(&changed.plan).ok_or_else(not_reviewed)?;
        catalog::duplicate_check(p, ctl.allow, d.journal.as_ref())?;
        self.execute_item(p, ctl)
    }
}

impl FormBackend for LiveForm {
    fn build_plan(&self, req: &PlanRequest<'_>) -> std::result::Result<fsv_formui::Plan, String> {
        match &self.meta {
            Meta::Form(m) => {
                let p = self.form_plan(req, m.clone()).map_err(|e| e.to_string())?;
                Ok(self.wrap_change(p))
            }
            Meta::Item(m) => {
                let p = self.item_plan(req, m).map_err(|e| e.to_string())?;
                Ok(self.wrap_item(p))
            }
        }
    }

    fn save_draft(
        &self,
        req: &PlanRequest<'_>,
        plan: &fsv_formui::Plan,
        path: &Path,
    ) -> std::io::Result<()> {
        let path = path.to_string_lossy();
        match &self.meta {
            Meta::Form(m) => {
                let built;
                let plan = match carried::<change::Plan>(plan) {
                    Some(p) => p,
                    None => {
                        built = self
                            .form_plan(req, m.clone())
                            .map_err(std::io::Error::other)?;
                        &built
                    }
                };
                change::save_draft(plan, &flows_draft(req.draft), &path)
            }
            Meta::Item(m) => {
                let built;
                let plan = match carried::<SRPlan>(plan) {
                    Some(p) => p,
                    None => {
                        built = self.item_plan(req, m).map_err(std::io::Error::other)?;
                        &built
                    }
                };
                let d = fsv_flows::sr::SrDraft {
                    draft: flows_draft(req.draft),
                    requested_for: None,
                };
                fsv_flows::sr::save_sr_draft(plan, &d, &path)
            }
        }
        .map_err(std::io::Error::other)?;
        Ok(())
    }

    fn send(
        &self,
        req: &PlanRequest<'_>,
        plan: &fsv_formui::Plan,
        allow_duplicate: bool,
    ) -> Result<Json> {
        let ctl = SendCtl {
            allow: allow_duplicate,
            ..SESSION_CTL
        };
        match self.send_with(req, plan, &ctl) {
            Ok(SendOutcome::Sent(v)) => Ok(v),
            Ok(SendOutcome::Changed(c)) => Err(c.refusal()),
            Err(e) => Err(e.into_error()),
        }
    }
}

fn form_error(e: FormError) -> Error {
    if e.is_form_error() || e.is_value_error() {
        Error::value(e.to_string())
    } else {
        Error::other(e.to_string())
    }
}

/// `_default_requester(client, key)`.
pub fn config_requester(deps: &FormDeps, key: &str) -> (Option<LookupValue>, String) {
    default_requester(&deps.config, key, |v| {
        fsv_api::formmeta::resolve_requester(deps.api.as_ref(), v, None).map_err(|e| e.to_string())
    })
}

struct Open {
    kind: FormKind,
    mode: FormMode,
    title: String,
    meta: Meta,
    record: Json,
    target: Option<Json>,
    requester: Option<LookupValue>,
    note: String,
    dry_run: bool,
    status_label: Option<StatusLabel>,
}

fn open(deps: &FormDeps, o: Open) -> Result<FormSession> {
    let (spec, rules, warnings, tz_env) = match &o.meta {
        Meta::Form(m) => (
            m.spec(),
            m.engine()?,
            fsv_flows::meta::rules_warnings(&m.rules),
            m.tz_env.clone(),
        ),
        Meta::Item(m) => (
            m.spec(),
            m.engine()?,
            fsv_flows::meta::rules_warnings(&m.rules),
            None,
        ),
    };
    let existing = match (o.mode, o.record.as_object()) {
        (FormMode::Edit, Some(rec)) => name_existing(
            existing_from_record(&spec, rec),
            &spec,
            rec,
            Some(deps.resolver.as_ref() as &dyn Resolver),
        ),
        _ => Values::new(),
    };
    let backend = LiveForm {
        deps: deps.clone(),
        meta: o.meta,
        status_label: o.status_label,
    };
    let init = SessionInit {
        kind: o.kind,
        mode: o.mode,
        title: o.title,
        spec,
        rules: Some(rules),
        rules_warnings: warnings,
        record: o.record,
        target: o.target,
        existing,
        requester: o.requester,
        requester_note: o.note,
        dry_run: o.dry_run,
        max_file_bytes: files::max_file_bytes(&deps.config),
        tz_env,
        paths: deps.paths.clone(),
        api: deps.form_api.clone(),
        resolver: Arc::new(FormResolver(deps.resolver.clone())),
        backend: Arc::new(backend),
    };
    FormSession::new(init).map_err(form_error)
}

/// Options of [`change_session`] (`change_session(client, mode=..., ...)`).
#[derive(Clone)]
pub struct ChangeForm {
    pub mode: FormMode,
    pub target: Option<Json>,
    /// The record being edited; fetched when absent or a list row.
    pub record: Option<Json>,
    pub meta: Option<Arc<FormMeta>>,
    pub requester: Option<LookupValue>,
    pub dry_run: bool,
    pub status_label: Option<StatusLabel>,
    pub title: Option<String>,
}

impl ChangeForm {
    pub fn new(mode: FormMode) -> ChangeForm {
        ChangeForm {
            mode,
            target: None,
            record: None,
            meta: None,
            requester: None,
            dry_run: false,
            status_label: None,
            title: None,
        }
    }
}

/// A new-change form, or an edit form for the record (`target` = id).
pub fn change_session(deps: &FormDeps, o: ChangeForm) -> Result<FormSession> {
    let mut record = o.record.unwrap_or(Json::Null);
    if o.mode == FormMode::Edit && record.get("custom_fields").is_none() {
        let target = fsv_py::py_str(o.target.as_ref().unwrap_or(&Json::Null));
        let data = deps.api.int_get(&format!("changes/{target}"), None)?;
        record = match &data {
            Json::Object(m) => m.get("change").cloned().unwrap_or_else(|| data.clone()),
            _ => json!({}),
        };
    }
    if !record.is_object() {
        record = json!({});
    }
    let meta = match o.meta {
        Some(m) => m,
        None => {
            let mid = match o.mode {
                FormMode::Edit => ["display_id", "id"]
                    .iter()
                    .filter_map(|k| record.get(*k))
                    .find(|v| fsv_py::truthy(v))
                    .cloned()
                    .or_else(|| o.target.clone())
                    .unwrap_or(Json::Null),
                FormMode::New => Json::Null,
            };
            Arc::new(
                deps.meta
                    .load_meta("change", o.mode.as_str(), false, &mid)?,
            )
        }
    };
    let (requester, note) = match o.requester {
        None if o.mode == FormMode::New => config_requester(deps, "requester"),
        r => (r, String::new()),
    };
    let title = o
        .title
        .unwrap_or_else(|| change_title(o.mode, &record, o.target.as_ref()));
    open(
        deps,
        Open {
            kind: FormKind::Change,
            mode: o.mode,
            title,
            meta: Meta::Form(meta),
            record,
            target: o.target,
            requester,
            note,
            dry_run: o.dry_run,
            status_label: o.status_label,
        },
    )
}

/// A service-request form for a catalog item (display id or name).
pub fn sr_session(
    deps: &FormDeps,
    item_ref: &str,
    meta: Option<ItemMeta>,
    requester: Option<LookupValue>,
    dry_run: bool,
) -> Result<FormSession> {
    let meta = match meta {
        Some(m) => m,
        None => catalog::load_item_meta(deps.api.as_ref(), deps.store.as_ref(), item_ref, None)?,
    };
    let (requester, note) = match requester {
        None => config_requester(deps, "sr_requester"),
        r => (r, String::new()),
    };
    let title = sr_title(&meta.spec(), &meta.name());
    open(
        deps,
        Open {
            kind: FormKind::Sr,
            mode: FormMode::New,
            title,
            meta: Meta::Item(Arc::new(meta)),
            record: json!({}),
            target: None,
            requester,
            note,
            dry_run,
            status_label: None,
        },
    )
}

/// The incident form; the requester is never implicitly the signed-in agent.
pub fn ticket_session(
    deps: &FormDeps,
    meta: Option<Arc<FormMeta>>,
    requester: Option<LookupValue>,
    dry_run: bool,
) -> Result<FormSession> {
    let meta = match meta {
        Some(m) => m,
        None => Arc::new(deps.meta.load_meta("ticket", "new", false, &Json::Null)?),
    };
    let (requester, note) = match requester {
        None => config_requester(deps, "requester"),
        r => (r, String::new()),
    };
    open(
        deps,
        Open {
            kind: FormKind::Ticket,
            mode: FormMode::New,
            title: TICKET_TITLE.into(),
            meta: Meta::Form(meta),
            record: json!({}),
            target: None,
            requester,
            note,
            dry_run,
            status_label: None,
        },
    )
}

/// A submitted form: the reviewed plan and the session to send it with
/// (`session.send(&plan, allow_duplicate)`).
pub struct Submitted {
    pub plan: fsv_formui::Plan,
    pub session: FormSession,
}

impl Submitted {
    fn request<'a>(&'a self, ack: bool, pending: &'a [String]) -> PlanRequest<'a> {
        let s = &self.session;
        PlanRequest {
            kind: s.kind,
            mode: s.mode,
            draft: &s.draft,
            record: &s.record,
            target: s.target.as_ref(),
            requester: s.requester.as_ref(),
            ack_all: ack,
            pending_errors: pending,
        }
    }

    fn form(&self) -> Result<&LiveForm> {
        if self.session.dry_run {
            return Err(Error::exit(2, "dry run: nothing is sent"));
        }
        carrier(&self.plan).ok_or_else(not_reviewed)
    }

    /// An edit is never a duplicate of an earlier send.
    fn allow(&self, allow_duplicate: bool) -> bool {
        let s = &self.session;
        allow_duplicate || s.mode == FormMode::Edit && s.kind == FormKind::Change
    }

    /// `session.send(plan)` that hands back a changed plan instead of
    /// refusing it, and a partial failure as data.
    pub fn send_checked(
        &self,
        allow_duplicate: bool,
    ) -> std::result::Result<SendOutcome, SendError> {
        let form = self.form()?;
        let pending = self.session.input_errors();
        let ctl = SendCtl {
            allow: self.allow(allow_duplicate),
            ..SESSION_CTL
        };
        form.send_with(&self.request(self.session.ack, &pending), &self.plan, &ctl)
    }

    /// Send the [`Changed::plan`] the user confirmed.
    pub fn send_confirmed(
        &self,
        changed: &Changed,
        allow_duplicate: bool,
    ) -> std::result::Result<Json, SendError> {
        let ctl = SendCtl {
            allow: self.allow(allow_duplicate),
            ..SESSION_CTL
        };
        self.form()?.send_changed(changed, &ctl)
    }

    /// The CLI's send of a form-reviewed plan: `change_cli._send` (changes;
    /// partial failures print `_report_partial` and exit 5),
    /// `cli_sr._send` (service requests) or `ticket_flow.send` (incidents).
    /// When the metadata changed, prints the diff to stderr; a terminal
    /// without `--yes`/`--expect-plan` shows the new review and asks
    /// `confirm(question)`, anything else exits 4.
    pub fn send_cli(
        &self,
        o: &SendOptions<'_>,
        confirm: &mut dyn FnMut(&str) -> Result<bool>,
    ) -> Result<Json> {
        let form = self.form()?;
        let s = &self.session;
        let ack = match s.kind {
            FormKind::Ticket => {
                o.ack
                    || carried::<change::Plan>(&self.plan)
                        .is_some_and(|p| !p.state.acknowledged.is_empty())
            }
            _ => o.ack || s.ack,
        };
        let pending = s.input_errors();
        let ctl = SendCtl {
            allow: self.allow(o.allow_duplicate),
            expect: o.expect,
            duplicate: o.duplicate,
            identity_check: true,
        };
        let changed = match form.send_with(&self.request(ack, &pending), &self.plan, &ctl) {
            Ok(SendOutcome::Sent(v)) => return Ok(v),
            Ok(SendOutcome::Changed(c)) => c,
            Err(e) => return Err(e.report("change")),
        };
        let err = fsv_core::term::err();
        for line in changed.lines() {
            err.print(&line);
        }
        if !changed.ok() {
            changed.plan.review.render(err);
            return Err(flow_exit(
                2,
                format!(
                    "the updated form needs more answers: {}",
                    changed.plan.errors.join("; ")
                ),
            ));
        }
        if o.expect.is_some_and(|e| !e.is_empty()) || o.yes || !o.tty {
            let h = match changed.plan.hash.as_str() {
                "" => "(incomplete)",
                h => h,
            };
            return Err(flow_exit(
                4,
                format!("plan changed to {h}; review it again with --dry-run"),
            ));
        }
        changed.plan.review.render(err);
        if !confirm(changed.question())? {
            return Err(changed.cancelled());
        }
        form.send_changed(&changed, &ctl)
            .map_err(|e| e.report("change"))
    }
}

/// `_run_form`'s exits: a saved draft stops with exit 0, anything but a
/// submit with exit 1.
pub fn finish(result: FormResult) -> Result<Submitted> {
    match (result.action, result.plan) {
        (Action::SaveDraft, _) => {
            let path = result
                .saved_path
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            Err(flow_exit(
                0,
                format!("saved {path}; continue later with --from {path} (nothing was sent)"),
            ))
        }
        (Action::Submit, Some(plan)) => Ok(Submitted {
            plan,
            session: result.session,
        }),
        _ => Err(flow_exit(1, "cancelled; nothing was sent")),
    }
}

/// Open the form full-screen; the reviewed plan on submit, else the exit
/// `_run_form` raises. Needs a terminal (see `fsv_formui::form_available`).
pub fn run_form(session: FormSession, theme: Theme) -> Result<Submitted> {
    finish(fsv_formui::run_form(session, theme)?)
}

#[cfg(test)]
mod send_tests;
#[cfg(test)]
mod tests;
