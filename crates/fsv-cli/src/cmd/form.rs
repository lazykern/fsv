//! `fsv form` (`form_cli.py`).

use clap::ArgMatches;
use serde_json::Value;

use fsv_core::resource::CHANGES;
use fsv_core::term::escape;
use fsv_core::{Error, Result};

use crate::cmd::parse;
use crate::generated::*;
use crate::pending;
use crate::util::{CmdResult, client, fail, fail_exc, uncaught};

const SNAPSHOT_VERSION: i64 = 1;

/// `form_cli._fail`: the message is plain text.
fn form_fail(msg: &str) -> Error {
    fail(&escape(msg), 2)
}

/// `formmeta.module_name`.
fn module(text: &str) -> Result<&'static str> {
    match text.trim().to_lowercase().trim_end_matches('s') {
        "change" => Ok("change"),
        "ticket" => Ok("ticket"),
        _ => Err(form_fail(&format!(
            "unsupported form {}; supported: changes, tickets",
            fsv_py::py_repr_str(text)
        ))),
    }
}

pub fn dispatch(sub: &str, m: &ArgMatches) -> CmdResult {
    match sub {
        "explain" => {
            let a: FormExplainArgs = parse(m)?;
            let mode = if a.id_.is_some() { "edit" } else { "new" };
            let module = module(&a.module)?;
            let _c = client()?;
            if let Some(id) = &a.id_ {
                if !id.chars().all(|c| c.is_ascii_digit()) || id.is_empty() {
                    fsv_core::resource::parse_id(id, &CHANGES).map_err(uncaught)?;
                }
                return pending::form_record().map_err(fail_exc);
            }
            pending::load_meta(module, mode).map(drop).map_err(fail_exc)
        }
        "fields" => {
            let a: FormFieldsArgs = parse(m)?;
            let module = module(&a.module)?;
            let _c = client()?;
            pending::load_meta(module, "new")
                .map(drop)
                .map_err(fail_exc)
        }
        "dump" => {
            let a: FormDumpArgs = parse(m)?;
            let module = module(&a.module)?;
            if a.mode != "new" && a.mode != "edit" {
                return Err(form_fail("--mode must be new or edit"));
            }
            let _c = client()?;
            pending::load_meta(module, &a.mode)
                .map(drop)
                .map_err(fail_exc)
        }
        "diff" => {
            let a: FormDiffArgs = parse(m)?;
            let old = read_snapshot(&a.snapshot)?;
            if let Some(other) = a.against.as_deref().filter(|s| !s.is_empty()) {
                read_snapshot(other)?;
                return pending::form_diff().map_err(uncaught);
            }
            let _c = client()?;
            let mode = if old.get("form_type").and_then(Value::as_str) == Some("edit_form") {
                "edit"
            } else {
                "new"
            };
            let module = old
                .get("module")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("change");
            pending::load_meta(module, mode).map(drop).map_err(fail_exc)
        }
        _ => Err(Error::other(format!("unknown command: form {sub}"))),
    }
}

fn read_snapshot(path: &str) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| form_fail(&format!("cannot read snapshot {path}: {e}")))?;
    let data: Value = serde_json::from_str(&text)
        .map_err(|e| form_fail(&format!("cannot read snapshot {path}: {e}")))?;
    if data.get("fsv_form_snapshot").and_then(Value::as_i64) != Some(SNAPSHOT_VERSION) {
        return Err(form_fail(&format!(
            "{path} is not an fsv form snapshot (make one with `fsv form dump`)"
        )));
    }
    Ok(data)
}
