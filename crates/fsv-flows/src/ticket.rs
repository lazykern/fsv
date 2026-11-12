//! Reviewed, metadata-driven incident creation; no automatic write retry
//! (`ticket_flow.py`).

use std::collections::HashSet;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::{Map, Value as Json, json};

use fsv_api::recipients::plain_email;
use fsv_api::service::{Api, Journal};
use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::review::Review;
use fsv_forms::{FormError, LookupValue, ModuleOptions, Value, build_ticket_payload};
use fsv_py::truthy;

use crate::change::{
    self, Plan, acknowledge, auto_suffix, dropped, flow_exit, messages, notes, split_special,
    unsent_suffix,
};
use crate::drafts::Draft;
use crate::meta::{
    FormMeta, MetaSource, RULES_UNAVAILABLE_KEY, Resolver, apply_answers, form_err, is_requester,
    is_status, rules_warnings,
};

pub const ACTION: &str = "ticket_create";

/// Keyword arguments of [`build_create_plan`].
#[derive(Clone, Default)]
pub struct TicketOptions {
    pub resolver: Option<Arc<dyn Resolver>>,
    pub requester: Option<LookupValue>,
    pub ack_all: bool,
    pub cc_emails: Vec<String>,
}

pub fn build_create_plan(
    meta: &Arc<FormMeta>,
    draft: &Draft,
    opts: &TicketOptions,
) -> Result<Plan> {
    let spec = meta.spec();
    let mut state = meta.state("new", change::lookup(&opts.resolver), None)?;
    let sp = split_special(meta, draft);
    let mut errors = sp.errors;
    if sp
        .requester_raw
        .as_ref()
        .is_some_and(|r| !r.is_null() && r.as_str() != Some(""))
    {
        errors.push(
            "requester must be specified with --requester or the draft requester key, not answers"
                .into(),
        );
    }
    if !draft.attachments.is_empty() || !draft.planning.is_empty() {
        errors.push("ticket attachments and planning fields are not supported".into());
    }
    let req_field = spec.fields().iter().find(|f| is_requester(f));
    let requester = opts.requester.as_ref();
    if let (Some(r), Some(rf)) = (requester, req_field) {
        match state.set(&rf.api_name, Value::Lookup(r.clone())) {
            Ok(_) => {}
            Err(e) if e.is_form_error() => errors.push(e.to_string()),
            Err(e) => return Err(form_err(e)),
        }
    }
    let mut failed = HashSet::new();
    errors.extend(apply_answers(&mut state, &sp.answers, Some(&mut failed))?);
    let warnings: Vec<String> = spec
        .warnings
        .iter()
        .cloned()
        .chain(rules_warnings(&meta.rules))
        .chain(acknowledge(&mut state, &draft.ack, opts.ack_all))
        .collect();
    let sf = spec.fields().iter().find(|f| is_status(f));
    let sf_visible = sf.is_some_and(|f| state.evaluated.is_visible(&f.api_name));
    let default = sf.and_then(|f| f.default.clone()).unwrap_or(Value::Null);
    let mut status = if sf_visible {
        default.clone()
    } else {
        Value::Null
    };
    let mut risks = Vec::new();
    if let Some(raw) = sp
        .status_raw
        .as_ref()
        .filter(|r| !r.is_null() && r.as_str() != Some(""))
    {
        match sf.filter(|_| sf_visible) {
            None => errors.push("status is not visible on this form".into()),
            Some(f) => match state.parse_field(f, &Value::from_json(raw)) {
                Ok(v) => {
                    status = v;
                    if status != default {
                        risks.push(
                            "Non-default status may trigger workflows or notifications".to_string(),
                        );
                    }
                }
                Err(e) if e.is_form_error() => errors.push(e.to_string()),
                Err(e) => return Err(form_err(e)),
            },
        }
    }
    let label = |n: &str| {
        spec.get(n)
            .map_or_else(|| n.to_string(), |f| f.label.clone())
    };
    let mut skip: HashSet<String> = req_field
        .map(|f| format!("{} is required", f.label))
        .into_iter()
        .collect();
    skip.extend(failed.iter().map(|n| format!("{} is required", label(n))));
    errors.extend(
        state
            .errors()
            .map_err(form_err)?
            .into_iter()
            .filter(|p| !skip.contains(p)),
    );
    // Unknown action targets or absent rule context can change which fields
    // are visible/required. Static checks are not a safe substitute.
    if meta.rules.get(RULES_UNAVAILABLE_KEY).is_some_and(truthy) {
        errors.push("business rules are unavailable; use the web form".into());
    }
    if !state.rules.unavailable_dependencies("new").is_empty()
        || state
            .rules
            .warnings()
            .iter()
            .any(|w| w.contains("unknown field") || w.contains("none was given"))
    {
        errors.push(
            "business rules reference unavailable fields or context; use the web form".into(),
        );
    }
    let sent = state.payload_values();
    for raw in meta.raw_fields() {
        if ["deleted", "is_deleted", "deprecated", "is_deprecated"]
            .iter()
            .any(|k| raw.get(*k) == Some(&Json::Bool(true)))
        {
            let name = raw
                .get("name")
                .filter(|v| truthy(v))
                .map(fsv_py::py_str)
                .unwrap_or_default();
            if let Some(f) = spec.find(&name)
                && sent.contains_key(&f.api_name)
            {
                errors.push(format!(
                    "{}: deprecated or deleted field cannot be sent",
                    f.label
                ));
            }
        }
    }
    let email = requester
        .and_then(|r| r.email.clone())
        .filter(|e| !e.is_empty());
    if email.is_none() {
        errors.insert(
            0,
            "requester email is required; use --requester or the draft requester key".into(),
        );
    }
    if opts.cc_emails.iter().any(|v| !plain_email(v)) {
        errors.push("CC recipients must be plain email addresses".into());
    }
    let mut payload = None;
    if let (Some(r), Some(email)) = (requester, &email) {
        let mopts = ModuleOptions {
            requester_email: Some(email.clone()),
            requester_name: Some(r.display.clone()),
            status: (!status.is_null()).then(|| status.clone()),
            allow_incomplete: !errors.is_empty(),
            ..ModuleOptions::default()
        };
        match build_ticket_payload(&state, &mopts) {
            Ok(mut p) => {
                if !opts.cc_emails.is_empty() {
                    p.insert("cc_emails".into(), json!(opts.cc_emails));
                }
                payload = Some(p);
            }
            Err(FormError::Payload { problems }) => errors.extend(problems),
            Err(e) => errors.push(e.to_string()),
        }
    }
    let mut review = Review::new("New Incident");
    review.rows.push((
        "Requester".into(),
        requester.map_or_else(|| "(not set)".to_string(), ToString::to_string),
    ));
    if !opts.cc_emails.is_empty() {
        review.rows.push(("CC".into(), opts.cc_emails.join(", ")));
    }
    for name in state.payload_values().keys() {
        let Some(f) = spec.get(name) else { continue };
        if !is_requester(f) && !is_status(f) {
            let shown = state.display(name, None).map_err(form_err)?
                + &auto_suffix(&state, name)?
                + unsent_suffix(f);
            review.rows.push((f.label.clone(), shown));
        }
    }
    if let Some(f) = sf
        && !status.is_null()
    {
        let shown = match f.choice(&status) {
            Ok(Some(c)) => c.label.clone(),
            _ => status.py_str(),
        };
        review.rows.push((format!("{} on create", f.label), shown));
    }
    review.notify = vec![
        "Creating an incident may notify its requester and assignees and start workflows.".into(),
    ];
    review.messages = messages(&state);
    review.notes = notes(&state)?;
    review.dropped = dropped(&state)?;
    review.risks = risks.clone();
    review.errors = errors.clone();
    review.warnings = warnings;
    review.metadata = meta.as_of();
    let mut files = IndexMap::new();
    files.insert("attachments".to_string(), Vec::new());
    let mut plan = Plan {
        action: "create".into(),
        meta: meta.clone(),
        state,
        draft: draft.clone(),
        payload,
        files,
        planning: Vec::new(),
        review,
        errors,
        requester: opts.requester.clone(),
        target: None,
        hash: String::new(),
        risks,
    };
    if let Some(p) = &plan.payload
        && plan.errors.is_empty()
    {
        plan.hash = change::hash(&Json::Object(p.clone()), &plan.files, &[], None)?;
        plan.review.plan_hash = plan.hash.clone();
    }
    Ok(plan)
}

/// An unresolved `started` event is never overridden by `--allow-duplicate`.
pub fn check_duplicate(plan: &Plan, allow_duplicate: bool, journal: &dyn Journal) -> Result<()> {
    let kind = format!("plan:{ACTION}");
    let rows: Vec<Json> = journal
        .read()
        .into_iter()
        .filter(|r| {
            r.get("kind").and_then(Json::as_str) == Some(&kind)
                && r.get("plan").and_then(Json::as_str) == Some(&plan.hash)
        })
        .collect();
    if rows
        .last()
        .and_then(|r| r.get("event"))
        .and_then(Json::as_str)
        == Some("started")
    {
        return Err(flow_exit(
            4,
            "previous incident create outcome is unknown; check the server before any retry",
        ));
    }
    if !allow_duplicate && journal.recent_plan(&plan.hash, ACTION, 24.0).is_some() {
        return Err(flow_exit(
            4,
            "this incident plan was already sent; check the server before --allow-duplicate",
        ));
    }
    Ok(())
}

fn unknown_outcome() -> Error {
    flow_exit(
        5,
        "incident create outcome unknown; check the server before any retry",
    )
}

pub fn execute(
    plan: &Plan,
    client: &dyn Api,
    journal: &dyn Journal,
    allow_duplicate: bool,
) -> Result<Json> {
    let Some(payload) = plan
        .payload
        .as_ref()
        .filter(|_| plan.ok() && !plan.hash.is_empty())
    else {
        return Err(flow_exit(2, "cannot create an incomplete incident plan"));
    };
    check_duplicate(plan, allow_duplicate, journal)?;
    // CSRF before the started mark: it only reads.
    let csrf = client.get_csrf()?;
    let before = journal.read().len();
    journal.record_plan("started", &plan.hash, ACTION, None, None);
    let entries = journal.read();
    let last = entries.last();
    if entries.len() <= before
        || last.and_then(|r| r.get("plan")).and_then(Json::as_str) != Some(&plan.hash)
        || last.and_then(|r| r.get("event")).and_then(Json::as_str) != Some("started")
    {
        return Err(flow_exit(
            5,
            "could not persist the incident journal; nothing was sent",
        ));
    }
    let body = fsv_py::Dumps::default()
        .ensure_ascii(false)
        .dump(&Json::Object(payload.clone()));
    let result =
        match client.send_once("POST", "tickets", Body::Raw(body.into_bytes()), None, &csrf) {
            Ok(r) => r,
            Err(e) => {
                let Some(status) = fsv_api::service::definite_rejection(&e) else {
                    return Err(unknown_outcome());
                };
                journal.record_plan("failed", &plan.hash, ACTION, None, None);
                return Err(flow_exit(
                    5,
                    format!("incident create rejected (HTTP {status}); no automatic retry"),
                ));
            }
        };
    let ticket = match &result {
        Json::Object(m) => Some(m.get("ticket").cloned().unwrap_or_else(|| result.clone())),
        _ => None,
    };
    let cid = ticket
        .as_ref()
        .and_then(|t| t.as_object())
        .and_then(|t| t.get("id"))
        .filter(|v| !v.is_null())
        .cloned();
    let Some(cid) = cid else {
        return Err(flow_exit(
            5,
            "incident create response has no ID; outcome unknown, check the server before retry",
        ));
    };
    journal.record_plan("done", &plan.hash, ACTION, None, Some(&cid));
    let mut out = Map::new();
    out.insert("id".into(), cid.clone());
    out.insert("result".into(), ticket.unwrap_or(Json::Null));
    out.insert("plan_hash".into(), json!(plan.hash));
    out.insert("steps".into(), json!([{"step": "create", "ok": true}]));
    if let Some(want) = payload.get("cc_emails") {
        let field = (|| -> Option<&'static str> {
            let raw = client
                .int_get(&format!("tickets/{}", fsv_py::py_str(&cid)), None)
                .ok()?;
            let row = match &raw {
                Json::Object(m) => m.get("ticket").cloned().unwrap_or_else(|| raw.clone()),
                _ => return None,
            };
            let row = row.as_object()?;
            if fsv_py::py_str(row.get("id").unwrap_or(&Json::Null)) != fsv_py::py_str(&cid) {
                return None;
            }
            Some(match row.get("cc_emails") {
                None => "unavailable",
                Some(v) if v == want => "matched",
                Some(_) => "mismatched",
            })
        })()
        .unwrap_or("unavailable");
        let mut v = Map::new();
        v.insert("verified".into(), json!(field == "matched"));
        v.insert("field".into(), json!(field));
        if field != "matched" {
            v.insert(
                "warning".into(),
                json!(
                    "Incident CC was not fully verified on exact-ticket read-back; do not resend."
                ),
            );
        }
        out.insert("verification".into(), Json::Object(v));
    }
    Ok(Json::Object(out))
}

#[allow(clippy::too_many_arguments)]
pub fn send(
    plan: Plan,
    client: &dyn Api,
    source: &dyn MetaSource,
    rebuild: &mut dyn FnMut(Arc<FormMeta>) -> Result<Plan>,
    journal: &dyn Journal,
    expected: Option<&str>,
    allow_duplicate: bool,
) -> Result<Json> {
    check_duplicate(&plan, allow_duplicate, journal)?;
    let (new, diff) = change::freshen(plan, source, rebuild)?;
    if diff.is_some() {
        return Err(flow_exit(
            4,
            "form metadata changed; review again with --dry-run",
        ));
    }
    change::expect(&new, expected)?;
    execute(&new, client, journal, allow_duplicate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Fail, FakeApi, MemJournal, acme_meta, golden};
    use serde_json::json;

    fn jane() -> LookupValue {
        LookupValue::new(9000007001i64, "Jane Doe", Some("jane.doe@example.com"))
    }

    fn golden_plan(case: &str) -> (Plan, &'static Json) {
        let g = golden(&["ticket", case]);
        let mut d = Draft {
            answers: g["answers"].as_object().unwrap().clone(),
            ..Draft::default()
        };
        if let Some(a) = g["extra"].get("attachments") {
            d.attachments = a
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
        }
        let cc = g["cc"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        let meta = Arc::new(acme_meta("ticket", "new_form"));
        let opts = TicketOptions {
            requester: Some(jane()),
            cc_emails: cc,
            ..TicketOptions::default()
        };
        (build_create_plan(&meta, &d, &opts).unwrap(), g)
    }

    fn without_metadata(mut j: Json) -> Json {
        j["review"]
            .as_object_mut()
            .unwrap()
            .shift_remove("metadata");
        j
    }

    #[test]
    fn golden_ticket_plans() {
        for case in ["basic", "cc", "bad_cc", "status_attach"] {
            let (p, g) = golden_plan(case);
            assert_eq!(json!(p.errors), g["errors"], "{case}");
            assert_eq!(p.hash, g["hash"].as_str().unwrap(), "{case}");
            assert_eq!(p.ok(), g["ok"].as_bool().unwrap());
            assert_eq!(json!(p.payload), g["payload"], "{case}");
            assert_eq!(
                fsv_py::canonical(&without_metadata(p.to_json())),
                fsv_py::canonical(&g["json"]),
                "{case}"
            );
            let keys = |j: &Json| j.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
            assert_eq!(keys(&p.to_json()), keys(&g["json"]), "{case} key order");
        }
    }

    #[test]
    fn valid_recipient_shapes() {
        use fsv_api::recipients::valid_recipient;
        assert!(valid_recipient("a@example.com"));
        assert!(valid_recipient("Name <a@example.com>"));
        assert!(!valid_recipient("a@example"));
        assert!(!valid_recipient("a@example.com\n"));
    }

    #[test]
    fn requester_missing_is_first_error() {
        let meta = Arc::new(acme_meta("ticket", "new_form"));
        let p = build_create_plan(&meta, &Draft::default(), &TicketOptions::default()).unwrap();
        assert_eq!(
            p.errors[0],
            "requester email is required; use --requester or the draft requester key"
        );
        assert!(p.payload.is_none() && p.hash.is_empty());
    }

    #[test]
    fn single_write_duplicate_and_unknown_outcome() {
        let (p, _) = golden_plan("basic");
        let (c, j) = (FakeApi::default(), MemJournal::default());
        let out = execute(&p, &c, &j, false).unwrap();
        assert_eq!(out["id"], json!(123));
        let w = c.writes();
        assert_eq!(
            (w.len(), w[0].0.as_str(), w[0].1.as_str()),
            (1, "POST", "tickets")
        );
        assert_eq!(w[0].2["csrf"], json!("synthetic-token"));
        assert_eq!(w[0].2["body"], json!(p.payload));
        assert_eq!(execute(&p, &c, &j, false).unwrap_err().exit_code(), 4);
        execute(&p, &c, &j, true).unwrap();
        assert_eq!(c.writes().len(), 2);
        let (q, _) = golden_plan("cc");
        *c.failure.lock().unwrap() = Some(Fail::Timeout);
        let e = execute(&q, &c, &j, false).unwrap_err();
        assert!(e.to_string().contains("outcome unknown") && e.exit_code() == 5);
        let e = execute(&q, &c, &j, true).unwrap_err();
        assert!(e.to_string().contains("outcome is unknown"));
        assert_eq!(c.writes().len(), 3);
    }

    #[test]
    fn rejection_is_recorded_403_is_unknown() {
        let (p, _) = golden_plan("basic");
        let (c, j) = (FakeApi::default(), MemJournal::default());
        *c.failure.lock().unwrap() = Some(Fail::Api(403));
        let e = execute(&p, &c, &j, false).unwrap_err().to_string();
        assert!(e.contains("outcome unknown") && !e.contains("response body"));
        assert_eq!(j.read().last().unwrap()["event"], json!("started"));
        *c.failure.lock().unwrap() = Some(Fail::Api(422));
        let j2 = MemJournal::default();
        let e = execute(&p, &c, &j2, false).unwrap_err().to_string();
        assert_eq!(e, "incident create rejected (HTTP 422); no automatic retry");
        assert_eq!(j2.read().last().unwrap()["event"], json!("failed"));
    }

    #[test]
    fn journal_not_persisted_sends_nothing() {
        let (p, _) = golden_plan("basic");
        let c = FakeApi::default();
        let j = MemJournal {
            drop_writes: true,
            ..MemJournal::default()
        };
        assert_eq!(
            execute(&p, &c, &j, false).unwrap_err().to_string(),
            "could not persist the incident journal; nothing was sent"
        );
        assert!(c.writes().is_empty());
        let (bad, _) = golden_plan("bad_cc");
        assert_eq!(execute(&bad, &c, &j, false).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn cc_read_back_verification() {
        let (p, _) = golden_plan("cc");
        let (c, j) = (FakeApi::default(), MemJournal::default());
        let out = execute(&p, &c, &j, false).unwrap();
        assert_eq!(out["verification"]["field"], json!("unavailable"));
        *c.ticket_read.lock().unwrap() =
            Some(json!({"ticket": {"id": 123, "cc_emails": ["a@example.com", "b@example.com"]}}));
        let out = execute(&p, &c, &j, true).unwrap();
        assert_eq!(
            out["verification"],
            json!({"verified": true, "field": "matched"})
        );
    }

    #[test]
    fn check_duplicate_started_blocks_even_with_allow() {
        let (p, _) = golden_plan("basic");
        let j = MemJournal::default();
        check_duplicate(&p, false, &j).unwrap();
        j.record_plan("started", &p.hash, ACTION, None, None);
        assert!(
            check_duplicate(&p, true, &j)
                .unwrap_err()
                .to_string()
                .contains("outcome is unknown")
        );
    }

    struct Same(FormMeta);
    impl MetaSource for Same {
        fn load_meta(&self, _m: &str, _mode: &str, _f: bool, _id: &Json) -> Result<FormMeta> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn send_checks_expected_hash_then_executes() {
        let (p, _) = golden_plan("basic");
        let (c, j) = (FakeApi::default(), MemJournal::default());
        let src = Same(acme_meta("ticket", "new_form"));
        let mut rebuild = |_m: Arc<FormMeta>| -> Result<Plan> { unreachable!() };
        let e = send(
            p.clone(),
            &c,
            &src,
            &mut rebuild,
            &j,
            Some("0000000000000000"),
            false,
        )
        .unwrap_err();
        assert_eq!(e.exit_code(), 4);
        let out = send(
            p.clone(),
            &c,
            &src,
            &mut rebuild,
            &j,
            Some(&p.hash[..16]),
            false,
        )
        .unwrap();
        assert_eq!(out["plan_hash"], json!(p.hash));
    }
}
