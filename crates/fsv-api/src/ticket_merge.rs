//! Reviewed, irreversible ticket merges using the accepted complete-merge
//! contract (`ticket_merge.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_py::py_str;

use crate::formmeta::{MetaStore, is_status, load_meta};
use crate::properties::{dumps_sorted, record};
use crate::readback::{same_text, submitted_value_matches};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, load_conversations, obj};

pub const ACTION: &str = "ticket_merge";
pub const PATH: &str = "merge_tickets/complete_merge";

const WARNING: &str = "Irreversible: secondary tickets are closed and their conversations move to the surviving primary. Tenant workflows may notify people. Unknown outcomes must not be resent.";

fn ticket_id(value: &str) -> Result<i64> {
    let id = parse_id(value, &TICKETS)?;
    if id <= 0 {
        return Err(Error::value("ticket ID must be positive"));
    }
    Ok(id)
}

/// Complete conversation history with unique positive numeric IDs.
fn history(c: &dyn Api, ticket: i64) -> Result<Vec<Value>> {
    let pages = load_conversations(c, ticket)?;
    if pages.missing.is_some()
        || !pages.complete()
        || pages.meta.get("partial").is_some_and(fsv_py::truthy)
    {
        return Err(refused(
            "complete conversation history is required; no merge attempted",
        ));
    }
    let ids: Vec<String> = pages
        .rows
        .iter()
        .map(|r| py_str(r.get("id").unwrap_or(&Value::Null)))
        .collect();
    let bad = pages.rows.iter().zip(&ids).any(|(r, s)| {
        r.get("id").is_some_and(Value::is_boolean)
            || s.is_empty()
            || !s.chars().all(|ch| ch.is_ascii_digit())
            || s.trim_start_matches('0').is_empty()
    });
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    if bad || unique.len() != ids.len() {
        return Err(refused(
            "conversation history lacks unique IDs; no merge attempted",
        ));
    }
    Ok(pages.rows)
}

/// The unique visible runtime Closed status id and the metadata hash.
fn closed(c: &dyn Api, store: &dyn MetaStore, ticket: i64) -> Result<(i64, String)> {
    let meta = load_meta(c, store, "ticket", "edit", true, &json!(ticket))?;
    let spec = meta.spec();
    let fields: Vec<_> = spec.fields().iter().filter(|f| is_status(f)).collect();
    let choices: Vec<_> = fields
        .iter()
        .flat_map(|f| f.choices.iter())
        .filter(|ch| ch.visible && casefold(&ch.label) == "closed")
        .collect();
    let id = match (fields.len(), choices.as_slice()) {
        (1, [one]) => one
            .id
            .as_ref()
            .filter(|v| v.is_i64() || v.is_u64())
            .and_then(Value::as_i64)
            .filter(|i| *i > 0),
        _ => None,
    };
    let Some(id) = id else {
        return Err(refused("a unique runtime Closed status ID is required"));
    };
    Ok((id, meta.fingerprint().hash))
}

/// `build_plan(client, primary, secondaries, note_primary=..., ...)`: one
/// `complete_merge` POST bound to every record, history and Closed status.
#[allow(clippy::too_many_arguments)]
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    primary: &str,
    secondaries: &[String],
    note_primary: &str,
    note_secondary: &str,
    private_primary: bool,
    private_secondary: bool,
) -> Result<WritePlan> {
    let target = ticket_id(primary)?;
    if secondaries.is_empty() {
        return Err(Error::value("at least one secondary ticket is required"));
    }
    let ids = secondaries
        .iter()
        .map(|s| ticket_id(s))
        .collect::<Result<Vec<_>>>()?;
    let unique: std::collections::HashSet<&i64> = ids.iter().collect();
    if ids.contains(&target) || unique.len() != ids.len() {
        return Err(Error::value(
            "primary and secondary tickets must be distinct",
        ));
    }
    if [note_primary, note_secondary]
        .iter()
        .any(|n| strip(n).is_empty() || n.contains('\0'))
    {
        return Err(Error::value("both merge notes must be nonempty text"));
    }
    let all: Vec<i64> = std::iter::once(target).chain(ids.iter().copied()).collect();
    let mut records = Map::new();
    for i in &all {
        records.insert(i.to_string(), Value::Object(record(c, *i)?));
    }
    let mut histories = Map::new();
    for i in &all {
        histories.insert(i.to_string(), Value::Array(history(c, *i)?));
    }
    let mut closed_ids = Map::new();
    for i in &ids {
        let (id, hash) = closed(c, store, *i)?;
        closed_ids.insert(i.to_string(), json!([id, hash]));
    }
    let context = json!({"records": records, "histories": histories, "closed": closed_ids});
    let payload = json!({
        "target": {"ticket_id": target, "note": note_primary, "is_private": private_primary},
        "source": {"ticket_ids": ids, "note": note_secondary, "is_private": private_secondary},
    });
    Ok(WritePlan {
        action: ACTION.into(),
        target,
        tenant: c.domain(),
        method: "POST".into(),
        path: PATH.into(),
        payload_json: dumps_sorted(&payload),
        context_hash: dumps_sorted(&context),
    })
}

/// `review(plan)`: the write review plus the surviving and merged tickets.
pub fn review(plan: &WritePlan) -> Map<String, Value> {
    let mut out = plan.review();
    out.insert("surviving_ticket".into(), json!(plan.target));
    out.insert(
        "merged_tickets".into(),
        plan.payload()["source"]["ticket_ids"].clone(),
    );
    out.insert("warning".into(), json!(WARNING));
    out
}

fn ids_of(rows: &[Value]) -> Result<Vec<String>> {
    rows.iter()
        .map(|r| {
            r.get("id")
                .map(py_str)
                .ok_or_else(|| Error::other("conversation row without id"))
        })
        .collect()
}

/// A new history row carries the note text with exactly this visibility.
fn has_note(rows: &[&Value], note: &Value, private: &Value) -> bool {
    rows.iter().any(|r| {
        same_text(r.get("body").unwrap_or(&Value::Null), note)
            && r.get("private").is_some_and(Value::is_boolean)
            && r.get("private") == Some(private)
    })
}

/// `send(plan, client, reviewed_hash=...)`: one POST, then an independent
/// readback of every ticket and history.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let payload = plan.payload();
    let (target, source) = (&payload["target"], &payload["source"]);
    let before: Value = serde_json::from_str(&plan.context_hash).unwrap_or(Value::Null);
    let secondaries: Vec<String> = source["ticket_ids"]
        .as_array()
        .map(|a| a.iter().map(py_str).collect())
        .unwrap_or_default();
    let rebuild = || {
        build_plan(
            c,
            store,
            &py_str(&target["ticket_id"]),
            &secondaries,
            target["note"].as_str().unwrap_or_default(),
            source["note"].as_str().unwrap_or_default(),
            target["is_private"] == true,
            source["is_private"] == true,
        )
    };
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let t = plan.target.to_string();
        let primary_record = record(c, plan.target)?;
        let primary_history = history(c, plan.target)?;
        let primary_ids = ids_of(&primary_history)?;
        let prior = ids_of(
            before["histories"][&t]
                .as_array()
                .map_or(&[], Vec::as_slice),
        )?;
        let new_primary: Vec<&Value> = primary_history
            .iter()
            .zip(&primary_ids)
            .filter(|(_, id)| !prior.contains(id))
            .map(|(r, _)| r)
            .collect();
        let primary_note = has_note(&new_primary, &target["note"], &target["is_private"]);
        let unchanged = match (
            primary_record.get("status"),
            before["records"][&t].get("status"),
        ) {
            (Some(now), Some(was)) => submitted_value_matches(now, was),
            _ => false,
        };
        let mut checks = Map::new();
        for sid in &secondaries {
            let id: i64 = sid.parse().map_err(|_| Error::other("bad ticket id"))?;
            let rec = record(c, id)?;
            let hist = history(c, id)?;
            let hist_ids = ids_of(&hist)?;
            let original = ids_of(
                before["histories"][sid]
                    .as_array()
                    .map_or(&[], Vec::as_slice),
            )?;
            let new: Vec<&Value> = hist
                .iter()
                .zip(&hist_ids)
                .filter(|(_, i)| !original.contains(i))
                .map(|(r, _)| r)
                .collect();
            let closed_now = rec
                .get("status")
                .is_some_and(|s| submitted_value_matches(s, &before["closed"][sid][0]));
            checks.insert(
                sid.clone(),
                json!({
                    "closed": closed_now,
                    "conversations_moved": original.iter().all(|i| primary_ids.contains(i)),
                    "source_note": has_note(&new, &source["note"], &source["is_private"]),
                }),
            );
        }
        let all_checks = checks
            .values()
            .all(|v| v.as_object().is_some_and(|m| m.values().all(|x| x == true)));
        Ok(obj(json!({
            "surviving_ticket": plan.target, "primary_status_unchanged": unchanged,
            "primary_note": primary_note, "secondaries": checks,
            "verified": unchanged && primary_note && all_checks,
        })))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests;
