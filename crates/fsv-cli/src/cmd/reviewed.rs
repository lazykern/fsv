//! Single-attempt reviewed commands with JSON-only output: watchers
//! (`watcher_cli`), ticket time (`ticket_time_cli`), planning text and the
//! known-error mark (`planning_cli`), bulk priority (`bulk_priority_cli`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_api::formmeta::{ClientResolver, DiskStore};
use fsv_api::service::{Api, DiskJournal};
use fsv_api::{bulk_priority, planning_text, problem_known_error, ticket_time, watchers};
use fsv_core::resource::{Resource, TICKETS, parse_id};
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
fn bad_parameter(usage: &str, msg: &str) -> Error {
    let path = fsv_core::runtime::command_path().unwrap_or_else(|| "fsv".into());
    let width = fsv_core::term::err().width().max(12);
    let inner = width - 4;
    let mut lines: Vec<String> = Vec::new();
    for word in format!("Invalid value: {msg}").split(' ') {
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
    Error::exit(2, "")
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
    let c = client()?;
    let schema = &planning_text::live_change_schema;
    let run = || -> Result<()> {
        let plan = planning_text::build_plan(
            c.as_ref(),
            res.name,
            &a.id_,
            &a.section,
            &a.description,
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
