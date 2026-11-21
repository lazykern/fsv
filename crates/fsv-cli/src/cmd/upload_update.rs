//! `_reviewed_upload_update`: the shared driver of the reviewed upload
//! transactions (`changes|problems update --planning`, attach-only
//! `changes update --attach`): a no-op exits 0 unless `--expect-plan` was
//! given (then 2); a dry run prints the plan hash; `--yes --expect-plan
//! FULL_HASH` sends; an unverified readback exits 4.

use serde_json::{Map, Value, json};

use fsv_api::change_attach::{self, ChangeAttachPlan};
use fsv_api::planning_files::{self, Failure, PlanningUpdatePlan};
use fsv_api::service::{DiskJournal, Journal};
use fsv_core::Error;
use fsv_core::output::emit_json;
use fsv_py::Dumps;

use crate::util::{CmdResult, no_input, plain_fail};

/// What the driver needs of a reviewed upload plan.
pub trait UploadPlan {
    fn noop(&self) -> bool;
    fn action(&self) -> &'static str;
    fn hash(&self) -> String;
    fn review(&self) -> Map<String, Value>;
}

impl UploadPlan for PlanningUpdatePlan {
    fn noop(&self) -> bool {
        PlanningUpdatePlan::noop(self)
    }
    fn action(&self) -> &'static str {
        PlanningUpdatePlan::action(self)
    }
    fn hash(&self) -> String {
        PlanningUpdatePlan::hash(self)
    }
    fn review(&self) -> Map<String, Value> {
        PlanningUpdatePlan::review(self)
    }
}

impl UploadPlan for ChangeAttachPlan {
    fn noop(&self) -> bool {
        ChangeAttachPlan::noop(self)
    }
    fn action(&self) -> &'static str {
        change_attach::ACTION
    }
    fn hash(&self) -> String {
        ChangeAttachPlan::hash(self)
    }
    fn review(&self) -> Map<String, Value> {
        ChangeAttachPlan::review(self)
    }
}

/// `tenant_json(obj, ensure_ascii=False, indent=2)`.
fn pretty(v: &Value) -> String {
    Dumps::default()
        .ensure_ascii(false)
        .indent(2)
        .dump(&fsv_core::tenant::annotate(v))
}

/// `except (ValueError, ReviewedWriteError)`: a refusal, exit 2.
fn is_refusal(e: &Error) -> bool {
    matches!(e, Error::Value(_) | Error::Exit { code: 2, .. })
}

pub struct Texts<'a> {
    pub changed: &'a str,
    pub confirm: &'a str,
}

pub struct Flags<'a> {
    pub dry_run: bool,
    pub yes: bool,
    pub expect_plan: Option<&'a str>,
    pub json_out: bool,
    pub no_input: bool,
}

/// The driver. `send` is called with the reviewed plan; errors it does not
/// map to a refusal propagate for the caller's `_err_exc`.
pub fn run<P: UploadPlan>(
    build: &dyn Fn() -> fsv_core::Result<P>,
    send: &dyn Fn(&P) -> Result<Value, Failure>,
    noop: &dyn Fn(&P) -> (Value, String),
    flags: &Flags,
    texts: &Texts,
) -> CmdResult {
    let plan = match build() {
        Ok(p) => p,
        Err(e) if is_refusal(&e) => return Err(plain_fail(&e.to_string(), 2)),
        Err(e) => return Err(e),
    };
    let stale = |expect: &str| DiskJournal.stale_plan_refusal(Some(expect), &[plan.action()]);
    if plan.noop() {
        if let Some(expect) = flags.expect_plan {
            let msg = stale(expect).unwrap_or_else(|| {
                "nothing to change; the reviewed plan does not exist. Nothing was sent".into()
            });
            return Err(plain_fail(&msg, 2));
        }
        let (body, text) = noop(&plan);
        if flags.json_out {
            emit_json(&body);
        } else {
            println!("{text}");
        }
        return Ok(());
    }
    if flags.dry_run {
        let mut out = Map::new();
        out.insert("status".into(), json!("dry_run"));
        out.extend(plan.review());
        println!("{}", pretty(&Value::Object(out)));
        return Ok(());
    }
    let hash = plan.hash();
    let attempt = || -> Result<Option<Value>, Failure> {
        if let Some(expect) = flags.expect_plan
            && expect != hash
        {
            return Err(planning_files::refusal(stale(expect).unwrap_or_else(
                || {
                    format!(
                        "{} (or a partial hash); review again with --dry-run. Nothing was sent",
                        texts.changed
                    )
                },
            )));
        }
        if flags.yes {
            if flags.expect_plan != Some(hash.as_str()) {
                return Err(planning_files::refusal(
                    "--yes requires the full --expect-plan from a prior --dry-run review",
                ));
            }
        } else if no_input(flags.no_input) {
            return Err(planning_files::refusal(
                "sending requires --yes and --expect-plan in no-input mode",
            ));
        } else {
            eprintln!("{}", pretty(&Value::Object(plan.review())));
            if !fsv_core::term::confirm(texts.confirm, Some(false), false)? {
                println!("Cancelled; nothing sent.");
                return Ok(None);
            }
        }
        send(&plan).map(Some)
    };
    match attempt() {
        Ok(None) => Ok(()),
        Ok(Some(result)) => {
            println!("{}", pretty(&result));
            if result.get("verified").is_some_and(fsv_py::truthy) {
                Ok(())
            } else {
                Err(Error::exit(4, ""))
            }
        }
        Err(f) if is_refusal(&f.error) || matches!(f.error, Error::Api(_)) => {
            if flags.json_out {
                let mut out = obj_error(&plan.hash());
                out.extend(f.as_dict());
                emit_json(&Value::Object(out));
            }
            Err(plain_fail(&f.error.to_string(), 2))
        }
        Err(f) => Err(f.error),
    }
}

fn obj_error(hash: &str) -> Map<String, Value> {
    match json!({"status": "error", "plan_hash": hash, "verified": false}) {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// `_change_planning_update`: `changes|problems update --planning`.
#[allow(clippy::too_many_arguments)]
pub fn planning(
    module: &str,
    cid: i64,
    section: &str,
    description: Option<&str>,
    files: &[String],
    duplicate: &str,
    backup_replaced: bool,
    max_bytes: Option<i64>,
    flags: &Flags,
) -> CmdResult {
    let c = crate::util::client()?;
    let schema: &fsv_api::planning_text::ChangeSchema = &fsv_api::planning_text::live_change_schema;
    let opts = planning_files::PlanOpts {
        duplicate,
        backup_replaced,
        max_bytes,
        module,
    };
    let build = || {
        planning_files::build_plan(
            c.as_ref(),
            &DiskJournal,
            &fsv_api::create::StdinPrompter,
            schema,
            cid,
            section,
            description,
            files,
            &opts,
        )
    };
    let send = |plan: &PlanningUpdatePlan| {
        planning_files::send(
            plan,
            c.as_ref(),
            &DiskJournal,
            schema,
            Some(&plan.hash()),
            &build,
        )
    };
    let noop = |plan: &PlanningUpdatePlan| {
        let skipped = plan.skipped.join(", ");
        (
            json!({"status": "nothing_to_change", "target": cid, "section": plan.section,
                   "skipped_files": plan.skipped}),
            format!(
                "nothing to change in planning field {}{}",
                fsv_py::py_repr_str(&plan.label),
                if skipped.is_empty() {
                    String::new()
                } else {
                    format!(" ({skipped} already attached)")
                }
            ),
        )
    };
    let changed = format!(
        "the {}, planning section or files changed since review",
        module.trim_end_matches('s')
    );
    run(
        &build,
        &send,
        &noop,
        flags,
        &Texts {
            changed: &changed,
            confirm: "Upload these files and save this planning section?",
        },
    )
}

/// `_change_attach_update`: `changes update ID --attach FILE` (attach-only).
#[allow(clippy::too_many_arguments)]
pub fn attach(
    cid: i64,
    files: &[String],
    duplicate: &str,
    backup_replaced: bool,
    max_bytes: Option<i64>,
    flags: &Flags,
) -> CmdResult {
    let c = crate::util::client()?;
    let opts = change_attach::AttachOpts {
        duplicate,
        backup_replaced,
        max_bytes,
        no_input: no_input(flags.no_input),
    };
    let build = || {
        change_attach::build_plan(
            c.as_ref(),
            &DiskJournal,
            &fsv_api::create::StdinPrompter,
            cid,
            files,
            &opts,
        )
    };
    let send = |plan: &ChangeAttachPlan| {
        change_attach::send(plan, c.as_ref(), &DiskJournal, Some(&plan.hash()), &build)
    };
    let noop = |plan: &ChangeAttachPlan| {
        let skipped = plan.skipped.join(", ");
        (
            json!({"status": "nothing_to_change", "target": cid, "skipped_files": plan.skipped}),
            format!(
                "nothing to change on change {cid}{}",
                if skipped.is_empty() {
                    String::new()
                } else {
                    format!(" ({skipped} already attached)")
                }
            ),
        )
    };
    run(
        &build,
        &send,
        &noop,
        flags,
        &Texts {
            changed: "the change's attachments or files changed since review",
            confirm: "Upload these files and attach them to the change?",
        },
    )
}
