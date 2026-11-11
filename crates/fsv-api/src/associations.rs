//! Reviewed initiating-ticket relationships; no write retries or inferred
//! success (`change_associations.py`).

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, parse_id};

use crate::service::{Api, Journal, collect_pages, definite_rejection, dumps_bytes, last_event, obj, plan_hash, positive_id};

const ACTION: &str = "change_ticket_association";
const UNKNOWN: &str = "association outcome unknown/unverified; inspect the change and journal; do not resubmit";
pub const KINDS: &[&str] = &["tickets", "problems", "releases"];

/// `AssociationError` (a `ValueError`).
fn fail(msg: &str) -> Error {
    Error::value(msg)
}

/// `AssociationReader.int_get`: validate rows before the paginator can
/// discard malformed entries.
pub fn read_page(c: &dyn Api, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
    let data = match c.int_get(path, params) {
        Ok(d) => d,
        Err(e @ Error::Api(_)) => return Err(e),
        Err(_) => return Err(fail("association page unavailable")),
    };
    let key = path.rsplit('/').next().unwrap_or(path);
    let ok = data
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|rows| rows.iter().all(|r| r.is_object() && r.get("id").and_then(positive_id).is_some()));
    if !data.is_object() || !ok {
        return Err(fail("unreadable association page"));
    }
    Ok(data)
}

/// `inventory`: rows per kind with explicit completeness; unavailable is never empty.
pub fn inventory(c: &dyn Api, change_id: i64, kinds: &[&str]) -> Result<Value> {
    let mut meta = Map::new();
    let mut warnings: Vec<Value> = Vec::new();
    let mut out = Map::new();
    for kind in kinds {
        let params = obj(json!({"per_page": 50, "page": 1}));
        let pages = collect_pages(|p, q| read_page(c, p, q), &format!("changes/{change_id}/{kind}"), kind, Some(&params), kind, crate::service::MAX_DETAIL_PAGES)?;
        if pages.missing.is_some() || !pages.complete() {
            let partial = pages.meta.get("partial").filter(|v| fsv_py::truthy(v)).cloned();
            warnings.push(partial.unwrap_or_else(|| json!(format!("{kind}: unavailable"))));
        }
        out.insert((*kind).to_string(), Value::Array(pages.rows));
        meta.insert((*kind).to_string(), Value::Object(pages.meta));
    }
    let complete = meta.values().all(|m| m.get("complete") == Some(&Value::Bool(true)));
    let mut result = Map::new();
    result.insert("meta".into(), Value::Object(meta));
    result.insert("warnings".into(), Value::Array(warnings));
    result.extend(out);
    result.insert("complete".into(), json!(complete));
    Ok(Value::Object(result))
}

/// Initiating ticket ids; an incomplete inventory is an error, not an empty set.
pub fn ticket_ids(c: &dyn Api, change_id: i64) -> Result<BTreeSet<i64>> {
    let data = inventory(c, change_id, &["tickets"])?;
    if data["complete"] != Value::Bool(true) {
        return Err(fail("initiating-ticket inventory unavailable/incomplete; not an empty list"));
    }
    Ok(data["tickets"].as_array().into_iter().flatten().filter_map(|r| r.get("id").and_then(positive_id)).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociationPlan {
    pub change_id: i64,
    pub ticket_ids: Vec<i64>,
    pub unlink: bool,
    pub tenant: String,
}

impl AssociationPlan {
    pub fn method(&self) -> &'static str {
        if self.unlink { "DELETE" } else { "PUT" }
    }

    pub fn path(&self) -> String {
        let tail = if self.unlink { self.ticket_ids[0].to_string() } else { "associate".into() };
        format!("changes/{}/tickets/{tail}", self.change_id)
    }

    pub fn payload(&self) -> Value {
        if self.unlink { json!({"type": "unlink"}) } else { json!({"item_ids": self.ticket_ids, "module_type": "ticket"}) }
    }

    pub fn params(&self) -> Option<Map<String, Value>> {
        self.unlink.then(|| obj(json!({"module_type": "ticket"})))
    }

    fn params_json(&self) -> Value {
        self.params().map_or(Value::Null, Value::Object)
    }

    pub fn hash(&self) -> String {
        plan_hash(&json!({"tenant": self.tenant, "method": self.method(), "path": self.path(),
                          "payload": self.payload(), "params": self.params_json()}))
    }

    pub fn review(&self) -> Map<String, Value> {
        obj(json!({
            "action": ACTION, "tenant": self.tenant, "change_id": self.change_id,
            "ticket_ids": self.ticket_ids, "method": self.method(),
            "path": format!("/api/_/{}", self.path()), "payload": self.payload(), "params": self.params_json(),
            "plan_hash": self.hash(),
            "warning": "Initiating-ticket relationship only. Tenant workflows may notify people. One attempt; never automatically resubmit.",
        }))
    }
}

/// `build_plan`. `change_id` is a display id (`CHN-12` or `12`); ticket ids
/// are positive internal ids, deduplicated and sorted.
pub fn build_plan(change_id: &str, ticket_ids: &[Value], unlink: bool, tenant: &str) -> Result<AssociationPlan> {
    let cid = parse_id(change_id, &CHANGES)?;
    if cid <= 0 {
        return Err(fail("invalid change ID or association action"));
    }
    if ticket_ids.is_empty() || ticket_ids.iter().any(|v| positive_id(v).is_none()) {
        return Err(fail("ticket IDs must be positive internal ticket IDs"));
    }
    let ids: BTreeSet<i64> = ticket_ids.iter().filter_map(positive_id).collect();
    if unlink && ids.len() != 1 {
        return Err(fail("unlink accepts one ticket per reviewed request; review each removal separately"));
    }
    Ok(AssociationPlan { change_id: cid, ticket_ids: ids.into_iter().collect(), unlink, tenant: tenant.to_string() })
}

fn with_review(status: &str, verified: bool, plan: &AssociationPlan) -> Value {
    let mut out = obj(json!({"status": status, "verified": verified}));
    out.extend(plan.review());
    Value::Object(out)
}

/// Send options shared by the reviewed write flows.
#[derive(Debug, Clone, Copy, Default)]
pub struct SendOpts<'a> {
    pub reviewed_hash: Option<&'a str>,
    pub read_only: bool,
    pub dry_run: bool,
}

/// `send`: one reviewed request, then a complete read-back.
pub fn send(plan: &AssociationPlan, c: &dyn Api, journal: &dyn Journal, opts: SendOpts<'_>) -> Result<Value> {
    let ids: Vec<Value> = plan.ticket_ids.iter().map(|i| json!(i)).collect();
    if build_plan(&plan.change_id.to_string(), &ids, plan.unlink, &c.domain())? != *plan {
        return Err(fail("association plan or tenant changed; review again"));
    }
    if opts.dry_run {
        let mut out = obj(json!({"status": "dry_run"}));
        out.extend(plan.review());
        return Ok(Value::Object(out));
    }
    if opts.read_only {
        return Err(fail("read-only mode: no association request was sent"));
    }
    let Some(reviewed) = opts.reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    if reviewed != hash || plan.tenant.is_empty() {
        return Err(fail("association review hash/tenant does not match; review again"));
    }
    if matches!(last_event(journal, ACTION, &hash).as_deref(), Some("started" | "done")) {
        return Err(fail(UNKNOWN));
    }
    let prepared = ticket_ids(c, plan.change_id).and_then(|b| c.get_csrf().map(|t| (b, t)));
    let Ok((before, csrf)) = prepared else {
        return Err(fail("could not prepare complete initiating-ticket inventory/session; no write attempted"));
    };
    let desired: BTreeSet<i64> = plan.ticket_ids.iter().copied().collect();
    if (!plan.unlink && desired.is_subset(&before)) || (plan.unlink && desired.is_disjoint(&before)) {
        return Ok(with_review("already_in_requested_state", true, plan));
    }
    let target = json!(plan.change_id);
    journal.record_plan("started", &hash, ACTION, Some(&target), None);
    if last_event(journal, ACTION, &hash).as_deref() != Some("started") {
        return Err(fail("could not persist association journal; no write attempted"));
    }
    let body = Body::Raw(dumps_bytes(&plan.payload(), true));
    if let Err(e) = c.send_once(plan.method(), &plan.path(), body, plan.params(), &csrf) {
        if let Some(status) = definite_rejection(&e) {
            journal.record_plan("failed", &hash, ACTION, Some(&target), None);
            return Err(fail(&format!("association rejected (HTTP {status}); no automatic retry")));
        }
        return Err(fail(UNKNOWN));
    }
    let after = ticket_ids(c, plan.change_id).map_err(|_| fail(UNKNOWN))?;
    let verified = if plan.unlink { desired.is_disjoint(&after) } else { desired.is_subset(&after) };
    if !verified {
        return Err(fail(UNKNOWN));
    }
    journal.record_plan("done", &hash, ACTION, Some(&target), None);
    Ok(with_review("submitted", true, plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::*;

    const T: &str = "acme.freshservice.com";

    fn empty_kinds(api: FakeApi, id: i64) -> FakeApi {
        api.ok(&format!("GET changes/{id}/problems"), json!({"problems": []}))
            .ok(&format!("GET changes/{id}/releases"), json!({"releases": []}))
    }

    #[test]
    fn plan_shape_and_hash() {
        let p = build_plan("CHN-7", &[json!(9), json!("3"), json!(9)], false, T).unwrap();
        assert_eq!(p.ticket_ids, [3, 9]);
        assert_eq!(p.payload(), json!({"item_ids": [3, 9], "module_type": "ticket"}));
        assert_eq!(p.path(), "changes/7/tickets/associate");
        assert_eq!(
            p.hash(),
            plan_hash(&json!({"tenant": T, "method": "PUT", "path": "changes/7/tickets/associate",
                               "payload": {"item_ids": [3, 9], "module_type": "ticket"}, "params": null}))
        );
        let u = build_plan("7", &[json!(4)], true, T).unwrap();
        assert_eq!((u.method(), u.path()), ("DELETE", "changes/7/tickets/4".to_string()));
        assert_eq!(build_plan("7", &[json!(4), json!(5)], true, T).unwrap_err().to_string(),
                   "unlink accepts one ticket per reviewed request; review each removal separately");
        assert!(build_plan("7", &[json!(true)], false, T).is_err());
        assert!(build_plan("INC-7", &[json!(1)], false, T).is_err());
    }

    #[test]
    fn inventory_marks_unreadable_pages_incomplete() {
        let api = empty_kinds(FakeApi::new(), 7).ok("GET changes/7/tickets", json!({"tickets": [{"id": "x"}]}));
        let inv = inventory(&api, 7, KINDS).unwrap();
        assert_eq!(inv["complete"], false);
        assert_eq!(inv["warnings"][0], "tickets: incomplete (0 loaded; tickets: unavailable)");
        assert!(ticket_ids(&api, 7).is_err());
    }

    #[test]
    fn send_verifies_with_readback() {
        let api = FakeApi::new()
            .ok("GET changes/7/tickets", json!({"tickets": []}))
            .ok("GET changes/7/tickets", json!({"tickets": [{"id": 3}]}))
            .ok("PUT changes/7/tickets/associate", json!({}));
        let j = MemJournal::default();
        let p = build_plan("7", &[json!(3)], false, T).unwrap();
        let h = p.hash();
        let out = send(&p, &api, &j, SendOpts { reviewed_hash: Some(&h), ..Default::default() }).unwrap();
        assert_eq!(out["status"], "submitted");
        assert_eq!(j.events(), ["started", "done"]);
        assert_eq!(api.writes()[0].raw.as_deref(), Some(br#"{"item_ids": [3], "module_type": "ticket"}"#.as_slice()));
        // a second send is blocked by the journal
        assert_eq!(send(&p, &api, &j, SendOpts { reviewed_hash: Some(&h), ..Default::default() }).unwrap_err().to_string(), UNKNOWN);
    }

    #[test]
    fn send_rejection_and_cancel() {
        let api = FakeApi::new().ok("GET changes/7/tickets", json!({"tickets": [{"id": 1}]})).on("DELETE changes/7/tickets/1", crate::service::testing::Reply::Status(422));
        let j = MemJournal::default();
        let p = build_plan("7", &[json!(1)], true, T).unwrap();
        assert_eq!(send(&p, &api, &j, SendOpts::default()).unwrap()["status"], "cancelled");
        let h = p.hash();
        let e = send(&p, &api, &j, SendOpts { reviewed_hash: Some(&h), ..Default::default() }).unwrap_err();
        assert_eq!(e.to_string(), "association rejected (HTTP 422); no automatic retry");
        assert_eq!(j.events(), ["started", "failed"]);
        assert_eq!(api.writes()[0].params, Some(obj(json!({"module_type": "ticket"}))));
    }
}
