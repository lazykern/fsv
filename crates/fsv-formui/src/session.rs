//! `FormSession`: the form UI's model, independent of the terminal.
//!
//! It wraps one live [`FormState`] (the same engine, rules and replay order
//! the plan builders use), the [`Draft`] being filled in, the lookups and
//! the mode. Every widget change goes through [`FormSession::set`], which
//! returns the rule notes to show ("+ Staging Start (required) appeared
//! because Cloud Only = No"). [`FormSession::build_plan`] hands the draft
//! to the injected [`FormBackend`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use fsv_forms::spec::is_planning_type;
use fsv_forms::tz::TzEnv;
use fsv_forms::{
    Choice, Diff, Field, FormError, FormSpec, FormState, Kind, LookupValue, RulesEngine,
    StateOptions, Value, Values, Zone, fuzzy_filter, when, zone,
};
use serde_json::{Map, Value as Json};

use crate::backend::{Draft, FormBackend, FormKind, FormMode, Plan, PlanRequest, PlanningFiles};
use crate::files::{FileRules, Paths, check_files};
use crate::lookups::{AgentsGroups, EngineResolver, FormApi, LookupError, Resolver, search_field};

pub const REQUESTER: &str = "__requester__";
pub const ATTACHMENTS: &str = "__attachments__";
pub const PLANNING: &str = "planning:";
pub const MAX_EFFECT_CHOICES: usize = 30;
pub const RULES_UNAVAILABLE_KEY: &str = "unavailable";

/// The editor a form row uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WidgetKind {
    Text,
    Number,
    Textarea,
    Richtext,
    Checkbox,
    Choice,
    Multi,
    Lookup,
    Date,
    Datetime,
    File,
    Notice,
}

impl WidgetKind {
    /// `WIDGET_FOR_KIND` (unknown kinds show as notices).
    pub fn for_kind(kind: Kind) -> WidgetKind {
        match kind {
            Kind::Text => WidgetKind::Text,
            Kind::Textarea => WidgetKind::Textarea,
            Kind::Richtext => WidgetKind::Richtext,
            Kind::Number => WidgetKind::Number,
            Kind::Checkbox => WidgetKind::Checkbox,
            Kind::Dropdown | Kind::Department | Kind::Nested => WidgetKind::Choice,
            Kind::MultiSelect => WidgetKind::Multi,
            Kind::LookupUser | Kind::LookupGroup | Kind::LookupAgent | Kind::Lookup => {
                WidgetKind::Lookup
            }
            Kind::Date => WidgetKind::Date,
            Kind::Datetime => WidgetKind::Datetime,
            Kind::Attachment => WidgetKind::File,
            Kind::Notice | Kind::Unsupported => WidgetKind::Notice,
        }
    }
}

fn entity_for(f: &Field) -> String {
    match f.kind {
        Kind::LookupUser => "user".into(),
        Kind::LookupGroup => "group".into(),
        Kind::LookupAgent => "agent".into(),
        _ => f
            .lookup_entity
            .clone()
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "lookup".into()),
    }
}

fn plain(text: &str, limit: usize) -> String {
    let s = fsv_forms::spec::plain_text(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if s.chars().count() <= limit {
        s
    } else {
        s.chars().take(limit - 1).collect::<String>() + "…"
    }
}

pub fn is_status(f: &Field) -> bool {
    f.field_type == "default_status"
}

pub fn is_requester(f: &Field) -> bool {
    f.field_type == "default_requester"
}

pub fn is_description(f: &Field) -> bool {
    f.field_type == "default_description"
}

/// Review warning when the tenant gave no business rules.
pub fn rules_warnings(rules: Option<&Json>) -> Vec<String> {
    let why = rules
        .and_then(|r| r.get(RULES_UNAVAILABLE_KEY))
        .filter(|v| fsv_py::truthy(v));
    match why {
        None => vec![],
        Some(why) => vec![format!(
            "business rules could not be loaded ({}); only the form's static required/visible \
             settings were checked, the server may still ask for more. If the account has \
             workspaces, `fsv config set forms.workspace_id N` picks the right one",
            fsv_py::py_str(why)
        )],
    }
}

/// `(choice, restriction rule)` when a rule limits `name` to exactly one
/// real (non-empty) choice.
pub fn only_option(state: &FormState, name: &str) -> Option<(Choice, String)> {
    let r = state.evaluated.restricted.get(name)?;
    let offered: Vec<Choice> = state
        .choices(name)
        .ok()?
        .into_iter()
        .filter(|c| c.visible && !c.label.is_empty() && !c.value.is_empty())
        .collect();
    let rule = if r.rule_name.is_empty() {
        fsv_py::py_str(&r.rule_id)
    } else {
        r.rule_name.clone()
    };
    (offered.len() == 1).then(|| (offered[0].clone(), rule))
}

/// `config defaults.<key>` resolved to a person: `(requester, note)`. A
/// default never blocks the form.
pub fn default_requester(
    config: &Map<String, Json>,
    key: &str,
    resolve: impl FnOnce(&str) -> Result<LookupValue, String>,
) -> (Option<LookupValue>, String) {
    let v = config
        .get("defaults")
        .and_then(|d| d.get(key))
        .filter(|v| fsv_py::truthy(v))
        .map(|v| fsv_py::py_str(v).trim().to_string())
        .unwrap_or_default();
    if v.is_empty() {
        return (None, String::new());
    }
    match resolve(&v) {
        Ok(p) => (Some(p), format!("Requester {v} from config defaults.{key}")),
        Err(e) => (
            None,
            format!(
                "config defaults.{key} = {} not used: {}",
                fsv_py::py_repr_str(&v),
                e.split_whitespace().collect::<Vec<_>>().join(" ")
            ),
        ),
    }
}

/// `shlex.quote`.
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

/// A throw-away copy to try a value on (`set` replaces `evaluated`).
fn fork(state: &FormState) -> FormState {
    state.clone()
}

/// One row of the form (a spec field or a form-level input).
#[derive(Clone, Debug)]
pub struct FormItem {
    pub key: String,
    pub label: String,
    pub widget: WidgetKind,
    pub field: Option<Field>,
    pub entity: Option<String>,
    pub help: String,
    pub placeholder: String,
    pub allowed_types: Vec<String>,
    pub max_files: Option<i64>,
}

impl FormItem {
    fn synthetic(key: &str, label: &str, widget: WidgetKind, help: &str) -> Self {
        FormItem {
            key: key.into(),
            label: label.into(),
            widget,
            field: None,
            entity: None,
            help: help.into(),
            placeholder: String::new(),
            allowed_types: vec![],
            max_files: None,
        }
    }

    pub fn is_synthetic(&self) -> bool {
        self.field.is_none()
    }
}

/// What a row shows now.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemView {
    pub visible: bool,
    pub required: bool,
    pub disabled: bool,
    pub disabled_reason: String,
    pub value: Value,
    pub text: String,
    pub error: String,
    pub note: String,
}

impl Default for ItemView {
    fn default() -> Self {
        ItemView {
            visible: false,
            required: false,
            disabled: false,
            disabled_reason: String::new(),
            value: Value::Null,
            text: String::new(),
            error: String::new(),
            note: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    pub value: Value,
    pub selected: bool,
}

impl Suggestion {
    pub fn new(label: impl Into<String>, detail: impl Into<String>, value: Value) -> Self {
        Suggestion {
            label: label.into(),
            detail: detail.into(),
            value,
            selected: false,
        }
    }
}

/// Rule notes of a change, or the error that refused it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub notes: Vec<String>,
    pub error: String,
}

impl Outcome {
    fn error(e: impl Into<String>) -> Self {
        Outcome {
            notes: vec![],
            error: e.into(),
        }
    }
}

/// The help panel's content for one row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HelpInfo {
    pub label: String,
    pub required: bool,
    pub help: String,
    pub placeholder: String,
    pub choices: Vec<(String, String)>,
    pub effects: Vec<String>,
    pub why: Vec<String>,
    pub kind: Option<WidgetKind>,
    pub disabled: String,
    pub note: String,
}

/// A remote lookup to run off the UI thread (no engine state inside).
#[derive(Clone)]
pub struct SuggestJob {
    api: Arc<dyn FormApi>,
    resolver: Arc<dyn Resolver>,
    boot: Arc<AgentsGroups>,
    field: Field,
    entity: String,
}

impl SuggestJob {
    /// `(suggestions, note)`; `Err` for a failed call other than "not
    /// offered" (the combo shows the message).
    pub fn run(&self, query: &str) -> Result<(Vec<Suggestion>, String), String> {
        if (self.entity == "user" || self.entity == "lookup") && query.trim().is_empty() {
            return Ok((vec![], "type part of a name or email".into()));
        }
        match search_field(
            self.api.as_ref(),
            self.resolver.as_ref(),
            &self.boot,
            &self.field,
            &self.entity,
            query,
        ) {
            Ok(rows) => {
                let note = if rows.is_empty() { "no match" } else { "" };
                Ok((
                    rows.into_iter()
                        .map(|r| Suggestion::new(r.to_string(), "", Value::Lookup(r)))
                        .collect(),
                    note.into(),
                ))
            }
            Err(LookupError::Unavailable(e)) => {
                let mut out = vec![];
                if self.entity == "user" && query.contains('@') {
                    let q = query.trim();
                    out.push(Suggestion::new(
                        format!("use {q}"),
                        "typed address (search unavailable)",
                        Value::Lookup(LookupValue::new(Json::Null, q, Some(q))),
                    ));
                }
                let note = if self.entity == "user" {
                    format!("{e}; type the exact email")
                } else {
                    e.to_string()
                };
                Ok((out, note))
            }
            Err(LookupError::Api(e)) => Err(e.message),
        }
    }
}

/// How to build a session (the loaders in `fsv-flows` fill this in:
/// `load_meta`, `catalog.load_item_meta`, the edit record, the default
/// requester).
pub struct SessionInit {
    pub kind: FormKind,
    pub mode: FormMode,
    pub title: String,
    pub spec: Arc<FormSpec>,
    /// `meta.engine()`; `None`: no business rules.
    pub rules: Option<RulesEngine>,
    /// `rules_warnings(meta.rules)`.
    pub rules_warnings: Vec<String>,
    /// The record being edited (`{}` on a new form).
    pub record: Json,
    pub target: Option<Json>,
    /// `name_existing(existing_from_record(...))` on an edit form.
    pub existing: Values,
    pub requester: Option<LookupValue>,
    pub requester_note: String,
    pub dry_run: bool,
    /// `forms.max_file_mb` in bytes (see [`crate::files::max_file_bytes`]).
    pub max_file_bytes: Option<i64>,
    /// `None`: the process environment.
    pub tz_env: Option<TzEnv>,
    /// Where `~` and relative paths (files, drafts) point.
    pub paths: Paths,
    pub api: Arc<dyn FormApi>,
    pub resolver: Arc<dyn Resolver>,
    pub backend: Arc<dyn FormBackend>,
}

/// The form model.
pub struct FormSession {
    pub kind: FormKind,
    pub mode: FormMode,
    pub title: String,
    pub record: Json,
    pub target: Option<Json>,
    pub dry_run: bool,
    pub state: FormState,
    pub draft: Draft,
    pub requester: Option<LookupValue>,
    pub touched: std::collections::HashSet<String>,
    pub show_all_errors: bool,
    pub errors_by_key: indexmap::IndexMap<String, String>,
    pub ack: bool,
    pub plan: Option<Plan>,
    pub saved_path: Option<PathBuf>,
    pub tz: Zone,
    pub tz_source: String,
    pub items: Vec<FormItem>,
    pub load_notes: Vec<String>,
    pub paths: Paths,
    existing: Values,
    max_file_bytes: Option<i64>,
    desc_prefill: String,
    desc_md: Option<String>,
    by_key: HashMap<String, usize>,
    effects_cache: RefCell<HashMap<String, Vec<String>>>,
    api: Arc<dyn FormApi>,
    resolver: Arc<dyn Resolver>,
    boot: Arc<AgentsGroups>,
    backend: Arc<dyn FormBackend>,
}

impl FormSession {
    pub fn new(init: SessionInit) -> Result<FormSession, FormError> {
        if init.kind == FormKind::Ticket && init.mode != FormMode::New {
            return Err(FormError::Value(
                "kind must be change|sr|ticket and mode new|edit (ticket: new only)".into(),
            ));
        }
        let existing = if init.mode == FormMode::Edit {
            init.existing
        } else {
            Values::new()
        };
        let opts = StateOptions {
            resolver: Some(Arc::new(EngineResolver(init.resolver.clone()))),
            existing: (!existing.is_empty()).then(|| existing.clone()),
            tz_env: init.tz_env,
            ..StateOptions::default()
        };
        let state = FormState::new(
            init.spec.clone(),
            init.rules,
            init.mode.as_str(),
            None,
            opts,
        )?;
        let boot = Arc::new(AgentsGroups::new(init.api.clone()));
        let mut s = FormSession {
            kind: init.kind,
            mode: init.mode,
            title: init.title,
            record: init.record,
            target: init.target,
            dry_run: init.dry_run,
            tz: state.tz.clone(),
            tz_source: state.tz_source.clone(),
            state,
            draft: Draft::default(),
            requester: init.requester.clone(),
            touched: Default::default(),
            show_all_errors: false,
            errors_by_key: Default::default(),
            ack: false,
            plan: None,
            saved_path: None,
            items: vec![],
            load_notes: vec![],
            paths: init.paths.clone(),
            existing,
            max_file_bytes: init.max_file_bytes,
            desc_prefill: String::new(),
            desc_md: None,
            by_key: HashMap::new(),
            effects_cache: RefCell::new(HashMap::new()),
            api: init.api,
            resolver: init.resolver,
            boot,
            backend: init.backend,
        };
        (s.tz, s.tz_source) = s.timezone();
        if let (Some(req), Some(who)) = (s.requester_field(), &init.requester)
            && s.requester_is_special()
        {
            let name = req.api_name.clone();
            let _ = s.state.set(&name, Value::Lookup(who.clone()));
        }
        if let Some(who) = &init.requester {
            s.draft.requester = Some(who.email.clone().unwrap_or_else(|| who.display.clone()));
        }
        if let Some(desc) = s.description_field()
            && s.mode == FormMode::Edit
        {
            let html = s
                .existing
                .get(&desc.api_name)
                .filter(|v| v.truthy())
                .map(Value::py_str)
                .unwrap_or_default();
            s.desc_prefill = fsv_forms::spec::plain_text(&html);
        }
        s.items = s.build_items();
        s.by_key = s
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| (it.key.clone(), i))
            .collect();
        let pre = s.preselect_all();
        s.load_notes = init.rules_warnings;
        s.load_notes.extend(pre);
        if !init.requester_note.is_empty() {
            s.load_notes.insert(0, init.requester_note);
        }
        Ok(s)
    }

    // -- structure --------------------------------------------------------
    pub fn spec(&self) -> &FormSpec {
        &self.state.spec
    }

    pub fn item(&self, key: &str) -> Option<&FormItem> {
        self.by_key.get(key).map(|&i| &self.items[i])
    }

    fn label_of(&self, name: &str) -> String {
        self.spec()
            .get(name)
            .map_or_else(|| name.to_string(), |f| f.label.clone())
    }

    fn requester_field(&self) -> Option<Field> {
        self.spec()
            .fields()
            .iter()
            .find(|f| is_requester(f))
            .cloned()
    }

    fn description_field(&self) -> Option<Field> {
        if self.kind != FormKind::Change {
            return None;
        }
        self.spec()
            .fields()
            .iter()
            .find(|f| is_description(f))
            .cloned()
    }

    /// The plan builder takes the requester separately (new change, SR).
    fn requester_is_special(&self) -> bool {
        self.kind == FormKind::Sr || self.mode == FormMode::New
    }

    fn timezone(&self) -> (Zone, String) {
        let (tz, source) = (self.state.tz.clone(), self.state.tz_source.clone());
        let has_user = self.state.rules.user().is_some_and(|u| !u.is_empty());
        if source != "system time zone" || has_user {
            return (tz, source);
        }
        let Ok(me) = self.api.check_session() else {
            return (tz, source);
        };
        let name = me.get("time_zone").map(fsv_py::py_str).unwrap_or_default();
        match me.get("time_zone").and_then(Json::as_str).and_then(zone) {
            Some(found) => (found, format!("Freshservice user time zone ({name})")),
            None => (tz, source),
        }
    }

    fn build_items(&self) -> Vec<FormItem> {
        let mut out = vec![];
        let req_field = self.requester_field();
        if self.kind == FormKind::Sr || (self.kind == FormKind::Ticket && req_field.is_none()) {
            let mut it = FormItem::synthetic(
                REQUESTER,
                "Requester",
                WidgetKind::Lookup,
                "Select the requester by name or email.",
            );
            it.entity = Some("user".into());
            out.push(it);
        }
        for f in self.spec().ordered() {
            if matches!(self.kind, FormKind::Change | FormKind::Ticket)
                && self.mode == FormMode::New
                && is_status(f)
            {
                continue; // status on create is the form default (shown on the review)
            }
            let widget = WidgetKind::for_kind(f.kind);
            out.push(FormItem {
                key: f.api_name.clone(),
                label: f.label.clone(),
                widget,
                field: Some(f.clone()),
                entity: (widget == WidgetKind::Lookup).then(|| entity_for(f)),
                help: plain(f.help(), 600),
                placeholder: plain(&f.placeholder, 80),
                allowed_types: f.allowed_file_types.clone(),
                max_files: f.max_files,
            });
        }
        if self.kind == FormKind::Change {
            out.push(FormItem::synthetic(
                ATTACHMENTS,
                "Attachments",
                WidgetKind::File,
                "Files for the change's Attachments section; uploaded only after you confirm. \
                 Separate several with ';'.",
            ));
            // every planning type, keyed by label (names differ per account)
            for f in self.spec().fields() {
                if is_planning_type(&f.field_type) {
                    let help = if f.help().is_empty() {
                        String::new()
                    } else {
                        plain(f.help(), 600) + " "
                    };
                    out.push(FormItem::synthetic(
                        &format!("{PLANNING}{}", f.label),
                        &format!("{} (planning files)", f.label),
                        WidgetKind::File,
                        &(help + "Planning field files, attached after the change is saved."),
                    ));
                }
            }
        } else if self
            .spec()
            .item
            .as_ref()
            .is_some_and(|i| i.allow_attachments)
        {
            out.push(FormItem::synthetic(
                ATTACHMENTS,
                "Attachments",
                WidgetKind::File,
                "General files for the request; uploaded only after you confirm.",
            ));
        }
        out
    }

    // -- reading ----------------------------------------------------------
    pub fn view(&self, key: &str) -> ItemView {
        let Some(it) = self.item(key) else {
            return ItemView::default();
        };
        let ev = &self.state.evaluated;
        let mut v = if key == REQUESTER {
            ItemView {
                visible: true,
                required: true,
                value: self.requester.clone().map_or(Value::Null, Value::Lookup),
                text: self
                    .requester
                    .as_ref()
                    .map(|r| r.to_string())
                    .unwrap_or_default(),
                ..ItemView::default()
            }
        } else if key == ATTACHMENTS {
            let need = match (&self.spec().item, self.kind) {
                (Some(item), FormKind::Sr) => {
                    ev.attachments_required.unwrap_or(item.attachment_mandatory)
                }
                _ => false,
            };
            ItemView {
                visible: true,
                required: need,
                value: Value::list(self.draft.attachments.iter().cloned().map(Value::Str)),
                text: self.draft.attachments.join("; "),
                ..ItemView::default()
            }
        } else if let Some(label) = key.strip_prefix(PLANNING) {
            let files = self
                .draft
                .planning
                .get(label)
                .map(|p| p.files.clone())
                .unwrap_or_default();
            ItemView {
                visible: true,
                value: Value::list(files.iter().cloned().map(Value::Str)),
                text: files.join("; "),
                ..ItemView::default()
            }
        } else {
            let Some(f) = &it.field else {
                return ItemView::default();
            };
            let name = &f.api_name;
            let mut value = ev.values.get(name).cloned().unwrap_or(Value::Null);
            if is_requester(f) && self.requester_is_special() {
                value = self.requester.clone().map_or(Value::Null, Value::Lookup);
            }
            let mut v = ItemView {
                visible: ev.visible.contains(name),
                required: ev.required.contains(name),
                text: self.text_for(it, &value),
                value,
                ..ItemView::default()
            };
            if ev.disabled.contains(name) {
                v.disabled = true;
                v.disabled_reason = ev
                    .decision(name, "disabled")
                    .map_or_else(|| "read-only".into(), |d| d.describe());
            }
            if let Some(r) = ev.restricted.get(name) {
                let offered: Vec<String> = self
                    .state
                    .choices(name)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|c| c.visible && !c.label.is_empty())
                    .map(|c| c.label)
                    .take(8)
                    .collect();
                let rule = if r.rule_name.is_empty() {
                    fsv_py::py_str(&r.rule_id)
                } else {
                    r.rule_name.clone()
                };
                let shown = if offered.is_empty() {
                    "none".to_string()
                } else {
                    offered.join(", ")
                };
                v.note = format!("choices limited by rule {rule}: {shown}");
            }
            if ev.unavailable.contains_key(name) {
                v.error = self
                    .state
                    .unavailable_errors()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|e| e.starts_with(&f.label))
                    .unwrap_or_else(|| "not offered now".into());
            }
            v
        };
        if let Some(e) = self.errors_by_key.get(key) {
            v.error = e.clone();
        } else if v.error.is_empty()
            && v.visible
            && v.required
            && Self::empty(it, &v.value)
            && (self.touched.contains(key) || self.show_all_errors)
        {
            v.error = "required".into();
        }
        v
    }

    fn empty(it: &FormItem, value: &Value) -> bool {
        if it.widget == WidgetKind::Checkbox {
            return !matches!(value, Value::Bool(true) | Value::Json(Json::Bool(true)));
        }
        value.is_empty()
    }

    /// The editor text for a stored value.
    pub fn text_for(&self, it: &FormItem, value: &Value) -> String {
        if value.is_empty() {
            return String::new();
        }
        match (it.widget, &it.field) {
            (WidgetKind::Date | WidgetKind::Datetime, _) => when::to_text(value, Some(&self.tz)),
            (WidgetKind::Choice | WidgetKind::Multi, Some(f)) => self
                .state
                .display(&f.api_name, Some(value))
                .unwrap_or_else(|_| value.py_str()),
            (WidgetKind::File, _) => value
                .as_list()
                .map(|l| l.iter().map(Value::py_str).collect::<Vec<_>>().join("; "))
                .unwrap_or_else(|| value.py_str()),
            (WidgetKind::Richtext, _) if self.kind == FormKind::Change => self.desc_text().into(),
            _ => match value {
                Value::Lookup(l) => l.to_string(),
                Value::List(items) => items
                    .iter()
                    .map(Value::py_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.py_str(),
            },
        }
    }

    fn desc_text(&self) -> &str {
        self.desc_md.as_deref().unwrap_or(&self.desc_prefill)
    }

    pub fn visible_keys(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|it| self.view(&it.key).visible)
            .map(|it| it.key.clone())
            .collect()
    }

    /// Why Submit is not available yet (blocking messages are acknowledged
    /// on the review, so they are not listed).
    pub fn problems(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        let special = self.requester_is_special();
        let req = self.requester_field();
        if special
            && self.requester.is_none()
            && (matches!(self.kind, FormKind::Sr | FormKind::Ticket) || req.is_some())
        {
            out.push("Requester is required".into());
        }
        for e in self.state.errors().unwrap_or_else(|e| vec![e.to_string()]) {
            if e.starts_with("blocked by rule") {
                continue;
            }
            if special
                && let Some(r) = &req
                && e == format!("{} is required", r.label)
            {
                continue;
            }
            out.push(e);
        }
        if self.kind == FormKind::Sr
            && self.by_key.contains_key(ATTACHMENTS)
            && self.view(ATTACHMENTS).required
            && self.draft.attachments.is_empty()
        {
            out.push("Attachments: at least one file is required".into());
        }
        for (k, e) in &self.errors_by_key {
            if self.view(k).visible
                && let Some(it) = self.item(k)
            {
                out.push(format!("{}: {e}", it.label));
            }
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|p| seen.insert(p.clone()));
        out
    }

    pub fn missing_keys(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|it| {
                let v = self.view(&it.key);
                v.visible && v.required && Self::empty(it, &v.value)
            })
            .map(|it| it.key.clone())
            .collect()
    }

    pub fn help_for(&self, key: &str) -> HelpInfo {
        let Some(it) = self.item(key) else {
            return HelpInfo::default();
        };
        let v = self.view(key);
        let mut info = HelpInfo {
            label: it.label.clone(),
            required: v.required,
            help: it.help.clone(),
            placeholder: it.placeholder.clone(),
            kind: Some(it.widget),
            disabled: v.disabled_reason.clone(),
            note: v.note.clone(),
            ..HelpInfo::default()
        };
        if matches!(it.widget, WidgetKind::Choice | WidgetKind::Multi) && it.field.is_some() {
            info.choices = self
                .pool(it)
                .into_iter()
                .map(|c| (c.label, c.description))
                .collect();
        }
        match it.widget {
            WidgetKind::Datetime => info.why.push(format!("time zone: {}", self.tz_source)),
            WidgetKind::Date => info.why.push("date only".into()),
            _ => {}
        }
        if let Some(f) = &it.field {
            for e in self
                .state
                .explain(Some(&[f.api_name.as_str()]))
                .unwrap_or_default()
            {
                if e.decision.is_some() {
                    info.why.push(e.to_string());
                }
            }
        }
        info.effects = self.effects(key);
        info
    }

    fn pool(&self, it: &FormItem) -> Vec<Choice> {
        let Some(f) = &it.field else { return vec![] };
        self.state
            .choices(&f.api_name)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.visible && !c.label.is_empty() && !c.value.is_empty())
            .collect()
    }

    /// A lookup row's search, to run on a worker thread; `None` for rows
    /// that filter locally.
    pub fn suggest_job(&self, key: &str) -> Option<SuggestJob> {
        let it = self.item(key)?;
        if it.widget != WidgetKind::Lookup {
            return None;
        }
        Some(SuggestJob {
            api: self.api.clone(),
            resolver: self.resolver.clone(),
            boot: self.boot.clone(),
            field: it
                .field
                .clone()
                .unwrap_or_else(|| Field::new(key, it.label.clone(), Kind::LookupUser)),
            entity: it.entity.clone().unwrap_or_else(|| "lookup".into()),
        })
    }

    /// `(suggestions, note)`. People lookups call the network (blocking):
    /// the UI runs them through [`FormSession::suggest_job`] instead.
    pub fn suggestions(&self, key: &str, query: &str) -> Result<(Vec<Suggestion>, String), String> {
        let Some(it) = self.item(key) else {
            return Ok((vec![], String::new()));
        };
        match (it.widget, &it.field) {
            (WidgetKind::Choice | WidgetKind::Multi, Some(f)) => {
                let pool = self.pool(it);
                if pool.is_empty() && f.is_nested_level() {
                    let parent = f
                        .parent
                        .as_deref()
                        .and_then(|p| self.spec().get(p))
                        .map_or_else(|| "the level above".to_string(), |p| p.label.clone());
                    return Ok((vec![], format!("choose {parent} first")));
                }
                let current = self.state.evaluated.values.get(&f.api_name);
                let chosen: Vec<Value> = match current {
                    Some(Value::List(items)) => items.clone(),
                    Some(v) => vec![v.clone()],
                    None => vec![Value::Null],
                };
                let items: Vec<(String, String)> = pool
                    .iter()
                    .map(|c| (c.label.clone(), c.description.clone()))
                    .collect();
                let idx = fuzzy_filter(query, &items);
                let note = if idx.is_empty() {
                    "no choice matches"
                } else {
                    ""
                };
                Ok((
                    idx.into_iter()
                        .map(|i| {
                            let c = &pool[i];
                            let value = Value::Str(c.value.clone());
                            Suggestion {
                                label: c.label.clone(),
                                detail: c.description.clone(),
                                selected: chosen.contains(&value),
                                value,
                            }
                        })
                        .collect(),
                    note.into(),
                ))
            }
            (WidgetKind::Lookup, _) => match self.suggest_job(key) {
                Some(job) => job.run(query),
                None => Ok((vec![], String::new())),
            },
            _ => Ok((vec![], String::new())),
        }
    }

    // -- effects ("Picking No will show ...") --------------------------------
    pub fn effects(&self, key: &str) -> Vec<String> {
        let Some(it) = self.item(key) else {
            return vec![];
        };
        let Some(f) = &it.field else { return vec![] };
        if !matches!(it.widget, WidgetKind::Choice | WidgetKind::Checkbox)
            || self.state.evaluated.disabled.contains(&f.api_name)
        {
            return vec![];
        }
        let mut sorted: Vec<(&String, &Value)> = self.state.values.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        let cache_key = format!(
            "{key}\u{0}{}",
            sorted
                .iter()
                .map(|(k, v)| format!("{k}={}", v.py_repr()))
                .collect::<Vec<_>>()
                .join("\u{1}")
        );
        if let Some(hit) = self.effects_cache.borrow().get(&cache_key) {
            return hit.clone();
        }
        let options: Vec<(String, Value)> = if it.widget == WidgetKind::Checkbox {
            vec![
                ("ticked".into(), Value::Bool(true)),
                ("unticked".into(), Value::Bool(false)),
            ]
        } else {
            self.pool(it)
                .into_iter()
                .take(MAX_EFFECT_CHOICES)
                .map(|c| (c.label, Value::Str(c.value)))
                .collect()
        };
        let current = self.state.evaluated.values.get(&f.api_name);
        let mut out = vec![];
        for (label, value) in options {
            if current == Some(&value)
                || (current.is_none() && value.is_null())
                || (it.widget == WidgetKind::Checkbox
                    && Value::Bool(current.is_some_and(Value::truthy)) == value)
            {
                continue;
            }
            let mut s2 = fork(&self.state);
            let Ok(d) = s2.set(&f.api_name, value.clone()) else {
                continue;
            };
            let parts = self.diff_parts(&s2, &d);
            if !parts.is_empty() {
                let verb = match value {
                    Value::Bool(true) => "Ticking".to_string(),
                    Value::Bool(false) => "Unticking".to_string(),
                    _ => format!("Picking {label}"),
                };
                out.push(format!("{verb} will {}", parts.join("; ")));
            }
        }
        self.effects_cache
            .borrow_mut()
            .insert(cache_key, out.clone());
        out
    }

    fn labels_req(&self, names: &[String], st: &FormState) -> String {
        names
            .iter()
            .map(|n| {
                let req = if st.evaluated.required.contains(n) {
                    " (required)"
                } else {
                    ""
                };
                format!("{}{req}", self.label_of(n))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn labels<'a>(&self, names: impl IntoIterator<Item = &'a String>) -> String {
        names
            .into_iter()
            .map(|n| self.label_of(n))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn diff_parts(&self, st: &FormState, d: &Diff) -> Vec<String> {
        let mut parts = vec![];
        if !d.appeared.is_empty() {
            parts.push(format!("show {}", self.labels_req(&d.appeared, st)));
        }
        if !d.disappeared.is_empty() {
            parts.push(format!("hide {}", self.labels(&d.disappeared)));
        }
        let req: Vec<&String> = d
            .became_required
            .iter()
            .filter(|n| !d.appeared.contains(n))
            .collect();
        if !req.is_empty() {
            parts.push(format!("make required: {}", self.labels(req)));
        }
        let opt: Vec<&String> = d
            .no_longer_required
            .iter()
            .filter(|n| st.evaluated.visible.contains(n))
            .collect();
        if !opt.is_empty() {
            parts.push(format!("make optional: {}", self.labels(opt)));
        }
        for (n, labels) in &d.restricted {
            let shown = if labels.is_empty() {
                "nothing".to_string()
            } else {
                labels.join(", ")
            };
            parts.push(format!("limit {} to {shown}", self.label_of(n)));
        }
        if !d.unrestricted.is_empty() {
            parts.push(format!(
                "offer every choice again in {}",
                self.labels(&d.unrestricted)
            ));
        }
        for m in &d.messages_added {
            parts.push(format!("show the message {}", fsv_py::py_repr_str(&m.text)));
        }
        parts
    }

    // -- writing ----------------------------------------------------------
    /// Apply one widget value; returns the rule notes or the error.
    pub fn set(&mut self, key: &str, raw: Value) -> Outcome {
        let Some(it) = self.item(key).cloned() else {
            return Outcome::error(format!("unknown form item {key}"));
        };
        self.touched.insert(key.to_string());
        self.errors_by_key.shift_remove(key);
        let spec_requester = it
            .field
            .as_ref()
            .is_some_and(|f| is_requester(f) && self.requester_is_special());
        if key == REQUESTER || spec_requester {
            return self.set_requester(&it, raw);
        }
        let Some(f) = it.field.clone() else {
            if it.widget != WidgetKind::File {
                return Outcome::error(format!("unknown form item {key}"));
            }
            let rules = FileRules {
                max_bytes: self.max_file_bytes,
                ..FileRules::default()
            };
            let (paths, err) = check_files(&raw, &rules, &self.paths);
            if !err.is_empty() {
                self.errors_by_key.insert(key.into(), err.clone());
                return Outcome::error(err);
            }
            if key == ATTACHMENTS {
                self.draft.attachments = paths;
            } else if let Some(label) = key.strip_prefix(PLANNING) {
                if paths.is_empty() {
                    self.draft.planning.shift_remove(label);
                } else {
                    self.draft.planning.insert(
                        label.into(),
                        PlanningFiles {
                            files: paths,
                            description_html: None,
                        },
                    );
                }
            }
            return Outcome::default();
        };
        let name = f.api_name.clone();
        let mut raw = raw;
        if it.widget == WidgetKind::File {
            let rules = FileRules {
                allowed: &it.allowed_types,
                max_files: it.max_files,
                max_bytes: self.max_file_bytes,
            };
            let (paths, err) = check_files(&raw, &rules, &self.paths);
            if !err.is_empty() {
                self.errors_by_key.insert(key.into(), err.clone());
                return Outcome::error(err);
            }
            raw = if paths.is_empty() {
                Value::Null
            } else {
                Value::list(paths.into_iter().map(Value::Str))
            };
        }
        if it.widget == WidgetKind::Richtext && self.kind == FormKind::Change {
            let md = if raw.is_null() {
                String::new()
            } else {
                raw.py_str()
            };
            return self.set_description(&it, &f, md);
        }
        let diff = match self.state.set(&name, raw) {
            Ok(d) => d,
            Err(e) => {
                self.errors_by_key.insert(key.into(), e.to_string());
                return Outcome::error(e.to_string());
            }
        };
        self.remember(&name);
        let mut notes = self.notes(&f, &diff);
        let restricted: Vec<String> = diff.restricted.keys().cloned().collect();
        notes.extend(self.preselect(&restricted));
        Outcome {
            notes,
            error: String::new(),
        }
    }

    /// An editor holds input it cannot read: keep the stored value but
    /// report `error` until fixed (an empty error withdraws it).
    pub fn mark_invalid(&mut self, key: &str, error: &str) {
        if error.is_empty() {
            self.errors_by_key.shift_remove(key);
        } else {
            self.touched.insert(key.to_string());
            self.errors_by_key.insert(key.into(), error.into());
        }
    }

    /// Unresolved input errors on visible fields (stop the submit).
    pub fn input_errors(&self) -> Vec<String> {
        self.errors_by_key
            .iter()
            .filter(|(k, _)| self.by_key.contains_key(*k) && self.view(k).visible)
            .filter_map(|(k, e)| self.item(k).map(|it| format!("{}: {e}", it.label)))
            .collect()
    }

    fn remember(&mut self, name: &str) {
        let user = self.state.sources.get(name).map(String::as_str) == Some("user");
        let value = self
            .state
            .values
            .get(name)
            .filter(|v| user && !v.is_null())
            .cloned();
        match value {
            Some(v) => {
                self.draft.answers.insert(name.into(), v);
            }
            None if self.mode == FormMode::Edit => {
                // clearing a field is a change
                self.draft.answers.insert(name.into(), Value::Null);
            }
            None => {
                self.draft.answers.shift_remove(name);
            }
        }
    }

    fn set_requester(&mut self, it: &FormItem, raw: Value) -> Outcome {
        let person = match raw {
            Value::Null => None,
            Value::Lookup(l) => Some(l),
            _ => {
                let err = "pick a person from the list".to_string();
                self.errors_by_key.insert(it.key.clone(), err.clone());
                return Outcome::error(err);
            }
        };
        if let Some(p) = &person
            && matches!(self.kind, FormKind::Sr | FormKind::Ticket)
            && p.email.as_deref().is_none_or(str::is_empty)
        {
            let err = format!("{p} has no email address");
            self.errors_by_key.insert(it.key.clone(), err.clone());
            return Outcome::error(err);
        }
        self.draft.requester = person
            .as_ref()
            .map(|p| p.email.clone().unwrap_or_else(|| p.display.clone()));
        self.requester = person.clone();
        if let Some(req) = self.requester_field()
            && matches!(self.kind, FormKind::Change | FormKind::Ticket)
            && let Err(e) = self
                .state
                .set(&req.api_name, person.map_or(Value::Null, Value::Lookup))
        {
            return Outcome::error(e.to_string());
        }
        Outcome::default()
    }

    fn set_description(&mut self, it: &FormItem, f: &Field, md: String) -> Outcome {
        let name = f.api_name.clone();
        let unchanged = self.mode == FormMode::Edit && md.trim() == self.desc_prefill.trim();
        self.desc_md = Some(md.clone());
        let html = if unchanged {
            self.draft.description_html = None;
            self.existing.get(&name).cloned().unwrap_or(Value::Null)
        } else if md.trim().is_empty() {
            self.draft.description_html = (self.mode == FormMode::Edit).then(String::new);
            Value::Null
        } else {
            let h = fsv_core::mdlite::to_html(&md);
            self.draft.description_html = Some(h.clone());
            Value::Str(h)
        };
        match self.state.set(&name, html) {
            Ok(diff) => Outcome {
                notes: self.notes(f, &diff),
                error: String::new(),
            },
            Err(e) => {
                self.errors_by_key.insert(it.key.clone(), e.to_string());
                Outcome::error(e.to_string())
            }
        }
    }

    fn notes(&self, f: &Field, diff: &Diff) -> Vec<String> {
        let ev = &self.state.evaluated;
        let now = self
            .state
            .display(&f.api_name, None)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(empty)".into());
        let cause = format!("{} = {now}", f.label);
        let mut out = vec![];
        if !diff.appeared.is_empty() {
            out.push(format!(
                "+ {} appeared because {cause}",
                self.labels_req(&diff.appeared, &self.state)
            ));
        }
        if !diff.disappeared.is_empty() {
            out.push(format!(
                "- {} hidden because {cause}",
                self.labels(&diff.disappeared)
            ));
        }
        let req: Vec<&String> = diff
            .became_required
            .iter()
            .filter(|n| !diff.appeared.contains(n) && ev.visible.contains(n))
            .collect();
        if !req.is_empty() {
            out.push(format!(
                "* {} now required because {cause}",
                self.labels(req)
            ));
        }
        let opt: Vec<&String> = diff
            .no_longer_required
            .iter()
            .filter(|n| ev.visible.contains(n))
            .collect();
        if !opt.is_empty() {
            out.push(format!("{} no longer required", self.labels(opt)));
        }
        for (n, labels) in &diff.restricted {
            let shown = if labels.is_empty() {
                "nothing".to_string()
            } else {
                labels.join(", ")
            };
            out.push(format!("{}: choices limited to {shown}", self.label_of(n)));
        }
        for n in &diff.unrestricted {
            out.push(format!(
                "{}: every choice is offered again",
                self.label_of(n)
            ));
        }
        for m in &diff.messages_added {
            let tail = if m.blocking {
                " (blocking: acknowledge it on the review)"
            } else {
                ""
            };
            out.push(format!("! {}{tail}", m.text));
        }
        if !diff.cleared.is_empty() {
            out.push(format!(
                "{} cleared ({} changed)",
                self.labels(&diff.cleared),
                f.label
            ));
        }
        out
    }

    /// A rule left exactly one choice on an empty field: pick it (the
    /// review says so), like the YAML draft does.
    fn preselect(&mut self, names: &[String]) -> Vec<String> {
        let mut out = vec![];
        for n in names {
            let ev = &self.state.evaluated;
            if !ev.visible.contains(n)
                || ev.disabled.contains(n)
                || ev.values.get(n).is_some_and(|v| !v.is_empty())
            {
                continue;
            }
            let Some(f) = self.spec().get(n).cloned() else {
                continue;
            };
            if !matches!(f.kind, Kind::Dropdown | Kind::Department | Kind::Nested) {
                continue;
            }
            let Some((ch, rule)) = only_option(&self.state, n) else {
                continue;
            };
            let Ok(diff) = self.state.set(n, Value::Str(ch.value.clone())) else {
                continue;
            };
            self.remember(n);
            out.push(format!(
                "{} = {} (the only choice rule {rule} allows)",
                f.label, ch.label
            ));
            out.extend(self.notes(&f, &diff));
        }
        out
    }

    fn preselect_all(&mut self) -> Vec<String> {
        let names: Vec<String> = self.state.evaluated.restricted.keys().cloned().collect();
        self.preselect(&names)
    }

    // -- plans --------------------------------------------------------------
    fn request<'a>(&'a self, ack: bool, pending: &'a [String]) -> PlanRequest<'a> {
        PlanRequest {
            kind: self.kind,
            mode: self.mode,
            draft: &self.draft,
            record: &self.record,
            target: self.target.as_ref(),
            requester: self.requester.as_ref(),
            ack_all: ack,
            pending_errors: pending,
        }
    }

    /// Build the plan through the backend; pending input errors block it.
    pub fn build_plan(&mut self, ack: Option<bool>) -> Result<Plan, String> {
        let ack = ack.unwrap_or(self.ack);
        let pending = self.input_errors();
        let mut plan = self.backend.build_plan(&self.request(ack, &pending))?;
        plan.block_on(&pending);
        self.plan = Some(plan.clone());
        Ok(plan)
    }

    pub fn default_draft_path(&self) -> PathBuf {
        let name = match (self.kind, self.mode) {
            (FormKind::Ticket, _) => "ticket-draft.yaml".to_string(),
            (FormKind::Sr, _) => "sr-draft.yaml".to_string(),
            (FormKind::Change, FormMode::Edit) => format!(
                "change-{}-draft.yaml",
                self.target
                    .as_ref()
                    .map(fsv_py::py_str)
                    .unwrap_or_else(|| "None".into())
            ),
            (FormKind::Change, FormMode::New) => "change-draft.yaml".to_string(),
        };
        self.paths.cwd.join(name)
    }

    /// Write the draft for `plan` (or a fresh plan) to `path`.
    pub fn save_draft(&mut self, path: &str, plan: Option<&Plan>) -> std::io::Result<PathBuf> {
        let built;
        let plan = match plan {
            Some(p) => p,
            None => {
                built = self.build_plan(None).map_err(std::io::Error::other)?;
                &built
            }
        };
        let p = self.paths.abspath(path);
        let pending = self.input_errors();
        self.backend
            .save_draft(&self.request(self.ack, &pending), plan, &p)?;
        self.saved_path = Some(p.clone());
        Ok(p)
    }

    /// The equivalent CLI command for a saved draft.
    pub fn command_for(&self, path: &str, plan: &Plan) -> String {
        let h: String = plan.hash.chars().take(16).collect();
        let q = shell_quote(path);
        let ack = if self.ack { " --ack" } else { "" };
        match (self.kind, self.mode) {
            (FormKind::Sr, _) => {
                let id = self
                    .spec()
                    .item
                    .as_ref()
                    .map_or_else(|| "None".into(), |i| fsv_py::py_str(&i.display_id));
                format!("fsv sr create {id} --from {q} --yes --expect-plan {h}{ack}")
            }
            (FormKind::Ticket, _) => {
                format!("fsv tickets create --from {q} --yes --expect-plan {h}{ack}")
            }
            (FormKind::Change, FormMode::Edit) => format!(
                "fsv changes update {} --from {q} --yes --expect-plan {h}{ack}",
                self.target
                    .as_ref()
                    .map(fsv_py::py_str)
                    .unwrap_or_else(|| "None".into())
            ),
            (FormKind::Change, FormMode::New) => {
                format!("fsv changes create --from {q} --yes --expect-plan {h}{ack}")
            }
        }
    }

    /// The non-interactive send path for embedding apps (TUI): duplicate
    /// check, freshness re-check, then the writes. A dry run refuses.
    pub fn send(&self, plan: &Plan, allow_duplicate: bool) -> Result<Json, fsv_core::Error> {
        if self.dry_run {
            return Err(fsv_core::Error::exit(2, "dry run: nothing is sent"));
        }
        let pending = self.input_errors();
        // an edit is never a duplicate of an earlier send
        let allow = allow_duplicate || self.mode == FormMode::Edit && self.kind == FormKind::Change;
        self.backend
            .send(&self.request(self.ack, &pending), plan, allow)
    }
}

/// `"New Change"` / `"Edit change CHN-1"`.
pub fn change_title(mode: FormMode, record: &Json, target: Option<&Json>) -> String {
    match mode {
        FormMode::New => "New Change".into(),
        FormMode::Edit => {
            let shown = record
                .get("human_display_id")
                .filter(|v| fsv_py::truthy(v))
                .or(target)
                .map(fsv_py::py_str)
                .unwrap_or_else(|| "None".into());
            format!("Edit change {shown}")
        }
    }
}

/// `"New service request: <item name>"`.
pub fn sr_title(spec: &FormSpec, meta_name: &str) -> String {
    let name = spec
        .item
        .as_ref()
        .map(|i| i.name.clone())
        .unwrap_or_else(|| meta_name.to_string());
    format!("New service request: {name}")
}

pub const TICKET_TITLE: &str = "New Incident";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acme;

    // Expected values: `change_session(acme_change_form.FakeClient())` in Python.

    #[test]
    fn plain_squashes_and_cuts() {
        assert_eq!(
            plain("<p>Hello <b>big</b>\n  world</p>", 600),
            "Hello big world"
        );
        assert_eq!(plain("abcdefghij", 5), "abcd…");
        assert_eq!(plain("", 600), "");
    }

    #[test]
    fn remember_keeps_user_answers_only() {
        let (_a, mut s) = acme::acme();
        let answers = |s: &FormSession| s.draft.answers.clone().into_iter().collect::<Vec<_>>();
        s.state.set("subject", "Patch").expect("set");
        s.remember("subject");
        assert_eq!(answers(&s), [("subject".into(), Value::from("Patch"))]);
        s.state.sources.insert("subject".into(), "rule".into());
        s.remember("subject");
        assert_eq!(answers(&s), []);
        s.state.sources.insert("subject".into(), "user".into());
        s.remember("subject");
        assert_eq!(answers(&s), [("subject".into(), Value::from("Patch"))]);
        s.state.values.shift_remove("subject");
        s.remember("subject");
        assert_eq!(answers(&s), []);
        s.mode = FormMode::Edit;
        s.remember("subject");
        assert_eq!(answers(&s), [("subject".into(), Value::Null)]);
        s.state
            .values
            .insert("subject".into(), Value::from("Again"));
        s.remember("subject");
        assert_eq!(answers(&s), [("subject".into(), Value::from("Again"))]);
    }

    #[test]
    fn structure_of_a_new_change() {
        let (_a, s) = acme::acme();
        assert_eq!(
            s.requester_field().map(|f| f.api_name),
            Some("requester".into())
        );
        assert_eq!(
            s.description_field().map(|f| f.api_name),
            Some("description".into())
        );
        assert!(s.requester_is_special());
        let (tz, source) = s.timezone();
        assert_eq!(
            (tz.name.as_str(), source.as_str()),
            ("Asia/Tokyo", "Freshservice user time zone (Tokyo)")
        );
        let items: Vec<(String, String, WidgetKind, bool, Option<String>)> = s
            .build_items()
            .into_iter()
            .map(|it| {
                let synthetic = it.is_synthetic();
                (it.key, it.label, it.widget, synthetic, it.entity)
            })
            .collect();
        let user = || Some("user".to_string());
        use WidgetKind::*;
        let want: Vec<(&str, &str, WidgetKind, bool, Option<String>)> = vec![
            ("requester", "Requester", Lookup, false, user()),
            ("requested_for", "Request for", Lookup, false, user()),
            ("subject", "Subject", Text, false, None),
            ("change_type", "Change Type", Choice, false, None),
            ("cloud_only", "Cloud Only", Choice, false, None),
            ("stage", "Stage", Choice, false, None),
            ("staging_start", "Staging Start", Datetime, false, None),
            ("staging_end", "Staging End", Datetime, false, None),
            ("services", "Services", Multi, false, None),
            ("notify_ops", "Notify Ops Team", Choice, false, None),
            ("reviewer", "Reviewer", Lookup, false, user()),
            ("downtime_window", "Downtime Window", Text, false, None),
            ("legacy_flag", "Legacy Flag", Checkbox, false, None),
            ("description", "Description", Richtext, false, None),
            ("priority", "Priority", Choice, false, None),
            ("impact", "Impact", Choice, false, None),
            ("risk", "Risk", Choice, false, None),
            ("group", "Group", Lookup, false, Some("group".into())),
            ("agent", "Agent", Lookup, false, Some("agent".into())),
            (
                "planned_start_date",
                "Planned Start Date",
                Datetime,
                false,
                None,
            ),
            (
                "planned_end_date",
                "Planned End Date",
                Datetime,
                false,
                None,
            ),
            (ATTACHMENTS, "Attachments", File, true, None),
            (
                "planning:Go-Live Checklist",
                "Go-Live Checklist (planning files)",
                File,
                true,
                None,
            ),
        ];
        let want: Vec<_> = want
            .into_iter()
            .map(|(k, l, w, syn, e)| (k.to_string(), l.to_string(), w, syn, e))
            .collect();
        assert_eq!(items, want);
        let synthetic = FormItem::synthetic(REQUESTER, "Requester", Lookup, "help");
        assert!(synthetic.is_synthetic() && synthetic.entity.is_none());
    }

    #[test]
    fn visible_missing_and_pools() {
        let (_a, s) = acme::acme();
        let visible = s.visible_keys();
        assert_eq!(visible.len(), 22);
        assert!(!visible.iter().any(|k| k == "legacy_flag"));
        assert_eq!(visible[..3], ["requester", "requested_for", "subject"]);
        assert_eq!(
            s.missing_keys(),
            [
                "requester",
                "subject",
                "cloud_only",
                "description",
                "planned_start_date",
                "planned_end_date"
            ]
        );
        let labels = |key: &str| -> Vec<String> {
            s.pool(s.item(key).expect("item"))
                .into_iter()
                .map(|c| c.label)
                .collect()
        };
        assert_eq!(labels("priority"), ["Low", "Medium", "High", "Urgent"]);
        assert_eq!(labels("stage"), ["Live", "Staging"]);
    }

    #[test]
    fn requester_and_description_setters() {
        let (_a, mut s) = acme::acme();
        let req = s.item("requester").cloned().expect("item");
        let out = s.set_requester(&req, Value::str("jane"));
        assert_eq!(out.error, "pick a person from the list");
        assert_eq!(
            s.errors_by_key.get("requester").map(String::as_str),
            Some("pick a person from the list")
        );
        let out = s.set_requester(&req, Value::Lookup(acme::jane()));
        assert_eq!(out, Outcome::default());
        assert_eq!(s.draft.requester.as_deref(), Some("jane.doe@example.com"));
        assert_eq!(
            s.requester.as_ref().map(ToString::to_string).as_deref(),
            Some("Jane Doe <jane.doe@example.com>")
        );
        assert_eq!(s.desc_text(), "");
        let desc = s.item("description").cloned().expect("item");
        let f = desc.field.clone().expect("field");
        let out = s.set_description(&desc, &f, "Rotate the **billing** keys.".into());
        assert_eq!(out, Outcome::default());
        assert_eq!(
            s.draft.description_html.as_deref(),
            Some("<p>Rotate the <strong>billing</strong> keys.</p>")
        );
        assert_eq!(s.desc_text(), "Rotate the **billing** keys.");
    }

    #[test]
    fn a_forked_state_explains_a_choice_and_leaves_the_form_alone() {
        let (_a, s) = acme::acme();
        let mut st = fork(&s.state);
        let d = st.set("cloud_only", Value::str("No")).expect("set");
        assert_eq!(
            s.diff_parts(&st, &d),
            [
                "make required: Staging Start, Staging End",
                "limit Stage to Staging"
            ]
        );
        assert!(matches!(s.state.get("cloud_only"), Ok(None)));
    }

    #[test]
    fn a_single_allowed_choice_is_preselected_once() {
        let (_a, mut s) = acme::acme();
        assert!(s.preselect_all().is_empty());
        assert!(s.load_notes.is_empty());
        s.state.set("cloud_only", Value::str("No")).expect("set");
        assert_eq!(
            s.preselect(&["stage".to_string()]),
            ["Stage = Staging (the only choice rule Cloud only: no allows)"]
        );
        assert!(s.preselect(&["stage".to_string()]).is_empty());
    }
}
