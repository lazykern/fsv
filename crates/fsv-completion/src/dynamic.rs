//! `fsv._complete` and the completers in `fsv.completion`.

use serde_json::{Map, Value, json};

use fsv_core::config;
use fsv_core::pystr::casefold;
use fsv_core::resource::{self, Resource};

type Obj = Map<String, Value>;
type Pairs = Vec<(String, String)>;

const FALLBACK_FIELDS: &[(&str, &str, &str, &str)] = &[
    ("requester", "Requester", "default_requester", "default"),
    ("agent", "Agent", "default_agent", "default"),
    ("group", "Group", "default_group", "default"),
    ("status", "Status", "default_status", "default"),
    ("priority", "Priority", "default_priority", "default"),
    ("created_at", "Created", "default_date", "default"),
    ("updated_at", "Updated", "default_date", "default"),
];

const FALLBACK_CHOICES: &[(&str, &[&str])] = &[
    ("status", &["Open", "Pending", "Resolved", "Closed"]),
    ("priority", &["Low", "Medium", "High", "Urgent"]),
    ("impact", &["Low", "Medium", "High"]),
    ("risk", &["Low", "Medium", "High", "Very High"]),
];

const DATE_EXAMPLES: &[&str] = &["2025-01-01T00:00:00+07:00", "2025-12-31T23:59:59+07:00"];

const OPERATORS: &[&str] = &[">=", "<=", "!=", "~=", "=", ">", "<"];

fn fallback_choices(field: &str) -> Vec<String> {
    FALLBACK_CHOICES
        .iter()
        .find(|(k, _)| *k == field)
        .map(|(_, v)| v.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

fn truthy(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| fsv_py::truthy(v))
}

/// `str(d.get(k) or "")`.
fn text(d: &Obj, key: &str) -> String {
    truthy(d.get(key)).map(fsv_py::py_str).unwrap_or_default()
}

/// `str(d.get(a) or d.get(b) or ... or "")`.
fn first_text(d: &Obj, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| truthy(d.get(*k)))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

/// Port of fsv.completion._match: casefolded prefix test.
fn matches(text: &str, incomplete: &str) -> bool {
    casefold(text).starts_with(&casefold(incomplete))
}

fn load_doc(paths: Vec<std::path::PathBuf>) -> Option<Obj> {
    paths.into_iter().find_map(|p| fsv_core::cache::load(&p).0)
}

fn objects(v: Option<&Value>) -> Vec<Obj> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_object).cloned().collect())
        .unwrap_or_default()
}

fn cached_fields(res: &Resource) -> Vec<Obj> {
    let Some(doc) = load_doc(config::schema_cache_candidates(res.name, None)) else {
        return Vec::new();
    };
    let data = truthy(doc.get("data")).or_else(|| doc.get("fields"));
    if let Some(Value::Array(_)) = data {
        return objects(data);
    }
    objects(truthy(doc.get("fields")))
}

fn field_scope(f: &Obj) -> &'static str {
    if truthy(f.get("default_field")).is_some() {
        "default"
    } else {
        "custom"
    }
}

fn field_name(f: &Obj) -> String {
    text(f, "name")
}

fn field_label(f: &Obj) -> String {
    first_text(f, &["label", "name"])
}

fn field_type(f: &Obj) -> String {
    text(f, "field_type")
}

fn field_help(f: &Obj) -> String {
    let t = field_type(f);
    format!(
        "{} · {} · {}",
        field_label(f),
        if t.is_empty() { "?" } else { &t },
        field_scope(f)
    )
}

fn fields_or_fallback(res: &Resource) -> Vec<Obj> {
    let fields = cached_fields(res);
    if !fields.is_empty() {
        return fields;
    }
    FALLBACK_FIELDS
        .iter()
        .map(|(name, label, ftype, scope)| {
            let v = json!({"name": name, "label": label, "field_type": ftype, "default_field": *scope == "default"});
            v.as_object().cloned().unwrap_or_default()
        })
        .collect()
}

/// Port of fsv.completion._field_matches: prefix match on name or label.
/// `cli._field_matches` and `query._field_matches` are substring matches and
/// live in fsv-cli (`lookup::field_matches`) and fsv-api.
fn field_matches(f: &Obj, incomplete: &str) -> bool {
    matches(&field_name(f), incomplete) || matches(&field_label(f), incomplete)
}

fn find_field(res: &Resource, name: &str) -> Option<Obj> {
    let mut raw = name;
    let mut scope = None;
    if let Some((maybe, rest)) = name.split_once(':')
        && (maybe == "default" || maybe == "custom")
    {
        scope = Some(maybe);
        raw = rest;
    }
    let folded = casefold(raw);
    let mut fields = fields_or_fallback(res);
    match scope {
        Some("default") => fields.retain(|f| truthy(f.get("default_field")).is_some()),
        Some("custom") => fields.retain(|f| truthy(f.get("default_field")).is_none()),
        _ => {}
    }
    if let Some(f) = fields
        .iter()
        .find(|f| casefold(&field_name(f)) == folded || casefold(&field_label(f)) == folded)
    {
        return Some(f.clone());
    }
    let hits: Vec<&Obj> = fields
        .iter()
        .filter(|f| {
            casefold(&field_name(f)).contains(&folded)
                || casefold(&field_label(f)).contains(&folded)
        })
        .collect();
    if hits.len() == 1 {
        Some(hits[0].clone())
    } else {
        None
    }
}

fn choice_text(c: &Value) -> String {
    c.as_object()
        .map(|c| first_text(c, &["value", "name", "label", "id"]))
        .unwrap_or_default()
}

fn choice_list(f: &Obj) -> Vec<Value> {
    truthy(f.get("choices"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn choices_for(res: &Resource, field: &str) -> Vec<String> {
    let Some(f) = find_field(res, field) else {
        return fallback_choices(field);
    };
    let choices: Vec<String> = choice_list(&f)
        .iter()
        .map(choice_text)
        .filter(|x| !x.is_empty())
        .collect();
    if choices.is_empty() {
        fallback_choices(&field_name(&f))
    } else {
        choices
    }
}

fn where_parts(incomplete: &str) -> Option<(String, &'static str, String)> {
    OPERATORS
        .iter()
        .find(|op| incomplete.contains(**op))
        .map(|op| {
            let (left, right) = incomplete.split_once(*op).unwrap_or((incomplete, ""));
            (
                fsv_core::pystr::strip(left).to_string(),
                *op,
                right.to_string(),
            )
        })
}

fn pair(a: impl Into<String>, b: impl Into<String>) -> (String, String) {
    (a.into(), b.into())
}

fn complete_pairs(incomplete: &str, pairs: &[(&str, &str)]) -> Pairs {
    let mut out: Pairs = pairs
        .iter()
        .filter(|(v, _)| matches(v, incomplete))
        .map(|(v, h)| pair(*v, *h))
        .collect();
    out.push(pair(incomplete, ""));
    out
}

fn completion_case(candidate: &str, query: &str) -> String {
    if candidate.starts_with(query) {
        return candidate.to_string();
    }
    if casefold(candidate).starts_with(&casefold(query)) {
        let rest: String = candidate.chars().skip(query.chars().count()).collect();
        return format!("{query}{rest}");
    }
    candidate.to_string()
}

/// The tenant calls the completers make; `None` is a failed request.
trait Remote {
    fn autocomplete(&self, kind: &str, query: &str, params: Option<Obj>) -> Option<Vec<Value>>;
    fn lookup_choices(&self, link: &str, query: &str) -> Option<Vec<Value>>;
}

struct LiveRemote;

impl Remote for LiveRemote {
    fn autocomplete(&self, kind: &str, query: &str, params: Option<Obj>) -> Option<Vec<Value>> {
        fsv_core::get_client()
            .and_then(|c| c.autocomplete(kind, query, params))
            .ok()
    }

    fn lookup_choices(&self, link: &str, query: &str) -> Option<Vec<Value>> {
        fsv_core::get_client()
            .and_then(|c| c.lookup_choices(link, query))
            .ok()
    }
}

struct Env<'a> {
    network: bool,
    remote: &'a dyn Remote,
}

impl Env<'_> {
    fn remote_user_values(&self, kind: &str, query: &str) -> Pairs {
        let query = fsv_core::pystr::strip(query);
        if !self.network || query.chars().count() < 2 {
            return Vec::new();
        }
        let params = (kind == "requesters").then(|| {
            let mut m = Map::new();
            m.insert("all_users".into(), json!("true"));
            m
        });
        let Some(rows) = self.remote.autocomplete(kind, query, params) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for row in rows.iter().take(10).filter_map(Value::as_object) {
            let candidates = [
                text(row, "email"),
                text(row, "details"),
                text(row, "value"),
                text(row, "name"),
            ];
            let mut value = candidates
                .iter()
                .find(|x| !x.is_empty() && casefold(x).starts_with(&casefold(query)))
                .cloned()
                .unwrap_or_default();
            if value.is_empty() {
                value = first_text(row, &["value", "name", "email"]);
            }
            let detail = first_text(row, &["value", "name", "email", "details", "id", "user_id"]);
            if !value.is_empty() {
                out.push((completion_case(&value, query), detail));
            }
        }
        out
    }
}

fn cached_list(doc: Option<Obj>, key: &str) -> Vec<Obj> {
    let Some(doc) = doc else { return Vec::new() };
    let data = truthy(doc.get("data"));
    let list = match data {
        Some(Value::Object(d)) => d.get(key).cloned(),
        Some(other) => Some(other.clone()),
        None => doc.get(key).cloned(),
    };
    match list {
        Some(Value::Array(_)) => objects(list.as_ref()),
        _ => objects(truthy(doc.get(key))),
    }
}

fn cached_filters(res: &Resource) -> Vec<(String, String)> {
    if res.filters_path.is_none() {
        return Vec::new();
    }
    cached_list(
        load_doc(config::filters_cache_candidates(res.name, None)),
        "filters",
    )
    .iter()
    .filter(|f| truthy(f.get("id")).is_some() || truthy(f.get("name")).is_some())
    .map(|f| (text(f, "id"), text(f, "name")))
    .collect()
}

fn cached_groups() -> Vec<Obj> {
    cached_list(load_doc(config::groups_cache_candidates(None)), "groups")
}

fn complete_field_names(res: &Resource, incomplete: &str) -> Pairs {
    let mut out: Pairs = fields_or_fallback(res)
        .iter()
        .filter(|f| !field_name(f).is_empty() && field_matches(f, incomplete))
        .map(|f| (field_name(f), field_help(f)))
        .collect();
    out.push(pair(incomplete, ""));
    out
}

fn complete_choice_field_names(res: &Resource, incomplete: &str) -> Pairs {
    let mut out = Pairs::new();
    for f in fields_or_fallback(res) {
        let name = field_name(&f);
        if name.is_empty() || !field_matches(&f, incomplete) {
            continue;
        }
        let choices = choice_list(&f);
        if !choices.is_empty() {
            out.push((
                name,
                format!("{} · {} choices", field_help(&f), choices.len()),
            ));
        }
    }
    out.push(pair(incomplete, ""));
    out
}

fn complete_planning_field_names(res: &Resource, incomplete: &str) -> Pairs {
    let mut out = Pairs::new();
    for f in fields_or_fallback(res) {
        let (name, label) = (field_name(&f), field_label(&f));
        if !fsv_forms::spec::is_planning_type(&field_type(&f)) {
            continue;
        }
        if !name.is_empty() && (matches(&name, incomplete) || matches(&label, incomplete)) {
            out.push((
                if label.is_empty() {
                    name.clone()
                } else {
                    label.clone()
                },
                name.clone(),
            ));
            out.push((name, label));
        }
    }
    out.push(pair(incomplete, ""));
    out
}

fn complete_lookup_kind(res: &Resource, incomplete: &str) -> Pairs {
    let base = [
        ("requester", "requester autocomplete"),
        ("agent", "agent autocomplete"),
        ("group", "group lookup"),
    ];
    let mut out: Pairs = base
        .iter()
        .filter(|(v, _)| matches(v, incomplete))
        .map(|(v, h)| pair(*v, *h))
        .collect();
    out.extend(complete_field_names(res, incomplete));
    out
}

fn group_names(incomplete: &str) -> Pairs {
    cached_groups()
        .iter()
        .map(|g| (text(g, "name"), text(g, "description")))
        .filter(|(n, _)| !n.is_empty() && matches(n, incomplete))
        .collect()
}

/// `kind` is the `--kind` value Python hands over in a `_MockCtx`.
fn complete_lookup_query(env: &Env, res: &Resource, kind: &str, incomplete: &str) -> Pairs {
    let mut out = match kind {
        "requester" | "requesters" => env.remote_user_values("requesters", incomplete),
        "agent" | "agents" => env.remote_user_values("agents", incomplete),
        "group" | "groups" => group_names(incomplete),
        "" => Pairs::new(),
        _ => choices_for(res, kind)
            .into_iter()
            .filter(|c| matches(c, incomplete))
            .map(|c| (c, kind.to_string()))
            .collect(),
    };
    out.push(pair(incomplete, ""));
    out
}

fn complete_filter_name(res: &Resource, incomplete: &str) -> Pairs {
    let mut out: Pairs = cached_filters(res)
        .into_iter()
        .filter(|(id, name)| {
            !id.is_empty() && (matches(id, incomplete) || matches(name, incomplete))
        })
        .map(|(id, name)| {
            (
                id,
                if name.is_empty() {
                    "saved view".into()
                } else {
                    name
                },
            )
        })
        .collect();
    out.push(pair(incomplete, ""));
    out
}

fn complete_group_query(incomplete: &str) -> Pairs {
    let mut out = group_names(incomplete);
    out.push(pair(incomplete, ""));
    out
}

fn complete_set(env: &Env, res: &Resource, incomplete: &str) -> Pairs {
    let incomplete = incomplete.trim_matches(|c| c == '"' || c == '\'');
    let mut out = Pairs::new();
    if !incomplete.contains('=') {
        for f in fields_or_fallback(res) {
            let (name, label) = (field_name(&f), field_label(&f));
            if name.is_empty() || !field_matches(&f, incomplete) {
                continue;
            }
            if matches(&name, incomplete) {
                out.push((format!("{name}="), field_help(&f)));
            } else {
                let rest: String = label.chars().skip(incomplete.chars().count()).collect();
                let t = field_type(&f);
                out.push((
                    format!("{incomplete}{rest}="),
                    format!(
                        "{name} · {} · {}",
                        if t.is_empty() { "?" } else { &t },
                        field_scope(&f)
                    ),
                ));
            }
        }
        return out;
    }
    let (field, raw_value) = incomplete.split_once('=').unwrap_or((incomplete, ""));
    let Some(f) = find_field(res, field) else {
        out.push(pair(incomplete, ""));
        return out;
    };
    let name = field_name(&f);
    let ftype = field_type(&f);
    if ftype.contains("checkbox") {
        for v in ["true", "false"] {
            if matches(v, raw_value) {
                out.push((format!("{field}={v}"), String::new()));
            }
        }
    } else if [
        "planned_start_date",
        "planned_end_date",
        "created_at",
        "updated_at",
    ]
    .contains(&name.as_str())
    {
        for v in DATE_EXAMPLES {
            if matches(v, raw_value) {
                out.push((format!("{field}={v}"), "ISO-8601".into()));
            }
        }
    } else if ftype.contains("lookup") {
        if env.network && fsv_core::pystr::strip(raw_value).chars().count() >= 2 {
            let link = f
                .get("lookup_config")
                .and_then(Value::as_object)
                .map(|c| text(c, "link"))
                .unwrap_or_default();
            if !link.is_empty() {
                out.extend(lookup_values(env, field, raw_value, &link));
            } else {
                let mut seen = Vec::new();
                for kind in ["agents", "requesters"] {
                    for (v, detail) in env.remote_user_values(kind, raw_value) {
                        if !seen.contains(&v) {
                            seen.push(v.clone());
                            out.push((format!("{field}={v}"), detail));
                        }
                    }
                }
            }
        }
    } else {
        let choices: Vec<String> = choice_list(&f)
            .iter()
            .map(choice_text)
            .filter(|c| !c.is_empty())
            .collect();
        for c in choices {
            if matches(&c, raw_value) {
                out.push((format!("{field}={c}"), name.clone()));
            }
        }
    }
    out.push(pair(incomplete, ""));
    out
}

fn lookup_values(env: &Env, field: &str, raw_value: &str, link: &str) -> Pairs {
    let Some(mut rows) = env.remote.lookup_choices(link, raw_value) else {
        return Vec::new();
    };
    let r = casefold(raw_value);
    let starts = |x: &Value, k: &str| {
        x.as_object()
            .is_some_and(|o| casefold(&text(o, k)).starts_with(&r))
    };
    rows.sort_by_key(|x| {
        if starts(x, "email") || starts(x, "name") {
            0
        } else {
            1
        }
    });
    let mut out = Pairs::new();
    for row in rows.iter().filter_map(Value::as_object) {
        let (name, email) = (text(row, "name"), text(row, "email"));
        let skip = raw_value.chars().count();
        if !email.is_empty() && matches(&email, raw_value) {
            out.push((
                format!(
                    "{field}={raw_value}{}",
                    email.chars().skip(skip).collect::<String>()
                ),
                name,
            ));
        } else if !name.is_empty() && matches(&name, raw_value) {
            out.push((
                format!(
                    "{field}={raw_value}{}",
                    name.chars().skip(skip).collect::<String>()
                ),
                email,
            ));
        }
        if out.len() >= 20 {
            break;
        }
    }
    out
}

fn complete_where(env: &Env, res: &Resource, incomplete: &str) -> Pairs {
    let mut out = Pairs::new();
    let Some((field, op, raw_value)) = where_parts(incomplete) else {
        for f in fields_or_fallback(res) {
            let name = field_name(&f);
            if !name.is_empty() && field_matches(&f, incomplete) {
                out.push((format!("{name}="), field_help(&f)));
            }
        }
        for (name, help) in [
            ("created_at>=", "created lower bound"),
            ("created_at<=", "created upper bound"),
            ("updated_at>=", "updated lower bound"),
        ] {
            if matches(name, incomplete) {
                out.push(pair(name, help));
            }
        }
        return out;
    };
    let f = find_field(res, &field);
    let name = f.as_ref().map_or(field.clone(), field_name);
    if [
        "created_at",
        "updated_at",
        "due_by",
        "planned_start_date",
        "planned_end_date",
    ]
    .contains(&name.as_str())
    {
        for v in DATE_EXAMPLES {
            if matches(v, &raw_value) {
                out.push((format!("{field}{op}{v}"), "ISO-8601 timestamp".into()));
            }
        }
    } else if name == "requester" || name == "agent" {
        let kind = if name == "requester" {
            "requesters"
        } else {
            "agents"
        };
        for (v, detail) in env.remote_user_values(kind, &raw_value) {
            out.push((format!("{field}{op}{v}"), detail));
        }
    } else {
        for c in choices_for(res, &field) {
            if matches(&c, &raw_value) {
                out.push((format!("{field}{op}{c}"), name.clone()));
            }
        }
    }
    out.push(pair(incomplete, ""));
    out
}

fn complete_update_agent_id(env: &Env, incomplete: &str) -> Pairs {
    if !env.network || fsv_core::pystr::strip(incomplete).chars().count() < 2 {
        return vec![pair(incomplete, "")];
    }
    env.remote_user_values("agents", incomplete)
}

fn complete_update_group_id(incomplete: &str) -> Pairs {
    let mut out = Pairs::new();
    for g in cached_groups() {
        let (gid, name, desc) = (text(&g, "id"), text(&g, "name"), text(&g, "description"));
        let detail = format!(
            "id={gid}{}",
            if desc.is_empty() {
                String::new()
            } else {
                format!(" · {desc}")
            }
        );
        if !name.is_empty() && matches(&name, incomplete) {
            out.push((name.clone(), detail));
        }
        if !gid.is_empty() && matches(&gid, incomplete) {
            out.push((gid, name));
        }
    }
    out.push(pair(incomplete, ""));
    out
}

/// Port of fsv.completion.complete_update_choice: labels and ids of one
/// field's choices, then the fallback labels for status/priority/impact/risk.
/// The scripts pass the field after `--`.
pub fn complete_update_choice(res: &Resource, field_name: &str, incomplete: &str) -> Pairs {
    let mut out = Pairs::new();
    let mut emitted: Vec<String> = Vec::new();
    if let Some(f) = fields_or_fallback(res)
        .into_iter()
        .find(|f| self::field_name(f) == field_name)
    {
        for c in choice_list(&f).iter().filter_map(Value::as_object) {
            let cid = text(c, "id");
            let label = {
                let l = first_text(c, &["value", "name", "label"]);
                if l.is_empty() { cid.clone() } else { l }
            };
            let id_help = if cid.is_empty() {
                String::new()
            } else {
                format!("id={cid}")
            };
            for (value, help) in [(label.clone(), id_help), (cid.clone(), label.clone())] {
                if !value.is_empty() && !emitted.contains(&value) && matches(&value, incomplete) {
                    emitted.push(value.clone());
                    out.push((value, help));
                }
            }
        }
    }
    if FALLBACK_CHOICES.iter().any(|(k, _)| *k == field_name) {
        for label in fallback_choices(field_name) {
            if !emitted.contains(&label) && matches(&label, incomplete) {
                emitted.push(label.clone());
                out.push((label, field_name.to_string()));
            }
        }
    }
    out.push(pair(incomplete, ""));
    out
}

/// Port of fsv.completion.complete_help_topic.
pub fn complete_help_topic(topics: &[&str], incomplete: &str) -> Pairs {
    let pairs: Vec<(&str, &str)> = topics.iter().map(|t| (*t, "help topic")).collect();
    complete_pairs(incomplete, &pairs)
}

fn static_table(name: &str) -> &'static [(&'static str, &'static str)] {
    crate::model::STATIC_COMPLETERS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(&[], |(_, v)| *v)
}

/// Port of fsv.completion.complete_format.
fn complete_format(incomplete: &str) -> Pairs {
    complete_pairs(incomplete, static_table("complete_format"))
}

/// Port of fsv.completion.complete_store.
fn complete_store(incomplete: &str) -> Pairs {
    complete_pairs(incomplete, static_table("complete_store"))
}

/// Port of fsv.completion.complete_shell.
fn complete_shell(incomplete: &str) -> Pairs {
    complete_pairs(incomplete, static_table("complete_shell"))
}

/// Port of fsv.completion.complete_sort_order.
fn complete_sort_order(incomplete: &str) -> Pairs {
    complete_pairs(incomplete, static_table("complete_sort_order"))
}

/// Port of fsv.completion.complete_cache_target.
fn complete_cache_target(incomplete: &str) -> Pairs {
    complete_pairs(incomplete, static_table("complete_cache_target"))
}

/// Port of fsv.completion.complete_search_sort.
fn complete_search_sort(incomplete: &str) -> Pairs {
    complete_pairs(incomplete, static_table("complete_search_sort"))
}

/// Port of fsv.completion.complete_duplicate_mode.
fn complete_duplicate_mode(incomplete: &str) -> Pairs {
    complete_pairs(incomplete, static_table("complete_duplicate_mode"))
}

/// Port of fsv.completion.complete_config_key. The script generator's table
/// (`STATIC_COMPLETERS`) lists only `completion.network`; this one has three.
fn complete_config_key(incomplete: &str) -> Pairs {
    complete_pairs(
        incomplete,
        &[
            (
                "completion.network",
                "enable remote requester/agent completion [on|off]",
            ),
            ("defaults.requester", "requester email for changes create"),
            ("forms.workspace_id", "workspace whose business rules apply"),
        ],
    )
}

/// Port of fsv.completion.complete_config_value.
fn complete_config_value(incomplete: &str) -> Pairs {
    let mut out: Pairs = ["on", "off", "true", "false", "1", "0"]
        .iter()
        .filter(|v| matches(v, incomplete))
        .map(|v| pair(*v, ""))
        .collect();
    out.push(pair(incomplete, ""));
    out
}

type Plain = fn(&str) -> Pairs;

/// The `PLAIN` table of `_complete._completions`, minus the three completers
/// that read caches or the network.
fn plain(name: &str) -> Option<Plain> {
    Some(match name {
        "complete_format" => complete_format,
        "complete_store" => complete_store,
        "complete_shell" => complete_shell,
        "complete_sort_order" => complete_sort_order,
        "complete_config_key" => complete_config_key,
        "complete_config_value" => complete_config_value,
        "complete_cache_target" => complete_cache_target,
        "complete_search_sort" => complete_search_sort,
        "complete_duplicate_mode" => complete_duplicate_mode,
        _ => return None,
    })
}

fn extract_flag(tokens: &[String], flags: &[&str]) -> String {
    for (i, tok) in tokens.iter().enumerate() {
        if flags.contains(&tok.as_str()) && i + 1 < tokens.len() {
            return tokens[i + 1].clone();
        }
        for f in flags {
            if let Some(rest) = tok.strip_prefix(&format!("{f}=")) {
                return rest.to_string();
            }
        }
    }
    String::new()
}

/// `_completions`: `(value, help)` candidates for one completer.
pub fn complete(
    completer: &str,
    resource_name: &str,
    token: &str,
    extra: &[String],
    network: bool,
) -> Vec<(String, String)> {
    let env = Env {
        network,
        remote: &LiveRemote,
    };
    complete_in(&env, completer, resource_name, token, extra)
}

fn complete_in(
    env: &Env,
    completer: &str,
    resource_name: &str,
    token: &str,
    extra: &[String],
) -> Pairs {
    match completer {
        "complete_update_agent_id" => return complete_update_agent_id(env, token),
        "complete_update_group_id" => return complete_update_group_id(token),
        "complete_group_query" => return complete_group_query(token),
        _ => {}
    }
    if let Some(f) = plain(completer) {
        return f(token);
    }
    let Some(res) = resource::lookup(resource_name) else {
        return Vec::new();
    };
    match completer {
        "complete_where" => complete_where(env, res, token),
        "complete_set" => complete_set(env, res, token),
        "complete_filter_name" => complete_filter_name(res, token),
        "complete_field_names" => complete_field_names(res, token),
        "complete_choice_field_names" => complete_choice_field_names(res, token),
        "complete_planning_field_names" => complete_planning_field_names(res, token),
        "complete_lookup_kind" => complete_lookup_kind(res, token),
        "complete_lookup_query" => {
            complete_lookup_query(env, res, &extract_flag(extra, &["--kind", "-k"]), token)
        }
        "complete_update_choice" => match extra.first() {
            Some(field) => complete_update_choice(res, field, token),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Port of fsv._complete.main: `args` are `<shell> <completer> <resource|-> <token> [-- tokens...]`;
/// returns the lines to print.
pub fn run(args: &[String], network: bool) -> String {
    let env = Env {
        network,
        remote: &LiveRemote,
    };
    run_in(&env, args)
}

fn run_in(env: &Env, args: &[String]) -> String {
    if args.len() < 4 {
        return String::new();
    }
    let (shell, completer, resource_name, token) = (&args[0], &args[1], &args[2], &args[3]);
    let extra: &[String] = match args[4..].iter().position(|a| a == "--") {
        Some(i) => &args[4 + i + 1..],
        None => &[],
    };
    let mut out = String::new();
    for (value, help) in complete_in(env, completer, resource_name, token, extra) {
        if value.is_empty() || &value == token {
            continue;
        }
        if shell == "fish" && !help.is_empty() {
            out.push_str(&format!("{value}\t{help}\n"));
        } else {
            out.push_str(&value);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod golden;
#[cfg(test)]
mod units;

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn static_completer_filters_by_prefix() {
        assert_eq!(
            run(&args(&["fish", "complete_format", "-", "c"]), false),
            "csv\tcomma-separated values\n"
        );
        assert_eq!(
            run(&args(&["bash", "complete_format", "-", "t"]), false),
            "table\ntsv\n"
        );
    }

    #[test]
    fn too_few_args_prints_nothing() {
        assert_eq!(run(&args(&["fish", "complete_format", "-"]), false), "");
    }

    #[test]
    fn unknown_resource_prints_nothing() {
        assert_eq!(
            run(&args(&["fish", "complete_where", "nope", "st"]), false),
            ""
        );
    }

    #[test]
    fn extract_flag_forms() {
        assert_eq!(
            extract_flag(&args(&["fsv", "--kind", "group"]), &["--kind", "-k"]),
            "group"
        );
        assert_eq!(
            extract_flag(&args(&["-k=agent"]), &["--kind", "-k"]),
            "agent"
        );
        assert_eq!(extract_flag(&args(&["--kind"]), &["--kind", "-k"]), "");
    }

    #[test]
    fn where_parts_uses_operator_order() {
        assert_eq!(
            where_parts("created_at>=2025"),
            Some(("created_at".into(), ">=", "2025".into()))
        );
        assert_eq!(
            where_parts(" status = Open"),
            Some(("status".into(), "=", " Open".into()))
        );
        assert_eq!(where_parts("status"), None);
    }
}
