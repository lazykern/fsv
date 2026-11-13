//! Ticket templates (read-only) and change maintenance windows
//! (`templates_cli.py`).

use serde_json::{Map, Value, json};

use fsv_api::change_windows::{self as windows, CreateArgs, WindowEnv};
use fsv_api::service::DiskJournal;
use fsv_api::templates;
use fsv_core::resource::CHANGES;
use fsv_core::{Error, Result};

use crate::cmd::inventory::guard;
use crate::generated::*;
use crate::util::{CmdResult, client};

/// `json.dumps(value, indent=2, ensure_ascii=False)`.
fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default()
        .indent(2)
        .ensure_ascii(false)
        .dump(v)
}

fn text(v: Option<&Value>) -> String {
    fsv_py::py_str(v.unwrap_or(&Value::Null))
}

fn review_required() -> Error {
    Error::exit(
        2,
        "review with --dry-run, then --yes --expect-plan FULL_HASH",
    )
}

fn dry_run(review: Map<String, Value>) {
    let mut out = Map::new();
    out.insert("status".into(), json!("dry_run"));
    out.extend(review);
    println!("{}", dumps(&Value::Object(out)));
}

/// Print a send result; an unverified readback exits 4.
fn finish(result: &Value) -> Result<()> {
    println!("{}", dumps(result));
    if !result.get("verified").is_some_and(fsv_py::truthy) {
        return Err(Error::exit(4, ""));
    }
    Ok(())
}

/// `fsv templates list`.
pub fn templates_list(a: TemplatesListArgs) -> CmdResult {
    let c = client()?;
    guard((|| {
        let rows = templates::list_templates(c.as_ref())?;
        if a.json_out {
            println!("{}", dumps(&Value::Array(rows)));
            return Ok(());
        }
        for r in &rows {
            let kind = r.get("type").filter(|v| fsv_py::truthy(v));
            println!(
                "{}\t{}\t{}",
                text(r.get("id")),
                text(r.get("name")),
                kind.map(fsv_py::py_str).unwrap_or_default()
            );
        }
        if rows.is_empty() {
            println!("No ticket templates.");
        }
        Ok(())
    })())
}

/// `fsv templates show NAME_OR_ID`.
pub fn templates_show(a: TemplatesShowArgs) -> CmdResult {
    let c = client()?;
    guard((|| {
        let row = templates::show(c.as_ref(), &a.name)?;
        let p = templates::to_draft(&row);
        let doc = json!({"id": row["id"], "name": row.get("name").cloned().unwrap_or(Value::Null),
                         "prefills": p.applied, "not_applied": p.ignored,
                         "template_data": row["template_data"]});
        println!("{}", dumps(&doc));
        Ok(())
    })())
}

/// `fsv changes windows list`.
pub fn windows_list(a: ChangesWindowsListArgs) -> CmdResult {
    let c = client()?;
    guard((|| {
        let rows = windows::list_windows(c.as_ref())?;
        if a.json_out {
            println!("{}", dumps(&Value::Array(rows)));
            return Ok(());
        }
        for r in &rows {
            println!(
                "{}\t{}\t{} -> {}",
                text(r.get("id")),
                text(r.get("name")),
                text(r.get("start_time")),
                text(r.get("end_time"))
            );
        }
        if rows.is_empty() {
            println!("No maintenance windows.");
        }
        Ok(())
    })())
}

/// `fsv changes windows create`.
pub fn windows_create(a: ChangesWindowsCreateArgs) -> CmdResult {
    let c = client()?;
    let api = c.as_ref();
    let me = std::cell::OnceCell::new();
    let current_user = || -> Result<Value> {
        if let Some(v) = me.get() {
            return Ok(Value::clone(v));
        }
        // `views.current_user`: a session error propagates, anything else
        // unreadable is `{}`.
        let v = match api.me() {
            Ok(v) if v.is_object() => v,
            Err(e @ Error::Session(_)) => return Err(e),
            _ => json!({}),
        };
        Ok(me.get_or_init(|| v).clone())
    };
    let schema = || fsv_api::schema::load(&CHANGES, &c, false);
    let env = WindowEnv {
        schema: &schema,
        configured_workspace: fsv_api::configured_workspace(),
        fsv_tz: std::env::var("FSV_TZ").ok(),
        current_user: &current_user,
    };
    let args = CreateArgs {
        name: &a.name,
        description: &a.description,
        start: &a.start,
        end: &a.end,
        criteria: &a.criteria,
        alert_suppression: !a.no_alert_suppression,
        timezone: a.timezone.as_deref(),
        allow_overlap: a.allow_overlap,
    };
    let build = || windows::create_plan(api, &env, &args);
    guard((|| {
        let plan = build()?;
        if a.dry_run {
            dry_run(plan.review()?);
            return Ok(());
        }
        if !a.yes || a.expect_plan.as_deref() != Some(plan.hash().as_str()) {
            return Err(review_required());
        }
        let result = windows::send_create(
            &plan,
            api,
            &DiskJournal,
            a.expect_plan.as_deref(),
            Some(&build),
        )?;
        finish(&result)
    })())
}

/// `fsv changes windows delete WINDOW_ID`.
pub fn windows_delete(a: ChangesWindowsDeleteArgs) -> CmdResult {
    let c = client()?;
    guard((|| {
        let plan = windows::delete_plan(c.as_ref(), a.ident)?;
        if a.dry_run {
            dry_run(plan.review());
            return Ok(());
        }
        if !a.yes || a.expect_plan.as_deref() != Some(plan.hash().as_str()) {
            return Err(review_required());
        }
        let result =
            windows::send_delete(&plan, c.as_ref(), &DiskJournal, a.expect_plan.as_deref())?;
        finish(&result)
    })())
}
