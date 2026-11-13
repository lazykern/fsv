//! Ticket trash list, restore and permanent deletion (`ticket_trash_cli`),
//! and single-ticket spam / unspam (`ticket_spam_cli`).

use serde_json::{Map, Value, json};

use fsv_api::reviewed_write::refused;
use fsv_api::service::DiskJournal;
use fsv_api::ticket_trash;
use fsv_core::{Error, Result};

use crate::cmd::reviewed::dumps;
use crate::generated::*;
use crate::lazy::LazyApi;
use crate::util::{CmdResult, plain_fail, uncaught};

/// `_fail`: `ValueError`, `ReviewedWriteError` and `APIError` print one
/// line and exit 2; an `APIError` reads as a failed read.
fn fail(r: Result<()>) -> CmdResult {
    r.map_err(|e| match e {
        Error::Value(m) => plain_fail(&m, 2),
        Error::Exit { code: 2, message } if !message.is_empty() => plain_fail(&message, 2),
        Error::Api(ref api) => plain_fail(
            &format!("read failed (HTTP {}); nothing was sent", api.status),
            2,
        ),
        other => uncaught(other),
    })
}

fn dry_run(review: Map<String, Value>) {
    let mut out = Map::new();
    out.insert("status".into(), json!("dry_run"));
    out.extend(review);
    println!("{}", dumps(&Value::Object(out)));
}

fn finish(result: &Value) -> Result<()> {
    println!("{}", dumps(result));
    if !result.get("verified").is_some_and(fsv_py::truthy) {
        return Err(Error::exit(4, ""));
    }
    Ok(())
}

/// `tickets trash list`: one page of the Trash view.
pub fn trash_list(a: TicketsTrashListArgs) -> CmdResult {
    fail((|| {
        let v = ticket_trash::trash(&LazyApi::default(), a.page, a.per_page)?;
        println!("{}", dumps(&v));
        Ok(())
    })())
}

/// `tickets restore TICKET`.
pub fn restore(a: TicketsRestoreArgs) -> CmdResult {
    fail((|| {
        let c = LazyApi::default();
        let plan = ticket_trash::build_restore(&c, &a.ticket)?;
        if a.dry_run {
            dry_run(plan.review());
            return Ok(());
        }
        if !a.yes || a.expect_plan.as_deref() != Some(plan.hash().as_str()) {
            return Err(refused(
                "review with --dry-run, then --yes --expect-plan FULL_HASH",
            ));
        }
        let result = ticket_trash::send_restore(&plan, &c, &DiskJournal, a.expect_plan.as_deref())?;
        finish(&result)
    })())
}

/// `tickets delete-forever TICKET...`.
pub fn delete_forever(a: TicketsDeleteForeverArgs) -> CmdResult {
    fail((|| {
        let c = LazyApi::default();
        let plan = ticket_trash::build_forever(&c, &a.tickets, a.max_records)?;
        let ids: Vec<String> = plan.ids().iter().map(i64::to_string).collect();
        eprintln!("Tickets to delete PERMANENTLY: {}", ids.join(", "));
        for row in plan.subjects() {
            eprintln!(
                "  {}: {}",
                fsv_py::py_str(&row["id"]),
                fsv_py::py_str(&row["subject"])
            );
        }
        if a.dry_run {
            dry_run(plan.review());
            return Ok(());
        }
        if !a.yes || a.expect_plan.as_deref() != Some(plan.hash().as_str()) {
            return Err(refused(
                "review with --dry-run, then --yes --expect-plan FULL_HASH --confirm-destructive",
            ));
        }
        let result = ticket_trash::send_forever(
            &plan,
            &c,
            &DiskJournal,
            a.expect_plan.as_deref(),
            a.confirm_destructive,
        )?;
        finish(&result)
    })())
}
