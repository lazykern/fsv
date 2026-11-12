//! Reviewed problem-to-incident association mutations
//! (`problem_incidents.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{PROBLEMS, TICKETS, parse_id};
use fsv_py::py_str;

use crate::associations::read_page;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, plan_hash};

pub const LINK: &str = "problem_link_incident";
pub const UNLINK: &str = "problem_unlink_incident";

/// Port of problem_incidents._inventory: the complete, duplicate-free
/// ticket inventory of a problem.
fn inventory(c: &dyn Api, problem: i64) -> Result<Vec<Value>> {
    let params = obj(json!({"page": 1, "per_page": 50}));
    let pages = collect_pages(
        |p, q| read_page(c, p, q),
        &format!("problems/{problem}/tickets"),
        "tickets",
        Some(&params),
        "tickets",
        MAX_DETAIL_PAGES,
    )?;
    if pages.missing.is_some() || !pages.complete() {
        return Err(refused(
            "complete problem ticket inventory unavailable; no write attempted",
        ));
    }
    let ids: Vec<String> = pages.rows.iter().map(|r| py_str(&r["id"])).collect();
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    if unique.len() != ids.len() {
        return Err(refused("problem ticket inventory has duplicate IDs"));
    }
    Ok(pages.rows)
}

fn present(rows: &[Value], ticket: i64) -> bool {
    let id = ticket.to_string();
    rows.iter().any(|r| py_str(&r["id"]) == id)
}

/// `raw.get(key, raw)` for a dict read, else `None`.
fn unwrap(raw: Value, key: &str) -> Option<Map<String, Value>> {
    match raw {
        Value::Object(mut m) => match m.remove(key) {
            Some(Value::Object(inner)) => Some(inner),
            Some(_) => None,
            None => Some(m),
        },
        _ => None,
    }
}

/// Port of problem_incidents.build_plan: a PUT `associate` (link) or a
/// DELETE of the exact ticket (unlink), bound to both records and the
/// complete inventory.
pub fn build_plan(c: &dyn Api, problem: &str, incident: &str, unlink: bool) -> Result<WritePlan> {
    if incident.to_uppercase().starts_with("SR-") {
        return Err(Error::value("incident ID must use the INC prefix, not SR"));
    }
    let pid = parse_id(problem, &PROBLEMS)?;
    let tid = parse_id(incident, &TICKETS)?;
    if pid <= 0 || tid <= 0 {
        return Err(Error::value("problem and incident IDs must be positive"));
    }
    let pr = unwrap(c.int_get(&format!("problems/{pid}"), None)?, "problem");
    let ticket = unwrap(c.int_get(&format!("tickets/{tid}"), None)?, "ticket");
    let id_of = |m: &Map<String, Value>| py_str(m.get("id").unwrap_or(&Value::Null));
    let Some(pr) = pr.filter(|m| id_of(m) == pid.to_string()) else {
        return Err(refused("problem read did not identify exact target"));
    };
    let Some(ticket) = ticket.filter(|m| id_of(m) == tid.to_string()) else {
        return Err(refused("incident read did not identify exact target"));
    };
    let display = ticket.get("human_display_id").filter(|v| !v.is_null());
    if ticket.get("type") != Some(&json!("Incident"))
        || display.is_some_and(|d| *d != json!(format!("INC-{tid}")))
    {
        return Err(refused("ticket is not a verified incident"));
    }
    let rows = inventory(c, pid)?;
    if present(&rows, tid) != unlink {
        return Err(refused(
            "incident is already in the requested association state",
        ));
    }
    let (path, payload) = if unlink {
        (
            format!("problems/{pid}/tickets/{tid}"),
            json!({"type": "unlink"}),
        )
    } else {
        (
            format!("problems/{pid}/tickets/associate"),
            json!({"item_ids": [tid]}),
        )
    };
    let context = plan_hash(&json!({"problem": pr, "ticket": ticket, "inventory": rows}));
    Ok(WritePlan {
        action: if unlink { UNLINK } else { LINK }.into(),
        target: pid,
        tenant: c.domain(),
        method: if unlink { "DELETE" } else { "PUT" }.into(),
        path,
        payload_json: fsv_py::Dumps::default().sort_keys(true).dump(&payload),
        context_hash: context,
    })
}

/// Port of problem_incidents.send: one write, then the inventory read-back.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    incident: &str,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let tid = parse_id(incident, &TICKETS)?;
    let unlink = plan.action == UNLINK;
    let rebuild = || build_plan(c, &plan.target.to_string(), &tid.to_string(), unlink);
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let linked = present(&inventory(c, plan.target)?, tid);
        Ok(obj(json!({
            "problem_id": plan.target, "incident_id": tid, "linked": linked,
            "verified": linked != unlink,
        })))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests;
