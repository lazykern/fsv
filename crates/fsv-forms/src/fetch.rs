//! Where the web gets a form's business rules, and how to read the
//! response (`fsv.forms.fetch`).
//!
//! The forms crate does no HTTP: [`business_rules_query`] builds the
//! request the web UI sends when it opens a form, and
//! [`parse_business_rules`] reads either response shape.
//!
//! The server *filters* the rules for the context in the query (module,
//! catalog item, form type, requester, record), so fetch them with the same
//! parameters the web uses.
//!
//! Two response shapes exist:
//!
//! * `{"business_rules": [...], "user": {...}}`: what `ember_api=true`
//!   returns; every rule has `condition_data` and `action_data`. This is the
//!   one to evaluate.
//! * `{"business_rule": [...]}`: the plain `GET .../business_rules` listing
//!   (admin view): every rule of the workspace with `position` but
//!   **without** conditions or actions. It cannot be evaluated (`evaluable`
//!   is false and [`crate::RulesEngine`] refuses it).

use serde_json::{Map, Value as Json, json};

use crate::errors::{FormError, Result};
use crate::pytext::{jstr, truthy};

const FORM_TYPES: [(&str, &str); 4] = [
    ("new", "new_form"),
    ("edit", "edit_form"),
    ("new_form", "new_form"),
    ("edit_form", "edit_form"),
];
// service-catalog item rules name custom fields "customField.<name>"; module
// forms use "customFields.<name>" (as the web sends cf_name_prefix)
const SR_MODULES: [&str; 2] = ["service_request", "sr"];

/// `GET /api/_/ws/<workspace>/business_rules?...` as the web sends it.
#[derive(Clone, Debug, PartialEq)]
pub struct BusinessRulesQuery {
    pub workspace_id: Json,
    pub params: Vec<(String, String)>,
}

/// `urllib.parse.quote_plus(s, safe=".")`.
fn quote_plus(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl BusinessRulesQuery {
    pub fn path(&self) -> String {
        format!("/api/_/ws/{}/business_rules", jstr(&self.workspace_id))
    }

    /// `urlencode(params, safe=".")`.
    pub fn query_string(&self) -> String {
        self.params
            .iter()
            .map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v)))
            .collect::<Vec<_>>()
            .join("&")
    }

    /// Path and query (prefixed with `base`, e.g. `https://acme.freshservice.com`).
    pub fn url(&self, base: &str) -> String {
        format!(
            "{}{}?{}",
            base.trim_end_matches('/'),
            self.path(),
            self.query_string()
        )
    }
}

fn null_str(v: &Json) -> String {
    match v {
        Json::Null => "null".to_string(),
        Json::String(s) if s.is_empty() => "null".to_string(),
        other => jstr(other),
    }
}

fn blank_str(v: &Json) -> String {
    if v.is_null() { String::new() } else { jstr(v) }
}

/// Keyword arguments of [`business_rules_query`] (`Json::Null` is Python `None`).
#[derive(Clone, Debug)]
pub struct BusinessRulesOptions {
    /// Required: the workspace the form belongs to.
    pub workspace_id: Json,
    /// The catalog item's `id` (service requests), else null.
    pub item_id: Json,
    /// `new`/`new_form` or `edit`/`edit_form`.
    pub form_type: String,
    /// User ids when known (empty otherwise).
    pub requester: Json,
    pub requested_for: Json,
    /// `None` sends `null` (what the agent UI sends).
    pub user_type: Option<String>,
    /// The record's display id on an edit form.
    pub module_id: Json,
    pub include_workspace: bool,
    /// Defaults to true except for service requests (where the web omits it).
    pub filter_by_default_form_action: Option<bool>,
    /// Defaults to `customField.` for service requests and `customFields.` otherwise.
    pub cf_name_prefix: Option<String>,
}

impl Default for BusinessRulesOptions {
    fn default() -> Self {
        BusinessRulesOptions {
            workspace_id: Json::Null,
            item_id: Json::Null,
            form_type: "new_form".to_string(),
            requester: Json::Null,
            requested_for: Json::Null,
            user_type: None,
            module_id: Json::Null,
            include_workspace: false,
            filter_by_default_form_action: None,
            cf_name_prefix: None,
        }
    }
}

impl BusinessRulesOptions {
    pub fn workspace(workspace_id: impl Into<Json>) -> Self {
        BusinessRulesOptions {
            workspace_id: workspace_id.into(),
            ..Default::default()
        }
    }
}

/// The business-rules request for one form context (`module_type`:
/// `change`, `ticket`, `service_request`, ...).
pub fn business_rules_query(
    module_type: &str,
    opts: &BusinessRulesOptions,
) -> Result<BusinessRulesQuery> {
    let Some(form_type) = FORM_TYPES
        .iter()
        .find(|(k, _)| *k == opts.form_type)
        .map(|(_, v)| *v)
    else {
        return Err(FormError::Value(
            "form_type must be one of ['edit', 'edit_form', 'new', 'new_form']".to_string(),
        ));
    };
    if opts.workspace_id.is_null() || opts.workspace_id.as_str() == Some("") {
        return Err(FormError::Value(
            "workspace_id is required (the workspace the form belongs to)".to_string(),
        ));
    }
    let sr = SR_MODULES.contains(&module_type);
    let filter = opts.filter_by_default_form_action.unwrap_or(!sr);
    let prefix = opts
        .cf_name_prefix
        .clone()
        .unwrap_or_else(|| if sr { "customField." } else { "customFields." }.into());
    let p = |k: &str, v: String| (k.to_string(), v);
    let mut params = vec![
        p("ember_api", "true".into()),
        p(
            "module_type",
            if sr {
                "service_request".into()
            } else {
                module_type.to_string()
            },
        ),
        p("item_id", null_str(&opts.item_id)),
        p("requester", blank_str(&opts.requester)),
        p("requested_for", blank_str(&opts.requested_for)),
        p(
            "include_workspace",
            if opts.include_workspace {
                "true"
            } else {
                "false"
            }
            .into(),
        ),
    ];
    if filter {
        params.push(p("filter_by_default_form_action", "true".into()));
    }
    let user_type = opts
        .user_type
        .as_deref()
        .map_or(Json::Null, |u| Json::String(u.to_string()));
    params.extend([
        p("form_type", form_type.into()),
        p("user_type", null_str(&user_type)),
        p("module_id", blank_str(&opts.module_id)),
        p("cf_name_prefix", prefix),
    ]);
    Ok(BusinessRulesQuery {
        workspace_id: opts.workspace_id.clone(),
        params,
    })
}

/// Which response shape [`parse_business_rules`] read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulesShape {
    /// `business_rules` + `user`: evaluable.
    Form,
    /// `business_rule`: no conditions/actions.
    Listing,
    /// A bare list.
    List,
}

impl RulesShape {
    pub fn as_str(self) -> &'static str {
        match self {
            RulesShape::Form => "form",
            RulesShape::Listing => "listing",
            RulesShape::List => "list",
        }
    }
}

/// A parsed business-rules response; `raw` is what [`crate::RulesEngine`] takes.
#[derive(Clone, Debug, PartialEq)]
pub struct RulesResponse {
    pub rules: Vec<Json>,
    pub user: Option<Map<String, Json>>,
    pub shape: RulesShape,
    pub evaluable: bool,
    pub raw: Json,
}

fn type_name(v: &Json) -> &'static str {
    match v {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(n) if n.is_f64() => "float",
        Json::Number(_) => "int",
        Json::String(_) => "str",
        Json::Array(_) => "list",
        Json::Object(_) => "dict",
    }
}

/// `list(data.get(key) or [])`.
fn py_list(v: Option<&Json>) -> Result<Vec<Json>> {
    let Some(v) = v.filter(|v| truthy(v)) else {
        return Ok(vec![]);
    };
    match v {
        Json::Array(a) => Ok(a.clone()),
        Json::Object(m) => Ok(m.keys().map(|k| Json::String(k.clone())).collect()),
        Json::String(s) => Ok(s.chars().map(|c| Json::String(c.to_string())).collect()),
        other => Err(FormError::Type(format!(
            "'{}' object is not iterable",
            type_name(other)
        ))),
    }
}

/// Python `needle in r`.
fn py_contains(r: &Json, needle: &str) -> Result<bool> {
    match r {
        Json::Object(m) => Ok(m.contains_key(needle)),
        Json::String(s) => Ok(s.contains(needle)),
        Json::Array(a) => Ok(a.iter().any(|x| x.as_str() == Some(needle))),
        other => Err(FormError::Type(format!(
            "argument of type '{}' is not iterable",
            type_name(other)
        ))),
    }
}

/// Read either response shape (see the module docs).
pub fn parse_business_rules(data: &Json) -> Result<RulesResponse> {
    let (rules, shape, user) = match data {
        Json::Object(m) => {
            let (rules, shape) = if m.contains_key("business_rules") {
                (py_list(m.get("business_rules"))?, RulesShape::Form)
            } else if m.contains_key("business_rule") {
                (py_list(m.get("business_rule"))?, RulesShape::Listing)
            } else {
                return Err(FormError::Value(
                    "unrecognised business_rules response (no business_rules / business_rule key)"
                        .to_string(),
                ));
            };
            (
                rules,
                shape,
                m.get("user").and_then(Json::as_object).cloned(),
            )
        }
        Json::Array(a) => (a.clone(), RulesShape::List, None),
        _ => {
            return Err(FormError::Value(
                "unrecognised business_rules response".to_string(),
            ));
        }
    };
    let mut evaluable = true;
    for r in &rules {
        if !(py_contains(r, "condition_data")? || py_contains(r, "action_data")?) {
            evaluable = false;
            break;
        }
    }
    let raw = if shape == RulesShape::Listing && !rules.is_empty() && !evaluable {
        json!({"business_rule": rules})
    } else {
        match &user {
            Some(u) => json!({"business_rules": rules, "user": u}),
            None => json!({"business_rules": rules}),
        }
    };
    Ok(RulesResponse {
        rules,
        user,
        shape,
        evaluable,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_param_order_matches_python() {
        let q = business_rules_query("ticket", &BusinessRulesOptions::workspace(2)).expect("query");
        assert_eq!(
            q.url("https://acme.freshservice.com/"),
            "https://acme.freshservice.com/api/_/ws/2/business_rules?ember_api=true&module_type=ticket&item_id=null\
             &requester=&requested_for=&include_workspace=false&filter_by_default_form_action=true&form_type=new_form\
             &user_type=null&module_id=&cf_name_prefix=customFields."
        );

        let opts = BusinessRulesOptions {
            item_id: json!(55),
            requester: json!(7),
            requested_for: json!("a b/c&d"),
            user_type: Some(String::new()),
            module_id: json!(9),
            form_type: "edit".into(),
            include_workspace: true,
            ..BusinessRulesOptions::workspace(3)
        };
        assert_eq!(
            business_rules_query("sr", &opts)
                .expect("query")
                .query_string(),
            "ember_api=true&module_type=service_request&item_id=55&requester=7&requested_for=a+b%2Fc%26d\
             &include_workspace=true&form_type=edit_form&user_type=null&module_id=9&cf_name_prefix=customField."
        );

        let opts = BusinessRulesOptions {
            cf_name_prefix: Some("x.y é~".into()),
            filter_by_default_form_action: Some(false),
            user_type: Some("agent".into()),
            ..BusinessRulesOptions::workspace("1")
        };
        assert_eq!(
            business_rules_query("change", &opts)
                .expect("query")
                .query_string(),
            "ember_api=true&module_type=change&item_id=null&requester=&requested_for=&include_workspace=false\
             &form_type=new_form&user_type=agent&module_id=&cf_name_prefix=x.y+%C3%A9~"
        );
    }

    #[test]
    fn query_errors_match_python() {
        let bad = BusinessRulesOptions {
            form_type: "new_formx".into(),
            ..BusinessRulesOptions::workspace(2)
        };
        assert_eq!(
            business_rules_query("ticket", &bad)
                .unwrap_err()
                .to_string(),
            "form_type must be one of ['edit', 'edit_form', 'new', 'new_form']"
        );
        for ws in [Json::Null, json!("")] {
            let err =
                business_rules_query("ticket", &BusinessRulesOptions::workspace(ws)).unwrap_err();
            assert_eq!(
                err.to_string(),
                "workspace_id is required (the workspace the form belongs to)"
            );
        }
    }

    #[test]
    fn parse_shapes() {
        let form = json!({"business_rules": [{"condition_data": {}, "action_data": []}], "user": {"id": 1}});
        let r = parse_business_rules(&form).expect("form");
        assert_eq!(
            (r.shape, r.evaluable, r.rules.len()),
            (RulesShape::Form, true, 1)
        );
        assert_eq!(r.raw, form);
        assert_eq!(r.user, form["user"].as_object().cloned());

        let listing = json!({"business_rule": [{"id": 1, "position": 1}]});
        let r = parse_business_rules(&listing).expect("listing");
        assert_eq!((r.shape, r.evaluable), (RulesShape::Listing, false));
        assert_eq!(r.raw, listing);

        let r = parse_business_rules(&json!({"business_rule": []})).expect("empty listing");
        assert_eq!(r.raw, json!({"business_rules": []}));
        assert!(r.evaluable);

        let r = parse_business_rules(&json!([{"action_data": []}])).expect("list");
        assert_eq!(
            (r.shape, r.evaluable, r.user.is_none()),
            (RulesShape::List, true, true)
        );
        assert_eq!(r.raw, json!({"business_rules": [{"action_data": []}]}));

        let r = parse_business_rules(&json!({"business_rules": null, "user": "x"}))
            .expect("null rules");
        assert_eq!((r.rules.len(), r.user.is_none()), (0, true));

        assert_eq!(
            parse_business_rules(&json!({"rules": []}))
                .unwrap_err()
                .to_string(),
            "unrecognised business_rules response (no business_rules / business_rule key)"
        );
        assert_eq!(
            parse_business_rules(&json!("x")).unwrap_err().to_string(),
            "unrecognised business_rules response"
        );
        assert!(matches!(
            parse_business_rules(&json!([1])),
            Err(FormError::Type(_))
        ));
    }
}
