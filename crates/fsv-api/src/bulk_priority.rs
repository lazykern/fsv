//! Captured explicit-ticket bulk priority request; no other bulk operations
//! (`bulk_priority.py`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_forms::LookupResolver;

use crate::formmeta::MetaStore;
use crate::properties::{self, PropertyArgs};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, plan_hash};

/// Port of fsv.bulk_priority.BulkPriorityPlan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkPriorityPlan {
    pub write: WritePlan,
    pub label: String,
    pub before_json: String,
}

impl BulkPriorityPlan {
    /// Port of fsv.bulk_priority.BulkPriorityPlan.hash.
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    /// Port of fsv.bulk_priority.BulkPriorityPlan.review.
    pub fn review(&self) -> Map<String, Value> {
        let mut review = self.write.review();
        review.insert(
            "before".into(),
            serde_json::from_str(&self.before_json).unwrap_or(Value::Null),
        );
        review.insert("priority_label".into(), json!(self.label));
        review.insert(
            "warning".into(),
            json!("Explicit tickets only. Bulk actions may notify people; partial per-ticket outcomes are possible."),
        );
        review
    }
}

/// Port of fsv.bulk_priority.build_plan: one property plan per explicit
/// ticket, all resolving `priority` to the same enum id.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    ids: &[String],
    priority: &str,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<BulkPriorityPlan> {
    if ids.is_empty() || ids.len() > 100 {
        return Err(Error::value(
            "specify between 1 and 100 explicit ticket IDs",
        ));
    }
    let cids = ids
        .iter()
        .map(|v| parse_id(v, &TICKETS))
        .collect::<Result<Vec<i64>>>()?;
    let mut unique = cids.clone();
    unique.sort_unstable();
    unique.dedup();
    if cids.iter().any(|c| *c < 1) || unique.len() != cids.len() {
        return Err(Error::value("ticket IDs must be positive and unique"));
    }
    let mut answers = Map::new();
    answers.insert("priority".into(), json!(priority));
    let args = PropertyArgs {
        answers: Some(&answers),
        ..Default::default()
    };
    let plans = cids
        .iter()
        .map(|cid| properties::build_plan_with(c, store, &cid.to_string(), &args, resolver.clone()))
        .collect::<Result<Vec<_>>>()?;
    let mut enum_ids: Vec<Value> = Vec::new();
    for p in &plans {
        let id = p.payload().get("priority").cloned().unwrap_or(Value::Null);
        if !enum_ids.contains(&id) {
            enum_ids.push(id);
        }
    }
    let priority_id = match enum_ids.as_slice() {
        [one] if !one.is_null() => one.clone(),
        _ => {
            return Err(refused(
                "ticket metadata resolves different priorities; separate reviews are required",
            ));
        }
    };
    let body = json!({"priority": priority_id, "ids": cids, "reply": {}});
    let hashes: Vec<String> = plans
        .iter()
        .map(properties::AnsweredPropertyPlan::hash)
        .collect();
    let write = WritePlan {
        action: "ticket_bulk_priority".into(),
        target: cids[0],
        tenant: c.domain(),
        method: "PUT".into(),
        path: "bulk_ticket_actions/update_multiple".into(),
        payload_json: fsv_py::Dumps::default().sort_keys(true).dump(&body),
        context_hash: plan_hash(&json!(hashes)),
    };
    let before: Vec<Value> = plans
        .iter()
        .map(|p| {
            json!({"ticket_id": p.plan.ticket_id,
                   "priority": p.plan.before().get("priority").cloned().unwrap_or(Value::Null)})
        })
        .collect();
    Ok(BulkPriorityPlan {
        write,
        label: priority.to_string(),
        before_json: fsv_py::Dumps::default().dump(&json!(before)),
    })
}

/// Port of fsv.bulk_priority.send: one bulk PUT, then each ticket read back.
pub fn send(
    plan: &BulkPriorityPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    store: &dyn MetaStore,
    reviewed_hash: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<Value> {
    let payload = plan.write.payload();
    let ids: Vec<Value> = payload
        .get("ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let expected = payload.get("priority").cloned().unwrap_or(Value::Null);
    let id_args: Vec<String> = ids.iter().map(fsv_py::py_str).collect();
    let rebuild = || build_plan(c, store, &id_args, &plan.label, resolver.clone()).map(|p| p.write);
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let mut fields = Map::new();
        for cid in &ids {
            let state = match cid.as_i64().map(|id| properties::record(c, id)) {
                Some(Ok(row)) => match row.get("priority") {
                    None => "unavailable",
                    Some(p) if *p == expected => "matched",
                    Some(_) => "mismatched",
                },
                _ => "unavailable",
            };
            fields.insert(fsv_py::py_str(cid), json!(state));
        }
        let verified = fields.values().all(|v| v == "matched");
        Ok(obj(json!({"tickets": fields, "verified": verified})))
    };
    let bounded = |r: &Value| Ok(crate::readback::bounded_readback(|| verify(r)));
    execute(&plan.write, c, journal, reviewed_hash, &rebuild, &bounded)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::formmeta::MemStore;
    use crate::service::testing::*;
    use fsv_core::ApiError;

    struct World {
        rows: Map<String, Value>,
        failure: Option<u16>,
        partial: bool,
    }

    fn fields(cid: i64, high: i64) -> Value {
        json!({"ticket_fields": [
            form_field(cid + 9000000000, "subject", "Subject", "default_subject", 1, json!({})),
            form_field(cid + 9000000001, "priority", "Priority", "default_priority", 2,
                       json!({"choices": [{"id": 1, "value": "Low"}, {"id": high, "value": "High"}]})),
        ]})
    }

    fn client(w: &Arc<Mutex<World>>, domain: &str, cid: i64, high: i64) -> FnApi {
        let w = w.clone();
        let mut api = FnApi::new(move |call| {
            let mut w = w.lock().unwrap();
            let path = call.path.as_str();
            if call.method == "PUT" {
                assert_eq!(path, "bulk_ticket_actions/update_multiple");
                if let Some(s) = w.failure {
                    return Err(ApiError::new(s, json!({"error": "synthetic denial"})).into());
                }
                let body = call.body.clone().unwrap();
                let ids = body["ids"].as_array().unwrap().clone();
                let n = if w.partial { 1 } else { ids.len() };
                for id in &ids[..n] {
                    w.rows[&id.to_string()]["priority"] = body["priority"].clone();
                }
                return Ok(json!({}));
            }
            if path == "ticket_form_fields" {
                return Ok(fields(cid, high));
            }
            if path.ends_with("business_rules") {
                return Ok(json!({"business_rules": []}));
            }
            let id = path.strip_prefix("tickets/").unwrap();
            Ok(json!({"ticket": w.rows[id]}))
        });
        api.domain = domain.into();
        api
    }

    fn world(cid: i64) -> Arc<Mutex<World>> {
        let mut rows = Map::new();
        rows.insert(
            cid.to_string(),
            json!({"id": cid, "subject": "Acme one", "priority": 1}),
        );
        rows.insert(
            (cid + 1).to_string(),
            json!({"id": cid + 1, "subject": "Globex two", "priority": 1}),
        );
        Arc::new(Mutex::new(World {
            rows,
            failure: None,
            partial: false,
        }))
    }

    const WIRES: [(&str, i64, i64); 2] = [
        ("acme.example.com", 123, 87),
        ("globex.example.com", 742, 26),
    ];

    fn ids(cid: i64) -> Vec<String> {
        vec![cid.to_string(), (cid + 1).to_string()]
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    /// `test_bulk_priority.py::test_exact_wire_and_fresh_per_target_readback`.
    #[test]
    fn exact_wire_and_per_target_readback() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        for (domain, cid, high) in WIRES {
            let w = world(cid);
            let c = client(&w, domain, cid, high);
            let store = MemStore::default();
            let p = build_plan(&c, &store, &ids(cid), "High", None).unwrap();
            assert!(c.writes().is_empty());
            assert_eq!(
                p.write.payload(),
                json!({"priority": high, "ids": [cid, cid + 1], "reply": {}})
            );
            assert_eq!(
                p.hash(),
                oracle["bulk_priority"][domain]["hash"],
                "{domain}"
            );
            assert_eq!(
                Value::Object(p.review()),
                oracle["bulk_priority"][domain]["review"]
            );
            let out = send(
                &p,
                &c,
                &MemJournal::default(),
                &store,
                Some(&p.hash()),
                None,
            )
            .unwrap();
            assert_eq!(out["verified"], true, "{out}");
            assert!(
                out["tickets"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|v| v == "matched")
            );
            let writes = c.writes();
            assert_eq!(writes.len(), 1);
            assert_eq!(writes[0].body.clone().unwrap(), p.write.payload());
        }
    }

    /// `test_partial_is_not_success_and_is_not_resent`.
    #[test]
    fn partial_is_not_success() {
        let (domain, cid, high) = WIRES[0];
        let w = world(cid);
        w.lock().unwrap().partial = true;
        let c = client(&w, domain, cid, high);
        let (store, j) = (MemStore::default(), MemJournal::default());
        let p = build_plan(&c, &store, &ids(cid), "High", None).unwrap();
        let out = send(&p, &c, &j, &store, Some(&p.hash()), None).unwrap();
        assert_eq!(out["verified"], false);
        assert_eq!(out["tickets"][(cid + 1).to_string()], "mismatched");
        assert!(err(send(&p, &c, &j, &store, Some(&p.hash()), None)).contains("already sent at"));
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_bulk_no_error_replay`.
    #[test]
    fn no_error_replay() {
        for status in [403, 429, 500] {
            let (domain, cid, high) = WIRES[1];
            let w = world(cid);
            let c = client(&w, domain, cid, high);
            let (store, j) = (MemStore::default(), MemJournal::default());
            let p = build_plan(&c, &store, &ids(cid), "High", None).unwrap();
            w.lock().unwrap().failure = Some(status);
            for _ in 0..2 {
                assert!(err(send(&p, &c, &j, &store, Some(&p.hash()), None)).contains("unknown"));
            }
            assert_eq!(c.writes().len(), 1);
        }
    }

    /// `test_stale_target_and_duplicates_fail_before_send`.
    #[test]
    fn stale_target_and_duplicates() {
        let (domain, cid, high) = WIRES[0];
        let w = world(cid);
        let c = client(&w, domain, cid, high);
        let store = MemStore::default();
        let p = build_plan(&c, &store, &ids(cid), "High", None).unwrap();
        w.lock().unwrap().rows[&(cid + 1).to_string()]["subject"] = json!("changed");
        assert!(
            err(send(
                &p,
                &c,
                &MemJournal::default(),
                &store,
                Some(&p.hash()),
                None
            ))
            .contains("changed")
        );
        assert_eq!(
            err(build_plan(
                &c,
                &store,
                &[cid.to_string(), cid.to_string()],
                "High",
                None
            )),
            "ticket IDs must be positive and unique"
        );
        assert_eq!(
            err(build_plan(&c, &store, &[], "High", None)),
            "specify between 1 and 100 explicit ticket IDs"
        );
        assert!(c.writes().is_empty());
    }
}
