//! Reviewed edits and deletes of exact ticket conversation notes
//! (`ticket_note_mutation.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_py::{py_str, truthy};

use crate::notes::{self, BodyFormat};
use crate::properties::dumps_sorted;
use crate::readback::same_text;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, load_conversations, snapshot_hash};

pub const EDIT: &str = "ticket_note_edit";
pub const DELETE: &str = "ticket_note_delete";

/// Port of ticket_note_mutation._note: the exact note in a complete history.
fn note(c: &dyn Api, ticket: i64, note_id: i64) -> Result<Map<String, Value>> {
    let pages = load_conversations(c, ticket)?;
    if pages.missing.is_some() || pages.meta.get("partial").is_some_and(truthy) {
        return Err(refused(
            "complete conversation history unavailable; no write attempted",
        ));
    }
    let id = note_id.to_string();
    let matches: Vec<&Value> = pages
        .rows
        .iter()
        .filter(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == id)
        .collect();
    let [Value::Object(row)] = matches.as_slice() else {
        return Err(refused("exact note not found or ambiguous"));
    };
    let source_is_note = row
        .get("source")
        .is_some_and(|s| (s.is_i64() || s.is_u64()) && s.as_i64() == Some(2));
    if !source_is_note || !row.get("private").is_some_and(Value::is_boolean) {
        return Err(refused("conversation is not a verified note"));
    }
    Ok(row.clone())
}

fn before_hash(row: &Map<String, Value>) -> Result<String> {
    snapshot_hash(&Value::Object(row.clone()))
}

fn note_id_of(plan: &WritePlan) -> i64 {
    plan.path
        .rsplit('/')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Port of ticket_note_mutation.build_plan: a PUT of the escaped body when
/// `body` is given, else a bodyless DELETE, bound to the note's prior state.
pub fn build_plan(
    c: &dyn Api,
    ticket_id: &str,
    note_id: &str,
    body: Option<&str>,
) -> Result<WritePlan> {
    let cid = parse_id(ticket_id, &TICKETS)?;
    let digits = !note_id.is_empty() && note_id.bytes().all(|b| b.is_ascii_digit());
    let nid = digits.then(|| note_id.parse::<i64>().ok()).flatten();
    let (true, Some(nid)) = (cid > 0, nid.filter(|n| *n > 0)) else {
        return Err(Error::value("ticket and conversation IDs must be positive"));
    };
    let before = note(c, cid, nid)?;
    let tenant = c.domain();
    let payload = match body {
        None => Value::Null,
        Some(b) => {
            let html =
                notes::build_plan(&cid.to_string(), b, false, &[], BodyFormat::Plain, &tenant)?
                    .body_html;
            json!({"body": html, "conversation_from_summary": false})
        }
    };
    Ok(WritePlan {
        action: if body.is_none() { DELETE } else { EDIT }.into(),
        target: cid,
        tenant,
        method: if body.is_none() { "DELETE" } else { "PUT" }.into(),
        path: format!("conversations/{nid}"),
        payload_json: dumps_sorted(&payload),
        context_hash: before_hash(&before)?,
    })
}

/// Port of ticket_note_mutation.review: the write review plus the prior note,
/// refused when the note changed since the plan.
pub fn review(plan: &WritePlan, c: &dyn Api) -> Result<Map<String, Value>> {
    let before = note(c, plan.target, note_id_of(plan))?;
    if before_hash(&before)? != plan.context_hash {
        return Err(refused("note changed since review; review again"));
    }
    let mut out = plan.review();
    out.insert("ticket_id".into(), json!(plan.target));
    out.insert("before".into(), Value::Object(before));
    Ok(out)
}

/// Port of ticket_note_mutation.send: one PUT or DELETE, then an exact-ID
/// read-back of the complete history.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let nid = note_id_of(plan);
    let payload = plan.payload();
    let body_html = payload.get("body").cloned().unwrap_or(Value::Null);
    let rebuild = || -> Result<WritePlan> {
        let before = note(c, plan.target, nid)?;
        Ok(WritePlan {
            context_hash: before_hash(&before)?,
            tenant: c.domain(),
            ..plan.clone()
        })
    };
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let pages = load_conversations(c, plan.target)?;
        let complete = pages.missing.is_none() && !pages.meta.get("partial").is_some_and(truthy);
        let id = nid.to_string();
        let found: Vec<&Value> = pages
            .rows
            .iter()
            .filter(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == id)
            .collect();
        if plan.method == "DELETE" {
            return Ok(crate::service::obj(json!({
                "conversation_id": nid, "history_complete": complete,
                "verified": complete && found.is_empty(),
            })));
        }
        let status = match found.as_slice() {
            [row] if row.get("body").is_some() => {
                if same_text(&row["body"], &body_html) {
                    "matched"
                } else {
                    "mismatched"
                }
            }
            _ => "unavailable",
        };
        Ok(crate::service::obj(json!({
            "conversation_id": nid, "history_complete": complete, "body": status,
            "verified": complete && status == "matched",
        })))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests;
