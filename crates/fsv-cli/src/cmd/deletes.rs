//! Reviewed record deletion (`record_delete_cli`): problems delete, restore
//! and delete-forever; solutions delete and delete-forever.

use serde_json::{Map, Value, json};

use fsv_api::record_delete as flow;
use fsv_api::reviewed_write::{WritePlan, refused};
use fsv_api::service::DiskJournal;
use fsv_core::Result;

use crate::cmd::inventory::guard;
use crate::generated::*;
use crate::lazy::LazyApi;
use crate::util::CmdResult;

/// `json.dumps(v, indent=2, ensure_ascii=False)`.
fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default()
        .indent(2)
        .ensure_ascii(false)
        .dump(v)
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
        return Err(fsv_core::Error::exit(4, ""));
    }
    Ok(())
}

struct Gate<'a> {
    dry_run: bool,
    yes: bool,
    expect_plan: Option<&'a str>,
    confirm_destructive: bool,
}

/// `_run`: review, or send the exact reviewed plan.
fn run(build: impl FnOnce(&LazyApi) -> Result<WritePlan>, g: Gate<'_>) -> CmdResult {
    let go = || -> Result<()> {
        let c = LazyApi::default();
        let plan = build(&c)?;
        if g.dry_run {
            dry_run(flow::review(&plan, &c)?);
            return Ok(());
        }
        if !g.yes || g.expect_plan != Some(plan.hash().as_str()) {
            return Err(refused(
                "review with --dry-run, then --yes --expect-plan FULL_HASH (plus --confirm-destructive where the review requires it)",
            ));
        }
        let result = flow::send(
            &plan,
            &c,
            &DiskJournal,
            g.expect_plan,
            g.confirm_destructive,
        )?;
        finish(&result)
    };
    guard(go())
}

/// `_forever_command`: explicit IDs, the printed list, then the gate.
fn forever(kind: &str, idents: &[String], max_records: Option<i64>, g: Gate<'_>) -> CmdResult {
    let label = if kind == "problem" {
        "Problems"
    } else {
        "Articles"
    };
    let go = || -> Result<()> {
        let c = LazyApi::default();
        let batch = flow::build_forever(&c, &DiskJournal, kind, idents, max_records)?;
        let ids: Vec<String> = batch.ids().iter().map(i64::to_string).collect();
        eprintln!("{label} to delete PERMANENTLY: {}", ids.join(", "));
        for row in batch.rows() {
            eprintln!(
                "  {}: {}",
                fsv_py::py_str(&row["id"]),
                fsv_py::py_str(&row["title"])
            );
        }
        if g.dry_run {
            dry_run(batch.review());
            return Ok(());
        }
        if !g.yes || g.expect_plan != Some(batch.hash().as_str()) {
            return Err(refused(
                "review with --dry-run, then --yes --expect-plan FULL_HASH --confirm-destructive",
            ));
        }
        let result = flow::send_forever(
            &batch,
            &c,
            &DiskJournal,
            g.expect_plan,
            g.confirm_destructive,
        )?;
        finish(&result)
    };
    guard(go())
}

macro_rules! gate {
    ($a:expr) => {
        Gate {
            dry_run: $a.dry_run,
            yes: $a.yes,
            expect_plan: $a.expect_plan.as_deref(),
            confirm_destructive: $a.confirm_destructive,
        }
    };
}

pub fn problems_delete(a: ProblemsDeleteArgs) -> CmdResult {
    run(
        |c| flow::problem_plan(c, &DiskJournal, &a.ident, "delete"),
        gate!(a),
    )
}

pub fn problems_restore(a: ProblemsRestoreArgs) -> CmdResult {
    run(
        |c| flow::problem_plan(c, &DiskJournal, &a.ident, "restore"),
        gate!(a),
    )
}

pub fn problems_delete_forever(a: ProblemsDeleteForeverArgs) -> CmdResult {
    forever("problem", &a.idents, a.max_records, gate!(a))
}

pub fn solutions_delete(a: SolutionsDeleteArgs) -> CmdResult {
    run(|c| flow::article_plan(c, &a.ident, "delete"), gate!(a))
}

pub fn solutions_delete_forever(a: SolutionsDeleteForeverArgs) -> CmdResult {
    forever("article", &a.idents, a.max_records, gate!(a))
}
