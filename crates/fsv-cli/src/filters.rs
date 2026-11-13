//! `--where` and `--query-hash` as `fsv RES ls` resolves them (`cli.py`).
//!
//! This is the CLI's resolver, not `query.py`'s: field labels and aliases
//! ("Agents", "Created"), periods ("Last 6 months"), `me` and empty values
//! are accepted here only.

use serde_json::{Value, json};

use fsv_api::Explain;
use fsv_core::resource::{PROBLEMS, Resource};
use fsv_core::{Client, Result, pystr};
use fsv_py::{py_repr_str, py_str};

use crate::lookup;
use crate::util::usage;

const PSEUDO_DATES: &[&str] = fsv_api::DATE_FIELDS;

const DATE_ALIASES: &[(&str, &str)] = &[
    ("created", "created_at"),
    ("created date", "created_at"),
    ("created on", "created_at"),
    ("created at", "created_at"),
    ("updated", "updated_at"),
    ("last modified", "updated_at"),
    ("modified", "updated_at"),
    ("updated at", "updated_at"),
    ("due", "due_by"),
    ("due by", "due_by"),
    ("due date", "due_by"),
    ("first response due", "fr_due_by"),
    ("first response due by", "fr_due_by"),
    ("fr due by", "fr_due_by"),
    ("closed", "closed_at"),
    ("closed at", "closed_at"),
    ("closed date", "closed_at"),
    ("closed on", "closed_at"),
    ("resolved", "resolved_at"),
    ("resolved at", "resolved_at"),
    ("resolved date", "resolved_at"),
    ("resolved on", "resolved_at"),
];

const FIELD_ALIASES: &[(&str, &str)] = &[
    ("assigned to", "agent"),
    ("agents", "agent"),
    ("groups", "group"),
    ("requesters", "requester"),
    ("departments", "department"),
    ("responder", "agent"),
];

const PERIODS: &[(&str, &str)] = &[
    ("today", "today"),
    ("yesterday", "yesterday"),
    ("this week", "week"),
    ("last week", "last_week"),
    ("this month", "month"),
    ("last month", "last_month"),
    ("last 2 months", "two_months"),
    ("last 6 months", "six_months"),
    ("last 24 hours", "twenty_four_hours"),
    ("last 7 days", "last_week"),
];

const EXTRA_PERIOD_TOKENS: &[&str] = &[
    "five_minutes",
    "fifteen_minutes",
    "thirty_minutes",
    "one_hour",
    "four_hours",
    "twelve_hours",
];

const ME: &[&str] = &["me", "(me)", "myself"];
const EMPTY: &[&str] = &["(empty)", "empty", "unassigned", "(none)", "none", "-"];

fn lookup_in<'a>(table: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn is_period_token(s: &str) -> bool {
    PERIODS.iter().any(|(_, v)| *v == s) || EXTRA_PERIOD_TOKENS.contains(&s)
}

/// Python `int(s)` for a `str.isdigit()` string; text when it does not fit.
fn int_or_text(s: &str) -> Value {
    pystr::parse_decimal(s).map_or_else(|| json!(s), |n| json!(n))
}

/// Port of fsv.cli._strip_quotes.
fn strip_quotes(value: &str) -> &str {
    let v = pystr::strip(value);
    let mut chars = v.chars();
    match (chars.next(), chars.next_back()) {
        (Some(a), Some(b)) if a == b && (a == '"' || a == '\'') => &v[1..v.len() - 1],
        _ => v,
    }
}

/// Port of fsv.cli._parse_where, which is `query._parse_where`'s strict
/// grammar: the field is the text before the first operator character,
/// and controls, an empty field or value, an unterminated quote or a value
/// starting with an operator character are refused.
pub fn parse_where(expr: &str) -> Result<(String, String, String)> {
    if expr.chars().any(|ch| (ch as u32) < 32) {
        return Err(usage(
            "invalid filter; controls and multiline expressions are not supported",
        ));
    }
    let invalid = || {
        usage(&format!(
            "invalid filter {}; expected FIELD followed by = != >= <= > < or ~=",
            py_repr_str(expr)
        ))
    };
    let text = pystr::strip(expr);
    let start = text.find(['<', '>', '=', '!', '~']).ok_or_else(invalid)?;
    if start == 0 {
        return Err(invalid());
    }
    let rest = &text[start..];
    let op = [">=", "<=", "!=", "~=", "=", ">", "<"]
        .into_iter()
        .find(|op| rest.starts_with(op))
        .ok_or_else(invalid)?;
    let field = pystr::strip(&text[..start]);
    let value = pystr::strip(&rest[op.len()..]);
    if field.is_empty() || value.is_empty() {
        return Err(usage(
            "invalid filter; field and value are required (quote literal empty text)",
        ));
    }
    let first = value.chars().next().unwrap_or_default();
    if first == '"' || first == '\'' {
        if value.chars().count() < 2 || !value.ends_with(first) {
            return Err(usage("invalid filter; unterminated quoted value"));
        }
    } else if "<>=!~".contains(first) {
        return Err(usage(
            "invalid filter operator sequence; quote a literal value starting with an operator",
        ));
    }
    Ok((
        field.to_string(),
        op.to_string(),
        strip_quotes(value).to_string(),
    ))
}

/// Port of fsv.cli._where_operator.
fn where_operator(op: &str) -> Result<&'static str> {
    match op {
        "!=" => Ok("is_not_in"),
        ">" | ">=" | "<" | "<=" => Err(usage(&format!(
            "operator {} not supported; inequalities only work for date fields (created_at, updated_at, etc.). Use = or !=",
            py_repr_str(op)
        ))),
        "~=" => Err(usage("operator ~= (contains) not supported; use = or !=")),
        "=" => Ok("is_in"),
        _ => Err(usage(&format!(
            "unknown filter operator {}; no equality fallback",
            py_repr_str(op)
        ))),
    }
}

/// Port of fsv.cli._query_item.
fn query_item(
    condition: &str,
    operator: &str,
    value: Value,
    type_: &str,
    explain: &mut Vec<Explain>,
    field: &str,
    raw: &str,
) -> Value {
    explain.push(Explain {
        field: field.into(),
        condition: condition.into(),
        operator: operator.into(),
        type_name: type_.into(),
        value: raw.into(),
    });
    json!({"value": value, "condition": condition, "operator": operator, "type": type_})
}

/// Port of fsv.cli._date_condition: the web's period labels, else
/// `query.date_condition`.
pub fn date_condition(
    field_name: &str,
    op: &str,
    raw: &str,
    explain: &mut Vec<Explain>,
    shown: &str,
    res_name: &str,
) -> Result<Value> {
    let trimmed = pystr::strip(raw);
    let token = lookup_in(PERIODS, &pystr::casefold(trimmed))
        .or_else(|| is_period_token(trimmed).then_some(trimmed));
    if token.is_some() && fsv_api::TICKET_ONLY_DATES.contains(&field_name) {
        return Err(usage(&format!(
            "no filter-panel period exists for {field_name}; use on:/last:/next: PERIOD or a YYYY-MM-DD range (nothing was sent)"
        )));
    }
    if let Some(token) = token {
        if !matches!(op, "=" | ">=") {
            return Err(usage(
                "relative date periods support only = or >=; no inverse period wire is proven",
            ));
        }
        if res_name == PROBLEMS.name && (token != "today" || field_name != "created_at") {
            return Err(usage(
                "only Created=today is a proven relative date filter on problems (other shapes returned 0 rows silently); use a created_at>=YYYY-MM-DD and created_at<=YYYY-MM-DD range. Nothing was sent",
            ));
        }
        return Ok(query_item(
            field_name,
            "is_greater_than",
            json!(token),
            "default",
            explain,
            shown,
            raw,
        ));
    }
    fsv_api::date_condition(res_name, field_name, op, raw, explain, shown).map_err(|e| usage(&e.0))
}

/// Port of fsv.cli._where_schema_field: exact name or label, then the
/// web's plural/alias labels, then the singular of a plural.
pub fn where_schema_field(sch: &Value, field_text: &str) -> Result<Option<Value>> {
    if let Some(f) = lookup::find_schema_field(sch, field_text)? {
        return Ok(Some(f));
    }
    let folded = pystr::casefold(&pystr::collapse_ws(field_text));
    if let Some(alias) = lookup_in(FIELD_ALIASES, &folded)
        && let Some(f) = lookup::find_schema_field(sch, alias)?
    {
        return Ok(Some(f));
    }
    for cut in ["s", "es"] {
        let (n, k) = (folded.chars().count(), cut.len());
        if folded.ends_with(cut) && n > k + 1 {
            let keep = field_text.chars().count().saturating_sub(k);
            let stem: String = field_text.chars().take(keep).collect();
            if let Some(f) = lookup::find_schema_field(sch, &stem)? {
                return Ok(Some(f));
            }
        }
    }
    Ok(None)
}

fn field_labels(sch: &Value) -> String {
    let mut labels: Vec<String> = lookup::schema_fields(sch)
        .iter()
        .filter(|f| f.get("label").is_some_and(fsv_py::truthy))
        .map(|f| lookup::s(f, "label"))
        .collect();
    labels.sort();
    labels.dedup();
    labels.truncate(25);
    labels.join(", ")
}

/// Port of fsv.cli._resolve_where: one `FIELD<op>VALUE` as a query_hash item.
pub fn resolve_where(
    c: &Client,
    res: &Resource,
    sch: &Value,
    expr: &str,
    explain: &mut Vec<Explain>,
) -> Result<Value> {
    let (field_text, op, raw) = parse_where(expr)?;
    let date_name = if PSEUDO_DATES.contains(&field_text.as_str()) {
        Some(field_text.as_str())
    } else {
        lookup_in(
            DATE_ALIASES,
            &pystr::casefold(&pystr::collapse_ws(&field_text)),
        )
    };
    if let Some(name) = date_name {
        return date_condition(name, &op, &raw, explain, &field_text, res.name);
    }
    if field_text.ends_with("_id") {
        if matches!(op.as_str(), ">" | "<" | ">=" | "<=" | "~=") {
            return Err(usage(&format!(
                "operator {} not supported on ID fields; use = or !=",
                py_repr_str(&op)
            )));
        }
        let operator = if op == "=" { "is_in" } else { "is_not_in" };
        let val = if pystr::is_digit(&raw) {
            int_or_text(&raw)
        } else {
            json!(raw)
        };
        return Ok(query_item(
            &field_text,
            operator,
            json!([val]),
            "default",
            explain,
            &field_text,
            &raw,
        ));
    }
    let Some(f) = where_schema_field(sch, &field_text)? else {
        return Err(usage(&format!(
            "field not found: {field_text}; filter by a label shown on the form ({}) or see `fsv {} fields`",
            field_labels(sch),
            res.name
        )));
    };
    let name = f
        .get("name")
        .filter(|v| fsv_py::truthy(v))
        .map_or_else(|| field_text.clone(), py_str);
    if PSEUDO_DATES.contains(&name.as_str()) {
        return date_condition(&name, &op, &raw, explain, &field_text, res.name);
    }
    if let Some(typed) = fsv_api::typed_condition(res.name, &f, &op, &raw, explain, &field_text)
        .map_err(|e| usage(&e.0))?
    {
        return Ok(typed);
    }
    let type_ = if f.get("default_field").is_some_and(fsv_py::truthy) {
        "default"
    } else {
        "custom_field"
    };
    let ftype = lookup::s_or(&f, "field_type");
    fsv_api::filter_proof::require_filterable_field(&f).map_err(|e| fsv_core::Error::value(e.0))?;
    let operator = where_operator(&op)?;
    let folded = pystr::casefold(pystr::strip(&raw));
    let has_choices = f.get("choices").is_some_and(fsv_py::truthy);

    if name == "requester" {
        if ME.contains(&folded.as_str()) {
            let me = crate::cmd::resource::current_user(c)?;
            let Some(id) = me.get("id").filter(|v| fsv_py::truthy(v)).cloned() else {
                return Err(usage(
                    "cannot tell who you are (agents/me is not readable); use your email instead of 'me'",
                ));
            };
            let who = me
                .get("name")
                .filter(|v| fsv_py::truthy(v))
                .cloned()
                .unwrap_or_else(|| json!("Me"));
            let shown = format!("Me ({})", py_str(&id));
            return Ok(query_item(
                "requester_id",
                operator,
                json!([{"id": id, "name": who}]),
                "default",
                explain,
                &name,
                &shown,
            ));
        }
        let (id, who) = lookup::resolve_requester(c, &raw)?;
        let shown = format!("{who} ({})", py_str(&id));
        return Ok(query_item(
            "requester_id",
            operator,
            json!([{"id": id, "name": who}]),
            "default",
            explain,
            &name,
            &shown,
        ));
    }
    if name == "agent" {
        let condition = if res.name == "tickets" {
            "responder_id"
        } else {
            "agent_id"
        };
        if ME.contains(&folded.as_str()) {
            return Ok(query_item(
                condition,
                operator,
                json!(["0"]),
                "default",
                explain,
                &name,
                "Me",
            ));
        }
        if EMPTY.contains(&folded.as_str()) {
            return Ok(query_item(
                condition,
                operator,
                json!(["-1"]),
                "default",
                explain,
                &name,
                "Unassigned",
            ));
        }
        let agent_id = lookup::resolve_agent(c, &raw)?;
        return Ok(query_item(
            condition,
            operator,
            json!([agent_id]),
            "default",
            explain,
            &name,
            &agent_id,
        ));
    }
    if name == "group" {
        if EMPTY.contains(&folded.as_str()) {
            return Ok(query_item(
                "group_id",
                operator,
                json!(["-1"]),
                "default",
                explain,
                &name,
                "(empty)",
            ));
        }
        let val = if has_choices {
            py_str(&lookup::choice_value(&f, &raw, "default")?)
        } else {
            lookup::resolve_group(c, &raw)?
        };
        return Ok(query_item(
            "group_id",
            operator,
            json!([val]),
            "default",
            explain,
            &name,
            &val,
        ));
    }
    if EMPTY.contains(&folded.as_str())
        && (has_choices || ftype.contains("dropdown") || name == "department")
    {
        let has_choice = f
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(|chs| {
                chs.iter().any(|ch| {
                    let text = ["value", "name"]
                        .iter()
                        .find_map(|k| ch.get(*k).filter(|v| fsv_py::truthy(v)))
                        .map(py_str)
                        .unwrap_or_default();
                    pystr::casefold(pystr::strip(&text)) == folded
                })
            });
        if !has_choice {
            let cond = if name == "department" {
                "department_id"
            } else {
                name.as_str()
            };
            return Ok(query_item(
                cond,
                operator,
                json!(["-1"]),
                type_,
                explain,
                &name,
                "(empty)",
            ));
        }
    }
    // Ticket type filters take the runtime choice name, never the numeric ID.
    if let Some(named) = fsv_api::filter_proof::name_valued_choice(&f, &raw)
        .map_err(|e| fsv_core::Error::value(e.0))?
    {
        return Ok(query_item(
            &name,
            operator,
            json!([named.clone()]),
            type_,
            explain,
            &name,
            &named,
        ));
    }
    if has_choices
        || ftype.contains("dropdown")
        || ftype == "default_status"
        || ftype == "default_priority"
    {
        let val = lookup::choice_value(&f, &raw, type_)?;
        let cond = if name == "department" && type_ == "default" {
            "department_id"
        } else {
            name.as_str()
        };
        let shown = py_str(&val);
        return Ok(query_item(
            cond,
            operator,
            json!([val]),
            type_,
            explain,
            &name,
            &shown,
        ));
    }
    let val = if pystr::is_digit(&raw) && ftype.ends_with("number") {
        int_or_text(&raw)
    } else {
        json!(raw)
    };
    Ok(query_item(
        &name,
        operator,
        json!([val]),
        type_,
        explain,
        &name,
        &raw,
    ))
}

/// Port of fsv.cli._load_query_hash: always percent-decoded once.
pub fn load_query_hash(raw: &str) -> Result<Vec<Value>> {
    match serde_json::from_str::<Value>(&pystr::unquote(raw)) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(_) => Err(usage("--query-hash must be a JSON array")),
        Err(e) => Err(usage(&format!("invalid --query-hash JSON: {e}"))),
    }
}

/// Port of fsv.cli._merge_date_ranges (`query.merge_date_ranges`).
pub fn merge_date_ranges(query: Vec<Value>) -> Result<Vec<Value>> {
    fsv_api::merge_date_ranges(query).map_err(|e| fsv_core::Error::value(e.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(field: &str, condition: &str, operator: &str, value: &str) -> Explain {
        Explain {
            field: field.into(),
            condition: condition.into(),
            operator: operator.into(),
            type_name: "default".into(),
            value: value.into(),
        }
    }

    #[test]
    fn parse_where_strips_value_quotes_only() {
        let got = parse_where("  \"Due Date\" >= '2026-01-01' ").unwrap();
        assert_eq!(
            got,
            ("\"Due Date\"".into(), ">=".into(), "2026-01-01".into())
        );
        let got = parse_where("a=b=c").unwrap();
        assert_eq!(got, ("a".into(), "=".into(), "b=c".into()));
    }

    #[test]
    fn parse_where_is_strict() {
        for bad in ["=x", "a=", "a==b", "a='b", "a\n=b"] {
            assert!(parse_where(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn date_condition_periods_and_ranges() {
        let mut ex = Vec::new();
        for (field, op, raw, res) in [
            ("created_at", "<=", "last 7 days", "changes"),
            ("due_by", "=", "Five_minutes", "changes"),
            ("due_by", "=", "today", "problems"),
            ("created_at", "=", "on:yesterday", "problems"),
        ] {
            assert!(
                date_condition(field, op, raw, &mut ex, field, res).is_err(),
                "{raw}"
            );
        }
        let a = date_condition(
            "created_at",
            "<=",
            "2026-01-31",
            &mut ex,
            "Created",
            "changes",
        )
        .unwrap();
        let b = date_condition(
            "created_at",
            ">",
            "2026-01-31",
            &mut ex,
            "Created",
            "changes",
        )
        .unwrap();
        let c = date_condition("due_by", ">=", "five_minutes", &mut ex, "due", "changes").unwrap();
        let d = date_condition("created_at", "=", "today", &mut ex, "Created", "problems").unwrap();
        assert_eq!(
            a,
            json!({"value": {"to": "2026-01-31"}, "condition": "created_at", "operator": "is_in_the_range", "type": "default"})
        );
        assert_eq!(
            b,
            json!({"value": {"from": "2026-02-01"}, "condition": "created_at", "operator": "is_in_the_range", "type": "default"})
        );
        assert_eq!(
            c,
            json!({"value": "five_minutes", "condition": "due_by", "operator": "is_greater_than", "type": "default"})
        );
        assert_eq!(d["value"], "today");
        assert_eq!(
            ex,
            [
                row("Created", "created_at", "is_in_the_range", "2026-01-31"),
                row(
                    "Created",
                    "created_at",
                    "is_in_the_range",
                    "2026-01-31 (no strict operator exists: sent as from 2026-02-01, whole days)"
                ),
                row("due", "due_by", "is_greater_than", "five_minutes"),
                row("Created", "created_at", "is_greater_than", "today"),
            ]
        );
    }

    #[test]
    fn load_query_hash_always_unquotes() {
        assert_eq!(
            load_query_hash(r#"[{"v":"50%25 off"}]"#).unwrap(),
            vec![json!({"v": "50% off"})]
        );
    }

    #[test]
    fn where_schema_field_aliases_and_plurals() {
        let sch = json!({"fields": [
            {"name": "status", "label": "Status"},
            {"name": "agent", "label": "Agent"},
            {"name": "class", "label": "Class"},
            {"name": "box", "label": "Box"},
        ]});
        for (text, want) in [
            ("Statuses", Some("status")),
            ("Agents", Some("agent")),
            ("responder", Some("agent")),
            ("Classes", Some("class")),
            ("Boxes", Some("box")),
            ("ss", Some("class")),
            ("Assigned  To", Some("agent")),
            ("Agentes", Some("agent")),
            ("zzz", None),
            ("xs", None),
        ] {
            let got = where_schema_field(&sch, text).unwrap();
            let name = got.as_ref().and_then(|f| f["name"].as_str());
            assert_eq!(name, want, "{text}");
        }
    }
}
