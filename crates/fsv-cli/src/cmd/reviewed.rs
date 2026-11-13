//! Single-attempt reviewed commands with JSON-only output: watchers
//! (`watcher_cli`), ticket time (`ticket_time_cli`), planning text and the
//! known-error mark (`planning_cli`), bulk priority (`bulk_priority_cli`),
//! ticket lifecycle, merge, assignment, note edits and tasks
//! (`ticket_*_cli`), problem incidents and status (`problem_cli`), change
//! and problem time (`module_time_cli`), problem task writes
//! (`problem_task_update_cli`) and SR requested items
//! (`sr_requested_items_cli`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_api::formmeta::{ClientResolver, DiskStore};
use fsv_api::service::{Api, DiskJournal};
use fsv_api::{
    asset_picker, bulk_priority, change_relationships, module_time, planning_text,
    problem_incidents, problem_known_error, problem_status, problem_task_update,
    sr_requested_items, ticket_assignment, ticket_bulk, ticket_lifecycle, ticket_merge,
    ticket_note_mutation, ticket_tasks, ticket_time, watchers,
};
use fsv_core::resource::{CHANGES, PROBLEMS, Resource, TICKETS, parse_id};
use fsv_core::{Error, Result};

use crate::generated::*;
use crate::util::{CmdResult, client, plain_fail, uncaught};

/// `json.dumps(value, indent=2)`.
pub(crate) fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().indent(2).dump(v)
}

/// `{"status": "dry_run", **review}`, indented.
pub(crate) fn print_dry_run(review: Map<String, Value>) {
    let mut out = Map::new();
    out.insert("status".into(), json!("dry_run"));
    out.extend(review);
    println!("{}", dumps(&Value::Object(out)));
}

/// Sending needs `--yes` and the exact hash of the current review.
pub(crate) fn require_review(yes: bool, expect: Option<&str>, hash: &str) -> Result<()> {
    if !yes || expect != Some(hash) {
        return Err(Error::exit(
            2,
            "review with --dry-run, then pass --yes and the full --expect-plan hash",
        ));
    }
    Ok(())
}

/// `except (ValueError, ReviewedWriteError) as exc`: the message on stderr,
/// exit 2. Anything else is left uncaught.
pub(crate) fn refusal(e: Error) -> Error {
    match e {
        Error::Value(m) => plain_fail(&m, 2),
        Error::Exit { code: 2, message } if !message.is_empty() => plain_fail(&message, 2),
        other => uncaught(other),
    }
}

/// `typer.BadParameter(msg)` raised from a command body: Typer's rich usage
/// error (usage line, help hint, then an "Error" panel as wide as stderr).
pub(crate) fn bad_parameter(usage: &str, msg: &str) -> Error {
    let path = fsv_core::runtime::command_path().unwrap_or_else(|| "fsv".into());
    usage_error(&path, usage, &format!("Invalid value: {msg}"));
    Error::exit(2, "")
}

/// Click's `UsageError` as Typer's rich formatter prints it on stderr.
pub(crate) fn usage_error(path: &str, usage: &str, message: &str) {
    let width = fsv_core::term::err().width().max(12);
    let inner = width - 4;
    let mut lines: Vec<String> = Vec::new();
    for word in message.split(' ') {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= inner => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    eprintln!("Usage: {path} {usage}");
    eprintln!("Try '{path} -h' for help.");
    eprintln!("╭─ Error {}╮", "─".repeat(width - 10));
    for line in lines {
        let pad = inner.saturating_sub(line.chars().count());
        eprintln!("│ {line}{} │", " ".repeat(pad));
    }
    eprintln!("╰{}╯", "─".repeat(width - 2));
}

pub fn watchers_list(res: &'static Resource, a: ReviewedListArgs) -> CmdResult {
    let c = client()?;
    let ids = parse_id(&a.id_, res)
        .and_then(|cid| watchers::positive_id(&json!(cid)))
        .and_then(|cid| watchers::inventory(c.as_ref(), res.name, cid));
    match ids {
        Ok(ids) => {
            println!(
                "{}",
                fsv_py::Dumps::default().dump(&json!({"watcher_ids": ids}))
            );
            Ok(())
        }
        Err(Error::Value(m)) => Err(bad_parameter("[OPTIONS] ID", &m)),
        Err(Error::Exit { code: 2, message }) if !message.is_empty() => {
            Err(bad_parameter("[OPTIONS] ID", &message))
        }
        Err(e) => Err(uncaught(e)),
    }
}

pub fn watchers_set(res: &'static Resource, a: WatchersSetArgs) -> CmdResult {
    let c = client()?;
    let run = || -> Result<()> {
        let plan = watchers::build_plan(c.as_ref(), res.name, &a.id_, a.watch)?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = watchers::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn time_list(a: ReviewedListArgs) -> CmdResult {
    let c = client()?;
    let rows = parse_id(&a.id_, &TICKETS)
        .and_then(|cid| ticket_time::inventory(c.as_ref(), cid))
        .map_err(uncaught)?;
    println!("{}", dumps(&json!(rows)));
    Ok(())
}

pub fn time_write(a: TicketsTimeWriteArgs) -> CmdResult {
    let c = client()?;
    let run = || -> Result<()> {
        let args = ticket_time::TimeArgs {
            action: &a.action,
            entry_id: a.entry,
            spent: a.spent.as_deref(),
            executed_at: a.executed_at.as_deref(),
            note: a.note.as_deref(),
        };
        let plan = ticket_time::build_plan(c.as_ref(), &a.id_, &args)?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = ticket_time::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn planning_text(res: &'static Resource, a: PlanningTextArgs) -> CmdResult {
    let schema = &planning_text::live_change_schema;
    let run = || -> Result<()> {
        let c = client()?;
        let plan = planning_text::build_plan_for(
            c.as_ref(),
            res.name,
            &a.id_,
            &a.section,
            a.description.as_deref(),
            a.clear,
            schema,
        )?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = planning_text::send(&plan, c.as_ref(), &DiskJournal, Some(&hash), schema)?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(|e| match e {
        Error::Api(_) | Error::Session(_) => plain_fail(&e.to_string(), e.exit_code()),
        other => refusal(other),
    })
}

/// Port of fsv.asset_picker_cli.register's `asset-picker`: one read-only search page.
pub fn asset_picker(a: ChangesAssetPickerArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let result = asset_picker::candidates(c.as_ref(), &json!(a.target), &a.search, a.page)?;
        let mut out = result.to_json();
        if let Some(n) = a.select {
            let row = asset_picker::select(&result, &json!(n))?;
            out["selected"] = Value::Object(row.clone());
        }
        println!("{}", dumps(&out));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn mark_known_error(a: ProblemsMarkKnownErrorArgs) -> CmdResult {
    let c = client()?;
    let run = || -> Result<()> {
        let plan = problem_known_error::build_plan(c.as_ref(), &a.id_)?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = problem_known_error::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn bulk_priority(a: TicketsBulkPriorityArgs) -> CmdResult {
    let c = client()?;
    let store = DiskStore::current();
    let api: Arc<dyn Api + Send + Sync> = c.clone();
    let resolver = || -> Option<Arc<dyn fsv_forms::LookupResolver>> {
        Some(Arc::new(ClientResolver::new(api.clone())))
    };
    let run = || -> Result<bool> {
        let plan = bulk_priority::build_plan(c.as_ref(), &store, &a.ids, &a.priority, resolver())?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(true);
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = bulk_priority::send(
            &plan,
            c.as_ref(),
            &DiskJournal,
            &store,
            Some(&hash),
            resolver(),
        )?;
        println!("{}", dumps(&result));
        Ok(result.get("verified").is_some_and(fsv_py::truthy))
    };
    match run().map_err(refusal)? {
        true => Ok(()),
        false => Err(Error::exit(1, "")),
    }
}

fn lookup_resolver(c: &Arc<fsv_core::Client>) -> Option<Arc<dyn fsv_forms::LookupResolver>> {
    let api: Arc<dyn Api + Send + Sync> = c.clone();
    Some(Arc::new(ClientResolver::new(api)))
}

/// `--set FIELD=VALUE` answers; each field once.
fn set_answers(sets: &[String]) -> Result<Map<String, Value>> {
    let mut answers = Map::new();
    for expression in sets {
        match expression.split_once('=') {
            Some((key, value)) if !key.is_empty() && !answers.contains_key(key) => {
                answers.insert(key.into(), json!(value));
            }
            _ => return Err(Error::value("--set requires unique FIELD=VALUE answers")),
        }
    }
    Ok(answers)
}

/// Port of fsv.ticket_lifecycle_cli's shared `run`.
#[allow(clippy::too_many_arguments)]
fn lifecycle(
    target: &str,
    status: &str,
    note: Option<&str>,
    sets: &[String],
    dry_run: bool,
    yes: bool,
    expect_plan: Option<&str>,
    ack: bool,
) -> CmdResult {
    let run = || -> Result<()> {
        let answers = set_answers(sets)?;
        let c = client()?;
        let store = DiskStore::current();
        let plan = ticket_lifecycle::build_plan_ack(
            c.as_ref(),
            &store,
            target,
            status,
            note,
            Some(&answers),
            lookup_resolver(&c),
            ack,
        )?;
        if dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(yes, expect_plan, &hash)?;
        let result = ticket_lifecycle::send(
            &plan,
            c.as_ref(),
            &store,
            &DiskJournal,
            Some(&hash),
            lookup_resolver(&c),
        )?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn resolve(a: TicketsResolveArgs) -> CmdResult {
    let note = Some(a.note.as_str());
    lifecycle(
        &a.id_,
        "Resolved",
        note,
        &a.sets,
        a.dry_run,
        a.yes,
        a.expect_plan.as_deref(),
        a.ack,
    )
}

pub fn close(a: TicketsCloseArgs) -> CmdResult {
    lifecycle(
        &a.id_,
        "Closed",
        None,
        &a.sets,
        a.dry_run,
        a.yes,
        a.expect_plan.as_deref(),
        a.ack,
    )
}

pub fn reopen(a: TicketsReopenArgs) -> CmdResult {
    lifecycle(
        &a.id_,
        "Open",
        None,
        &[],
        a.dry_run,
        a.yes,
        a.expect_plan.as_deref(),
        a.ack,
    )
}

pub fn status(a: TicketsStatusArgs) -> CmdResult {
    lifecycle(
        &a.id_,
        &a.status_,
        None,
        &a.sets,
        a.dry_run,
        a.yes,
        a.expect_plan.as_deref(),
        a.ack,
    )
}

/// Port of fsv.ticket_merge_cli's `merge`.
pub fn merge(a: TicketsMergeArgs) -> CmdResult {
    let dumps = |v: &Value| {
        fsv_py::Dumps::default()
            .indent(2)
            .ensure_ascii(false)
            .dump(v)
    };
    let run = || -> Result<()> {
        let c = client()?;
        let store = DiskStore::current();
        let plan = ticket_merge::build_plan(
            c.as_ref(),
            &store,
            &a.primary,
            &a.secondary,
            &a.note_primary,
            &a.note_secondary,
            !a.public_primary,
            !a.public_secondary,
        )?;
        if a.dry_run {
            let mut out = Map::new();
            out.insert("status".into(), json!("dry_run"));
            out.extend(ticket_merge::review(&plan));
            println!("{}", dumps(&Value::Object(out)));
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = ticket_merge::send(&plan, c.as_ref(), &store, &DiskJournal, Some(&hash))?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

/// Port of fsv.ticket_assignment_cli's shared `run`.
fn assignment(
    ids: &[String],
    args: &ticket_assignment::AssignmentArgs<'_>,
    dry_run: bool,
    yes: bool,
    expect_plan: Option<&str>,
) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let store = DiskStore::current();
        let plan =
            ticket_assignment::build_plan(c.as_ref(), &store, ids, args, lookup_resolver(&c))?;
        if dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(yes, expect_plan, &hash)?;
        let result = ticket_assignment::send(
            &plan,
            c.as_ref(),
            &store,
            &DiskJournal,
            Some(&hash),
            lookup_resolver(&c),
        )?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn assign(a: TicketsAssignArgs) -> CmdResult {
    let args = ticket_assignment::AssignmentArgs {
        agent: a.agent.as_deref(),
        group: a.group.as_deref(),
        unassign: a.unassign,
        pickup: false,
    };
    let ids = [a.id_.clone()];
    assignment(&ids, &args, a.dry_run, a.yes, a.expect_plan.as_deref())
}

pub fn pick_up(a: TicketsPickUpArgs) -> CmdResult {
    let args = ticket_assignment::AssignmentArgs {
        pickup: true,
        ..Default::default()
    };
    assignment(&a.ids, &args, a.dry_run, a.yes, a.expect_plan.as_deref())
}

/// `json.dumps(value, indent=2, ensure_ascii=False)`.
fn dumps_utf8(v: &Value) -> String {
    fsv_py::Dumps::default()
        .indent(2)
        .ensure_ascii(false)
        .dump(v)
}

/// [`refusal`] that also catches `APIError` (its text, exit 2).
fn api_refusal(e: Error) -> Error {
    match e {
        Error::Api(_) => plain_fail(&e.to_string(), 2),
        other => refusal(other),
    }
}

/// `{"status": "dry_run", **review, **extra}`.
fn dry_run_value(review: Map<String, Value>, extra: Map<String, Value>) -> Value {
    let mut out = Map::new();
    out.insert("status".into(), json!("dry_run"));
    out.extend(review);
    out.extend(extra);
    Value::Object(out)
}

/// Prints the send result; exit 4 when it is not verified.
pub(crate) fn print_verified(result: &Value) -> Result<()> {
    println!("{}", dumps(result));
    if result.get("verified").is_some_and(fsv_py::truthy) {
        Ok(())
    } else {
        Err(Error::exit(4, ""))
    }
}

/// Port of fsv.ticket_note_mutation_cli's shared `run`.
fn note_mutation(
    ticket: &str,
    note: &str,
    body: Option<&str>,
    dry_run: bool,
    yes: bool,
    expect_plan: Option<&str>,
) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let plan = ticket_note_mutation::build_plan(c.as_ref(), ticket, note, body)?;
        if dry_run {
            let review = ticket_note_mutation::review(&plan, c.as_ref())?;
            println!("{}", dumps_utf8(&dry_run_value(review, Map::new())));
            return Ok(());
        }
        let hash = plan.hash();
        require_review(yes, expect_plan, &hash)?;
        let result = ticket_note_mutation::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        println!("{}", dumps_utf8(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn note_edit(a: TicketsNoteEditArgs) -> CmdResult {
    let body = Some(a.body.as_str());
    note_mutation(
        &a.id_,
        &a.conversation_id,
        body,
        a.dry_run,
        a.yes,
        a.expect_plan.as_deref(),
    )
}

pub fn note_delete(a: TicketsNoteDeleteArgs) -> CmdResult {
    note_mutation(
        &a.id_,
        &a.conversation_id,
        None,
        a.dry_run,
        a.yes,
        a.expect_plan.as_deref(),
    )
}

/// Port of fsv.problem_cli's `incident_action`.
fn incident_action(
    problem: &str,
    incident: &str,
    unlink: bool,
    dry_run: bool,
    yes: bool,
    expect_plan: Option<&str>,
) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let plan = problem_incidents::build_plan(c.as_ref(), problem, incident, unlink)?;
        if dry_run {
            let mut extra = Map::new();
            extra.insert("problem_id".into(), json!(plan.target));
            extra.insert("incident_id".into(), json!(incident));
            println!("{}", dumps(&dry_run_value(plan.review(), extra)));
            return Ok(());
        }
        let hash = plan.hash();
        require_review(yes, expect_plan, &hash)?;
        let result =
            problem_incidents::send(&plan, c.as_ref(), &DiskJournal, incident, Some(&hash))?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(api_refusal)
}

pub fn link_incident(a: ProblemsLinkIncidentArgs) -> CmdResult {
    let expect = a.expect_plan.as_deref();
    incident_action(&a.problem, &a.incident, false, a.dry_run, a.yes, expect)
}

pub fn unlink_incident(a: ProblemsUnlinkIncidentArgs) -> CmdResult {
    let expect = a.expect_plan.as_deref();
    incident_action(&a.problem, &a.incident, true, a.dry_run, a.yes, expect)
}

/// Sending tasks, problem status and attachments needs `--yes` and the
/// exact hash of the current review.
pub(crate) fn require_full_hash(yes: bool, expect: Option<&str>, hash: &str) -> Result<()> {
    if !yes || expect != Some(hash) {
        return Err(Error::exit(
            2,
            "review with --dry-run, then --yes --expect-plan FULL_HASH",
        ));
    }
    Ok(())
}

/// Port of fsv.problem_cli's `status`.
pub fn problem_status(a: ProblemsStatusArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let store = DiskStore::current();
        let plan =
            problem_status::build_plan(c.as_ref(), &store, &a.target, &a.to, lookup_resolver(&c))?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_full_hash(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = problem_status::send(
            &plan,
            c.as_ref(),
            &store,
            &DiskJournal,
            Some(&hash),
            lookup_resolver(&c),
        )?;
        print_verified(&result)
    };
    run().map_err(api_refusal)
}

/// Port of fsv.ticket_tasks_cli's `list`.
pub fn tasks_list(a: TicketsTasksListArgs) -> CmdResult {
    let c = client()?;
    let rows = parse_id(&a.target, &TICKETS)
        .and_then(|cid| ticket_tasks::inventory(c.as_ref(), cid))
        .map_err(uncaught)?;
    println!("{}", dumps(&json!(rows)));
    Ok(())
}

/// Arguments of fsv.ticket_tasks_cli's shared `run`.
struct TaskWrite<'a> {
    target: &'a str,
    action: &'a str,
    task: Option<&'a str>,
    title: Option<&'a str>,
    status: Option<&'a str>,
    sets: &'a [String],
    owner: Option<&'a str>,
    group: Option<&'a str>,
    dry_run: bool,
    yes: bool,
    expect_plan: Option<&'a str>,
}

/// Port of fsv.ticket_tasks_cli's shared `run`.
fn task_write(w: TaskWrite<'_>) -> CmdResult {
    let run = || -> Result<()> {
        let mut values = Map::new();
        for item in w.sets {
            match item.split_once('=') {
                Some((key, value)) if !key.is_empty() && !values.contains_key(key) => {
                    values.insert(key.into(), json!(value));
                }
                _ => return Err(Error::value("--set requires unique FIELD=VALUE")),
            }
        }
        let c = client()?;
        let args = ticket_tasks::TaskArgs {
            action: w.action.into(),
            task: w.task.map_or(Value::Null, |t| json!(t)),
            title: w.title.map(str::to_string),
            status: w.status.unwrap_or_default().into(),
            values: Some(values),
        };
        let plan = ticket_tasks::build_plan_assigned(
            c.as_ref(),
            w.target,
            &args,
            w.owner,
            w.group,
            w.status,
        )?;
        if w.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_full_hash(w.yes, w.expect_plan, &hash)?;
        let result = ticket_tasks::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        print_verified(&result)
    };
    run().map_err(refusal)
}

pub fn tasks_add(a: TicketsTasksAddArgs) -> CmdResult {
    task_write(TaskWrite {
        target: &a.target,
        action: "add",
        task: None,
        title: Some(&a.title),
        status: Some(&a.status),
        sets: &a.set_,
        owner: a.owner.as_deref(),
        group: a.group.as_deref(),
        dry_run: a.dry_run,
        yes: a.yes,
        expect_plan: a.expect_plan.as_deref(),
    })
}

pub fn tasks_update(a: TicketsTasksUpdateArgs) -> CmdResult {
    task_write(TaskWrite {
        target: &a.target,
        action: "update",
        task: Some(&a.task),
        title: a.title.as_deref(),
        status: None,
        sets: &[],
        owner: a.owner.as_deref(),
        group: a.group.as_deref(),
        dry_run: a.dry_run,
        yes: a.yes,
        expect_plan: a.expect_plan.as_deref(),
    })
}

pub fn tasks_status(a: TicketsTasksStatusArgs) -> CmdResult {
    task_write(TaskWrite {
        target: &a.target,
        action: "status",
        task: Some(&a.task),
        title: None,
        status: Some(&a.to),
        sets: &[],
        owner: None,
        group: None,
        dry_run: a.dry_run,
        yes: a.yes,
        expect_plan: a.expect_plan.as_deref(),
    })
}

pub fn tasks_delete(a: TicketsTasksDeleteArgs) -> CmdResult {
    task_write(TaskWrite {
        target: &a.target,
        action: "delete",
        task: Some(&a.task),
        title: None,
        status: None,
        sets: &[],
        owner: None,
        group: None,
        dry_run: a.dry_run,
        yes: a.yes,
        expect_plan: a.expect_plan.as_deref(),
    })
}

/// Port of fsv.ticket_bulk_cli's `bulk`.
pub fn tickets_bulk(a: TicketsBulkArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let store = DiskStore::current();
        let args = ticket_bulk::BulkArgs {
            action: &a.action,
            where_: &a.r#where,
            select_all: a.select_all,
            status: a.status.as_deref(),
            agent: a.agent.as_deref(),
            group: a.group.as_deref(),
            primary: a.primary.as_deref(),
            note_primary: a.note_primary.as_deref(),
            note_secondary: a.note_secondary.as_deref(),
        };
        let mut answers = Map::new();
        for expression in &a.set_ {
            let (key, value) = match expression.split_once('=') {
                Some((key, value)) => (key, value),
                None => {
                    return Err(Error::value("--set requires unique FIELD=VALUE answers"));
                }
            };
            let key = fsv_core::pystr::strip(key).to_string();
            if key.is_empty() || answers.contains_key(&key) {
                return Err(Error::value("--set requires unique FIELD=VALUE answers"));
            }
            answers.insert(key, json!(value));
        }
        let plan = ticket_bulk::build_plan_with_answers(
            c.as_ref(),
            &store,
            &a.ids,
            &args,
            a.max_records,
            (!answers.is_empty()).then_some(&answers),
        )?;
        let ids: Vec<String> = plan.ids.iter().map(i64::to_string).collect();
        eprintln!("Selected ticket IDs: {}", ids.join(", "));
        let selected: Vec<Value> = serde_json::from_str(&plan.selected_json)?;
        for row in &selected {
            eprintln!(
                "  {}: {}",
                fsv_py::py_str(&row["id"]),
                fsv_py::py_str(&row["subject"])
            );
        }
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_full_hash(a.yes, a.expect_plan.as_deref(), &hash)?;
        let mut result = ticket_bulk::send(
            &plan,
            c.as_ref(),
            &store,
            &DiskJournal,
            Some(&hash),
            a.confirm_destructive,
        )?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "selected_tickets".into(),
                serde_json::from_str(&plan.selected_json).unwrap_or(Value::Null),
            );
        }
        print_verified(&result)
    };
    run().map_err(refusal)
}

/// Port of fsv.change_relationships_cli's `list`.
pub fn relationships_list(a: ChangesRelationshipsListArgs) -> CmdResult {
    let c = client()?;
    let rows = parse_id(&a.target, &CHANGES)
        .and_then(|cid| change_relationships::inventory(c.as_ref(), cid, &a.kind))
        .map_err(refusal)?;
    println!("{}", dumps(&json!(rows)));
    Ok(())
}

/// Port of fsv.change_relationships_cli's `write`.
pub fn relationships_write(a: ChangesRelationshipsWriteArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let project: Option<Value> = match a.project.as_deref() {
            Some(text) if !text.is_empty() => Some(serde_json::from_str(text)?),
            _ => None,
        };
        let args = change_relationships::RelArgs {
            kind: &a.kind,
            action: &a.action,
            item: a.item.as_deref(),
            project: project.as_ref().and_then(Value::as_object),
            project_key: a.project_key.as_deref(),
            name: a.name.as_deref(),
        };
        let plan = change_relationships::build_plan(c.as_ref(), &a.target, &args)?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_full_hash(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = change_relationships::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        print_verified(&result)
    };
    run().map_err(refusal)
}

/// Port of fsv.module_time_cli's `list` (changes and problems).
pub fn module_time_list(module: &str, id_: &str) -> CmdResult {
    let c = client()?;
    let res = if module == "changes" {
        &CHANGES
    } else {
        &PROBLEMS
    };
    let rows = parse_id(id_, res)
        .and_then(|target| module_time::inventory(c.as_ref(), module, target))
        .map_err(uncaught)?;
    println!("{}", dumps(&json!(rows)));
    Ok(())
}

/// Arguments of fsv.module_time_cli's `write`.
pub struct ModuleTimeWrite<'a> {
    pub module: &'a str,
    pub id_: &'a str,
    pub action: &'a str,
    pub entry: Option<i64>,
    pub spent: Option<&'a str>,
    pub executed_at: Option<&'a str>,
    pub note: Option<&'a str>,
    pub billable: bool,
    pub non_billable: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub expect_plan: Option<&'a str>,
}

/// Port of fsv.module_time_cli's `write`.
pub fn module_time_write(w: ModuleTimeWrite<'_>) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let billable = match (w.billable, w.non_billable) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        };
        let args = module_time::TimeArgs {
            action: w.action,
            entry_id: w.entry,
            spent: w.spent,
            executed_at: w.executed_at,
            note: w.note,
            billable,
        };
        let plan = module_time::build_plan(c.as_ref(), w.module, w.id_, &args)?;
        if w.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(w.yes, w.expect_plan, &hash)?;
        let result = module_time::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn changes_time_write(a: ChangesTimeWriteArgs) -> CmdResult {
    module_time_write(ModuleTimeWrite {
        module: "changes",
        id_: &a.id_,
        action: &a.action,
        entry: a.entry,
        spent: a.spent.as_deref(),
        executed_at: a.executed_at.as_deref(),
        note: a.note.as_deref(),
        billable: a.billable,
        non_billable: a.non_billable,
        dry_run: a.dry_run,
        yes: a.yes,
        expect_plan: a.expect_plan.as_deref(),
    })
}

pub fn problems_time_write(a: ProblemsTimeWriteArgs) -> CmdResult {
    module_time_write(ModuleTimeWrite {
        module: "problems",
        id_: &a.id_,
        action: &a.action,
        entry: a.entry,
        spent: a.spent.as_deref(),
        executed_at: a.executed_at.as_deref(),
        note: a.note.as_deref(),
        billable: a.billable,
        non_billable: a.non_billable,
        dry_run: a.dry_run,
        yes: a.yes,
        expect_plan: a.expect_plan.as_deref(),
    })
}

const TASK_SIDE_EFFECTS: &str =
    "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible.";

/// Port of fsv.problem_task_update_cli's `task-write`.
pub fn problem_task_write(a: ProblemsTaskWriteArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let mut inputs = Map::new();
        for (key, value) in [
            ("title", &a.title),
            ("status", &a.status),
            ("owner", &a.owner),
            ("group", &a.group),
            ("note", &a.note),
            ("due_date", &a.due_date),
            ("planned_start", &a.planned_start),
            ("planned_end", &a.planned_end),
            ("planned_effort", &a.planned_effort),
            ("notify_before", &a.notify_before),
        ] {
            if let Some(v) = value {
                inputs.insert(key.into(), json!(v));
            }
        }
        if !matches!(a.action.as_str(), "update" | "complete" | "delete") {
            return Err(Error::value(
                "task action must be update, complete or delete",
            ));
        }
        if a.action != "update" && !inputs.is_empty() {
            return Err(Error::value("complete/delete do not accept update fields"));
        }
        if a.action == "complete" {
            inputs = Map::from_iter([("status".to_string(), json!("Completed"))]);
        }
        let (target, task) = (json!(a.target), json!(a.task));
        let delete = a.action == "delete";
        let (write, update) = if delete {
            (
                problem_task_update::delete_plan(c.as_ref(), &target, &task)?,
                None,
            )
        } else {
            let plan = problem_task_update::build_plan(c.as_ref(), &target, &task, &inputs)?;
            (plan.write.clone(), Some(plan))
        };
        if a.dry_run {
            let extra = Map::from_iter([("side_effects".to_string(), json!(TASK_SIDE_EFFECTS))]);
            println!("{}", dumps(&dry_run_value(write.review(), extra)));
            return Ok(());
        }
        let hash = write.hash();
        require_full_hash(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = match &update {
            Some(plan) => problem_task_update::send(plan, c.as_ref(), &DiskJournal, Some(&hash))?,
            None => problem_task_update::delete(&write, c.as_ref(), &DiskJournal, Some(&hash))?,
        };
        print_verified(&result)
    };
    run().map_err(refusal)
}

/// Port of fsv.sr_requested_items_cli's `list`.
pub fn sr_items_list(a: SrItemsListArgs) -> CmdResult {
    let c = client()?;
    let rows = parse_id(&a.target, &TICKETS)
        .and_then(|cid| sr_requested_items::inventory(c.as_ref(), cid))
        .map_err(refusal)?;
    println!("{}", dumps(&json!(rows)));
    Ok(())
}

/// Port of fsv.sr_requested_items_cli's `write`.
pub fn sr_items_write(a: SrItemsWriteArgs) -> CmdResult {
    let run = || -> Result<()> {
        let answers: Option<Value> = match a.answers.as_deref() {
            Some(text) if !text.is_empty() => Some(serde_json::from_str(text)?),
            _ => None,
        };
        let answers = match &answers {
            Some(Value::Object(map)) => Some(map),
            Some(_) => return Err(Error::value("answers must be a JSON object")),
            None => None,
        };
        let c = client()?;
        let store = DiskStore::current();
        let args = sr_requested_items::ItemArgs {
            action: &a.action,
            item: a.item,
            catalog_item: a.catalog_item.as_deref(),
            answers,
            stage: a.stage,
        };
        let plan = sr_requested_items::build_plan(c.as_ref(), &store, &a.target, &args)?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_full_hash(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result =
            sr_requested_items::send(&plan, c.as_ref(), &store, &DiskJournal, Some(&hash))?;
        print_verified(&result)
    };
    run().map_err(refusal)
}
