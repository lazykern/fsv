//! Reviewed change resolution due date (`changes update --due-by`,
//! `change_due_by.py`).
//!
//! One `PATCH /api/_/changes/<id>` of only `{"due_by": ISO}` on the shared
//! reviewed contract: the timestamp must be ISO-8601 with an explicit UTC
//! offset (checked before any request is planned; a naive time is refused with
//! the tenant time zone as a hint), the plan hash binds tenant, change,
//! payload, a stable snapshot of the change and a record-state token,
//! `--yes` needs the full `--expect-plan`, the send is journaled once, and
//! readback compares the stored due date with the sent one as instants.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_forms::pydt::{DateTimeValue, datetime_fromisoformat};
use fsv_py::{Dumps, py_repr_str, py_str};

use crate::attachments::owner_row;
use crate::problem_properties::{prior_guard_hashes, stateful_context};
use crate::readback::same_instant;
use crate::reviewed_write::{WritePlan, execute_guarded, refused};
use crate::service::{Api, Journal, obj, stable_snapshot};
use crate::watchers::positive_id;

pub const ACTION: &str = "change_due_by";

/// `NothingToChange` and every other failure of building the plan.
#[derive(Debug)]
pub enum DueByError {
    NothingToChange(String),
    Failed(Error),
}

impl From<Error> for DueByError {
    fn from(e: Error) -> Self {
        DueByError::Failed(e)
    }
}

/// Port of change_due_by (a `StatefulWritePlan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueByPlan {
    pub write: WritePlan,
    pub state_json: String,
    pub prior_context: String,
}

impl DueByPlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }
}

fn tenant_zone(c: &dyn Api) -> Option<String> {
    let me = c.me().ok()?;
    let zone = me.get("time_zone")?.as_str()?;
    (!zone.trim().is_empty()).then(|| zone.to_string())
}

/// ISO-8601 date-time with an explicit offset (`Z` or `+07:00`).
pub fn parse_due_by(value: &str, c: Option<&dyn Api>) -> Result<DateTimeValue> {
    let text = fsv_core::pystr::strip(value);
    let Some((local, offset)) = datetime_fromisoformat(text) else {
        return Err(Error::value(format!(
            "--due-by {} is not an ISO-8601 date-time (e.g. 2030-06-09T18:00:00+07:00); nothing was sent",
            py_repr_str(value)
        )));
    };
    if !text.contains('T') && !text.contains(' ') {
        return Err(Error::value(format!(
            "--due-by {} has no time of day; give a date-time with an offset; nothing was sent",
            py_repr_str(value)
        )));
    }
    let Some(offset) = offset else {
        let hint = c
            .and_then(tenant_zone)
            .map(|zone| format!(" (the agent profile time zone is {zone})"))
            .unwrap_or_default();
        return Err(Error::value(format!(
            "--due-by {} has no UTC offset{hint}; add one, e.g. {}+07:00 or ...Z; nothing was sent",
            py_repr_str(value),
            local.strftime("%Y-%m-%dT%H:%M:%S")
        )));
    };
    Ok(DateTimeValue::new(local, offset))
}

/// Port of change_due_by.build_plan.
pub fn build_plan(
    c: &dyn Api,
    journal: &dyn Journal,
    target: i64,
    due_by: &str,
) -> std::result::Result<DueByPlan, DueByError> {
    parse_due_by(due_by, Some(c))?;
    let cid = positive_id(&json!(target))?;
    let row = owner_row(c, "changes", cid)?;
    if row.get("deleted") == Some(&json!(true)) {
        return Err(refused(format!("change {cid} is deleted; nothing was sent")).into());
    }
    if let Some(current) = row.get("due_by")
        && same_instant(current, &json!(due_by))
    {
        return Err(DueByError::NothingToChange(format!(
            "change {cid} is already due by {}; nothing to change",
            py_str(current)
        )));
    }
    let row = Value::Object(row);
    let token = journal.repeat_token(&[ACTION], &json!(cid), Some(&row));
    let context = json!({"change": stable_snapshot(&row)?});
    let (bound, state_json, prior_context) = stateful_context(&context, &token)?;
    Ok(DueByPlan {
        write: WritePlan {
            action: ACTION.into(),
            target: cid,
            tenant: c.domain(),
            method: "PATCH".into(),
            path: format!("changes/{cid}"),
            payload_json: Dumps::default()
                .sort_keys(true)
                .dump(&json!({"due_by": due_by})),
            context_hash: bound,
        },
        state_json,
        prior_context,
    })
}

/// Port of change_due_by.review (`StatefulWritePlan.review` plus `due_by`).
pub fn review(plan: &DueByPlan) -> Result<Map<String, Value>> {
    let mut out = plan.write.review();
    out.insert(
        "record_state".into(),
        serde_json::from_str(&plan.state_json).unwrap_or(Value::Null),
    );
    let payload = plan.write.payload();
    let sent = payload.get("due_by").and_then(Value::as_str).unwrap_or("");
    let parsed = parse_due_by(sent, None)?;
    let utc = DateTimeValue::new(
        jiff::tz::Offset::UTC.to_datetime(parsed.timestamp()),
        jiff::tz::Offset::UTC,
    );
    out.insert(
        "due_by".into(),
        json!({"sent": sent, "utc": utc.isoformat()}),
    );
    Ok(out)
}

/// Port of change_due_by.send.
pub fn send(
    plan: &DueByPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let payload = plan.write.payload();
    let due_by = payload
        .get("due_by")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let target = plan.write.target;
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let row = owner_row(c, "changes", target)?;
        let stored = row.get("due_by").cloned().unwrap_or(Value::Null);
        let present = row.contains_key("due_by");
        let matched = present && same_instant(&stored, &json!(due_by));
        let status = if matched {
            "matched"
        } else if !present {
            "unavailable"
        } else {
            "mismatched"
        };
        let mut out = obj(json!({
            "target": target, "fields": {"due_by": status}, "due_by": stored, "verified": matched,
        }));
        if let Some(manual) = row.get("manual_dueby") {
            out.insert("manual_dueby".into(), manual.clone());
        }
        Ok(out)
    };
    let rebuild = || {
        build_plan(c, journal, target, &due_by)
            .map(|p| p.hash())
            .map_err(|e| match e {
                DueByError::NothingToChange(m) => refused(m),
                DueByError::Failed(e) => e,
            })
    };
    execute_guarded(
        &plan.write,
        &plan.hash(),
        c,
        journal,
        reviewed_hash,
        &rebuild,
        &verify,
        None,
        &prior_guard_hashes(&plan.write, &plan.prior_context),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_refusals_name_the_problem() {
        for (bad, needle) in [
            ("not-a-date", "is not an ISO-8601 date-time"),
            ("2030-13-40T10:00:00+07:00", "is not an ISO-8601 date-time"),
            ("2030-06-09", "has no time of day"),
            (
                "2030-06-09T18:00:00",
                "has no UTC offset; add one, e.g. 2030-06-09T18:00:00+07:00 or ...Z",
            ),
        ] {
            let e = parse_due_by(bad, None).unwrap_err().to_string();
            assert!(e.contains(needle), "{bad}: {e}");
            assert!(e.ends_with("nothing was sent"), "{e}");
        }
        let ok = parse_due_by("2030-06-09T18:00:00+07:00", None).unwrap();
        assert_eq!(ok.timestamp().to_string(), "2030-06-09T11:00:00Z");
        assert!(parse_due_by("2030-06-09T11:00:00Z", None).is_ok());
    }
}
