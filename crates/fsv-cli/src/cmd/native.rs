//! Native reviewed workflows with JSON-only output: canned replies and
//! scenarios (`canned_cli`), private ticket views (`private_view_cli`), list
//! columns and exports (`list_preferences_cli`), problem create and child
//! writes (`problem_cli`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_api::formmeta::{ClientResolver, DiskStore};
use fsv_api::service::{Api, DiskJournal};
use fsv_api::{
    canned, conversations, list_preferences, private_views, problem_workflows, scenarios,
};
use fsv_core::resource::Resource;
use fsv_core::{Error, Result};
use fsv_flows::drafts::resolve_draft;

use crate::cmd::reviewed::{dumps, print_dry_run, refusal, require_review};
use crate::generated::*;
use crate::util::{CmdResult, client, uncaught};

/// A read with no `try` in Python: any failure is a traceback (exit 1),
/// including fsv-api refusals, which carry exit 2.
fn show(read: impl FnOnce(&dyn Api) -> Result<Value>) -> CmdResult {
    let c = client()?;
    let v = read(c.as_ref()).map_err(|e| match e {
        Error::Exit { code, message } if code != 0 => uncaught(Error::Other(message)),
        other => uncaught(other),
    })?;
    println!("{}", dumps(&v));
    Ok(())
}

pub fn canned_folders(_: JsonOnlyArgs) -> CmdResult {
    show(canned::folders)
}

pub fn canned_list(a: TicketsCannedListArgs) -> CmdResult {
    show(|c| canned::responses(c, &json!(a.folder)))
}

pub fn canned_show(a: PositiveIdArgs) -> CmdResult {
    show(|c| canned::show(c, &json!(a.id_)))
}

pub fn canned_reply(a: TicketsCannedReplyArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let store = DiskStore::current();
        let plan = canned::reply_plan(
            c.as_ref(),
            &store,
            &a.target,
            &json!(a.response),
            &a.content_pointer,
            &a.sender,
        )?;
        if a.dry_run {
            let mut review = plan.review();
            review.insert(
                "source".into(),
                json!({"id": plan.canned_id, "hash": plan.canned_hash}),
            );
            review.insert(
                "insertion".into(),
                json!("Literal selected HTML only; no attachments, signatures or placeholder expansion."),
            );
            print_dry_run(review);
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result =
            conversations::send(&plan, c.as_ref(), &DiskJournal, &store, Some(&hash), true)?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn scenarios_list(_: JsonOnlyArgs) -> CmdResult {
    show(canned::scenarios)
}

/// `scenarios show` raises `typer.BadParameter` for a refused identity.
pub fn scenarios_show(a: PositiveIdArgs) -> CmdResult {
    let c = client()?;
    match canned::scenario(c.as_ref(), &json!(a.id_)) {
        Ok(row) => {
            println!("{}", dumps(&Value::Object(row)));
            Ok(())
        }
        Err(Error::Value(m)) => Err(crate::cmd::reviewed::bad_parameter("[OPTIONS] ID_", &m)),
        Err(Error::Exit { code: 2, message }) if !message.is_empty() => Err(
            crate::cmd::reviewed::bad_parameter("[OPTIONS] ID_", &message),
        ),
        Err(e) => Err(uncaught(e)),
    }
}

pub fn scenarios_apply(a: TicketsScenariosApplyArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let plan = scenarios::build_plan(c.as_ref(), &a.target, &json!(a.scenario_id))?;
        if a.dry_run {
            let mut review = plan.review();
            review.insert(
                "definition".into(),
                Value::Object(canned::scenario(c.as_ref(), &json!(a.scenario_id))?),
            );
            review.insert(
                "warning".into(),
                json!("Native automation may send mail or change records. No complete side-effect verification contract is captured."),
            );
            print_dry_run(review);
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = scenarios::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn private_views_show(a: PositiveIdArgs) -> CmdResult {
    show(|c| private_views::detail(c, a.id_).map(Value::Object))
}

pub fn private_views_write(a: TicketsPrivateViewsWriteArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let plan =
            private_views::build_plan(c.as_ref(), &json!(a.id_), &a.action, a.name.as_deref())?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_review(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = private_views::send(&plan, c.as_ref(), &DiskJournal, Some(&hash))?;
        println!("{}", dumps(&result));
        Ok(())
    };
    run().map_err(refusal)
}

pub fn columns_list(res: &'static Resource, _: JsonOnlyArgs) -> CmdResult {
    show(|c| {
        list_preferences::headers(c, res.name)
            .map(|rows| Value::Array(rows.into_iter().map(Value::Object).collect()))
    })
}

fn finish_list(
    c: &dyn Api,
    plan: list_preferences::ListPlan,
    dry_run: bool,
    yes: bool,
    expect: Option<&str>,
) -> Result<()> {
    if dry_run {
        print_dry_run(plan.review());
        return Ok(());
    }
    let hash = plan.hash();
    require_review(yes, expect, &hash)?;
    let result = list_preferences::send(&plan, c, &DiskJournal, Some(&hash))?;
    println!("{}", dumps(&result));
    Ok(())
}

pub fn columns_set(res: &'static Resource, a: ColumnsSetArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let plan = list_preferences::columns_plan(c.as_ref(), res.name, &a.column, &a.freeze)?;
        finish_list(c.as_ref(), plan, a.dry_run, a.yes, a.expect_plan.as_deref())
    };
    run().map_err(refusal)
}

pub fn export(res: &'static Resource, a: ResExportArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let plan = list_preferences::export_plan(c.as_ref(), res.name, &a.view, &a.field)?;
        finish_list(c.as_ref(), plan, a.dry_run, a.yes, a.expect_plan.as_deref())
    };
    run().map_err(refusal)
}

fn finish_problem(
    c: &Arc<fsv_core::Client>,
    plan: problem_workflows::ProblemPlan,
    dry_run: bool,
    yes: bool,
    expect: Option<&str>,
) -> Result<()> {
    if dry_run {
        print_dry_run(plan.review());
        return Ok(());
    }
    let hash = plan.hash();
    require_review(yes, expect, &hash)?;
    let store = DiskStore::current();
    let api: Arc<dyn Api + Send + Sync> = c.clone();
    let result = problem_workflows::send(
        &plan,
        c.as_ref(),
        &DiskJournal,
        &store,
        Some(&hash),
        Some(Arc::new(ClientResolver::new(api))),
    )?;
    println!("{}", dumps(&result));
    Ok(())
}

pub fn problems_create(a: ProblemsCreateArgs) -> CmdResult {
    let run = || -> Result<()> {
        let draft = resolve_draft(a.from_.as_deref(), None, &a.set_, None)?;
        if !draft.attachments.is_empty()
            || !draft.planning.is_empty()
            || draft.requester.as_deref().is_some_and(|r| !r.is_empty())
        {
            return Err(Error::value(
                "use answers plus --requester; files, planning and special draft keys are unsupported",
            ));
        }
        let c = client()?;
        let store = DiskStore::current();
        let api: Arc<dyn Api + Send + Sync> = c.clone();
        let answers: Map<String, Value> = draft.answers;
        let plan = problem_workflows::create_plan(
            c.as_ref(),
            &store,
            &answers,
            &a.requester,
            Some(Arc::new(ClientResolver::new(api))),
        )?;
        finish_problem(&c, plan, a.dry_run, a.yes, a.expect_plan.as_deref())
    };
    run().map_err(refusal)
}

pub fn problems_write_child(a: ProblemsWriteChildArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let plan = problem_workflows::child_plan(c.as_ref(), &a.target, &a.kind, &a.text)?;
        finish_problem(&c, plan, a.dry_run, a.yes, a.expect_plan.as_deref())
    };
    run().map_err(refusal)
}
