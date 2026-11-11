//! Commands every resource group has (`_make_subapp`).

use serde_json::{Map, Value, json};

use fsv_api::service;
use fsv_api::site::{SavedView, default_view, parse_saved_views};
use fsv_core::resource::{Resource, TICKETS};
use fsv_core::safety;
use fsv_core::term::{Column, Table, err, escape, out};
use fsv_core::{Error, Result, config, pystr};

use crate::generated::*;
use crate::pending;
use crate::types::effective;
use crate::util::{
    CmdResult, cid, client, confirm_writes, emit_dry_run, emit_fmt, fail_exc, first_str,
    format_cid, head, obj, plain_fail, usage,
};

/// `fsv RES ls`.
pub fn ls(res: &Resource, _a: ResLsArgs) -> CmdResult {
    let c = client()?;
    pending::list_resource(res, &c)
}

/// `fsv RES get ID`.
pub fn get(res: &Resource, a: ResGetArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let c = client()?;
    pending::get_resource(res, &c, id)
}

/// `fsv RES activity ID`.
pub fn activity(res: &Resource, a: ResActivityArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let c = client()?;
    pending::tab_resource(res, &c, id, "activity")
}

/// `fsv RES tasks ID`.
pub fn tasks(res: &Resource, a: ResTasksArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let c = client()?;
    pending::tab_resource(res, &c, id, "tasks")
}

/// `fsv RES fields`.
pub fn fields(res: &Resource, a: ResFieldsArgs) -> CmdResult {
    if a.default && a.custom {
        return Err(usage("use --default or --custom, not both"));
    }
    let c = client()?;
    let sch = pending::schema_load(res, &c, a.refresh).map_err(fail_exc)?;
    pending::fields_resource(res, &sch).map_err(fail_exc)
}

/// `lookup_resource`.
pub fn lookup(res: &Resource, a: ResLookupArgs) -> CmdResult {
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
            let sch = pending::schema_load(res, &c, false).map_err(fail_exc)?;
            pending::fields_resource(res, &sch).map_err(fail_exc)?;
            Vec::new()
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
fn load_views(c: &fsv_core::Client, res: &Resource) -> Result<Value> {
    let data = c.int_get(res.filters_path.unwrap_or(""), None)?;
    Ok(data
        .as_object()
        .and_then(|m| m.iter().find(|(k, _)| k.ends_with("_filters")))
        .map_or_else(|| json!([]), |(_, v)| v.clone()))
}

/// `_saved_views`: `[]` unless the session itself failed.
pub fn saved_views(c: &fsv_core::Client, res: &Resource) -> Result<Vec<SavedView>> {
    match load_views(c, res) {
        Ok(raw) => Ok(parse_saved_views(&raw)),
        Err(e @ Error::Session(_)) => Err(e),
        Err(_) => Ok(Vec::new()),
    }
}

/// `views_resource`.
pub fn views(res: &Resource, a: ResViewsArgs) -> CmdResult {
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
pub fn search(res: &Resource, a: ResSearchArgs) -> CmdResult {
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
pub fn url(res: &Resource, a: ResUrlArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    out().print(&format!(
        "https://{}/a/{}/{id}",
        config::require_domain()?,
        res.api_path
    ));
    Ok(())
}

/// `notes_resource` / `conversations_resource`.
pub fn history(res: &Resource, id_: &str, kind: &str) -> CmdResult {
    let id = cid(id_, res)?;
    let c = client()?;
    pending::history_rows(res, &c, id, kind).map_err(fail_exc)
}

/// `fsv RES notes ID`.
pub fn notes(res: &Resource, a: ResNotesArgs) -> CmdResult {
    history(res, &a.id_, "notes")
}

/// `note_resource` (changes, problems).
pub fn add_note(res: &Resource, a: ResAddNoteArgs) -> CmdResult {
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
    res: &Resource,
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
pub fn update(res: &Resource, a: ResUpdateArgs) -> CmdResult {
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
        let _c = client()?;
        return pending::ticket_review("ticket_properties.build_plan");
    }
    if a.expect_plan.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err(usage(
            "--expect-plan is only supported with --tag, --planned-start or --planned-end",
        ));
    }
    let id = cid(&a.id_, res)?;
    let c = client()?;
    let _sch = pending::schema_load(res, &c, false).map_err(fail_exc)?;
    pending::update_resource(res, &c, id)
}

/// Generic `download` (tickets, problems).
pub fn download(res: &Resource, a: ResDownloadArgs) -> CmdResult {
    let id = cid(&a.id_, res)?;
    let c = client()?;
    pending::download(res, &c, id).map_err(fail_exc)
}
