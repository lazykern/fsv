//! Reviewed ticket property changes from a blocked UI request
//! (`ticket_properties.py`).

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_forms::build_br_validation_excludes;
use fsv_py::py_str;

use crate::formmeta::{MetaStore, RULES_UNAVAILABLE_KEY, existing_from_record, load_meta};
use crate::service::{Api, Journal, definite_rejection, last_event, obj, plan_hash};

const ACTION: &str = "ticket_properties";

/// `PropertyError`: a safe user-facing failure.
pub(crate) fn refused(msg: impl Into<String>) -> Error {
    Error::exit(2, msg)
}

/// `json.dumps(v, sort_keys=True, ensure_ascii=False)`.
pub(crate) fn dumps_sorted(v: &Value) -> String {
    fsv_py::Dumps::default()
        .sort_keys(true)
        .ensure_ascii(false)
        .dump(v)
}

/// `_record`: the ticket, refusing anything that is not exactly it.
pub fn record(c: &dyn Api, ticket_id: i64) -> Result<Map<String, Value>> {
    let raw = c.int_get(&format!("tickets/{ticket_id}"), None)?;
    let Value::Object(m) = raw else {
        return Err(refused(
            "ticket read returned an invalid record; no update attempted",
        ));
    };
    let row = match m.get("ticket") {
        Some(t) => t.clone(),
        None => Value::Object(m),
    };
    match row {
        Value::Object(r)
            if py_str(r.get("id").unwrap_or(&Value::Null)) == ticket_id.to_string() =>
        {
            Ok(r)
        }
        _ => Err(refused(
            "ticket read did not identify the exact target; no update attempted",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyPlan {
    pub ticket_id: i64,
    pub tenant: String,
    pub body_json: String,
    pub before_json: String,
    pub record_hash: String,
}

impl PropertyPlan {
    pub fn payload(&self) -> Value {
        serde_json::from_str(&self.body_json).unwrap_or(Value::Null)
    }

    pub fn before(&self) -> Value {
        serde_json::from_str(&self.before_json).unwrap_or(Value::Null)
    }

    pub fn hash(&self) -> String {
        plan_hash(
            &json!({"tenant": self.tenant, "ticket_id": self.ticket_id, "payload": self.payload(),
                          "before": self.before(), "record_hash": self.record_hash}),
        )
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut after = self.payload().as_object().cloned().unwrap_or_default();
        after.remove("br_validation_excludes");
        obj(
            json!({"action": ACTION, "method": "PUT", "path": format!("/api/_/tickets/{}", self.ticket_id),
                   "before": self.before(), "after": after, "payload": self.payload(),
                   "plan_hash": self.hash(), "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."}),
        )
    }
}

/// The fields' business-rule token, evaluated on the fresh edit form with
/// `body` applied.
fn rules_token(
    c: &dyn Api,
    store: &dyn MetaStore,
    cid: i64,
    row: &Map<String, Value>,
    body: &Map<String, Value>,
) -> Result<String> {
    let meta = load_meta(c, store, "ticket", "edit", true, &json!(cid))?;
    if meta
        .rules
        .get(RULES_UNAVAILABLE_KEY)
        .is_some_and(fsv_py::truthy)
    {
        return Err(refused(
            "ticket business rules are unavailable; use the web form",
        ));
    }
    let spec = meta.spec();
    let mut state = meta.state("edit", None, Some(existing_from_record(&spec, row)))?;
    for (key, value) in body {
        if let Some(field) = spec.find(key) {
            state
                .set(&field.api_name, fsv_forms::Value::from_json(value))
                .map_err(|e| Error::other(e.to_string()))?;
        }
    }
    if !state.rules.unavailable_dependencies("edit").is_empty()
        || state
            .rules
            .warnings()
            .iter()
            .any(|w| w.contains("unknown field") || w.contains("none was given"))
    {
        return Err(refused(
            "ticket business rules need unavailable fields or context; use the web form",
        ));
    }
    let problems = state.errors().map_err(|e| Error::other(e.to_string()))?;
    if !problems.is_empty() {
        return Err(refused(format!(
            "ticket form validation failed: {}",
            problems.join("; ")
        )));
    }
    build_br_validation_excludes(&state.evaluated, "ticket", &Value::Null, &Value::Null)
        .map_err(|e| Error::other(e.to_string()))
}

/// `build_plan`: validated tags/planned dates, the exact `before` values and
/// the business-rule token from fresh form metadata.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    ticket_id: &str,
    tags: Option<&[String]>,
    planned_start: Option<&str>,
    planned_end: Option<&str>,
) -> Result<PropertyPlan> {
    let cid = parse_id(ticket_id, &TICKETS)?;
    if cid <= 0 {
        return Err(Error::value("ticket ID must be positive"));
    }
    if let Some(tags) = tags
        && (tags.is_empty()
            || tags.iter().any(|t| {
                fsv_core::pystr::strip(t).is_empty()
                    || t != fsv_core::pystr::strip(t)
                    || crate::service::has_control(t, "")
            }))
    {
        return Err(Error::value(
            "tags must be nonempty text without surrounding whitespace or control characters",
        ));
    }
    let mut body = Map::new();
    if let Some(tags) = tags {
        body.insert("tags".into(), json!(tags));
    }
    for (key, raw) in [
        ("planned_start_date", planned_start),
        ("planned_end_date", planned_end),
    ] {
        if let Some(raw) = raw {
            let Some(parsed) = crate::tasks::date(Some(raw))? else {
                return Err(Error::value("planned dates cannot be empty"));
            };
            body.insert(key.into(), json!(parsed));
        }
    }
    if body.is_empty() {
        return Err(Error::value(
            "specify --tag, --planned-start or --planned-end",
        ));
    }
    let row = record(c, cid)?;
    let missing: Vec<&str> = body
        .keys()
        .filter(|k| !row.contains_key(*k))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(refused(format!(
            "ticket read did not include: {}",
            missing.join(", ")
        )));
    }
    let before: Map<String, Value> = body.keys().map(|k| (k.clone(), row[k].clone())).collect();
    let token = match rules_token(c, store, cid, &row, &body) {
        Ok(t) => t,
        Err(e @ Error::Exit { .. }) => return Err(e),
        Err(_) => {
            return Err(refused(
                "could not evaluate ticket edit form and rules; no update attempted",
            ));
        }
    };
    body.insert("br_validation_excludes".into(), json!(token));
    Ok(PropertyPlan {
        ticket_id: cid,
        tenant: c.domain(),
        body_json: dumps_sorted(&Value::Object(body)),
        before_json: dumps_sorted(&Value::Object(before)),
        record_hash: plan_hash(&Value::Object(row)),
    })
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    v?.as_array().map(|a| a.iter().map(py_str).collect())
}

/// `send`: one PUT after the plan is rebuilt unchanged, then a read-back.
pub fn send(
    plan: &PropertyPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    store: &dyn MetaStore,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled"}));
    };
    let hash = plan.hash();
    if plan.tenant.is_empty() || plan.tenant != c.domain() || reviewed != hash {
        return Err(refused("tenant or review hash changed; review again"));
    }
    if matches!(
        last_event(journal, ACTION, &hash).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "update was sent or its outcome is unknown; inspect the ticket and journal before retry",
        ));
    }
    let payload = plan.payload();
    let tags = strings(payload.get("tags"));
    let fresh = build_plan(
        c,
        store,
        &plan.ticket_id.to_string(),
        tags.as_deref(),
        payload.get("planned_start_date").and_then(Value::as_str),
        payload.get("planned_end_date").and_then(Value::as_str),
    )?;
    if fresh.hash() != hash {
        return Err(refused("ticket or form changed since review; review again"));
    }
    let Ok(csrf) = c.get_csrf() else {
        return Err(refused("could not prepare session; no update attempted"));
    };
    let target = json!(plan.ticket_id);
    journal.record_plan("started", &hash, ACTION, Some(&target), None);
    if last_event(journal, ACTION, &hash).as_deref() != Some("started") {
        return Err(refused("could not persist journal; no update attempted"));
    }
    if let Err(e) = c.send_once(
        "PUT",
        &format!("tickets/{}", plan.ticket_id),
        Body::Raw(plan.body_json.clone().into_bytes()),
        None,
        &csrf,
    ) {
        if let Some(status) = definite_rejection(&e) {
            journal.record_plan("failed", &hash, ACTION, Some(&target), None);
            return Err(refused(format!(
                "update rejected (HTTP {status}); no retry"
            )));
        }
        return Err(refused(
            "update outcome unknown; inspect ticket and journal before retry",
        ));
    }
    journal.record_plan("done", &hash, ACTION, Some(&target), None);
    let mut result = obj(
        json!({"status": "submitted", "ticket_id": plan.ticket_id, "plan_hash": hash, "verified": false}),
    );
    if let Ok(row) = record(c, plan.ticket_id) {
        let mut fields = Map::new();
        for (key, expected) in payload.as_object().into_iter().flatten() {
            if key == "br_validation_excludes" {
                continue;
            }
            let state = match row.get(key) {
                None => "unavailable",
                Some(actual) if key == "planned_start_date" || key == "planned_end_date" => {
                    let same = expected.as_str().and_then(|e| {
                        crate::tasks::same_datetime(&py_str(actual).replace('Z', "+00:00"), e)
                    });
                    if same == Some(true) {
                        "matched"
                    } else {
                        "mismatched"
                    }
                }
                Some(actual) if actual == expected => "matched",
                Some(_) => "mismatched",
            };
            fields.insert(key.clone(), json!(state));
        }
        let verified = fields.values().all(|v| v == "matched");
        result.insert("fields".into(), Value::Object(fields));
        result.insert("verified".into(), json!(verified));
    }
    if result["verified"] != Value::Bool(true) {
        result.insert(
            "warning".into(),
            json!("Ticket fields not fully verified on read-back; do not resend."),
        );
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formmeta::MemStore;
    use crate::service::testing::*;

    const TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=";
    const HASH: &str = "8d4a9b0683a1e5313139a37cb19d4b6f4e9fc917ee7891bd836574a744b03ffc";
    const UNVERIFIED: &str = "Ticket fields not fully verified on read-back; do not resend.";

    #[test]
    fn payload_parses_the_reviewed_body() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/coverage_oracle.json")).unwrap();
        let plan = PropertyPlan {
            ticket_id: 5,
            tenant: "acme".into(),
            body_json: r#"{"status": 2, "custom_fields": {"a": null}}"#.into(),
            before_json: "{}".into(),
            record_hash: "h".into(),
        };
        assert_eq!(plan.payload(), oracle["misc"]["property_payload"]);
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    fn row() -> Value {
        json!({"id": 123, "subject": "Test", "tags": ["old"], "planned_start_date": null, "planned_end_date": null, "status": 2})
    }

    fn tags() -> Vec<String> {
        vec!["a".into(), "b".into()]
    }

    fn plan() -> PropertyPlan {
        let t = tags();
        build_plan(
            &ticket_api(json!({"ticket": row()})),
            &MemStore::default(),
            "123",
            Some(&t),
            Some("2030-01-01T09:00:00+07:00"),
            None,
        )
        .unwrap()
    }

    #[test]
    fn record_exact_target() {
        assert_eq!(
            Value::Object(record(&ticket_api(json!({"ticket": row()})), 123).unwrap()),
            row()
        );
        assert_eq!(
            Value::Object(record(&ticket_api(json!({"id": "123", "x": 1})), 123).unwrap()),
            json!({"id": "123", "x": 1})
        );
        assert_eq!(
            err(record(&ticket_api(json!({"ticket": {"id": 5}})), 123)),
            "ticket read did not identify the exact target; no update attempted"
        );
        assert_eq!(
            err(record(&ticket_api(json!([1])), 123)),
            "ticket read returned an invalid record; no update attempted"
        );
    }

    #[test]
    fn build_plan_matches_python() {
        let p = plan();
        assert_eq!(
            p.body_json,
            format!(
                "{{\"br_validation_excludes\": \"{TOKEN}\", \"planned_start_date\": \"2030-01-01T09:00:00+07:00\", \"tags\": [\"a\", \"b\"]}}"
            )
        );
        assert_eq!(
            p.before_json,
            r#"{"planned_start_date": null, "tags": ["old"]}"#
        );
        assert_eq!(
            (p.record_hash.as_str(), p.hash()),
            (
                "4ff17c2dd9deab1b41491a97c93c5857169f1d7dee30a7decb4552d53e4b9f7a",
                HASH.to_string()
            )
        );
        assert_eq!(
            Value::Object(p.review()),
            json!({"action": "ticket_properties", "method": "PUT", "path": "/api/_/tickets/123",
            "before": {"planned_start_date": null, "tags": ["old"]}, "after": {"planned_start_date": "2030-01-01T09:00:00+07:00", "tags": ["a", "b"]},
            "payload": {"br_validation_excludes": TOKEN, "planned_start_date": "2030-01-01T09:00:00+07:00", "tags": ["a", "b"]},
            "plan_hash": HASH, "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."})
        );
    }

    #[test]
    fn build_plan_errors() {
        let c = ticket_api(json!({"ticket": row()}));
        let store = MemStore::default();
        let tag_err =
            "tags must be nonempty text without surrounding whitespace or control characters";
        assert_eq!(
            err(build_plan(
                &c,
                &store,
                "123",
                Some(&[" a".to_string()]),
                None,
                None
            )),
            tag_err
        );
        assert_eq!(
            err(build_plan(&c, &store, "123", Some(&[]), None, None)),
            tag_err
        );
        assert_eq!(
            err(build_plan(&c, &store, "123", None, None, None)),
            "specify --tag, --planned-start or --planned-end"
        );
        assert_eq!(
            err(build_plan(
                &c,
                &store,
                "123",
                None,
                None,
                Some("2030-01-01")
            )),
            "planned dates require ISO-8601 date/time with explicit offset, whole seconds (no host timezone inferred)"
        );
        assert_eq!(
            err(build_plan(&c, &store, "123", None, Some(" "), None)),
            "planned dates cannot be empty"
        );
        let t = vec!["a".to_string()];
        assert_eq!(
            err(build_plan(
                &ticket_api(json!({"ticket": {"id": 123}})),
                &store,
                "123",
                Some(&t),
                None,
                None
            )),
            "ticket read did not include: tags"
        );
        let no_rules = FakeApi::new()
            .ok("GET ticket_form_fields", ticket_form(3))
            .ok("GET tickets/123", json!({"ticket": row()}));
        assert_eq!(
            err(build_plan(&no_rules, &store, "123", Some(&t), None, None)),
            "ticket business rules are unavailable; use the web form"
        );
        assert_eq!(
            err(build_plan(&c, &store, "CHN-1", Some(&t), None, None)),
            "prefix CHN not valid for tickets (expect INC or SR)"
        );
    }

    #[test]
    fn send_verifies_readback() {
        let p = plan();
        let mut after = row();
        after["tags"] = json!(["a", "b"]);
        after["planned_start_date"] = json!("2030-01-01T02:00:00Z");
        let c = ticket_api(json!({"ticket": row()}))
            .ok("GET tickets/123", json!({"ticket": after}))
            .ok("PUT tickets/123", json!({}));
        let (j, store) = (MemJournal::default(), MemStore::default());
        assert_eq!(
            send(&p, &c, &j, &store, Some(HASH)).unwrap(),
            json!({"status": "submitted", "ticket_id": 123, "plan_hash": HASH, "verified": true, "fields": {"planned_start_date": "matched", "tags": "matched"}})
        );
        assert_eq!(j.events(), ["started", "done"]);
        let w = c.writes();
        assert_eq!(
            (w[0].method.as_str(), w[0].path.as_str()),
            ("PUT", "tickets/123")
        );
        assert_eq!(
            String::from_utf8(w[0].raw.clone().unwrap()).unwrap(),
            p.body_json
        );
        assert_eq!(
            err(send(&p, &c, &j, &store, Some(HASH))),
            "update was sent or its outcome is unknown; inspect the ticket and journal before retry"
        );
        assert_eq!(
            send(&p, &c, &j, &store, None).unwrap(),
            json!({"status": "cancelled"})
        );
        assert_eq!(
            err(send(&p, &c, &j, &store, Some("x"))),
            "tenant or review hash changed; review again"
        );
    }

    #[test]
    fn send_mismatch_readback_failure_and_rejection() {
        let p = plan();
        let store = MemStore::default();
        let mut other = row();
        other["tags"] = json!(["z"]);
        let c = ticket_api(json!({"ticket": row()}))
            .ok("GET tickets/123", json!({"ticket": other}))
            .ok("PUT tickets/123", json!({}));
        assert_eq!(
            send(&p, &c, &MemJournal::default(), &store, Some(HASH)).unwrap(),
            json!({"status": "submitted", "ticket_id": 123, "plan_hash": HASH, "verified": false,
                          "fields": {"planned_start_date": "mismatched", "tags": "mismatched"}, "warning": UNVERIFIED})
        );
        let c = ticket_api(json!({"ticket": row()}))
            .on("GET tickets/123", Reply::Status(500))
            .ok("PUT tickets/123", json!({}));
        assert_eq!(
            send(&p, &c, &MemJournal::default(), &store, Some(HASH)).unwrap(),
            json!({"status": "submitted", "ticket_id": 123, "plan_hash": HASH, "verified": false, "warning": UNVERIFIED})
        );
        let mut changed = row();
        changed["subject"] = json!("Changed");
        assert_eq!(
            err(send(
                &p,
                &ticket_api(json!({"ticket": changed})),
                &MemJournal::default(),
                &store,
                Some(HASH)
            )),
            "ticket or form changed since review; review again"
        );
        let j = MemJournal::default();
        let c = ticket_api(json!({"ticket": row()})).on("PUT tickets/123", Reply::Status(409));
        assert_eq!(
            err(send(&p, &c, &j, &store, Some(HASH))),
            "update rejected (HTTP 409); no retry"
        );
        assert_eq!(j.events(), ["started", "failed"]);
    }
}
