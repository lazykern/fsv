//! `fsv tickets problems|changes|children list|candidates|link|unlink`:
//! ticket-side problem, change and child-ticket associations, reviewed
//! (`ticket_associations_cli.py`).

use clap::ArgMatches;

use fsv_api::service::DiskJournal;
use fsv_api::ticket_associations::{self as links, KIND_HELP};
use fsv_core::{Error, Result};

use super::parse;
use super::reviewed::{dumps, print_dry_run, print_verified, refusal, require_full_hash};
use crate::generated::*;
use crate::util::{CmdResult, client, plain_fail};

/// `_fail`: a read failure is a one-line refusal; ValueError and
/// ReviewedWriteError print their message; everything else is uncaught.
fn fail(e: Error) -> Error {
    match e.as_api() {
        Some(api) => plain_fail(
            &format!("read failed (HTTP {}); nothing was sent", api.status),
            2,
        ),
        None => refusal(e),
    }
}

/// The `list` command of `_group`.
fn list(kind: &str, ticket: &str, ctype: Option<&str>) -> CmdResult {
    let run = || -> Result<()> {
        if kind == "change" && ctype.is_none_or(str::is_empty) {
            return Err(Error::value(format!("--kind is required: {KIND_HELP}")));
        }
        let c = client()?;
        println!(
            "{}",
            dumps(&links::listing(c.as_ref(), ticket, kind, ctype)?)
        );
        Ok(())
    };
    run().map_err(fail)
}

/// The `candidates` command of `_group`.
fn candidates(kind: &str, ticket: &str, search: Option<&str>, ctype: Option<&str>) -> CmdResult {
    let run = || -> Result<()> {
        let needs_type = kind == "change";
        let ctype = ctype.filter(|t| !t.is_empty());
        if needs_type && ctype.is_none() && search.is_none_or(str::is_empty) {
            return Err(Error::value(format!(
                "--kind is required for the picker list: {KIND_HELP}"
            )));
        }
        let ctype = needs_type.then(|| ctype.unwrap_or("change"));
        let c = client()?;
        let out = links::candidates(c.as_ref(), ticket, kind, search, ctype)?;
        println!("{}", dumps(&out));
        Ok(())
    };
    run().map_err(fail)
}

struct Mutate<'a> {
    ticket: &'a str,
    item: &'a str,
    ctype: Option<&'a str>,
    dry_run: bool,
    yes: bool,
    expect_plan: Option<&'a str>,
}

/// The `link` / `unlink` commands of `_group`.
fn mutate(kind: &str, unlink: bool, a: Mutate<'_>) -> CmdResult {
    let run = || -> Result<()> {
        let ctype = a.ctype.filter(|t| !t.is_empty());
        if kind == "change" && ctype.is_none() {
            return Err(Error::value(format!("--kind is required: {KIND_HELP}")));
        }
        if kind != "change" && ctype.is_some() {
            return Err(Error::value("--kind applies to changes only"));
        }
        let c = client()?;
        let plan = links::build_plan(c.as_ref(), a.ticket, kind, a.item, unlink, ctype)?;
        if a.dry_run {
            print_dry_run(plan.review());
            return Ok(());
        }
        let hash = plan.hash();
        require_full_hash(a.yes, a.expect_plan, &hash)?;
        let result = links::send(&plan, c.as_ref(), &DiskJournal, a.expect_plan)?;
        print_verified(&result)
    };
    run().map_err(fail)
}

/// Run `tickets <group> <command>` when `sub` is one of these commands.
pub fn dispatch(sub: &str, m: &ArgMatches) -> Option<CmdResult> {
    macro_rules! go {
        ($t:ty, |$a:ident| $body:expr) => {
            match parse::<$t>(m) {
                Ok($a) => $body,
                Err(e) => Err(e),
            }
        };
    }
    macro_rules! write {
        ($t:ty, $kind:expr, $unlink:expr) => {
            go!($t, |a| mutate(
                $kind,
                $unlink,
                Mutate {
                    ticket: &a.ticket,
                    item: &a.item,
                    ctype: a.ctype.as_deref(),
                    dry_run: a.dry_run,
                    yes: a.yes,
                    expect_plan: a.expect_plan.as_deref(),
                },
            ))
        };
    }
    Some(match sub {
        "problems list" => go!(TicketsProblemsListArgs, |a| list(
            "problem",
            &a.ticket,
            a.ctype.as_deref()
        )),
        "changes list" => go!(TicketsChangesListArgs, |a| list(
            "change",
            &a.ticket,
            a.ctype.as_deref()
        )),
        "children list" => go!(TicketsChildrenListArgs, |a| list(
            "child",
            &a.ticket,
            a.ctype.as_deref()
        )),
        "problems candidates" => go!(TicketsProblemsCandidatesArgs, |a| candidates(
            "problem",
            &a.ticket,
            a.search.as_deref(),
            a.ctype.as_deref()
        )),
        "changes candidates" => go!(TicketsChangesCandidatesArgs, |a| candidates(
            "change",
            &a.ticket,
            a.search.as_deref(),
            a.ctype.as_deref()
        )),
        "children candidates" => go!(TicketsChildrenCandidatesArgs, |a| candidates(
            "child",
            &a.ticket,
            a.search.as_deref(),
            a.ctype.as_deref()
        )),
        "problems link" => write!(TicketsProblemsLinkArgs, "problem", false),
        "problems unlink" => write!(TicketsProblemsUnlinkArgs, "problem", true),
        "changes link" => write!(TicketsChangesLinkArgs, "change", false),
        "changes unlink" => write!(TicketsChangesUnlinkArgs, "change", true),
        "children link" => write!(TicketsChildrenLinkArgs, "child", false),
        "children unlink" => write!(TicketsChildrenUnlinkArgs, "child", true),
        _ => return None,
    })
}
