//! Business-rules engine (`fsv.forms.rules`): the client-side form rules
//! Freshservice runs in the browser (`GET /api/_/ws/2/business_rules`).
//!
//! Two evaluation models ([`Model`]):
//!
//! * `event` (the web's own model, the default): on load every rule is
//!   evaluated in array order against the load values; each user change is
//!   a change event that evaluates the rules again on the current state.
//!   Answers without an order are replayed in web field order (`position`).
//!   Setting a field to the value it already has is not a change event.
//! * `replay`: every evaluation starts from the metadata and applies every
//!   rule in order; a later rule overrides an earlier one.
//!
//! `populate` ([`PopulateMode`]): `restrict` (default) limits a dropdown's
//! choices to the action's values; `set` (legacy) fills in the first value.
//!
//! A matching rule applies its actions; a rule that does not match applies
//! the reverse (show<->hide, mandate<->unmandate, disable<->enable) only if
//! `is_reversible`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use serde_json::{Map, Value as Json};

use crate::errors::{FormError, Result};
use crate::pytext::{jrepr, jstr, repr_str_list, strip, truthy};
use crate::spec::{CUSTOM_PREFIXES, Choice, Field, FormSpec, Kind};
use crate::value::{PyObj, Value, Values, norm};

pub const SUPPORTED_OPERATORS: [&str; 6] = [
    "is",
    "is_not",
    "includes",
    "does_not_include",
    "selected",
    "not_selected",
];
pub const SUPPORTED_ACTIONS: [&str; 7] = [
    "show",
    "hide",
    "mandate",
    "unmandate",
    "populate",
    "disable",
    "message",
];
/// `evaluate_on` values that read a form field (the module's own name).
pub const FORM_SOURCES: [&str; 6] = [
    "change",
    "ticket",
    "problem",
    "release",
    "service_request",
    "form",
];
/// `evaluate_on` values that read a person record from the rule context.
pub const USER_SOURCES: [(&str, &str); 3] = [
    ("current_user", "user"),
    ("requester", "requester"),
    ("requested_for", "requested_for"),
];
/// ... and the context's `system` mapping (e.g. `portal`).
pub const CONTEXT_SOURCES: [(&str, &str); 4] = [
    ("current_user", "user"),
    ("requester", "requester"),
    ("requested_for", "requested_for"),
    ("system", "system"),
];
/// Action targets that are not form fields but form-level switches.
pub const PSEUDO_TARGETS: [&str; 1] = ["attachment_ids"];
pub const EXCLUDE_KEYS: [&str; 3] = ["optional", "disabled", "hidden"];
pub const MODELS: [&str; 2] = ["event", "replay"];
pub const POPULATE_MODES: [&str; 2] = ["restrict", "set"];
pub const DEDUPE_POLICIES: [&str; 4] = ["exact", "first", "last", "none"];
pub const EVENT_SCOPES: [&str; 2] = ["referenced", "all"];
pub const DEFAULT_MODEL: Model = Model::Event;
pub const DEFAULT_POPULATE: PopulateMode = PopulateMode::Restrict;
pub const DEFAULT_EVENT_SCOPE: EventScope = EventScope::All;
pub const DEFAULT_EVENT_REVERSE: EventReverse = EventReverse::Always;
pub const MAX_PASSES: usize = 8;

/// Every `evaluate_on` value the engine understands.
pub fn supported_evaluate_on(v: &str) -> bool {
    FORM_SOURCES.contains(&v) || context_source(v).is_some()
}

fn context_source(v: &str) -> Option<&'static str> {
    CONTEXT_SOURCES
        .iter()
        .find(|(k, _)| *k == v)
        .map(|(_, s)| *s)
}

fn reverse_of(action: &str) -> Option<&'static str> {
    Some(match action {
        "show" => "hide",
        "hide" => "show",
        "mandate" => "unmandate",
        "unmandate" => "mandate",
        "disable" => "enable",
        "populate" => "restore",
        _ => return None,
    })
}

fn kind_of(action: &str) -> &'static str {
    match action {
        "show" | "hide" => "visible",
        "mandate" | "unmandate" => "required",
        _ => "disabled",
    }
}

fn is_positive(action: &str) -> bool {
    matches!(action, "show" | "mandate" | "disable")
}

fn py_tuple_repr(items: &[&str]) -> String {
    let parts: Vec<String> = items.iter().map(|s| fsv_py::py_repr_str(s)).collect();
    format!("({})", parts.join(", "))
}

macro_rules! option_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident => $s:literal),+ $(,)? } default $def:ident, $err:expr) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($var),+ }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $($name::$var => $s),+ }
            }

            /// Parse the Python option string (`""` selects the default).
            pub fn parse(s: &str) -> Result<Self> {
                match s {
                    "" => Ok($name::$def),
                    $($s => Ok($name::$var),)+
                    _ => Err(FormError::Value($err)),
                }
            }
        }

        impl Default for $name {
            fn default() -> Self { $name::$def }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }
    };
}

option_enum!(
    /// Evaluation model.
    Model { Event => "event", Replay => "replay" } default Event,
    format!("model must be one of {}", py_tuple_repr(&MODELS))
);
option_enum!(
    /// What `populate` does.
    PopulateMode { Restrict => "restrict", Set => "set" } default Restrict,
    format!("populate must be one of {}", py_tuple_repr(&POPULATE_MODES))
);
option_enum!(
    /// Several entries with the same rule id: `exact` drops identical
    /// repeats; `first`/`last` keep one per id; `none` keeps everything.
    Dedupe { Exact => "exact", First => "first", Last => "last", None => "none" } default Exact,
    format!("dedupe must be one of {}", py_tuple_repr(&DEDUPE_POLICIES))
);
option_enum!(
    /// Event model: which rules a change event runs.
    EventScope { Referenced => "referenced", All => "all" } default All,
    format!("event_scope must be one of {}", py_tuple_repr(&EVENT_SCOPES))
);
option_enum!(
    /// Event model: when a reversible rule that does not match reverses.
    EventReverse { Transition => "transition", Always => "always" } default Always,
    "event_reverse must be 'transition' or 'always'".to_string()
);

/// `is_empty(value)`: None, `""` or an empty collection.
pub fn is_empty(v: Option<&Value>) -> bool {
    v.is_none_or(Value::is_empty)
}

// ---------------------------------------------------------------------------
// data model

/// Who is filling which form: decides which rules apply and what
/// `current_user` / `requester` / `system` conditions read.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleContext {
    pub module: Option<String>,
    /// `"new"` | `"edit"`.
    pub form: String,
    pub user_type: String,
    /// One id or a JSON array of ids; `None` disables workspace filtering.
    pub workspace_id: Option<Json>,
    pub item_id: Json,
    pub user: Option<Map<String, Json>>,
    pub requester: Option<Map<String, Json>>,
    pub requested_for: Option<Map<String, Json>>,
    pub system: Option<Map<String, Json>>,
}

impl Default for RuleContext {
    fn default() -> Self {
        RuleContext {
            module: None,
            form: "new".to_string(),
            user_type: "agent".to_string(),
            workspace_id: None,
            item_id: Json::Null,
            user: None,
            requester: None,
            requested_for: None,
            system: None,
        }
    }
}

impl RuleContext {
    pub fn workspaces(&self) -> Option<HashSet<String>> {
        match self.workspace_id.as_ref()? {
            Json::Null => None,
            Json::Array(a) => Some(a.iter().map(jstr).collect()),
            Json::Object(o) => Some(o.keys().cloned().collect()),
            other => Some(HashSet::from([jstr(other)])),
        }
    }

    /// The record a context source names (`user`, `requester`, ...).
    pub fn source(&self, source: &str) -> Option<&Map<String, Json>> {
        match source {
            "user" => self.user.as_ref(),
            "requester" => self.requester.as_ref(),
            "requested_for" => self.requested_for.as_ref(),
            "system" => self.system.as_ref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    /// Raw name as written in the rule.
    pub name: String,
    /// API name (`None` if unknown); the attribute for context sources.
    pub field: Option<String>,
    pub operator: String,
    pub values: Vec<String>,
    /// `"form"` | `"user"` | `"requester"` | `"requested_for"` | `"system"`.
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConditionItem {
    Condition(Condition),
    Group(ConditionGroup),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConditionGroup {
    /// `"all"` | `"any"`.
    pub mode: String,
    pub items: Vec<ConditionItem>,
}

/// One action target. `api` is the form field (`None` when unknown);
/// `base`/`custom` give the server-side name used in `br_validation_excludes`.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub api: Option<String>,
    pub raw: String,
    pub base: String,
    pub custom: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub property: String,
    /// API names of known fields; message texts for `message`.
    pub targets: Vec<String>,
    pub values: Vec<Json>,
    pub all_targets: Vec<Target>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub id: Json,
    pub name: String,
    pub condition: ConditionGroup,
    pub actions: Vec<Action>,
    pub reversible: bool,
    pub form_type: String,
    pub user_type: String,
    pub is_validation: bool,
    pub index: usize,
    /// Form fields its conditions read.
    pub fields: BTreeSet<String>,
    pub unconditional: bool,
}

impl Rule {
    pub fn applies_to(&self, mode: &str, user_type: &str) -> bool {
        let ft = match mode {
            "new" => "new_form",
            "edit" => "edit_form",
            m => m,
        };
        if self.form_type != "both" && self.form_type != ft {
            return false;
        }
        self.user_type == "both" || self.user_type == user_type || user_type == "any"
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub text: String,
    pub rule_id: Json,
    pub rule_name: String,
    pub blocking: bool,
}

impl Message {
    pub fn key(&self) -> (String, String) {
        (jstr(&self.rule_id), self.text.clone())
    }
}

/// One rule action applied to one field property.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub rule_id: Json,
    pub rule_name: String,
    /// show/hide/mandate/unmandate/disable/enable/populate/restore.
    pub action: String,
    /// False: reverse action of a reversible rule that did not match.
    pub matched: bool,
    /// Field whose change triggered it; `None` = load / replay.
    pub event: Option<String>,
    pub rule_index: i64,
}

impl Decision {
    pub fn describe(&self) -> String {
        let how = if self.matched {
            ""
        } else {
            " (reversed: rule does not match)"
        };
        let on = match &self.event {
            Some(e) if !e.is_empty() => format!(" on change of {e}"),
            _ => String::new(),
        };
        format!(
            "{} by rule {} (id {}){how}{on}",
            self.action,
            fsv_py::py_repr_str(&self.rule_name),
            jstr(&self.rule_id)
        )
    }
}

/// Matching rules that disagree on one field property; `winner` is the
/// decision in effect (the last one applied).
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub field: String,
    pub property: String,
    pub decisions: Vec<Decision>,
    pub winner: Decision,
}

impl Conflict {
    pub fn describe(&self) -> String {
        let rules: Vec<String> = self
            .decisions
            .iter()
            .map(|d| {
                format!(
                    "{} by {} (id {})",
                    d.action,
                    fsv_py::py_repr_str(&d.rule_name),
                    jstr(&d.rule_id)
                )
            })
            .collect();
        format!(
            "{}.{}: {} -> {} wins ({})",
            self.field,
            self.property,
            rules.join("; "),
            self.winner.action,
            fsv_py::py_repr_str(&self.winner.rule_name)
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Explanation {
    pub field: String,
    pub label: String,
    /// visible | required | disabled | choices restricted.
    pub property: String,
    pub value: bool,
    /// rule | metadata | section | nested | parent.
    pub source: String,
    pub decision: Option<Decision>,
}

impl fmt::Display for Explanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = if self.value {
            self.property.clone()
        } else {
            format!("not {}", self.property)
        };
        if let Some(d) = &self.decision {
            return write!(f, "{}: {state} ({})", self.label, d.describe());
        }
        let why = match self.source.as_str() {
            "metadata" => "form metadata",
            "section" => "its section's choice is not selected",
            "nested" => "the upper level has no options for it",
            "parent" => "the field it belongs to is hidden",
            s => s,
        };
        write!(f, "{}: {state} ({why})", self.label)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExcludeName {
    pub base: String,
    pub custom: bool,
}

impl ExcludeName {
    pub fn name(&self, account_id: Option<&str>) -> Result<String> {
        if !self.custom {
            return Ok(self.base.clone());
        }
        match account_id.filter(|a| !a.is_empty()) {
            Some(acc) => Ok(format!("{}_{acc}", self.base)),
            None => Err(FormError::Value(format!(
                "custom field {} needs the account id",
                fsv_py::py_repr_str(&self.base)
            ))),
        }
    }
}

/// `br_validation_excludes` lists (keys of [`EXCLUDE_KEYS`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Excludes {
    pub optional: Vec<ExcludeName>,
    pub disabled: Vec<ExcludeName>,
    pub hidden: Vec<ExcludeName>,
}

impl Excludes {
    /// The list for one of [`EXCLUDE_KEYS`].
    pub fn get(&self, key: &str) -> Option<&[ExcludeName]> {
        match key {
            "optional" => Some(&self.optional),
            "disabled" => Some(&self.disabled),
            "hidden" => Some(&self.hidden),
            _ => None,
        }
    }

    /// `(key, list)` in [`EXCLUDE_KEYS`] order.
    pub fn items(&self) -> [(&'static str, &[ExcludeName]); 3] {
        [
            ("optional", &self.optional),
            ("disabled", &self.disabled),
            ("hidden", &self.hidden),
        ]
    }
}

/// The choices a `populate` action leaves on offer for one dropdown.
///
/// `allowed` maps each offered choice (normalised choice value) to the
/// restriction of its nested options: `None` means all of them.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceRestriction {
    pub allowed: IndexMap<String, Option<ChoiceRestriction>>,
    pub rule_id: Json,
    pub rule_name: String,
}

impl ChoiceRestriction {
    pub fn offers<R: PyObj + ?Sized>(&self, value: &R) -> bool {
        self.allowed.contains_key(&norm(&value.py_str()))
    }

    pub fn below<R: PyObj + ?Sized>(&self, value: &R) -> Option<&ChoiceRestriction> {
        self.allowed
            .get(&norm(&value.py_str()))
            .and_then(Option::as_ref)
    }
}

/// The result of [`RulesEngine::evaluate`].
#[derive(Clone, Debug)]
pub struct EvaluatedForm {
    /// Ordered, inputs and notices.
    pub visible: Vec<String>,
    /// Visible and required.
    pub required: BTreeSet<String>,
    pub disabled: BTreeSet<String>,
    /// Effective values (user + populated).
    pub values: Values,
    /// API name -> id of the rule that populated it.
    pub populated: IndexMap<String, Json>,
    pub messages: Vec<Message>,
    /// Ids of matching rules.
    pub matched: Vec<Json>,
    pub model: Model,
    /// Fields rules unmandated / disabled / hid, in the order the actions ran.
    pub excludes: Excludes,
    pub decisions: IndexMap<(String, String), Decision>,
    pub conflicts: Vec<Conflict>,
    /// User answers refused because a rule disabled the field.
    pub refused: Vec<(String, Option<Decision>)>,
    /// User answers a later populate replaced: field -> (user value, rule id).
    pub overridden: IndexMap<String, (Value, Json)>,
    /// Rule override for item attachments.
    pub attachments_required: Option<bool>,
    pub account_id: Option<String>,
    pub spec: Arc<FormSpec>,
    /// `populate="restrict"`: top-level field -> choices still on offer.
    pub restricted: IndexMap<String, ChoiceRestriction>,
    /// The populate decision in effect (also kept for restores).
    pub restricted_by: IndexMap<String, Decision>,
    /// Visible fields whose value is not on offer any more.
    pub unavailable: IndexMap<String, (Value, Option<Decision>)>,
    pub populate_mode: PopulateMode,
}

impl EvaluatedForm {
    /// Choices the dropdown offers now: nested levels follow the upper
    /// levels, and a `populate` restriction keeps only its choices.
    pub fn choices(&self, name: &str) -> Result<Vec<Choice>> {
        let f = self.spec.field(name)?;
        offered_choices(&self.spec, f, &self.values, &self.restricted)
    }

    /// The restriction that applies to `name` (its top-level field's) and
    /// the decision that set it.
    pub fn restriction_for(
        &self,
        name: &str,
    ) -> Result<(Option<&ChoiceRestriction>, Option<&Decision>)> {
        let f = self.spec.field(name)?;
        let top = &nested_chain(&self.spec, f)[0].api_name;
        Ok((self.restricted.get(top), self.restricted_by.get(top)))
    }

    pub fn is_visible(&self, name: &str) -> bool {
        self.visible.iter().any(|n| n == name)
    }

    pub fn is_required(&self, name: &str) -> bool {
        self.required.contains(name)
    }

    /// Messages of `is_validation` rules: the web blocks submit until they
    /// are dealt with, so fsv requires an explicit acknowledgement.
    pub fn blocking_messages(&self) -> Vec<&Message> {
        self.messages.iter().filter(|m| m.blocking).collect()
    }

    pub fn decision(&self, name: &str, prop: &str) -> Option<&Decision> {
        self.decisions.get(&(name.to_string(), prop.to_string()))
    }

    /// Why each field is (not) visible, required and disabled.
    pub fn explain(&self, names: Option<&[&str]>) -> Result<Vec<Explanation>> {
        let spec = &self.spec;
        let wanted: Vec<String> = match names {
            Some(ns) => ns.iter().map(|s| s.to_string()).collect(),
            None => spec.ordered().iter().map(|f| f.api_name.clone()).collect(),
        };
        let mut out = Vec::new();
        for n in wanted {
            let f = spec.field(&n)?;
            let n = f.api_name.clone();
            let vis = self.is_visible(&n);
            let mut d = self.decision(&n, "visible").cloned();
            let mut src = if d.is_some() { "rule" } else { "metadata" };
            if !vis && d.as_ref().is_none_or(|d| d.action == "show") {
                if f.parent
                    .as_deref()
                    .is_some_and(|p| !p.is_empty() && !self.is_visible(p))
                {
                    (src, d) = ("parent", None);
                } else if spec.section_parent(&n).is_some() {
                    (src, d) = ("section", None);
                } else if f.is_nested_level() {
                    (src, d) = ("nested", None);
                }
            }
            out.push(explanation(f, "visible", vis, src, d));
            if vis && f.kind != Kind::Notice {
                let d = self.decision(&n, "required").cloned();
                let src = if d.is_some() { "rule" } else { "metadata" };
                out.push(explanation(
                    f,
                    "required",
                    self.required.contains(&n),
                    src,
                    d,
                ));
            }
            let d = self.decision(&n, "disabled").cloned();
            if d.is_some() || self.disabled.contains(&n) {
                let src = if d.is_some() { "rule" } else { "metadata" };
                out.push(explanation(
                    f,
                    "disabled",
                    self.disabled.contains(&n),
                    src,
                    d,
                ));
            }
            let (r, d) = self.restriction_for(&n)?;
            if let Some(d) = d
                && f.kind.is_choice()
            {
                out.push(explanation(
                    f,
                    "choices restricted",
                    r.is_some(),
                    "rule",
                    Some(d.clone()),
                ));
            }
        }
        Ok(out)
    }
}

fn explanation(
    f: &Field,
    prop: &str,
    value: bool,
    source: &str,
    decision: Option<Decision>,
) -> Explanation {
    Explanation {
        field: f.api_name.clone(),
        label: f.label.clone(),
        property: prop.to_string(),
        value,
        source: source.to_string(),
        decision,
    }
}

// ---------------------------------------------------------------------------
// engine

/// Keyword arguments of `RulesEngine(...)`.
#[derive(Clone, Debug)]
pub struct EngineOptions {
    pub module: Option<String>,
    pub user_type: String,
    pub item_id: Json,
    pub workspace_id: Option<Json>,
    pub user: Option<Map<String, Json>>,
    /// Replaces `module`/`user_type`/`item_id`/`workspace_id`/`user`.
    pub context: Option<RuleContext>,
    pub dedupe: Dedupe,
    /// Unsupported rules raise (`true`) or are skipped and listed.
    pub strict: bool,
    pub model: Model,
    pub same_value_fires: bool,
    pub event_reverse: EventReverse,
    pub populate: PopulateMode,
    pub system: Option<Map<String, Json>>,
    pub event_scope: EventScope,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            module: None,
            user_type: "agent".to_string(),
            item_id: Json::Null,
            workspace_id: None,
            user: None,
            context: None,
            dedupe: Dedupe::Exact,
            strict: true,
            model: DEFAULT_MODEL,
            same_value_fires: false,
            event_reverse: DEFAULT_EVENT_REVERSE,
            populate: DEFAULT_POPULATE,
            system: None,
            event_scope: DEFAULT_EVENT_SCOPE,
        }
    }
}

impl EngineOptions {
    /// The options older tests use: `populate="set"`,
    /// `event_scope="referenced"`, `event_reverse="transition"`.
    pub fn legacy() -> Self {
        EngineOptions {
            populate: PopulateMode::Set,
            event_scope: EventScope::Referenced,
            event_reverse: EventReverse::Transition,
            ..EngineOptions::default()
        }
    }

    pub fn module(mut self, module: impl Into<String>) -> Self {
        self.module = Some(module.into());
        self
    }
}

/// Evaluates business rules for one [`FormSpec`].
///
/// `strict` (default) fails with `UnsupportedRuleError`
/// ([`FormError::FormRule`] with `unsupported`) for a rule with an unknown
/// operator, action or `evaluate_on`; non-strict skips such rules and lists
/// them in `unsupported`.
#[derive(Debug)]
pub struct RulesEngine {
    pub spec: Arc<FormSpec>,
    pub context: RuleContext,
    pub populate: PopulateMode,
    pub dedupe: Dedupe,
    pub strict: bool,
    pub model: Model,
    pub same_value_fires: bool,
    pub event_reverse: EventReverse,
    pub event_scope: EventScope,
    pub unsupported: Vec<String>,
    pub skipped: Vec<String>,
    pub duplicates: Vec<Json>,
    pub rules: Vec<Rule>,
    warnings: Mutex<Vec<String>>,
    pos: HashMap<String, f64>,
}

impl Clone for RulesEngine {
    fn clone(&self) -> Self {
        RulesEngine {
            spec: self.spec.clone(),
            context: self.context.clone(),
            populate: self.populate,
            dedupe: self.dedupe,
            strict: self.strict,
            model: self.model,
            same_value_fires: self.same_value_fires,
            event_reverse: self.event_reverse,
            event_scope: self.event_scope,
            unsupported: self.unsupported.clone(),
            skipped: self.skipped.clone(),
            duplicates: self.duplicates.clone(),
            rules: self.rules.clone(),
            warnings: Mutex::new(self.warnings()),
            pos: self.pos.clone(),
        }
    }
}

struct Skip(String);

fn obj(v: &Json) -> Option<&Map<String, Json>> {
    v.as_object()
}

/// `d.get(k)` with Python's `None` for missing and null alike.
fn mget<'a>(m: &'a Map<String, Json>, k: &str) -> Option<&'a Json> {
    m.get(k).filter(|v| !v.is_null())
}

fn mget_str(m: &Map<String, Json>, k: &str) -> String {
    m.get(k).map(jstr).unwrap_or_else(|| "None".to_string())
}

static NULL: Json = Json::Null;

impl RulesEngine {
    /// `RulesEngine(spec, rules, **options)`; `rules` is the business_rules
    /// response (object with `business_rules`) or a list of rules.
    pub fn new(
        spec: Arc<FormSpec>,
        rules: Option<&Json>,
        opts: EngineOptions,
    ) -> Result<RulesEngine> {
        let raw_user = rules
            .and_then(obj)
            .and_then(|m| m.get("user"))
            .and_then(obj)
            .cloned();
        let context = match opts.context {
            None => RuleContext {
                module: opts.module,
                user_type: opts.user_type,
                item_id: opts.item_id,
                workspace_id: opts.workspace_id,
                user: opts.user.or(raw_user),
                system: opts.system,
                ..RuleContext::default()
            },
            Some(mut ctx) => {
                if opts.system.is_some() && ctx.system.is_none() {
                    ctx.system = opts.system;
                }
                ctx
            }
        };
        let mut engine = RulesEngine {
            spec: spec.clone(),
            context,
            populate: opts.populate,
            dedupe: opts.dedupe,
            strict: opts.strict,
            model: opts.model,
            same_value_fires: opts.same_value_fires,
            event_reverse: opts.event_reverse,
            event_scope: opts.event_scope,
            unsupported: vec![],
            skipped: vec![],
            duplicates: vec![],
            rules: vec![],
            warnings: Mutex::new(vec![]),
            pos: HashMap::new(),
        };
        engine.rules = engine.load(rules)?;
        let order = spec.ordered();
        let n = order.len() as f64;
        for (i, f) in order.iter().enumerate() {
            engine.pos.insert(f.api_name.clone(), i as f64);
        }
        for f in spec.fields() {
            engine
                .pos
                .entry(f.api_name.clone())
                .or_insert(n + f.position.unwrap_or(0.0));
        }
        Ok(engine)
    }

    pub fn module(&self) -> Option<&str> {
        self.context.module.as_deref()
    }

    pub fn user_type(&self) -> &str {
        &self.context.user_type
    }

    pub fn item_id(&self) -> &Json {
        &self.context.item_id
    }

    pub fn user(&self) -> Option<&Map<String, Json>> {
        self.context.user.as_ref()
    }

    /// Warnings so far (loading and evaluation).
    pub fn warnings(&self) -> Vec<String> {
        self.warnings.lock().map(|w| w.clone()).unwrap_or_default()
    }

    fn warn(&self, msg: String) {
        if let Ok(mut w) = self.warnings.lock() {
            w.push(msg);
        }
    }

    fn warn_once(&self, msg: String) {
        if let Ok(mut w) = self.warnings.lock()
            && !w.contains(&msg)
        {
            w.push(msg);
        }
    }

    // -- loading ----------------------------------------------------------
    fn load(&mut self, data: Option<&Json>) -> Result<Vec<Rule>> {
        let raw: Vec<&Json> = match data {
            Some(Json::Object(m)) => {
                let v = match m.get("business_rules") {
                    Some(v) => v,
                    None => m.get("business_rule").unwrap_or(&NULL),
                };
                v.as_array().map(|a| a.iter().collect()).unwrap_or_default()
            }
            Some(Json::Array(a)) => a.iter().collect(),
            _ => vec![],
        };
        let listing = raw
            .iter()
            .filter(|r| {
                r.as_object().is_none_or(|m| {
                    !m.contains_key("condition_data") && !m.contains_key("action_data")
                })
            })
            .count();
        if listing > 0 {
            return Err(FormError::FormRule {
                message: format!(
                    "{listing} business rule(s) have no conditions or actions: this is the rules listing, not the \
                     form's rules. Fetch them with fsv.forms.business_rules_query(...) (ember_api=true)."
                ),
                rule_id: Box::new(Json::Null),
                rule_name: None,
                unsupported: false,
            });
        }
        let mut kept: Vec<&Map<String, Json>> = Vec::new();
        for r in raw.into_iter().filter_map(obj) {
            if let Some(why) = self.filter(r) {
                self.skipped.push(format!("{}: {why}", mget_str(r, "name")));
                continue;
            }
            kept.push(r);
        }
        let kept = self.dedupe_rules(kept);
        let mut out: Vec<Rule> = Vec::new();
        for r in kept {
            if let Some(rule) = self.parse_rule(r, out.len())? {
                out.push(rule);
            }
        }
        Ok(out)
    }

    fn filter(&self, r: &Map<String, Json>) -> Option<String> {
        let ctx = &self.context;
        if r.get("deleted").is_some_and(truthy) {
            return Some("deleted".to_string());
        }
        if r.get("state").map(jstr).as_deref().unwrap_or("active") != "active" {
            return Some(format!("state {}", mget_str(r, "state")));
        }
        if r.get("rule_type")
            .map(jstr)
            .as_deref()
            .unwrap_or("client_side")
            != "client_side"
        {
            return Some(format!(
                "rule_type {} (server-side rules are not evaluated)",
                mget_str(r, "rule_type")
            ));
        }
        if let Some(module) = ctx.module.as_deref().filter(|m| !m.is_empty())
            && let Some(mt) = mget(r, "module_type")
            && mt.as_str() != Some(module)
        {
            return Some(format!("module {}", jstr(mt)));
        }
        if let Some(item) = mget(r, "item_id")
            && item.as_str() != Some("")
            && jstr(item) != jstr(&ctx.item_id)
        {
            return Some(format!("item {}", jstr(item)));
        }
        if let Some(ws) = ctx.workspaces()
            && let Some(w) = mget(r, "workspace_id")
            && !ws.contains(&jstr(w))
        {
            return Some(format!("workspace {}", jstr(w)));
        }
        None
    }

    fn dedupe_rules<'a>(
        &mut self,
        rules: Vec<&'a Map<String, Json>>,
    ) -> Vec<&'a Map<String, Json>> {
        let mut seen: Vec<&Json> = Vec::new();
        for r in &rules {
            let Some(rid) = mget(r, "id") else { continue };
            if seen.contains(&rid) {
                self.duplicates.push(rid.clone());
            } else {
                seen.push(rid);
            }
        }
        if self.dedupe == Dedupe::None || self.duplicates.is_empty() {
            return rules;
        }
        let mut out: Vec<&Map<String, Json>> = Vec::new();
        if self.dedupe == Dedupe::Exact {
            for r in rules {
                let rid = mget(r, "id");
                if rid.is_some() && out.iter().any(|o| mget(o, "id") == rid && *o == r) {
                    continue;
                }
                out.push(r);
            }
            return out;
        }
        let mut position: Vec<(&Json, usize)> = Vec::new();
        for r in rules {
            let rid = mget(r, "id");
            match rid.and_then(|id| position.iter().find(|(k, _)| *k == id)) {
                None => {
                    if let Some(id) = rid {
                        position.push((id, out.len()));
                    }
                    out.push(r);
                }
                Some(&(_, at)) => {
                    if self.dedupe == Dedupe::Last {
                        out[at] = r;
                    }
                }
            }
        }
        out
    }

    fn parse_rule(&mut self, r: &Map<String, Json>, index: usize) -> Result<Option<Rule>> {
        let name_raw = match mget(r, "name").filter(|v| truthy(v)) {
            Some(n) => jstr(n),
            None => mget_str(r, "id"),
        };
        let name = strip(&name_raw).to_string();
        let empty = Json::Object(Map::new());
        let parsed = (|| -> std::result::Result<(ConditionGroup, Vec<Action>), Skip> {
            let raw_cond = mget(r, "condition_data")
                .filter(|v| truthy(v))
                .unwrap_or(&empty);
            let cond = self.parse_group(raw_cond, &name)?;
            let actions = match mget(r, "action_data").filter(|v| truthy(v)).and_then(obj) {
                Some(ad) => match mget(ad, "action").filter(|v| truthy(v)) {
                    Some(Json::Array(a)) => self.parse_actions(a, &name)?,
                    _ => vec![],
                },
                None => vec![],
            };
            Ok((cond, actions))
        })();
        let id = r.get("id").cloned().unwrap_or(Json::Null);
        let (cond, actions) = match parsed {
            Ok(p) => p,
            Err(Skip(e)) => {
                let msg = format!(
                    "form has a rule fsv doesn't understand: {} (id {}): {e}. Fill this form in the web UI, or \
                     evaluate with strict=False to skip the rule.",
                    fsv_py::py_repr_str(&name),
                    jstr(&id)
                );
                if self.strict {
                    return Err(FormError::FormRule {
                        message: msg,
                        rule_id: Box::new(id),
                        rule_name: Some(name),
                        unsupported: true,
                    });
                }
                self.unsupported.push(msg.clone());
                self.warn(msg);
                return Ok(None);
            }
        };
        let mut fields = BTreeSet::new();
        condition_fields(&cond, &mut fields);
        let unconditional = cond.items.is_empty();
        Ok(Some(Rule {
            id,
            name,
            condition: cond,
            actions,
            reversible: r.get("is_reversible").is_some_and(truthy),
            form_type: mget(r, "form_type")
                .filter(|v| truthy(v))
                .map(jstr)
                .unwrap_or_else(|| "both".to_string()),
            user_type: mget(r, "user_type")
                .filter(|v| truthy(v))
                .map(jstr)
                .unwrap_or_else(|| "both".to_string()),
            is_validation: r.get("is_validation").is_some_and(truthy),
            index,
            fields,
            unconditional,
        }))
    }

    fn parse_group(&self, data: &Json, rule: &str) -> std::result::Result<ConditionGroup, Skip> {
        let Some(m) = obj(data).filter(|m| !m.is_empty()) else {
            return Ok(ConditionGroup {
                mode: "all".to_string(),
                items: vec![],
            });
        };
        let modes: Vec<&str> = ["all", "any"]
            .into_iter()
            .filter(|k| m.contains_key(*k))
            .collect();
        if modes.len() != 1 {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            return Err(Skip(format!(
                "condition must have exactly one of all/any, got {}",
                repr_str_list(&keys)
            )));
        }
        let mode = modes[0];
        let mut items = Vec::new();
        let empty = Map::new();
        let list: &[Json] = match mget(m, mode) {
            Some(Json::Array(a)) => a,
            _ => &[],
        };
        for c in list {
            if let Some(cm) = obj(c)
                && (cm.contains_key("all") || cm.contains_key("any"))
                && !cm.contains_key("operator")
            {
                items.push(ConditionItem::Group(self.parse_group(c, rule)?));
                continue;
            }
            let c = obj(c).unwrap_or(&empty);
            let op = mget(c, "operator")
                .filter(|v| truthy(v))
                .map(jstr)
                .unwrap_or_default();
            if !SUPPORTED_OPERATORS.contains(&op.as_str()) {
                return Err(Skip(format!(
                    "unsupported operator {}",
                    fsv_py::py_repr_str(&op)
                )));
            }
            let ev_on = mget(c, "evaluate_on");
            let ev_key = ev_on.map(jstr).unwrap_or_default();
            if !ev_key.is_empty() && !supported_evaluate_on(&ev_key) {
                return Err(Skip(format!(
                    "unsupported evaluate_on {}",
                    jrepr(ev_on.unwrap_or(&NULL))
                )));
            }
            let raw_name = mget(c, "name")
                .filter(|v| truthy(v))
                .map(jstr)
                .unwrap_or_default();
            let vals: Vec<String> = match mget(c, "value") {
                Some(Json::Array(a)) => a.iter().map(jstr).collect(),
                None => vec![],
                Some(v) => vec![jstr(v)],
            };
            if let Some(source) = context_source(&ev_key) {
                let attr = raw_name.rsplit('.').next().unwrap_or("").to_string();
                items.push(ConditionItem::Condition(Condition {
                    name: raw_name,
                    field: Some(attr),
                    operator: op,
                    values: vals,
                    source: source.to_string(),
                }));
                continue;
            }
            let api = self.spec.resolve_name(&raw_name).map(str::to_string);
            if api.is_none() {
                self.warn(format!(
                    "rule {}: condition on unknown field {}",
                    fsv_py::py_repr_str(rule),
                    fsv_py::py_repr_str(&raw_name)
                ));
            }
            items.push(ConditionItem::Condition(Condition {
                name: raw_name,
                field: api,
                operator: op,
                values: vals,
                source: "form".to_string(),
            }));
        }
        Ok(ConditionGroup {
            mode: mode.to_string(),
            items,
        })
    }

    fn target(&self, raw: &str) -> Target {
        let api = self.spec.resolve_name(raw).map(str::to_string);
        let (mut stripped, mut had_prefix) = (raw, false);
        for p in CUSTOM_PREFIXES {
            if let Some(rest) = raw.strip_prefix(p) {
                (stripped, had_prefix) = (rest, true);
                break;
            }
        }
        if let Some(api) = api {
            let custom = self.spec.get(&api).is_some_and(|f| f.is_custom);
            if custom {
                return Target {
                    api: Some(api.clone()),
                    raw: raw.to_string(),
                    base: api,
                    custom: true,
                };
            }
            return Target {
                api: Some(api),
                raw: raw.to_string(),
                base: stripped.to_string(),
                custom: false,
            };
        }
        Target {
            api: None,
            raw: raw.to_string(),
            base: stripped.to_string(),
            custom: had_prefix,
        }
    }

    fn parse_actions(&self, data: &[Json], rule: &str) -> std::result::Result<Vec<Action>, Skip> {
        let mut out = Vec::new();
        let empty = Map::new();
        for a in data {
            let a = obj(a).unwrap_or(&empty);
            let prop = mget(a, "property")
                .filter(|v| truthy(v))
                .map(jstr)
                .unwrap_or_default();
            if !SUPPORTED_ACTIONS.contains(&prop.as_str()) {
                return Err(Skip(format!(
                    "unsupported action {}",
                    fsv_py::py_repr_str(&prop)
                )));
            }
            let names: Vec<Json> = match a.get("name") {
                Some(Json::String(s)) => vec![Json::String(s.clone())],
                Some(Json::Array(l)) => l.clone(),
                Some(Json::Object(o)) => o.keys().map(|k| Json::String(k.clone())).collect(),
                Some(v) if truthy(v) => vec![v.clone()],
                _ => vec![],
            };
            let vals_raw = match a.get("values") {
                Some(v) => v,
                None => a.get("value").unwrap_or(&NULL),
            };
            let vals: Vec<Json> = match vals_raw {
                Json::Array(l) => l.clone(),
                Json::Null => vec![],
                v => vec![v.clone()],
            };
            if prop == "message" {
                out.push(Action {
                    property: prop,
                    targets: names.iter().map(jstr).collect(),
                    values: vec![],
                    all_targets: vec![],
                });
                continue;
            }
            let mut targets = Vec::new();
            for n in &names {
                let t = self.target(&jstr(n));
                if t.api.is_none() && !PSEUDO_TARGETS.contains(&t.base.as_str()) {
                    self.warn(format!(
                        "rule {}: {prop} on unknown field {} ignored",
                        fsv_py::py_repr_str(rule),
                        jrepr(n)
                    ));
                }
                targets.push(t);
            }
            out.push(Action {
                property: prop,
                targets: targets.iter().filter_map(|t| t.api.clone()).collect(),
                values: vals,
                all_targets: targets,
            });
        }
        Ok(out)
    }

    // -- evaluation -------------------------------------------------------
    pub fn rules_for(&self, mode: &str) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|r| r.applies_to(mode, &self.context.user_type))
            .collect()
    }

    /// Unresolved fields or context needed by applicable rules.
    ///
    /// Inspects all branches, not just those reached by boolean evaluation.
    /// A present key with an empty value is known; a missing key is not.
    /// Callers preparing writes should treat any result as a blocker.
    pub fn unavailable_dependencies(&self, mode: &str) -> Vec<String> {
        let mut problems: Vec<String> = Vec::new();
        fn add(problems: &mut Vec<String>, m: String) {
            if !problems.contains(&m) {
                problems.push(m);
            }
        }
        fn inspect(
            engine: &RulesEngine,
            g: &ConditionGroup,
            rule: &Rule,
            problems: &mut Vec<String>,
        ) {
            for item in &g.items {
                match item {
                    ConditionItem::Group(sub) => inspect(engine, sub, rule, problems),
                    ConditionItem::Condition(c) if c.source == "form" => {
                        if c.field.is_none() {
                            add(
                                problems,
                                format!(
                                    "rule {}: unknown condition field {}",
                                    fsv_py::py_repr_str(&rule.name),
                                    fsv_py::py_repr_str(&c.name)
                                ),
                            );
                        }
                    }
                    ConditionItem::Condition(c) => {
                        let field = c.field.as_deref().unwrap_or("");
                        if engine
                            .context
                            .source(&c.source)
                            .is_none_or(|v| !v.contains_key(field))
                        {
                            add(
                                problems,
                                format!(
                                    "rule {}: unavailable {} context field {}",
                                    fsv_py::py_repr_str(&rule.name),
                                    c.source,
                                    fsv_py::py_repr_str(field)
                                ),
                            );
                        }
                    }
                }
            }
        }
        for rule in self.rules_for(mode) {
            inspect(self, &rule.condition, rule, &mut problems);
            for action in &rule.actions {
                for t in &action.all_targets {
                    if t.api.is_none() && !PSEUDO_TARGETS.contains(&t.base.as_str()) {
                        add(
                            &mut problems,
                            format!(
                                "rule {}: unknown action field {}",
                                fsv_py::py_repr_str(&rule.name),
                                fsv_py::py_repr_str(&t.raw)
                            ),
                        );
                    }
                }
            }
        }
        problems
    }

    /// Field names sorted in web form order (the replay contract).
    pub fn position_order<I, S>(&self, names: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out: Vec<String> = names.into_iter().map(|s| s.as_ref().to_string()).collect();
        let key = |n: &String| self.pos.get(n).copied().unwrap_or((1u64 << 30) as f64);
        out.sort_by(|a, b| {
            key(a)
                .partial_cmp(&key(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });
        out
    }

    /// Evaluate the rules (`evaluate(values, mode)` with defaults).
    pub fn evaluate(&self, values: &Values, mode: &str) -> Result<EvaluatedForm> {
        self.evaluate_with(values, mode, EvalOptions::default())
    }

    /// Evaluate the rules.
    ///
    /// `base` are the load values (defaults, or the record being edited);
    /// `values` the user's answers (API names). In the event model the
    /// answers are replayed as change events in `order` (default: web field
    /// order); names missing from `order` follow in field order. `events`
    /// replaces that with an explicit click history (repeats allowed); the
    /// replay model only uses its final values.
    pub fn evaluate_with(
        &self,
        values: &Values,
        mode: &str,
        opts: EvalOptions<'_>,
    ) -> Result<EvaluatedForm> {
        let model = opts.model.unwrap_or(self.model);
        let rules = self.rules_for(mode);
        let base: Values = opts
            .base
            .map(|b| {
                b.iter()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let st = match model {
            Model::Replay => {
                let mut user = base.clone();
                for (k, v) in values {
                    user.insert(k.clone(), v.clone());
                }
                user.retain(|_, v| !v.is_empty());
                let user_only: Vec<&String> = values
                    .iter()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(k, _)| k)
                    .collect();
                self.evaluate_replay(&rules, &user, &user_only)?
            }
            Model::Event => match opts.events {
                Some(events) => self.evaluate_events(&rules, base, events)?,
                None => {
                    let seq = self.event_order(values, opts.order);
                    let events: Vec<(String, Value)> = seq
                        .into_iter()
                        .map(|n| {
                            let v = values.get(&n).cloned().unwrap_or(Value::Null);
                            (n, v)
                        })
                        .collect();
                    self.evaluate_events(&rules, base, &events)?
                }
            },
        };
        self.finish(st, model)
    }

    fn event_order(&self, values: &Values, order: Option<&[String]>) -> Vec<String> {
        let Some(order) = order else {
            return self.position_order(values.keys());
        };
        let mut seen: Vec<String> = Vec::new();
        for n in order {
            if values.contains_key(n) && !seen.contains(n) {
                seen.push(n.clone());
            }
        }
        let rest = self.position_order(values.keys().filter(|n| !seen.contains(n)));
        seen.extend(rest);
        seen
    }

    // replay model -------------------------------------------------------
    fn evaluate_replay(
        &self,
        rules: &[&Rule],
        user: &Values,
        user_only: &[&String],
    ) -> Result<State> {
        let mut populated: Values = Values::new();
        let mut st: Option<State> = None;
        let mut settled = false;
        for _ in 0..MAX_PASSES {
            let mut eff = user.clone();
            for (k, v) in &populated {
                eff.entry(k.clone()).or_insert_with(|| v.clone());
            }
            let mut s = self.fresh(eff);
            for rule in rules {
                let ok = self.match_group(&rule.condition, &s.values)?;
                self.apply(rule, ok, &mut s, None, Some(user), None)?;
            }
            let new_pop: Values = s
                .populated
                .keys()
                .map(|k| (k.clone(), s.values.get(k).cloned().unwrap_or(Value::Null)))
                .collect();
            st = Some(s);
            if new_pop == populated {
                settled = true;
                break;
            }
            populated = new_pop;
        }
        if !settled {
            self.warn("populate rules did not settle; result may be unstable".to_string());
        }
        let mut st = st.expect("MAX_PASSES > 0");
        for n in user_only {
            if st.dis.contains(*n) && user.contains_key(*n) {
                let d = st.why.get(&((*n).clone(), "disabled".to_string())).cloned();
                st.refused.push(((*n).clone(), d));
            }
        }
        Ok(st)
    }

    // event model --------------------------------------------------------
    fn evaluate_events(
        &self,
        rules: &[&Rule],
        base: Values,
        events: &[(String, Value)],
    ) -> Result<State> {
        let mut st = self.fresh(base);
        for rule in rules {
            let ok = self.match_group(&rule.condition, &st.values)?;
            self.apply(rule, ok, &mut st, None, None, None)?;
        }
        let mut user_set: HashSet<String> = HashSet::new();
        for (name, value) in events {
            if st.dis.contains(name) {
                let d = st.why.get(&(name.clone(), "disabled".to_string())).cloned();
                st.refused.push((name.clone(), d));
                continue;
            }
            let same = same(st.values.get(name), Some(value));
            if value.is_empty() {
                st.values.shift_remove(name);
            } else {
                st.values.insert(name.clone(), value.clone());
            }
            user_set.insert(name.clone());
            st.overridden.shift_remove(name);
            if same && !self.same_value_fires {
                continue;
            }
            st.populated.shift_remove(name);
            let mut changed = self.clear_lower_levels(name, &mut st);
            changed.insert(name.clone());
            for rule in rules {
                if self.event_scope == EventScope::Referenced
                    && rule.fields.iter().all(|f| !changed.contains(f))
                {
                    continue;
                }
                let ok = self.match_group(&rule.condition, &st.values)?;
                if !ok
                    && self.event_reverse == EventReverse::Transition
                    && !st.last_match.contains_key(&rule.index)
                {
                    continue;
                }
                self.apply(rule, ok, &mut st, Some(name), None, Some(&user_set))?;
            }
        }
        Ok(st)
    }

    /// A nested parent changed: its lower levels are cleared (web behaviour).
    fn clear_lower_levels(&self, name: &str, st: &mut State) -> HashSet<String> {
        let mut cleared = HashSet::new();
        let mut f = self.spec.get(name);
        while let Some(cur) = f.filter(|f| f.kind == Kind::Nested) {
            let child = self.spec.fields().iter().find(|c| {
                c.parent.as_deref() == Some(cur.api_name.as_str()) && c.is_nested_level()
            });
            let Some(child) = child else { break };
            if st.values.contains_key(&child.api_name) {
                st.values.shift_remove(&child.api_name);
                st.populated.shift_remove(&child.api_name);
                cleared.insert(child.api_name.clone());
            }
            f = Some(child);
        }
        cleared
    }

    // shared -------------------------------------------------------------
    fn fresh(&self, values: Values) -> State {
        let mut vis = HashMap::new();
        let mut req = HashMap::new();
        for n in self.spec.names() {
            let f = self.spec.get(n).expect("indexed name");
            vis.insert(n.to_string(), f.visible_base);
            req.insert(n.to_string(), f.required_base);
        }
        State {
            vis,
            req,
            values,
            ..State::default()
        }
    }

    /// Apply `rule` (matched or not) to the state. `fixed`: replay model,
    /// values populate must not overwrite. `user_set`: event model, user
    /// answers so far (overwrites recorded).
    fn apply(
        &self,
        rule: &Rule,
        ok: bool,
        st: &mut State,
        event: Option<&str>,
        fixed: Option<&Values>,
        user_set: Option<&HashSet<String>>,
    ) -> Result<()> {
        if ok {
            st.last_match.insert(rule.index, rule.id.clone());
        } else {
            st.last_match.remove(&rule.index);
        }
        if rule.actions.iter().any(|a| a.property == "message") {
            st.messages.remove(&rule.index);
        }
        for a in &rule.actions {
            let prop = if ok {
                Some(a.property.as_str())
            } else if rule.reversible {
                reverse_of(&a.property)
            } else {
                None
            };
            let Some(prop) = prop else { continue };
            if self.populate == PopulateMode::Restrict && a.property == "populate" {
                self.restrict(rule, a, ok, st, event);
                continue;
            }
            if prop == "restore" {
                continue;
            }
            if prop == "message" {
                let msgs = a
                    .targets
                    .iter()
                    .map(|t| Message {
                        text: t.clone(),
                        rule_id: rule.id.clone(),
                        rule_name: rule.name.clone(),
                        blocking: rule.is_validation,
                    })
                    .collect();
                st.messages.insert(rule.index, msgs);
                continue;
            }
            if prop == "populate" {
                for t in &a.all_targets {
                    let Some(api) = &t.api else { continue };
                    if fixed.is_some_and(|f| f.contains_key(api)) {
                        continue;
                    }
                    let f = self.spec.get(api).expect("target resolved");
                    let Some(v) = self.populate_value(f, &a.values, rule)? else {
                        continue;
                    };
                    if user_set.is_some_and(|u| u.contains(api))
                        && !same(st.values.get(api), Some(&v))
                    {
                        let old = st.values.get(api).cloned().unwrap_or(Value::Null);
                        st.overridden.insert(api.clone(), (old, rule.id.clone()));
                    }
                    st.values.insert(api.clone(), v);
                    st.populated.insert(api.clone(), rule.id.clone());
                }
                continue;
            }
            let d = Decision {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                action: prop.to_string(),
                matched: ok,
                event: event.map(str::to_string),
                rule_index: rule.index as i64,
            };
            for t in &a.all_targets {
                let Some(n) = &t.api else {
                    if PSEUDO_TARGETS.contains(&t.base.as_str())
                        && (prop == "mandate" || prop == "unmandate")
                    {
                        st.pseudo_req.insert(t.base.clone(), prop == "mandate");
                        st.touch(prop, t);
                    }
                    continue;
                };
                st.touch(prop, t);
                match prop {
                    "show" => {
                        st.vis.insert(n.clone(), true);
                    }
                    "hide" => {
                        st.vis.insert(n.clone(), false);
                    }
                    "mandate" => {
                        st.req.insert(n.clone(), true);
                    }
                    "unmandate" => {
                        st.req.insert(n.clone(), false);
                    }
                    "disable" => {
                        st.dis.insert(n.clone());
                    }
                    "enable" => {
                        st.dis.remove(n);
                    }
                    _ => {}
                }
                let key = (n.clone(), kind_of(prop).to_string());
                st.why.insert(key.clone(), d.clone());
                if ok {
                    st.matched_decisions.entry(key).or_default().push(d.clone());
                }
            }
        }
        Ok(())
    }

    /// `populate="restrict"`: a matching rule limits the choices; a
    /// reversible rule that does not match restores them.
    fn restrict(&self, rule: &Rule, a: &Action, ok: bool, st: &mut State, event: Option<&str>) {
        let d = Decision {
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            action: if ok { "populate" } else { "restore" }.to_string(),
            matched: ok,
            event: event.map(str::to_string),
            rule_index: rule.index as i64,
        };
        for t in &a.all_targets {
            let Some(api) = &t.api else { continue };
            let f = self.spec.get(api).expect("target resolved");
            if !ok {
                st.restricted.shift_remove(api);
                st.restricted_by.insert(api.clone(), d.clone());
                continue;
            }
            if let Some(r) = self.restriction(f, &a.values, rule) {
                st.restricted.insert(api.clone(), r);
                st.restricted_by.insert(api.clone(), d.clone());
            }
        }
    }

    fn restriction(&self, f: &Field, values: &[Json], rule: &Rule) -> Option<ChoiceRestriction> {
        if !f.kind.is_choice() {
            self.warn(format!(
                "rule {}: populate on {}, which has no choices, ignored",
                fsv_py::py_repr_str(&rule.name),
                fsv_py::py_repr_str(&f.api_name)
            ));
            return None;
        }
        let allowed = self.build_restriction(f, rule, &f.choices, values);
        if allowed.is_empty() {
            if !values.is_empty() {
                self.warn(format!(
                    "rule {}: populate on {} offers no known choice; ignored",
                    fsv_py::py_repr_str(&rule.name),
                    fsv_py::py_repr_str(&f.api_name)
                ));
            }
            return None;
        }
        Some(ChoiceRestriction {
            allowed,
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
        })
    }

    fn build_restriction(
        &self,
        f: &Field,
        rule: &Rule,
        pool: &[Choice],
        vals: &[Json],
    ) -> IndexMap<String, Option<ChoiceRestriction>> {
        let mut out = IndexMap::new();
        for raw in vals {
            let mut kids: Option<&Json> = None;
            let mut raw = raw;
            if let Some(m) = raw.as_object() {
                kids = mget(m, "nested_options");
                raw = match m.get("display_id") {
                    Some(v) => v,
                    None => match m.get("id") {
                        Some(v) => v,
                        None => m.get("value").unwrap_or(&NULL),
                    },
                };
            }
            let Some(ch) = choice_in_pool(pool, raw) else {
                self.warn(format!(
                    "rule {}: populate value {} is not a choice of {}; ignored",
                    fsv_py::py_repr_str(&rule.name),
                    jrepr(raw),
                    fsv_py::py_repr_str(&f.api_name)
                ));
                continue;
            };
            let below = match kids.filter(|k| truthy(k)) {
                Some(k) => {
                    let kid_vals: Vec<Json> = match k {
                        Json::Array(a) => a.clone(),
                        Json::Object(o) => o.keys().map(|s| Json::String(s.clone())).collect(),
                        other => vec![other.clone()],
                    };
                    Some(ChoiceRestriction {
                        allowed: self.build_restriction(f, rule, &ch.children, &kid_vals),
                        rule_id: rule.id.clone(),
                        rule_name: rule.name.clone(),
                    })
                }
                None => None,
            };
            out.insert(norm(&ch.value), below);
        }
        out
    }

    fn finish(&self, st: State, model: Model) -> Result<EvaluatedForm> {
        let spec = &self.spec;
        let mut visible: Vec<String> = Vec::new();
        let mut shown: HashSet<String> = HashSet::new();
        let values = &st.values;
        for f in spec.ordered() {
            if !st.vis.get(&f.api_name).copied().unwrap_or(false) {
                continue;
            }
            if let Some(parent) = f.parent.as_deref().filter(|p| !p.is_empty()) {
                if !shown.contains(parent) {
                    continue;
                }
                if let Some((p, choices)) = spec.section_parent(&f.api_name) {
                    let cur = values.get(&p.api_name);
                    let items: Vec<&Value> = match cur {
                        Some(Value::List(l)) => l.iter().collect(),
                        Some(v) if !v.is_empty() => vec![v],
                        _ => vec![],
                    };
                    if !items.iter().any(|x| choices.contains(&x.norm())) {
                        continue;
                    }
                }
                if f.is_nested_level() && spec.choices_for(f, values)?.is_empty() {
                    continue;
                }
            }
            visible.push(f.api_name.clone());
            shown.insert(f.api_name.clone());
        }
        let required: BTreeSet<String> = visible
            .iter()
            .filter(|n| {
                st.req.get(*n).copied().unwrap_or(false)
                    && spec.get(n).is_some_and(|f| f.kind != Kind::Notice)
            })
            .cloned()
            .collect();
        let mut conflicts = Vec::new();
        for ((n, kind), ds) in &st.matched_decisions {
            let pos = ds.iter().any(|d| is_positive(&d.action));
            let neg = ds.iter().any(|d| !is_positive(&d.action));
            if pos && neg {
                let mut uniq: Vec<Decision> = Vec::new();
                for d in ds {
                    if uniq
                        .iter()
                        .all(|u| (&u.rule_id, &u.action) != (&d.rule_id, &d.action))
                    {
                        uniq.push(d.clone());
                    }
                }
                conflicts.push(Conflict {
                    field: n.clone(),
                    property: kind.clone(),
                    decisions: uniq,
                    winner: st.why[&(n.clone(), kind.clone())].clone(),
                });
            }
        }
        let messages: Vec<Message> = st.messages.values().flatten().cloned().collect();
        let excludes = self.excludes(&st);
        let mut unavailable = IndexMap::new();
        if !st.restricted.is_empty() {
            for n in &visible {
                let f = spec.get(n).expect("visible field");
                let Some(v) = values.get(n).filter(|v| !v.is_empty()) else {
                    continue;
                };
                if !f.kind.is_choice() {
                    continue;
                }
                let top = &nested_chain(spec, f)[0].api_name;
                if !st.restricted.contains_key(top) {
                    continue;
                }
                let pool = offered_choices(spec, f, values, &st.restricted)?;
                let items: Vec<&Value> = match v {
                    Value::List(l) => l.iter().collect(),
                    v => vec![v],
                };
                let mut missing = false;
                for x in items {
                    if f.choice_in(x, &pool)?.is_none() {
                        missing = true;
                        break;
                    }
                }
                if missing {
                    unavailable.insert(n.clone(), (v.clone(), st.restricted_by.get(top).cloned()));
                }
            }
        }
        Ok(EvaluatedForm {
            visible,
            required,
            disabled: st.dis.clone(),
            values: st.values.clone(),
            populated: st.populated.clone(),
            messages,
            matched: st.last_match.values().cloned().collect(),
            model,
            excludes,
            decisions: st.why.clone(),
            conflicts,
            refused: st.refused.clone(),
            overridden: st.overridden.clone(),
            attachments_required: st.pseudo_req.get("attachment_ids").copied(),
            account_id: spec.account_id.clone(),
            spec: spec.clone(),
            restricted: st.restricted.clone(),
            restricted_by: st.restricted_by.clone(),
            unavailable,
            populate_mode: self.populate,
        })
    }

    /// `br_validation_excludes` lists. Order: first time a rule action (or
    /// its reverse) touched the field's property. Membership: the final
    /// state. `optional` holds only statically required fields that the
    /// rules leave not required (those are the server checks to skip).
    fn excludes(&self, st: &State) -> Excludes {
        let spec = &self.spec;
        let mut hidden: Vec<ExcludeName> = st.reg[0]
            .iter()
            .filter(|(n, _)| !st.vis.get(*n).copied().unwrap_or(true))
            .map(|(_, x)| x.clone())
            .collect();
        let mut optional: Vec<ExcludeName> = Vec::new();
        for (n, x) in &st.reg[1] {
            let (stat, now) = if PSEUDO_TARGETS.contains(&n.as_str()) {
                let stat = spec.item.as_ref().is_some_and(|i| i.attachment_mandatory);
                (stat, st.pseudo_req.get(n).copied().unwrap_or(stat))
            } else {
                (
                    spec.get(n).is_some_and(|f| f.required_base),
                    st.req.get(n).copied().unwrap_or(false),
                )
            };
            if stat && !now {
                optional.push(x.clone());
            }
        }
        let disabled: Vec<ExcludeName> = st.reg[2]
            .iter()
            .filter(|(n, _)| st.dis.contains(*n))
            .map(|(_, x)| x.clone())
            .collect();
        let mut names_h: HashSet<ExcludeName> = hidden.iter().cloned().collect();
        let mut names_o: HashSet<ExcludeName> = optional.iter().cloned().collect();
        for n in st.reg[0].keys() {
            let Some(f) = spec.get(n) else { continue };
            if st.vis.get(n).copied().unwrap_or(true)
                || f.kind != Kind::Nested
                || !f.is_custom
                || f.is_nested_level()
            {
                continue;
            }
            for lvl in &f.levels {
                let Some(lf) = spec.get(lvl) else { continue };
                if !lf.on_form {
                    continue;
                }
                let key = ExcludeName {
                    base: lvl.clone(),
                    custom: true,
                };
                if !names_h.contains(&key) {
                    hidden.push(key.clone());
                    names_h.insert(key.clone());
                }
                if !names_o.contains(&key) && !st.req.get(lvl).copied().unwrap_or(false) {
                    optional.push(key.clone());
                    names_o.insert(key);
                }
            }
        }
        Excludes {
            optional,
            disabled,
            hidden,
        }
    }

    // -- matching ---------------------------------------------------------
    fn match_group(&self, g: &ConditionGroup, eff: &Values) -> Result<bool> {
        if g.items.is_empty() {
            return Ok(true);
        }
        let all = g.mode == "all";
        for item in &g.items {
            let r = match item {
                ConditionItem::Group(sub) => self.match_group(sub, eff)?,
                ConditionItem::Condition(c) => self.match_condition(c, eff)?,
            };
            if all && !r {
                return Ok(false);
            }
            if !all && r {
                return Ok(true);
            }
        }
        Ok(all)
    }

    fn person(&self, source: &str) -> Option<&Map<String, Json>> {
        let person = self.context.source(source);
        if person.is_none() {
            let what = if source == "system" {
                "system values (RuleContext.system)".to_string()
            } else {
                format!("the {source} record")
            };
            self.warn_once(format!(
                "rules read {what} but none was given; their conditions see empty values"
            ));
        }
        person
    }

    /// Port of fsv.forms.rules.RulesEngine._match.
    fn match_condition(&self, c: &Condition, eff: &Values) -> Result<bool> {
        let (f, v): (Option<&Field>, Option<Value>) = if c.source != "form" {
            let v = self
                .person(&c.source)
                .and_then(|p| p.get(c.field.as_deref().unwrap_or("")))
                .map(Value::from_json);
            (None, v)
        } else {
            let f = c.field.as_deref().and_then(|n| self.spec.get(n));
            let v = c.field.as_deref().and_then(|n| eff.get(n)).cloned();
            (f, v)
        };
        if c.operator == "selected" || c.operator == "not_selected" {
            let sel = if f.is_some_and(|f| f.kind == Kind::Checkbox) {
                matches!(v, Some(Value::Bool(true)))
            } else {
                !is_empty(v.as_ref()) && !matches!(v, Some(Value::Bool(false)))
            };
            return Ok(if c.operator == "selected" { sel } else { !sel });
        }
        let wanted: HashSet<String> = c.values.iter().map(|x| norm(x)).collect();
        let hit = self
            .value_keys(f, v.as_ref(), eff)?
            .iter()
            .any(|keys| keys.iter().any(|k| wanted.contains(k)));
        Ok(if c.operator == "is" || c.operator == "includes" {
            hit
        } else {
            !hit
        })
    }

    fn value_keys(
        &self,
        f: Option<&Field>,
        v: Option<&Value>,
        eff: &Values,
    ) -> Result<Vec<HashSet<String>>> {
        let Some(v) = v.filter(|v| !v.is_empty()) else {
            return Ok(vec![]);
        };
        let items: Vec<&Value> = match v {
            Value::List(l) => l.iter().collect(),
            v => vec![v],
        };
        let pool: &[Choice] = match f {
            Some(f) => self.spec.choices_for(f, eff)?,
            None => &[],
        };
        let mut out = Vec::new();
        for item in items {
            if let Value::Bool(b) = item {
                let keys: [&str; 3] = if *b {
                    ["true", "yes", "1"]
                } else {
                    ["false", "no", "0"]
                };
                out.push(keys.iter().map(|s| s.to_string()).collect());
                continue;
            }
            let mut keys = HashSet::from([item.norm()]);
            if let Value::Lookup(l) = item {
                if !l.id.is_null() && l.id.as_str() != Some("") {
                    keys.insert(norm(&jstr(&l.id)));
                }
                if !l.display.is_empty() {
                    keys.insert(norm(&l.display));
                }
                if let Some(e) = l.email.as_deref().filter(|e| !e.is_empty()) {
                    keys.insert(norm(e));
                }
            }
            if let Some(f) = f
                && !pool.is_empty()
                && let Some(ch) = f.choice_in(item, pool)?
            {
                keys.extend(ch.keys());
            }
            out.push(keys);
        }
        Ok(out)
    }

    fn populate_value(&self, f: &Field, values: &[Json], rule: &Rule) -> Result<Option<Value>> {
        if values.is_empty() {
            return Ok(None);
        }
        if matches!(
            f.kind,
            Kind::Dropdown | Kind::Department | Kind::Nested | Kind::MultiSelect
        ) {
            let mut picked: Vec<&Choice> = Vec::new();
            for raw in values {
                match choice_by_any(f, raw)? {
                    Some(ch) => picked.push(ch),
                    None => {
                        self.warn(format!(
                            "rule {}: populate value {} is not a choice of {}",
                            fsv_py::py_repr_str(&rule.name),
                            jrepr(raw),
                            fsv_py::py_repr_str(&f.api_name)
                        ));
                        return Ok(None);
                    }
                }
            }
            if f.kind == Kind::MultiSelect {
                return Ok(Some(Value::List(
                    picked.iter().map(|c| Value::Str(c.value.clone())).collect(),
                )));
            }
            return Ok(Some(Value::Str(picked[0].value.clone())));
        }
        if f.kind == Kind::Checkbox {
            let s = jstr(&values[0]).to_lowercase();
            return Ok(Some(Value::Bool(matches!(
                s.as_str(),
                "true" | "1" | "yes"
            ))));
        }
        Ok(Some(if values.len() == 1 {
            Value::from_json(&values[0])
        } else {
            Value::List(values.iter().map(Value::from_json).collect())
        }))
    }
}

/// Optional arguments of [`RulesEngine::evaluate_with`].
#[derive(Clone, Copy, Debug, Default)]
pub struct EvalOptions<'a> {
    pub base: Option<&'a Values>,
    pub order: Option<&'a [String]>,
    pub model: Option<Model>,
    pub events: Option<&'a [(String, Value)]>,
}

fn condition_fields(g: &ConditionGroup, out: &mut BTreeSet<String>) {
    for item in &g.items {
        match item {
            ConditionItem::Group(sub) => condition_fields(sub, out),
            ConditionItem::Condition(c) if c.source == "form" => {
                if let Some(f) = c.field.as_ref().filter(|f| !f.is_empty()) {
                    out.insert(f.clone());
                }
            }
            ConditionItem::Condition(_) => {}
        }
    }
}

/// `[top field, level 2, ...]` down to `f` (just `[f]` if not nested).
pub fn nested_chain<'a>(spec: &'a FormSpec, f: &'a Field) -> Vec<&'a Field> {
    let mut chain = vec![f];
    let mut cur = f;
    while cur.is_nested_level() {
        let Some(parent) = cur.parent.as_deref().and_then(|p| spec.get(p)) else {
            break;
        };
        cur = parent;
        chain.insert(0, cur);
    }
    chain
}

/// Choices `f` offers given the upper nested levels' values and the
/// `populate` restrictions in effect.
pub fn offered_choices(
    spec: &FormSpec,
    f: &Field,
    values: &Values,
    restricted: &IndexMap<String, ChoiceRestriction>,
) -> Result<Vec<Choice>> {
    let pool = spec.choices_for(f, values)?;
    let chain = nested_chain(spec, f);
    let mut node = restricted.get(&chain[0].api_name);
    for upper in &chain[..chain.len() - 1] {
        let Some(n) = node else {
            return Ok(pool.to_vec());
        };
        let v = values.get(&upper.api_name);
        let Some(v) = v.filter(|v| !v.is_empty()) else {
            return Ok(vec![]);
        };
        if !n.offers(v) {
            return Ok(vec![]);
        }
        node = n.below(v);
    }
    Ok(match node {
        None => pool.to_vec(),
        Some(n) => pool
            .iter()
            .filter(|c| n.offers(&c.value))
            .cloned()
            .collect(),
    })
}

/// Populate values are choice `display_id`s; fall back to id, then value/label.
pub fn choice_in_pool<'a>(pool: &'a [Choice], raw: &Json) -> Option<&'a Choice> {
    let key = norm(&jstr(raw));
    for attr in [0, 1] {
        for c in pool {
            let v = if attr == 0 { &c.display_id } else { &c.id };
            if let Some(v) = v.as_ref().filter(|v| !v.is_null())
                && norm(&jstr(v)) == key
            {
                return Some(c);
            }
        }
    }
    pool.iter()
        .find(|c| norm(&c.value) == key || norm(&c.label) == key)
}

/// Populate values are choice `display_id`s; fall back to id/value.
pub fn choice_by_any<'a>(f: &'a Field, raw: &Json) -> Result<Option<&'a Choice>> {
    if let Some(ch) = f.choice_by_display_id(raw) {
        return Ok(Some(ch));
    }
    let key = norm(&jstr(raw));
    if let Some(c) = f.choices.iter().find(|c| {
        c.id.as_ref()
            .is_some_and(|i| !i.is_null() && norm(&jstr(i)) == key)
    }) {
        return Ok(Some(c));
    }
    f.choice(raw)
}

/// `_same(a, b)`: whether setting `b` over `a` is "the same value" (no
/// change event).
pub fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
    let null = Value::Null;
    let a = a.unwrap_or(&null);
    let b = b.unwrap_or(&null);
    if a.is_empty() && b.is_empty() {
        return true;
    }
    if let (Value::List(x), Value::List(y)) = (a, b) {
        return x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.norm() == q.norm());
    }
    let a_bool = matches!(a, Value::Bool(_));
    let b_bool = matches!(b, Value::Bool(_));
    if a_bool || b_bool {
        return matches!((a, b), (Value::Bool(x), Value::Bool(y)) if x == y);
    }
    a == b || (!a.is_null() && !b.is_null() && a.type_tag() == b.type_tag() && a.norm() == b.norm())
}

#[derive(Debug, Default)]
struct State {
    vis: HashMap<String, bool>,
    req: HashMap<String, bool>,
    values: Values,
    dis: BTreeSet<String>,
    populated: IndexMap<String, Json>,
    messages: BTreeMap<usize, Vec<Message>>,
    last_match: BTreeMap<usize, Json>,
    why: IndexMap<(String, String), Decision>,
    matched_decisions: IndexMap<(String, String), Vec<Decision>>,
    /// visible, required, disabled: field -> server name, first-touch order.
    reg: [IndexMap<String, ExcludeName>; 3],
    pseudo_req: IndexMap<String, bool>,
    restricted: IndexMap<String, ChoiceRestriction>,
    restricted_by: IndexMap<String, Decision>,
    refused: Vec<(String, Option<Decision>)>,
    overridden: IndexMap<String, (Value, Json)>,
}

impl State {
    fn touch(&mut self, prop: &str, t: &Target) {
        let slot = match kind_of(prop) {
            "visible" => 0,
            "required" => 1,
            _ => 2,
        };
        let key = t.api.clone().unwrap_or_else(|| t.base.clone());
        self.reg[slot].entry(key).or_insert_with(|| ExcludeName {
            base: t.base.clone(),
            custom: t.custom,
        });
    }
}

#[cfg(test)]
mod py_tests;
