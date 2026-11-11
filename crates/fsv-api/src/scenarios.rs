//! Single-target scenario execution: the wire is known, side effects are not
//! provable (`scenarios.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::Result;
use fsv_core::resource::{TICKETS, parse_id};
use fsv_py::py_str;

use crate::canned;
use crate::properties::{dumps_sorted, record};
use crate::reviewed_write::{WritePlan, execute};
use crate::service::{Api, Journal, obj, plan_hash};
use crate::watchers::positive_id;

/// Port of fsv.scenarios.context: the scenario row and the ticket.
pub fn context(c: &dyn Api, target: i64, scenario_id: i64) -> Result<Value> {
    Ok(json!({"scenario": canned::scenario(c, &json!(scenario_id))?, "ticket": record(c, target)?}))
}

/// Port of fsv.scenarios.build_plan.
pub fn build_plan(c: &dyn Api, target: &str, scenario_id: &Value) -> Result<WritePlan> {
    let target = positive_id(&json!(parse_id(target, &TICKETS)?))?;
    let scenario_id = positive_id(scenario_id)?;
    Ok(WritePlan {
        action: "ticket_scenario".into(),
        target,
        tenant: c.domain(),
        method: "PUT".into(),
        path: "tickets/execute_scenario".into(),
        payload_json: dumps_sorted(&json!({"scenario_id": scenario_id, "ids": [target]})),
        context_hash: plan_hash(&context(c, target, scenario_id)?),
    })
}

/// Port of fsv.scenarios.send: reads the exact ticket back, but never
/// claims the scenario's effects.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let scenario_id = plan
        .payload()
        .get("scenario_id")
        .cloned()
        .unwrap_or(Value::Null);
    let target = plan.target.to_string();
    let rebuild = || build_plan(c, &target, &scenario_id);
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let row = record(c, plan.target)?;
        Ok(obj(json!({
            "ticket_id": plan.target,
            "identity_verified": py_str(row.get("id").unwrap_or(&Value::Null)) == target,
            "verified": false,
            "warning": "Exact ticket was read back; scenario side effects (including mail/workflows) are unverified. Do not resend.",
        })))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}
