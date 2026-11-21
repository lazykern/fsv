//! `changes update --due-by` on the reviewed contract
//! (`cli._change_due_by_update`).

use serde_json::{Map, Value, json};

use fsv_api::change_due_by::{self as flow, DueByError};
use fsv_core::Error;
use fsv_core::output::emit_json;

use crate::util::{CmdResult, client, plain_fail, uncaught};

pub struct DueByOpts<'a> {
    pub dry_run: bool,
    pub yes: bool,
    pub expect_plan: Option<&'a str>,
    pub json_out: bool,
}

fn pretty(v: &Value) -> String {
    fsv_py::Dumps::default()
        .indent(2)
        .ensure_ascii(false)
        .dump(v)
}

pub fn change_due_by_update(cid: i64, due_by: &str, o: DueByOpts<'_>) -> CmdResult {
    let c = client()?;
    let journal = fsv_api::service::DiskJournal;
    let built = flow::parse_due_by(due_by, Some(c.as_ref()))
        .map_err(DueByError::from)
        .and_then(|_| flow::build_plan(c.as_ref(), &journal, cid, due_by));
    let plan = match built {
        Ok(plan) => plan,
        Err(DueByError::NothingToChange(message)) => {
            if o.expect_plan.is_some() {
                let said = fsv_core::journal::stale_plan_refusal(o.expect_plan, &[flow::ACTION])
                    .unwrap_or_else(|| {
                        "nothing to change; the reviewed plan does not exist. Nothing was sent"
                            .into()
                    });
                return Err(plain_fail(&said, 2));
            }
            if o.json_out {
                emit_json(&json!({"status": "nothing_to_change", "target": cid}));
            } else {
                println!("{message}");
            }
            return Ok(());
        }
        Err(DueByError::Failed(e)) => {
            return Err(match e {
                Error::Value(m) => plain_fail(&m, 2),
                Error::Exit { code: 2, message } if !message.is_empty() => plain_fail(&message, 2),
                Error::Api(api) => plain_fail(
                    &format!(
                        "could not read change {cid} (HTTP {}); nothing was sent",
                        api.status
                    ),
                    2,
                ),
                other => uncaught(other),
            });
        }
    };
    let review = flow::review(&plan).map_err(|e| plain_fail(&e.to_string(), 2))?;
    if o.dry_run {
        let mut out = Map::new();
        out.insert("status".into(), json!("dry_run"));
        out.extend(review);
        println!("{}", pretty(&Value::Object(out)));
        return Ok(());
    }
    let hash = plan.hash();
    let sent = (|| -> Result<Value, Error> {
        if o.expect_plan.is_some_and(|e| e != hash) {
            return Err(Error::exit(
                2,
                fsv_core::journal::stale_plan_refusal(o.expect_plan, &[flow::ACTION])
                    .unwrap_or_else(|| "the change or the due date changed since review (or a partial hash); review again with --dry-run. Nothing was sent".into()),
            ));
        }
        if !o.yes || o.expect_plan != Some(hash.as_str()) {
            return Err(Error::exit(
                2,
                "review with --dry-run, then pass --yes and the full --expect-plan hash. Nothing was sent",
            ));
        }
        flow::send(&plan, c.as_ref(), &journal, Some(&hash))
    })();
    let result = match sent {
        Ok(result) => result,
        Err(e) => {
            let message = match &e {
                Error::Exit { code: 2, message } if !message.is_empty() => message.clone(),
                _ => return Err(uncaught(e)),
            };
            if o.json_out {
                emit_json(&json!({
                    "status": "error", "plan_hash": hash, "verified": false, "error": message,
                }));
            }
            return Err(plain_fail(&message, 2));
        }
    };
    println!("{}", pretty(&result));
    if !result.get("verified").is_some_and(fsv_py::truthy) {
        return Err(Error::exit(4, ""));
    }
    Ok(())
}
