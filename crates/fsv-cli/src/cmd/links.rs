//! `changes assets ...` and `changes associations ...`
//! (`cli.assets_resource`, `asset_categories_resource`,
//! `change_associations_resource`).

use std::sync::{LazyLock, Mutex};

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_api::associations::{self, SendOpts};
use fsv_api::create;
use fsv_api::service::{Api, DiskJournal};
use fsv_core::output::emit_json;
use fsv_core::resource::{CHANGES, TICKETS, format_id, parse_id};
use fsv_core::term::{self, Column, Table, err, out};
use fsv_core::{Client, Error};

use crate::types::{OutputFormat, effective};
use crate::util::{CmdResult, cid, client, emit_fmt, fail, fail_exc, no_input, stdin_tty, usage};

type Category = Map<String, Value>;

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .filter(|x| fsv_py::truthy(x))
        .map(fsv_py::py_str)
        .unwrap_or_default()
}

fn or_dash(v: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|k| v.get(*k).filter(|x| fsv_py::truthy(x)).cloned())
        .unwrap_or_else(|| json!("-"))
}

fn casefold(t: &str) -> String {
    fsv_core::pystr::casefold(t)
}

fn is_digits(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}

fn py_int(v: &Value) -> Result<i64, Error> {
    let text = fsv_py::py_str(v);
    fsv_core::pystr::parse_decimal(text.trim()).ok_or_else(|| {
        Error::value(format!(
            "invalid literal for int() with base 10: {}",
            fsv_py::py_repr_str(&text)
        ))
    })
}

fn change_label(id: i64) -> String {
    format_id(&json!({"id": id}), &CHANGES)
}

fn ticket_label(t: &Value) -> String {
    let hid = s(t, "human_display_id");
    if hid.is_empty() {
        format_id(t, &TICKETS)
    } else {
        hid
    }
}

// ---------------------------------------------------------------------------
// assets

/// `_prefixed_value`.
fn prefixed_value(ci: &Value, prefix: &str) -> Option<Value> {
    let obj = ci.as_object()?;
    let mut keys: Vec<&String> = obj.keys().filter(|k| k.starts_with(prefix)).collect();
    keys.sort();
    let type_id = s(ci, "ci_type_id");
    let preferred = format!("{prefix}{type_id}");
    if !type_id.is_empty()
        && let Some(pos) = keys.iter().position(|k| **k == preferred)
    {
        let k = keys.remove(pos);
        keys.insert(0, k);
    }
    keys.into_iter()
        .map(|k| &obj[k])
        .find(|v| !v.is_null() && **v != json!(""))
        .cloned()
}

/// `_asset_display`.
fn asset_display(asset: &Value) -> Value {
    let ci = match asset.get("config_item") {
        Some(c) if c.is_object() => c,
        _ => asset,
    };
    let id = ["display_id", "id"]
        .iter()
        .find_map(|k| ci.get(*k).filter(|x| fsv_py::truthy(x)).cloned())
        .unwrap_or_else(|| asset.get("id").cloned().unwrap_or(Value::Null));
    let truthy_or_dash = |v: Option<Value>| v.filter(fsv_py::truthy).unwrap_or_else(|| json!("-"));
    json!({
        "id": id,
        "name": or_dash(ci, &["name"]),
        "type": or_dash(ci, &["asset_type", "ci_type_name"]),
        "used_by": or_dash(ci, &["user_name", "used_by"]),
        "location": or_dash(ci, &["location_name"]),
        "state": truthy_or_dash(prefixed_value(ci, "asset_state_")),
        "serial": truthy_or_dash(prefixed_value(ci, "serial_number_")),
    })
}

static SELECT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<select[^>]*name="ci_type_id"[^>]*>(.*?)</select>"#).expect("regex")
});
static OPTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<option(?:\s+value(?:="([^"]*)")?)?[^>]*>(.*?)</option>"#).expect("regex")
});
static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("regex"));
static CATEGORIES: Mutex<Option<Vec<Category>>> = Mutex::new(None);

/// `_fetch_asset_categories`.
fn fetch_asset_categories(c: &Client) -> Result<Vec<Category>, Error> {
    if let Some(cached) = CATEGORIES.lock().ok().and_then(|g| g.clone()) {
        return Ok(cached);
    }
    let url = format!("https://{}/cmdb/items", c.tenant().domain);
    let html = Api::fetch(c, &url)?.text();
    let Some(block) = SELECT_RE.captures(&html).and_then(|m| m.get(1)) else {
        return Err(usage("could not load asset categories from /cmdb/items"));
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cap in OPTION_RE.captures_iter(block.as_str()) {
        let ci_type_id = cap.get(1).map_or("", |m| m.as_str()).to_string();
        let raw = cap.get(2).map_or("", |m| m.as_str());
        let stripped = TAG_RE.replace_all(raw, "");
        let label = fsv_core::pystr::html_unescape(&stripped)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if label.is_empty() {
            continue;
        }
        let key = casefold(&label);
        if !seen.insert(key.clone()) {
            continue;
        }
        let all = ci_type_id.is_empty();
        let mut m = Map::new();
        m.insert(
            "name".into(),
            json!(if all && key == "all assets" {
                "All assets".to_string()
            } else {
                label
            }),
        );
        m.insert(
            "filter".into(),
            json!(if all {
                "all_assets".to_string()
            } else {
                ci_type_id.clone()
            }),
        );
        m.insert("ci_type_id".into(), json!(ci_type_id));
        out.push(m);
    }
    if let Ok(mut g) = CATEGORIES.lock() {
        *g = Some(out.clone());
    }
    Ok(out)
}

fn cat_name(cat: &Category) -> String {
    cat.get("name").map(fsv_py::py_str).unwrap_or_default()
}

/// `_resolve_asset_category`.
fn resolve_asset_category(value: &str, c: &Client) -> Result<Category, Error> {
    let raw = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if raw.is_empty() {
        return Err(usage("empty asset category"));
    }
    let folded = casefold(&raw);
    if folded == "all" || folded == "all assets" {
        let mut m = Map::new();
        m.insert("name".into(), json!("All assets"));
        m.insert("filter".into(), json!("all_assets"));
        m.insert("ci_type_id".into(), json!(""));
        return Ok(m);
    }
    let cats = fetch_asset_categories(c)?;
    let shown = |v: &[&Category], n: usize| {
        v.iter()
            .take(n)
            .map(|c| cat_name(c))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let exact: Vec<&Category> = cats
        .iter()
        .filter(|c| casefold(&cat_name(c)) == folded)
        .collect();
    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    let repr = fsv_py::py_repr_str(value);
    if exact.len() > 1 {
        return Err(usage(&format!(
            "multiple asset categories match {repr}: {}",
            shown(&exact, 5)
        )));
    }
    let matches: Vec<&Category> = cats
        .iter()
        .filter(|c| casefold(&cat_name(c)).contains(&folded))
        .collect();
    match matches.len() {
        1 => Ok(matches[0].clone()),
        0 => Err(usage(&format!(
            "unknown asset category: {repr}. Try --list-categories"
        ))),
        _ => Err(usage(&format!(
            "multiple asset categories match {repr}: {}",
            shown(&matches, 8)
        ))),
    }
}

/// `_filter_assets_by_category`.
fn filter_by_category(items: Vec<Value>, category: Option<&Category>) -> Vec<Value> {
    let Some(cat) = category.filter(|c| c.get("filter") != Some(&json!("all_assets"))) else {
        return items;
    };
    let wanted = casefold(&cat_name(cat));
    items
        .into_iter()
        .filter(|item| {
            let row = if item.get("type").is_some() && item.get("name").is_some() {
                item.clone()
            } else {
                asset_display(item)
            };
            casefold(&s(&row, "type")) == wanted
        })
        .collect()
}

fn in_category(category: Option<&Category>) -> String {
    category.map_or(String::new(), |c| {
        format!(" in category {}", fsv_py::py_repr_str(&cat_name(c)))
    })
}

fn search_assets(
    c: &Client,
    id: i64,
    query: &str,
    page: i64,
    per_page: i64,
) -> Result<Value, Error> {
    create::search_assets_for_change(c, id, query, page, per_page)
}

fn assets_of(data: &Value) -> Vec<Value> {
    data.get("assets")
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `_resolve_change_asset_id`.
fn resolve_asset_id(
    id: i64,
    value: &str,
    c: &Client,
    associated: bool,
    category: Option<&Category>,
) -> Result<i64, Error> {
    let raw = fsv_core::pystr::strip(value);
    if is_digits(raw) {
        return py_int(&json!(raw)).map_err(fail_exc);
    }
    let folded = casefold(raw);
    let repr = fsv_py::py_repr_str(value);
    if associated {
        let rows: Vec<Value> = create::get_change_assets(c, id)
            .map_err(fail_exc)?
            .iter()
            .map(asset_display)
            .collect();
        let rows = filter_by_category(rows, category);
        let shown = |v: &[&Value]| {
            v.iter()
                .take(5)
                .map(|r| {
                    format!(
                        "{} [{}]",
                        fsv_py::py_str(&r["name"]),
                        fsv_py::py_str(&r["id"])
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let exact: Vec<&Value> = rows
            .iter()
            .filter(|r| s(r, "id") == raw || casefold(&s(r, "name")) == folded)
            .collect();
        if exact.len() == 1 {
            return py_int(&exact[0]["id"]).map_err(fail_exc);
        }
        if exact.len() > 1 {
            return Err(usage(&format!(
                "multiple associated assets match {repr}: {}",
                shown(&exact)
            )));
        }
        let matches: Vec<&Value> = rows
            .iter()
            .filter(|r| casefold(&s(r, "name")).contains(&folded))
            .collect();
        return match matches.len() {
            1 => py_int(&matches[0]["id"]).map_err(fail_exc),
            0 => Err(usage(&format!(
                "asset not associated on {}{}: {repr}",
                change_label(id),
                in_category(category)
            ))),
            _ => Err(usage(&format!(
                "multiple associated assets match {repr}: {}",
                shown(&matches)
            ))),
        };
    }
    let assets = assets_of(&search_assets(c, id, raw, 1, 50).map_err(fail_exc)?);
    let assets = filter_by_category(assets, category);
    let shown = |v: &[&Value]| {
        v.iter()
            .take(5)
            .map(|a| {
                let name = s(a, "name");
                let did = s(a, "display_id");
                format!(
                    "{} [{}]",
                    if name.is_empty() { "-" } else { &name },
                    if did.is_empty() { "?" } else { &did }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let exact: Vec<&Value> = assets
        .iter()
        .filter(|a| s(a, "display_id") == raw || casefold(&s(a, "name")) == folded)
        .collect();
    if exact.len() == 1 {
        return py_int(&exact[0]["display_id"]).map_err(fail_exc);
    }
    if exact.len() > 1 {
        return Err(usage(&format!(
            "multiple assets match {repr}: {}",
            shown(&exact)
        )));
    }
    match assets.len() {
        1 => py_int(&assets[0]["display_id"]).map_err(fail_exc),
        0 => Err(usage(&format!(
            "no asset match{}: {value}",
            in_category(category)
        ))),
        _ => {
            let all: Vec<&Value> = assets.iter().collect();
            Err(usage(&format!(
                "multiple assets match {repr}: {}",
                shown(&all)
            )))
        }
    }
}

fn resolve_asset_ids(
    id: i64,
    values: &[String],
    c: &Client,
    associated: bool,
    category: Option<&Category>,
) -> Result<Vec<i64>, Error> {
    let mut out = Vec::new();
    for v in values {
        let did = resolve_asset_id(id, v, c, associated, category)?;
        if !out.contains(&did) {
            out.push(did);
        }
    }
    Ok(out)
}

/// Line-prompt stand-in for prompt_toolkit's `input_dialog`; `None` on EOF.
fn prompt_text(title: &str, text: &str, default: &str) -> Option<String> {
    err().print_plain(&format!("{title}: {text}"));
    let d = (!default.is_empty()).then_some(default);
    term::prompt(title, d, d.is_some()).ok()
}

/// Line-prompt stand-in for `checkboxlist_dialog`: comma-separated numbers.
fn prompt_multi_select(
    title: &str,
    text: &str,
    values: &[(String, String)],
) -> Option<Vec<String>> {
    err().print_plain(&format!("{title}: {text}"));
    for (i, (_, label)) in values.iter().enumerate() {
        err().print_plain(&format!("  {}. {label}", i + 1));
    }
    let answer = term::prompt(
        "Numbers (comma-separated, empty to cancel)",
        Some(""),
        false,
    )
    .ok()?;
    let picked: Vec<String> = answer
        .split(',')
        .filter_map(|p| p.trim().parse::<usize>().ok())
        .filter_map(|n| values.get(n.wrapping_sub(1)).map(|v| v.0.clone()))
        .collect();
    (!picked.is_empty()).then_some(picked)
}

fn require_tty() -> CmdResult {
    if no_input(false) || !stdin_tty() {
        return Err(usage("interactive picker requires TTY"));
    }
    Ok(())
}

/// `_pick_change_assets`.
fn pick_assets(id: i64, c: &Client, category: Option<Category>) -> Result<Vec<i64>, Error> {
    require_tty()?;
    let category = match category {
        Some(cat) => cat,
        None => {
            let Some(v) = prompt_text(
                "Asset category",
                "Category for asset search. Leave empty for All assets.",
                "All assets",
            ) else {
                return Err(Error::exit(0, ""));
            };
            let raw = fsv_core::pystr::strip(&v);
            resolve_asset_category(if raw.is_empty() { "All assets" } else { raw }, c)?
        }
    };
    let mut prompt = format!("Search assets for {}", change_label(id));
    if category.get("filter") != Some(&json!("all_assets")) {
        prompt.push_str(&format!("\nCategory: {}", cat_name(&category)));
    }
    let Some(query) = prompt_text("Associate assets", &prompt, "") else {
        return Err(Error::exit(0, ""));
    };
    let items = assets_of(&search_assets(c, id, &query, 1, 50).map_err(fail_exc)?);
    let items = filter_by_category(items, Some(&category));
    if items.is_empty() {
        return Err(usage(&format!(
            "no asset match{}: {query}",
            in_category(Some(&category))
        )));
    }
    let dash = |v: &Value, k: &str| {
        let t = s(v, k);
        if t.is_empty() { "-".to_string() } else { t }
    };
    let values: Vec<(String, String)> = items
        .iter()
        .filter(|i| i.get("display_id").is_some_and(fsv_py::truthy))
        .map(|i| {
            (
                s(i, "display_id"),
                format!(
                    "[{}] {} · {} · {}",
                    s(i, "display_id"),
                    dash(i, "name"),
                    dash(i, "ci_type_name"),
                    dash(i, "location_name")
                ),
            )
        })
        .collect();
    let Some(selected) = prompt_multi_select(
        "Associate assets",
        &format!("Select asset(s) for {}", change_label(id)),
        &values,
    ) else {
        return Err(Error::exit(0, ""));
    };
    selected
        .iter()
        .map(|v| py_int(&json!(v)).map_err(fail_exc))
        .collect()
}

pub struct AssetOpts {
    pub id_: String,
    pub search: Option<String>,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub page: i64,
    pub per_page: i64,
    pub dry_run: bool,
    pub yes: bool,
    pub json_out: bool,
    pub fmt: OutputFormat,
    pub pick: bool,
    pub category: Option<String>,
    pub list_categories: bool,
}

impl AssetOpts {
    pub fn new(id_: &str) -> Self {
        AssetOpts {
            id_: id_.into(),
            search: None,
            add: Vec::new(),
            remove: Vec::new(),
            page: 1,
            per_page: 30,
            dry_run: false,
            yes: false,
            json_out: false,
            fmt: OutputFormat::Table,
            pick: false,
            category: None,
            list_categories: false,
        }
    }
}

fn print_categories(items: &[Category], fmt: OutputFormat) -> CmdResult {
    let raw = Value::Array(items.iter().cloned().map(Value::Object).collect());
    if emit_fmt(&raw, items, fmt)? {
        return Ok(());
    }
    let mut t = Table::new().title("Asset categories");
    t.add_column(Column::new("Name"));
    t.add_column(Column::new("CI Type ID").style("dim"));
    for item in items {
        let name = item
            .get("name")
            .map(fsv_py::py_str)
            .filter(|x| !x.is_empty());
        let ci = item
            .get("ci_type_id")
            .map(fsv_py::py_str)
            .filter(|x| !x.is_empty());
        t.add_row([name.unwrap_or("-".into()), ci.unwrap_or("-".into())]);
    }
    out().print_table(&t);
    Ok(())
}

/// `asset_categories_resource`.
pub fn categories(fmt: OutputFormat, json_out: bool) -> CmdResult {
    let c = client()?;
    let items = fetch_asset_categories(&c).map_err(fail_exc)?;
    print_categories(&items, effective(fmt, json_out))
}

fn confirm_or_fail(yes: bool, what: &str, question: &str) -> CmdResult {
    if !yes {
        if no_input(false) || !stdin_tty() {
            return Err(usage(&format!("pass --yes to {what}")));
        }
        term::confirm(question, Some(false), true)?;
    }
    Ok(())
}

/// `assets_resource`.
pub fn assets(o: AssetOpts) -> CmdResult {
    let actions = [
        o.search.is_some(),
        !o.add.is_empty(),
        !o.remove.is_empty(),
        o.pick,
        o.list_categories,
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if actions > 1 {
        return Err(usage(
            "choose only one action: --search, --add, --remove, --pick, or --list-categories",
        ));
    }
    let fmt = effective(o.fmt, o.json_out);
    let c = client()?;
    let category = match &o.category {
        Some(name) => Some(resolve_asset_category(name, &c).map_err(fail_exc)?),
        None => None,
    };
    if o.list_categories {
        let items = fetch_asset_categories(&c).map_err(fail_exc)?;
        return print_categories(&items, fmt);
    }
    let id = cid(&o.id_, &CHANGES)?;
    let label = change_label(id);
    let associate = |asset_ids: Vec<i64>| -> CmdResult {
        let payload =
            json!({"action": "associate_assets", "change_id": id, "asset_ids": asset_ids});
        if o.dry_run {
            emit_json(&payload);
            return Ok(());
        }
        confirm_or_fail(
            o.yes,
            "associate assets",
            &format!("Associate {} asset(s) with {label}?", asset_ids.len()),
        )?;
        create::associate_assets(c.as_ref(), id, &asset_ids).map_err(fail_exc)?;
        if o.json_out {
            let mut p = payload;
            p["ok"] = json!(true);
            emit_json(&p);
            return Ok(());
        }
        out().print(&format!(
            "[green]associated[/] {} asset(s) with {label}",
            asset_ids.len()
        ));
        Ok(())
    };
    if o.pick {
        let ids = pick_assets(id, &c, category)?;
        if ids.is_empty() {
            out().print("cancelled");
            return Ok(());
        }
        return associate(ids);
    }
    if !o.add.is_empty() {
        let ids = resolve_asset_ids(id, &o.add, &c, false, category.as_ref())?;
        return associate(ids);
    }
    if !o.remove.is_empty() {
        let ids = resolve_asset_ids(id, &o.remove, &c, true, category.as_ref())?;
        let payload =
            json!({"action": "dissociate_assets", "change_id": id, "asset_display_ids": ids});
        if o.dry_run {
            emit_json(&payload);
            return Ok(());
        }
        confirm_or_fail(
            o.yes,
            "remove assets",
            &format!("Remove {} asset(s) from {label}?", ids.len()),
        )?;
        create::dissociate_assets(c.as_ref(), id, &ids).map_err(fail_exc)?;
        if o.json_out {
            let mut p = payload;
            p["ok"] = json!(true);
            emit_json(&p);
            return Ok(());
        }
        out().print(&format!(
            "[green]removed[/] {} asset(s) from {label}",
            ids.len()
        ));
        return Ok(());
    }
    let items = if let Some(q) = &o.search {
        let data = search_assets(&c, id, q, o.page, o.per_page).map_err(fail_exc)?;
        let items = filter_by_category(assets_of(&data), category.as_ref());
        let flat = flat_maps(&items);
        let payload = match &category {
            Some(cat) => {
                let mut p = data.as_object().cloned().unwrap_or_default();
                p.insert("assets".into(), Value::Array(items.clone()));
                let mut meta = data
                    .get("meta")
                    .filter(|m| fsv_py::truthy(m))
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                meta.insert("filtered_count".into(), json!(items.len()));
                meta.insert("category".into(), json!(cat_name(cat)));
                p.insert("meta".into(), Value::Object(meta));
                Value::Object(p)
            }
            None => data,
        };
        if emit_fmt(&payload, &flat, fmt)? {
            return Ok(());
        }
        items
    } else {
        let items = filter_by_category(
            create::get_change_assets(c.as_ref(), id).map_err(fail_exc)?,
            category.as_ref(),
        );
        if emit_fmt(&Value::Array(items.clone()), &flat_maps(&items), fmt)? {
            return Ok(());
        }
        items
    };
    let mut t = Table::new().title("Assets");
    t.add_column(Column::new("ID").style("cyan"));
    for h in ["Name", "Type", "Used By", "Location", "State", "Serial"] {
        t.add_column(Column::new(h));
    }
    for item in &items {
        let row = asset_display(item);
        t.add_row(
            [
                "id", "name", "type", "used_by", "location", "state", "serial",
            ]
            .iter()
            .map(|k| fsv_py::py_str(&row[*k])),
        );
    }
    out().print_table(&t);
    Ok(())
}

fn flat_maps(items: &[Value]) -> Vec<Map<String, Value>> {
    items
        .iter()
        .map(|x| asset_display(x).as_object().cloned().unwrap_or_default())
        .collect()
}

// ---------------------------------------------------------------------------
// associations

/// `_resolve_change_ticket_id`.
fn resolve_ticket_id(value: &str, c: &Client) -> Result<i64, Error> {
    let raw = fsv_core::pystr::strip(value);
    if let Ok(id) = parse_id(raw, &TICKETS) {
        return Ok(id);
    }
    let tickets = create::search_change_tickets(c, raw).map_err(fail_exc)?;
    if tickets.is_empty() {
        return Err(usage(&format!("no ticket match: {value}")));
    }
    let folded = casefold(raw);
    let repr = fsv_py::py_repr_str(value);
    let shown = |v: &[&Value]| {
        v.iter()
            .take(5)
            .map(|t| ticket_label(t))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let exact: Vec<&Value> = tickets
        .iter()
        .filter(|t| casefold(&ticket_label(t)) == folded || casefold(&s(t, "subject")) == folded)
        .collect();
    if exact.len() == 1 {
        return py_int(&exact[0]["id"]).map_err(fail_exc);
    }
    if exact.len() > 1 {
        return Err(usage(&format!(
            "multiple ticket matches for {repr}: {}",
            shown(&exact)
        )));
    }
    if tickets.len() == 1 {
        return py_int(&tickets[0]["id"]).map_err(fail_exc);
    }
    let all: Vec<&Value> = tickets.iter().collect();
    Err(usage(&format!(
        "multiple ticket matches for {repr}: {}",
        shown(&all)
    )))
}

/// `_resolve_associated_change_ticket_id`.
fn resolve_associated_ticket_id(id: i64, value: &str, c: &Client) -> Result<i64, Error> {
    let raw = fsv_core::pystr::strip(value);
    let target = parse_id(raw, &TICKETS).ok();
    let inventory = create::get_change_associations(c, id).map_err(fail_exc)?;
    if !inventory["meta"]["tickets"]
        .get("complete")
        .is_some_and(fsv_py::truthy)
    {
        return Err(usage(
            "initiating-ticket inventory unavailable/incomplete; cannot resolve removal",
        ));
    }
    let tickets: Vec<Value> = inventory["tickets"].as_array().cloned().unwrap_or_default();
    if let Some(t) = target
        && tickets.iter().any(|x| {
            fsv_core::pystr::parse_decimal(&fsv_py::py_str(
                x.get("id")
                    .filter(|v| fsv_py::truthy(v))
                    .unwrap_or(&json!(0)),
            )) == Some(t)
        })
    {
        return Ok(t);
    }
    let folded = casefold(raw);
    let repr = fsv_py::py_repr_str(value);
    let shown = |v: &[&Value]| {
        v.iter()
            .take(5)
            .map(|t| ticket_label(t))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let exact: Vec<&Value> = tickets
        .iter()
        .filter(|t| casefold(&ticket_label(t)) == folded || casefold(&s(t, "subject")) == folded)
        .collect();
    if exact.len() == 1 {
        return py_int(&exact[0]["id"]).map_err(fail_exc);
    }
    if exact.len() > 1 {
        return Err(usage(&format!(
            "multiple associated ticket matches for {repr}: {}",
            shown(&exact)
        )));
    }
    let matches: Vec<&Value> = tickets
        .iter()
        .filter(|t| casefold(&s(t, "subject")).contains(&folded))
        .collect();
    if matches.len() == 1 {
        return py_int(&matches[0]["id"]).map_err(fail_exc);
    }
    if matches.is_empty() {
        return Err(usage(&format!(
            "ticket not associated on change #{id}: {repr}"
        )));
    }
    Err(usage(&format!(
        "multiple associated ticket matches for {repr}: {}",
        shown(&matches)
    )))
}

/// `_pick_change_tickets`.
fn pick_tickets(id: i64, c: &Client) -> Result<Vec<i64>, Error> {
    require_tty()?;
    let Some(query) = prompt_text(
        "Associate tickets",
        &format!("Search ticket ID or subject for {}", change_label(id)),
        "",
    ) else {
        return Err(Error::exit(0, ""));
    };
    let items = create::search_change_tickets(c, &query).map_err(fail_exc)?;
    if items.is_empty() {
        return Err(usage(&format!("no ticket match: {query}")));
    }
    let values: Vec<(String, String)> = items
        .iter()
        .filter(|i| i.get("id").is_some_and(fsv_py::truthy))
        .map(|i| {
            let subject = s(i, "subject");
            let status = fsv_py::py_str(&or_dash(i, &["status_name", "status"]));
            (
                s(i, "id"),
                format!(
                    "[{}] {} · {status}",
                    ticket_label(i),
                    if subject.is_empty() { "-" } else { &subject }
                ),
            )
        })
        .collect();
    let Some(selected) = prompt_multi_select(
        "Associate tickets",
        &format!("Select ticket(s) for {}", change_label(id)),
        &values,
    ) else {
        return Err(Error::exit(0, ""));
    };
    selected
        .iter()
        .map(|v| py_int(&json!(v)).map_err(fail_exc))
        .collect()
}

fn ticket_row(item: &Value, hid: String) -> [String; 3] {
    let subj = fsv_py::py_str(&or_dash(item, &["subject", "title"]));
    let status = item
        .get("status_name")
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_else(|| item.get("status").map_or("-".into(), fsv_py::py_str));
    [hid, subj, status]
}

fn hid_or_id(item: &Value) -> String {
    let hid = s(item, "human_display_id");
    if hid.is_empty() {
        item.get("id").map_or("-".into(), fsv_py::py_str)
    } else {
        hid
    }
}

pub struct AssocOpts {
    pub id_: String,
    pub search: Option<String>,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub dry_run: bool,
    pub yes: bool,
    pub json_out: bool,
    pub fmt: OutputFormat,
    pub pick: bool,
    pub expect_plan: Option<String>,
    pub read_only: bool,
}

impl AssocOpts {
    pub fn new(id_: &str) -> Self {
        AssocOpts {
            id_: id_.into(),
            search: None,
            add: Vec::new(),
            remove: Vec::new(),
            dry_run: false,
            yes: false,
            json_out: false,
            fmt: OutputFormat::Table,
            pick: false,
            expect_plan: None,
            read_only: false,
        }
    }
}

/// `change_associations.AssociationError` is a `ValueError`: `_err_exc` exits 2.
fn assoc_exc(e: Error) -> Error {
    match e {
        Error::Exit { code: 2, message } if !message.is_empty() => fail(&term::escape(&message), 2),
        other => fail_exc(other),
    }
}

/// `change_associations_resource`.
pub fn associations(o: AssocOpts) -> CmdResult {
    let actions = [
        o.search.is_some(),
        !o.add.is_empty(),
        !o.remove.is_empty(),
        o.pick,
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if actions > 1 {
        return Err(usage(
            "choose only one action: --search, --add, or --remove",
        ));
    }
    let id = cid(&o.id_, &CHANGES)?;
    let fmt = effective(o.fmt, o.json_out);
    if let Some(q) = &o.search {
        let c = crate::util::shared_client().map_err(fail_exc)?;
        let items = create::search_change_tickets(c.as_ref(), q).map_err(fail_exc)?;
        let flat: Vec<Map<String, Value>> = items
            .iter()
            .map(|i| {
                let [id, subject, status] = ticket_row(i, hid_or_id(i));
                crate::util::obj(vec![
                    ("id", json!(id)),
                    ("subject", json!(subject)),
                    ("status", json!(status)),
                ])
            })
            .collect();
        if emit_fmt(&Value::Array(items.clone()), &flat, fmt)? {
            return Ok(());
        }
        let mut t = Table::new().title("Ticket search");
        t.add_column(Column::new("ID").style("cyan"));
        t.add_column(Column::new("Subject"));
        t.add_column(Column::new("Status"));
        for item in &items {
            let subj = s(item, "subject");
            let status = item
                .get("status_name")
                .filter(|v| fsv_py::truthy(v))
                .map(fsv_py::py_str)
                .unwrap_or_else(|| item.get("status").map_or("-".into(), fsv_py::py_str));
            t.add_row([
                ticket_label(item),
                if subj.is_empty() { "-".into() } else { subj },
                status,
            ]);
        }
        out().print_table(&t);
        return Ok(());
    }
    if o.pick || !o.add.is_empty() || !o.remove.is_empty() {
        if o.read_only && !o.dry_run {
            return Err(usage("read-only mode: no association request was sent"));
        }
        if !o.remove.is_empty() && o.remove.len() != 1 {
            return Err(usage(
                "unlink accepts one ticket per reviewed request; review each removal separately",
            ));
        }
        let c = crate::util::shared_client().map_err(fail_exc)?;
        let ids: Vec<i64> = if o.pick {
            pick_tickets(id, &c)?
        } else if !o.remove.is_empty() {
            o.remove
                .iter()
                .map(|x| resolve_associated_ticket_id(id, x, &c))
                .collect::<Result<_, _>>()?
        } else {
            o.add
                .iter()
                .map(|x| resolve_ticket_id(x, &c))
                .collect::<Result<_, _>>()?
        };
        if ids.is_empty() {
            out().print("cancelled");
            return Ok(());
        }
        let ids_json: Vec<Value> = ids.iter().map(|i| json!(i)).collect();
        let plan = associations::build_plan(
            &id.to_string(),
            &ids_json,
            !o.remove.is_empty(),
            &c.tenant().domain,
        )
        .map_err(assoc_exc)?;
        let journal = DiskJournal;
        if o.dry_run {
            let opts = SendOpts {
                dry_run: true,
                ..Default::default()
            };
            emit_json(&associations::send(&plan, c.as_ref(), &journal, opts).map_err(assoc_exc)?);
            return Ok(());
        }
        if o.read_only {
            return Err(usage("read-only mode: no association request was sent"));
        }
        eprintln!(
            "{}",
            fsv_py::Dumps::default()
                .indent(2)
                .ensure_ascii(false)
                .dump(&Value::Object(plan.review()))
        );
        let hash = plan.hash();
        if o.expect_plan.as_deref().is_some_and(|e| e != hash) {
            return Err(usage(
                "association review hash does not match; review again",
            ));
        }
        if o.yes {
            if o.expect_plan.as_deref() != Some(hash.as_str()) {
                return Err(usage(
                    "--yes requires the full --expect-plan from a prior association review",
                ));
            }
        } else if no_input(false) || !stdin_tty() {
            return Err(usage(
                "sending requires --yes and --expect-plan in no-input mode",
            ));
        } else if !term::confirm(
            "Send this exact reviewed association request?",
            Some(false),
            false,
        )? {
            out().print("cancelled");
            return Ok(());
        }
        let opts = SendOpts {
            reviewed_hash: Some(&hash),
            read_only: o.read_only,
            dry_run: false,
        };
        let result = associations::send(&plan, c.as_ref(), &journal, opts).map_err(assoc_exc)?;
        if o.json_out {
            emit_json(&result);
        } else {
            out().print(&format!(
                "Association {}; complete inventory verified: {}",
                fsv_py::py_str(&result["status"]),
                fsv_py::py_str(&result["verified"])
            ));
        }
        return Ok(());
    }
    let c = crate::util::shared_client().map_err(fail_exc)?;
    let assoc = create::get_change_associations(c.as_ref(), id).map_err(assoc_exc)?;
    let kinds = ["tickets", "problems", "releases"];
    let mut flat = Vec::new();
    for k in kinds {
        for item in assoc[k].as_array().into_iter().flatten() {
            let [id, subject, status] = ticket_row(item, hid_or_id(item));
            flat.push(crate::util::obj(vec![
                ("type", json!(k)),
                ("id", json!(id)),
                ("subject", json!(subject)),
                ("status", json!(status)),
            ]));
        }
    }
    for w in assoc["warnings"].as_array().into_iter().flatten() {
        err().print_plain(&fsv_py::py_str(w));
    }
    if emit_fmt(&assoc, &flat, fmt)? {
        return Ok(());
    }
    let mut any = false;
    for k in kinds {
        let items = assoc[k].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            continue;
        }
        any = true;
        let mut title = k.to_string();
        title[..1].make_ascii_uppercase();
        let mut t = Table::new().title(title);
        t.add_column(Column::new("ID").style("cyan"));
        t.add_column(Column::new("Subject"));
        t.add_column(Column::new("Status"));
        for item in &items {
            t.add_row(ticket_row(item, hid_or_id(item)));
        }
        out().print_table(&t);
    }
    if !any {
        out().print(if assoc["complete"] == json!(true) {
            "no associations"
        } else {
            "association inventory unavailable/incomplete; not an empty list"
        });
    }
    Ok(())
}
