//! Reviewed change writes: `tasks-add` (`change_tasks.py`), `approval-request`
//! (`change_approvals.py`), `approval-respond`/`approval-cancel`
//! (`change_approval_actions.py`).

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use fsv_api::approval_model::{CHAINS, LEGACY};
use fsv_api::associations::SendOpts;
use fsv_api::service::{Api, DiskJournal};
use fsv_api::{approval_recipients, approvals, legacy_approvals, tasks, ticket_tasks};
use fsv_core::Error;
use fsv_core::resource::{CHANGES, parse_id};
use fsv_core::term;

use crate::generated::*;
use crate::lazy::LazyApi;
use crate::util::{CmdResult, fail_exc, plain_fail};

/// `except (ValueError, TaskError/ApprovalError): echo(str(exc), err=True); Exit(2)`.
fn refused(e: Error) -> Error {
    match e {
        Error::Value(msg) => plain_fail(&msg, 2),
        Error::Exit { code: 2, message } if !message.is_empty() => plain_fail(&message, 2),
        other => fail_exc(other),
    }
}

fn value(msg: &str) -> Error {
    Error::value(msg)
}

fn indent2(v: &Value) -> String {
    fsv_py::Dumps::default().indent(2).dump(v)
}

fn confirm(text: &str) -> Result<bool, Error> {
    term::confirm(text, Some(false), false)
}

pub fn tasks_add(a: ChangesTasksAddArgs) -> CmdResult {
    tasks_add_inner(a).map_err(refused)
}

fn tasks_add_inner(a: ChangesTasksAddArgs) -> CmdResult {
    if a.read_only && !a.dry_run {
        return Err(value("read-only mode: no task was sent"));
    }
    let c = LazyApi::default();
    let tenant = c.domain();
    if let Some(status) = &a.status {
        tasks::status_label(status)?;
    }
    check_task_inputs(&a)?;
    let fields = tasks::runtime_fields(&c)?;
    let status = match &a.status {
        Some(s) => s.clone(),
        None => tasks::default_status(&fields)?,
    };
    let reminder = tasks::runtime_reminder_label(a.notify_before.as_deref(), &fields)?;
    let mut args = tasks::TaskArgs {
        status: &status,
        note: a.note.as_deref(),
        notify_before: &reminder,
        planned_start: a.planned_start.as_deref(),
        planned_end: a.planned_end.as_deref(),
        planned_effort: a.planned_effort.as_deref(),
        ..Default::default()
    };
    let mut raw_values = Map::new();
    for item in &a.values {
        let Some((key, val)) = item.split_once('=') else {
            return Err(value("--set requires NAME=VALUE"));
        };
        if raw_values.contains_key(key) {
            return Err(value("duplicate custom field assignment"));
        }
        raw_values.insert(key.into(), json!(val));
    }
    for (key, val) in [("cf_system", &a.system), ("cf_environment", &a.environment)] {
        if let Some(val) = val {
            if raw_values.contains_key(key) {
                return Err(value("duplicate custom field assignment"));
            }
            raw_values.insert(key.into(), json!(val));
        }
    }
    let custom = ticket_tasks::custom_values(&fields, &raw_values)?;
    args.owner_id = tasks::resolve_owner(&c, a.owner.as_deref())?;
    args.group_id = tasks::resolve_group(&c, a.group.as_deref())?;
    args.status_id = Some(tasks::resolve_status(&c, &status)?);
    args.custom_fields = Some(&custom);
    args.fields = Some(&fields);
    let plan = tasks::build_plan(&a.id_, &a.title, &args, &tenant)?;
    let journal = DiskJournal;
    if a.dry_run {
        let opts = SendOpts {
            dry_run: true,
            ..Default::default()
        };
        println!("{}", indent2(&tasks::send(&plan, &c, &journal, opts)?));
        return Ok(());
    }
    eprintln!("{}", indent2(&Value::Object(plan.review())));
    let hash = plan.hash();
    if a.expected.as_deref().is_some_and(|e| e != hash) {
        return Err(value(
            "task review hash does not match; review the exact task again",
        ));
    }
    if a.yes {
        if a.expected.as_deref() != Some(hash.as_str()) {
            return Err(value(
                "--yes requires the full --expect-plan from a prior task review",
            ));
        }
    } else if a.no_input {
        return Err(value(
            "sending requires --yes and --expect-plan in no-input mode",
        ));
    } else if !confirm("Create this exact reviewed task?")? {
        println!("Cancelled; no task was sent.");
        return Ok(());
    }
    let opts = SendOpts {
        reviewed_hash: Some(&hash),
        ..Default::default()
    };
    let result = tasks::send(&plan, &c, &journal, opts)?;
    if a.json_out {
        println!("{}", indent2(&result));
    } else {
        println!(
            "Task {} submitted on change {}; exact-ID/state read-back: {}",
            fsv_py::py_str(&result["task_id"]),
            plan.change_id,
            fsv_py::py_str(&result["verified"])
        );
        if let Some(w) = result.get("warning").filter(|v| fsv_py::truthy(v)) {
            eprintln!("{}", fsv_py::py_str(w));
        }
    }
    Ok(())
}

/// Port of change_tasks.check_inputs: local checks before any remote lookup;
/// status and reminder need runtime metadata.
fn check_task_inputs(a: &ChangesTasksAddArgs) -> Result<(), Error> {
    if parse_id(&a.id_, &CHANGES)? <= 0 {
        return Err(value("change ID must be a positive display ID"));
    }
    if fsv_core::pystr::strip(&a.title).is_empty()
        || a.title.chars().any(|c| (c as u32) < 32 || c as u32 == 127)
    {
        return Err(value(
            "task title must be nonempty text without control characters",
        ));
    }
    let instant = |v: &Option<String>| -> Result<Option<jiff::Timestamp>, Error> {
        Ok(tasks::date(v.as_deref())?.and_then(|s| {
            let (dt, off) = fsv_forms::pydt::datetime_fromisoformat(&s)?;
            off?.to_timestamp(dt).ok()
        }))
    };
    if let (Some(start), Some(end)) = (instant(&a.planned_start)?, instant(&a.planned_end)?)
        && end < start
    {
        return Err(value("planned end must not precede planned start"));
    }
    tasks::note(a.note.as_deref())?;
    tasks::effort(a.planned_effort.as_deref())?;
    Ok(())
}

pub fn approval_request(a: ChangesApprovalRequestArgs) -> CmdResult {
    approval_request_inner(a).map_err(refused)
}

fn approval_request_inner(a: ChangesApprovalRequestArgs) -> CmdResult {
    if a.read_only && !a.dry_run {
        return Err(value("read-only mode: no approval request was sent"));
    }
    if a.approver.is_empty() && a.approver_group.is_empty() && a.cab.is_empty() {
        return Err(value(
            "provide --approver, --approver-group or --cab explicitly",
        ));
    }
    if !matches!(a.model.as_str(), "auto" | "chains" | "legacy") {
        return Err(value("--model must be auto, chains or legacy"));
    }
    if a.model == "chains" && (a.name.is_none() || a.message.is_none()) {
        return Err(value(
            "--name and --message are required on the approval chains model",
        ));
    }
    let c = LazyApi::default();
    if a.model != "chains" && approval_model(&c, parse_id(&a.id_, &CHANGES)?, &a.model)? == LEGACY {
        return legacy_request(&c, &a);
    }
    let (Some(name), Some(message)) = (a.name.clone(), a.message.clone()) else {
        return Err(value(
            "--name and --message are required on the approval chains model",
        ));
    };
    let tenant = c.domain();
    approvals::build_plan(&a.id_, &name, &message, &[1], &a.rule, &tenant)?;
    let target = parse_id(&a.id_, &CHANGES)?;
    let (extra, context) = approval_recipients::resolve(&c, target, &a.approver_group, &a.cab)?;
    let mut ids: BTreeSet<i64> = extra.into_iter().collect();
    if !a.approver.is_empty() {
        ids.extend(approvals::resolve_approvers(&c, &a.approver)?);
    }
    let ids: Vec<i64> = ids.into_iter().collect();
    let plan =
        approvals::build_plan_with(&a.id_, &name, &message, &ids, &a.rule, &tenant, &context)?;
    let journal = DiskJournal;
    if a.dry_run {
        let opts = SendOpts {
            dry_run: true,
            ..Default::default()
        };
        println!("{}", indent2(&approvals::send(&plan, &c, &journal, opts)?));
        return Ok(());
    }
    eprintln!("{}", indent2(&Value::Object(plan.review())));
    let hash = plan.hash();
    if a.expected.as_deref().is_some_and(|e| e != hash) {
        return Err(value(
            "approval review hash does not match; review the exact request again",
        ));
    }
    if a.yes {
        if a.expected.as_deref() != Some(hash.as_str()) {
            return Err(value(
                "--yes requires the full --expect-plan from a prior approval review",
            ));
        }
    } else if a.no_input {
        return Err(value(
            "sending requires --yes and --expect-plan in no-input mode",
        ));
    } else if !confirm(
        "Send this exact approval request and irreversible emails (no verified revoke)?",
    )? {
        println!("Cancelled; no approval request was sent.");
        return Ok(());
    }
    let opts = SendOpts {
        reviewed_hash: Some(&hash),
        ..Default::default()
    };
    let result = approvals::send(&plan, &c, &journal, opts)?;
    if a.json_out {
        println!("{}", indent2(&result));
    } else {
        println!(
            "Approval requested on change {}; approval record IDs: {}; complete inventory verification: {}; group name verified: {}",
            plan.change_id,
            fsv_py::py_str(&result["approval_ids"]),
            fsv_py::py_str(&result["verified"]),
            fsv_py::py_str(&result["group_name_verified"])
        );
    }
    Ok(())
}

/// Port of change_approvals._model: `auto` is one read-only approval-chains GET.
fn approval_model(c: &LazyApi, cid: i64, choice: &str) -> Result<&'static str, Error> {
    if !matches!(choice, "auto" | "chains" | "legacy") {
        return Err(value("--model must be auto, chains or legacy"));
    }
    match choice {
        "chains" => Ok(CHAINS),
        "legacy" => Ok(LEGACY),
        _ => {
            let (model, _) = fsv_api::approval_model::detect(c, "changes", cid)?;
            Ok(if model == LEGACY { LEGACY } else { CHAINS })
        }
    }
}

/// Port of change_approvals._legacy_request_cli. `--model auto` never
/// forwards `--name`/`--message`.
fn legacy_request(c: &LazyApi, a: &ChangesApprovalRequestArgs) -> CmdResult {
    let explicit = a.model == "legacy";
    let given = |v: &Option<String>| explicit && v.as_deref().is_some_and(|s| !s.is_empty());
    if given(&a.name) || given(&a.message) {
        return Err(value(
            "--name/--message do not apply to legacy flat approvals (no group name or message on the wire)",
        ));
    }
    if !a.approver_group.is_empty() {
        return Err(value(
            "--approver-group has no legacy flat-approval equivalent; use --cab or --approver",
        ));
    }
    let cid = parse_id(&a.id_, &CHANGES)?;
    let (extra, context) = approval_recipients::resolve(c, cid, &[], &a.cab)?;
    let mut ids: BTreeSet<i64> = extra.into_iter().collect();
    if !a.approver.is_empty() {
        ids.extend(approvals::resolve_approvers(c, &a.approver)?);
    }
    let ids: Vec<i64> = ids.into_iter().collect();
    let plan = legacy_approvals::build_request(c, "changes", cid, &ids, &a.rule, &context)?;
    if a.dry_run {
        let mut out = Map::new();
        out.insert("status".into(), json!("dry_run"));
        out.extend(plan.review());
        println!("{}", indent2(&Value::Object(out)));
        return Ok(());
    }
    eprintln!("{}", indent2(&Value::Object(plan.review())));
    let hash = plan.hash();
    if a.yes {
        if a.expected.as_deref() != Some(hash.as_str()) {
            return Err(value(
                "--yes requires the full --expect-plan from a prior approval review",
            ));
        }
    } else if a.no_input {
        return Err(value(
            "sending requires --yes and --expect-plan in no-input mode",
        ));
    } else if !confirm("Send this exact legacy approval request and irreversible emails?")? {
        println!("Cancelled; no approval request was sent.");
        return Ok(());
    }
    approval_recipients::validate(c, cid, &context)?;
    let result = legacy_approvals::send(&plan, c, &DiskJournal, Some(&hash))?;
    crate::cmd::reviewed::print_verified(&result)
}

const CHAINS_REMIND_GAP: &str = "approval-remind is not available on the approval-chains model: the only reminder accepted on a chains tenant is the service-request one (`fsv sr approvals remind`, bodyless PUT tickets/<id>/approvals/<approval id>/resend). The same web constant would build changes/<id>/approvals/<approval id>/resend, but no change reminder was ever accepted (the legacy change capture was aborted unsent), so fsv does not guess it; nothing was sent. Use the web UI: change > Approvals > the approval row menu > Send reminder";

/// `changes approval-remind CHANGE_ID APPROVAL_ID` (legacy flat approvals).
pub fn approval_remind(a: ChangesApprovalRemindArgs) -> CmdResult {
    let run = || -> CmdResult {
        let c = LazyApi::default();
        let cid = parse_id(&a.id_, &CHANGES)?;
        if approval_model(&c, cid, &a.model)? != LEGACY {
            return Err(value(CHAINS_REMIND_GAP));
        }
        let plan = legacy_approvals::build_remind(&c, "changes", cid, a.approval_id)?;
        if a.dry_run {
            let mut out = Map::new();
            out.insert("status".into(), json!("dry_run"));
            out.extend(plan.review());
            println!("{}", indent2(&Value::Object(out)));
            return Ok(());
        }
        let hash = plan.hash();
        if !a.yes || a.expected.as_deref() != Some(hash.as_str()) {
            return Err(value(
                "review with --dry-run, then --yes --expect-plan FULL_HASH",
            ));
        }
        let result = legacy_approvals::send(&plan, &c, &DiskJournal, a.expected.as_deref())?;
        crate::cmd::reviewed::print_verified(&result)
    };
    run().map_err(refused)
}

struct ActionArgs {
    dry_run: bool,
    read_only: bool,
    yes: bool,
    expected: Option<String>,
    no_input: bool,
    json_out: bool,
}

pub fn approval_respond(a: ChangesApprovalRespondArgs) -> CmdResult {
    let run = || -> CmdResult {
        if a.approve == a.reject {
            return Err(value("choose exactly one of --approve or --reject"));
        }
        let op = if a.approve { "approve" } else { "reject" };
        let plan = approvals::build_action_plan(
            &a.change,
            &json!(a.approval_id),
            op,
            a.remark.as_deref(),
            &fsv_core::config::domain(),
        )?;
        run_action(
            &plan,
            ActionArgs {
                dry_run: a.dry_run,
                read_only: a.read_only,
                yes: a.yes,
                expected: a.expected.clone(),
                no_input: a.no_input,
                json_out: a.json_out,
            },
        )
    };
    run().map_err(refused)
}

pub fn approval_cancel(a: ChangesApprovalCancelArgs) -> CmdResult {
    let run = || -> CmdResult {
        let plan = approvals::build_action_plan(
            &a.change,
            &json!(a.group_id),
            "cancel",
            None,
            &fsv_core::config::domain(),
        )?;
        run_action(
            &plan,
            ActionArgs {
                dry_run: a.dry_run,
                read_only: a.read_only,
                yes: a.yes,
                expected: a.expected.clone(),
                no_input: a.no_input,
                json_out: a.json_out,
            },
        )
    };
    run().map_err(refused)
}

/// `change_approval_actions._run`.
fn run_action(plan: &approvals::ActionPlan, o: ActionArgs) -> CmdResult {
    if o.read_only && !o.dry_run {
        return Err(value("read-only mode: no approval action was sent"));
    }
    let c = LazyApi::default();
    let journal = DiskJournal;
    if o.dry_run {
        let opts = SendOpts {
            dry_run: true,
            ..Default::default()
        };
        println!(
            "{}",
            indent2(&approvals::send_action(plan, &c, &journal, opts)?)
        );
        return Ok(());
    }
    let hash = plan.hash();
    if o.expected.as_deref().is_some_and(|e| e != hash) {
        return Err(value(
            "approval action review hash does not match; review the exact action again",
        ));
    }
    if o.yes {
        if o.expected.as_deref() != Some(hash.as_str()) {
            return Err(value(
                "--yes requires the full --expect-plan from a prior approval action review",
            ));
        }
    } else if o.no_input {
        return Err(value(
            "sending requires --yes and --expect-plan in no-input mode",
        ));
    }
    let cancel = plan.operation == "cancel";
    let mut review = plan.review();
    if cancel {
        let client = crate::util::shared_client().map_err(fail_exc)?;
        let group = approvals::cancel_group(client.as_ref(), plan)?;
        review.insert("group".into(), group);
    }
    eprintln!("{}", indent2(&Value::Object(review)));
    if !o.yes
        && !confirm(
            "Send this exact approval action (notifies people; may move workflow; cancel irreversible)?",
        )?
    {
        println!("Cancelled; no approval action was sent.");
        return Ok(());
    }
    let opts = SendOpts {
        reviewed_hash: Some(&hash),
        read_only: o.read_only,
        dry_run: false,
    };
    let result = approvals::send_action(plan, &c, &journal, opts)?;
    if o.json_out {
        println!("{}", indent2(&result));
    } else {
        println!(
            "Approval {} {} submitted on change {}; observed raw status: {}; verification: {}",
            if cancel { "group" } else { "record" },
            plan.target_id,
            plan.change_id,
            fsv_py::Dumps::default().dump(&result["observed_status"]),
            fsv_py::py_str(&result["verification_basis"])
        );
        if let Some(w) = result.get("warning").filter(|v| fsv_py::truthy(v)) {
            eprintln!("{}", fsv_py::py_str(w));
        }
    }
    Ok(())
}
