//! Integrated inventory (`integrated_assets_cli`), service-request approvals
//! (`sr_approvals_cli`) and draft solutions (`solutions_cli`). All share
//! `integrated_assets_cli.finish` and `guard`.

use serde_json::{Map, Value, json};

use fsv_api::service::DiskJournal;
use fsv_api::{
    approval_model, integrated_assets as assets, legacy_approvals, solutions, sr_approvals,
};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_core::{Error, Result};

use crate::cmd::reviewed::{dumps, print_dry_run};
use crate::generated::*;
use crate::lazy::LazyApi;
use crate::util::{CmdResult, plain_fail, uncaught};

/// `--dry-run`, `--yes`, `--expect-plan`, `--read-only`.
struct Flags {
    dry_run: bool,
    yes: bool,
    expected: Option<String>,
    read_only: bool,
}

macro_rules! flags {
    ($a:expr) => {
        Flags {
            dry_run: $a.dry_run,
            yes: $a.yes,
            expected: $a.expected.clone(),
            read_only: $a.read_only,
        }
    };
}

macro_rules! action {
    ($m:expr, $t:ty, $op:expr) => {
        match crate::cmd::parse::<$t>($m) {
            Ok(a) => approvals_action(&a.target, $op, a.ident, &a.remark, flags!(a)),
            Err(e) => Err(e),
        }
    };
}

/// `guard`: `ValueError`, `ReviewedWriteError`, `APIError` and
/// `SessionError` print their message and exit 2.
pub(crate) fn guard(r: Result<()>) -> CmdResult {
    r.map_err(|e| match e {
        Error::Value(m) | Error::Session(m) => plain_fail(&m, 2),
        Error::Exit { code: 2, message } if !message.is_empty() => plain_fail(&message, 2),
        Error::Api(_) => plain_fail(&e.to_string(), 2),
        other => uncaught(other),
    })
}

/// `finish`: dry-run review, or send only with `--yes` and the exact hash.
pub(crate) fn finish(
    review: Map<String, Value>,
    hash: &str,
    send: impl FnOnce() -> Result<Value>,
    dry_run: bool,
    yes: bool,
    expected: Option<&str>,
    read_only: bool,
) -> Result<()> {
    if dry_run {
        print_dry_run(review);
        return Ok(());
    }
    if read_only {
        return Err(Error::value("read-only mode; no write attempted"));
    }
    if !yes || expected != Some(hash) {
        return Err(Error::value(
            "review with --dry-run, then --yes --expect-plan FULL_HASH",
        ));
    }
    println!("{}", dumps(&send()?));
    Ok(())
}

/// `assets search QUERY` (top level and `changes assets search`).
pub fn search(query: &str, kind: &str, page: i64) -> CmdResult {
    let c = LazyApi::default();
    guard(assets::search(&c, query, kind, page).map(|v| println!("{}", dumps(&v))))
}

/// `MODULE assets list|ls TARGET`.
pub fn listing(module: &str, target: &str, kind: &str) -> CmdResult {
    let c = LazyApi::default();
    guard(assets::listing(&c, module, &json!(target), kind).map(|v| println!("{}", dumps(&v))))
}

/// `MODULE assets candidates TARGET`.
pub fn candidates(module: &str, target: &str) -> CmdResult {
    let c = LazyApi::default();
    guard(
        assets::legacy_candidates(&c, module, &json!(target))
            .map(|rows| println!("{}", dumps(&json!({"data": rows, "complete": true})))),
    )
}

/// `changes assets legacy TARGET [--search Q]`.
pub fn legacy(target: &str, search_: Option<&str>) -> CmdResult {
    eprintln!("deprecated: use assets search or assets list");
    match search_ {
        None => listing("changes", target, "devices"),
        Some(q) => search(q, "devices", 1),
    }
}

/// Flags of `assets add|remove`.
pub struct Mutate<'a> {
    pub target: &'a str,
    pub ids: &'a [i64],
    pub kind: &'a str,
    pub dry_run: bool,
    pub yes: bool,
    pub expected: Option<&'a str>,
    pub read_only: bool,
}

/// `MODULE assets add|remove TARGET IDS...`.
pub fn mutate(module: &str, remove: bool, a: Mutate<'_>) -> CmdResult {
    let run = || -> Result<()> {
        let c = LazyApi::default();
        let ids: Vec<Value> = a.ids.iter().map(|i| json!(i)).collect();
        let plan = assets::build_plan(&c, module, &json!(a.target), &ids, a.kind, remove)?;
        let hash = plan.hash();
        finish(
            plan.review(),
            &hash,
            || {
                assets::send(
                    &plan,
                    &c,
                    &DiskJournal,
                    module,
                    &ids,
                    a.kind,
                    remove,
                    a.expected,
                )
            },
            a.dry_run,
            a.yes,
            a.expected,
            a.read_only,
        )
    };
    guard(run())
}

/// Dispatch `assets ...`, `ci-*` and `approvals ...` below a module.
pub fn dispatch(module: &str, sub: &str, m: &clap::ArgMatches) -> Option<CmdResult> {
    use crate::cmd::parse;
    macro_rules! list {
        ($t:ty) => {{
            let a: $t = match parse(m) {
                Ok(a) => a,
                Err(e) => return Some(Err(e)),
            };
            listing(module, &a.target, &a.kind)
        }};
    }
    macro_rules! candidates {
        ($t:ty) => {{
            let a: $t = match parse(m) {
                Ok(a) => a,
                Err(e) => return Some(Err(e)),
            };
            candidates(module, &a.target)
        }};
    }
    macro_rules! write {
        ($t:ty, $remove:expr) => {{
            let a: $t = match parse(m) {
                Ok(a) => a,
                Err(e) => return Some(Err(e)),
            };
            mutate(
                module,
                $remove,
                Mutate {
                    target: &a.target,
                    ids: &a.ids,
                    kind: &a.kind,
                    dry_run: a.dry_run,
                    yes: a.yes,
                    expected: a.expected.as_deref(),
                    read_only: a.read_only,
                },
            )
        }};
    }
    Some(match (module, sub) {
        ("changes", "assets list") => list!(ChangesAssetsListArgs),
        ("changes", "assets ls") => list!(ChangesAssetsLsArgs),
        ("changes", "ci-list") => list!(ChangesCiListArgs),
        ("changes", "assets candidates") => candidates!(ChangesAssetsCandidatesArgs),
        ("tickets", "assets candidates") => candidates!(TicketsAssetsCandidatesArgs),
        ("problems", "assets candidates") => candidates!(ProblemsAssetsCandidatesArgs),
        ("sr", "assets candidates") => candidates!(SrAssetsCandidatesArgs),
        ("changes", "assets add") => write!(ChangesAssetsAddArgs, false),
        ("changes", "ci-add") => write!(ChangesCiAddArgs, false),
        ("changes", "assets remove") => write!(ChangesAssetsRemoveArgs, true),
        ("changes", "ci-remove") => write!(ChangesCiRemoveArgs, true),
        ("changes", "assets legacy") => match parse::<ChangesAssetsLegacyArgs>(m) {
            Ok(a) => legacy(&a.target, a.search.as_deref()),
            Err(e) => Err(e),
        },
        ("changes", "assets search") => match parse::<ChangesAssetsSearchArgs>(m) {
            Ok(a) => search(&a.query, &a.kind, a.page),
            Err(e) => Err(e),
        },
        ("tickets", "assets list") => list!(TicketsAssetsListArgs),
        ("tickets", "assets ls") => list!(TicketsAssetsLsArgs),
        ("tickets", "assets add") => write!(TicketsAssetsAddArgs, false),
        ("tickets", "assets remove") => write!(TicketsAssetsRemoveArgs, true),
        ("problems", "assets list") => list!(ProblemsAssetsListArgs),
        ("problems", "assets ls") => list!(ProblemsAssetsLsArgs),
        ("problems", "assets add") => write!(ProblemsAssetsAddArgs, false),
        ("problems", "assets remove") => write!(ProblemsAssetsRemoveArgs, true),
        ("sr", "assets list") => list!(SrAssetsListArgs),
        ("sr", "assets ls") => list!(SrAssetsLsArgs),
        ("sr", "assets add") => write!(SrAssetsAddArgs, false),
        ("sr", "assets remove") => write!(SrAssetsRemoveArgs, true),
        ("sr", "approvals list") => match parse::<SrApprovalsListArgs>(m) {
            Ok(a) => approvals_list(&a.target, a.json_out),
            Err(e) => Err(e),
        },
        ("sr", "approvals request") => match parse::<SrApprovalsRequestArgs>(m) {
            Ok(a) => approvals_request(a),
            Err(e) => Err(e),
        },
        ("sr", "approvals approve") => action!(m, SrApprovalsApproveArgs, "approve"),
        ("sr", "approvals reject") => action!(m, SrApprovalsRejectArgs, "reject"),
        ("sr", "approvals cancel") => action!(m, SrApprovalsCancelArgs, "cancel"),
        ("sr", "approvals remind") => action!(m, SrApprovalsRemindArgs, "remind"),
        ("problems", "link-solution") => match parse::<ProblemsLinkSolutionArgs>(m) {
            Ok(a) => link_solution(&a.target, a.article, false, flags!(a)),
            Err(e) => Err(e),
        },
        ("problems", "unlink-solution") => match parse::<ProblemsUnlinkSolutionArgs>(m) {
            Ok(a) => link_solution(&a.target, a.article, true, flags!(a)),
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

/// `approval_model.flat_table`: one row per flat (legacy) approval record.
pub(crate) fn flat_approval_table(records: &[Value]) -> fsv_core::term::Table {
    use fsv_core::term::{Column, Overflow, Table, escape};
    let mut t = Table::new();
    for col in ["Level", "Approver", "Status", "Decided", "Type", "Comment"] {
        t.add_column(Column::new(col).overflow(Overflow::Fold));
    }
    for r in records {
        t.add_row(approval_model::flat_cells(r).map(|c| escape(&c)));
    }
    t
}

/// `sr approvals list TARGET`: the flat (legacy) list gets the same row keys
/// and table as `tickets approvals`; chains output is unchanged.
fn approvals_list(target: &str, json_out: bool) -> CmdResult {
    let c = LazyApi::default();
    guard(
        parse_id(target, &TICKETS)
            .and_then(|id| sr_approvals::listing(&c, id))
            .map(|mut out| {
                let flat = match out.get("approvals") {
                    Some(Value::Array(rows)) => Some(approval_model::with_flat(rows)),
                    Some(_) => Some(Vec::new()),
                    None => None,
                };
                let Some(flat) = flat else {
                    println!("{}", dumps(&out));
                    return;
                };
                out["approvals"] = Value::Array(flat.clone());
                if json_out {
                    println!("{}", dumps(&out));
                    return;
                }
                fsv_core::term::out().print_table(&flat_approval_table(&flat));
                if let Some(note) = out.get("note").filter(|n| fsv_py::truthy(n)) {
                    fsv_core::term::err().print(&format!("[dim]{}[/dim]", fsv_py::py_str(note)));
                }
            }),
    )
}

fn approvals_finish(c: &LazyApi, plan: sr_approvals::ApprovalPlan, f: Flags) -> Result<()> {
    let hash = plan.hash();
    finish(
        plan.review(),
        &hash,
        || sr_approvals::send(&plan, c, &DiskJournal, f.expected.as_deref()),
        f.dry_run,
        f.yes,
        f.expected.as_deref(),
        f.read_only,
    )
}

/// `sr_approvals_cli.legacy`: the ticket ID and whether the tenant uses
/// legacy flat approvals (one read-only approval-chains GET).
fn legacy_target(c: &LazyApi, target: &str) -> Result<(i64, bool)> {
    let tid = parse_id(target, &TICKETS)?;
    let (model, _) = approval_model::detect(c, "tickets", tid)?;
    Ok((tid, model == approval_model::LEGACY))
}

fn legacy_finish(c: &LazyApi, plan: legacy_approvals::LegacyPlan, f: Flags) -> Result<()> {
    let hash = plan.hash();
    finish(
        plan.review(),
        &hash,
        || legacy_approvals::send(&plan, c, &DiskJournal, f.expected.as_deref()),
        f.dry_run,
        f.yes,
        f.expected.as_deref(),
        f.read_only,
    )
}

/// `sr approvals request TARGET [--name --message] --approver...`.
fn approvals_request(a: SrApprovalsRequestArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = LazyApi::default();
        let approvers: Vec<Value> = a.approver.iter().map(|s| json!(s)).collect();
        let (tid, is_legacy) = legacy_target(&c, &a.target)?;
        if is_legacy {
            if a.name.as_deref().is_some_and(|s| !s.is_empty())
                || a.message.as_deref().is_some_and(|s| !s.is_empty())
            {
                return Err(Error::value(
                    "--name/--message do not apply to legacy flat approvals; the email is the tenant's approval template",
                ));
            }
            let ids = sr_approvals::recipients(&c, &approvers)?;
            let plan = legacy_approvals::build_request(&c, "tickets", tid, &ids, &a.rule, "")?;
            return legacy_finish(&c, plan, flags!(a));
        }
        let plan = sr_approvals::build_plan(
            &c,
            &json!(a.target),
            "request",
            None,
            a.name.as_deref(),
            a.message.as_deref(),
            Some(&approvers),
            &a.rule,
            "",
        )?;
        approvals_finish(&c, plan, flags!(a))
    };
    guard(run())
}

/// `sr approvals approve|reject|cancel|remind TARGET ID`.
fn approvals_action(target: &str, op: &str, ident: i64, remark: &str, f: Flags) -> CmdResult {
    let run = || -> Result<()> {
        let c = LazyApi::default();
        if op == "remind" {
            let (tid, is_legacy) = legacy_target(&c, target)?;
            if is_legacy {
                let plan = legacy_approvals::build_remind(&c, "tickets", tid, ident)?;
                return legacy_finish(&c, plan, f);
            }
        }
        let plan = sr_approvals::build_plan(
            &c,
            &json!(target),
            op,
            Some(&json!(ident)),
            None,
            None,
            None,
            "everyone",
            remark,
        )?;
        approvals_finish(&c, plan, f)
    };
    guard(run())
}

/// `solutions create --title --body --type [--category] [--folder]`.
pub fn solutions_create(a: SolutionsCreateArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = LazyApi::default();
        let plan = solutions::create_plan(
            &c,
            &a.title,
            &a.body,
            &a.kind,
            a.category.as_deref(),
            a.folder.as_deref(),
        )?;
        let hash = plan.hash();
        let f = flags!(a);
        finish(
            plan.review(),
            &hash,
            || solutions::send_create(&plan, &c, &DiskJournal, f.expected.as_deref()),
            f.dry_run,
            f.yes,
            f.expected.as_deref(),
            f.read_only,
        )
    };
    guard(run())
}

/// `problems link-solution|unlink-solution TARGET ARTICLE`.
fn link_solution(target: &str, article: i64, unlink: bool, f: Flags) -> CmdResult {
    let run = || -> Result<()> {
        let c = LazyApi::default();
        let plan = solutions::link_plan(&c, &json!(target), &json!(article), unlink)?;
        let hash = plan.hash();
        finish(
            plan.review(),
            &hash,
            || solutions::send_link(&plan, &c, &DiskJournal, f.expected.as_deref()),
            f.dry_run,
            f.yes,
            f.expected.as_deref(),
            f.read_only,
        )
    };
    guard(run())
}
