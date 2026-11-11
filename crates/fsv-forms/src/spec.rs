//! Form specification: the fields a Freshservice form shows, built from the
//! same metadata the web UI uses (`fsv.forms.spec`).
//!
//! Two sources: module form fields (`GET /api/_/change_form_fields`,
//! `/api/_/ticket_form_fields`, the `/api/v2/*_form_fields` variants) via
//! [`FormSpec::from_form_fields`], and a service-catalog item
//! (`GET /api/_/catalog/service-items/<id>`) via [`FormSpec::from_service_item`].
//! The spec is pure data: no HTTP, no rules.

use std::collections::HashSet;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value as Json};

use crate::errors::{FormError, Result};
use crate::pytext::{
    get, get_str, get_truthy, has, html_unescape, is_digits, jrepr, jstr, squash_ws, strip, truthy,
};
use crate::value::{PyObj, Value, Values, norm};

/// Field kinds understood by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Text,
    Textarea,
    Richtext,
    Number,
    Checkbox,
    Dropdown,
    MultiSelect,
    Date,
    Datetime,
    LookupUser,
    LookupGroup,
    LookupAgent,
    /// Any other lookup entity (e.g. maintenance window).
    Lookup,
    Department,
    Nested,
    Attachment,
    Notice,
    Unsupported,
}

/// `KINDS`.
pub const KINDS: [Kind; 18] = [
    Kind::Text,
    Kind::Textarea,
    Kind::Richtext,
    Kind::Number,
    Kind::Checkbox,
    Kind::Dropdown,
    Kind::MultiSelect,
    Kind::Date,
    Kind::Datetime,
    Kind::LookupUser,
    Kind::LookupGroup,
    Kind::LookupAgent,
    Kind::Lookup,
    Kind::Department,
    Kind::Nested,
    Kind::Attachment,
    Kind::Notice,
    Kind::Unsupported,
];

impl Kind {
    /// The Python kind string (`"multi_select"`, `"lookup_user"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Textarea => "textarea",
            Kind::Richtext => "richtext",
            Kind::Number => "number",
            Kind::Checkbox => "checkbox",
            Kind::Dropdown => "dropdown",
            Kind::MultiSelect => "multi_select",
            Kind::Date => "date",
            Kind::Datetime => "datetime",
            Kind::LookupUser => "lookup_user",
            Kind::LookupGroup => "lookup_group",
            Kind::LookupAgent => "lookup_agent",
            Kind::Lookup => "lookup",
            Kind::Department => "department",
            Kind::Nested => "nested",
            Kind::Attachment => "attachment",
            Kind::Notice => "notice",
            Kind::Unsupported => "unsupported",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        KINDS.iter().copied().find(|k| k.as_str() == s)
    }

    /// `CHOICE_KINDS`: dropdown, multi_select, department, nested.
    pub fn is_choice(self) -> bool {
        matches!(
            self,
            Kind::Dropdown | Kind::MultiSelect | Kind::Department | Kind::Nested
        )
    }

    /// `LOOKUP_KINDS`.
    pub fn is_lookup(self) -> bool {
        matches!(
            self,
            Kind::LookupUser | Kind::LookupGroup | Kind::LookupAgent | Kind::Lookup
        )
    }

    /// Single-choice kinds (dropdown, department, nested).
    pub fn is_single_choice(self) -> bool {
        matches!(self, Kind::Dropdown | Kind::Department | Kind::Nested)
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `CHOICE_KINDS`.
pub const CHOICE_KINDS: [Kind; 4] = [
    Kind::Dropdown,
    Kind::MultiSelect,
    Kind::Department,
    Kind::Nested,
];
/// `LOOKUP_KINDS`.
pub const LOOKUP_KINDS: [Kind; 4] = [
    Kind::LookupUser,
    Kind::LookupGroup,
    Kind::LookupAgent,
    Kind::Lookup,
];
/// `INPUTLESS_KINDS`.
pub const INPUTLESS_KINDS: [Kind; 1] = [Kind::Notice];

/// Content-only block field types (static notes, guides).
pub const CONTENT_FIELD_TYPES: [&str; 8] = [
    "custom_content",
    "content",
    "nested_content",
    "default_content",
    "custom_static_rich_text",
    "static_rich_text",
    "custom_static_text",
    "static_content",
];

static CONTENT_TYPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|_)(content|static)(_|$)").expect("content regex"));

/// A form-field type that is a static content block, not an input.
pub fn is_content_type(field_type: &str) -> bool {
    let ft = field_type.to_lowercase();
    CONTENT_FIELD_TYPES.contains(&ft.as_str()) || CONTENT_TYPE.is_match(&ft)
}

fn default_kind(ftype: &str) -> Option<Kind> {
    Some(match ftype {
        "default_requester" => Kind::LookupUser,
        "default_department" => Kind::Department,
        "default_group" => Kind::LookupGroup,
        "default_agent" => Kind::LookupAgent,
        "default_subject" => Kind::Text,
        "default_description" => Kind::Richtext,
        "default_planned_start_date" | "default_planned_end_date" => Kind::Datetime,
        "default_change_window" => Kind::Lookup,
        "default_category" => Kind::Nested,
        "default_planned_effort" => Kind::Text,
        "default_change_type"
        | "default_impact"
        | "default_priority"
        | "default_risk"
        | "default_status"
        | "default_urgency"
        | "default_source"
        | "default_ticket_type" => Kind::Dropdown,
        "default_requested_for" => Kind::LookupUser,
        "default_change_reason"
        | "default_change_impact"
        | "default_change_plan"
        | "default_backout_plan" => Kind::Richtext,
        _ => return None,
    })
}

fn custom_kind(ftype: &str) -> Option<Kind> {
    Some(match ftype {
        "custom_text" => Kind::Text,
        "custom_paragraph" => Kind::Textarea,
        "custom_number" | "custom_decimal" => Kind::Number,
        "custom_checkbox" => Kind::Checkbox,
        "custom_dropdown" => Kind::Dropdown,
        "custom_multi_select_dropdown" => Kind::MultiSelect,
        "custom_date" => Kind::Datetime,
        "custom_content" => Kind::Notice,
        "nested_field" => Kind::Nested,
        "custom_lookup" | "custom_multi_lookup" => Kind::LookupUser,
        "planning_field" => Kind::Richtext,
        _ => return None,
    })
}

/// Default dropdowns the web pre-selects with their first visible choice.
pub const FIRST_CHOICE_DEFAULT_TYPES: [&str; 7] = [
    "default_change_type",
    "default_impact",
    "default_priority",
    "default_risk",
    "default_urgency",
    "default_status",
    "default_source",
];

/// "Request for": the user the record is for. The web change form always
/// renders it though the metadata does not list it; its create/update key
/// is not verified, so it is display-only (`Field::sent` false).
pub const REQUESTED_FOR: &str = "requested_for";
pub const REQUESTED_FOR_TYPE: &str = "default_requested_for";
pub const REQUESTED_FOR_LABEL: &str = "Request for";
pub const REQUESTED_FOR_MODULES: [&str; 1] = ["change"];
/// Field types whose request key is unknown: never sent.
pub const UNSENT_TYPES: [&str; 1] = [REQUESTED_FOR_TYPE];

/// Change planning fields: not on the create form, edited through
/// `/changes/<id>/planning-fields`. Their names differ between accounts for
/// the same label, so callers resolve them by label, never by name.
pub const PLANNING_TYPES: [&str; 5] = [
    "planning_field",
    "default_change_reason",
    "default_change_impact",
    "default_change_plan",
    "default_backout_plan",
];

pub fn is_planning_type(field_type: &str) -> bool {
    PLANNING_TYPES.contains(&field_type)
}

const USER_LOOKUP_SOURCE: i64 = 5;

/// Service-catalog ("formserv") field type code -> kind.
pub fn sr_type_kind(code: i64) -> Option<Kind> {
    Some(match code {
        501 | 502 => Kind::Lookup,
        1001 => Kind::Text,
        1003 | 1019 => Kind::Dropdown,
        1006 => Kind::Checkbox,
        1007 => Kind::Date,
        1008 => Kind::Textarea,
        1014 => Kind::Attachment,
        1021 => Kind::Number,
        1024 => Kind::Notice,
        _ => return None,
    })
}

fn name_alias(n: &str) -> Option<&'static str> {
    Some(match n {
        "department_id" => "department",
        "group_id" => "group",
        "agent_id" | "responder_id" => "agent",
        "requester_id" => "requester",
        "change_window_id" => "change_window",
        "type" => "ticket_type",
        "description_html" => "description",
        _ => return None,
    })
}

/// Rule/DOM custom-field prefixes (`customFields.x`, `customField.sr_x`, ...).
pub const CUSTOM_PREFIXES: [&str; 4] = [
    "customFields.",
    "customField.",
    "custom_fields.",
    "custom_field.",
];

static LOOKUP_FIELD_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"field_name=([A-Za-z0-9_]+?)_(\d+)(?:&|$)").expect("lookup regex")
});

/// `camelCase` -> `snake_case` (only when the name has an uppercase letter).
pub fn camel_to_snake(name: &str) -> String {
    if !name.chars().any(char::is_uppercase) {
        return name.to_string();
    }
    let mut out = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(c);
    }
    out.to_lowercase()
}

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("tag regex"));
static BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<\s*(br|/p|/div|/li)\s*/?>").expect("block regex"));

/// `str.splitlines()`.
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let brk = matches!(
            c,
            '\n' | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if brk {
            out.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r'
                && let Some((_, '\n')) = it.peek()
            {
                it.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Rough HTML -> text for notices, placeholders and help text.
pub fn plain_text(markup: &str) -> String {
    if markup.is_empty() {
        return String::new();
    }
    let mut text = html_unescape(markup);
    if text.contains('<') {
        text = BLOCK.replace_all(&text, "\n").into_owned();
        text = TAG.replace_all(&text, "").into_owned();
        text = html_unescape(&text);
    }
    let text = text.replace('\u{a0}', " ");
    let lines: Vec<String> = splitlines(&text)
        .into_iter()
        .map(squash_ws)
        .filter(|l| !l.is_empty())
        .collect();
    strip(&lines.join("\n")).to_string()
}

/// One selectable option.
///
/// `value` is what the form stores (and what custom fields send); `id` is
/// what default fields send (e.g. `priority: 2`). `description` is the long
/// meaning the web shows (parsed from `Label - meaning || ...` help text).
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub id: Option<Json>,
    pub display_id: Option<Json>,
    pub description: String,
    pub visible: bool,
    pub children: Vec<Choice>,
}

impl Choice {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Choice {
            value: value.into(),
            label: label.into(),
            id: None,
            display_id: None,
            description: String::new(),
            visible: true,
            children: vec![],
        }
    }

    /// Normalised keys this choice answers to (value, label, id, display id).
    pub fn keys(&self) -> HashSet<String> {
        let mut out = HashSet::from([norm(&self.value), norm(&self.label)]);
        if let Some(id) = &self.id {
            out.insert(norm(&jstr(id)));
        }
        if let Some(d) = &self.display_id {
            out.insert(norm(&jstr(d)));
        }
        out
    }

    fn id_key(&self) -> String {
        norm(
            &self
                .id
                .as_ref()
                .map(jstr)
                .unwrap_or_else(|| "None".to_string()),
        )
    }
}

/// Fields revealed when the controlling dropdown has one of `choices`.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Normalised choice values.
    pub choices: Vec<String>,
    /// API names.
    pub fields: Vec<String>,
}

/// One form field.
#[derive(Clone, Debug)]
pub struct Field {
    pub api_name: String,
    pub label: String,
    pub kind: Kind,
    pub choices: Vec<Choice>,
    pub required_base: bool,
    pub position: Option<f64>,
    pub helptext: String,
    pub hint: bool,
    pub placeholder: String,
    pub description: String,
    pub default: Option<Value>,
    pub is_custom: bool,
    pub field_type: String,
    pub id: Option<Json>,
    pub multi: bool,
    pub visible_base: bool,
    pub on_form: bool,
    /// Notice HTML.
    pub content: String,
    /// Controlling dropdown (section) or upper nested level.
    pub parent: Option<String>,
    /// Nested level (1 = top).
    pub level: i64,
    /// API names of deeper nested levels (on the top field).
    pub levels: Vec<String>,
    pub sections: Vec<Section>,
    pub date_only: bool,
    pub lookup_entity: Option<String>,
    pub lookup_link: Option<String>,
    /// Catalog lookup options, kept apart from the executable module-form link.
    pub lookup_config: Option<Map<String, Json>>,
    pub allowed_file_types: Vec<String>,
    pub max_files: Option<i64>,
    /// False: shown and answerable, but its request key is unknown (never sent).
    pub sent: bool,
    /// Added by fsv (the web renders it; the metadata does not list it).
    pub synthetic: bool,
}

impl Field {
    /// A field with Python's dataclass defaults.
    pub fn new(api_name: impl Into<String>, label: impl Into<String>, kind: Kind) -> Self {
        Field {
            api_name: api_name.into(),
            label: label.into(),
            kind,
            choices: vec![],
            required_base: false,
            position: None,
            helptext: String::new(),
            hint: false,
            placeholder: String::new(),
            description: String::new(),
            default: None,
            is_custom: false,
            field_type: String::new(),
            id: None,
            multi: false,
            visible_base: true,
            on_form: true,
            content: String::new(),
            parent: None,
            level: 1,
            levels: vec![],
            sections: vec![],
            date_only: false,
            lookup_entity: None,
            lookup_link: None,
            lookup_config: None,
            allowed_file_types: vec![],
            max_files: None,
            sent: true,
            synthetic: false,
        }
    }

    /// `help` (alias of `helptext`).
    pub fn help(&self) -> &str {
        &self.helptext
    }

    /// False for content-only blocks: never answered, listed, required or sent.
    pub fn is_input(&self) -> bool {
        !INPUTLESS_KINDS.contains(&self.kind)
    }

    pub fn is_nested_level(&self) -> bool {
        self.kind == Kind::Nested && self.level > 1
    }

    /// Find a choice by value/label (normalised exact), else by id/display
    /// id. Several value/label matches raise the Python `ValueError`
    /// `"<label>: ambiguous choice <raw!r>"`.
    pub fn choice<R: PyObj + ?Sized>(&self, raw: &R) -> Result<Option<&Choice>> {
        self.choice_in(raw, &self.choices)
    }

    /// [`Field::choice`] over another pool (nested levels, restrictions).
    pub fn choice_in<'a, R: PyObj + ?Sized>(
        &self,
        raw: &R,
        pool: &'a [Choice],
    ) -> Result<Option<&'a Choice>> {
        let key = norm(&raw.py_str());
        let mut matches: Vec<&Choice> = pool
            .iter()
            .filter(|c| norm(&c.value) == key || norm(&c.label) == key)
            .collect();
        if matches.is_empty() {
            matches = pool
                .iter()
                .filter(|c| {
                    c.id.as_ref().is_some_and(|i| norm(&jstr(i)) == key)
                        || c.display_id.as_ref().is_some_and(|d| norm(&jstr(d)) == key)
                })
                .collect();
        }
        if matches.len() > 1 {
            return Err(FormError::Value(format!(
                "{}: ambiguous choice {}",
                self.label,
                raw.py_repr()
            )));
        }
        Ok(matches.first().copied())
    }

    /// The choice with this `display_id` (what `populate` actions use).
    pub fn choice_by_display_id<R: PyObj + ?Sized>(&self, display_id: &R) -> Option<&Choice> {
        let key = norm(&display_id.py_str());
        self.choices
            .iter()
            .rev()
            .find(|c| c.display_id.as_ref().is_some_and(|d| norm(&jstr(d)) == key))
    }
}

/// Service-catalog item metadata needed to place a request.
#[derive(Clone, Debug, PartialEq)]
pub struct ServiceItem {
    pub id: Json,
    pub display_id: Json,
    pub name: String,
    pub short_description: String,
    pub category_name: String,
    pub description_html: String,
    pub allow_attachments: bool,
    pub attachment_mandatory: bool,
    pub allow_quantity: bool,
    pub subject_template: String,
}

static SUBJECT_PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*custom_field\.([\w]+)\s*\}\}").expect("subject regex"));

impl ServiceItem {
    /// Fill `{{custom_field.cf_<name>}}` placeholders from field values.
    pub fn render_subject(&self, values: &Values) -> String {
        let out =
            SUBJECT_PLACEHOLDER.replace_all(&self.subject_template, |m: &regex::Captures<'_>| {
                let key = &m[1];
                let short = key.strip_prefix("cf_").unwrap_or(key);
                for cand in [key, short] {
                    if let Some(v) = values.get(cand)
                        && !v.is_null()
                        && v.as_str() != Some("")
                    {
                        return v.py_str();
                    }
                }
                String::new()
            });
        strip(&out).to_string()
    }
}

/// A form: its fields (in metadata order), the catalog item for service
/// requests, build warnings and the Freshservice account id (custom fields
/// are known to the server as `<name>_<account id>`).
#[derive(Clone, Debug)]
pub struct FormSpec {
    /// `"change"`, `"ticket"`, `"service_request"` or another module name.
    pub module: String,
    fields: Vec<Field>,
    pub item: Option<ServiceItem>,
    pub warnings: Vec<String>,
    pub account_id: Option<String>,
    by_name: IndexMap<String, usize>,
    by_id: IndexMap<String, usize>,
}

impl FormSpec {
    pub fn new(
        module: impl Into<String>,
        fields: Vec<Field>,
        item: Option<ServiceItem>,
        warnings: Vec<String>,
        account_id: Option<String>,
    ) -> Self {
        let mut spec = FormSpec {
            module: module.into(),
            fields,
            item,
            warnings,
            account_id,
            by_name: IndexMap::new(),
            by_id: IndexMap::new(),
        };
        spec.reindex();
        spec
    }

    fn reindex(&mut self) {
        self.by_name.clear();
        self.by_id.clear();
        for (i, f) in self.fields.iter().enumerate() {
            self.by_name.insert(f.api_name.clone(), i);
            if let Some(id) = &f.id {
                self.by_id.insert(jstr(id), i);
            }
        }
    }

    /// Every field in metadata order (including off-form fields).
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Edit fields in place (names and ids must not change).
    pub fn fields_mut(&mut self) -> &mut [Field] {
        &mut self.fields
    }

    /// `by_name[name]`: exact API name lookup (the last field with that name).
    pub fn get(&self, name: &str) -> Option<&Field> {
        self.by_name.get(name).map(|i| &self.fields[*i])
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Field> {
        self.by_name
            .get(name)
            .copied()
            .map(move |i| &mut self.fields[i])
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// API names in `by_name` order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// `by_id[str(id)]`.
    pub fn get_by_id(&self, id: &str) -> Option<&Field> {
        self.by_id.get(id).map(|i| &self.fields[*i])
    }

    /// Find by API name, rule/DOM name, or label (normalised exact, unique).
    pub fn find(&self, name: &str) -> Option<&Field> {
        if let Some(f) = self.get(name) {
            return Some(f);
        }
        if let Some(c) = self.resolve_name(name) {
            return self.get(c);
        }
        let key = norm(name);
        let hits: Vec<&Field> = self
            .fields
            .iter()
            .filter(|f| norm(&f.label) == key)
            .collect();
        if hits.len() == 1 { Some(hits[0]) } else { None }
    }

    /// [`FormSpec::find`] or `UnknownFieldError` with label suggestions.
    pub fn field(&self, name: &str) -> Result<&Field> {
        self.find(name).ok_or_else(|| {
            let key = norm(name);
            let suggestions = self
                .fields
                .iter()
                .filter(|f| !key.is_empty() && norm(&f.label).contains(&key))
                .take(5)
                .map(|f| f.label.clone())
                .collect();
            FormError::UnknownField {
                name: name.to_string(),
                suggestions,
            }
        })
    }

    /// Map a business-rule / web DOM field name to the API name.
    ///
    /// Handles `customFields.x` / `customField.sr_x` prefixes, catalog `cf_`
    /// aliases, camelCase DOM names (`plannedStartDate`) and `*_id` foreign
    /// keys (`department_id` -> `department`). `None` when unknown.
    pub fn resolve_name(&self, name: &str) -> Option<&str> {
        if name.is_empty() {
            return None;
        }
        if let Some((k, _)) = self.by_name.get_key_value(name) {
            return Some(k);
        }
        let mut n = name;
        for p in CUSTOM_PREFIXES {
            if let Some(rest) = n.strip_prefix(p) {
                n = rest;
                break;
            }
        }
        let mut cands: Vec<String> = vec![n.to_string(), camel_to_snake(n)];
        for c in cands.clone() {
            if let Some(a) = name_alias(&c) {
                cands.push(a.to_string());
            }
            if let Some(rest) = c.strip_prefix("sr_") {
                cands.push(rest.to_string());
            }
            if self.module == "service_request"
                && let Some(rest) = c.strip_prefix("cf_")
            {
                cands.push(rest.to_string());
            }
            if let Some(rest) = c.strip_suffix("_id") {
                cands.push(rest.to_string());
            }
        }
        cands.iter().find_map(|c| {
            self.by_name
                .get_key_value(c.as_str())
                .map(|(k, _)| k.as_str())
        })
    }

    /// Fields in web order: by position, with section children and nested
    /// levels right after the field that controls them (on-form fields only).
    pub fn ordered(&self) -> Vec<&Field> {
        let mut children: IndexMap<&str, Vec<&Field>> = IndexMap::new();
        let mut top: Vec<&Field> = Vec::new();
        for f in &self.fields {
            if !f.on_form {
                continue;
            }
            match &f.parent {
                Some(p) if self.by_name.contains_key(p) => {
                    children.entry(p.as_str()).or_default().push(f)
                }
                _ => top.push(f),
            }
        }
        top.sort_by(|a, b| {
            pos_key(a)
                .partial_cmp(&pos_key(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut out = Vec::new();
        fn emit<'a>(
            f: &'a Field,
            children: &mut IndexMap<&str, Vec<&'a Field>>,
            out: &mut Vec<&'a Field>,
        ) {
            out.push(f);
            let mut kids = children
                .get(f.api_name.as_str())
                .cloned()
                .unwrap_or_default();
            kids.sort_by(|a, b| {
                (a.level, pos_key(a))
                    .partial_cmp(&(b.level, pos_key(b)))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            if let Some(slot) = children.get_mut(f.api_name.as_str()) {
                *slot = kids.clone();
            }
            for k in kids {
                emit(k, children, out);
            }
        }
        for f in top {
            emit(f, &mut children, &mut out);
        }
        out
    }

    /// [`FormSpec::ordered`] without content-only blocks.
    pub fn inputs(&self) -> Vec<&Field> {
        self.ordered()
            .into_iter()
            .filter(|f| f.is_input())
            .collect()
    }

    /// (controlling field, normalised choices) if `name` sits in a section.
    pub fn section_parent(&self, name: &str) -> Option<(&Field, Vec<String>)> {
        let f = self.get(name)?;
        let parent = self.get(f.parent.as_deref()?)?;
        let mut choices: Vec<String> = Vec::new();
        let mut listed = false;
        for s in &parent.sections {
            if s.fields.iter().any(|x| x == name) {
                listed = true;
                for c in &s.choices {
                    if !choices.contains(c) {
                        choices.push(c.clone());
                    }
                }
            }
        }
        if !choices.is_empty() || listed {
            return Some((parent, choices));
        }
        // Catalog descendants not mapped by a choice must not appear by
        // default. Module-form nested fields use a different mechanism.
        if self.module == "service_request" {
            return Some((parent, vec![]));
        }
        None
    }

    /// Choices available now (nested levels depend on the upper levels).
    pub fn choices_for<'a>(&'a self, f: &'a Field, values: &Values) -> Result<&'a [Choice]> {
        if !f.is_nested_level() {
            return Ok(&f.choices);
        }
        let mut chain: Vec<&Field> = Vec::new();
        let mut cur = f;
        while cur.is_nested_level() {
            let Some(parent) = self.get(cur.parent.as_deref().unwrap_or("")) else {
                return Ok(&[]);
            };
            chain.push(parent);
            cur = parent;
        }
        let mut pool: &[Choice] = chain.last().map(|p| p.choices.as_slice()).unwrap_or(&[]);
        for upper in chain.iter().rev() {
            let picked = values.get(&upper.api_name);
            let Some(picked) = picked.filter(|v| !v.is_null() && v.as_str() != Some("")) else {
                return Ok(&[]);
            };
            match upper.choice_in(picked, pool)? {
                Some(c) => pool = &c.children,
                None => return Ok(&[]),
            }
        }
        Ok(pool)
    }

    /// Build from a module form-fields response (internal or v2 API).
    pub fn from_form_fields(data: &Json, module: &str) -> Result<FormSpec> {
        let raw = unwrap_fields(data)?;
        let mut warnings = Vec::new();
        let mut fields = Vec::new();
        for f in raw {
            fields.extend(fields_from_form_field(f, &mut warnings)?);
        }
        if REQUESTED_FOR_MODULES.contains(&module) {
            add_requested_for(&mut fields);
        }
        let account = account_from_form_fields(raw);
        let mut spec = FormSpec::new(module, fields, None, warnings, account);
        link_ticket_sections(&mut spec, raw);
        Ok(spec)
    }

    /// Build from `GET /api/_/catalog/service-items/<id>`.
    pub fn from_service_item(data: &Json) -> Result<FormSpec> {
        let si = if data.is_object() {
            data.get("service_item").unwrap_or(data)
        } else {
            data
        };
        let mut warnings = Vec::new();
        let mut fields: Vec<Field> = Vec::new();
        let mut id_to_name: IndexMap<String, String> = IndexMap::new();
        let mut pending: Vec<(usize, Vec<SrSection>)> = Vec::new();

        fn walk(
            items: &Json,
            parent: Option<&str>,
            fields: &mut Vec<Field>,
            id_to_name: &mut IndexMap<String, String>,
            pending: &mut Vec<(usize, Vec<SrSection>)>,
            warnings: &mut Vec<String>,
        ) -> Result<()> {
            for c in items.as_array().into_iter().flatten() {
                if get_truthy(c, "deleted").is_some() {
                    continue;
                }
                let (f, sections) = field_from_sr(c, parent, warnings)?;
                let name = f.api_name.clone();
                let idx = fields.len();
                fields.push(f);
                id_to_name.insert(jstr(c.get("id").unwrap_or(&Json::Null)), name.clone());
                if !sections.is_empty() {
                    pending.push((idx, sections));
                }
                let kids = get_truthy(c, "fields")
                    .cloned()
                    .unwrap_or(Json::Array(vec![]));
                walk(&kids, Some(&name), fields, id_to_name, pending, warnings)?;
            }
            Ok(())
        }

        let top = get_truthy(si, "custom_fields")
            .cloned()
            .unwrap_or(Json::Array(vec![]));
        walk(
            &top,
            None,
            &mut fields,
            &mut id_to_name,
            &mut pending,
            &mut warnings,
        )?;
        for (idx, sections) in pending {
            let fname = fields[idx].api_name.clone();
            let mut out = Vec::new();
            for s in sections {
                if s.fields.iter().any(|i| !id_to_name.contains_key(&jstr(i))) {
                    return Err(FormError::Value(format!(
                        "item field {}: section references unknown child id",
                        fsv_py::py_repr_str(&fname)
                    )));
                }
                let names: Vec<String> = s
                    .fields
                    .iter()
                    .map(|i| id_to_name[&jstr(i)].clone())
                    .collect();
                let non_child = names.iter().any(|n| {
                    fields
                        .iter()
                        .find(|c| &c.api_name == n)
                        .and_then(|c| c.parent.as_deref())
                        != Some(fname.as_str())
                });
                if non_child {
                    return Err(FormError::Value(format!(
                        "item field {}: section references a non-child field",
                        fsv_py::py_repr_str(&fname)
                    )));
                }
                out.push(Section {
                    choices: s.choices.iter().map(|x| norm(&jstr(x))).collect(),
                    fields: names,
                });
            }
            fields[idx].sections = out;
        }
        for child in &fields {
            if let Some(p) = &child.parent {
                let parent = fields.iter().find(|f| &f.api_name == p);
                let listed = parent.is_some_and(|pf| {
                    pf.sections
                        .iter()
                        .any(|s| s.fields.contains(&child.api_name))
                });
                if !listed {
                    warnings.push(format!(
                        "item field {}: no section choice reveals this child; hidden",
                        fsv_py::py_repr_str(&child.api_name)
                    ));
                }
            }
        }
        let configs = get_truthy(si, "configs")
            .cloned()
            .unwrap_or(Json::Object(Map::new()));
        let get_or =
            |k: &str, default: bool| -> bool { if has(si, k) { truthy(&si[k]) } else { default } };
        let cfg_mandatory = if has(&configs, "attachment_mandatory") {
            truthy(&configs["attachment_mandatory"])
        } else {
            false
        };
        let item = ServiceItem {
            id: si.get("id").cloned().unwrap_or(Json::Null),
            display_id: si.get("display_id").cloned().unwrap_or(Json::Null),
            name: get_str(si, "name"),
            short_description: get_str(si, "short_description"),
            category_name: get_str(si, "category_name"),
            description_html: get_str(si, "description"),
            allow_attachments: get_or("allow_attachments", true),
            attachment_mandatory: get_or("attachment_mandatory", cfg_mandatory),
            allow_quantity: get_or("allow_quantity", false),
            subject_template: get_str(&configs, "subject"),
        };
        let account = account_from_item(si);
        Ok(FormSpec::new(
            "service_request",
            fields,
            Some(item),
            warnings,
            account,
        ))
    }
}

fn pos_key(f: &Field) -> (bool, f64) {
    (f.position.is_none(), f.position.unwrap_or(0.0))
}

fn unwrap_fields(data: &Json) -> Result<&[Json]> {
    if let Some(list) = data.as_array() {
        return Ok(list);
    }
    if let Some(obj) = data.as_object() {
        for key in [
            "change_fields",
            "ticket_fields",
            "problem_fields",
            "release_fields",
            "fields",
            "form_fields",
        ] {
            if let Some(Json::Array(a)) = obj.get(key) {
                return Ok(a);
            }
        }
        for v in obj.values() {
            if let Json::Array(a) = v
                && let Some(Json::Object(first)) = a.first()
                && first.contains_key("field_type")
            {
                return Ok(a);
            }
        }
    }
    Err(FormError::Value(
        "unrecognised form-fields payload".to_string(),
    ))
}

fn account_from_form_fields(raw: &[Json]) -> Option<String> {
    let mut found: Vec<String> = Vec::new();
    for f in raw {
        let link = get_truthy(f, "lookup_config")
            .and_then(|c| get_truthy(c, "link"))
            .map(jstr)
            .unwrap_or_default();
        if let Some(m) = LOOKUP_FIELD_NAME.captures(&link)
            && m[1] == jstr(f.get("name").unwrap_or(&Json::Null))
            && !found.contains(&m[2].to_string())
        {
            found.push(m[2].to_string());
        }
    }
    if found.len() == 1 { found.pop() } else { None }
}

fn account_from_item(si: &Json) -> Option<String> {
    let mut found: Vec<String> = Vec::new();
    fn add(found: &mut Vec<String>, v: Option<&Json>) {
        if let Some(v) = v
            && !v.is_null()
            && v.as_str() != Some("")
        {
            let s = jstr(v);
            if !found.contains(&s) {
                found.push(s);
            }
        }
    }
    fn walk(items: Option<&Json>, found: &mut Vec<String>) {
        for c in items.and_then(Json::as_array).into_iter().flatten() {
            add(found, c.get("account_id"));
            walk(get_truthy(c, "fields"), found);
        }
    }
    walk(get_truthy(si, "custom_fields"), &mut found);
    add(&mut found, si.get("account_id"));
    if found.len() == 1 { found.pop() } else { None }
}

/// `_parse_choices`: module form-field choices (mappings, `[value, id]`
/// pairs or bare values), recursively with `nested_options`.
pub fn parse_choices(raw: Option<&Json>) -> Vec<Choice> {
    let mut out = Vec::new();
    for c in raw
        .filter(|r| truthy(r))
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        match c {
            Json::Object(m) => {
                let value = m.get("value").filter(|v| !v.is_null());
                let label = if m.contains_key("label") {
                    m.get("label").filter(|v| !v.is_null())
                } else {
                    value
                };
                let value_s = jstr(value.or(label).unwrap_or(&Json::Null));
                let label_s = html_unescape(&jstr(label.or(value).unwrap_or(&Json::Null)));
                out.push(Choice {
                    value: value_s,
                    label: label_s,
                    id: m.get("id").filter(|v| !v.is_null()).cloned(),
                    display_id: m.get("display_id").filter(|v| !v.is_null()).cloned(),
                    description: squash_ws(&get_str(c, "description")),
                    visible: m.get("visible") != Some(&Json::Bool(false)),
                    children: parse_choices(m.get("nested_options")),
                });
            }
            Json::Array(a) if !a.is_empty() => {
                let mut ch = Choice::new(jstr(&a[0]), jstr(&a[0]));
                ch.id = a.get(1).filter(|v| !v.is_null()).cloned();
                out.push(ch);
            }
            other => out.push(Choice::new(jstr(other), jstr(other))),
        }
    }
    out
}

/// Parse `"Very High - meaning || Low - meaning"` help text.
///
/// Each `||` segment goes to the longest choice label it starts with
/// (case-insensitive) followed by optional spaces and `-`; that becomes the
/// choice's description. Returns the choices and the unmatched segments
/// (general help text). Choices keep any description they already have.
pub fn split_choice_help(choices: &[Choice], helptext: &str) -> (Vec<Choice>, String) {
    if helptext.is_empty() || !helptext.contains("||") {
        return (choices.to_vec(), strip(helptext).to_string());
    }
    let segs: Vec<String> = helptext.split("||").map(squash_ws).collect();
    let mut names: IndexMap<String, String> = IndexMap::new();
    for c in choices {
        let name = if c.label.is_empty() {
            &c.value
        } else {
            &c.label
        };
        if !name.is_empty() {
            names.insert(squash_ws(name), name.clone());
        }
    }
    let mut names: Vec<(String, String)> = names.into_iter().collect();
    names.sort_by_key(|(flat, _)| std::cmp::Reverse(flat.chars().count()));
    static DASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*-\s*(.+)$").expect("dash regex"));
    let mut found: IndexMap<String, String> = IndexMap::new();
    let mut rest: Vec<String> = Vec::new();
    for seg in segs {
        if seg.is_empty() {
            continue;
        }
        let low = seg.to_lowercase();
        let mut matched = false;
        for (flat, name) in &names {
            let fl = flat.to_lowercase();
            if low.starts_with(&fl) {
                let tail = char_slice_from(&seg, flat.chars().count());
                if let Some(m) = DASH.captures(tail) {
                    found
                        .entry(norm(name))
                        .or_insert_with(|| strip(&m[1]).to_string());
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            rest.push(seg);
        }
    }
    let out = choices
        .iter()
        .map(|c| {
            let d = found
                .get(&norm(&c.label))
                .filter(|d| !d.is_empty())
                .or_else(|| found.get(&norm(&c.value)));
            match d.filter(|d| !d.is_empty()) {
                Some(d) => Choice {
                    description: d.clone(),
                    ..c.clone()
                },
                None => c.clone(),
            }
        })
        .collect();
    (out, rest.join(" "))
}

fn char_slice_from(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[i..],
        None => "",
    }
}

/// `attach_choice_descriptions`.
pub fn attach_choice_descriptions(choices: &[Choice], helptext: &str) -> Vec<Choice> {
    split_choice_help(choices, helptext).0
}

fn json_f64(v: Option<&Json>) -> Option<f64> {
    v.and_then(Json::as_f64)
}

fn level_key(n: &Json) -> f64 {
    get_truthy(n, "level").and_then(Json::as_f64).unwrap_or(0.0)
}

/// `int(x)` of a JSON number or numeric string.
fn json_int(v: &Json) -> Option<i64> {
    match v {
        Json::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Json::Bool(b) => Some(*b as i64),
        Json::String(s) => crate::pytext::parse_int(strip(s)),
        _ => None,
    }
}

/// `_fields_from_form_field`: one module form field (plus its nested levels).
pub fn fields_from_form_field(f: &Json, warnings: &mut Vec<String>) -> Result<Vec<Field>> {
    let ftype = get_str(f, "field_type");
    let name = get_str(f, "name");
    let is_default = match f.get("default_field") {
        Some(v) => truthy(v),
        None => ftype.starts_with("default_"),
    };
    let mut kind = default_kind(&ftype).or_else(|| custom_kind(&ftype));
    if kind.is_none() && is_content_type(&ftype) {
        kind = Some(Kind::Notice);
    }
    let mut kind = match kind {
        Some(k) => k,
        None => {
            warnings.push(format!(
                "field {}: unsupported field_type {}; writes blocked when required or answered",
                fsv_py::py_repr_str(&name),
                fsv_py::py_repr_str(&ftype)
            ));
            Kind::Unsupported
        }
    };
    let empty = Json::Object(Map::new());
    let lookup_cfg = get_truthy(f, "lookup_config")
        .filter(|c| c.is_object())
        .unwrap_or(&empty);
    let mut lookup_entity = None;
    if ftype == "custom_lookup" || ftype == "custom_multi_lookup" {
        let src = if has(lookup_cfg, "data_source") {
            &lookup_cfg["data_source"]
        } else {
            f.get("data_source").unwrap_or(&Json::Null)
        };
        let is_user =
            src.is_null() || (src.is_number() && src.as_f64() == Some(USER_LOOKUP_SOURCE as f64));
        if is_user {
            lookup_entity = Some("user".to_string());
        } else {
            kind = if get_truthy(lookup_cfg, "link").is_some() {
                Kind::Lookup
            } else {
                Kind::Unsupported
            };
            lookup_entity = Some(jstr(src));
        }
    } else if ftype == "default_change_window" {
        lookup_entity = Some("change_window".to_string());
    }
    let date_only_raw = f.get("date_only").unwrap_or(&Json::Null);
    if kind == Kind::Datetime
        && (date_only_raw == &Json::Bool(true) || jstr(date_only_raw).to_lowercase() == "true")
    {
        kind = Kind::Date;
    }
    let helptext = f
        .get("helptext")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let (choices, helptext) = split_choice_help(&parse_choices(f.get("choices")), &helptext);
    let position = json_f64(f.get("position"));
    let required = match get(f, "required") {
        Some(v) => truthy(v),
        None => get(f, "required_for_agents").is_some_and(truthy),
    };
    let mut default = None;
    if FIRST_CHOICE_DEFAULT_TYPES.contains(&ftype.as_str()) {
        default = choices
            .iter()
            .find(|c| c.visible)
            .map(|c| Value::Str(c.value.clone()));
    }
    let explicit = if has(f, "default_value") {
        f.get("default_value")
    } else {
        f.get("default")
    };
    if let Some(explicit) = explicit.filter(|v| !v.is_null()) {
        let key = norm(&jstr(explicit));
        let matches: Vec<&Choice> = choices
            .iter()
            .filter(|c| {
                c.visible && (key == norm(&c.value) || key == norm(&c.label) || key == c.id_key())
            })
            .collect();
        if kind.is_choice() {
            if matches.len() != 1 {
                return Err(FormError::Value(format!(
                    "{name}: explicit default must match one active choice"
                )));
            }
            default = Some(Value::Str(matches[0].value.clone()));
        } else {
            default = Some(Value::from_json(explicit));
        }
    }
    let mut nested: Vec<&Json> = get_truthy(f, "nested_fields")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .collect();
    nested.sort_by(|a, b| {
        level_key(a)
            .partial_cmp(&level_key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let content = if kind == Kind::Notice {
        content_of(f)
    } else {
        get_str(f, "content")
    };
    let mut top = Field::new(
        name.clone(),
        html_unescape(&jstr(
            get_truthy(f, "label").unwrap_or(&Json::String(name.clone())),
        )),
        kind,
    );
    top.choices = choices;
    top.required_base = required;
    top.position = position;
    top.helptext = helptext;
    top.hint = get(f, "hint").is_some_and(truthy);
    top.placeholder = plain_text(&get_str(f, "placeholder"));
    top.description = get_str(f, "description");
    top.default = default;
    top.is_custom = !is_default;
    top.field_type = ftype.clone();
    top.id = f.get("id").filter(|v| !v.is_null()).cloned();
    top.multi = ftype == "custom_multi_select_dropdown" || ftype == "custom_multi_lookup";
    top.visible_base = f.get("displayed_to_agents") != Some(&Json::Bool(false));
    top.on_form = f.get("position").is_some_and(|p| !p.is_null()) && !is_planning_type(&ftype);
    top.content = html_unescape(&content);
    top.levels = nested
        .iter()
        .map(|n| jstr(n.get("name").unwrap_or(&Json::Null)))
        .collect();
    top.date_only = kind == Kind::Date;
    top.lookup_entity = if ftype != REQUESTED_FOR_TYPE {
        lookup_entity
    } else {
        Some("user".to_string())
    };
    top.lookup_link = get(lookup_cfg, "link").map(jstr);
    top.sent = kind != Kind::Unsupported && !UNSENT_TYPES.contains(&ftype.as_str());
    let mut out = vec![top];
    let mut prev = name;
    for n in nested {
        let lvl_name = jstr(n.get("name").unwrap_or(&Json::Null));
        let mut lf = Field::new(
            lvl_name.clone(),
            html_unescape(&jstr(
                get_truthy(n, "label").unwrap_or(&Json::String(lvl_name.clone())),
            )),
            Kind::Nested,
        );
        lf.position = position;
        lf.is_custom = out[0].is_custom;
        lf.field_type = ftype.clone();
        lf.id = n.get("id").filter(|v| !v.is_null()).cloned();
        lf.visible_base = out[0].visible_base;
        lf.on_form = out[0].on_form;
        lf.parent = Some(prev.clone());
        lf.level = get_truthy(n, "level").and_then(json_int).unwrap_or(2);
        lf.placeholder = plain_text(&get_str(n, "placeholder"));
        lf.description = get_str(n, "description");
        prev = lvl_name;
        out.push(lf);
    }
    Ok(out)
}

fn add_requested_for(fields: &mut Vec<Field>) {
    if fields
        .iter()
        .any(|f| f.field_type == REQUESTED_FOR_TYPE || f.api_name == REQUESTED_FOR)
    {
        return;
    }
    let Some(req) = fields.iter().find(|f| f.field_type == "default_requester") else {
        return;
    };
    let pos = req.position.map(|p| p + 0.5);
    let mut f = Field::new(REQUESTED_FOR, REQUESTED_FOR_LABEL, Kind::LookupUser);
    f.position = pos;
    f.helptext =
        "The user this change is for, if not the requester (shown in fsv only: not sent yet)."
            .to_string();
    f.field_type = REQUESTED_FOR_TYPE.to_string();
    f.visible_base = req.visible_base;
    f.on_form = req.on_form && pos.is_some();
    f.lookup_entity = Some("user".to_string());
    f.sent = false;
    f.synthetic = true;
    fields.push(f);
}

/// A content block's HTML: `content`, else `field_options.content`, else
/// its description/help text.
fn content_of(f: &Json) -> String {
    let empty = Json::Null;
    let opts = f
        .get("field_options")
        .filter(|o| o.is_object())
        .unwrap_or(&empty);
    for v in [
        f.get("content"),
        opts.get("content"),
        opts.get("content_html"),
        f.get("description"),
        f.get("helptext"),
    ] {
        if let Some(Json::String(s)) = v
            && !strip(s).is_empty()
        {
            return s.clone();
        }
    }
    String::new()
}

/// Dynamic sections (`sections: [{choice_ids, ticket_field_ids}]`). A field
/// flagged `belongs_to_section` that no section lists is never revealed by
/// a choice: it starts hidden (a rule may still show it).
fn link_ticket_sections(spec: &mut FormSpec, raw: &[Json]) {
    link_sections(spec, raw);
    for f in raw {
        if get_truthy(f, "belongs_to_section").is_none() {
            continue;
        }
        let name = jstr(f.get("name").unwrap_or(&Json::Null));
        if let Some(fld) = spec.get_mut(&name)
            && fld.parent.is_none()
            && fld.visible_base
        {
            fld.visible_base = false;
            let api = fld.api_name.clone();
            spec.warnings.push(format!(
                "field {} belongs to a section that no choice reveals; hidden",
                fsv_py::py_repr_str(&api)
            ));
        }
    }
}

fn link_sections(spec: &mut FormSpec, raw: &[Json]) {
    for f in raw {
        let Some(sections) = get_truthy(f, "sections").and_then(Json::as_array) else {
            continue;
        };
        let name = jstr(f.get("name").unwrap_or(&Json::Null));
        let Some(ctrl) = spec.get(&name) else {
            continue;
        };
        let ctrl_name = ctrl.api_name.clone();
        let mut by_choice_id: IndexMap<String, String> = IndexMap::new();
        for c in &ctrl.choices {
            by_choice_id.insert(
                c.id.as_ref()
                    .map(jstr)
                    .unwrap_or_else(|| "None".to_string()),
                c.value.clone(),
            );
        }
        let mut out = Vec::new();
        for s in sections {
            let ids = get_truthy(s, "ticket_field_ids")
                .or_else(|| get_truthy(s, "field_ids"))
                .or_else(|| get_truthy(s, "change_field_ids"))
                .and_then(Json::as_array)
                .cloned()
                .unwrap_or_default();
            let names: Vec<String> = ids
                .iter()
                .filter_map(|i| spec.get_by_id(&jstr(i)).map(|x| x.api_name.clone()))
                .collect();
            let vals: Vec<String> = get_truthy(s, "choice_ids")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .filter_map(|i| by_choice_id.get(&jstr(i)).map(|v| norm(v)))
                .collect();
            for n in &names {
                if let Some(child) = spec.get_mut(n) {
                    child.parent = Some(ctrl_name.clone());
                }
            }
            out.push(Section {
                choices: vals,
                fields: names,
            });
        }
        if let Some(ctrl) = spec.get_mut(&name) {
            ctrl.sections = out;
        }
    }
}

/// A catalog section before child ids are resolved to names.
#[derive(Clone, Debug)]
pub struct SrSection {
    pub fields: Vec<Json>,
    pub choices: Vec<Json>,
}

fn opt_true(opts: &Json, key: &str) -> bool {
    let v = if has(opts, key) {
        jstr(&opts[key])
    } else {
        String::new()
    };
    v.to_lowercase() == "true"
}

/// Python iteration over a JSON value used as a list (`for x in v`):
/// arrays yield items, strings their characters, anything else nothing.
fn iter_items(v: &Json) -> Vec<Json> {
    match v {
        Json::Array(a) => a.clone(),
        Json::String(s) => s.chars().map(|c| Json::String(c.to_string())).collect(),
        _ => vec![],
    }
}

/// `_field_from_sr`: one service-catalog item field and its raw sections.
pub fn field_from_sr(
    c: &Json,
    parent: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<(Field, Vec<SrSection>)> {
    let empty = Json::Object(Map::new());
    let opts = get_truthy(c, "field_options").unwrap_or(&empty);
    let code = c.get("type").unwrap_or(&Json::Null);
    let int_code = match code {
        Json::Number(n) => n.as_i64(),
        _ => None,
    };
    let name = jstr(
        get_truthy(c, "name")
            .or_else(|| c.get("id"))
            .unwrap_or(&Json::Null),
    );
    let mut kind = match int_code.and_then(sr_type_kind) {
        Some(k) => k,
        None => {
            warnings.push(format!(
                "item field {}: unsupported type {}; writes blocked when required or answered",
                fsv_py::py_repr_str(&name),
                jrepr(code)
            ));
            Kind::Unsupported
        }
    };
    if kind == Kind::Date && get(opts, "date_only").is_some() && !opt_true(opts, "date_only") {
        kind = Kind::Datetime;
    }
    let raw_label = jstr(get_truthy(c, "label").unwrap_or(&Json::String(name.clone())));
    let mut content = String::new();
    let mut label = html_unescape(&raw_label);
    if kind == Kind::Notice {
        content = html_unescape(&raw_label);
        label = get_truthy(opts, "content_name")
            .map(jstr)
            .unwrap_or_else(|| "Notice".to_string());
    }
    let mut raw_choices: Vec<Json> = get_truthy(c, "choices")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    raw_choices.sort_by(|a, b| {
        let ka = get_truthy(a, "position")
            .and_then(Json::as_f64)
            .unwrap_or(0.0);
        let kb = get_truthy(b, "position")
            .and_then(Json::as_f64)
            .unwrap_or(0.0);
        ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
    });
    let choices: Vec<Choice> = raw_choices
        .iter()
        .map(|x| {
            let value = x.get("value").unwrap_or(&Json::Null);
            let label = if has(x, "label") { &x["label"] } else { value };
            Choice {
                value: jstr(value),
                label: jstr(label),
                id: x.get("id").filter(|v| !v.is_null()).cloned(),
                display_id: None,
                description: squash_ws(&get_str(x, "description")),
                visible: true,
                children: vec![],
            }
        })
        .collect();
    let helptext = c
        .get("helptext")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let (choices, helptext) = split_choice_help(&choices, &helptext);
    let mut sections: Vec<SrSection> = Vec::new();
    let info = opts.get("section_info").unwrap_or(&Json::Null);
    let has_info = !(info.is_null() || info.as_str() == Some(""));
    if has_info {
        let unreadable = || {
            FormError::Value(format!(
                "item field {}: unreadable section_info",
                fsv_py::py_repr_str(&name)
            ))
        };
        let parsed = match info {
            Json::String(s) => serde_json::from_str::<Json>(s).map_err(|_| unreadable())?,
            other => other.clone(),
        };
        let Json::Array(list) = parsed else {
            return Err(unreadable());
        };
        for s in list {
            if !s.is_object() {
                return Err(unreadable());
            }
            let fields = get_truthy(&s, "sectionFields")
                .map(iter_items)
                .unwrap_or_default();
            let choices = get_truthy(&s, "sectionChoices")
                .map(iter_items)
                .unwrap_or_default();
            sections.push(SrSection { fields, choices });
        }
    }
    let mut dep_sections = Vec::new();
    let mut has_dep = false;
    for x in &raw_choices {
        let dependent = get_truthy(x, "dependent_ids");
        has_dep = has_dep || dependent.is_some_and(|d| has(d, "field"));
        let dep = dependent
            .and_then(|d| get_truthy(d, "field"))
            .map(iter_items)
            .unwrap_or_default();
        if !dep.is_empty() {
            dep_sections.push(SrSection {
                fields: dep,
                choices: vec![x.get("value").cloned().unwrap_or(Json::Null)],
            });
        }
    }
    if !has_info {
        sections = dep_sections;
    } else if has_dep {
        let pairs = |secs: &[SrSection]| -> HashSet<(String, String)> {
            secs.iter()
                .flat_map(|s| {
                    s.choices.iter().flat_map(move |ch| {
                        s.fields.iter().map(move |fid| (norm(&jstr(ch)), jstr(fid)))
                    })
                })
                .collect()
        };
        if pairs(&sections) != pairs(&dep_sections) {
            return Err(FormError::Value(format!(
                "item field {}: section_info and dependent_ids differ",
                fsv_py::py_repr_str(&name)
            )));
        }
    }
    let mut file_types = Vec::new();
    if let Some(ft) = get_truthy(opts, "allowed_file_types") {
        let parsed = match ft {
            Json::String(s) => serde_json::from_str::<Json>(s).ok(),
            other => Some(other.clone()),
        };
        match parsed {
            Some(Json::Array(a)) => file_types = a.iter().map(jstr).collect(),
            Some(Json::String(s)) => file_types = s.chars().map(String::from).collect(),
            _ => {}
        }
    }
    let limit = if has(opts, "max_attachment_limit") {
        jstr(&opts["max_attachment_limit"])
    } else {
        String::new()
    };
    let max_files = if is_digits(&limit) {
        crate::pytext::parse_int(&limit)
    } else {
        None
    };
    let is_lookup = int_code == Some(501) || int_code == Some(502);
    if is_lookup {
        warnings.push(format!(
            "item field {}: catalog lookup submission is unverified; selection is not sent",
            fsv_py::py_repr_str(&name)
        ));
    }
    let mut f = Field::new(name, label, kind);
    f.choices = choices;
    f.required_base = get(c, "required").is_some_and(truthy);
    f.position = json_f64(c.get("position"));
    f.helptext = helptext;
    f.hint = c.get("hint") == Some(&Json::Bool(true));
    f.placeholder = plain_text(&get_str(c, "placeholder"));
    f.is_custom = true;
    f.field_type = format!("formserv:{}", jstr(code));
    f.id = c.get("id").filter(|v| !v.is_null()).cloned();
    f.multi = int_code == Some(502);
    f.visible_base = c.get("visible") != Some(&Json::Bool(false));
    f.on_form = true;
    f.content = content;
    f.parent = parent.map(str::to_string);
    f.date_only = kind == Kind::Date;
    f.lookup_entity = is_lookup.then(|| "catalog_lookup".to_string());
    f.lookup_config = if is_lookup {
        opts.as_object().cloned()
    } else {
        None
    };
    f.sent = kind != Kind::Unsupported && !is_lookup;
    f.allowed_file_types = file_types;
    f.max_files = max_files;
    Ok((f, sections))
}

#[cfg(test)]
mod py_tests;
