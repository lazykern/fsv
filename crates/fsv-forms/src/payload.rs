//! Create-request payload builders matching the bodies the Freshservice web
//! UI sends (`fsv.forms.payload`):
//!
//! * change:  `POST /api/_/changes`
//! * ticket:  `POST /api/_/tickets`
//! * service request: `POST /api/_/catalog/service-items/<display_id>/place-request`
//!
//! Only visible fields with a value are sent; hidden values are dropped.
//! Status is never sent on create unless the caller passes `status`
//! explicitly. Files must be uploaded first (`POST /api/_/attachments`); the
//! builders only take the resulting attachment ids.
//!
//! Every body carries `br_validation_excludes`, built from the engine state
//! ([`build_br_validation_excludes`]). Callers cannot pass their own token:
//! it tells the server which static required checks to skip, so a
//! user-supplied value would be a way around required fields.

use std::collections::HashSet;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value as Json};

use crate::errors::{FormError, Result};
use crate::pytext::{html_escape, is_py_space, jstr, strip};
use crate::rules::{EXCLUDE_KEYS, EvaluatedForm, is_empty};
use crate::spec::{Field, FormSpec, Kind, ServiceItem};
use crate::state::FormState;
use crate::value::{Value, Values};

pub const BR_VALIDATION_EXCLUDES: &str = "br_validation_excludes";
// The token is JWT-shaped but unsigned: `<b64(header)>.<b64(payload)>`.
const BR_HEADER: &str = r#"{"alg":"HS256","typ":"JWT"}"#;
// `^[A-Za-z0-9_]+$` as Python's `re.match` reads it (a trailing newline passes).
static SAFE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[A-Za-z0-9_]+\n?\z").expect("name regex"));
pub const FLAT_KINDS: [&str; 4] = ["change", "ticket", "problem", "release"];
pub const SR_KINDS: [&str; 2] = ["service_request", "sr"];

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard alphabet with padding, like the browser's `btoa()`.
fn b64(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `base64.b64decode(s, altchars=b"-_")` (non-validating: other characters
/// are discarded).
fn b64_decode(s: &str) -> Result<Vec<u8>> {
    let mut vals = Vec::new();
    let mut pad = false;
    for c in s.bytes() {
        let v = match c {
            b'-' => 62,
            b'_' => 63,
            b'=' => {
                pad = true;
                continue;
            }
            c => match B64.iter().position(|x| *x == c) {
                Some(p) if !pad => p as u32,
                _ => continue,
            },
        };
        vals.push(v);
    }
    if vals.len() % 4 == 1 {
        return Err(FormError::Value(
            "Invalid base64-encoded string".to_string(),
        ));
    }
    let mut out = Vec::with_capacity(vals.len() * 3 / 4);
    for chunk in vals.chunks(4) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, v)| acc | v << (18 - 6 * i));
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Ok(out)
}

/// The server-side lists are JSON text with `", "` separators.
fn name_list(names: &[String]) -> String {
    let list = Json::Array(names.iter().map(|n| Json::String(n.clone())).collect());
    fsv_py::dumps(&list, &fsv_py::Dumps::default())
}

fn account(evaluated: &EvaluatedForm, account_id: &Json) -> Result<Option<String>> {
    let meta = evaluated.account_id.clone().filter(|m| !m.is_empty());
    let given = match account_id {
        Json::Null => None,
        Json::String(s) if s.is_empty() => None,
        other => Some(jstr(other)),
    };
    if let (Some(m), Some(g)) = (&meta, &given)
        && !g.is_empty()
        && m != g
    {
        return Err(FormError::payload(format!(
            "account id {g} does not match the form metadata ({m})"
        )));
    }
    Ok(meta.or(given))
}

/// `{"optional": [...], "disabled": [...], "hidden": [...]}`: the fields
/// the rules unmandated, disabled and hid, as server field names (custom
/// fields get the `_<account id>` suffix), in action order.
pub fn excludes_lists(
    evaluated: &EvaluatedForm,
    account_id: &Json,
) -> Result<IndexMap<String, Vec<String>>> {
    let acct = account(evaluated, account_id)?;
    let acct = acct.as_deref().filter(|a| !a.is_empty());
    let mut out = IndexMap::new();
    for key in EXCLUDE_KEYS {
        let mut names = Vec::new();
        for x in evaluated.excludes.get(key).unwrap_or(&[]) {
            if x.custom && acct.is_none() {
                return Err(FormError::payload(format!(
                    "br_validation_excludes: custom field {} needs the Freshservice account id (not found in the \
                     form metadata; pass account_id=)",
                    fsv_py::py_repr_str(&x.base)
                )));
            }
            let n = x.name(acct)?;
            if !SAFE_NAME.is_match(&n) {
                return Err(FormError::payload(format!(
                    "br_validation_excludes: unexpected field name {}",
                    fsv_py::py_repr_str(&n)
                )));
            }
            names.push(n);
        }
        out.insert(key.to_string(), names);
    }
    Ok(out)
}

/// The `br_validation_excludes` token the web sends with a create.
///
/// Change/ticket: `{"optional": "<list>", "disabled": "<list>", "hidden": "<list>"}`;
/// service request: `{"<item id>": {...same...}}` keyed by the item's `id`
/// (not its display id). Each list is a JSON-encoded string.
pub fn build_br_validation_excludes(
    evaluated: &EvaluatedForm,
    kind: &str,
    account_id: &Json,
    item_id: &Json,
) -> Result<String> {
    let lists = excludes_lists(evaluated, account_id)?;
    let mut inner = Map::new();
    for k in EXCLUDE_KEYS {
        inner.insert(
            k.to_string(),
            Json::String(name_list(lists.get(k).map_or(&[][..], Vec::as_slice))),
        );
    }
    let payload = if SR_KINDS.contains(&kind) {
        if item_id.is_null() || item_id.as_str() == Some("") {
            return Err(FormError::payload(
                "br_validation_excludes: a service request needs the item id",
            ));
        }
        let mut m = Map::new();
        m.insert(jstr(item_id), Json::Object(inner));
        Json::Object(m)
    } else if FLAT_KINDS.contains(&kind) {
        Json::Object(inner)
    } else {
        return Err(FormError::Value(format!(
            "unknown form kind {}",
            fsv_py::py_repr_str(kind)
        )));
    };
    Ok(format!(
        "{}.{}",
        b64(BR_HEADER),
        b64(&fsv_py::compact(&payload))
    ))
}

/// Inspect a token (debugging/tests only; builders never accept one).
pub fn decode_br_validation_excludes(token: &str) -> Result<Json> {
    let (head, body) = token.split_once('.').unwrap_or((token, ""));
    let load = |part: &str| -> Result<Json> {
        let bytes = b64_decode(part)?;
        serde_json::from_slice(&bytes).map_err(|e| FormError::Value(e.to_string()))
    };
    let header = load(head)?;
    let payload = load(body)?;
    let Json::Object(payload) = payload else {
        return Err(FormError::Value(
            "br_validation_excludes payload is not an object".to_string(),
        ));
    };
    let lists = |d: &Map<String, Json>| -> Map<String, Json> {
        d.iter()
            .map(|(k, v)| {
                let v = match v {
                    Json::String(s) => serde_json::from_str(s).unwrap_or_else(|_| v.clone()),
                    other => other.clone(),
                };
                (k.clone(), v)
            })
            .collect()
    };
    let mut out = Map::new();
    out.insert("header".to_string(), header);
    if payload.keys().all(|k| EXCLUDE_KEYS.contains(&k.as_str())) {
        out.insert("kind".to_string(), Json::String("flat".to_string()));
        out.extend(lists(&payload));
    } else {
        out.insert("kind".to_string(), Json::String("item".to_string()));
        let items = payload
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    Json::Object(v.as_object().map(lists).unwrap_or_default()),
                )
            })
            .collect();
        out.insert("items".to_string(), Json::Object(items));
    }
    Ok(Json::Object(out))
}

/// Default (non-custom) change fields whose API key differs from the field name.
pub const CHANGE_KEYS: [(&str, &str); 4] = [
    ("department", "department_id"),
    ("group", "group_id"),
    ("agent", "agent_id"),
    ("change_window", "change_window_id"),
];
/// Default (non-custom) ticket fields whose API key differs from the field name.
pub const TICKET_KEYS: [(&str, &str); 4] = [
    ("department", "department_id"),
    ("group", "group_id"),
    ("agent", "responder_id"),
    ("ticket_type", "type"),
];
// Default dropdowns sent by their label rather than their numeric id.
const SEND_VALUE_TYPES: [&str; 1] = ["default_ticket_type"];
// Never sent from form state: requester becomes `email`; status only via
// the explicit `status` argument.
const NEVER_FROM_STATE: [&str; 2] = ["requester", "status"];

const CHANGE_ORDER: [&str; 12] = [
    "subject",
    "description",
    "status",
    "priority",
    "change_type",
    "risk",
    "impact",
    "group_id",
    "department_id",
    "agent_id",
    "planned_start_date",
    "planned_end_date",
];
const TICKET_ORDER: [&str; 8] = [
    "subject",
    "description",
    "department_id",
    "status",
    "priority",
    "urgency",
    "impact",
    "source",
];

/// `Name <email>` as the web sends it, or the bare email.
pub fn format_requester(email: &str, name: Option<&str>) -> Result<String> {
    let email = strip(email);
    if !email.contains('@') {
        return Err(FormError::payload(format!(
            "requester email {} is not an email address",
            fsv_py::py_repr_str(email)
        )));
    }
    let name = strip(name.unwrap_or(""));
    Ok(if name.is_empty() {
        email.to_string()
    } else {
        format!("{name} <{email}>")
    })
}

/// Wrap plain text in the minimal HTML the description field expects.
/// Text that already looks like HTML is returned unchanged.
pub fn text_to_html(text: &str) -> String {
    if text.trim_start_matches(is_py_space).starts_with('<') {
        return text.to_string();
    }
    let s = text.replace("\r\n", "\n");
    let body: String = s
        .split("\n\n")
        .filter(|p| !strip(p).is_empty())
        .map(|p| format!("<div>{}</div>", html_escape(p).replace('\n', "<br>")))
        .collect();
    if body.is_empty() {
        "<div></div>".to_string()
    } else {
        body
    }
}

/// `list(value)`: a string iterates its characters.
fn items_of(value: &Value) -> Result<Vec<Value>> {
    match value.as_str() {
        Some(s) => Ok(s.chars().map(|c| Value::Str(c.to_string())).collect()),
        None => crate::state::py_list(value),
    }
}

/// Form value -> JSON value for the API.
pub fn encode_value(spec: &FormSpec, f: &Field, value: &Value, values: &Values) -> Result<Json> {
    match f.kind {
        Kind::Unsupported => Err(FormError::payload(format!(
            "{}: unsupported field semantics; write blocked",
            f.label
        ))),
        Kind::Dropdown | Kind::Department | Kind::Nested => {
            let pool = spec.choices_for(f, values)?;
            let Some(ch) = f.choice_in(value, pool)? else {
                return Err(FormError::payload(format!(
                    "{}: {} is not a valid choice",
                    f.label,
                    value.py_repr()
                )));
            };
            match ch.id.as_ref().filter(|id| !id.is_null()) {
                Some(id)
                    if !f.is_custom
                        && f.kind != Kind::Nested
                        && !SEND_VALUE_TYPES.contains(&f.field_type.as_str()) =>
                {
                    Ok(id.clone())
                }
                _ => Ok(Json::String(ch.value.clone())),
            }
        }
        Kind::MultiSelect => Ok(Json::Array(
            items_of(value)?
                .iter()
                .map(|v| Json::String(v.py_str()))
                .collect(),
        )),
        Kind::Checkbox => Ok(Json::Bool(value.truthy())),
        Kind::Date => Ok(Json::String(match value {
            Value::DateTime(d) => d.isoformat(),
            other => other.py_str(),
        })),
        Kind::Datetime => Ok(Json::String(match value {
            Value::DateTime(d) => d.isoformat_seconds(),
            other => other.py_str(),
        })),
        Kind::LookupUser | Kind::LookupGroup | Kind::LookupAgent | Kind::Lookup => match value {
            Value::List(items) => Ok(Json::Array(
                items
                    .iter()
                    .map(|v| lookup_id(f, v))
                    .collect::<Result<_>>()?,
            )),
            other => lookup_id(f, other),
        },
        Kind::Richtext => Ok(Json::String(text_to_html(&value.py_str()))),
        _ => {
            unsent_type(value)?;
            value.to_json()
        }
    }
}

/// Python passes other values through untouched; dates then fail in
/// `json.dumps` when the body is sent.
fn unsent_type(value: &Value) -> Result<()> {
    let name = match value {
        Value::Date(_) => "date",
        Value::DateTime(_) => "datetime",
        Value::List(items) => return items.iter().try_for_each(unsent_type),
        _ => return Ok(()),
    };
    Err(FormError::Type(format!(
        "Object of type {name} is not JSON serializable"
    )))
}

fn lookup_id(f: &Field, v: &Value) -> Result<Json> {
    match v {
        Value::Lookup(lv) => Ok(lv.id.clone()),
        Value::Int(i) => Ok(Json::from(*i)),
        Value::Bool(b) => Ok(Json::Bool(*b)),
        Value::Json(n @ Json::Number(_)) if n.is_i64() || n.is_u64() => Ok(n.clone()),
        other => Err(FormError::payload(format!(
            "{}: {} is not resolved to an id",
            f.label,
            other.py_repr()
        ))),
    }
}

fn problems(state: &FormState, skip: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = state
        .missing_required()
        .iter()
        .filter(|f| !skip.contains(&f.api_name))
        .map(|f| format!("{} is required", f.label))
        .collect();
    out.extend(state.unavailable_errors()?);
    out.extend(state.blocked_lines());
    Ok(out)
}

/// Refuse a form that cannot be sent as it stands: rules this engine cannot
/// evaluate faithfully always (`FormError::Payload` with those lines); with
/// `allow_incomplete` nothing else, otherwise also missing required fields
/// (except the API names in `skip`), unavailable choices and blocked lines.
///
/// Port of fsv.forms.payload._check.
pub fn check(state: &FormState, allow_incomplete: bool, skip: &[String]) -> Result<()> {
    let unsupported = state.unsupported_errors();
    if !unsupported.is_empty() {
        return Err(FormError::Payload {
            problems: unsupported,
        });
    }
    if allow_incomplete {
        return Ok(());
    }
    let problems = problems(state, skip)?;
    if !problems.is_empty() {
        return Err(FormError::Payload { problems });
    }
    Ok(())
}

/// The sent fields of a flat module form as `(top, custom_fields)`. `keys`
/// renames API names to request keys (`&[]`: none), `order` lists the
/// leading top-level keys (the rest follow in field order) and `status`
/// (label, value or id; `None`/null: not sent) is resolved to its choice id.
///
/// Port of fsv.forms.payload._module_body.
pub fn module_body(
    state: &FormState,
    keys: &[(&str, &str)],
    order: &[&str],
    status: Option<&Value>,
) -> Result<(Map<String, Json>, Map<String, Json>)> {
    let spec = &state.spec;
    let values = &state.evaluated.values;
    let mut top = Map::new();
    let mut custom = Map::new();
    for (name, v) in &state.payload_values() {
        if NEVER_FROM_STATE.contains(&name.as_str()) {
            continue;
        }
        let Some(f) = spec.get(name) else { continue };
        if !f.sent {
            continue; // display-only: its request key is not known
        }
        if f.kind == Kind::Attachment {
            return Err(FormError::payload(format!(
                "{}: file fields are not supported on this form",
                f.label
            )));
        }
        let enc = encode_value(spec, f, v, values)?;
        if f.is_custom {
            custom.insert(name.clone(), enc);
        } else {
            let key = keys
                .iter()
                .find(|(k, _)| k == name)
                .map_or(name.as_str(), |(_, v)| v);
            top.insert(key.to_string(), enc);
        }
    }
    if let Some(status) = status.filter(|s| !s.is_null()) {
        top.insert("status".to_string(), status_id(spec, status)?);
    }
    let mut ordered = Map::new();
    for k in order {
        if let Some(v) = top.get(*k) {
            ordered.insert(k.to_string(), v.clone());
        }
    }
    for (k, v) in top {
        if !ordered.contains_key(&k) {
            ordered.insert(k, v);
        }
    }
    Ok((ordered, custom))
}

fn status_id(spec: &FormSpec, status: &Value) -> Result<Json> {
    let Some(f) = spec.get("status") else {
        return Err(FormError::payload(
            "status schema is unavailable; no raw status permitted",
        ));
    };
    let Some(ch) = f.choice(status)? else {
        return Err(FormError::payload(format!(
            "status {} is not a valid choice",
            status.raw_repr()
        )));
    };
    Ok(ch
        .id
        .clone()
        .filter(|id| !id.is_null())
        .unwrap_or_else(|| Json::String(ch.value.clone())))
}

fn requester(state: &FormState, email: Option<&str>, name: Option<&str>) -> Result<String> {
    let (mut email, mut name) = (
        email.filter(|e| !e.is_empty()).map(str::to_string),
        name.map(str::to_string),
    );
    if email.is_none()
        && let Some(Value::Lookup(v)) = state.values.get("requester")
        && let Some(e) = v.email.as_ref().filter(|e| !e.is_empty())
    {
        email = Some(e.clone());
        if name.as_deref().is_none_or(str::is_empty) {
            name = Some(v.display.clone());
        }
    }
    let Some(email) = email else {
        return Err(FormError::payload("requester email is required"));
    };
    format_requester(&email, name.as_deref())
}

/// Keyword arguments of [`build_change_payload`] / [`build_ticket_payload`].
#[derive(Clone, Debug, Default)]
pub struct ModuleOptions {
    pub requester_email: Option<String>,
    pub requester_name: Option<String>,
    /// Status label/value/id; sent only when given.
    pub status: Option<Value>,
    /// Freshservice account id (`Json::Null`: from the form metadata).
    pub account_id: Json,
    pub allow_incomplete: bool,
}

fn module_payload(
    state: &FormState,
    kind: &str,
    keys: &[(&str, &str)],
    order: &[&str],
    opts: &ModuleOptions,
) -> Result<Map<String, Json>> {
    check(state, opts.allow_incomplete, &["requester".to_string()])?;
    let (top, custom) = module_body(state, keys, order, opts.status.as_ref())?;
    let mut body = top;
    body.insert("custom_fields".to_string(), Json::Object(custom));
    let email = requester(
        state,
        opts.requester_email.as_deref(),
        opts.requester_name.as_deref(),
    )?;
    body.insert("email".to_string(), Json::String(email));
    let token =
        build_br_validation_excludes(&state.evaluated, kind, &opts.account_id, &Json::Null)?;
    body.insert(BR_VALIDATION_EXCLUDES.to_string(), Json::String(token));
    Ok(body)
}

/// Body for `POST /api/_/changes`.
pub fn build_change_payload(state: &FormState, opts: &ModuleOptions) -> Result<Map<String, Json>> {
    module_payload(state, "change", &CHANGE_KEYS, &CHANGE_ORDER, opts)
}

/// Body for `POST /api/_/tickets`.
pub fn build_ticket_payload(state: &FormState, opts: &ModuleOptions) -> Result<Map<String, Json>> {
    module_payload(state, "ticket", &TICKET_KEYS, &TICKET_ORDER, opts)
}

/// Keyword arguments of [`build_sr_payload`].
#[derive(Clone, Debug, Default)]
pub struct SrOptions {
    /// A file field's API name -> uploaded attachment ids.
    pub field_attachments: IndexMap<String, Vec<Json>>,
    /// The general request attachments.
    pub attachments: Vec<Json>,
    pub account_id: Json,
    pub allow_incomplete: bool,
}

/// Body for `POST /api/_/catalog/service-items/<display_id>/place-request`.
///
/// IDs are trusted upload receipts: their file types cannot be verified
/// from opaque IDs.
pub fn build_sr_payload(
    item: Option<&ServiceItem>,
    state: &FormState,
    requester_email: &str,
    opts: &SrOptions,
) -> Result<Map<String, Json>> {
    let spec = &state.spec;
    let Some(item) = item.or(spec.item.as_ref()) else {
        return Err(FormError::payload("not a service-item form"));
    };
    let mut field_attachments = opts.field_attachments.clone();
    let attachments = &opts.attachments;
    let mut problems_out: Vec<String> = Vec::new();
    let visible: HashSet<&str> = state
        .visible_fields()
        .iter()
        .map(|f| f.api_name.as_str())
        .collect();
    let file_fields: Vec<&Field> = spec
        .fields()
        .iter()
        .filter(|f| f.kind == Kind::Attachment)
        .collect();
    let names: Vec<String> = field_attachments.keys().cloned().collect();
    for name in names {
        match spec.find(&name) {
            Some(target) if target.kind == Kind::Attachment => {
                if target.api_name != name {
                    if field_attachments.contains_key(&target.api_name) {
                        problems_out.push(format!(
                            "{}: file receipts supplied more than once",
                            target.label
                        ));
                    } else if let Some(ids) = field_attachments.shift_remove(&name) {
                        field_attachments.insert(target.api_name.clone(), ids);
                    }
                }
            }
            _ => problems_out.push(format!(
                "{} is not a file field of this item",
                fsv_py::py_repr_str(&name)
            )),
        }
    }
    for f in &file_fields {
        if let (Some(ids), Some(max)) = (field_attachments.get(&f.api_name), f.max_files)
            && ids.len() as i64 > max
        {
            problems_out.push(format!("{}: at most {max} file(s) are allowed", f.label));
        }
    }
    if !opts.allow_incomplete {
        let satisfied: Vec<String> = file_fields
            .iter()
            .filter(|f| {
                field_attachments
                    .get(&f.api_name)
                    .is_some_and(|ids| !ids.is_empty())
            })
            .map(|f| f.api_name.clone())
            .collect();
        problems_out.extend(problems(state, &satisfied)?);
        let need = state
            .evaluated
            .attachments_required
            .unwrap_or(item.attachment_mandatory);
        if need && attachments.is_empty() {
            problems_out.push("this item requires at least one attachment".to_string());
        }
    }
    if !attachments.is_empty() && !item.allow_attachments {
        problems_out.push("this item does not accept attachments".to_string());
    }
    if !problems_out.is_empty() {
        return Err(FormError::Payload {
            problems: problems_out,
        });
    }

    let values = &state.evaluated.values;
    let mut custom = Map::new();
    for (name, v) in &state.payload_values() {
        let Some(f) = spec.get(name) else { continue };
        if f.kind == Kind::Attachment || !f.sent {
            continue;
        }
        custom.insert(name.clone(), encode_value(spec, f, v, values)?);
    }
    for f in &file_fields {
        let shown = visible.contains(f.api_name.as_str());
        match field_attachments.get(&f.api_name) {
            Some(ids) if !ids.is_empty() && shown => {
                custom.insert(f.api_name.clone(), Json::Array(ids.clone()));
            }
            _ if shown && !is_empty(state.values.get(&f.api_name)) && !opts.allow_incomplete => {
                return Err(FormError::payload(format!(
                    "{}: upload the file(s) first and pass their attachment ids",
                    f.label
                )));
            }
            _ => {}
        }
    }
    let mut body = Map::new();
    body.insert(
        "email".to_string(),
        Json::String(format_requester(requester_email, None)?),
    );
    body.insert("custom_fields".to_string(), Json::Object(custom));
    let token = build_br_validation_excludes(
        &state.evaluated,
        "service_request",
        &opts.account_id,
        &item.id,
    )?;
    body.insert(BR_VALIDATION_EXCLUDES.to_string(), Json::String(token));
    body.insert("attachments".to_string(), Json::Array(attachments.clone()));
    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::spec::Choice;
    use crate::state::StateOptions;
    use crate::tz::TzEnv;

    #[test]
    fn base64_round_trip() {
        for s in [
            "",
            "f",
            "fo",
            "foo",
            "foob",
            "fooba",
            "foobar",
            "{\"a\":\"é\"}",
        ] {
            let enc = b64(s);
            assert_eq!(
                String::from_utf8(b64_decode(&enc).expect("decode")).expect("utf8"),
                s
            );
        }
        assert_eq!(b64(BR_HEADER), "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
    }

    #[test]
    fn helpers_match_python() {
        assert_eq!(
            format_requester(" a@x.io ", Some(" Ann ")).expect("req"),
            "Ann <a@x.io>"
        );
        assert_eq!(
            format_requester("nobody", None).unwrap_err().to_string(),
            "requester email 'nobody' is not an email address"
        );
        assert_eq!(
            text_to_html("a & b\nc\n\n\nd"),
            "<div>a &amp; b<br>c</div><div><br>d</div>"
        );
        assert_eq!(text_to_html("  <p>x</p>"), "  <p>x</p>");
        assert_eq!(text_to_html("  "), "<div></div>");
    }

    #[test]
    fn status_id_matches_python() {
        let spec = crate::py_golden::change_spec();
        for raw in [Value::from("Open"), Value::from("open"), Value::from(1)] {
            assert_eq!(status_id(&spec, &raw).expect("status"), json!(1));
        }
        assert_eq!(
            status_id(&spec, &Value::from("nope"))
                .unwrap_err()
                .to_string(),
            "status 'nope' is not a valid choice"
        );
        let mut status = Field::new("status", "Status", Kind::Dropdown);
        status.choices = vec![Choice::new("Open", "Open")];
        let no_ids = FormSpec::new("change", vec![status], None, vec![], None);
        assert_eq!(
            status_id(&no_ids, &Value::from("Open")).expect("status"),
            json!("Open")
        );
        let bare = FormSpec::new("change", vec![], None, vec![], None);
        assert_eq!(
            status_id(&bare, &Value::from("Open"))
                .unwrap_err()
                .to_string(),
            "status schema is unavailable; no raw status permitted"
        );
    }

    fn change_state() -> FormState {
        let mut prio = Field::new("priority", "Priority", Kind::Dropdown);
        let mut low = Choice::new("Low", "Low");
        low.id = Some(json!(1));
        prio.choices = vec![low];
        let mut cf = Field::new("cf_env", "Env", Kind::Text);
        cf.is_custom = true;
        let fields = vec![
            Field::new("subject", "Subject", Kind::Text),
            Field::new("group", "Group", Kind::LookupGroup),
            prio,
            cf,
        ];
        let spec = Arc::new(FormSpec::new("change", fields, None, vec![], None));
        let opts = StateOptions {
            tz: Some("UTC".into()),
            tz_env: Some(TzEnv::empty()),
            ..Default::default()
        };
        let mut s = FormState::new(spec, None, "new", None, opts).expect("state");
        s.set("priority", "low").expect("set");
        s.set("cf_env", "prod").expect("set");
        s.set("subject", "Patch").expect("set");
        s.set("group", 7).expect("set");
        s
    }

    #[test]
    fn change_payload_order_and_token() {
        let s = change_state();
        let opts = ModuleOptions {
            requester_email: Some("a@x.io".into()),
            ..Default::default()
        };
        let body = build_change_payload(&s, &opts).expect("payload");
        assert_eq!(
            body.keys().collect::<Vec<_>>(),
            vec![
                "subject",
                "priority",
                "group_id",
                "custom_fields",
                "email",
                BR_VALIDATION_EXCLUDES
            ]
        );
        assert_eq!(body["priority"], json!(1));
        assert_eq!(body["group_id"], json!(7));
        assert_eq!(body["custom_fields"], json!({"cf_env": "prod"}));
        let token = body[BR_VALIDATION_EXCLUDES].as_str().expect("token");
        assert_eq!(
            token,
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0="
        );
        let decoded = decode_br_validation_excludes(token).expect("decode");
        assert_eq!(decoded["kind"], json!("flat"));
        assert_eq!(decoded["optional"], json!([]));

        let err = build_change_payload(&s, &ModuleOptions::default()).unwrap_err();
        assert_eq!(err.to_string(), "requester email is required");
        let err = build_br_validation_excludes(&s.evaluated, "incident", &Json::Null, &Json::Null)
            .unwrap_err();
        assert_eq!(err.to_string(), "unknown form kind 'incident'");
        let err =
            build_br_validation_excludes(&s.evaluated, "sr", &Json::Null, &Json::Null).unwrap_err();
        assert_eq!(
            err.to_string(),
            "br_validation_excludes: a service request needs the item id"
        );
    }
}

#[cfg(test)]
mod py_tests;
