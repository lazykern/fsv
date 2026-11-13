//! Reviewed ticket property changes from a blocked UI request
//! (`ticket_properties.py`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_forms::payload::{TICKET_KEYS, encode_value};
use fsv_forms::{FormState, Kind, LookupResolver, StateOptions, build_br_validation_excludes};
use fsv_py::py_str;

use crate::formmeta::{
    MetaStore, RULES_UNAVAILABLE_KEY, existing_from_record, form_err, is_status, load_meta,
};
use crate::readback::same_instant;
use crate::service::{
    Api, Journal, definite_rejection, last_event, obj, plan_hash, snapshot_hash, stable_snapshot,
};

const ACTION: &str = "ticket_properties";

/// Core keys whose update wire is proven.
const ALLOWED: [&str; 13] = [
    "subject",
    "description",
    "requester_id",
    "requested_for_id",
    "group_id",
    "responder_id",
    "status",
    "priority",
    "urgency",
    "impact",
    "source",
    "category",
    "type",
];

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

    /// The tag/date hash (no answers, so no `metadata_hash`).
    pub fn hash(&self) -> String {
        self.hash_with("", &[])
    }

    /// `build_plan` has already checked the baseline snapshots, so a
    /// hand-built plan whose baseline `urlsplit` rejects hashes it unprojected.
    fn hash_with(&self, metadata_hash: &str, acknowledged: &[String]) -> String {
        let before = self.before();
        let before = stable_snapshot(&before).unwrap_or(before);
        let mut intent = json!({"tenant": self.tenant, "ticket_id": self.ticket_id, "payload": self.payload(),
                                "before": before, "record_hash": self.record_hash});
        if !metadata_hash.is_empty() {
            intent["metadata_hash"] = json!(metadata_hash);
        }
        // Bound only when used, so unacknowledged review hashes are unchanged.
        if !acknowledged.is_empty() {
            intent["acknowledged"] = json!(acknowledged);
        }
        plan_hash(&intent)
    }

    pub fn review(&self) -> Map<String, Value> {
        self.review_with(self.hash())
    }

    fn review_with(&self, hash: String) -> Map<String, Value> {
        let mut after = self.payload().as_object().cloned().unwrap_or_default();
        after.shift_remove("br_validation_excludes");
        obj(
            json!({"action": ACTION, "method": "PUT", "path": format!("/api/_/tickets/{}", self.ticket_id),
                   "before": self.before(), "after": after, "payload": self.payload(),
                   "plan_hash": hash, "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."}),
        )
    }
}

/// Port of fsv.ticket_properties.PropertyPlan including its
/// `answers_json` and `metadata_hash` fields; [`PropertyPlan`] is the
/// answer-less (tag/date) shape, whose hash is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsweredPropertyPlan {
    pub plan: PropertyPlan,
    /// The answers the plan was built from (`"{}"` for tag/date-only plans).
    pub answers_json: String,
    /// The edit form's fingerprint when answers were given, else empty.
    pub metadata_hash: String,
    /// Acknowledged blocking rule message texts, sorted (`"[]"` when none).
    pub acknowledged_json: String,
}

impl From<PropertyPlan> for AnsweredPropertyPlan {
    fn from(plan: PropertyPlan) -> Self {
        AnsweredPropertyPlan {
            plan,
            answers_json: "{}".into(),
            metadata_hash: String::new(),
            acknowledged_json: "[]".into(),
        }
    }
}

impl AnsweredPropertyPlan {
    pub fn payload(&self) -> Value {
        self.plan.payload()
    }

    /// The stored answers (`json.loads(answers_json)`).
    pub fn answers(&self) -> Map<String, Value> {
        serde_json::from_str(&self.answers_json).unwrap_or_default()
    }

    pub fn acknowledged(&self) -> Vec<String> {
        serde_json::from_str(&self.acknowledged_json).unwrap_or_default()
    }

    /// `metadata_hash` joins the intent only when answers were given.
    pub fn hash(&self) -> String {
        self.plan
            .hash_with(&self.metadata_hash, &self.acknowledged())
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut out = self.plan.review_with(self.hash());
        let acknowledged = self.acknowledged();
        if !acknowledged.is_empty() {
            out.insert("acknowledged_rule_messages".into(), json!(acknowledged));
        }
        out
    }
}

fn missing_keys(body: &Map<String, Value>, row: &Map<String, Value>) -> Result<()> {
    let missing: Vec<&str> = body
        .keys()
        .filter(|k| !row.contains_key(*k))
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(refused(format!(
            "ticket read did not include: {}",
            missing.join(", ")
        )))
    }
}

fn values_of(body: &Map<String, Value>, row: &Map<String, Value>) -> Map<String, Value> {
    body.keys().map(|k| (k.clone(), row[k].clone())).collect()
}

/// The fresh edit form with the answers encoded into `body` and `body`
/// applied: `(business-rule token, metadata hash or "", acknowledged texts)`.
#[allow(clippy::too_many_arguments)]
fn evaluate(
    c: &dyn Api,
    store: &dyn MetaStore,
    cid: i64,
    row: &Map<String, Value>,
    body: &mut Map<String, Value>,
    answers: &Map<String, Value>,
    resolver: Option<Arc<dyn LookupResolver>>,
    ack: bool,
) -> Result<(String, String, Vec<String>)> {
    let other = |e: fsv_forms::FormError| Error::other(e.to_string());
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
    let mut engine = meta.engine()?;
    let mut beneficiary_answer = false;
    for name in answers.keys() {
        beneficiary_answer |= spec.field(name).map_err(other)?.api_name == "requested_for";
    }
    if beneficiary_answer {
        // Cached rule attributes describe the old beneficiary, not the newly
        // selected person. Missing exact context must block rules.
        engine.context.requested_for = None;
    }
    let mut state = FormState::new(
        spec.clone(),
        Some(engine),
        "edit",
        None,
        StateOptions {
            resolver,
            existing: Some(existing_from_record(&spec, row)),
            ..StateOptions::default()
        },
    )
    .map_err(form_err)?;
    let mut answered: Vec<String> = Vec::new();
    for (name, value) in answers {
        let field = spec.field(name).map_err(other)?;
        if answered.contains(&field.api_name) {
            return Err(refused(format!(
                "{}: duplicate property answer",
                field.label
            )));
        }
        answered.push(field.api_name.clone());
        let beneficiary =
            field.api_name == "requested_for" && field.field_type == "default_requested_for";
        if (!field.sent && !beneficiary) || field.synthetic || field.kind == Kind::Notice {
            return Err(refused(format!("{}: field is not writable", field.label)));
        }
        state
            .set(&field.api_name, fsv_forms::Value::from_json(value))
            .map_err(other)?;
    }
    for name in answers.keys() {
        let field = spec.field(name).map_err(other)?;
        let value = state
            .get(&field.api_name)
            .map_err(other)?
            .cloned()
            .unwrap_or(fsv_forms::Value::Null);
        if !state.evaluated.visible.contains(&field.api_name) {
            return Err(refused(format!(
                "{}: answered field is hidden",
                field.label
            )));
        }
        // Clearing an optional routing lookup is a captured explicit null,
        // not a lookup ID to resolve. Validation below still enforces rules.
        let encoded = if value == fsv_forms::Value::Null
            && matches!(field.kind, Kind::LookupAgent | Kind::LookupGroup)
        {
            Value::Null
        } else {
            encode_value(&spec, field, &value, &state.evaluated.values).map_err(other)?
        };
        if is_status(field) {
            let closing = match field.choice(&value).map_err(other)? {
                None => true,
                Some(ch) => {
                    let label = fsv_core::pystr::casefold(&ch.label);
                    label == "resolved" || label == "closed"
                }
            };
            if closing {
                return Err(refused(
                    "update does not resolve or close tickets (combined resolution/closure requirements are not proven here); use `fsv tickets resolve ID --note TEXT` or `fsv tickets close ID` (both check runtime closure requirements and accept --set/--ack), or the web form",
                ));
            }
        }
        if field.is_custom {
            let custom = body
                .entry("custom_fields")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(m) = custom {
                m.insert(field.api_name.clone(), encoded);
            }
        } else {
            let key = if matches!(field.api_name.as_str(), "requester" | "requested_for") {
                format!("{}_id", field.api_name)
            } else {
                TICKET_KEYS
                    .iter()
                    .find(|(k, _)| *k == field.api_name)
                    .map_or(field.api_name.clone(), |(_, v)| v.to_string())
            };
            if !ALLOWED.contains(&key.as_str()) {
                return Err(refused(format!(
                    "{}: update wire is not proven",
                    field.label
                )));
            }
            body.insert(key, encoded);
        }
    }
    missing_keys(body, row)?;
    for (key, value) in body.iter() {
        if let Some(field) = spec.find(key) {
            state
                .set(&field.api_name, fsv_forms::Value::from_json(value))
                .map_err(other)?;
        }
    }
    let gate = state.rule_gate(&[]).map_err(other)?;
    if !gate.blockers.is_empty() {
        return Err(refused(format!(
            "ticket business rules block this write: {}",
            gate.blockers.join("; ")
        )));
    }
    if ack {
        state.acknowledge_all();
    }
    let acknowledged = acknowledged_texts(&state);
    let problems = state.errors().map_err(other)?;
    if !problems.is_empty() {
        return Err(refused(format!(
            "ticket form validation failed: {}{}",
            problems.join("; "),
            ack_hint(&state)
        )));
    }
    let token =
        build_br_validation_excludes(&state.evaluated, "ticket", &Value::Null, &Value::Null)
            .map_err(other)?;
    let metadata_hash = if answers.is_empty() {
        String::new()
    } else {
        meta.fingerprint().hash
    };
    Ok((token, metadata_hash, acknowledged))
}

/// Sorted texts of the blocking rule messages the reviewer acknowledged.
pub(crate) fn acknowledged_texts(state: &FormState) -> Vec<String> {
    let mut texts: Vec<String> = state
        .evaluated
        .blocking_messages()
        .into_iter()
        .filter(|m| state.acknowledged.contains(&m.key()))
        .map(|m| m.text.clone())
        .collect();
    texts.sort();
    texts
}

pub(crate) fn ack_hint(state: &FormState) -> &'static str {
    if state.unacknowledged().is_empty() {
        ""
    } else {
        "; pass --ack to acknowledge the blocking rule message(s) after reading them"
    }
}

/// Keyword arguments of [`build_plan_with`] (Python `build_plan` keywords).
#[derive(Debug, Clone, Copy, Default)]
pub struct PropertyArgs<'a> {
    pub tags: Option<&'a [String]>,
    pub planned_start: Option<&'a str>,
    pub planned_end: Option<&'a str>,
    /// Field name or label -> value, evaluated on the fresh edit form.
    pub answers: Option<&'a Map<String, Value>>,
}

/// `build_plan` without answers: validated tags/planned dates, the exact
/// `before` values and the business-rule token from fresh form metadata.
pub fn build_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    ticket_id: &str,
    tags: Option<&[String]>,
    planned_start: Option<&str>,
    planned_end: Option<&str>,
) -> Result<PropertyPlan> {
    let args = PropertyArgs {
        tags,
        planned_start,
        planned_end,
        answers: None,
    };
    Ok(build_plan_with(c, store, ticket_id, &args, None)?.plan)
}

/// Port of fsv.ticket_properties.build_plan: tags, planned dates and
/// metadata-backed answers (`--set` / core flags / TUI edits). `resolver`
/// resolves lookup answers (agent, group, requester, beneficiary); pass a
/// `formmeta::ClientResolver`.
pub fn build_plan_with(
    c: &dyn Api,
    store: &dyn MetaStore,
    ticket_id: &str,
    args: &PropertyArgs<'_>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<AnsweredPropertyPlan> {
    build_plan_ack(c, store, ticket_id, args, resolver, false)
}

/// [`build_plan_with`] with `ack`: acknowledged blocking rule messages are
/// bound to the review hash and listed in the review.
pub fn build_plan_ack(
    c: &dyn Api,
    store: &dyn MetaStore,
    ticket_id: &str,
    args: &PropertyArgs<'_>,
    resolver: Option<Arc<dyn LookupResolver>>,
    ack: bool,
) -> Result<AnsweredPropertyPlan> {
    let cid = parse_id(ticket_id, &TICKETS)?;
    if cid <= 0 {
        return Err(Error::value("ticket ID must be positive"));
    }
    if let Some(tags) = args.tags
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
    if let Some(tags) = args.tags {
        body.insert("tags".into(), json!(tags));
    }
    for (key, raw) in [
        ("planned_start_date", args.planned_start),
        ("planned_end_date", args.planned_end),
    ] {
        if let Some(raw) = raw {
            let Some(parsed) = crate::tasks::date(Some(raw))? else {
                return Err(Error::value("planned dates cannot be empty"));
            };
            body.insert(key.into(), json!(parsed));
        }
    }
    let empty = Map::new();
    let answers = args.answers.unwrap_or(&empty);
    if body.is_empty() && answers.is_empty() {
        return Err(Error::value(
            "specify --set, --tag, --planned-start or --planned-end",
        ));
    }
    let row = record(c, cid)?;
    missing_keys(&body, &row)?;
    let (token, metadata_hash, acknowledged) =
        match evaluate(c, store, cid, &row, &mut body, answers, resolver, ack) {
            Ok(t) => t,
            Err(e @ Error::Exit { .. }) => return Err(e),
            Err(_) => {
                return Err(refused(
                    "could not evaluate ticket edit form and rules; no update attempted",
                ));
            }
        };
    let before = values_of(&body, &row);
    body.insert("br_validation_excludes".into(), json!(token));
    Ok(AnsweredPropertyPlan {
        plan: PropertyPlan {
            ticket_id: cid,
            tenant: c.domain(),
            body_json: dumps_sorted(&Value::Object(body)),
            before_json: dumps_sorted(&Value::Object(before)),
            record_hash: snapshot_hash(&Value::Object(row))?,
        },
        answers_json: dumps_sorted(&Value::Object(answers.clone())),
        metadata_hash,
        acknowledged_json: fsv_py::Dumps::default()
            .ensure_ascii(false)
            .dump(&json!(acknowledged)),
    })
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    v?.as_array().map(|a| a.iter().map(py_str).collect())
}

/// `send` without a lookup resolver (enough for tag/date plans).
pub fn send(
    plan: &PropertyPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    store: &dyn MetaStore,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    send_with(&plan.clone().into(), c, journal, store, reviewed_hash, None)
}

/// Port of fsv.ticket_properties.send: one PUT after the plan (with its
/// stored answers) is rebuilt unchanged, then a read-back.
pub fn send_with(
    answered: &AnsweredPropertyPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    store: &dyn MetaStore,
    reviewed_hash: Option<&str>,
    resolver: Option<Arc<dyn LookupResolver>>,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled"}));
    };
    let plan = &answered.plan;
    let hash = answered.hash();
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
    let answers = answered.answers();
    let args = PropertyArgs {
        tags: tags.as_deref(),
        planned_start: payload.get("planned_start_date").and_then(Value::as_str),
        planned_end: payload.get("planned_end_date").and_then(Value::as_str),
        answers: Some(&answers),
    };
    let ack = !answered.acknowledged().is_empty();
    let fresh = build_plan_ack(c, store, &plan.ticket_id.to_string(), &args, resolver, ack)?;
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
            let matched = match row.get(key) {
                None => {
                    fields.insert(key.clone(), json!("unavailable"));
                    continue;
                }
                Some(actual) if key == "planned_start_date" || key == "planned_end_date" => {
                    same_instant(actual, expected)
                }
                Some(Value::Object(actual)) if key == "custom_fields" => expected
                    .as_object()
                    .into_iter()
                    .flatten()
                    .all(|(k, v)| actual.get(k) == Some(v)),
                Some(actual) => actual == expected,
            };
            fields.insert(
                key.clone(),
                json!(if matched { "matched" } else { "mismatched" }),
            );
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
pub(crate) mod http_tests;

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
            "specify --set, --tag, --planned-start or --planned-end"
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

    fn rich_form(extra: &[Value]) -> Value {
        let mut form = ticket_form(3);
        let fields = form["ticket_fields"].as_array_mut().unwrap();
        fields.extend([
            form_field(
                5,
                "priority",
                "Priority",
                "default_priority",
                5,
                json!({"choices": [{"id": 73, "value": "High"}]}),
            ),
            form_field(
                6,
                "cost_code",
                "Cost Code",
                "custom_text",
                6,
                json!({"workspace_id": 1}),
            ),
            form_field(
                7,
                "requested_for",
                "Requested For",
                "default_requested_for",
                7,
                json!({}),
            ),
        ]);
        fields.extend(extra.iter().cloned());
        form
    }

    fn rich_row() -> Value {
        json!({"id": 123, "subject": "Test", "status": 2, "priority": 1, "tags": ["old"],
               "planned_start_date": null, "planned_end_date": null,
               "custom_fields": {"cost_code": "old", "other": "retained"}})
    }

    fn rich_api(form: Value, rules: Value, row: Value) -> FnApi {
        let row = std::sync::Mutex::new(row);
        FnApi::new(move |call| {
            let mut row = row.lock().unwrap();
            match (call.method.as_str(), call.path.as_str()) {
                ("GET", "ticket_form_fields") => Ok(form.clone()),
                ("GET", p) if p.ends_with("business_rules") => Ok(rules.clone()),
                ("GET", "tickets/123") => Ok(json!({"ticket": *row})),
                ("PUT", "tickets/123") => {
                    for (k, v) in call.body.clone().unwrap().as_object().unwrap() {
                        if k != "br_validation_excludes" {
                            row[k] = v.clone();
                        }
                    }
                    Ok(json!({"ticket": *row}))
                }
                other => panic!("unexpected {other:?}"),
            }
        })
    }

    fn answers(pairs: &[(&str, &str)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect()
    }

    fn answer_plan(c: &FnApi, id: &str, a: &Map<String, Value>) -> Result<AnsweredPropertyPlan> {
        let args = PropertyArgs {
            answers: Some(a),
            ..Default::default()
        };
        build_plan_with(c, &MemStore::default(), id, &args, Some(Arc::new(People)))
    }

    struct People;

    impl LookupResolver for People {
        fn lookup(
            &self,
            _entity: &str,
            _query: &str,
            _field: &fsv_forms::Field,
        ) -> std::result::Result<Vec<fsv_forms::LookupValue>, fsv_forms::state::ResolverError>
        {
            Ok(vec![fsv_forms::LookupValue::new(
                902,
                "Acme User",
                Some("person@example.com"),
            )])
        }
    }

    /// `test_ticket_properties.py::test_full_properties_runtime_enums_and_custom_readback`.
    #[test]
    fn full_properties_runtime_enums_and_custom_readback() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        let want = &oracle["properties"]["full"];
        let c = rich_api(rich_form(&[]), json!({"business_rules": []}), rich_row());
        let a = answers(&[
            ("Subject", "Globex"),
            ("Priority", "High"),
            ("Cost Code", "new"),
        ]);
        let p = answer_plan(&c, "SR-123", &a).unwrap();
        let payload = p.payload();
        assert_eq!(payload["priority"], 73);
        assert_eq!(payload["subject"], "Globex");
        assert_eq!(payload["custom_fields"], json!({"cost_code": "new"}));
        assert_eq!(p.plan.body_json, want["body_json"]);
        assert_eq!(p.answers_json, want["answers_json"]);
        assert_eq!(p.metadata_hash, want["metadata_hash"]);
        assert_eq!(p.hash(), want["hash"]);
        let review = p.review();
        let after: Vec<&String> = review["after"].as_object().unwrap().keys().collect();
        assert_eq!(json!(after), want["after_keys"]);
        let j = MemJournal::default();
        let out = send_with(
            &p,
            &c,
            &j,
            &MemStore::default(),
            Some(&p.hash()),
            Some(Arc::new(People)),
        )
        .unwrap();
        assert_eq!(out["verified"], true, "{out}");
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_reviewed_cli_uses_full_hash_and_refuses_unproven_closure` (closure half).
    #[test]
    fn closed_status_answer_refused() {
        let closed = form_field(
            2,
            "status",
            "Status",
            "default_status",
            2,
            json!({"choices": [{"id": 2, "value": "Open"}, {"id": 5, "value": "Closed"}]}),
        );
        let mut form = rich_form(&[]);
        form["ticket_fields"][1] = closed;
        let c = rich_api(form, json!({"business_rules": []}), rich_row());
        assert!(err(answer_plan(&c, "123", &answers(&[("Status", "Closed")]))).contains("closure"));
        assert!(c.writes().is_empty());
    }

    /// `test_legacy_property_hash_identity_is_unchanged`.
    #[test]
    fn legacy_hash_identity_unchanged() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        let c = rich_api(rich_form(&[]), json!({"business_rules": []}), rich_row());
        let t = vec!["Acme".to_string()];
        let p = build_plan(&c, &MemStore::default(), "123", Some(&t), None, None).unwrap();
        let args = PropertyArgs {
            tags: Some(&t),
            ..Default::default()
        };
        let full = build_plan_with(&c, &MemStore::default(), "123", &args, None).unwrap();
        assert_eq!(
            full.metadata_hash,
            oracle["properties"]["legacy"]["metadata_hash"]
        );
        assert_eq!(full.hash(), p.hash());
        assert_eq!(p.hash(), oracle["properties"]["legacy"]["hash"]);
        assert_eq!(
            p.hash(),
            plan_hash(
                &json!({"tenant": p.tenant, "ticket_id": p.ticket_id, "payload": p.payload(),
                              "before": serde_json::from_str::<Value>(&p.before_json).unwrap(),
                              "record_hash": p.record_hash})
            )
        );
    }

    /// `test_tui_ticket_properties.py::test_requested_for_update_is_property_only_and_resolves_current_user`.
    #[test]
    fn requested_for_answer_resolves_beneficiary() {
        let mut row = rich_row();
        row["requested_for_id"] = json!(901);
        let c = rich_api(rich_form(&[]), json!({"business_rules": []}), row);
        let p = answer_plan(
            &c,
            "SR-123",
            &answers(&[("Requested For", "person@example.com")]),
        )
        .unwrap();
        assert_eq!(p.payload()["requested_for_id"], 902);
        let out = send_with(
            &p,
            &c,
            &MemJournal::default(),
            &MemStore::default(),
            Some(&p.hash()),
            Some(Arc::new(People)),
        )
        .unwrap();
        assert_eq!(out["verified"], true, "{out}");
    }

    /// `test_reviewed_update_blocking_message_needs_ack`.
    #[test]
    fn reviewed_update_blocking_message_needs_ack() {
        let rule = json!({"id": 9_000_008_101_i64, "name": "High needs lead", "module_type": "ticket",
            "workspace_id": 1, "state": "active", "user_type": "both", "form_type": "both",
            "rule_type": "client_side", "is_validation": true,
            "condition_data": {"all": [{"name": "priority", "operator": "is", "value": "73", "evaluate_on": "ticket"}]},
            "action_data": {"action": [{"name": ["High priority needs the Acme lead"], "property": "message"}]}});
        let c = rich_api(
            rich_form(&[]),
            json!({"business_rules": [rule]}),
            rich_row(),
        );
        let a = answers(&[("Priority", "High")]);
        let refused = err(answer_plan(&c, "INC-123", &a));
        assert!(
            refused.contains("acknowledge to continue") && refused.contains("--ack"),
            "{refused}"
        );
        let args = PropertyArgs {
            answers: Some(&a),
            ..Default::default()
        };
        let acked = build_plan_ack(
            &c,
            &MemStore::default(),
            "INC-123",
            &args,
            Some(Arc::new(People)),
            true,
        )
        .unwrap();
        assert_eq!(
            acked.review()["acknowledged_rule_messages"],
            json!(["High priority needs the Acme lead"])
        );
        let unacked = AnsweredPropertyPlan {
            acknowledged_json: "[]".into(),
            ..acked.clone()
        };
        assert_ne!(acked.hash(), unacked.hash());
        assert!(
            send_with(
                &unacked,
                &c,
                &MemJournal::default(),
                &MemStore::default(),
                Some(&unacked.hash()),
                Some(Arc::new(People))
            )
            .is_err()
        );
        assert!(c.writes().is_empty());
        let out = send_with(
            &acked,
            &c,
            &MemJournal::default(),
            &MemStore::default(),
            Some(&acked.hash()),
            Some(Arc::new(People)),
        )
        .unwrap();
        assert_eq!(out["verified"], true, "{out}");
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_beneficiary_edit_never_reuses_previous_person_rule_context`.
    #[test]
    fn beneficiary_edit_never_reuses_previous_rule_context() {
        let rule = json!({"id": 9_000_000_006_i64, "name": "Acme rule", "module_type": "ticket",
            "state": "active", "user_type": "both", "form_type": "both", "rule_type": "client_side",
            "condition_data": {"all": [{"name": "department_id", "operator": "is", "value": ["9"], "evaluate_on": "requested_for"}]},
            "action_data": {"action": [{"name": ["subject"], "property": "hide"}]}});
        let mut row = rich_row();
        row["requested_for_id"] = json!(901);
        let c = rich_api(
            rich_form(&[]),
            json!({"business_rules": [rule], "requested_for": {"department_id": "9"}}),
            row,
        );
        assert_eq!(
            err(answer_plan(
                &c,
                "SR-123",
                &answers(&[("Requested For", "person@example.com")])
            )),
            "ticket business rules block this write: rule 9000000006 'Acme rule': unavailable fields or context (requested_for 'department_id'); it could change Subject"
        );
        assert!(c.writes().is_empty());
    }
}
