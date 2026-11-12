//! Reviewed two-step attachments (`attachments_cli`): the shared output
//! gate, owner-verified deletion and attached problem notes.

use serde_json::{Map, Value, json};

use fsv_api::attachments::{self, AttachedPlan};
use fsv_api::problem_workflows;
use fsv_api::service::{Api, DiskJournal};
use fsv_core::Result;

use super::native::finish_problem;
use super::reviewed::{dumps, print_verified, refusal, require_full_hash};
use crate::generated::*;
use crate::util::{CmdResult, client};

/// Port of fsv.attachments_cli.finish: the dry run, or one reviewed send
/// that exits 4 when the read-back is not verified.
pub(crate) fn finish(
    plan: &AttachedPlan,
    c: &dyn Api,
    dry_run: bool,
    yes: bool,
    expected: Option<&str>,
    rebuild: &dyn Fn() -> Result<AttachedPlan>,
) -> Result<()> {
    if dry_run {
        let mut out = Map::new();
        out.insert("status".into(), json!("dry_run"));
        out.extend(plan.review());
        println!("{}", dumps(&Value::Object(out)));
        return Ok(());
    }
    let hash = plan.hash();
    require_full_hash(yes, expected, &hash)?;
    let result = attachments::send(plan, c, &DiskJournal, Some(&hash), rebuild)?;
    print_verified(&result)
}

/// Port of fsv.attachments_cli's `delete`.
pub fn delete(a: AttachmentsDeleteArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        let child = a.child.map(|n| json!(n));
        let section = a.section.as_deref();
        let plan = attachments::delete_plan(
            c.as_ref(),
            &json!(a.ident),
            &a.owner,
            child.as_ref(),
            section,
        )?;
        if a.dry_run {
            let mut out = Map::new();
            out.insert("status".into(), json!("dry_run"));
            out.extend(plan.review());
            out.insert("owner".into(), json!(a.owner));
            out.insert("child".into(), child.clone().unwrap_or(Value::Null));
            out.insert("section".into(), json!(section));
            println!("{}", dumps(&Value::Object(out)));
            return Ok(());
        }
        let hash = plan.hash();
        require_full_hash(a.yes, a.expect_plan.as_deref(), &hash)?;
        let result = attachments::delete(
            &plan,
            c.as_ref(),
            &DiskJournal,
            &a.owner,
            Some(&hash),
            child.as_ref(),
            section,
        )?;
        print_verified(&result)
    };
    run().map_err(refusal)
}

/// Port of fsv.problem_cli's `note`: attached files take the two-step
/// upload; a body-only note is the captured child write.
pub fn problems_note(a: ProblemsNoteArgs) -> CmdResult {
    let run = || -> Result<()> {
        let c = client()?;
        if a.attach.is_empty() {
            let plan = problem_workflows::child_plan(c.as_ref(), &a.target, "note", &a.body)?;
            return finish_problem(&c, plan, a.dry_run, a.yes, a.expect_plan.as_deref());
        }
        let rebuild = || {
            attachments::problem_note_plan(c.as_ref(), &a.target, &a.body, &a.attach, a.max_bytes)
        };
        finish(
            &rebuild()?,
            c.as_ref(),
            a.dry_run,
            a.yes,
            a.expect_plan.as_deref(),
            &rebuild,
        )
    };
    run().map_err(refusal)
}
