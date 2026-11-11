//! The form's [`FormBackend`] over the fsv-flows plan builders and send
//! paths, the session constructors of `formui/session.py`
//! (`change_session`, `ticket_session`, `sr_session`) and the CLI's
//! `_run_form`.
//!
//! The flows' plans hold a `FormState` that is not `Send`, so they cannot
//! ride in [`fsv_formui::Plan::detail`]: [`LiveForm`] rebuilds the plan from
//! the same request and metadata when it saves or sends, and refuses when
//! the rebuilt hash differs from the reviewed one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value as Json, json};

use fsv_api::catalog::{self, ItemMeta, SRPlan};
use fsv_api::formmeta::{DiskStore, MetaStore};
use fsv_api::service::{Api, Journal};
use fsv_core::review::Review;
use fsv_core::{Client, Error, Result};
use fsv_flows::change::{
    self, ChangeWriter, CreateOptions, ExecError, PlanJournal, UpdateOptions, flow_exit,
};
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

/// `render_sr_template` (`cli_sr`) has no Rust port yet.
pub const SR_DRAFT_UNAVAILABLE: &str =
    "saving a service-request draft is not available in this build";

/// What the form sessions talk to. [`FormDeps::live`] is the signed-in
/// profile; tests swap in memory journals and stores.
#[derive(Clone)]
pub struct FormDeps {
    pub api: Arc<dyn Api + Send + Sync>,
    pub form_api: Arc<dyn FormApi>,
    pub meta: Arc<dyn MetaSource + Send + Sync>,
    pub store: Arc<dyn MetaStore + Send + Sync>,
    pub plan_journal: Arc<dyn PlanJournal + Send + Sync>,
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
            plan_journal: Arc::new(change::DiskJournal),
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

fn exec_error(e: ExecError) -> Error {
    match e {
        ExecError::Failed(e) => e,
        ExecError::Partial(p) => Error::exit(p.exit_code(), p.to_string()),
    }
}

fn not_rebuilt() -> Error {
    flow_exit(
        4,
        "the reviewed plan could not be rebuilt identically; review it again",
    )
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

    fn send_change(
        &self,
        req: &PlanRequest<'_>,
        meta: &Arc<FormMeta>,
        reviewed: &fsv_formui::Plan,
        allow: bool,
    ) -> Result<Json> {
        let plan = self.form_plan(req, meta.clone())?;
        if plan.hash != reviewed.hash {
            return Err(not_rebuilt());
        }
        let mut rebuild = |m: Arc<FormMeta>| self.form_plan(req, m);
        let d = &self.deps;
        if req.kind == FormKind::Ticket {
            return ticket::send(
                plan,
                d.api.as_ref(),
                d.meta.as_ref(),
                &mut rebuild,
                d.plan_journal.as_ref(),
                None,
                allow,
            );
        }
        change::check_duplicate(&plan, allow, d.plan_journal.as_ref())?;
        let (new, diff) = change::freshen(plan, d.meta.as_ref(), &mut rebuild)?;
        if let Some(diff) = diff {
            return Err(flow_exit(
                4,
                format!(
                    "form metadata changed since the review: {}",
                    diff.join("; ")
                ),
            ));
        }
        change::execute(
            &new,
            d.writer.as_ref(),
            d.plan_journal.as_ref(),
            "append",
            d.attach_flow.as_deref(),
        )
        .map_err(exec_error)
    }

    fn send_item(
        &self,
        req: &PlanRequest<'_>,
        meta: &ItemMeta,
        reviewed: &fsv_formui::Plan,
        allow: bool,
    ) -> Result<Json> {
        let plan = self.item_plan(req, meta)?;
        if plan.hash != reviewed.hash {
            return Err(not_rebuilt());
        }
        let d = &self.deps;
        catalog::duplicate_check(&plan, allow, d.journal.as_ref())?;
        let (new, diff) = catalog::freshen(plan, d.api.as_ref(), d.store.as_ref(), &mut |m| {
            self.item_plan(req, &m)
        })?;
        if let Some(diff) = diff {
            return Err(flow_exit(
                4,
                format!(
                    "the item form changed since the review: {}",
                    diff.join("; ")
                ),
            ));
        }
        catalog::execute(
            &new,
            d.api.as_ref(),
            d.journal.as_ref(),
            &catalog::upload,
            allow,
        )
    }
}

impl FormBackend for LiveForm {
    fn build_plan(&self, req: &PlanRequest<'_>) -> std::result::Result<fsv_formui::Plan, String> {
        let (errors, hash, review, payload) = match &self.meta {
            Meta::Form(m) => {
                let p = self.form_plan(req, m.clone()).map_err(|e| e.to_string())?;
                (p.errors, p.hash, p.review, p.payload)
            }
            Meta::Item(m) => {
                let p = self.item_plan(req, m).map_err(|e| e.to_string())?;
                (p.errors, p.hash, p.review, p.payload)
            }
        };
        Ok(fsv_formui::Plan {
            errors,
            hash,
            review,
            payload: payload.map(Json::Object),
            detail: None,
        })
    }

    fn save_draft(
        &self,
        req: &PlanRequest<'_>,
        _plan: &fsv_formui::Plan,
        path: &Path,
    ) -> std::io::Result<()> {
        let Meta::Form(m) = &self.meta else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                SR_DRAFT_UNAVAILABLE,
            ));
        };
        let plan = self
            .form_plan(req, m.clone())
            .map_err(std::io::Error::other)?;
        change::save_draft(&plan, &flows_draft(req.draft), &path.to_string_lossy())
            .map_err(std::io::Error::other)?;
        Ok(())
    }

    fn send(
        &self,
        req: &PlanRequest<'_>,
        plan: &fsv_formui::Plan,
        allow_duplicate: bool,
    ) -> Result<Json> {
        match &self.meta {
            Meta::Form(m) => self.send_change(req, m, plan, allow_duplicate),
            Meta::Item(m) => self.send_item(req, m, plan, allow_duplicate),
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
mod tests;
