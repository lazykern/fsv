//! Ticket trash: list, restore, delete forever, captured from the Trash view
//! (`ticket_trash.py`).
//!
//! * list: `GET tickets?filter=deleted&...` (the Trash filter id is `deleted`).
//! * restore: `PUT tickets/{id}/restore` (no body, 204), one request per
//!   ticket; readback `GET tickets/{id}` shows `deleted: false`.
//! * delete forever: `DELETE tickets/delete_forever?ids={id}` (204, no confirm
//!   in the UI); readback `GET tickets/{id}` answers 404. Only the single-id
//!   form is captured, so several IDs are sent as one request each, in order,
//!   stopping at the first failed or unverified ticket.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{TICKETS, parse_id};

use crate::properties::record;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, plan_hash, snapshot_hash};
use crate::watchers::positive_id;

pub const MAX_FOREVER: i64 = 50;
const LIST_INCLUDE: &str = "stats,responder,requester,ticket_states,ticket_status,group";

/// `_exact`: the ticket; a 404 refuses as not found, other API errors pass
/// through, and any other failure becomes a refusal with its message.
pub(crate) fn exact(c: &dyn Api, tid: i64) -> Result<Map<String, Value>> {
    record(c, tid).map_err(|e| match e.as_api() {
        Some(api) if api.status == 404 => refused(format!(
            "ticket {tid} not found (HTTP 404); nothing was sent"
        )),
        Some(_) => e,
        None => match e {
            Error::Exit { code: 2, .. } => e,
            other => refused(other.to_string()),
        },
    })
}

/// `_deleted`: the exact boolean deleted flag.
fn deleted(row: &Map<String, Value>, tid: i64) -> Result<bool> {
    match row.get("deleted") {
        Some(Value::Bool(flag)) => Ok(*flag),
        _ => Err(refused(format!(
            "ticket {tid}: deleted flag unavailable; nothing was sent"
        ))),
    }
}

fn ticket_id(ticket: &str) -> Result<i64> {
    positive_id(&json!(parse_id(ticket, &TICKETS)?))
}

/// Port of ticket_trash.trash: one page of the Trash view.
pub fn trash(c: &dyn Api, page: i64, per_page: i64) -> Result<Value> {
    if page < 1 || !(1..=100).contains(&per_page) {
        return Err(Error::value("page must be positive and per-page 1 to 100"));
    }
    let raw = c.int_get(
        "tickets",
        Some(obj(json!({
            "advanced_query_hash": "", "filter": "deleted", "include": LIST_INCLUDE,
            "order_by": "created_at", "order_type": "desc", "page": page,
            "per_page": per_page, "query_hash": "",
        }))),
    )?;
    let rows = match raw.get("tickets") {
        Some(Value::Array(rows)) if rows.iter().all(Value::is_object) => rows,
        _ => return Err(refused("trash list is unreadable")),
    };
    let meta = raw.get("meta").filter(|m| m.is_object());
    let field = |k: &str| meta.and_then(|m| m.get(k)).cloned().unwrap_or(Value::Null);
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut row = Map::new();
            for k in ["id", "human_display_id", "subject", "deleted", "updated_at"] {
                row.insert(k.into(), r.get(k).cloned().unwrap_or(Value::Null));
            }
            Value::Object(row)
        })
        .collect();
    let mut result = Map::new();
    result.insert("tickets".into(), Value::Array(out));
    result.insert("page".into(), json!(page));
    result.insert("has_next".into(), field("has_next"));
    result.insert("count".into(), field("count"));
    result.insert(
        "note".into(),
        json!("the trash holds every agent's deleted tickets; act on explicit IDs only"),
    );
    Ok(Value::Object(result))
}

/// Port of ticket_trash.build_restore.
pub fn build_restore(c: &dyn Api, ticket: &str) -> Result<WritePlan> {
    let tid = ticket_id(ticket)?;
    let row = exact(c, tid)?;
    if !deleted(&row, tid)? {
        return Err(refused(format!(
            "ticket {tid} is not in the trash; nothing was sent"
        )));
    }
    Ok(WritePlan {
        action: "ticket_restore".into(),
        target: tid,
        tenant: c.domain(),
        method: "PUT".into(),
        path: format!("tickets/{tid}/restore"),
        payload_json: "null".into(),
        context_hash: snapshot_hash(&json!({"ticket": row}))?,
    })
}

/// Port of ticket_trash.send_restore: verified by `deleted: false`.
pub fn send_restore(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let target = plan.target;
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let flag = record(c, target)?
            .get("deleted")
            .cloned()
            .unwrap_or(Value::Null);
        let verified = flag == Value::Bool(false);
        Ok(obj(
            json!({"ticket_id": target, "deleted": flag, "verified": verified}),
        ))
    };
    let rebuild = || build_restore(c, &target.to_string());
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

/// Port of ticket_trash.ForeverPlan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeverPlan {
    pub plans: Vec<WritePlan>,
    pub subjects_json: String,
    pub max_records: i64,
}

impl ForeverPlan {
    pub fn ids(&self) -> Vec<i64> {
        self.plans.iter().map(|p| p.target).collect()
    }

    pub fn hash(&self) -> String {
        let plans: Vec<String> = self.plans.iter().map(WritePlan::hash).collect();
        plan_hash(&json!({
            "action": "ticket_delete_forever", "plans": plans, "ids": self.ids(),
            "max_records": self.max_records,
        }))
    }

    pub fn subjects(&self) -> Vec<Value> {
        serde_json::from_str(&self.subjects_json).unwrap_or_default()
    }

    pub fn review(&self) -> Map<String, Value> {
        let requests: Vec<String> = self
            .plans
            .iter()
            .map(|p| format!("/api/_/{}", p.path))
            .collect();
        let mut out = Map::new();
        out.insert("action".into(), json!("ticket_delete_forever"));
        out.insert("method".into(), json!("DELETE"));
        out.insert("requests".into(), json!(requests));
        out.insert("ticket_ids".into(), json!(self.ids()));
        out.insert("selected_tickets".into(), Value::Array(self.subjects()));
        out.insert("plan_hash".into(), json!(self.hash()));
        out.insert(
            "warning".into(),
            json!(
                "PERMANENT: the tickets cannot be restored. One request per ticket, in order; stops at the first failed or unverified ticket. Never replay an uncertain outcome."
            ),
        );
        out
    }
}

/// Port of ticket_trash.build_forever: explicit trashed tickets under a
/// required `--max-records` bound.
pub fn build_forever(
    c: &dyn Api,
    tickets: &[String],
    max_records: Option<i64>,
) -> Result<ForeverPlan> {
    let Some(max_records) = max_records.filter(|n| (1..=MAX_FOREVER).contains(n)) else {
        return Err(Error::value(format!(
            "delete-forever requires --max-records N (1 to {MAX_FOREVER})"
        )));
    };
    let ids = tickets
        .iter()
        .map(|t| ticket_id(t))
        .collect::<Result<Vec<_>>>()?;
    let distinct: std::collections::HashSet<i64> = ids.iter().copied().collect();
    if ids.is_empty() || distinct.len() != ids.len() {
        return Err(Error::value(
            "give one or more distinct explicit ticket IDs",
        ));
    }
    if ids.len() as i64 > max_records {
        return Err(Error::value(
            "more IDs than --max-records; refusing the whole plan",
        ));
    }
    let mut plans = Vec::with_capacity(ids.len());
    let mut subjects = Vec::with_capacity(ids.len());
    for tid in ids {
        let row = exact(c, tid)?;
        if !deleted(&row, tid)? {
            return Err(refused(format!(
                "ticket {tid} is not in the trash; delete it first; nothing was sent"
            )));
        }
        subjects.push(json!({
            "id": tid,
            "subject": row.get("subject").cloned().unwrap_or(Value::Null),
        }));
        plans.push(WritePlan {
            action: "ticket_delete_forever".into(),
            target: tid,
            tenant: c.domain(),
            method: "DELETE".into(),
            path: format!("tickets/delete_forever?ids={tid}"),
            payload_json: "null".into(),
            context_hash: snapshot_hash(&json!({"ticket": row}))?,
        });
    }
    Ok(ForeverPlan {
        plans,
        subjects_json: fsv_py::Dumps::default()
            .sort_keys(true)
            .dump(&Value::Array(subjects)),
        max_records,
    })
}

/// `_gone`: the exact ticket answers 404. Other API errors are "not gone";
/// anything else fails the readback.
fn gone(c: &dyn Api, tid: i64) -> Result<bool> {
    match record(c, tid) {
        Ok(_) => Ok(false),
        Err(e) => match e.as_api() {
            Some(api) => Ok(api.status == 404),
            None => Err(e),
        },
    }
}

/// Port of ticket_trash.send_forever.
pub fn send_forever(
    plan: &ForeverPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    confirm_destructive: bool,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    if reviewed != hash {
        return Err(refused("full review hash changed; review again"));
    }
    if !confirm_destructive {
        return Err(refused(
            "delete-forever requires --confirm-destructive after inspecting the printed ID list",
        ));
    }
    let ids = plan.ids();
    let mut tickets = Map::new();
    for (index, sub) in plan.plans.iter().enumerate() {
        let tid = sub.target;
        let verify = |_: &Value| -> Result<Map<String, Value>> {
            let gone = gone(c, tid)?;
            Ok(obj(
                json!({"ticket_id": tid, "not_found_after": gone, "verified": gone}),
            ))
        };
        let rebuild = || build_forever(c, &[tid.to_string()], Some(1)).map(|p| p.plans[0].clone());
        let result = match execute(sub, c, journal, Some(&sub.hash()), &rebuild, &verify) {
            Ok(r) => r,
            Err(Error::Exit { code: 2, message }) => {
                json!({"verified": false, "error": message})
            }
            Err(e) => return Err(e),
        };
        let mut kept = Map::new();
        for k in ["verified", "error", "warning"] {
            if let Some(v) = result.get(k) {
                kept.insert(k.into(), v.clone());
            }
        }
        tickets.insert(tid.to_string(), Value::Object(kept));
        if !result.get("verified").is_some_and(fsv_py::truthy) {
            let mut out = Map::new();
            out.insert(
                "status".into(),
                json!(if index > 0 { "partial" } else { "failed" }),
            );
            out.insert("verified".into(), json!(false));
            out.insert("plan_hash".into(), json!(hash));
            out.insert("ticket_ids".into(), json!(ids));
            out.insert("tickets".into(), Value::Object(tickets));
            out.insert("stopped_at".into(), json!(tid));
            out.insert("not_attempted".into(), json!(ids[index + 1..]));
            out.insert(
                "warning".into(),
                json!(
                    "Stopped at the first failed or unverified ticket; inspect it and the journal; never replay."
                ),
            );
            return Ok(Value::Object(out));
        }
    }
    Ok(json!({
        "status": "submitted", "verified": true, "plan_hash": hash, "ticket_ids": ids,
        "tickets": tickets,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use fsv_core::ApiError;

    use super::*;
    use crate::service::testing::{Call, FnApi, MemJournal};

    struct World {
        rows: BTreeMap<i64, bool>,
        writes: Vec<(String, String, Option<Vec<u8>>)>,
        lists: Vec<Map<String, Value>>,
        fail: BTreeMap<i64, u16>,
        effect: bool,
    }

    fn tenant() -> (Arc<Mutex<World>>, FnApi) {
        let state = Arc::new(Mutex::new(World {
            rows: [(31, true), (32, true), (33, false)].into_iter().collect(),
            writes: Vec::new(),
            lists: Vec::new(),
            fail: BTreeMap::new(),
            effect: true,
        }));
        let s = Arc::clone(&state);
        let c = FnApi::new(move |call: &Call| {
            let mut s = s.lock().unwrap();
            if call.method == "GET" {
                if call.path == "tickets" {
                    s.lists.push(call.params.clone().unwrap_or_default());
                    let rows: Vec<Value> = s
                        .rows
                        .iter()
                        .filter(|(_, d)| **d)
                        .map(|(t, _)| json!({"id": t, "subject": format!("Acme {t}"), "deleted": true}))
                        .collect();
                    let n = rows.len();
                    return Ok(
                        json!({"tickets": rows, "meta": {"offset": 0, "has_next": false, "count": n, "editable_ids": []}}),
                    );
                }
                let tid: i64 = call.path.split('/').nth(1).unwrap().parse().unwrap();
                return match s.rows.get(&tid) {
                    None => Err(ApiError::new(404, json!({})).into()),
                    Some(d) => Ok(
                        json!({"ticket": {"id": tid, "subject": format!("Acme {tid}"), "deleted": d}}),
                    ),
                };
            }
            s.writes
                .push((call.method.clone(), call.path.clone(), call.raw.clone()));
            if let Some(ids) = call.path.strip_prefix("tickets/delete_forever?ids=") {
                let tid: i64 = ids.parse().unwrap();
                if let Some(status) = s.fail.get(&tid) {
                    return Err(ApiError::new(*status, json!({"errors": "synthetic"})).into());
                }
                if s.effect {
                    s.rows.remove(&tid);
                }
                return Ok(Value::Null);
            }
            let tid: i64 = call.path.split('/').nth(1).unwrap().parse().unwrap();
            s.rows.insert(tid, false);
            Ok(Value::Null)
        });
        (state, c)
    }

    fn msg(r: Result<impl std::fmt::Debug>) -> String {
        r.unwrap_err().to_string()
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn trash_list_uses_deleted_filter() {
        let (s, c) = tenant();
        let out = trash(&c, 1, 30).unwrap();
        let got: Vec<&Value> = out["tickets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| &r["id"])
            .collect();
        assert_eq!(got, [&json!(31), &json!(32)]);
        let q = &s.lock().unwrap().lists[0];
        assert_eq!(q["filter"], json!("deleted"));
        assert_eq!(q["order_by"], json!("created_at"));
        assert!(msg(trash(&c, 0, 30)).contains("page must be positive"));
        assert!(msg(trash(&c, 1, 101)).contains("per-page 1 to 100"));
    }

    #[test]
    fn restore_bodyless_put_and_deleted_flag_readback() {
        let (s, c) = tenant();
        let j = MemJournal::default();
        let p = build_restore(&c, "INC-32").unwrap();
        assert_eq!(
            (p.method.as_str(), p.path.as_str(), p.payload()),
            ("PUT", "tickets/32/restore", Value::Null)
        );
        let r = send_restore(&p, &c, &j, Some(&p.hash())).unwrap();
        assert_eq!(r["verified"], json!(true));
        assert_eq!(r["deleted"], json!(false));
        assert_eq!(s.lock().unwrap().writes[0].2, None);
        assert!(msg(build_restore(&c, "32")).contains("not in the trash"));
        assert!(msg(build_restore(&c, "99")).contains("not found"));
    }

    #[test]
    fn delete_forever_one_request_per_id_with_404_readback() {
        let (s, c) = tenant();
        let j = MemJournal::default();
        let two = ids(&["31", "32"]);
        assert!(msg(build_forever(&c, &two, None)).contains("max-records"));
        assert!(msg(build_forever(&c, &two, Some(1))).contains("more IDs"));
        assert!(msg(build_forever(&c, &ids(&["31", "33"]), Some(5))).contains("not in the trash"));
        let p = build_forever(&c, &ids(&["INC-31", "32"]), Some(2)).unwrap();
        assert_eq!(
            p.review()["requests"],
            json!([
                "/api/_/tickets/delete_forever?ids=31",
                "/api/_/tickets/delete_forever?ids=32"
            ])
        );
        assert!(
            msg(send_forever(&p, &c, &j, Some(&p.hash()), false)).contains("confirm-destructive")
        );
        assert!(s.lock().unwrap().writes.is_empty());
        let r = send_forever(&p, &c, &j, Some(&p.hash()), true).unwrap();
        assert_eq!(r["verified"], json!(true));
        let keys: Vec<&String> = r["tickets"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["31", "32"]);
        let w = s.lock().unwrap();
        assert!(
            w.writes
                .iter()
                .all(|(m, p, _)| m == "DELETE" && p.starts_with("tickets/delete_forever?"))
        );
        assert_eq!(w.writes.len(), 2);
    }

    #[test]
    fn delete_forever_stops_at_first_failure_and_unverified() {
        let (s, c) = tenant();
        let j = MemJournal::default();
        s.lock().unwrap().fail.insert(31, 500);
        let p = build_forever(&c, &ids(&["31", "32"]), Some(2)).unwrap();
        let r = send_forever(&p, &c, &j, Some(&p.hash()), true).unwrap();
        assert_eq!(r["status"], json!("failed"));
        assert_eq!(r["not_attempted"], json!([32]));
        assert_eq!(s.lock().unwrap().writes.len(), 1);
        {
            let mut w = s.lock().unwrap();
            w.fail.clear();
            w.effect = false;
        }
        let q = build_forever(&c, &ids(&["32"]), Some(1)).unwrap();
        let r = send_forever(&q, &c, &j, Some(&q.hash()), true).unwrap();
        assert_eq!(r["verified"], json!(false));
        assert_eq!(s.lock().unwrap().writes.len(), 2);
    }
}
