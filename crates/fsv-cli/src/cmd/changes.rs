//! `changes`-only commands.

use std::io::IsTerminal;

use clap::ArgMatches;
use serde_json::{Map, Value, json};

use fsv_api::create::{DuplicateOpts, StdinPrompter};
use fsv_api::state_flow::Printed;
use fsv_core::Error;
use fsv_core::output::emit_json;
use fsv_core::resource::CHANGES;
use fsv_core::safety::{self, RiskContext};
use fsv_core::term::{self, Column, Justify, Table, err, out};

use crate::cmd::links::{self, AssetOpts, AssocOpts};
use crate::cmd::parse;
use crate::generated::*;
use std::sync::Arc;

use fsv_flows::change::{self, Plan, flow_exit};
use fsv_flows::drafts::{Draft, resolve_draft};
use fsv_flows::meta::{FormMeta, Resolver, is_status};
use fsv_formui::FormMode;

use crate::cmd::formflow::{self, Requester, Reviewed};
use crate::cmd::sr::expanduser;
use crate::types::{OutputFormat, effective};
use crate::util::{
    CmdResult, cid, client, confirm_writes, edit_body, emit_dry_run, emit_fmt, fail_exc, first_str,
    format_cid, no_input, stdin_tty, usage,
};

pub fn dispatch(sub: &str, m: &ArgMatches) -> CmdResult {
    match sub {
        "tasks-add" => crate::cmd::review::tasks_add(parse(m)?),
        "approval-request" => crate::cmd::review::approval_request(parse(m)?),
        "approval-respond" => crate::cmd::review::approval_respond(parse(m)?),
        "approval-cancel" => crate::cmd::review::approval_cancel(parse(m)?),
        "tasks-update" => tasks_update(parse(m)?),
        "tasks-delete" => tasks_delete(parse(m)?),
        "approvals" => approvals(parse(m)?),
        "state" => state(parse(m)?),
        "clone" => clone(parse(m)?),
        "create" => create(parse(m)?),
        "assets legacy" => assets_legacy(parse(m)?),
        "assets ls" => {
            let a: ChangesAssetsLsArgs = parse(m)?;
            links::assets(AssetOpts {
                page: a.page,
                per_page: a.per_page,
                category: a.category,
                fmt: a.format_,
                json_out: a.json_out,
                ..AssetOpts::new(&a.id_)
            })
        }
        "assets search" => {
            let a: ChangesAssetsSearchArgs = parse(m)?;
            links::assets(AssetOpts {
                search: Some(a.query),
                page: a.page,
                per_page: a.per_page,
                category: a.category,
                fmt: a.format_,
                json_out: a.json_out,
                ..AssetOpts::new(&a.id_)
            })
        }
        "assets add" => {
            let a: ChangesAssetsAddArgs = parse(m)?;
            links::assets(AssetOpts {
                add: a.assets_,
                category: a.category,
                dry_run: a.dry_run,
                yes: a.yes,
                json_out: a.json_out,
                ..AssetOpts::new(&a.id_)
            })
        }
        "assets remove" => {
            let a: ChangesAssetsRemoveArgs = parse(m)?;
            links::assets(AssetOpts {
                remove: a.assets_,
                category: a.category,
                dry_run: a.dry_run,
                yes: a.yes,
                json_out: a.json_out,
                ..AssetOpts::new(&a.id_)
            })
        }
        "assets pick" => {
            let a: ChangesAssetsPickArgs = parse(m)?;
            links::assets(AssetOpts {
                pick: true,
                category: a.category,
                dry_run: a.dry_run,
                yes: a.yes,
                json_out: a.json_out,
                ..AssetOpts::new(&a.id_)
            })
        }
        "assets categories" => {
            let a: ChangesAssetsCategoriesArgs = parse(m)?;
            links::categories(a.format_, a.json_out)
        }
        "associations legacy" => {
            let a: ChangesAssociationsLegacyArgs = parse(m)?;
            links::associations(AssocOpts {
                search: a.search,
                add: a.add,
                remove: a.remove,
                pick: a.pick,
                dry_run: a.dry_run,
                yes: a.yes,
                expect_plan: a.expect_plan,
                read_only: a.read_only,
                fmt: a.format_,
                json_out: a.json_out,
                ..AssocOpts::new(&a.id_)
            })
        }
        "associations ls" => {
            let a: ChangesAssociationsLsArgs = parse(m)?;
            links::associations(AssocOpts {
                fmt: a.format_,
                json_out: a.json_out,
                ..AssocOpts::new(&a.id_)
            })
        }
        "associations search" => {
            let a: ChangesAssociationsSearchArgs = parse(m)?;
            links::associations(AssocOpts {
                search: Some(a.query),
                fmt: a.format_,
                json_out: a.json_out,
                ..AssocOpts::new(&a.id_)
            })
        }
        "associations add" => {
            let a: ChangesAssociationsAddArgs = parse(m)?;
            links::associations(AssocOpts {
                add: a.tickets_,
                dry_run: a.dry_run,
                yes: a.yes,
                expect_plan: a.expect_plan,
                read_only: a.read_only,
                json_out: a.json_out,
                ..AssocOpts::new(&a.id_)
            })
        }
        "associations remove" => {
            let a: ChangesAssociationsRemoveArgs = parse(m)?;
            links::associations(AssocOpts {
                remove: a.tickets_,
                dry_run: a.dry_run,
                yes: a.yes,
                expect_plan: a.expect_plan,
                read_only: a.read_only,
                json_out: a.json_out,
                ..AssocOpts::new(&a.id_)
            })
        }
        "associations pick" => {
            let a: ChangesAssociationsPickArgs = parse(m)?;
            links::associations(AssocOpts {
                pick: true,
                dry_run: a.dry_run,
                yes: a.yes,
                expect_plan: a.expect_plan,
                read_only: a.read_only,
                json_out: a.json_out,
                ..AssocOpts::new(&a.id_)
            })
        }
        _ => Err(Error::other(format!("unknown command: changes {sub}"))),
    }
}

/// `change_flow.interactive()`.
fn interactive() -> bool {
    stdin_tty()
}

/// `change_cli.form_tty()`.
pub fn form_tty() -> bool {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    stdin_tty()
        && std::io::stdout().is_terminal()
        && !set("FSV_NO_FORM")
        && !std::env::var("TERM").is_ok_and(|t| t == "dumb")
}

/// `change_flow.resolve_attach_flow`.
pub fn resolve_attach_flow(opt: Option<&str>) -> Result<String, Error> {
    let cfg = fsv_core::config::load_config();
    let configured = cfg
        .get("changes")
        .and_then(|c| c.get("attach_flow"))
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str);
    let value = opt
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(configured)
        .unwrap_or_else(|| "after-create".into());
    if value != "after-create" && value != "before-create" {
        return Err(flow_exit(
            2,
            format!(
                "attach flow {}: expected one of after-create, before-create",
                fsv_py::py_repr_str(&value)
            ),
        ));
    }
    Ok(value)
}

fn tasks_update(a: ChangesTasksUpdateArgs) -> CmdResult {
    let id = cid(&a.id_, &CHANGES)?;
    let task_id = a.task_id;
    let status_opt = a.status.clone();
    let task_risks = |body: &Map<String, Value>, current: Option<Map<String, Value>>| {
        let current = current.or_else(|| {
            ["status", "owner_id", "agent_id", "group_id"]
                .iter()
                .any(|k| body.contains_key(*k))
                .then(|| {
                    let c = crate::util::shared_client().ok()?;
                    fsv_api::create::get_task_for_edit(c.as_ref(), id, task_id).ok()
                })
                .flatten()
        });
        let label = |v: &Value| match status_opt.as_deref() {
            Some(s) if !s.is_empty() && !is_digits(s) => s.to_string(),
            _ => format!("status {}", fsv_py::py_str(v)),
        };
        safety::write_risks(
            CHANGES.name,
            body,
            &RiskContext {
                current: current.as_ref(),
                status_label: Some(&label),
                kind: "task",
            },
        )
    };
    let (body, risks) = if a.edit {
        let c = client()?;
        let current =
            fsv_api::create::get_task_for_edit(c.as_ref(), id, task_id).map_err(fail_exc)?;
        let hint = format!("Editing task {task_id} on change #{id}");
        let mut body = edit_body(&Value::Object(current.clone()), &hint, a.no_input)?;
        if let Some(st) = body.get("status").cloned() {
            body.insert(
                "status".into(),
                json!(task_status_value(&fsv_py::py_str(&st)).map_err(fail_exc)?),
            );
        }
        let risks = task_risks(&body, Some(current));
        (body, risks)
    } else {
        let mut body = Map::new();
        let mut custom = Map::new();
        if let Some(st) = &a.status {
            body.insert(
                "status".into(),
                json!(task_status_value(st).map_err(fail_exc)?),
            );
        }
        if let Some(t) = &a.title {
            body.insert("title".into(), json!(t));
        }
        if let Some(g) = &a.group_id {
            let c = client()?;
            let gid = crate::lookup::resolve_group(c.as_ref(), g).map_err(fail_exc)?;
            body.insert("group_id".into(), py_int(&gid)?);
        }
        if let Some(ag) = &a.agent_id {
            let c = client()?;
            let uid = crate::lookup::resolve_agent(c.as_ref(), ag).map_err(fail_exc)?;
            body.insert("owner_id".into(), py_int(&uid)?);
        }
        for (key, v) in [
            ("due_date", &a.due_date),
            ("planned_start_date", &a.planned_start),
            ("planned_end_date", &a.planned_end),
        ] {
            if let Some(v) = v {
                body.insert(key.into(), json!(v));
            }
        }
        if let Some(v) = &a.system {
            custom.insert(
                "system".into(),
                json!(task_choice_value(v, "system").map_err(fail_exc)?),
            );
        }
        if let Some(v) = &a.environment {
            custom.insert(
                "environment".into(),
                json!(task_choice_value(v, "environment").map_err(fail_exc)?),
            );
        }
        if !custom.is_empty() {
            body.insert("custom_fields".into(), Value::Object(custom));
        }
        if body.is_empty() {
            return Err(usage("nothing to update; use --edit or pass task fields"));
        }
        let risks = task_risks(&body, None);
        (body, risks)
    };
    if a.dry_run {
        let mut out =
            json!({"action": "update_task", "change_id": id, "task_id": task_id, "body": body});
        if !risks.is_empty() {
            out["requires_confirmation"] = json!(risks);
        }
        emit_json(&out);
        return Ok(());
    }
    confirm_writes(
        &risks,
        &format!("task {task_id} on #{id}"),
        a.yes,
        a.no_input,
    )?;
    let c = client()?;
    let updated = fsv_api::create::update_task(c.as_ref(), id, task_id, &body).map_err(fail_exc)?;
    if a.json_out {
        emit_json(&updated);
    } else {
        out().print(&format!("[green]updated[/] task {task_id} on #{id}"));
    }
    Ok(())
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Python `int(text)` of a resolved ID.
fn py_int(text: &str) -> Result<Value, Error> {
    fsv_core::pystr::parse_decimal(text.trim())
        .map(|n| json!(n))
        .ok_or_else(|| {
            fail_exc(Error::value(format!(
                "invalid literal for int() with base 10: {}",
                fsv_py::py_repr_str(text)
            )))
        })
}

/// `_task_field_defs`: runtime task fields, or nothing on any failure.
fn task_field_defs() -> Vec<Value> {
    let Ok(c) = crate::util::shared_client() else {
        return Vec::new();
    };
    let Ok(data) = c.int_get(
        "change_task_fields",
        Some(crate::util::obj(vec![(
            "include_deleted_choices",
            json!("true"),
        )])),
    ) else {
        return Vec::new();
    };
    ["change_task_fields", "task_fields", "fields"]
        .iter()
        .find_map(|k| data.get(*k).and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

/// `_task_status_value`.
fn task_status_value(value: &str) -> Result<i64, Error> {
    let fields = task_field_defs();
    fsv_api::resolve(value, &fields)
        .map(|r| r.0)
        .map_err(|e| Error::value(e.to_string()))
}

/// `_task_choice_value`.
fn task_choice_value(value: &str, field: &str) -> Result<i64, Error> {
    let raw = fsv_core::pystr::strip(value);
    if is_digits(raw) {
        return py_int(raw).map(|v| v.as_i64().unwrap_or(0));
    }
    let mut mapping: std::collections::BTreeMap<String, i64> = Default::default();
    let cf = format!("cf_{field}");
    if let Some(item) = task_field_defs().into_iter().find(|i| {
        let n = i
            .get("name")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_default();
        n == field || n == cf
    }) {
        for choice in item
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let cid = choice
                .get("id")
                .filter(|v| fsv_py::truthy(v))
                .or_else(|| choice.get("display_id"));
            let label = first_str(choice, &["value", "label", "name"], "");
            if let Some(cid) = cid.filter(|v| !v.is_null())
                && !label.is_empty()
            {
                let n = fsv_core::pystr::parse_decimal(fsv_py::py_str(cid).trim()).unwrap_or(0);
                mapping.insert(fsv_core::pystr::casefold(&label), n);
            }
        }
    }
    let key = fsv_core::pystr::casefold(raw);
    match mapping.get(&key) {
        Some(v) => Ok(*v),
        None if !mapping.is_empty() => {
            let names: Vec<&str> = mapping.keys().map(String::as_str).collect();
            Err(Error::value(format!(
                "{field} must be numeric ID or one of: {}",
                names.join(", ")
            )))
        }
        None => Err(Error::value(format!("{field} must be numeric ID"))),
    }
}

fn tasks_delete(a: ChangesTasksDeleteArgs) -> CmdResult {
    let id = cid(&a.id_, &CHANGES)?;
    if a.dry_run {
        emit_json(&json!({"action": "delete_task", "change_id": id, "task_id": a.task_id}));
        return Ok(());
    }
    if !a.yes {
        if no_input(false) || !stdin_tty() {
            return Err(usage("pass --yes to delete task"));
        }
        term::confirm(
            &format!(
                "Delete task {} from {}?",
                a.task_id,
                format_cid(id, &CHANGES)
            ),
            Some(false),
            true,
        )?;
    }
    let c = crate::util::shared_client().map_err(fail_exc)?;
    fsv_api::create::delete_task(c.as_ref(), id, a.task_id).map_err(fail_exc)?;
    out().print(&format!("[green]deleted[/] task {} from #{id}", a.task_id));
    Ok(())
}

fn approvals(a: ChangesApprovalsArgs) -> CmdResult {
    let id = cid(&a.id_, &CHANGES)?;
    let c = client()?;
    let fmt = effective(a.format_, a.json_out);
    if a.groups {
        let items =
            fsv_api::create::get_change_approval_groups(c.as_ref(), id).map_err(approval_exc)?;
        let flat: Vec<Map<String, Value>> = items
            .iter()
            .map(|g| {
                let members: Vec<String> = g["approvals"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| fsv_py::py_str(&r["member_id"]))
                    .collect();
                crate::util::obj(vec![
                    ("group_id", g["id"].clone()),
                    ("status", g["approval_status"].clone()),
                    ("member_ids", json!(members.join(", "))),
                ])
            })
            .collect();
        if emit_fmt(&Value::Array(items.clone()), &flat, fmt)? {
            return Ok(());
        }
        let mut t = Table::with_headers(&["Group ID", "Raw status", "Member IDs"]);
        for g in &flat {
            t.add_row([
                fsv_py::py_str(&g["group_id"]),
                fsv_py::py_str(&g["status"]),
                fsv_py::py_str(&g["member_ids"]),
            ]);
        }
        out().print_table(&t);
        return Ok(());
    }
    let items = fsv_api::create::get_change_approvals(c.as_ref(), id).map_err(approval_exc)?;
    let row = |x: &Value| -> [String; 5] {
        let remark = x
            .get("remark")
            .filter(|r| fsv_py::truthy(r))
            .and_then(|r| r.get(0))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let raw_status = x
            .get("status")
            .filter(|v| fsv_py::truthy(v))
            .and_then(|s| s.get("name"))
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_else(|| "-".into());
        let decided = first_str(&remark, &["updated_at"], "-");
        let member = x
            .get("member")
            .filter(|v| fsv_py::truthy(v))
            .and_then(|m| m.get("name"))
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_else(|| x.get("member_id").map_or("-".into(), fsv_py::py_str));
        [
            x.get("level_id").map_or("-".into(), fsv_py::py_str),
            member,
            if raw_status == "peer_responded" {
                "peer approved".into()
            } else {
                raw_status
            },
            if decided == "-" {
                decided
            } else {
                decided.chars().take(10).collect()
            },
            first_str(&remark, &["data"], ""),
        ]
    };
    let flat: Vec<Map<String, Value>> = items
        .iter()
        .map(|x| {
            ["level", "approver", "status", "decided", "message"]
                .iter()
                .zip(row(x))
                .map(|(k, v)| (k.to_string(), json!(v)))
                .collect()
        })
        .collect();
    if emit_fmt(&Value::Array(items.clone()), &flat, fmt)? {
        return Ok(());
    }
    let mut t = Table::new();
    t.add_column(Column::new("Level").style("cyan"));
    for h in ["Approver", "Status", "Decided", "Message"] {
        t.add_column(Column::new(h));
    }
    for x in &items {
        t.add_row(row(x));
    }
    out().print_table(&t);
    Ok(())
}

/// `_err_exc` of an `ApprovalError` (a `RuntimeError`: exit 1).
fn approval_exc(e: Error) -> Error {
    match e {
        Error::Exit { code: 2, message } if !message.is_empty() => {
            crate::util::fail(&term::escape(&message), 1)
        }
        other => fail_exc(other),
    }
}

fn state(a: ChangesStateArgs) -> CmdResult {
    let id = cid(&a.id_, &CHANGES)?;
    let c = client()?;
    let data = c
        .int_get(&format!("changes/{id}"), None)
        .map_err(fail_exc)?;
    let change = match data.get("change") {
        Some(ch) => ch.clone(),
        None => data,
    };
    match effective(a.format_, a.json_out) {
        OutputFormat::Json => {
            let Some(flow_id) = change.get("state_flow_id") else {
                return Err(usage("no state_flow_id on this change"));
            };
            let flow = fsv_api::state_flow::get_state_flow(
                c.as_ref(),
                flow_id,
                change.get("workspace_id"),
            )
            .map_err(fail_exc)?;
            emit_json(&json!({"change": change, "flow": flow}));
        }
        f @ (OutputFormat::Csv | OutputFormat::Tsv) => {
            let row = change.as_object().cloned().unwrap_or_default();
            fsv_core::output::emit_delimited(
                &[row],
                if f == OutputFormat::Csv { ',' } else { '\t' },
            )?;
        }
        OutputFormat::Table => {
            let printed =
                fsv_api::state_flow::render_flow(c.as_ref(), &change).map_err(fail_exc)?;
            print_flow(&printed);
        }
    }
    Ok(())
}

/// Print `state_flow.render_flow` output to the console.
fn print_flow(printed: &[Printed]) {
    for p in printed {
        match p {
            Printed::Text(s) => out().print(s),
            Printed::Blank => out().blank(),
            Printed::Table { title, rows } => {
                let mut t = Table::new().title(title.clone());
                t.add_column(
                    Column::new("→")
                        .justify(Justify::Center)
                        .style("dim")
                        .width(3),
                );
                t.add_column(Column::new("Status").no_wrap());
                t.add_column(
                    Column::new("State")
                        .justify(Justify::Center)
                        .style("dim")
                        .width(7),
                );
                for r in rows {
                    t.add_row(r.clone());
                }
                out().print_table(&t);
            }
        }
    }
}

fn clone(a: ChangesCloneArgs) -> CmdResult {
    let id = cid(&a.id_, &CHANGES)?;
    let c = client()?;
    let data = fsv_api::create::change_clone_data(c.as_ref(), id).map_err(fail_exc)?;
    if a.dry {
        emit_json(&Value::Object(data));
        if a.with_tasks || a.with_assets || a.with_planning {
            let fx = fsv_api::create::clone_side_effects(c.as_ref(), id).map_err(fail_exc)?;
            let n = |k: &str| fx.get(k).map(fsv_py::py_str).unwrap_or_default();
            let e = term::err();
            e.print("[bold]side effects if run without --dry-run:[/bold]");
            if a.with_tasks {
                let keep = if a.keep_task_owners {
                    "kept"
                } else {
                    "dropped"
                };
                e.print(&format!(
                    "  - {} task(s) created ({} with an owner; owners/past dates {keep})",
                    n("tasks"),
                    n("tasks_with_owner")
                ));
                e.print("  - tenant workflows may add their own tasks to the new change");
            }
            if a.with_assets {
                e.print(&format!("  - {} asset(s) associated", n("assets")));
            }
            if a.with_planning {
                e.print(&format!(
                    "  - {} planning field(s) copied, {} attachment(s) re-uploaded",
                    n("planning_fields"),
                    n("planning_attachments")
                ));
            }
        }
        return Ok(());
    }
    confirm_writes(
        &safety::clone_risks(a.with_tasks),
        &format!("clone of {}", format_cid(id, &CHANGES)),
        a.yes,
        a.no_input,
    )?;
    let body = if a.no_edit {
        data
    } else {
        edit_body(
            &Value::Object(data),
            &format!("Cloning change #{id}"),
            a.no_input,
        )?
    };
    let sch = crate::schema::load(&CHANGES, &c, false).map_err(fail_exc)?;
    let created = fsv_api::create::submit_change(c.as_ref(), &body, &crate::schema::fields(&sch))
        .map_err(fail_exc)?;
    let cid_out = created.get("id").cloned().unwrap_or_else(|| json!("?"));
    let label = fsv_core::resource::format_id(&created, &CHANGES);
    let e = term::err();
    if a.json_out {
        e.print(&format!("created {label}"));
    } else {
        out().print(&format!(
            "[green]created[/] {label}  {}/{}",
            CHANGES.portal_url(),
            fsv_py::py_str(&cid_out)
        ));
    }
    let missing_id = cid_out == json!("?");
    if missing_id {
        e.print(
            "[yellow]warning[/yellow]: change created but the response had no id; see `fsv journal tail`",
        );
    }
    let mut steps = Map::new();
    let mut failures: Vec<Error> = Vec::new();
    let mut step = |name: &str, r: Result<String, Error>| match r {
        Ok(msg) => {
            steps.insert(name.into(), json!("ok"));
            e.print(&format!("  {msg}"));
        }
        Err(err) => {
            steps.insert(name.into(), json!(format!("failed: {err}")));
            e.print(&format!("[red]step failed[/red]: {name}: {err}"));
            failures.push(err);
        }
    };
    if !missing_id {
        let new_id = fsv_core::pystr::parse_decimal(&fsv_py::py_str(&cid_out)).unwrap_or(0);
        if a.with_tasks {
            let mut dropped = Vec::new();
            let r = fsv_api::create::clone_tasks(
                c.as_ref(),
                id,
                new_id,
                a.keep_task_owners,
                None,
                Some(&mut dropped),
            )
            .map(|t| format!("cloned {} task(s)", t.len()));
            step("tasks", r);
            dropped.sort();
            dropped.dedup();
            for note in dropped {
                e.print(&format!("  note: {note}"));
            }
            e.print("  note: tenant workflows may have added their own tasks; compare with the source before relying on the copy");
        }
        if a.with_assets {
            let r = fsv_api::create::clone_assets(c.as_ref(), id, new_id)
                .map(|ids| format!("associated {} asset(s)", ids.len()));
            step("assets", r);
        }
        if a.with_planning {
            let r = fsv_api::create::clone_planning_fields(c.as_ref(), id, new_id)
                .map(|f| format!("cloned {} planning field(s)", f.len()));
            step("planning", r);
        }
    }
    if a.json_out {
        let mut out = created.as_object().cloned().unwrap_or_default();
        if !steps.is_empty() {
            out.insert("fsv_clone_steps".into(), Value::Object(steps));
        }
        emit_json(&Value::Object(out));
    }
    if let Some(first) = failures.first() {
        e.print(&format!(
            "[red]error[/red]: {label} WAS created but {} step(s) failed; do not rerun clone (it would duplicate the change). Fix the failed step(s) on {label} directly.",
            failures.len()
        ));
        return Err(Error::exit(first.exit_code(), ""));
    }
    Ok(())
}

fn create(mut a: ChangesCreateArgs) -> CmdResult {
    if a.description.as_deref() == Some("-") {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        a.description = Some(s);
    }
    let tty = interactive() && !no_input(a.no_input);
    let attach_flow = resolve_attach_flow(a.attach_flow.as_deref())?;
    let legacy = [
        &a.subject,
        &a.description,
        &a.status,
        &a.priority,
        &a.agent_id,
        &a.group_id,
    ]
    .iter()
    .any(|v| v.is_some());
    let has_input = a.from_.is_some()
        || !a.set_.is_empty()
        || a.preset.is_some()
        || legacy
        || !a.attach.is_empty();
    let mut edit = a.edit;
    let mut form = false;
    let mut template_only = false;
    if !has_input && !edit {
        if tty && form_tty() {
            form = true;
        } else if a.dry {
            template_only = true;
        } else if tty {
            edit = true;
        } else {
            return Err(flow_exit(
                2,
                "nothing to create: pass --from FILE, --set LABEL=VALUE or --preset NAME (the interactive form and --edit need a terminal); see `fsv changes create --help`",
            ));
        }
    }
    if edit && !tty {
        return Err(flow_exit(
            2,
            "--edit needs a terminal (no TTY or --no-input); use --from FILE",
        ));
    }
    let c = client()?;
    let modes = CreateModes {
        tty,
        edit,
        form,
        template_only,
        attach_flow,
    };
    change_create(&a, c, modes).map_err(fail_exc)
}

struct CreateModes {
    tty: bool,
    edit: bool,
    form: bool,
    template_only: bool,
    attach_flow: String,
}

/// `change_cli.create_command` after the input checks.
fn change_create(a: &ChangesCreateArgs, c: Arc<fsv_core::Client>, m: CreateModes) -> CmdResult {
    let as_json = a.json_out || a.format_ == OutputFormat::Json;
    let tty = m.tty;
    let deps = fsv_app::FormDeps::live(c)?;
    let fresh = a.dry || a.yes || !tty;
    let meta = Arc::new(deps.meta.load_meta("change", "new", fresh, &Value::Null)?);
    let mut extra = Draft {
        attachments: a.attach.iter().map(|p| expanduser(p)).collect(),
        ..Draft::default()
    };
    extra.answers = change::legacy_answers(
        &meta,
        a.subject.as_deref(),
        a.priority.as_deref(),
        a.agent_id.as_deref(),
        a.group_id.as_deref(),
    )?;
    if let Some(status) = &a.status
        && let Some(sf) = meta.spec().fields().iter().find(|f| is_status(f))
    {
        extra.answers.insert(sf.api_name.clone(), json!(status));
    }
    if let Some(d) = &a.description {
        extra.description_html = Some(d.clone());
    }
    let draft = resolve_draft(
        a.from_.as_deref(),
        a.preset.as_deref(),
        &a.set_,
        Some(&extra),
    )
    .map_err(formflow::draft_error)?;
    let who = Requester::new(deps.api.as_ref(), a.requester.clone(), tty);
    if tty && !m.form && !m.edit && !a.dry && !m.template_only && who.value(&draft).is_none() {
        who.ask()?;
    }
    let resolver: Arc<dyn Resolver> = deps.resolver.clone();
    let build = |d: &Draft, ack_all: bool, mt: Option<Arc<FormMeta>>| -> Result<Plan, Error> {
        let (req, problem) = who.resolve(d)?;
        let opts = change::CreateOptions {
            resolver: Some(resolver.clone()),
            requester: req,
            ack_all,
            allow_incomplete: a.no_validate,
            hints_path: None,
        };
        let mut plan = change::build_create_plan(&mt.unwrap_or_else(|| meta.clone()), d, &opts)?;
        if let Some(p) = problem {
            let mut errors = vec![p];
            errors.extend(
                plan.errors
                    .drain(..)
                    .filter(|e| !e.starts_with("requester is not set")),
            );
            plan.errors = errors;
            plan.review.errors = plan.errors.clone();
        }
        Ok(plan)
    };

    if m.template_only {
        let plan = build(&draft, a.ack, None)?;
        print!(
            "{}",
            change::template_for(
                &plan,
                &draft,
                "fsv change draft: fill in, then `fsv changes create --from FILE --dry-run`",
                &[],
                None,
                None
            )?
        );
        return Ok(());
    }
    let plan = if m.form {
        let mut req = None;
        if a.requester.as_deref().is_some_and(|r| !r.is_empty()) {
            let (r, problem) = who.resolve(&draft)?;
            req = r;
            if let Some(p) = problem {
                err().print(&format!("[yellow]--requester[/yellow]: {p}"));
            }
        }
        let session = fsv_app::change_session(
            &deps,
            fsv_app::ChangeForm {
                meta: Some(meta.clone()),
                requester: req,
                dry_run: a.dry,
                ..fsv_app::ChangeForm::new(FormMode::New)
            },
        )?;
        Reviewed::Form(Box::new(formflow::run_form(session)?))
    } else if m.edit {
        let mut b = |d: &Draft| build(d, a.ack, None);
        let mut ed = change::editor_edit;
        let (_, plan) = change::edit_loop(&mut b, draft, "New change", None, &mut ed, 20)
            .map_err(formflow::editor_abort)?;
        Reviewed::Flow(Box::new(plan))
    } else {
        Reviewed::Flow(Box::new(build(&draft, a.ack, None)?))
    };

    if a.dry {
        print_change_plan(&plan, as_json, "create", None);
        if !plan.ok() {
            return Err(flow_exit(2, ""));
        }
        return plan.expect(a.expect_plan.as_deref());
    }
    let expect = a.expect_plan.as_deref();
    let mut plan = match plan {
        Reviewed::Form(s) => {
            formflow::expect_hash(&s.plan.hash, &s.plan.errors, expect)?;
            let result = s.session.send(&s.plan, a.allow_duplicate)?;
            return report_created(&result, as_json);
        }
        Reviewed::Flow(p) => *p,
    };
    if tty && !a.yes {
        let rebuild = |d: &Draft, ack: bool| build(d, ack || a.ack, None);
        let draft = plan.draft.clone();
        (plan, _) = confirm_loop(plan, draft, &rebuild, "New change", "create")?;
    } else {
        if !tty && !a.yes {
            plan.review.render(err());
            return Err(flow_exit(
                2,
                "not a terminal: review with --dry-run, then send with --yes --expect-plan HASH",
            ));
        }
        if !plan.ok() {
            plan.review.render(err());
            return Err(flow_exit(
                2,
                format!("cannot create: {}", plan.errors.join("; ")),
            ));
        }
        let blocking = plan.review.blocking_unacked();
        if !blocking.is_empty() {
            plan.review.render(err());
            return Err(flow_exit(
                2,
                format!(
                    "blocking rule message(s) need --ack: {}",
                    blocking.join("; ")
                ),
            ));
        }
        change::expect(&plan, expect)?;
    }
    let ack_now = a.ack || !plan.state.acknowledged.is_empty();
    let d = plan.draft.clone();
    let mut rebuild = |mt: Arc<FormMeta>| build(&d, ack_now, Some(mt));
    let send = SendOpts {
        tty,
        yes: a.yes,
        expect,
        allow_duplicate: a.allow_duplicate,
        duplicate: "append",
        attach_flow: Some(&m.attach_flow),
    };
    let result = send_plan(plan, &deps, &mut rebuild, &send, "change")?;
    report_created(&result, as_json)
}

/// `change_cli.update_command` after the mode dispatch.
fn change_update(
    a: &ChangesUpdateArgs,
    id: i64,
    c: Arc<fsv_core::Client>,
    tty: bool,
    form: bool,
) -> CmdResult {
    let subject = fsv_core::resource::format_id(&json!({"id": id}), &CHANGES);
    let data = c.int_get(&format!("changes/{id}"), None)?;
    let record = match &data {
        Value::Object(o) => o.get("change").cloned().unwrap_or_else(|| data.clone()),
        _ => json!({}),
    };
    let rec = |k: &str| record.get(k).filter(|v| fsv_py::truthy(v)).cloned();
    let module_id = rec("display_id").or_else(|| rec("id")).unwrap_or(json!(id));
    let deps = fsv_app::FormDeps::live(c)?;
    let fresh = a.dry_run || a.yes || !tty;
    let meta = Arc::new(deps.meta.load_meta("change", "edit", fresh, &module_id)?);
    let mut extra = Draft {
        attachments: a.attach.iter().map(|p| expanduser(p)).collect(),
        ..Draft::default()
    };
    extra.answers = change::legacy_answers(
        &meta,
        None,
        a.priority.as_deref(),
        a.agent_id.as_deref(),
        a.group_id.as_deref(),
    )?;
    if let Some(status) = &a.status {
        let spec = meta.spec();
        let Some(sf) = spec.fields().iter().find(|f| is_status(f)) else {
            return Err(flow_exit(2, "this form has no status field"));
        };
        extra.answers.insert(sf.api_name.clone(), json!(status));
    }
    let draft = resolve_draft(
        a.from_.as_deref(),
        a.preset.as_deref(),
        &a.set_,
        Some(&extra),
    )
    .map_err(formflow::draft_error)?;
    let resolver: Arc<dyn Resolver> = deps.resolver.clone();
    let record_map = record.as_object().cloned().unwrap_or_default();
    let build = |d: &Draft, ack_all: bool, mt: Option<Arc<FormMeta>>| -> Result<Plan, Error> {
        let opts = change::UpdateOptions {
            resolver: Some(resolver.clone()),
            ack_all,
            status_label: None,
            hints_path: None,
        };
        change::build_update_plan(
            &mt.unwrap_or_else(|| meta.clone()),
            d,
            &record_map,
            id,
            &opts,
        )
    };
    let title = format!("Edit {subject}");
    let plan = if form {
        let session = fsv_app::change_session(
            &deps,
            fsv_app::ChangeForm {
                target: Some(json!(id)),
                record: Some(record.clone()),
                meta: Some(meta.clone()),
                dry_run: a.dry_run,
                title: Some(title.clone()),
                ..fsv_app::ChangeForm::new(FormMode::Edit)
            },
        )?;
        Reviewed::Form(Box::new(formflow::run_form(session)?))
    } else if a.edit {
        let mut b = |d: &Draft| build(d, a.ack, None);
        let mut ed = change::editor_edit;
        let (_, plan) = change::edit_loop(&mut b, draft, &title, None, &mut ed, 20)
            .map_err(formflow::editor_abort)?;
        Reviewed::Flow(Box::new(plan))
    } else {
        Reviewed::Flow(Box::new(build(&draft, a.ack, None)?))
    };

    if a.dry_run {
        print_change_plan(&plan, a.json_out, "update", Some(id));
        if !plan.ok() {
            return Err(flow_exit(2, ""));
        }
        return plan.expect(a.expect_plan.as_deref());
    }
    let no_changes = || out().print(&format!("[yellow]no changes[/] {subject}"));
    if plan.ok() && plan.hash().is_empty() {
        no_changes();
        return Ok(());
    }
    let expect = a.expect_plan.as_deref();
    let result = match plan {
        Reviewed::Form(s) => {
            formflow::expect_hash(&s.plan.hash, &s.plan.errors, expect)?;
            s.session.send(&s.plan, true)?
        }
        Reviewed::Flow(p) => {
            let mut plan = *p;
            if tty && !a.yes {
                let rebuild = |d: &Draft, ack: bool| build(d, ack || a.ack, None);
                let draft = plan.draft.clone();
                (plan, _) = confirm_loop(plan, draft, &rebuild, &title, "update")?;
                if plan.hash.is_empty() {
                    no_changes();
                    return Ok(());
                }
            } else {
                if !plan.ok() {
                    plan.review.render(err());
                    return Err(flow_exit(
                        2,
                        format!("cannot update: {}", plan.errors.join("; ")),
                    ));
                }
                if !plan.risks.is_empty() && !a.yes {
                    return Err(flow_exit(
                        2,
                        format!(
                            "{subject}: {}. Pass --yes to confirm.",
                            plan.risks.join("; ")
                        ),
                    ));
                }
                let blocking = plan.review.blocking_unacked();
                if !blocking.is_empty() {
                    return Err(flow_exit(
                        2,
                        format!(
                            "blocking rule message(s) need --ack: {}",
                            blocking.join("; ")
                        ),
                    ));
                }
                change::expect(&plan, expect)?;
            }
            let d = plan.draft.clone();
            let mut rebuild = |mt: Arc<FormMeta>| build(&d, a.ack, Some(mt));
            let send = SendOpts {
                tty,
                yes: a.yes,
                expect,
                allow_duplicate: true,
                duplicate: &a.duplicate,
                attach_flow: None,
            };
            send_plan(plan, &deps, &mut rebuild, &send, "change")?
        }
    };
    let steps = result
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if a.json_out {
        let mut o = Map::new();
        o.insert(
            "updated".into(),
            result.get("result").cloned().unwrap_or(Value::Null),
        );
        o.insert(
            "plan_hash".into(),
            result.get("plan_hash").cloned().unwrap_or(Value::Null),
        );
        o.insert("steps".into(), Value::Array(steps));
        println!("{}", formflow::pretty(&Value::Object(o)));
        return Ok(());
    }
    out().print(&format!("[green]updated[/] {subject}"));
    print_steps(&steps);
    Ok(())
}

fn print_steps(steps: &[Value]) {
    for s in steps.iter().skip(1) {
        out().print(&format!(
            "  [green]ok[/] {}",
            fsv_py::py_str(s.get("step").unwrap_or(&Value::Null))
        ));
    }
}

/// `change_cli._print_plan` (to stdout).
fn print_change_plan(plan: &Reviewed, as_json: bool, action: &str, target: Option<i64>) {
    if as_json {
        println!("{}", formflow::pretty(&plan.to_json(action)));
        return;
    }
    let o = out();
    plan.render(o);
    match plan.payload().filter(|_| plan.ok()) {
        None => o.print(&format!("[dim]{}[/dim]", fsv_core::review::NO_PAYLOAD)),
        Some(payload) => {
            let what = match (action, target) {
                ("update", Some(t)) => format!(" (PUT changes/{t})"),
                ("update", None) => " (PUT changes/None)".to_string(),
                _ => " (POST /api/_/changes)".to_string(),
            };
            o.print(&format!("[bold]Payload[/bold]{what}"));
            println!("{}", formflow::pretty(&payload));
        }
    }
    let h = plan.hash();
    if !h.is_empty() {
        o.print(&format!("plan_hash: {h}"));
        o.print(&format!(
            "[dim]send exactly this with: --yes --expect-plan {}[/dim]",
            &h[..h.len().min(16)]
        ));
    }
}

/// `change_cli._menu`.
pub(crate) fn menu(options: &str) -> Result<char, Error> {
    loop {
        let ans = term::prompt(options, Some("n"), false)?;
        let c = fsv_core::pystr::strip(&ans).to_lowercase().chars().next();
        if let Some(c) = c.filter(|c| "yesn".contains(*c)) {
            return Ok(c);
        }
    }
}

/// `change_cli._ack_blocking`.
fn ack_blocking(plan: &Plan) -> Result<bool, Error> {
    let msgs = plan.review.blocking_unacked();
    if msgs.is_empty() {
        return Ok(false);
    }
    for m in msgs {
        err().print(&format!("[magenta]blocking rule message:[/] {m}"));
    }
    term::confirm("Acknowledge and continue?", Some(false), false)
}

/// `change_cli._confirm_loop`: [y] send / [e] edit / [s] save draft / [n] cancel.
fn confirm_loop(
    mut plan: Plan,
    mut draft: Draft,
    build: &dyn Fn(&Draft, bool) -> Result<Plan, Error>,
    title: &str,
    verb: &str,
) -> Result<(Plan, Draft), Error> {
    let mut ack = false;
    loop {
        plan.review.render(err());
        let choice = if plan.ok() {
            menu(&format!("[y] {verb}  [e] edit  [s] save draft  [n] cancel"))?
        } else {
            let c = menu("Problems must be fixed first: [e] edit  [s] save draft  [n] cancel")?;
            if c == 'y' {
                continue;
            }
            c
        };
        match choice {
            'y' => {
                if !plan.review.blocking_unacked().is_empty() {
                    if !ack_blocking(&plan)? {
                        continue;
                    }
                    ack = true;
                    plan = build(&draft, ack)?;
                    if !plan.ok() {
                        continue;
                    }
                }
                return Ok((plan, draft));
            }
            'e' => {
                let mut b = |d: &Draft| build(d, ack);
                let mut ed = change::editor_edit;
                match change::edit_loop(&mut b, draft.clone(), title, None, &mut ed, 20) {
                    Ok((d, p)) => {
                        draft = d;
                        plan = p;
                    }
                    Err(Error::EditorAbort(m)) => {
                        err().print(&format!("[yellow]{m}[/yellow]; keeping the previous draft"));
                    }
                    Err(e) => return Err(e),
                }
            }
            's' => {
                let path = term::prompt("Save draft to", Some("change-draft.yaml"), true)?;
                if std::path::Path::new(&path).exists()
                    && !term::confirm(&format!("{path} exists; overwrite?"), Some(false), false)?
                {
                    continue;
                }
                let saved = change::save_draft(&plan, &draft, &path)?;
                err().print(&format!(
                    "[green]saved[/green] {saved}; continue later with --from {saved}"
                ));
                return Err(flow_exit(0, ""));
            }
            _ => return Err(flow_exit(1, "cancelled; nothing was sent")),
        }
    }
}

struct SendOpts<'a> {
    tty: bool,
    yes: bool,
    expect: Option<&'a str>,
    allow_duplicate: bool,
    duplicate: &'a str,
    attach_flow: Option<&'a str>,
}

/// `change_cli._send`.
fn send_plan(
    plan: Plan,
    deps: &fsv_app::FormDeps,
    rebuild: &mut dyn FnMut(Arc<FormMeta>) -> Result<Plan, Error>,
    o: &SendOpts<'_>,
    what: &str,
) -> Result<Value, Error> {
    let journal = deps.plan_journal.as_ref();
    change::check_duplicate(&plan, o.allow_duplicate, journal)?;
    let (plan, diff) = change::freshen(plan, deps.meta.as_ref(), rebuild)?;
    if let Some(diff) = diff {
        err().print(
            "[yellow]form metadata changed since the review; the plan is now different:[/yellow]",
        );
        for line in diff {
            err().print(&format!("  {line}"));
        }
        if !plan.ok() {
            plan.review.render(err());
            return Err(flow_exit(
                2,
                format!(
                    "the updated form needs more answers: {}",
                    plan.errors.join("; ")
                ),
            ));
        }
        if o.expect.is_some_and(|e| !e.is_empty()) || o.yes || !o.tty {
            let h = if plan.hash.is_empty() {
                "(incomplete)"
            } else {
                &plan.hash
            };
            return Err(flow_exit(
                4,
                format!("plan changed to {h}; review it again with --dry-run"),
            ));
        }
        plan.review.render(err());
        if !term::confirm("Send the updated plan?", Some(false), false)? {
            return Err(flow_exit(1, "cancelled; nothing was sent"));
        }
        change::check_duplicate(&plan, o.allow_duplicate, journal)?;
    }
    change::expect(&plan, o.expect)?;
    match change::execute(
        &plan,
        deps.writer.as_ref(),
        journal,
        o.duplicate,
        o.attach_flow,
    ) {
        Ok(v) => Ok(v),
        Err(change::ExecError::Failed(e)) => Err(e),
        Err(change::ExecError::Partial(p)) => Err(report_partial(&p, what)),
    }
}

/// `change_cli._report_partial`.
fn report_partial(e: &change::PartialFailure, what: &str) -> Error {
    let id = fsv_py::py_str(&e.created_id);
    err().print(&format!(
        "[red]error[/red]: {what} {id} was written, but a later step failed: {}",
        e.error
    ));
    if !e.uploaded.is_empty() {
        let ids: Vec<String> = e
            .uploaded
            .iter()
            .map(|u| {
                format!(
                    "{} ({})",
                    fsv_py::py_str(u.get("id").unwrap_or(&Value::Null)),
                    fsv_py::py_str(u.get("name").unwrap_or(&Value::Null))
                )
            })
            .collect();
        err().print(&format!("uploaded attachment ids: {}", ids.join(", ")));
    }
    for s in &e.steps {
        err().print(&format!("  {}", fsv_py::py_repr(s)));
    }
    flow_exit(5, format!("partial failure; {what} id {id}"))
}

/// `change_cli._report_created`.
fn report_created(result: &Value, as_json: bool) -> CmdResult {
    let get = |k: &str| result.get(k).cloned().unwrap_or(Value::Null);
    let id = get("id");
    let steps = get("steps").as_array().cloned().unwrap_or_default();
    if as_json {
        let mut o = Map::new();
        o.insert("created".into(), get("result"));
        o.insert("plan_hash".into(), get("plan_hash"));
        o.insert("steps".into(), Value::Array(steps));
        o.insert("uploaded".into(), get("uploaded"));
        println!("{}", formflow::pretty(&Value::Object(o)));
        return Ok(());
    }
    let r = get("result");
    let shown = if r.is_object() { r } else { json!({"id": id}) };
    out().print(&format!(
        "[green]created[/] {}  {}/{}",
        fsv_core::resource::format_id(&shown, &CHANGES),
        CHANGES.portal_url(),
        fsv_py::py_str(&id)
    ));
    print_steps(&steps);
    Ok(())
}

/// `changes update`: mode dispatch.
pub fn update(a: ChangesUpdateArgs) -> CmdResult {
    let local_no_input = no_input(a.no_input);
    let id = cid(&a.id_, &CHANGES)?;
    let any_flag = a.status.is_some()
        || a.priority.is_some()
        || a.agent_id.is_some()
        || a.group_id.is_some()
        || a.edit
        || !a.set_.is_empty()
        || a.planning.is_some()
        || a.description.is_some()
        || !a.files.is_empty()
        || !a.attach.is_empty()
        || a.due_by.is_some()
        || a.from_.is_some()
        || a.preset.is_some();
    let open_form = !local_no_input && !any_flag && interactive() && form_tty();
    let legacy = [&a.status, &a.priority, &a.agent_id, &a.group_id]
        .iter()
        .any(|v| v.is_some());
    if (a.edit
        || !a.set_.is_empty()
        || a.from_.is_some()
        || a.preset.is_some()
        || open_form
        || legacy)
        && a.due_by.is_none()
        && a.planning.is_none()
    {
        let tty = interactive() && !local_no_input;
        if a.edit && !tty {
            return Err(flow_exit(
                2,
                "--edit needs a terminal (no TTY or --no-input); use --from FILE or --set",
            ));
        }
        let c = client()?;
        return change_update(&a, id, c, tty, open_form).map_err(fail_exc);
    }
    let quick = !a.attach.is_empty()
        || !a.set_.is_empty()
        || [&a.status, &a.priority, &a.agent_id, &a.group_id]
            .iter()
            .any(|v| v.is_some());
    let modes = [a.due_by.is_some(), a.planning.is_some(), a.edit, quick]
        .iter()
        .filter(|x| **x)
        .count();
    if modes > 1 {
        return Err(usage(
            "choose one update mode: --due-by, --planning, --edit, or field/attachment flags",
        ));
    }
    if local_no_input && a.duplicate == "prompt" && (!a.files.is_empty() || !a.attach.is_empty()) {
        return Err(usage(
            "--duplicate prompt disabled by --no-input; use --duplicate skip|replace|append",
        ));
    }
    let dup = DuplicateOpts {
        duplicate: Some(a.duplicate.clone()),
        backup_replaced: if a.backup_replaced {
            Some(true)
        } else if a.no_backup_replaced {
            Some(false)
        } else {
            None
        },
        backup_name: a.backup_name.clone(),
    };
    if let Some(due) = &a.due_by {
        if a.dry_run {
            emit_dry_run(&CHANGES, id, json!({"due_by": due}), "set_due_by", &[]);
            return Ok(());
        }
        let c = client()?;
        fsv_api::create::set_due_by(c.as_ref(), id, due).map_err(fail_exc)?;
        out().print(&format!("[green]updated[/] #{id} resolution due → {due}"));
        return Ok(());
    }
    if let Some(planning) = &a.planning {
        let c = client()?;
        let sch = crate::schema::load(&CHANGES, &c, false).map_err(fail_exc)?;
        let fields = crate::schema::fields(&sch);
        let def =
            fsv_api::create::resolve_planning_definition(planning, &fields).map_err(fail_exc)?;
        let planning_id = fsv_api::create::planning_server_key(&def);
        let desc = if a.description.as_deref() == Some("-") {
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
            Some(buf)
        } else {
            a.description.clone()
        };
        if a.dry_run {
            emit_dry_run(
                &CHANGES,
                id,
                json!({"planning_field": planning_id, "description": desc, "files": a.files, "duplicate": a.duplicate}),
                "update_planning_field",
                &[],
            );
            return Ok(());
        }
        let label = def
            .get("label")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_else(|| planning.clone());
        let updated = fsv_api::create::update_planning_field(
            c.as_ref(),
            id,
            &planning_id,
            desc.as_deref(),
            &a.files,
            &dup,
            Some(&label),
            &StdinPrompter,
        )
        .map_err(fail_exc)?;
        let key = fsv_py::py_repr_str(&planning_id);
        if a.json_out {
            emit_json(&updated);
        } else if updated.get("_fsv_noop").is_some_and(fsv_py::truthy) {
            let skipped = joined(updated.get("skipped"));
            let tail = if skipped.is_empty() {
                String::new()
            } else {
                format!(" ({skipped} already attached)")
            };
            out().print(&format!("[yellow]no changes[/] planning field {key}{tail}"));
        } else {
            let mut suffix = Vec::new();
            if desc.is_some() {
                suffix.push("description".to_string());
            }
            if !a.files.is_empty() {
                suffix.push(format!("{} file(s)", a.files.len()));
            }
            let tail = if suffix.is_empty() {
                String::new()
            } else {
                format!(" ({})", suffix.join(", "))
            };
            out().print(&format!("[green]updated[/] planning field {key}{tail}"));
        }
        return Ok(());
    }
    if quick {
        // Field flags and --set take the form path above; only --attach lands here.
        let c = client()?;
        crate::schema::load(&CHANGES, &c, false).map_err(fail_exc)?;
        let subject = format_cid(id, &CHANGES);
        if a.dry_run {
            let body = json!({"attachments": {"preserve_existing": true, "upload_files": a.attach, "duplicate": a.duplicate}});
            emit_dry_run(&CHANGES, id, body, "update", &[]);
            return Ok(());
        }
        let result = fsv_api::create::attach_files_to_change(
            c.as_ref(),
            id,
            &a.attach,
            &dup,
            &StdinPrompter,
        )
        .map_err(fail_exc)?;
        if result.get("_fsv_noop").is_some_and(fsv_py::truthy) {
            let skipped = joined(result.get("skipped"));
            let tail = if skipped.is_empty() {
                String::new()
            } else {
                format!(" ({skipped} already attached)")
            };
            out().print(&format!("[yellow]no changes[/] {subject}{tail}"));
            return Ok(());
        }
        if fsv_py::truthy(&result) {
            if a.json_out {
                emit_json(&result);
            } else {
                out().print(&format!("[green]updated[/] {subject}"));
            }
            return Ok(());
        }
    }
    Err(usage(
        "nothing to update; use --set, --from, --attach, --planning, --edit, or pass --status/--priority/--agent/--group (in a terminal, no flags opens the edit form)",
    ))
}

/// `", ".join(v or [])`.
fn joined(v: Option<&Value>) -> String {
    v.and_then(Value::as_array)
        .map(|a| a.iter().map(fsv_py::py_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

/// `changes download`.
pub fn download(a: ChangesDownloadArgs) -> CmdResult {
    crate::cmd::download::changes(a)
}

fn assets_legacy(a: ChangesAssetsLegacyArgs) -> CmdResult {
    links::assets(AssetOpts {
        search: a.search,
        add: a.add,
        remove: a.remove,
        page: a.page,
        per_page: a.per_page,
        dry_run: a.dry_run,
        yes: a.yes,
        json_out: a.json_out,
        fmt: a.format_,
        pick: a.pick,
        category: a.category,
        list_categories: a.list_categories,
        ..AssetOpts::new(&a.id_)
    })
}
