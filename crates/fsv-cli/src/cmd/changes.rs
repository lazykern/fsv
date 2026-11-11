//! `changes`-only commands.

use std::io::IsTerminal;

use clap::ArgMatches;
use serde_json::json;

use fsv_core::Error;
use fsv_core::output::emit_json;
use fsv_core::resource::CHANGES;
use fsv_core::term::{self, out};

use crate::cmd::parse;
use crate::generated::*;
use crate::pending;
use crate::util::{
    CmdResult, cid, client, emit_dry_run, fail_exc, format_cid, no_input, plain_fail, stdin_tty,
    usage,
};

pub fn dispatch(sub: &str, m: &ArgMatches) -> CmdResult {
    match sub {
        "tasks-add" => tasks_add(parse(m)?),
        "approval-request" => approval_request(parse(m)?),
        "approval-respond" => approval_respond(parse(m)?),
        "approval-cancel" => approval_cancel(parse(m)?),
        "tasks-update" => tasks_update(parse(m)?),
        "tasks-delete" => tasks_delete(parse(m)?),
        "approvals" => approvals(parse(m)?),
        "state" => state(parse(m)?),
        "clone" => clone(parse(m)?),
        "create" => create(parse(m)?),
        "assets legacy" => assets_legacy(parse(m)?),
        "assets ls" => assets_read(
            &parse::<ChangesAssetsLsArgs>(m)?.id_,
            "assets_resource (ls)",
        ),
        "assets search" => assets_read(
            &parse::<ChangesAssetsSearchArgs>(m)?.id_,
            "assets_resource (search)",
        ),
        "assets add" => assets_read(
            &parse::<ChangesAssetsAddArgs>(m)?.id_,
            "assets_resource (add)",
        ),
        "assets remove" => assets_read(
            &parse::<ChangesAssetsRemoveArgs>(m)?.id_,
            "assets_resource (remove)",
        ),
        "assets pick" => assets_pick(parse(m)?),
        "assets categories" => {
            let _c = client()?;
            pending::change_links("asset_categories_resource").map_err(fail_exc)
        }
        "associations legacy" => associations_legacy(parse(m)?),
        "associations ls" => associations(
            &parse::<ChangesAssociationsLsArgs>(m)?.id_,
            false,
            0,
            false,
            "ls",
        ),
        "associations search" => associations(
            &parse::<ChangesAssociationsSearchArgs>(m)?.id_,
            false,
            0,
            false,
            "search",
        ),
        "associations add" => {
            let a: ChangesAssociationsAddArgs = parse(m)?;
            associations(
                &a.id_,
                a.read_only && !a.dry_run,
                a.tickets_.len(),
                false,
                "add",
            )
        }
        "associations remove" => {
            let a: ChangesAssociationsRemoveArgs = parse(m)?;
            associations(
                &a.id_,
                a.read_only && !a.dry_run,
                a.tickets_.len(),
                true,
                "remove",
            )
        }
        "associations pick" => {
            let a: ChangesAssociationsPickArgs = parse(m)?;
            associations(&a.id_, a.read_only && !a.dry_run, 0, false, "pick")
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

fn flow_exit(code: i32, msg: &str) -> Error {
    Error::exit(code, msg)
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
            &format!(
                "attach flow {}: expected one of after-create, before-create",
                fsv_py::py_repr_str(&value)
            ),
        ));
    }
    Ok(value)
}

fn tasks_add(a: ChangesTasksAddArgs) -> CmdResult {
    if a.read_only && !a.dry_run {
        return Err(plain_fail("read-only mode: no task was sent", 2));
    }
    pending::change_review_plan("change_tasks.build_plan")
}

fn approval_request(a: ChangesApprovalRequestArgs) -> CmdResult {
    if a.read_only && !a.dry_run {
        return Err(plain_fail(
            "read-only mode: no approval request was sent",
            2,
        ));
    }
    pending::change_review_plan("change_approvals.build_plan")
}

fn approval_respond(a: ChangesApprovalRespondArgs) -> CmdResult {
    if a.approve == a.reject {
        return Err(plain_fail("choose exactly one of --approve or --reject", 2));
    }
    if a.read_only && !a.dry_run {
        return Err(plain_fail("read-only mode: no approval action was sent", 2));
    }
    pending::change_review_plan("change_approval_actions.build_plan")
}

fn approval_cancel(a: ChangesApprovalCancelArgs) -> CmdResult {
    if a.read_only && !a.dry_run {
        return Err(plain_fail("read-only mode: no approval action was sent", 2));
    }
    pending::change_review_plan("change_approval_actions.build_plan")
}

fn tasks_update(a: ChangesTasksUpdateArgs) -> CmdResult {
    let id = cid(&a.id_, &CHANGES)?;
    let any_field = [
        &a.status,
        &a.title,
        &a.group_id,
        &a.agent_id,
        &a.system,
        &a.environment,
        &a.due_date,
        &a.planned_start,
        &a.planned_end,
    ]
    .iter()
    .any(|v| v.is_some());
    if !a.edit && !any_field {
        return Err(usage("nothing to update; use --edit or pass task fields"));
    }
    let _ = id;
    pending::change_helper("create.update_task").map_err(fail_exc)
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
    let c = fsv_core::get_client().map_err(fail_exc)?;
    pending::delete_task(&c, id, a.task_id).map_err(fail_exc)?;
    out().print(&format!("[green]deleted[/] task {} from #{id}", a.task_id));
    Ok(())
}

fn approvals(a: ChangesApprovalsArgs) -> CmdResult {
    let _id = cid(&a.id_, &CHANGES)?;
    let _c = client()?;
    pending::change_helper(if a.groups {
        "create.get_change_approval_groups"
    } else {
        "create.get_change_approvals"
    })
    .map_err(fail_exc)
}

fn state(a: ChangesStateArgs) -> CmdResult {
    let _id = cid(&a.id_, &CHANGES)?;
    let _c = client()?;
    pending::change_helper("state_flow.get_state_flow").map_err(fail_exc)
}

fn clone(a: ChangesCloneArgs) -> CmdResult {
    let _id = cid(&a.id_, &CHANGES)?;
    pending::change_helper("create.change_clone_data").map_err(fail_exc)
}

fn create(a: ChangesCreateArgs) -> CmdResult {
    let tty = interactive() && !no_input(a.no_input);
    let _attach_flow = resolve_attach_flow(a.attach_flow.as_deref())?;
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
    if !has_input && !edit {
        if tty && form_tty() || a.dry {
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
    let _c = client()?;
    pending::load_meta("change", "new")?;
    pending::change_form_flow("change_cli.create_command")
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
    if (a.edit || !a.set_.is_empty() || a.from_.is_some() || a.preset.is_some() || open_form)
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
        let _c = client()?;
        return pending::change_form_flow("change_cli.update_command");
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
    if let Some(due) = &a.due_by {
        if a.dry_run {
            emit_dry_run(&CHANGES, id, json!({"due_by": due}), "set_due_by", &[]);
            return Ok(());
        }
        return pending::change_helper("create.set_due_by").map_err(fail_exc);
    }
    if a.planning.is_some() {
        let _c = client()?;
        return pending::change_helper("create.update_planning_field").map_err(fail_exc);
    }
    if quick {
        let _c = client()?;
        return pending::change_helper("create.update_change").map_err(fail_exc);
    }
    Err(usage(
        "nothing to update; use --set, --from, --attach, --planning, --edit, or pass --status/--priority/--agent/--group (in a terminal, no flags opens the edit form)",
    ))
}

/// `changes download`.
pub fn download(a: ChangesDownloadArgs) -> CmdResult {
    let id = cid(&a.id_, &CHANGES)?;
    let c = client()?;
    pending::download(&CHANGES, &c, id).map_err(fail_exc)
}

fn assets_legacy(a: ChangesAssetsLegacyArgs) -> CmdResult {
    let actions = [
        a.search.is_some(),
        !a.add.is_empty(),
        !a.remove.is_empty(),
        a.pick,
        a.list_categories,
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if actions > 1 {
        return Err(usage(
            "choose only one action: --search, --add, --remove, --pick, or --list-categories",
        ));
    }
    let _c = client()?;
    if !a.list_categories {
        cid(&a.id_, &CHANGES)?;
    }
    pending::change_links("assets_resource").map_err(fail_exc)
}

fn assets_read(id_: &str, what: &str) -> CmdResult {
    let _c = client()?;
    cid(id_, &CHANGES)?;
    pending::change_links(what).map_err(fail_exc)
}

fn assets_pick(a: ChangesAssetsPickArgs) -> CmdResult {
    let _c = client()?;
    cid(&a.id_, &CHANGES)?;
    if !stdin_tty() || no_input(false) {
        return Err(usage("interactive picker requires TTY"));
    }
    pending::change_links("assets_resource (pick)").map_err(fail_exc)
}

fn associations_legacy(a: ChangesAssociationsLegacyArgs) -> CmdResult {
    let actions = [
        a.search.is_some(),
        !a.add.is_empty(),
        !a.remove.is_empty(),
        a.pick,
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if actions > 1 {
        return Err(usage(
            "choose only one action: --search, --add, or --remove",
        ));
    }
    let write = a.pick || !a.add.is_empty() || !a.remove.is_empty();
    associations(
        &a.id_,
        write && a.read_only && !a.dry_run,
        a.remove.len(),
        !a.remove.is_empty(),
        "legacy",
    )
}

/// `change_associations_resource` checks that need no server.
fn associations(
    id_: &str,
    read_only_write: bool,
    n_remove: usize,
    removing: bool,
    what: &str,
) -> CmdResult {
    cid(id_, &CHANGES)?;
    if read_only_write {
        return Err(usage("read-only mode: no association request was sent"));
    }
    if removing && n_remove != 1 {
        return Err(usage(
            "unlink accepts one ticket per reviewed request; review each removal separately",
        ));
    }
    pending::change_links(&format!("change_associations_resource ({what})")).map_err(fail_exc)
}
