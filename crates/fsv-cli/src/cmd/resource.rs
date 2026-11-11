//! Commands every resource group has (`_make_subapp`).

use serde_json::{Map, Value, json};

use fsv_api::service;
use fsv_api::site::{SavedView, default_view, parse_saved_views};
use fsv_core::resource::{Resource, TICKETS};
use fsv_core::safety;
use fsv_core::term::{Column, Justify, Table, err, escape, out};
use fsv_core::{Error, Result, config, pystr};

use crate::generated::*;
use crate::types::{OutputFormat, effective};
use crate::util::{
    CmdResult, cid, client, confirm_writes, emit_dry_run, emit_dry_run_as, emit_fmt, fail_exc,
    first_str, format_cid, head, obj, plain_fail, uncaught, usage,
};
use crate::{lookup, schema};

/// `fsv RES ls`.
pub fn ls(res: &'static Resource, a: ResLsArgs) -> CmdResult {
    let explicit = a.r#where.clone();
    let mut where_: Vec<String> = if a.no_defaults {
        Vec::new()
    } else {
        config::get_default_where(res.name)
    };
    if !where_.is_empty() && !explicit.is_empty() {
        let fields: Vec<String> = explicit.iter().map(|e| where_field(e)).collect();
        where_.retain(|e| !fields.contains(&where_field(e)));
    }
    where_.extend(explicit);
    list_resource(res, &a, &where_)
}

/// `_where_field`.
fn where_field(expr: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^(.+?)(>=|<=|!=|~=|=|>|<)").unwrap());
    let e = pystr::strip(expr);
    RE.captures(e)
        .map_or(e, |m| pystr::strip(m.get(1).map_or("", |g| g.as_str())))
        .to_string()
}

/// `_merge_same_conditions`: one `is_in`/`is_not_in` per condition, operator and type.
fn merge_same_conditions(query: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut seen: Vec<((Value, Value, Value), usize)> = Vec::new();
    for item in query {
        let op = item.get("operator").cloned().unwrap_or(Value::Null);
        if !matches!(op.as_str(), Some("is_in" | "is_not_in")) {
            out.push(item);
            continue;
        }
        let field = |k: &str| item.get(k).cloned().unwrap_or(Value::Null);
        let key = (field("condition"), op, field("type"));
        let values: Vec<Value> = item
            .get("value")
            .filter(|v| fsv_py::truthy(v))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        match seen.iter().find(|(k, _)| *k == key) {
            Some((_, at)) => {
                if let Some(Value::Array(v)) = out[*at].get_mut("value") {
                    v.extend(values);
                }
            }
            None => {
                let mut merged = item.as_object().cloned().unwrap_or_default();
                merged.insert("value".into(), Value::Array(values));
                seen.push((key, out.len()));
                out.push(Value::Object(merged));
            }
        }
    }
    out
}

/// `_build_query_hash`: `(payload, conditions, explanation)`.
fn build_query_hash(
    c: &std::sync::Arc<fsv_core::Client>,
    res: &'static Resource,
    raw: Option<&str>,
    where_: &[String],
    or_grouping: bool,
) -> Result<(Option<String>, Vec<Value>, Vec<fsv_api::Explain>)> {
    let sch = if where_.is_empty() {
        None
    } else {
        Some(schema::load(res, c, false)?)
    };
    let dir = fsv_app::tui::ClientDirectory(c.clone());
    let (_, query, explain) =
        fsv_api::build_query_hash(Some(&dir), res.name, sch.as_ref(), where_, raw, or_grouping)
            .map_err(|e| Error::Value(e.0))?;
    let query = merge_same_conditions(query);
    if query.is_empty() {
        return Ok((None, query, explain));
    }
    let payload = if or_grouping {
        json!({"any": query})
    } else {
        Value::Array(query.clone())
    };
    let text = fsv_py::Dumps::default().separators(",", ":").dump(&payload);
    Ok((Some(text), query, explain))
}

/// `_show_query_explain`.
fn show_query_explain(explain: &[fsv_api::Explain], query: &[Value], or_grouping: bool) {
    let mut t = Table::new().title("query".to_string());
    for h in ["field", "condition", "operator", "type"] {
        t.add_column(Column::new(h));
    }
    t.add_column(Column::new("value").style("dim"));
    for x in explain {
        t.add_row([
            x.field.clone(),
            x.condition.clone(),
            x.operator.clone(),
            x.type_name.clone(),
            x.value.clone(),
        ]);
    }
    out().print_table(&t);
    let grouping = if or_grouping { "OR" } else { "AND" };
    out().print(&format!(
        "api: internal /api/_/ with query_hash ({} conditions, {grouping})",
        query.len()
    ));
}

/// `_pick_view`.
fn pick_view(
    c: &fsv_core::Client,
    res: &'static Resource,
    view: Option<&str>,
    use_default: bool,
) -> Result<Option<SavedView>> {
    let saved = saved_views(c, res).map_err(fail_exc)?;
    if let Some(text) = view.filter(|v| !v.is_empty()) {
        return match fsv_api::site::find_view(&saved, text) {
            Ok(v) => Ok(Some(v)),
            Err(_) if saved.is_empty() => Ok(Some(SavedView {
                id: text.to_string(),
                name: text.to_string(),
                system: false,
                marked_default: false,
                raw: Value::Null,
            })),
            Err(e) => Err(usage(&e.0)),
        };
    }
    Ok(if use_default {
        default_view(&saved).cloned()
    } else {
        None
    })
}

/// `list_resource`.
fn list_resource(res: &'static Resource, a: &ResLsArgs, where_: &[String]) -> CmdResult {
    let c = client()?;
    let raw = a.query_hash.as_deref().filter(|s| !s.is_empty());
    let (qh, query, explain) =
        build_query_hash(&c, res, raw, where_, a.or_grouping).map_err(fail_exc)?;
    if a.debug {
        show_query_explain(&explain, &query, a.or_grouping);
        return Ok(());
    }
    if a.or_grouping && res.name != fsv_core::resource::CHANGES.name {
        return Err(usage(&format!(
            "OR grouping not supported for {}; use AND filters",
            res.name
        )));
    }
    let view = pick_view(
        &c,
        res,
        a.filter_name.as_deref(),
        where_.is_empty() && raw.is_none(),
    )?;
    let mut ot = a.order_type.as_str().to_string();
    let mut ob = a.order_by.clone().filter(|s| !s.is_empty());
    if let Some(v) = &view
        && ob.is_none()
        && fsv_py::truthy(&v.raw)
    {
        let settings = fsv_api::views::view_settings(c.as_ref(), res, v).map_err(fail_exc)?;
        ob = settings
            .get("order_by")
            .filter(|x| fsv_py::truthy(x))
            .map(fsv_py::py_str);
        if ob.is_some()
            && let Some(t) = settings.get("order_type").filter(|x| fsv_py::truthy(x))
        {
            ot = fsv_py::py_str(t);
        }
    }
    let ob = ob.unwrap_or_else(|| "created_at".into());
    let headers: Vec<Value> = if res.name == TICKETS.name {
        match fsv_api::views::optional_get(c.as_ref(), "tickets/headers", None, "list headers")
            .map_err(fail_exc)?
        {
            Ok(Value::Object(m)) => m
                .get("headers")
                .and_then(Value::as_array)
                .map(|rows| rows.iter().filter(|h| h.is_object()).cloned().collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let fmt = effective(a.format_, a.json_out);
    let view_id = view.as_ref().map(|v| v.id.clone());
    let paged = a.all_pages || a.n_pages.is_some();
    let size = if paged { 100 } else { a.per_page };
    let list = |p: i64| {
        service::list_page(
            c.as_ref(),
            res,
            &service::ListArgs {
                page: p,
                per_page: size,
                filter_name: view_id.as_deref(),
                order_by: Some(&ob),
                order_type: Some(&ot),
                query_hash: qh.as_deref(),
                or_grouping: a.or_grouping,
            },
        )
    };
    let (items, total, more) = if paged {
        let mut acc: Vec<Value> = Vec::new();
        let (mut total, mut more) = (None, false);
        let limit = a.n_pages.filter(|n| *n != 0).unwrap_or(100);
        for p in a.page..a.page + limit {
            let (batch, data) = list(p).map_err(fail_exc)?;
            let n = batch.len();
            acc.extend(batch);
            (total, more) = fsv_api::site::list_meta(&data, n, p, size);
            err().print(&format!("  fetched {} so far...", acc.len()));
            if (n as i64) < size || !more {
                break;
            }
        }
        (acc, total, more)
    } else {
        let (items, data) = list(a.page).map_err(fail_exc)?;
        let (total, more) = fsv_api::site::list_meta(&data, items.len(), a.page, size);
        (items, total, more)
    };
    let sch = schema::load(res, &c, false).map_err(fail_exc)?;
    let me = if matches!(fmt, OutputFormat::Json) {
        json!({})
    } else {
        current_user(&c)?
    };
    let tz = fsv_forms::tz::resolve_timezone(
        None,
        me.as_object()
            .filter(|m| m.get("time_zone").is_some_and(fsv_py::truthy)),
    )
    .map(|(z, _)| z.tz)
    .unwrap_or_else(|_| jiff::tz::TimeZone::system());
    let columns = fsv_api::site::list_columns(res, Some(&sch), Some(&headers));
    let rows: Vec<Map<String, Value>> = items
        .iter()
        .map(|x| fsv_api::site::list_row(x, res, &columns, Some(&sch), me.get("id"), Some(&tz)))
        .collect();
    match fmt {
        OutputFormat::Json => {
            fsv_core::output::emit_json(&Value::Array(items));
            return Ok(());
        }
        OutputFormat::Csv => return fsv_core::output::emit_delimited(&rows, ','),
        OutputFormat::Tsv => return fsv_core::output::emit_delimited(&rows, '\t'),
        OutputFormat::Table => {}
    }
    let eff_page = if paged { 1 } else { a.page };
    let per_page = if eff_page == a.page && !paged {
        a.per_page
    } else {
        (items.len() as i64).max(1)
    };
    let n = items.len();
    let lp = fsv_api::site::ListPage {
        resource_name: res.name.to_string(),
        items,
        columns,
        rows,
        view_name: view.as_ref().map(|v| v.name.clone()),
        page: eff_page,
        per_page,
        total,
        has_next: more,
        order_by: ob.clone(),
        order_type: ot.clone(),
        sort: fsv_api::site::sort_label(Some(&ob), Some(&sch)),
        notes: Vec::new(),
    };
    let mut head_line = fsv_views::pages::list_header(&lp);
    if !where_.is_empty() || raw.is_some() {
        let what = if where_.is_empty() {
            "query hash".to_string()
        } else {
            where_.join(" ")
        };
        head_line.push_str(&format!("[dim] · filtered: {}[/]", escape(&what)));
    }
    let print = || {
        out().print(&head_line);
        out().print_table(&fsv_views::pages::list_table(&lp));
    };
    let pager = a.pager && !a.no_pager;
    if fsv_core::term::should_page(n * 2 + 2, pager) {
        let ((), text) = fsv_core::term::capture(print);
        if !fsv_core::term::page_text(&text) {
            print();
        }
    } else {
        print();
    }
    err().print(&format!("{n} rows"));
    Ok(())
}

/// `fsv RES get ID`.
pub fn get(res: &'static Resource, a: ResGetArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let c = client()?;
    let fmt = effective(a.format_, a.json_out);
    let as_json = matches!(fmt, OutputFormat::Json);
    if a.stats && res.name == fsv_core::resource::CHANGES.name && as_json {
        let sch = schema::load(res, &c, false).map_err(fail_exc)?;
        let ev = service::get_change_evidence(c.as_ref(), id).map_err(fail_exc)?;
        let mut doc = ev
            .get("change")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, from) in [
            ("planning_fields", "planning_fields_by_name"),
            ("change_planning_fields", "planning_fields"),
        ] {
            doc.insert(key.into(), ev.get(from).cloned().unwrap_or(Value::Null));
        }
        doc.insert(
            "planning_field_definitions".into(),
            Value::Array(service::get_change_planning_field_definitions(&sch)),
        );
        for key in ["main_attachments", "description_attachment_urls"] {
            doc.insert(key.into(), ev.get(key).cloned().unwrap_or(Value::Null));
        }
        fsv_core::output::emit_json(&Value::Object(doc));
        return Ok(());
    }
    let d = load_detail(res, &c, id, &a)?;
    if as_json {
        let mut doc = d.record.as_object().cloned().unwrap_or_default();
        if let Some(items) = &d.requested_items {
            doc.insert("requested_items".into(), Value::Array(items.clone()));
        }
        doc.insert("view".into(), d.to_json());
        fsv_core::output::emit_json(&Value::Object(doc));
        return Ok(());
    }
    if let Some(delim) = match fmt {
        OutputFormat::Csv => Some(','),
        OutputFormat::Tsv => Some('\t'),
        _ => None,
    } {
        let row = |section: &str, label: &str, value: &str| -> Map<String, Value> {
            let mut m = Map::new();
            m.insert("section".into(), json!(section));
            m.insert("label".into(), json!(label));
            m.insert("value".into(), json!(value));
            m
        };
        let mut rows: Vec<Map<String, Value>> = d
            .summary()
            .iter()
            .map(|(k, v)| row("summary", k, v))
            .collect();
        rows.extend(
            d.properties
                .iter()
                .map(|p| row("properties", &p.label, &p.value)),
        );
        fsv_core::output::emit_delimited(&rows, delim)?;
        return Ok(());
    }
    let active = match a.tab.as_deref() {
        Some(t) if !a.all_tabs => Some(d.resolve_tab(t).map_err(|e| usage(&e.to_string()))?),
        _ => None,
    };
    fsv_views::pages::print_detail(out(), &d, active.as_deref(), a.all_tabs);
    Ok(())
}

/// `get_resource`'s `load`: record, schema, detail page and requested tabs.
fn load_detail(
    res: &'static Resource,
    c: &std::sync::Arc<fsv_core::Client>,
    id: i64,
    a: &ResGetArgs,
) -> Result<fsv_views::Detail> {
    let session = |e: fsv_views::SessionError| fail_exc(Error::Session(e.0));
    let sch = schema::load(res, c, false).map_err(fail_exc)?;
    let includes = (res.name == TICKETS.name)
        .then_some("requester,stats,phone,feedback,ticket_status,responder");
    let record = service::get_item(c.as_ref(), res, id, includes).map_err(fail_exc)?;
    let user = current_user(c)?;
    let tz = fsv_forms::tz::resolve_timezone(
        None,
        user.as_object()
            .filter(|m| m.get("time_zone").is_some_and(fsv_py::truthy)),
    )
    .map(|(z, _)| z.tz)
    .unwrap_or_else(|_| jiff::tz::TimeZone::system());
    let api: std::sync::Arc<dyn fsv_api::service::Api + Send + Sync> = c.clone();
    let store = fsv_api::formmeta::DiskStore::current();
    let props = fsv_views::properties::load_properties(
        api,
        &store,
        res,
        &record,
        Some(&sch),
        Some(&tz),
        Some(&user),
    )
    .map_err(session)?;
    let mut src = fsv_app::ClientSource::new(c.clone());
    let input = fsv_views::DetailInput {
        record,
        schema: Some(sch),
        user,
        tz,
        item_id: json!(id),
        all_conversations: a.all_conversations,
        tabs: &[],
    };
    let mut d = fsv_views::load_detail(&mut src, res, input, Some(props)).map_err(session)?;
    let wanted: Vec<String> = if a.all_tabs {
        d.tab_names()
            .into_iter()
            .filter(|n| n != "details")
            .collect()
    } else if let Some(t) = a.tab.as_deref() {
        vec![d.resolve_tab(t).map_err(|e| usage(&e.to_string()))?]
    } else {
        Vec::new()
    };
    for name in wanted.iter().filter(|n| *n != "details") {
        let td = fsv_views::load_tab(&mut src, res, &d.record, name).map_err(session)?;
        d.set_tab(td);
    }
    Ok(d)
}

/// `views.current_user`: the signed-in agent, `{}` when unreadable.
fn current_user(c: &fsv_core::Client) -> Result<Value> {
    let mut me = match c.me() {
        Ok(Value::Object(m)) => m,
        Err(e @ Error::Session(_)) => return Err(fail_exc(e)),
        _ => Map::new(),
    };
    if !me.is_empty() && !me.get("name").is_some_and(fsv_py::truthy) {
        let name = ["first_name", "last_name"]
            .iter()
            .map(|k| {
                me.get(*k)
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ");
        me.insert("name".into(), json!(pystr::strip(&name)));
    }
    Ok(Value::Object(me))
}

/// `fsv RES activity ID`.
pub fn activity(res: &'static Resource, a: ResActivityArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let c = client()?;
    let td = load_tab(&c, res, id, "activity")?;
    read_warnings(&td)?;
    let all = td
        .data
        .as_ref()
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let acts = &all[..py_slice_end(all.len(), a.limit)];
    let flat: Vec<Map<String, Value>> = acts
        .iter()
        .map(|x| {
            let dt = local_dt(x.get("created_at"));
            let mut row = Map::new();
            row.insert("day".into(), json!(activity_day(dt.as_ref())));
            row.insert("time".into(), json!(clock(dt.as_ref())));
            row.insert("actor".into(), actor_name(x));
            row.insert("content".into(), json!(activity_text(x.get("content"))));
            row
        })
        .collect();
    if emit_fmt(
        &Value::Array(acts.to_vec()),
        &flat,
        effective(a.format_, a.json_out),
    )? {
        return Ok(());
    }
    let mut last_day: Option<String> = None;
    for x in acts {
        let dt = local_dt(x.get("created_at"));
        let day = activity_day(dt.as_ref());
        if last_day.as_deref() != Some(day.as_str()) {
            out().print(&format!("\n[bold]{day}[/]"));
            last_day = Some(day);
        }
        let actor = fsv_py::py_str(&actor_name(x));
        out().print(&format!(
            "[dim]{}[/]  [cyan]{}[/]  {}",
            clock(dt.as_ref()),
            escape(&actor),
            escape(&activity_text(x.get("content")))
        ));
        for sub in x
            .get("sub_contents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            out().print(&format!(
                "      [dim]·[/] {}",
                escape(&activity_text(Some(sub)))
            ));
        }
    }
    Ok(())
}

/// `fsv RES tasks ID`.
pub fn tasks(res: &'static Resource, a: ResTasksArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let c = client()?;
    let td = load_tab(&c, res, id, "tasks")?;
    read_warnings(&td)?;
    let items = td
        .data
        .as_ref()
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let is_change = res.name == fsv_core::resource::CHANGES.name;
    let cells = |x: &Value| -> Vec<(&'static str, String)> {
        let name_of = |k: &str| {
            x.get(k)
                .filter(|v| fsv_py::truthy(v))
                .and_then(|v| v.get("name"))
                .filter(|v| fsv_py::truthy(v))
                .map(fsv_py::py_str)
        };
        let mut row = vec![
            (
                "id",
                first_truthy(x, "human_display_id").unwrap_or_else(|| py_get(x, "id")),
            ),
            (
                "status",
                first_truthy(x, "status_name").unwrap_or_else(|| py_get(x, "status")),
            ),
            (
                "group_agent",
                name_of("group")
                    .or_else(|| name_of("agent"))
                    .unwrap_or_else(|| "-".into()),
            ),
            (
                "due",
                first_truthy(x, "task_due_status")
                    .or_else(|| first_truthy(x, "due_date"))
                    .unwrap_or_else(|| "-".into()),
            ),
        ];
        if is_change {
            let cf = x.get("custom_fields").cloned().unwrap_or(Value::Null);
            let part = |k: &str| first_truthy(&cf, k).unwrap_or_else(|| "-".into());
            row.push((
                "system_env",
                format!("{} / {}", part("system"), part("environment")),
            ));
        }
        let title = first_truthy(x, "title").unwrap_or_default();
        row.push(("title", title.chars().take(60).collect()));
        row
    };
    let flat: Vec<Map<String, Value>> = items
        .iter()
        .map(|x| {
            cells(x)
                .into_iter()
                .map(|(k, v)| (k.to_string(), json!(v)))
                .collect()
        })
        .collect();
    if emit_fmt(
        &Value::Array(items.clone()),
        &flat,
        effective(a.format_, a.json_out),
    )? {
        return Ok(());
    }
    let mut t = Table::new();
    t.add_column(Column::new("TSK").style("cyan"));
    t.add_column(Column::new("Status"));
    t.add_column(Column::new("Group/Agent"));
    t.add_column(Column::new("Due").style("dim"));
    if is_change {
        t.add_column(Column::new("System/Env"));
    }
    t.add_column(Column::new("Title"));
    for x in &items {
        t.add_row(cells(x).into_iter().map(|(_, v)| v));
    }
    out().print_table(&t);
    Ok(())
}

/// `views.load_tab` for `{"id": cid}` through `_api`.
fn load_tab(
    c: &std::sync::Arc<fsv_core::Client>,
    res: &'static Resource,
    id: i64,
    name: &str,
) -> Result<fsv_views::TabData> {
    let mut src = fsv_app::ClientSource::new(c.clone());
    fsv_views::load_tab(&mut src, res, &json!({"id": id}), name)
        .map_err(|e| fail_exc(Error::Session(e.0)))
}

/// `_read_warnings`.
pub(crate) fn read_warnings(td: &fsv_views::TabData) -> CmdResult {
    if let Some(miss) = &td.missing {
        err().print(&escape(&miss.note()));
        if td.data.is_none() && td.parts.is_empty() {
            return Err(Error::exit(1, ""));
        }
    }
    for note in &td.notes {
        err().print(&escape(note));
    }
    Ok(())
}

fn py_get(x: &Value, k: &str) -> String {
    fsv_py::py_str(x.get(k).unwrap_or(&Value::Null))
}

fn first_truthy(x: &Value, k: &str) -> Option<String> {
    x.get(k).filter(|v| fsv_py::truthy(v)).map(fsv_py::py_str)
}

/// End index of Python's `items[:n]`.
fn py_slice_end(len: usize, n: i64) -> usize {
    if n >= 0 {
        usize::try_from(n).unwrap_or(usize::MAX).min(len)
    } else {
        len.saturating_sub(usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX))
    }
}

fn actor_name(x: &Value) -> Value {
    match x.get("actor").filter(|v| fsv_py::truthy(v)) {
        Some(Value::Object(m)) => m.get("name").cloned().unwrap_or_else(|| json!("?")),
        _ => json!("?"),
    }
}

/// `_local_dt`: ISO text in the local zone (naive values are local already).
fn local_dt(v: Option<&Value>) -> Option<jiff::Zoned> {
    let text = v.and_then(Value::as_str).filter(|s| !s.is_empty())?;
    let (civil, offset) = fsv_core::output::parse_isoformat(&text.replace('Z', "+00:00"))?;
    let tz = jiff::tz::TimeZone::system();
    match offset {
        Some(secs) => {
            let off = jiff::tz::Offset::from_seconds(secs).ok()?;
            Some(
                civil
                    .to_zoned(jiff::tz::TimeZone::fixed(off))
                    .ok()?
                    .with_time_zone(tz),
            )
        }
        None => civil.to_zoned(tz).ok(),
    }
}

fn clock(dt: Option<&jiff::Zoned>) -> String {
    dt.map_or_else(|| "--:--".into(), |d| d.strftime("%H:%M").to_string())
}

/// `_activity_day`.
fn activity_day(dt: Option<&jiff::Zoned>) -> String {
    let Some(dt) = dt else {
        return "Unknown".into();
    };
    let today = jiff::Zoned::now().date();
    if dt.date() == today {
        return "Today".into();
    }
    if today.yesterday().ok() == Some(dt.date()) {
        return "Yesterday".into();
    }
    dt.strftime("%Y-%m-%d").to_string()
}

/// `_activity_text`.
fn activity_text(v: Option<&Value>) -> String {
    static WS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\s+").unwrap());
    let html = fsv_core::output::strip_html(v.and_then(Value::as_str));
    pystr::strip(&WS.replace_all(&html, " ")).to_string()
}

/// `fsv RES fields`.
pub fn fields(res: &'static Resource, a: ResFieldsArgs) -> CmdResult {
    if a.default && a.custom {
        return Err(usage("use --default or --custom, not both"));
    }
    let c = client()?;
    let sch = schema::load(res, &c, a.refresh).map_err(fail_exc)?;
    let fmt = effective(a.format_, a.json_out);
    if let Some(choices) = a.choices.as_deref().filter(|s| !s.is_empty()) {
        let Some(f) = lookup::find_schema_field(&sch, choices)? else {
            return Err(usage(&format!(
                "field not found: {choices}; run `fsv {} fields`",
                res.name
            )));
        };
        let items: Vec<Value> = f
            .get("choices")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let row = |x: &Value| {
            [
                first_str(x, &["id"], "-"),
                first_str(x, &["value", "name", "label"], "-"),
                first_str(x, &["requester_display_value", "display_id"], ""),
            ]
        };
        let flat: Vec<Map<String, Value>> = items
            .iter()
            .map(|x| {
                let [id, value, detail] = row(x);
                obj(vec![
                    ("id", json!(id)),
                    ("value", json!(value)),
                    ("detail", json!(detail)),
                ])
            })
            .collect();
        if emit_fmt(&Value::Array(items.clone()), &flat, fmt)? {
            return Ok(());
        }
        let mut t = Table::new();
        t.add_column(Column::new("id"));
        t.add_column(Column::new("value"));
        t.add_column(Column::new("detail").style("dim"));
        for x in &items {
            t.add_row(row(x));
        }
        out().print_table(&t);
        return Ok(());
    }
    let is_default = |f: &Value| f.get("default_field").is_some_and(fsv_py::truthy);
    let fields: Vec<Value> = schema::fields(&sch)
        .into_iter()
        .filter(|f| {
            a.search
                .as_deref()
                .is_none_or(|s| s.is_empty() || lookup::field_matches(f, s))
        })
        .filter(|f| !a.default || is_default(f))
        .filter(|f| !a.custom || !is_default(f))
        .filter(|f| !a.required || f.get("required").is_some_and(fsv_py::truthy))
        .collect();
    let flat: Vec<Map<String, Value>> = fields
        .iter()
        .map(|f| f.as_object().cloned().unwrap_or_default())
        .collect();
    if emit_fmt(&Value::Array(fields.clone()), &flat, fmt)? {
        return Ok(());
    }
    let mut t = Table::new();
    t.add_column(Column::new("name"));
    t.add_column(Column::new("label"));
    t.add_column(Column::new("type"));
    t.add_column(Column::new("scope"));
    t.add_column(Column::new("req").justify(Justify::Center));
    t.add_column(Column::new("choices").style("dim"));
    let get = |f: &Value, k: &str| f.get(k).map_or_else(|| "?".to_string(), fsv_py::py_str);
    for f in &fields {
        let n = f
            .get("choices")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        t.add_row([
            get(f, "name"),
            get(f, "label"),
            get(f, "field_type"),
            lookup::field_scope(f).to_string(),
            if f.get("required").is_some_and(fsv_py::truthy) {
                "[red]*[/]".to_string()
            } else {
                String::new()
            },
            if n > 0 {
                n.to_string()
            } else {
                "-".to_string()
            },
        ]);
    }
    out().print_table(&t);
    Ok(())
}

/// `lookup_resource`.
pub fn lookup(res: &'static Resource, a: ResLookupArgs) -> CmdResult {
    let c = client()?;
    let key = pystr::casefold(&a.kind);
    let q = pystr::casefold(&a.query);
    let items: Vec<Value> = match key.as_str() {
        "requester" | "requesters" => {
            let params = obj(vec![("all_users", json!("true"))]);
            c.autocomplete("requesters", &a.query, Some(params))
                .map_err(fail_exc)?
        }
        "agent" | "agents" => c.autocomplete("agents", &a.query, None).map_err(fail_exc)?,
        "group" | "groups" => {
            let data = c
                .int_get("bootstrap/agents_groups", None)
                .map_err(fail_exc)?;
            data.get("groups")
                .and_then(Value::as_array)
                .map(|gs| {
                    gs.iter()
                        .filter(|g| pystr::casefold(&first_str(g, &["name"], "")).contains(&q))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => {
            let sch = schema::load(res, &c, false).map_err(fail_exc)?;
            let Some(f) = lookup::find_schema_field(&sch, &a.kind)? else {
                return Err(usage(&format!("unknown lookup kind or field: {}", a.kind)));
            };
            f.get("choices")
                .and_then(Value::as_array)
                .map(|cs| {
                    cs.iter()
                        .filter(|x| {
                            a.query.is_empty()
                                || pystr::casefold(&first_str(
                                    x,
                                    &["value", "label", "name", "id"],
                                    "None",
                                ))
                                .contains(&q)
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        }
    };
    let row = |x: &Value| {
        [
            first_str(x, &["user_id", "id"], "-"),
            first_str(x, &["value", "name", "label"], "-"),
            first_str(x, &["email", "details"], ""),
        ]
    };
    let flat: Vec<Map<String, Value>> = items
        .iter()
        .map(|x| {
            let [id, value, detail] = row(x);
            obj(vec![
                ("id", json!(id)),
                ("value", json!(value)),
                ("detail", json!(detail)),
            ])
        })
        .collect();
    if emit_fmt(
        &Value::Array(items.clone()),
        &flat,
        effective(a.format_, a.json_out),
    )? {
        return Ok(());
    }
    let mut t = Table::new();
    t.add_column(Column::new("id"));
    t.add_column(Column::new("value"));
    t.add_column(Column::new("detail").style("dim"));
    for x in &items {
        t.add_row(row(x));
    }
    out().print_table(&t);
    Ok(())
}

/// `_load_views`: the list under the first key ending in `_filters`.
fn load_views(c: &fsv_core::Client, res: &'static Resource) -> Result<Value> {
    let data = c.int_get(res.filters_path.unwrap_or(""), None)?;
    Ok(data
        .as_object()
        .and_then(|m| m.iter().find(|(k, _)| k.ends_with("_filters")))
        .map_or_else(|| json!([]), |(_, v)| v.clone()))
}

/// `_saved_views`: `[]` unless the session itself failed.
pub fn saved_views(c: &fsv_core::Client, res: &'static Resource) -> Result<Vec<SavedView>> {
    match load_views(c, res) {
        Ok(raw) => Ok(parse_saved_views(&raw)),
        Err(e @ Error::Session(_)) => Err(e),
        Err(_) => Ok(Vec::new()),
    }
}

/// `views_resource`.
pub fn views(res: &'static Resource, a: ResViewsArgs) -> CmdResult {
    let c = client()?;
    let saved = saved_views(&c, res).map_err(fail_exc)?;
    let default = default_view(&saved).cloned();
    let rows: Vec<Value> = saved
        .iter()
        .map(|v| {
            let mut m = v.to_json().as_object().cloned().unwrap_or_default();
            m.insert(
                "opens_by_default".into(),
                json!(Some(v) == default.as_ref()),
            );
            Value::Object(m)
        })
        .collect();
    let flat: Vec<Map<String, Value>> = saved
        .iter()
        .map(|v| {
            obj(vec![
                ("name", json!(v.name)),
                ("id", json!(v.id)),
                ("type", json!(if v.system { "system" } else { "custom" })),
                (
                    "default",
                    json!(if Some(v) == default.as_ref() {
                        "yes"
                    } else {
                        ""
                    }),
                ),
            ])
        })
        .collect();
    if emit_fmt(&Value::Array(rows), &flat, effective(a.format_, a.json_out))? {
        return Ok(());
    }
    if saved.is_empty() {
        err().print(&format!(
            "no saved views for {} (the account does not list them)",
            res.name
        ));
        return Ok(());
    }
    let mut t = Table::new();
    t.add_column(Column::new("").width(1));
    t.add_column(Column::new("View"));
    t.add_column(Column::new("Type").style("dim"));
    t.add_column(Column::new("id").style("dim"));
    for v in &saved {
        let marker = if Some(v) == default.as_ref() {
            "▸"
        } else {
            ""
        };
        t.add_row([
            marker.to_string(),
            escape(&v.name),
            if v.system { "system" } else { "custom" }.to_string(),
            escape(&v.id),
        ]);
    }
    out().print_table(&t);
    err().print(&format!(
        "fsv {} ls --view NAME  (exact name or a unique prefix)",
        res.name
    ));
    Ok(())
}

/// `_fulltext_search` (per resource).
pub fn search(res: &'static Resource, a: ResSearchArgs) -> CmdResult {
    let c = client()?;
    let sort = Some(a.sort.as_str());
    let total_of = |totals: &Map<String, Value>, fallback: usize| {
        totals
            .get(res.name)
            .cloned()
            .unwrap_or_else(|| json!(fallback))
    };
    let (results, total) = if a.n_pages.is_some() || a.all_pages {
        let mut all: Vec<Value> = Vec::new();
        let mut p = a.page;
        let mut fetched = 0;
        let mut total;
        loop {
            let (batch, totals) =
                service::search_items(c.as_ref(), &a.query, Some(res.name), sort, p)
                    .map_err(fail_exc)?;
            total = total_of(&totals, all.len() + batch.len());
            let n = batch.len();
            all.extend(batch);
            fetched += 1;
            err().print_plain(&format!(
                "  fetched {} / {}...",
                all.len(),
                fsv_py::py_str(&total)
            ));
            if n < 30 || a.n_pages.is_some_and(|limit| fetched >= limit) {
                break;
            }
            p += 1;
        }
        (all, total)
    } else {
        let (results, totals) =
            service::search_items(c.as_ref(), &a.query, Some(res.name), sort, a.page)
                .map_err(fail_exc)?;
        let total = total_of(&totals, results.len());
        (results, total)
    };
    let clean: Vec<Value> = results
        .iter()
        .map(|r| {
            let m: Map<String, Value> = r
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter(|(k, _)| !k.starts_with('_'))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            Value::Object(m)
        })
        .collect();
    let row = |r: &Value| {
        let did = if r.get("human_display_id").is_some_and(fsv_py::truthy) {
            first_str(r, &["human_display_id"], "")
        } else {
            first_str(r, &["display_id"], "-")
        };
        [
            did,
            head(&first_str(r, &["subject"], ""), 80),
            first_str(r, &["status"], "-"),
            first_str(r, &["priority_label"], "-"),
            first_str(r, &["_group"], "-"),
        ]
    };
    let flat: Vec<Map<String, Value>> = results
        .iter()
        .map(|r| {
            let [id, subject, status, priority, group] = row(r);
            obj(vec![
                ("id", json!(id)),
                ("subject", json!(subject)),
                ("status", json!(status)),
                ("priority", json!(priority)),
                ("group", json!(group)),
            ])
        })
        .collect();
    if emit_fmt(
        &Value::Array(clean),
        &flat,
        effective(a.format_, a.json_out),
    )? {
        return Ok(());
    }
    let mut t = Table::new();
    t.add_column(Column::new("ID").style("cyan").no_wrap());
    t.add_column(Column::new("Subject"));
    t.add_column(Column::new("Status").style("green"));
    t.add_column(Column::new("Pri"));
    t.add_column(Column::new("Group").style("dim"));
    for r in &results {
        t.add_row(row(r));
    }
    out().print_table(&t);
    err().print_plain(&format!(
        "{} rows (total: {})",
        results.len(),
        fsv_py::py_str(&total)
    ));
    Ok(())
}

/// `url_resource`.
pub fn url(res: &'static Resource, a: ResUrlArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    out().print(&format!(
        "https://{}/a/{}/{id}",
        config::require_domain()?,
        res.api_path
    ));
    Ok(())
}

/// Paging options of `notes` / `conversations`.
pub struct HistoryArgs<'a> {
    pub id_: &'a str,
    pub page: i64,
    pub per_page: i64,
    pub all_pages: bool,
    pub n_pages: Option<i64>,
    pub fmt: crate::types::OutputFormat,
}

/// `_history_rows`.
fn history_rows(
    c: &fsv_core::Client,
    res: &Resource,
    cid: i64,
    a: &HistoryArgs<'_>,
) -> Result<Vec<Value>> {
    let key = if res.name == TICKETS.name {
        "conversations"
    } else {
        "notes"
    };
    let include = if res.name == TICKETS.name {
        "user,phone,feedback"
    } else {
        "user"
    };
    let params = obj(vec![
        ("page", json!(a.page)),
        ("per_page", json!(a.per_page)),
        ("include", json!(include)),
    ]);
    let max_pages = a
        .n_pages
        .filter(|n| *n != 0)
        .unwrap_or(if a.all_pages { 50 } else { 1 })
        .clamp(0, 50) as usize;
    let got = service::collect_pages(
        |p, q| c.int_get(p, q),
        &format!("{}/{cid}/{key}", res.api_path),
        key,
        Some(&params),
        key,
        max_pages,
    )?;
    let pages = got.meta.get("pages").is_some_and(fsv_py::truthy);
    if let Some(miss) = &got.missing
        && !pages
    {
        err().print(&escape(&miss.note()));
        return Err(Error::exit(1, ""));
    }
    if let Some(p) = got.meta.get("partial").filter(|v| fsv_py::truthy(v)) {
        err().print(&escape(&fsv_py::py_str(p)));
    }
    Ok(got.rows)
}

/// `notes_resource` / `conversations_resource`.
pub fn history(res: &'static Resource, a: HistoryArgs<'_>) -> CmdResult {
    let id = cid(a.id_, res)?;
    let c = client()?;
    let items = history_rows(&c, res, id, &a).map_err(fail_exc)?;
    let tickets = res.name == TICKETS.name;
    let row = |x: &Value| {
        let author = x
            .get("user")
            .and_then(|u| u.get("name"))
            .filter(|v| fsv_py::truthy(v))
            .map_or_else(
                || x.get("user_id").map_or_else(|| "-".into(), fsv_py::py_str),
                fsv_py::py_str,
            );
        let label = if x.get("private").is_some_and(fsv_py::truthy) {
            if tickets { "private note" } else { "private" }
        } else if !tickets {
            "public"
        } else if x.get("incoming").is_some_and(fsv_py::truthy) {
            "incoming"
        } else {
            "public reply"
        };
        let body = match x.get("body_text").filter(|v| fsv_py::truthy(v)) {
            Some(b) => fsv_py::py_str(b),
            None => fsv_core::output::strip_html(x.get("body").and_then(Value::as_str)),
        };
        [
            x.get("id").map_or_else(|| "-".into(), fsv_py::py_str),
            head(&first_str(x, &["created_at"], "-"), 19),
            author,
            label.to_string(),
            body,
        ]
    };
    let label_key = if tickets { "kind" } else { "visibility" };
    let flat: Vec<Map<String, Value>> = items
        .iter()
        .map(|x| {
            let [id, created, author, label, body] = row(x);
            obj(vec![
                ("id", json!(id)),
                ("created_at", json!(created)),
                ("author", json!(author)),
                (label_key, json!(label)),
                ("body", json!(body)),
            ])
        })
        .collect();
    if emit_fmt(&Value::Array(items.clone()), &flat, a.fmt)? {
        return Ok(());
    }
    let mut t = Table::new();
    t.add_column(Column::new("ID").style("cyan"));
    t.add_column(Column::new("Created").style("dim"));
    t.add_column(Column::new("Author"));
    t.add_column(Column::new(if tickets { "Kind" } else { "Visibility" }));
    t.add_column(Column::new("Body"));
    for x in &items {
        t.add_row(row(x));
    }
    out().print_table(&t);
    Ok(())
}

/// `fsv RES notes ID`.
pub fn notes(res: &'static Resource, a: ResNotesArgs) -> CmdResult {
    history(
        res,
        HistoryArgs {
            id_: &a.id_,
            page: a.page,
            per_page: a.per_page,
            all_pages: a.all_pages,
            n_pages: a.n_pages,
            fmt: effective(a.format_, a.json_out),
        },
    )
}

/// `note_resource` (changes, problems).
pub fn add_note(res: &'static Resource, a: ResAddNoteArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let risks = safety::note_risks(a.public);
    if a.dry_run {
        emit_dry_run(
            res,
            id,
            json!({"body": a.body, "private": !a.public}),
            "add_note",
            &risks,
        );
        return Ok(());
    }
    confirm_writes(
        &risks,
        &format!("note on {}", format_cid(id, res)),
        a.yes,
        a.no_input,
    )?;
    let c = client()?;
    let n = service::add_note(c.as_ref(), res, id, &a.body, a.public).map_err(fail_exc)?;
    let visibility = if a.public { "public" } else { "private" };
    let nid = n.get("id").map_or("None".to_string(), fsv_py::py_str);
    out().print(&format!("[green]added[/] {visibility} note {nid}"));
    Ok(())
}

/// `reply_resource` (tickets).
pub fn reply(
    res: &'static Resource,
    id_: &str,
    body: &str,
    dry_run: bool,
    yes: bool,
    no_input: bool,
) -> CmdResult {
    let id = cid(id_, res)?;
    let risks = safety::reply_risks();
    if dry_run {
        emit_dry_run(res, id, json!({"body": body}), "reply", &risks);
        return Ok(());
    }
    confirm_writes(
        &risks,
        &format!("reply on {}", format_cid(id, res)),
        yes,
        no_input,
    )?;
    let c = client()?;
    let r = service::add_reply(c.as_ref(), res, id, body).map_err(fail_exc)?;
    let rid = r.get("id").map(fsv_py::py_str).unwrap_or_default();
    out().print(&format!("[green]added[/] reply {rid}"));
    Ok(())
}

/// Generic `update` (tickets, problems).
pub fn update(res: &'static Resource, a: ResUpdateArgs) -> CmdResult {
    if !a.tag.is_empty() || a.planned_start.is_some() || a.planned_end.is_some() {
        if res.name != TICKETS.name {
            return Err(plain_fail(
                "--tag and planned dates are supported only for tickets",
                2,
            ));
        }
        if a.status.is_some()
            || a.priority.is_some()
            || a.agent_id.is_some()
            || a.group_id.is_some()
            || !a.set_.is_empty()
        {
            return Err(plain_fail(
                "do not combine ticket properties with other update flags; review separate updates",
                2,
            ));
        }
        let c = client()?;
        return ticket_properties(&a, &c).map_err(|e| match e {
            Error::Value(m) => plain_fail(&m, 2),
            Error::Exit { code: 2, message } if !message.is_empty() => plain_fail(&message, 2),
            other => uncaught(other),
        });
    }
    if a.expect_plan.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err(usage(
            "--expect-plan is only supported with --tag, --planned-start or --planned-end",
        ));
    }
    update_resource(res, &a)
}

/// Reviewed ticket property update (`ticket_properties`).
fn ticket_properties(a: &ResUpdateArgs, c: &fsv_core::Client) -> CmdResult {
    let store = fsv_api::formmeta::DiskStore::current();
    let plan = fsv_api::properties::build_plan(
        c,
        &store,
        &a.id_,
        (!a.tag.is_empty()).then_some(a.tag.as_slice()),
        a.planned_start.as_deref(),
        a.planned_end.as_deref(),
    )?;
    let pretty = |v: &Value| {
        fsv_py::Dumps::default()
            .indent(2)
            .ensure_ascii(false)
            .dump(v)
    };
    let review = plan.review();
    if a.dry_run {
        let mut out = Map::new();
        out.insert("status".into(), json!("dry_run"));
        out.extend(review);
        println!("{}", pretty(&Value::Object(out)));
        return Ok(());
    }
    eprintln!("{}", pretty(&Value::Object(review)));
    let hash = plan.hash();
    if a.expect_plan.as_deref().is_some_and(|e| e != hash) {
        return Err(Error::exit(2, "review hash changed; review again"));
    }
    if a.yes {
        if a.expect_plan.as_deref() != Some(hash.as_str()) {
            return Err(Error::exit(
                2,
                "--yes requires the full --expect-plan from a prior review",
            ));
        }
    } else if crate::util::no_input(a.no_input) {
        return Err(Error::exit(
            2,
            "sending requires --yes and --expect-plan in no-input mode",
        ));
    } else if !fsv_core::term::confirm("Update these exact ticket properties?", Some(false), false)?
    {
        println!("Cancelled; no update attempted.");
        return Ok(());
    }
    let result = fsv_api::properties::send(
        &plan,
        c,
        &fsv_api::service::DiskJournal,
        &store,
        Some(&hash),
    )?;
    println!("{}", pretty(&result));
    Ok(())
}

/// `update_resource`.
fn update_resource(res: &'static Resource, a: &ResUpdateArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let c = client()?;
    let sch = schema::load(res, &c, false).map_err(fail_exc)?;
    let mut body = Map::new();
    if let Some(s) = &a.status {
        body.insert(
            "status".into(),
            lookup::resolve_update_choice(&sch, "status", s)?,
        );
    }
    if let Some(p) = &a.priority {
        body.insert(
            "priority".into(),
            lookup::resolve_update_choice(&sch, "priority", p)?,
        );
    }
    if let Some(agent) = &a.agent_id {
        let key = if res.name == TICKETS.name {
            "responder_id"
        } else {
            "agent_id"
        };
        let v = lookup::resolve_agent(&c, agent).map_err(uncaught)?;
        body.insert(key.into(), lookup::int_id(&v));
    }
    if let Some(group) = &a.group_id {
        let v = lookup::resolve_group(&c, group).map_err(uncaught)?;
        body.insert("group_id".into(), lookup::int_id(&v));
    }
    for expr in &a.set_ {
        let Some((field_text, raw_value)) = expr.split_once('=') else {
            return Err(usage(&format!(
                "--set requires FIELD=VALUE format, got: {}",
                fsv_py::py_repr_str(expr)
            )));
        };
        let field_text = pystr::strip(field_text);
        let field = lookup::find_schema_field(&sch, field_text)?;
        let (k, v) =
            lookup::resolve_set_field(&c, &sch, field_text, raw_value).map_err(uncaught)?;
        let f = field.unwrap_or(Value::Null);
        let ftype = lookup::s_or(&f, "field_type");
        let is_default = match f.get("default_field") {
            Some(d) => fsv_py::truthy(d),
            None => ftype.starts_with("default_"),
        };
        if res.name == TICKETS.name && !is_default {
            let cf = body.entry("custom_fields").or_insert_with(|| json!({}));
            if let Value::Object(m) = cf {
                m.insert(k, v);
            }
        } else {
            body.insert(k, v);
        }
    }
    if body.is_empty() {
        return Err(usage(
            "nothing to update; pass at least one field (--status, --priority, --agent, --group, --set FIELD=VALUE)",
        ));
    }
    let current = if a.dry_run && res.name == TICKETS.name {
        lookup::current_flat(res, id, &c)
    } else {
        None
    };
    let risks = lookup::risks_for_item_write(res, id, &body, &sch, &c, current.clone());
    if a.dry_run {
        let display_id = (res.name == TICKETS.name).then(|| {
            static TID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r"(?i)^(?:INC|SR)-[0-9]+$").expect("regex")
            });
            let human = current
                .as_ref()
                .and_then(|m| m.get("human_display_id"))
                .filter(|v| fsv_py::truthy(v))
                .map(fsv_py::py_str);
            match human {
                Some(h) if TID.is_match(&h) => h,
                _ if TID.is_match(pystr::strip(&a.id_)) => pystr::strip(&a.id_).to_uppercase(),
                _ => format!("tickets/{id}"),
            }
        });
        emit_dry_run_as(res, id, Value::Object(body), "update", &risks, display_id);
        return Ok(());
    }
    confirm_writes(&risks, &format_cid(id, res), a.yes, a.no_input)?;
    let item = service::update_item(c.as_ref(), res, id, &Value::Object(body)).map_err(fail_exc)?;
    if a.json_out {
        fsv_core::output::emit_json(&item);
        return Ok(());
    }
    out().print(&format!(
        "[green]updated[/] {}",
        fsv_core::resource::format_id(&item, res)
    ));
    Ok(())
}

/// Generic `download` (tickets, problems).
pub fn download(res: &'static Resource, a: ResDownloadArgs) -> CmdResult {
    crate::cmd::download::generic(res, a)
}
