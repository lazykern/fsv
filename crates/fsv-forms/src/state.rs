//! Form state: the user's answers, validated, with rules re-evaluated after
//! every change (`fsv.forms.state`).
//!
//! The state keeps the *load* values (defaults on a new form, the record on
//! an edit form) apart from the user's answers. Every change re-runs the
//! engine from scratch: in the event model the answers are replayed as
//! change events (web field order by default, or the order they were given
//! with `event_order="input"`), so editing an earlier answer replays the
//! whole sequence with the new value.

use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Value as Json, json};

use crate::errors::{FormError, Result};
use crate::pydt::{date_dmy, date_fromisoformat, datetime_fromisoformat, offset_text};
use crate::pytext::{
    basename, casefold, decimal_value, is_digits, jstr, parse_int, split_ws, splitext_ext, strip,
    take_chars,
};
use crate::rules::{
    Conflict, Decision, EvalOptions, EvaluatedForm, Explanation, Message, RulesEngine, is_empty,
    offered_choices,
};
use crate::spec::{Choice, Field, FormSpec, Kind, plain_text};
use crate::tz::{TzEnv, Zone, localize, resolve_timezone_in};
pub use crate::value::LookupValue;
use crate::value::{Value, Values, norm};

/// How answers are replayed as change events.
///
/// * `Position`: in web field order (YAML / `--set` contract).
/// * `Input`: in the order they were first given.
/// * `History`: every `set()` call, repeats included (what a person
///   clicking through the web form does).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventOrder {
    #[default]
    Position,
    Input,
    History,
}

pub const EVENT_ORDERS: [&str; 3] = ["position", "input", "history"];

impl EventOrder {
    pub fn parse(s: &str) -> Result<EventOrder> {
        match s {
            "position" => Ok(EventOrder::Position),
            "input" => Ok(EventOrder::Input),
            "history" => Ok(EventOrder::History),
            _ => Err(FormError::Value(
                "event_order must be one of ('position', 'input', 'history')".to_string(),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        EVENT_ORDERS[self as usize]
    }
}

const TRUE_WORDS: [&str; 3] = ["yes", "true", "y"];
const FALSE_WORDS: [&str; 3] = ["no", "false", "n"];
static INT_RX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+-]?\d+$").expect("int regex"));
static ISO_DATE_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("date regex"));

/// Errors a [`LookupResolver`] may return (wrapped in [`FormError::Lookup`]).
pub type ResolverError = Box<dyn std::error::Error + Send + Sync>;

/// Injected by the caller (CLI/TUI); the forms crate does no HTTP.
///
/// `entity` is `user`, `group`, `agent` or the field's `lookup_entity`
/// (e.g. `change_window`). Return every candidate; the engine picks a
/// single or exact match, or fails with `AmbiguousLookup`.
pub trait LookupResolver: Send + Sync {
    /// Port of fsv.forms.state.LookupResolver.lookup.
    fn lookup(
        &self,
        entity: &str,
        query: &str,
        field: &Field,
    ) -> std::result::Result<Vec<LookupValue>, ResolverError>;
}

/// What changed in the form after a `set`/`unset`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diff {
    pub field: String,
    pub appeared: Vec<String>,
    pub disappeared: Vec<String>,
    pub became_required: Vec<String>,
    pub no_longer_required: Vec<String>,
    pub messages_added: Vec<Message>,
    pub messages_removed: Vec<Message>,
    pub populated: IndexMap<String, Value>,
    pub cleared: Vec<String>,
    /// `populate="restrict"`: field -> labels now on offer (restricted) ...
    pub restricted: IndexMap<String, Vec<String>>,
    /// ... and fields whose full choice list is back.
    pub unrestricted: Vec<String>,
}

impl Diff {
    /// Python `bool(diff)`: anything changed.
    pub fn any(&self) -> bool {
        !(self.appeared.is_empty()
            && self.disappeared.is_empty()
            && self.became_required.is_empty()
            && self.no_longer_required.is_empty()
            && self.messages_added.is_empty()
            && self.messages_removed.is_empty()
            && self.populated.is_empty()
            && self.cleared.is_empty()
            && self.restricted.is_empty()
            && self.unrestricted.is_empty())
    }

    /// Human-readable lines (labels, not API names).
    pub fn describe(&self, spec: &FormSpec) -> Vec<String> {
        let label = |n: &str| {
            spec.get(n)
                .map_or_else(|| n.to_string(), |f| f.label.clone())
        };
        let labels = |names: &[String]| {
            names
                .iter()
                .map(|n| label(n))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut out = Vec::new();
        if !self.appeared.is_empty() {
            let req: Vec<String> = self
                .appeared
                .iter()
                .filter(|n| self.became_required.contains(n))
                .cloned()
                .collect();
            let mut line = format!("added: {}", labels(&self.appeared));
            if !req.is_empty() {
                line.push_str(&format!(" (required: {})", labels(&req)));
            }
            out.push(line);
        }
        if !self.disappeared.is_empty() {
            out.push(format!("removed: {}", labels(&self.disappeared)));
        }
        let now_req: Vec<String> = self
            .became_required
            .iter()
            .filter(|n| !self.appeared.contains(n))
            .cloned()
            .collect();
        if !now_req.is_empty() {
            out.push(format!("now required: {}", labels(&now_req)));
        }
        if !self.no_longer_required.is_empty() {
            out.push(format!(
                "no longer required: {}",
                labels(&self.no_longer_required)
            ));
        }
        for (name, value) in &self.populated {
            out.push(format!("filled in: {} = {}", label(name), value.py_str()));
        }
        if !self.cleared.is_empty() {
            out.push(format!("cleared: {}", labels(&self.cleared)));
        }
        for (name, offered) in &self.restricted {
            let shown = if offered.is_empty() {
                "(none)".to_string()
            } else {
                offered.join(", ")
            };
            out.push(format!("choices limited: {} -> {shown}", label(name)));
        }
        if !self.unrestricted.is_empty() {
            out.push(format!(
                "all choices offered again: {}",
                labels(&self.unrestricted)
            ));
        }
        out.extend(
            self.messages_added
                .iter()
                .map(|m| format!("message: {}", m.text)),
        );
        out
    }
}

/// What [`FormState::acknowledge`] matches: a message, or a rule id /
/// message text.
#[derive(Clone, Debug)]
pub enum Ack {
    Message(Message),
    Text(String),
}

impl From<Message> for Ack {
    fn from(m: Message) -> Self {
        Ack::Message(m)
    }
}
impl From<&Message> for Ack {
    fn from(m: &Message) -> Self {
        Ack::Message(m.clone())
    }
}
impl From<&str> for Ack {
    fn from(s: &str) -> Self {
        Ack::Text(s.to_string())
    }
}
impl From<String> for Ack {
    fn from(s: String) -> Self {
        Ack::Text(s)
    }
}

fn message_repr(m: &Message) -> String {
    format!(
        "Message(text={}, rule_id={}, rule_name={}, blocking={})",
        fsv_py::py_repr_str(&m.text),
        fsv_py::py_repr(&m.rule_id),
        fsv_py::py_repr_str(&m.rule_name),
        if m.blocking { "True" } else { "False" }
    )
}

/// Keyword arguments of [`FormState::new`].
pub type FormStateOptions = StateOptions;

#[derive(Clone)]
pub struct StateOptions {
    pub resolver: Option<Arc<dyn LookupResolver>>,
    /// Explicit time zone for date-times typed without an offset.
    pub tz: Option<String>,
    pub apply_defaults: bool,
    /// The record being edited (API names or labels).
    pub existing: Option<Values>,
    pub event_order: EventOrder,
    /// Environment for time-zone resolution (`None`: the real process).
    pub tz_env: Option<TzEnv>,
}

impl Default for StateOptions {
    fn default() -> Self {
        StateOptions {
            resolver: None,
            tz: None,
            apply_defaults: true,
            existing: None,
            event_order: EventOrder::Position,
            tz_env: None,
        }
    }
}

/// User answers + the evaluated form.
///
/// `values` holds only what the user (or a default) set; rule-populated
/// values live in `evaluated.values` with provenance in
/// `evaluated.populated`.
#[derive(Clone)]
pub struct FormState {
    pub spec: Arc<FormSpec>,
    pub rules: RulesEngine,
    /// `"new"` | `"edit"`.
    pub mode: String,
    pub resolver: Option<Arc<dyn LookupResolver>>,
    pub event_order: EventOrder,
    pub tz: Zone,
    pub tz_source: String,
    pub values: Values,
    /// `"default"` | `"existing"` | `"user"`.
    pub sources: IndexMap<String, String>,
    /// Load values (defaults / the record being edited): the state the
    /// rules see on load, before any change event.
    pub base: Values,
    pub evaluated: EvaluatedForm,
    pub acknowledged: HashSet<(String, String)>,
    seq: Vec<String>,
    cleared: BTreeSet<String>,
    events: Vec<(String, Value)>,
}

type Snapshot = (
    Values,
    IndexMap<String, String>,
    Vec<String>,
    BTreeSet<String>,
    Vec<(String, Value)>,
);

fn null() -> Value {
    Value::Null
}

/// Python `str(raw)` of a raw answer (lists print as lists).
fn raw_str(v: &Value) -> String {
    match v {
        Value::List(_) => v.raw_repr(),
        other => other.py_str(),
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "str",
        Value::List(_) => "list",
        Value::Lookup(_) => "LookupValue",
        Value::Date(_) => "datetime.date",
        Value::DateTime(_) => "datetime.datetime",
        Value::Json(Json::Null) => "NoneType",
        Value::Json(Json::Bool(_)) => "bool",
        Value::Json(Json::Number(n)) if n.is_f64() => "float",
        Value::Json(Json::Number(_)) => "int",
        Value::Json(Json::String(_)) => "str",
        Value::Json(Json::Array(_)) => "list",
        Value::Json(Json::Object(_)) => "dict",
    }
}

/// `list(raw)` for a non-string raw answer.
pub(crate) fn py_list(v: &Value) -> Result<Vec<Value>> {
    match v {
        Value::List(items) => Ok(items.clone()),
        Value::Json(Json::Array(items)) => Ok(items.iter().map(Value::from_json).collect()),
        Value::Json(Json::Object(m)) => Ok(m.keys().map(|k| Value::Str(k.clone())).collect()),
        other => Err(FormError::Type(format!(
            "'{}' object is not iterable",
            type_name(other)
        ))),
    }
}

fn split_commas(s: &str) -> Vec<Value> {
    s.split(',')
        .map(strip)
        .filter(|p| !p.is_empty())
        .map(Value::from)
        .collect()
}

fn merged(a: &Values, b: &Values) -> Values {
    let mut out = a.clone();
    for (k, v) in b {
        out.insert(k.clone(), v.clone());
    }
    out
}

fn why(d: Option<&Decision>) -> String {
    d.map(|d| format!(" ({})", d.describe()))
        .unwrap_or_default()
}

fn by_rule(d: Option<&Decision>, with_id: bool) -> String {
    match d {
        Some(d) if with_id => format!(
            " by rule {} (id {})",
            fsv_py::py_repr_str(&d.rule_name),
            jstr(&d.rule_id)
        ),
        Some(d) => format!(" by rule {}", fsv_py::py_repr_str(&d.rule_name)),
        None => String::new(),
    }
}

fn visible_labels(choices: &[Choice]) -> Vec<String> {
    choices
        .iter()
        .filter(|c| c.visible)
        .map(|c| c.label.clone())
        .collect()
}

impl FormState {
    /// `FormState(spec, rules, mode, values, **options)`. `rules`: `None`
    /// means no business rules.
    pub fn new(
        spec: Arc<FormSpec>,
        rules: Option<RulesEngine>,
        mode: &str,
        values: Option<&Values>,
        opts: StateOptions,
    ) -> Result<FormState> {
        if mode != "new" && mode != "edit" {
            return Err(FormError::Value("mode must be 'new' or 'edit'".to_string()));
        }
        let rules = match rules {
            Some(r) => r,
            None => RulesEngine::new(spec.clone(), Some(&json!([])), Default::default())?,
        };
        let env = opts.tz_env.clone().unwrap_or_else(TzEnv::from_process);
        let (tz, tz_source) = resolve_timezone_in(opts.tz.as_deref(), rules.user(), &env)?;
        let mut vals = Values::new();
        let mut sources = IndexMap::new();
        if opts.apply_defaults && mode == "new" {
            for f in spec.fields() {
                if let Some(d) = f.default.as_ref().filter(|d| !d.is_null()) {
                    vals.insert(f.api_name.clone(), d.clone());
                    sources.insert(f.api_name.clone(), "default".to_string());
                }
            }
        }
        for (k, v) in opts.existing.iter().flatten() {
            let f = spec.field(k)?;
            if !v.is_empty() {
                vals.insert(f.api_name.clone(), v.clone());
                sources.insert(f.api_name.clone(), "existing".to_string());
            }
        }
        let base = vals.clone();
        let evaluated = rules.evaluate_with(
            &Values::new(),
            mode,
            EvalOptions {
                base: Some(&base),
                ..Default::default()
            },
        )?;
        let mut state = FormState {
            spec: spec.clone(),
            rules,
            mode: mode.to_string(),
            resolver: opts.resolver,
            event_order: opts.event_order,
            tz,
            tz_source,
            values: vals,
            sources,
            base,
            evaluated,
            acknowledged: HashSet::new(),
            seq: Vec::new(),
            cleared: BTreeSet::new(),
            events: Vec::new(),
        };
        // parse initial answers in form order so nested/section children
        // are validated after their parents
        let mut given = Values::new();
        for (k, v) in values.into_iter().flatten() {
            given.insert(spec.field(k)?.api_name.clone(), v.clone());
        }
        for name in state.rules.position_order(given.keys()) {
            let Some(f) = spec.get(&name) else { continue };
            let raw = given.get(&name).cloned().unwrap_or(Value::Null);
            let parsed = state.parse_field(f, &raw)?;
            state.store(f, parsed, "user");
            state.evaluated = state.evaluate()?;
        }
        state.check_refused()?;
        Ok(state)
    }

    // -- editing ----------------------------------------------------------
    fn evaluate(&self) -> Result<EvaluatedForm> {
        let mut user: Values = self
            .values
            .iter()
            .filter(|(k, _)| self.sources.get(*k).map(String::as_str) == Some("user"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for k in &self.cleared {
            user.insert(k.clone(), Value::Null);
        }
        let opts = EvalOptions {
            base: Some(&self.base),
            order: (self.event_order == EventOrder::Input).then_some(self.seq.as_slice()),
            events: (self.event_order == EventOrder::History).then_some(self.events.as_slice()),
            model: None,
        };
        self.rules.evaluate_with(&user, &self.mode, opts)
    }

    fn label(&self, name: &str) -> String {
        self.spec
            .get(name)
            .map_or_else(|| name.to_string(), |f| f.label.clone())
    }

    fn check_refused(&self) -> Result<()> {
        if let Some((name, d)) = self.evaluated.refused.first() {
            return Err(FormError::FieldDisabled(format!(
                "{} is disabled by a business rule{}; it cannot be set",
                self.label(name),
                why(d.as_ref())
            )));
        }
        Ok(())
    }

    fn snapshot(&self) -> Snapshot {
        (
            self.values.clone(),
            self.sources.clone(),
            self.seq.clone(),
            self.cleared.clone(),
            self.events.clone(),
        )
    }

    fn restore(&mut self, s: Snapshot) {
        (
            self.values,
            self.sources,
            self.seq,
            self.cleared,
            self.events,
        ) = s;
    }

    /// Parse/validate `raw` for the field and re-evaluate the rules.
    pub fn set(&mut self, name: &str, raw: impl Into<Value>) -> Result<Diff> {
        let raw = raw.into();
        let spec = self.spec.clone();
        let f = spec.field(name)?;
        if self.evaluated.disabled.contains(&f.api_name) {
            let d = self.evaluated.decision(&f.api_name, "disabled");
            return Err(FormError::FieldDisabled(format!(
                "{} is disabled by a business rule{}",
                f.label,
                why(d)
            )));
        }
        let parsed = self.parse_field(f, &raw)?;
        let snapshot = self.snapshot();
        let old = self.evaluated.values.get(&f.api_name).cloned();
        self.store(f, parsed, "user");
        let cleared = self.clear_lower_levels(f, old.as_ref());
        let after = match self.evaluate() {
            Ok(after) => after,
            Err(e) => {
                self.restore(snapshot);
                return Err(e);
            }
        };
        if let Some((name0, d)) = after.refused.first() {
            self.restore(snapshot);
            return Err(FormError::FieldDisabled(format!(
                "setting {} disables {}, which you also set{}",
                f.label,
                self.label(name0),
                why(d.as_ref())
            )));
        }
        let before = std::mem::replace(&mut self.evaluated, after);
        let mut diff = diff(&f.api_name, &before, &self.evaluated)?;
        diff.cleared = cleared;
        Ok(diff)
    }

    pub fn unset(&mut self, name: &str) -> Result<Diff> {
        self.set(name, Value::Null)
    }

    fn store(&mut self, f: &Field, value: Value, source: &str) {
        let name = f.api_name.clone();
        let user = source == "user";
        if user {
            let ev = if value.is_empty() {
                Value::Null
            } else {
                value.clone()
            };
            self.events.push((name.clone(), ev));
        }
        if value.is_empty() {
            self.values.shift_remove(&name);
            self.sources.shift_remove(&name);
            if user && self.base.contains_key(&name) {
                self.cleared.insert(name.clone()); // clearing a load value is a change event
                if !self.seq.contains(&name) {
                    self.seq.push(name);
                }
            } else {
                self.seq.retain(|n| *n != name);
            }
        } else {
            self.values.insert(name.clone(), value);
            self.sources.insert(name.clone(), source.to_string());
            self.cleared.remove(&name);
            if user && !self.seq.contains(&name) {
                self.seq.push(name);
            }
        }
    }

    /// A nested parent changed value: clear every lower level (web
    /// behaviour), even when a same-named child would still fit.
    fn clear_lower_levels(&mut self, f: &Field, old: Option<&Value>) -> Vec<String> {
        if f.kind != Kind::Nested || same_value(old, self.values.get(&f.api_name)) {
            return vec![];
        }
        let spec = self.spec.clone();
        let mut cleared = Vec::new();
        let mut cur = f;
        while let Some(child) = spec
            .fields()
            .iter()
            .find(|c| c.parent.as_deref() == Some(cur.api_name.as_str()) && c.is_nested_level())
        {
            if self.values.contains_key(&child.api_name) {
                self.store(child, Value::Null, "user");
                cleared.push(child.api_name.clone());
            }
            cur = child;
        }
        cleared
    }

    // -- acknowledgements -------------------------------------------------
    /// Blocking (`is_validation`) messages not acknowledged yet.
    pub fn unacknowledged(&self) -> Vec<Message> {
        self.evaluated
            .blocking_messages()
            .into_iter()
            .filter(|m| !self.acknowledged.contains(&m.key()))
            .cloned()
            .collect()
    }

    /// Acknowledge every current blocking message.
    pub fn acknowledge_all(&mut self) {
        let keys: Vec<_> = self
            .evaluated
            .blocking_messages()
            .iter()
            .map(|m| m.key())
            .collect();
        self.acknowledged.extend(keys);
    }

    /// Acknowledge blocking messages (by message, rule id, or message
    /// text); an empty list acknowledges every current one.
    pub fn acknowledge(&mut self, messages: &[Ack]) -> Result<()> {
        if messages.is_empty() {
            self.acknowledge_all();
            return Ok(());
        }
        for want in messages {
            let hits: Vec<(String, String)> = self
                .evaluated
                .blocking_messages()
                .into_iter()
                .filter(|m| match want {
                    Ack::Message(w) => {
                        w == *m || [jstr(&m.rule_id), m.text.clone()].contains(&message_repr(w))
                    }
                    Ack::Text(t) => *t == jstr(&m.rule_id) || *t == m.text,
                })
                .map(Message::key)
                .collect();
            if hits.is_empty() {
                let shown = match want {
                    Ack::Message(m) => message_repr(m),
                    Ack::Text(t) => fsv_py::py_repr_str(t),
                };
                return Err(FormError::Value(format!(
                    "no blocking message matches {shown}"
                )));
            }
            self.acknowledged.extend(hits);
        }
        Ok(())
    }

    // -- parsing ----------------------------------------------------------
    /// Validate `raw` for a field (API name or label) and return the stored value.
    pub fn parse(&self, name: &str, raw: &Value) -> Result<Value> {
        let spec = self.spec.clone();
        self.parse_field(spec.field(name)?, raw)
    }

    /// [`FormState::parse`] for a field already looked up.
    pub fn parse_field(&self, f: &Field, raw: &Value) -> Result<Value> {
        let blank = match raw {
            Value::Str(s) => strip(s).is_empty(),
            Value::Json(Json::String(s)) => strip(s).is_empty(),
            Value::List(l) => l.is_empty(),
            Value::Json(Json::Array(a)) => a.is_empty(),
            other => other.is_null(),
        };
        if blank {
            return Ok(Value::Null);
        }
        match f.kind {
            Kind::Notice => Err(FormError::parse(
                &f.label,
                raw,
                "is an information block and cannot be set",
            )),
            Kind::Text | Kind::Textarea | Kind::Richtext => Ok(Value::Str(raw_str(raw))),
            Kind::Number => parse_number(f, raw),
            Kind::Checkbox => parse_bool(f, raw).map(Value::Bool),
            Kind::Dropdown | Kind::Department | Kind::Nested => {
                Ok(Value::Str(self.parse_choice(f, raw)?.value))
            }
            Kind::MultiSelect => Ok(Value::List(
                self.parse_multi(f, raw)?
                    .into_iter()
                    .map(|c| Value::Str(c.value))
                    .collect(),
            )),
            Kind::Date => parse_date(f, raw).map(Value::Date),
            Kind::Datetime => parse_datetime(f, raw, &self.tz),
            Kind::LookupUser | Kind::LookupGroup | Kind::LookupAgent | Kind::Lookup => {
                self.parse_lookup(f, raw)
            }
            Kind::Attachment => parse_attachments(f, raw),
            Kind::Unsupported => Err(FormError::parse(
                &f.label,
                raw,
                format!(
                    "unsupported field kind {}",
                    fsv_py::py_repr_str(f.kind.as_str())
                ),
            )),
        }
    }

    fn answers(&self) -> Values {
        merged(&self.evaluated.values, &self.values)
    }

    fn pool(&self, f: &Field) -> Result<Vec<Choice>> {
        let pool = self.spec.choices_for(f, &self.answers())?.to_vec();
        if f.is_nested_level() && pool.is_empty() {
            let parent = self.spec.get(f.parent.as_deref().unwrap_or(""));
            let what = parent.map_or("the parent level", |p| p.label.as_str());
            return Err(FormError::parse(
                &f.label,
                &Value::Null,
                format!("choose {what} first"),
            ));
        }
        Ok(pool)
    }

    /// Choices the field offers now (nested levels and `populate`
    /// restrictions applied); hidden choices included, check `.visible`.
    pub fn choices(&self, name: &str) -> Result<Vec<Choice>> {
        let f = self.spec.field(name)?;
        offered_choices(&self.spec, f, &self.answers(), &self.evaluated.restricted)
    }

    fn parse_choice(&self, f: &Field, raw: &Value) -> Result<Choice> {
        let pool = self.pool(f)?;
        let ch = match f.choice_in(raw, &pool)? {
            Some(ch) if ch.visible => ch.clone(),
            _ => {
                let opts = visible_labels(&pool);
                let mut shown = opts.iter().take(15).cloned().collect::<Vec<_>>().join(", ");
                if opts.len() > 15 {
                    shown.push_str(&format!(", … ({} total)", opts.len()));
                }
                let msg = format!(
                    "{} is not a valid choice; choose one of: {shown}",
                    raw.raw_repr()
                );
                return Err(FormError::parse_opts(&f.label, raw, msg, opts));
            }
        };
        let offered = offered_choices(&self.spec, f, &self.answers(), &self.evaluated.restricted)?;
        if f.choice_in(&ch.value, &offered)?.is_none() {
            let opts = visible_labels(&offered);
            let (_, d) = self.evaluated.restriction_for(&f.api_name)?;
            let allowed = if opts.is_empty() {
                "nothing (choose another upper level first)".to_string()
            } else {
                opts.join(", ")
            };
            let msg = format!(
                "{} is not offered now: the choices are limited{} to: {allowed}",
                fsv_py::py_repr_str(&ch.label),
                by_rule(d, true)
            );
            return Err(FormError::parse_opts(&f.label, raw, msg, opts));
        }
        Ok(ch)
    }

    fn parse_multi(&self, f: &Field, raw: &Value) -> Result<Vec<Choice>> {
        let pool = self.pool(f)?;
        let parts = match raw.as_str() {
            Some(s) if f.choice_in(s, &pool)?.is_some() => vec![raw.clone()],
            Some(s) => split_commas(s),
            None => py_list(raw)?,
        };
        let mut out: Vec<Choice> = Vec::new();
        for p in &parts {
            let ch = self.parse_choice(f, p)?;
            if !out.contains(&ch) {
                out.push(ch);
            }
        }
        Ok(out)
    }

    fn parse_lookup(&self, f: &Field, raw: &Value) -> Result<Value> {
        if f.multi {
            let items = match raw.as_str() {
                Some(s) => split_commas(s),
                None => py_list(raw)?,
            };
            let mut out: Vec<LookupValue> = Vec::new();
            for it in &items {
                let lv = self.lookup_one(f, it)?;
                if !out.contains(&lv) {
                    out.push(lv);
                }
            }
            return Ok(Value::List(out.into_iter().map(Value::Lookup).collect()));
        }
        let single = match raw {
            Value::List(items) if items.len() == 1 => &items[0],
            Value::List(_) => return Err(FormError::parse(&f.label, raw, "takes a single value")),
            other => other,
        };
        self.lookup_one(f, single).map(Value::Lookup)
    }

    fn lookup_one(&self, f: &Field, raw: &Value) -> Result<LookupValue> {
        // Custom record ids must be verified against their metadata endpoint;
        // a numeric draft answer is not a trusted selection receipt.
        let verify = matches!(
            f.field_type.as_str(),
            "custom_lookup" | "custom_multi_lookup"
        ) && f.lookup_link.as_deref().is_some_and(|l| !l.is_empty());
        let id_value;
        let raw = match raw {
            Value::Lookup(lv) if !verify => return Ok(lv.clone()),
            Value::Lookup(lv) => {
                id_value = Value::from_json(&lv.id);
                &id_value
            }
            other => other,
        };
        if let Value::Bool(_) | Value::Json(Json::Bool(_)) = raw {
            return Err(FormError::parse(
                &f.label,
                raw,
                "expects a name, email or id",
            ));
        }
        let numeric: Option<Json> = match raw {
            Value::Int(i) => Some(json!(*i)),
            Value::Json(Json::Number(n)) if n.is_i64() || n.is_u64() => {
                Some(Json::Number(n.clone()))
            }
            _ => raw
                .as_str()
                .filter(|s| is_digits(strip(s)))
                .and_then(|s| parse_int(strip(s)))
                .map(Json::from),
        };
        if let Some(id) = &numeric
            && !verify
        {
            return Ok(LookupValue::id(id.clone()));
        }
        let query = strip(&raw_str(raw)).to_string();
        let Some(resolver) = &self.resolver else {
            return Err(FormError::parse(
                &f.label,
                raw,
                "needs a lookup resolver (or pass a numeric id)",
            ));
        };
        let entity = match f.kind {
            Kind::LookupUser => "user",
            Kind::LookupGroup => "group",
            Kind::LookupAgent => "agent",
            _ => f
                .lookup_entity
                .as_deref()
                .filter(|e| !e.is_empty())
                .unwrap_or("lookup"),
        };
        let mut found = resolver
            .lookup(entity, &query, f)
            .map_err(FormError::Lookup)?;
        if verify && let Some(id) = &numeric {
            let id = fsv_py::py_str(id);
            found.retain(|c| fsv_py::py_str(&c.id) == id);
        }
        if found.len() == 1 {
            return Ok(found.remove(0));
        }
        let q = norm(&query);
        let exact: Vec<LookupValue> = found
            .iter()
            .filter(|c| {
                q == norm(&c.display)
                    || q == norm(c.email.as_deref().unwrap_or(""))
                    || q == norm(&c.to_string())
            })
            .cloned()
            .collect();
        if let [one] = exact.as_slice() {
            return Ok(one.clone());
        }
        if found.is_empty() {
            return Err(FormError::parse(
                &f.label,
                raw,
                format!("no match for {}", fsv_py::py_repr_str(&query)),
            ));
        }
        let candidates = if exact.is_empty() { found } else { exact };
        Err(FormError::AmbiguousLookup {
            field: f.label.clone(),
            query,
            candidates,
        })
    }

    // -- queries ----------------------------------------------------------
    /// Effective value (user, default or populated).
    pub fn get(&self, name: &str) -> Result<Option<&Value>> {
        let f = self.spec.field(name)?;
        Ok(self.evaluated.values.get(&f.api_name))
    }

    pub fn is_populated(&self, name: &str) -> Result<bool> {
        Ok(self
            .evaluated
            .populated
            .contains_key(&self.spec.field(name)?.api_name))
    }

    pub fn visible_fields(&self) -> Vec<&Field> {
        self.evaluated
            .visible
            .iter()
            .filter_map(|n| self.spec.get(n))
            .collect()
    }

    pub fn messages(&self) -> &[Message] {
        &self.evaluated.messages
    }

    pub fn missing_required(&self) -> Vec<&Field> {
        self.visible_fields()
            .into_iter()
            .filter(|f| self.evaluated.required.contains(&f.api_name))
            .filter(|f| {
                let v = self.evaluated.values.get(&f.api_name);
                if f.kind == Kind::Checkbox {
                    !matches!(v, Some(Value::Bool(true)))
                } else {
                    is_empty(v)
                }
            })
            .collect()
    }

    pub fn unsupported_errors(&self) -> Vec<String> {
        self.spec
            .fields()
            .iter()
            .filter(|f| f.kind == Kind::Unsupported)
            .filter(|f| {
                (self.evaluated.is_visible(&f.api_name)
                    && self.evaluated.required.contains(&f.api_name))
                    || !is_empty(self.evaluated.values.get(&f.api_name))
            })
            .map(|f| {
                format!(
                    "{}: unsupported field semantics ({}); write blocked",
                    f.label, f.field_type
                )
            })
            .collect()
    }

    /// Everything that would stop the web form from submitting (blocking
    /// messages count until acknowledged).
    pub fn errors(&self) -> Result<Vec<String>> {
        let mut out: Vec<String> = self
            .missing_required()
            .iter()
            .map(|f| format!("{} is required", f.label))
            .collect();
        out.extend(self.unsupported_errors());
        out.extend(self.unavailable_errors()?);
        out.extend(self.blocked_lines());
        Ok(out)
    }

    pub(crate) fn blocked_lines(&self) -> Vec<String> {
        self.unacknowledged()
            .iter()
            .map(|m| format!("blocked by rule (acknowledge to continue): {}", m.text))
            .collect()
    }

    /// Visible fields whose value a `populate` restriction no longer
    /// offers (e.g. answered before the rule limited the choices).
    pub fn unavailable_errors(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for (name, (v, d)) in &self.evaluated.unavailable {
            let Some(f) = self.spec.get(name) else {
                continue;
            };
            let opts = visible_labels(&self.evaluated.choices(name)?);
            let shown = if opts.is_empty() {
                "(none)".to_string()
            } else {
                opts.join(", ")
            };
            out.push(format!(
                "{}: {} is not offered any more; the choices are limited{} to: {shown}",
                f.label,
                fsv_py::py_repr_str(&display_value(&self.spec, f, v, &self.evaluated.values)?),
                by_rule(d.as_ref(), true)
            ));
        }
        Ok(out)
    }

    /// Fields holding a value that will NOT be sent because they are
    /// hidden: (field, value, the hide decision if a rule hid it).
    pub fn dropped(&self) -> Vec<(&Field, Value, Option<Decision>)> {
        let vis: HashSet<&str> = self.evaluated.visible.iter().map(String::as_str).collect();
        self.spec
            .ordered()
            .into_iter()
            .filter(|f| !vis.contains(f.api_name.as_str()) && f.kind != Kind::Notice)
            .filter_map(|f| {
                let v = self
                    .evaluated
                    .values
                    .get(&f.api_name)
                    .filter(|v| !v.is_empty())?;
                Some((
                    f,
                    v.clone(),
                    self.evaluated.decision(&f.api_name, "visible").cloned(),
                ))
            })
            .collect()
    }

    pub fn explain(&self, names: Option<&[&str]>) -> Result<Vec<Explanation>> {
        self.evaluated.explain(names)
    }

    pub fn conflicts(&self) -> &[Conflict] {
        &self.evaluated.conflicts
    }

    /// Effective values of visible input fields, in form order. Hidden
    /// fields' values are dropped.
    pub fn payload_values(&self) -> Values {
        self.visible_fields()
            .into_iter()
            .filter(|f| f.kind != Kind::Notice)
            .filter_map(|f| {
                let v = self
                    .evaluated
                    .values
                    .get(&f.api_name)
                    .filter(|v| !v.is_empty())?;
                Some((f.api_name.clone(), v.clone()))
            })
            .collect()
    }

    /// Display text for a field's value (`value`: `None` = its effective value).
    pub fn display(&self, name: &str, value: Option<&Value>) -> Result<String> {
        let f = self.spec.field(name)?;
        let nul = null();
        let v = match value.filter(|v| !matches!(v, Value::Null)) {
            Some(v) => v,
            None => self.evaluated.values.get(&f.api_name).unwrap_or(&nul),
        };
        display_value(&self.spec, f, v, &self.evaluated.values)
    }

    /// Ordered (label, display value) rows for visible filled fields,
    /// followed by ("Message"/"Blocked", text) rows.
    pub fn to_review(&self) -> Result<Vec<(String, String)>> {
        let ev = &self.evaluated;
        let mut rows = Vec::new();
        for (name, v) in &self.payload_values() {
            let Some(f) = self.spec.get(name) else {
                continue;
            };
            let mut text = display_value(&self.spec, f, v, &ev.values)?;
            if ev.populated.contains_key(name) {
                text.push_str(" (auto)");
            }
            if !f.sent {
                text.push_str(" (not sent)");
            }
            rows.push((f.label.clone(), text));
        }
        for (name, (old, rid)) in &ev.overridden {
            let Some(f) = self.spec.get(name) else {
                continue;
            };
            let shown = display_value(&self.spec, f, old, &ev.values)?;
            rows.push((
                "Replaced".to_string(),
                format!(
                    "{}: your value {} was replaced by rule {} (populate)",
                    f.label,
                    fsv_py::py_repr_str(&shown),
                    jstr(rid)
                ),
            ));
        }
        for (name, (v, d)) in &ev.unavailable {
            let Some(f) = self.spec.get(name) else {
                continue;
            };
            let opts = visible_labels(&ev.choices(name)?).join(", ");
            rows.push((
                "Not offered".to_string(),
                format!(
                    "{} = {} (choices limited{}; pick one of: {opts})",
                    f.label,
                    display_value(&self.spec, f, v, &ev.values)?,
                    by_rule(d.as_ref(), false)
                ),
            ));
        }
        for (f, v, d) in self.dropped() {
            let why = match &d {
                Some(d) if d.action == "hide" => {
                    format!("hidden by rule {}", fsv_py::py_repr_str(&d.rule_name))
                }
                _ => "hidden".to_string(),
            };
            rows.push((
                "Dropped".to_string(),
                format!(
                    "{} = {} ({why}; not sent)",
                    f.label,
                    display_value(&self.spec, f, &v, &ev.values)?
                ),
            ));
        }
        for m in &ev.messages {
            let kind = if !m.blocking {
                "Message"
            } else if self.acknowledged.contains(&m.key()) {
                "Acknowledged"
            } else {
                "Blocked"
            };
            rows.push((kind.to_string(), m.text.clone()));
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// helpers

fn opt_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    let n = Value::Null;
    a.unwrap_or(&n) == b.unwrap_or(&n)
}

fn sort_by_index(names: impl IntoIterator<Item = String>, order: &[String]) -> Vec<String> {
    let mut out: Vec<String> = names.into_iter().collect();
    out.sort_by_key(|n| order.iter().position(|x| x == n).unwrap_or(1 << 30));
    out
}

fn diff(name: &str, before: &EvaluatedForm, after: &EvaluatedForm) -> Result<Diff> {
    let bv: HashSet<&String> = before.visible.iter().collect();
    let av: HashSet<&String> = after.visible.iter().collect();
    let mut d = Diff {
        field: name.to_string(),
        ..Diff::default()
    };
    d.appeared = after
        .visible
        .iter()
        .filter(|n| !bv.contains(n))
        .cloned()
        .collect();
    d.disappeared = before
        .visible
        .iter()
        .filter(|n| !av.contains(n))
        .cloned()
        .collect();
    d.became_required = sort_by_index(
        after.required.difference(&before.required).cloned(),
        &after.visible,
    );
    d.no_longer_required = sort_by_index(
        before.required.difference(&after.required).cloned(),
        &before.visible,
    );
    let key = |m: &Message| (m.rule_id.clone(), m.text.clone());
    let bm: Vec<_> = before.messages.iter().map(key).collect();
    let am: Vec<_> = after.messages.iter().map(key).collect();
    d.messages_added = after
        .messages
        .iter()
        .filter(|m| !bm.contains(&key(m)))
        .cloned()
        .collect();
    d.messages_removed = before
        .messages
        .iter()
        .filter(|m| !am.contains(&key(m)))
        .cloned()
        .collect();
    for (n, rid) in &after.populated {
        if before.populated.get(n) != Some(rid)
            || !opt_eq(before.values.get(n), after.values.get(n))
        {
            d.populated.insert(
                n.clone(),
                after.values.get(n).cloned().unwrap_or(Value::Null),
            );
        }
    }
    for (n, r) in &after.restricted {
        if before.restricted.get(n) != Some(r) {
            d.restricted
                .insert(n.clone(), visible_labels(&after.choices(n)?));
        }
    }
    d.unrestricted = before
        .restricted
        .keys()
        .filter(|n| !after.restricted.contains_key(*n))
        .cloned()
        .collect();
    Ok(d)
}

/// Python `float(s)` for a stripped string (underscores between digits,
/// Unicode decimal digits, `inf`/`nan`).
fn py_float(s: &str) -> Option<f64> {
    let chars: Vec<char> = s.chars().collect();
    let mut ascii = String::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() {
        if *c == '_' {
            let digit = |j: Option<usize>| {
                j.and_then(|j| chars.get(j))
                    .is_some_and(|c| decimal_value(*c).is_some())
            };
            if !digit(i.checked_sub(1)) || !digit(Some(i + 1)) {
                return None;
            }
            continue;
        }
        match decimal_value(*c) {
            Some(d) => ascii.push(char::from(b'0' + d as u8)),
            None if c.is_ascii() => ascii.push(*c),
            None => return None,
        }
    }
    ascii.parse::<f64>().ok()
}

fn parse_number(f: &Field, raw: &Value) -> Result<Value> {
    match raw {
        Value::Bool(_) | Value::Json(Json::Bool(_)) => {
            return Err(FormError::parse(&f.label, raw, "expects a number"));
        }
        Value::Int(_) | Value::Float(_) => return Ok(raw.clone()),
        Value::Json(n @ Json::Number(_)) => return Ok(Value::from_json(n)),
        _ => {}
    }
    let s = strip(&raw_str(raw)).replace(',', "");
    if INT_RX.is_match(&s)
        && let Some(i) = parse_int(&s)
    {
        return Ok(Value::Int(i));
    }
    py_float(&s).map(Value::Float).ok_or_else(|| {
        FormError::parse(&f.label, raw, format!("{} is not a number", raw.raw_repr()))
    })
}

fn parse_bool(f: &Field, raw: &Value) -> Result<bool> {
    match raw {
        Value::Bool(b) | Value::Json(Json::Bool(b)) => return Ok(*b),
        _ => {}
    }
    let s = casefold(strip(&raw_str(raw)));
    if TRUE_WORDS.contains(&s.as_str()) {
        return Ok(true);
    }
    if FALSE_WORDS.contains(&s.as_str()) {
        return Ok(false);
    }
    Err(FormError::parse_opts(
        &f.label,
        raw,
        format!("{} is not yes/no/true/false", raw.raw_repr()),
        vec!["yes".to_string(), "no".to_string()],
    ))
}

fn parse_date(f: &Field, raw: &Value) -> Result<jiff::civil::Date> {
    match raw {
        Value::DateTime(d) => return Ok(d.date()),
        Value::Date(d) => return Ok(*d),
        _ => {}
    }
    date_fromisoformat(strip(&raw_str(raw))).ok_or_else(|| {
        FormError::parse(
            &f.label,
            raw,
            format!("{} is not a date (YYYY-MM-DD)", raw.raw_repr()),
        )
    })
}

fn same_value(a: Option<&Value>, b: Option<&Value>) -> bool {
    if is_empty(a) && is_empty(b) {
        return true;
    }
    match (a.filter(|v| !v.is_null()), b.filter(|v| !v.is_null())) {
        (Some(a), Some(b)) => a.norm() == b.norm(),
        _ => false,
    }
}

fn parse_datetime(f: &Field, raw: &Value, tz: &Zone) -> Result<Value> {
    let (local, offset) = match raw {
        Value::DateTime(v) => return Ok(Value::DateTime(*v)),
        Value::Date(_) => {
            return Err(FormError::parse(
                &f.label,
                raw,
                "needs a time as well as a date",
            ));
        }
        _ => {
            let s = raw_str(raw);
            let s = strip(&s);
            if ISO_DATE_RX.is_match(s) {
                return Err(FormError::parse(
                    &f.label,
                    raw,
                    "needs a time as well as a date (YYYY-MM-DD HH:MM)",
                ));
            }
            datetime_fromisoformat(s).ok_or_else(|| {
                FormError::parse(
                    &f.label,
                    raw,
                    format!(
                        "{} is not a date-time (YYYY-MM-DD HH:MM, optional +HH:MM offset)",
                        raw.raw_repr()
                    ),
                )
            })?
        }
    };
    match offset {
        Some(off) => Ok(Value::DateTime(crate::pydt::DateTimeValue::new(local, off))),
        // refuse daylight-saving gaps and overlaps instead of guessing
        None => localize(local, tz)
            .map(Value::DateTime)
            .map_err(|e| FormError::parse(&f.label, raw, e)),
    }
}

fn parse_attachments(f: &Field, raw: &Value) -> Result<Value> {
    let items = match raw {
        Value::Str(_)
        | Value::Int(_)
        | Value::Bool(_)
        | Value::Json(Json::String(_) | Json::Bool(_)) => vec![raw.clone()],
        Value::Json(Json::Number(n)) if !n.is_f64() => vec![raw.clone()],
        other => py_list(other)?,
    };
    if let Some(max) = f.max_files
        && items.len() as i64 > max
    {
        return Err(FormError::parse(
            &f.label,
            raw,
            format!("accepts at most {max} file(s)"),
        ));
    }
    if !f.allowed_file_types.is_empty() {
        let allowed: BTreeSet<String> = f
            .allowed_file_types
            .iter()
            .map(|t| {
                if t.starts_with('.') {
                    t.to_lowercase()
                } else {
                    format!(".{}", t.to_lowercase())
                }
            })
            .collect();
        for i in &items {
            if let Some(s) = i.as_str()
                && !allowed.contains(&splitext_ext(s).to_lowercase())
            {
                let list = allowed.iter().cloned().collect::<Vec<_>>().join(", ");
                return Err(FormError::parse(
                    &f.label,
                    raw,
                    format!("{}: allowed file types are {list}", fsv_py::py_repr_str(s)),
                ));
            }
        }
    }
    Ok(Value::List(items))
}

/// Items of a value Python iterates (`for x in v`): a list, or a string's characters.
fn iter_items(v: &Value) -> Result<Vec<Value>> {
    match v.as_str() {
        Some(s) => Ok(s.chars().map(|c| Value::Str(c.to_string())).collect()),
        None => py_list(v),
    }
}

/// The text a review shows for a field's value.
pub fn display_value(spec: &FormSpec, f: &Field, v: &Value, values: &Values) -> Result<String> {
    if v.is_empty() {
        return Ok(String::new());
    }
    match (f.kind, v) {
        (Kind::Dropdown | Kind::Department | Kind::Nested, _) => {
            let pool = spec.choices_for(f, values)?;
            Ok(f.choice_in(v, pool)?
                .map_or_else(|| v.py_str(), |c| c.label.clone()))
        }
        (Kind::MultiSelect, _) => {
            let mut labels = Vec::new();
            for x in iter_items(v)? {
                labels.push(
                    f.choice(&x)?
                        .map_or_else(|| x.py_str(), |c| c.label.clone()),
                );
            }
            Ok(labels.join(", "))
        }
        (Kind::Checkbox, _) => Ok(if v.truthy() { "Yes" } else { "No" }.to_string()),
        (Kind::Date, Value::Date(d)) => Ok(date_dmy(*d)),
        (Kind::Date, Value::DateTime(d)) => Ok(date_dmy(d.date())),
        (Kind::Datetime, Value::DateTime(d)) => Ok(format!(
            "{} {:02}:{:02} ({})",
            date_dmy(d.date()),
            d.local.hour(),
            d.local.minute(),
            offset_text(d.offset)
        )),
        (_, Value::List(items)) => Ok(items
            .iter()
            .map(|x| match x.as_str() {
                Some(s) => basename(s).to_string(),
                None => x.py_str(),
            })
            .collect::<Vec<_>>()
            .join(", ")),
        (Kind::Textarea | Kind::Richtext, _) => {
            let text = split_ws(&plain_text(&v.py_str()))
                .collect::<Vec<_>>()
                .join(" ");
            if text.chars().count() <= 120 {
                Ok(text)
            } else {
                Ok(format!("{}…", take_chars(&text, 119)))
            }
        }
        _ => Ok(v.py_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Choice;

    fn spec() -> Arc<FormSpec> {
        let mut prio = Field::new("priority", "Priority", Kind::Dropdown);
        prio.choices = vec![Choice::new("Low", "Low"), Choice::new("High", "High")];
        let mut ok = Field::new("ok", "Approved", Kind::Checkbox);
        ok.required_base = true;
        let fields = vec![
            Field::new("subject", "Subject", Kind::Text),
            prio,
            Field::new("count", "Count", Kind::Number),
            ok,
            Field::new("when", "When", Kind::Datetime),
            Field::new("day", "Day", Kind::Date),
            Field::new("owner", "Owner", Kind::LookupUser),
        ];
        Arc::new(FormSpec::new("change", fields, None, vec![], None))
    }

    fn state() -> FormState {
        let opts = StateOptions {
            tz: Some("Asia/Bangkok".into()),
            tz_env: Some(TzEnv::empty()),
            ..Default::default()
        };
        FormState::new(spec(), None, "new", None, opts).expect("state")
    }

    struct People;
    impl LookupResolver for People {
        fn lookup(
            &self,
            _: &str,
            query: &str,
            _: &Field,
        ) -> std::result::Result<Vec<LookupValue>, ResolverError> {
            let all = vec![
                LookupValue::new(1, "Ann Lee", Some("ann@x.io")),
                LookupValue::new(2, "Ann Leeds", Some("leeds@x.io")),
            ];
            Ok(all
                .into_iter()
                .filter(|p| p.display.to_lowercase().contains(&query.to_lowercase()))
                .collect())
        }
    }

    #[test]
    fn parse_values() {
        let s = state();
        assert_eq!(
            s.parse("count", &"1,200".into()).expect("n"),
            Value::Int(1200)
        );
        assert_eq!(
            s.parse("count", &"1.5e2".into()).expect("n"),
            Value::Float(150.0)
        );
        assert_eq!(s.parse("count", &"  ".into()).expect("n"), Value::Null);
        assert_eq!(
            s.parse("count", &"abc".into()).unwrap_err().to_string(),
            "Count: 'abc' is not a number"
        );
        assert_eq!(
            s.parse("Approved", &"Yes".into()).expect("b"),
            Value::Bool(true)
        );
        assert_eq!(
            s.parse("ok", &"maybe".into()).unwrap_err().to_string(),
            "Approved: 'maybe' is not yes/no/true/false"
        );
        assert_eq!(
            s.parse("priority", &"high".into()).expect("c"),
            Value::str("High")
        );
        assert_eq!(
            s.parse("priority", &"mid".into()).unwrap_err().to_string(),
            "Priority: 'mid' is not a valid choice; choose one of: Low, High"
        );
        let when = s.parse("when", &"2026-11-26 19:00".into()).expect("dt");
        assert_eq!(when.py_str(), "2026-11-26 19:00:00+07:00");
        assert_eq!(
            s.parse("when", &"2026-11-26".into())
                .unwrap_err()
                .to_string(),
            "When: needs a time as well as a date (YYYY-MM-DD HH:MM)"
        );
        assert_eq!(
            s.parse("day", &"2026-11-26".into()).expect("d").py_str(),
            "2026-11-26"
        );
        assert_eq!(
            s.parse("owner", &"42".into()).expect("id"),
            Value::Lookup(LookupValue::id(42))
        );
        assert_eq!(
            s.parse("owner", &"ann".into()).unwrap_err().to_string(),
            "Owner: needs a lookup resolver (or pass a numeric id)"
        );
    }

    #[test]
    fn lookups_with_resolver() {
        let opts = StateOptions {
            tz: Some("UTC".into()),
            resolver: Some(Arc::new(People)),
            tz_env: Some(TzEnv::empty()),
            ..Default::default()
        };
        let s = FormState::new(spec(), None, "new", None, opts).expect("state");
        let got = s.parse("owner", &"ann lee".into()).expect("exact");
        assert_eq!(
            got,
            Value::Lookup(LookupValue::new(1, "Ann Lee", Some("ann@x.io")))
        );
        let err = s.parse("owner", &"ann".into()).unwrap_err();
        assert!(
            matches!(err, FormError::AmbiguousLookup { ref candidates, .. } if candidates.len() == 2)
        );
        assert_eq!(
            s.parse("owner", &"bob".into()).unwrap_err().to_string(),
            "Owner: no match for 'bob'"
        );
    }

    #[test]
    fn set_errors_review_and_payload_values() {
        let mut s = state();
        assert_eq!(
            s.errors().expect("errors"),
            vec!["Approved is required".to_string()]
        );
        s.set("subject", "Patch DB").expect("set");
        s.set("priority", "low").expect("set");
        s.set("ok", true).expect("set");
        assert!(s.errors().expect("errors").is_empty());
        let pv = s.payload_values();
        assert_eq!(
            pv.keys().collect::<Vec<_>>(),
            vec!["subject", "priority", "ok"]
        );
        let rows = s.to_review().expect("review");
        assert_eq!(rows[1], ("Priority".to_string(), "Low".to_string()));
        assert_eq!(rows[2], ("Approved".to_string(), "Yes".to_string()));
        s.unset("subject").expect("unset");
        assert!(s.get("subject").expect("get").is_none());
    }

    #[test]
    fn options_validated() {
        let err = FormState::new(spec(), None, "view", None, StateOptions::default())
            .err()
            .expect("err");
        assert_eq!(err.to_string(), "mode must be 'new' or 'edit'");
        assert_eq!(
            EventOrder::parse("random").unwrap_err().to_string(),
            "event_order must be one of ('position', 'input', 'history')"
        );
    }
}

#[cfg(test)]
mod py_tests;
