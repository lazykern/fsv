//! Reviewed manual approval requests (`change_approvals.py`) and approval
//! decisions / cancellation (`change_approval_actions.py`).
//!
//! Requests are verified by new per-member approval ids; decisions by the
//! exact record's changed raw status; cancellation by complete approval
//! chains. Never trust a write response or replay a write.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, parse_id};

use crate::associations::SendOpts;
use crate::service::{
    Api, Journal, change_tab, definite_rejection, dumps_bytes, has_control, last_event, obj,
    plan_hash, positive_id,
};

/// `--rule` → `(approval_type, label)`.
pub const RULES: &[(&str, i64, &str)] = &[
    ("everyone", 1, "Everyone"),
    ("anyone", 2, "Anyone"),
    ("majority", 3, "Majority"),
    ("first-responder", 4, "First responder"),
];
const ACTION: &str = "change_approval_request";
const UNKNOWN: &str = "approval request outcome unknown; inspect the change and journal before any retry; do not resubmit";
const WARNING: &str = "Approval emails are irreversible; cancel/revoke cannot recall them. \
This requests approval; it does not approve the change. Tenant workflows may also notify people.";
const ACTION_WARNING: &str = "Approval actions notify people; approval/rejection may move the change workflow. \
Cancellation is irreversible. Review the exact tenant and identifiers.";
const ACTION_UNKNOWN: &str = "approval action outcome unknown; inspect the change and journal before any retry; do not resubmit";

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+$").expect("regex"));

/// `ApprovalError`: a safe user-facing failure (the CLI exits 2).
pub(crate) fn refused(msg: impl Into<String>) -> Error {
    Error::exit(2, msg)
}

fn rule(name: &str) -> Option<&'static (&'static str, i64, &'static str)> {
    RULES.iter().find(|(k, _, _)| *k == name)
}

/// Python `type(v) is int`.
fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

fn id_at(row: &Value, key: &str) -> Option<i64> {
    row.get(key).and_then(positive_id)
}

/// `_recipient`: a positive agent id or one plain email address.
pub fn recipient(value: &str) -> Result<()> {
    if has_control(value, "") {
        return Err(Error::value(
            "approver must be a positive agent ID or exact email",
        ));
    }
    if positive_id(&json!(value)).is_none() && !EMAIL.is_match(value) {
        return Err(Error::value(
            "approver must be a positive agent ID or exact email (no implicit self)",
        ));
    }
    Ok(())
}

/// `resolve_approvers`: explicit agent ids or unique active exact-email hits
/// (the autocomplete `id` is the agent id, never `user_id`).
pub fn resolve_approvers(c: &dyn Api, values: &[String]) -> Result<Vec<i64>> {
    if values.is_empty() {
        return Err(Error::value("at least one explicit approver is required"));
    }
    for v in values {
        recipient(v)?;
    }
    let mut resolved = Vec::new();
    for value in values {
        let aid = match positive_id(&json!(value)) {
            Some(id) => id,
            None => {
                let lookup = || -> Option<i64> {
                    let rows = c.autocomplete("agents", value, None).ok()?;
                    let wanted = fsv_core::pystr::casefold(value);
                    let matches: Vec<&Value> = rows
                        .iter()
                        .filter(|r| {
                            r.is_object()
                                && fsv_core::pystr::casefold(
                                    &r.get("email").map(fsv_py::py_str).unwrap_or_default(),
                                ) == wanted
                                && !r.get("deleted").is_some_and(fsv_py::truthy)
                                && !r.get("deactivated").is_some_and(fsv_py::truthy)
                                && !fsv_core::pystr::casefold(
                                    &r.get("value").map(fsv_py::py_str).unwrap_or_default(),
                                )
                                .contains("(deactivated)")
                        })
                        .collect();
                    if matches.len() != 1 {
                        return None;
                    }
                    id_at(matches[0], "id")
                };
                lookup().ok_or_else(|| {
                    refused("approver email must resolve to exactly one active agent ID; no POST attempted; use an explicit agent ID if lookup is unavailable")
                })?
            }
        };
        if resolved.contains(&aid) {
            return Err(Error::value(
                "duplicate approver; specify each agent exactly once",
            ));
        }
        resolved.push(aid);
    }
    Ok(resolved)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPlan {
    pub change_id: i64,
    pub name: String,
    pub message: String,
    pub approver_ids: Vec<i64>,
    pub tenant: String,
    pub rule: String,
}

impl ApprovalPlan {
    pub fn path(&self) -> String {
        format!("changes/{}/approval-groups", self.change_id)
    }

    fn rule_id(&self) -> i64 {
        rule(&self.rule).map_or(1, |r| r.1)
    }

    pub fn payload(&self) -> Value {
        let content = crate::service::escape_lines(&self.message);
        json!({"approval_groups": [{"name": self.name, "approval_type": self.rule_id(),
               "email_content": format!("<p dir=\"ltr\">{content}</p>"),
               "notified_to": self.approver_ids}]})
    }

    pub fn hash(&self) -> String {
        plan_hash(&json!({"tenant": self.tenant, "path": self.path(), "payload": self.payload()}))
    }

    pub fn review(&self) -> Map<String, Value> {
        obj(json!({
            "action": ACTION, "tenant": self.tenant, "change_id": self.change_id,
            "method": "POST", "path": format!("/api/_/{}", self.path()), "payload": self.payload(),
            "recipients": self.approver_ids, "approval_rule": rule(&self.rule).map_or("", |r| r.2),
            "message_text": self.message, "plan_hash": self.hash(), "warning": WARNING,
            "verification": "Complete before/after approval record IDs, exact new recipients, requested status and exact selected rule ID. When complete chains identify exactly one new matching group, verify its exact name and ID too.",
            "contract": "Per-member approvals and named approval-chain groups observed; POST response not trusted.",
        }))
    }
}

/// `build_plan`: pure local validation.
pub fn build_plan(
    change_id: &str,
    name: &str,
    message: &str,
    approver_ids: &[i64],
    rule_name: &str,
    tenant: &str,
) -> Result<ApprovalPlan> {
    let cid = parse_id(change_id, &CHANGES)?;
    if cid <= 0 {
        return Err(Error::value("change ID must be positive"));
    }
    if fsv_core::pystr::strip(name).is_empty() || has_control(name, "") {
        return Err(Error::value(
            "approval group name must be nonempty text without control characters",
        ));
    }
    if fsv_core::pystr::strip(message).is_empty() || has_control(message, "\r\n\t") {
        return Err(Error::value(
            "message must be nonempty plain text without unsafe control characters",
        ));
    }
    if approver_ids.is_empty() || approver_ids.iter().any(|a| *a <= 0) {
        return Err(Error::value(
            "at least one explicit positive agent ID is required",
        ));
    }
    if approver_ids.iter().collect::<BTreeSet<_>>().len() != approver_ids.len() {
        return Err(Error::value(
            "duplicate approver; specify each agent exactly once",
        ));
    }
    if rule(rule_name).is_none() {
        return Err(Error::value(
            "rule must be everyone, anyone, majority or first-responder",
        ));
    }
    Ok(ApprovalPlan {
        change_id: cid,
        name: name.into(),
        message: message.into(),
        approver_ids: approver_ids.to_vec(),
        tenant: tenant.into(),
        rule: rule_name.into(),
    })
}

/// One raw approvals page, validated before the paginator can drop rows.
fn strict_page(
    c: &dyn Api,
    path: &str,
    params: Option<Map<String, Value>>,
    seen: &mut BTreeSet<i64>,
    totals: &mut BTreeSet<i64>,
) -> Result<Value> {
    let data = c.int_get(path, params)?;
    let data = if data.is_array() {
        json!({"approvals": data})
    } else {
        data
    };
    if !data.is_object() || !data.get("approvals").is_some_and(Value::is_array) {
        return Err(refused("approval inventory is unreadable"));
    }
    let empty = json!({});
    for source in [&data, data.get("meta").unwrap_or(&empty)] {
        if !source.is_object()
            || source.get("partial").is_some_and(fsv_py::truthy)
            || source.get("complete") == Some(&json!(false))
        {
            return Err(refused("approval inventory is partial"));
        }
        for key in ["total_count", "count"] {
            if let Some(total) = source.get(key) {
                let text = fsv_py::py_str(total);
                if total.is_boolean()
                    || text.is_empty()
                    || !text.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(refused("approval inventory has an invalid total"));
                }
                totals.insert(text.parse().unwrap_or(i64::MAX));
            }
        }
    }
    for row in data["approvals"].as_array().into_iter().flatten() {
        match id_at(row, "id") {
            Some(aid) if !seen.contains(&aid) => {
                seen.insert(aid);
            }
            _ => {
                return Err(refused(
                    "approval inventory contains invalid or duplicate IDs",
                ));
            }
        }
    }
    Ok(data)
}

/// `_inventory`: the complete approvals list of a change, strictly validated.
pub fn inventory(c: &dyn Api, change_id: i64) -> Result<Vec<Value>> {
    let mut seen = BTreeSet::new();
    let mut totals = BTreeSet::new();
    let tab = change_tab(
        |p, q| strict_page(c, p, q, &mut seen, &mut totals),
        change_id,
        "approvals",
    )?;
    let pages = tab.meta.get("pages").and_then(Value::as_i64).unwrap_or(0);
    let missing = tab.missing.is_some() && pages == 0;
    if missing
        || tab.meta.get("complete") != Some(&json!(true))
        || tab.meta.get("partial").is_some_and(fsv_py::truthy)
        || tab.meta.get("has_more").is_some_and(fsv_py::truthy)
    {
        return Err(refused("approval inventory is incomplete"));
    }
    let ids: Vec<Option<i64>> = tab.rows.iter().map(|r| id_at(r, "id")).collect();
    if ids.contains(&None) || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(refused(
            "approval inventory contains invalid or duplicate IDs",
        ));
    }
    if !totals.is_empty() && totals != BTreeSet::from([ids.len() as i64]) {
        return Err(refused(
            "approval inventory does not match the declared total",
        ));
    }
    Ok(tab.rows)
}

fn nested_complete(node: &Value, meta_key: &str, count: usize) -> Result<()> {
    let empty = json!({});
    for source in [node, node.get(meta_key).unwrap_or(&empty)] {
        let flag = |k: &str| source.get(k).is_some_and(fsv_py::truthy);
        if !source.is_object()
            || flag("partial")
            || source.get("complete") == Some(&json!(false))
            || flag("has_more")
            || flag("has_next")
            || flag("next_page")
            || flag("next")
            || flag("links")
        {
            return Err(refused("nested approval chains inventory is incomplete"));
        }
        for key in ["count", "total_count"] {
            if let Some(v) = source.get(key)
                && as_int(v) != Some(count as i64)
            {
                return Err(refused("nested approval chains inventory total mismatch"));
            }
        }
    }
    Ok(())
}

fn valid_record_status(status: Option<&Value>) -> bool {
    match status {
        Some(v) if as_int(v).is_some_and(|i| i >= 0) => true,
        Some(Value::Object(m)) => {
            m.get("id").and_then(as_int).is_some_and(|i| i >= 0)
                && match m.get("name") {
                    None => true,
                    Some(Value::String(n)) => !fsv_core::pystr::strip(n).is_empty(),
                    Some(_) => false,
                }
        }
        _ => false,
    }
}

/// `get_change_approval_groups`: raw groups of complete approval chains.
/// Permission, malformed or pagination failures are errors, never empty.
pub fn get_change_approval_groups(c: &dyn Api, change_id: &Value) -> Result<Vec<Value>> {
    let Some(cid) = positive_id(change_id) else {
        return Err(Error::value("change ID must be positive"));
    };
    let mut groups = Vec::new();
    let (mut seen_chains, mut seen_groups, mut seen_records) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    let mut totals: BTreeSet<i64> = BTreeSet::new();
    let empty = json!({});
    for page in 1..=100i64 {
        let data = c.int_get(
            &format!("changes/{cid}/approval-chains"),
            Some(obj(json!({"page": page, "per_page": 100}))),
        )?;
        if !data.is_object() || !data.get("approval_chains").is_some_and(Value::is_array) {
            return Err(refused("approval chains inventory is unreadable"));
        }
        let mut more = false;
        for source in [&data, data.get("meta").unwrap_or(&empty)] {
            if !source.is_object()
                || source.get("partial").is_some_and(fsv_py::truthy)
                || source.get("complete") == Some(&json!(false))
            {
                return Err(refused("approval chains inventory is partial"));
            }
            for key in ["total_count", "count"] {
                if let Some(v) = source.get(key) {
                    match as_int(v).filter(|i| *i >= 0) {
                        Some(i) => totals.insert(i),
                        None => {
                            return Err(refused("approval chains inventory has an invalid total"));
                        }
                    };
                }
            }
            for key in ["has_next", "has_more"] {
                if let Some(v) = source.get(key) {
                    let Some(b) = v.as_bool() else {
                        return Err(refused("approval chains pagination is unreadable"));
                    };
                    more = more || b;
                }
            }
            if let Some(np) = source.get("next_page").filter(|v| !v.is_null()) {
                if as_int(np) != Some(page + 1) {
                    return Err(refused("approval chains pagination is unreadable"));
                }
                more = true;
            }
            if let Some(tp) = source.get("total_pages") {
                match as_int(tp) {
                    Some(t) if t >= page => more = more || page < t,
                    _ => return Err(refused("approval chains pagination is unreadable")),
                }
            }
            if source.get("next").is_some_and(fsv_py::truthy)
                || source.get("links").is_some_and(fsv_py::truthy)
            {
                return Err(refused("unsupported approval chains pagination"));
            }
        }
        let chains = data["approval_chains"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if more && chains.is_empty() {
            return Err(refused("approval chains inventory is incomplete"));
        }
        for chain in &chains {
            let id = if chain.is_object() {
                id_at(chain, "id")
            } else {
                None
            };
            let groups_ok = chain.get("approval_groups").is_some_and(Value::is_array);
            let Some(chain_id) = id.filter(|i| !seen_chains.contains(i) && groups_ok) else {
                return Err(refused(
                    "approval chains contain invalid or duplicate chain identities",
                ));
            };
            seen_chains.insert(chain_id);
            let chain_groups = chain["approval_groups"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            nested_complete(chain, "approval_chain_meta", chain_groups.len())?;
            for group in chain_groups {
                let gid = if group.is_object() {
                    id_at(&group, "id")
                } else {
                    None
                };
                let approvals_ok = group.get("approvals").is_some_and(Value::is_array);
                let Some(gid) = gid.filter(|i| !seen_groups.contains(i) && approvals_ok) else {
                    return Err(refused(
                        "approval chains contain invalid or duplicate group identities",
                    ));
                };
                seen_groups.insert(gid);
                let rows = group["approvals"].as_array().cloned().unwrap_or_default();
                nested_complete(&group, "approval_group_meta", rows.len())?;
                if group
                    .get("approval_status")
                    .and_then(as_int)
                    .is_none_or(|s| s < 0)
                {
                    return Err(refused("approval group status unreadable"));
                }
                for row in &rows {
                    let rid = if row.is_object() {
                        id_at(row, "id")
                    } else {
                        None
                    };
                    let mid = if row.is_object() {
                        id_at(row, "member_id")
                    } else {
                        None
                    };
                    let (Some(rid), Some(mid)) = (rid.filter(|r| !seen_records.contains(r)), mid)
                    else {
                        return Err(refused(
                            "approval group record identity unreadable or duplicate",
                        ));
                    };
                    seen_records.insert(rid);
                    if let Some(member) = row.get("member")
                        && (!member.is_object() || id_at(member, "id") != Some(mid))
                    {
                        return Err(refused("approval group member identity conflicting"));
                    }
                    if let Some(rc) = row.get("change_id")
                        && positive_id(rc) != Some(cid)
                    {
                        return Err(refused("approval group change identity conflicting"));
                    }
                    if !valid_record_status(row.get("status")) {
                        return Err(refused("approval group record status unreadable"));
                    }
                }
                groups.push(group);
            }
        }
        if !more {
            if !totals.is_empty() && totals != BTreeSet::from([seen_chains.len() as i64]) {
                return Err(refused(
                    "approval chains inventory does not match the declared total",
                ));
            }
            return Ok(groups);
        }
    }
    Err(refused("approval chains inventory page limit exceeded"))
}

/// `_verify_new`: one fresh requested selected-rule record per recipient, no extras.
pub fn verify_new(rows: &[Value], plan: &ApprovalPlan) -> Result<()> {
    let mut members = Vec::new();
    for row in rows {
        let aid = id_at(row, "member_id");
        let member = row.get("member").filter(|m| !m.is_null());
        let conflict = member.is_some_and(|m| !m.is_object() || id_at(m, "id") != aid);
        let Some(aid) = aid.filter(|_| !conflict) else {
            return Err(refused(
                "approval member identity unavailable or conflicting",
            ));
        };
        let everyone = plan.rule == "everyone";
        for (key, expected_id, expected_name) in [
            ("status", 0, Some("requested")),
            ("type", plan.rule_id(), everyone.then_some("everyone")),
        ] {
            let v = row.get(key);
            let ok = v.is_some_and(|v| {
                v.is_object()
                    && v.get("id").and_then(as_int) == Some(expected_id)
                    && expected_name
                        .is_none_or(|n| v.get("name").and_then(Value::as_str) == Some(n))
            });
            if !ok {
                return Err(refused(
                    "approval status or rule does not match the request",
                ));
            }
        }
        members.push(aid);
    }
    let want: BTreeSet<i64> = plan.approver_ids.iter().copied().collect();
    if members.len() != plan.approver_ids.len()
        || members.iter().copied().collect::<BTreeSet<_>>() != want
    {
        return Err(refused(
            "new approval records do not exactly match the recipients",
        ));
    }
    Ok(())
}

fn row_ids(rows: &[Value]) -> BTreeSet<i64> {
    rows.iter().filter_map(|r| id_at(r, "id")).collect()
}

/// `send`: one reviewed POST, then complete before/after record verification.
pub fn send(
    plan: &ApprovalPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    opts: SendOpts<'_>,
) -> Result<Value> {
    let rebuilt = build_plan(
        &plan.change_id.to_string(),
        &plan.name,
        &plan.message,
        &plan.approver_ids,
        &plan.rule,
        &c.domain(),
    );
    if rebuilt.ok().as_ref() != Some(plan) {
        return Err(refused(
            "approval plan or tenant changed; review a new plan",
        ));
    }
    if opts.dry_run {
        let mut out = obj(json!({"status": "dry_run"}));
        out.extend(plan.review());
        return Ok(Value::Object(out));
    }
    if opts.read_only {
        return Err(refused("read-only mode: no approval request was sent"));
    }
    let Some(reviewed) = opts.reviewed_hash else {
        return Ok(json!({"status": "cancelled", "change_id": plan.change_id}));
    };
    let hash = plan.hash();
    if reviewed != hash {
        return Err(refused(
            "approval review hash does not match; review the exact request again",
        ));
    }
    if plan.tenant.is_empty() {
        return Err(refused("configure a tenant before requesting approval"));
    }
    if matches!(
        last_event(journal, ACTION, &hash).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "this approval request was sent or its outcome is unknown; inspect the change and journal; do not resubmit",
        ));
    }
    let prepared = inventory(c, plan.change_id).and_then(|rows| c.get_csrf().map(|t| (rows, t)));
    let Ok((before_rows, csrf)) = prepared else {
        return Err(refused(
            "could not prepare a complete approval inventory/session; no approval POST was attempted",
        ));
    };
    let before_ids = row_ids(&before_rows);
    let before_groups = get_change_approval_groups(c, &json!(plan.change_id)).ok();
    let target = json!(plan.change_id);
    journal.record_plan("started", &hash, ACTION, Some(&target), None);
    if last_event(journal, ACTION, &hash).as_deref() != Some("started") {
        return Err(refused(
            "could not persist approval journal; no approval request was sent",
        ));
    }
    if let Err(e) = c.send_once(
        "POST",
        &plan.path(),
        Body::Raw(dumps_bytes(&plan.payload(), false)),
        None,
        &csrf,
    ) {
        if let Some(status) = definite_rejection(&e) {
            journal.record_plan("failed", &hash, ACTION, Some(&target), None);
            return Err(refused(format!(
                "approval request rejected (HTTP {status}); no automatic retry"
            )));
        }
        return Err(refused(UNKNOWN));
    }
    let verify = || -> Result<Vec<Value>> {
        let after_rows = inventory(c, plan.change_id)?;
        if !before_ids.is_subset(&row_ids(&after_rows)) {
            return Err(refused(UNKNOWN));
        }
        let exact: Vec<Value> = after_rows
            .into_iter()
            .filter(|r| !id_at(r, "id").is_some_and(|i| before_ids.contains(&i)))
            .collect();
        verify_new(&exact, plan)?;
        Ok(exact)
    };
    let exact = verify().map_err(|_| refused(UNKNOWN))?;
    let mut result = obj(json!({
        "status": "submitted", "change_id": plan.change_id,
        "approval_ids": exact.iter().filter_map(|r| id_at(r, "id")).collect::<Vec<_>>(),
        "group_name_verified": false, "verified": true, "readback": exact, "plan_hash": hash,
    }));
    if let Some(before_groups) = before_groups
        && let Ok(after_groups) = get_change_approval_groups(c, &json!(plan.change_id))
    {
        let baseline = row_ids(&before_groups);
        let current = row_ids(&after_groups);
        let fresh: Vec<&Value> = after_groups
            .iter()
            .filter(|g| !id_at(g, "id").is_some_and(|i| baseline.contains(&i)))
            .collect();
        if baseline.is_subset(&current) && fresh.len() == 1 {
            let group = fresh[0];
            let approvals = group["approvals"].as_array().cloned().unwrap_or_default();
            let members: Vec<Option<i64>> =
                approvals.iter().map(|r| id_at(r, "member_id")).collect();
            let records: BTreeMap<Option<i64>, &Value> =
                approvals.iter().map(|r| (id_at(r, "id"), r)).collect();
            let want: BTreeSet<Option<i64>> = plan.approver_ids.iter().map(|i| Some(*i)).collect();
            let exact_ids: BTreeSet<Option<i64>> = exact.iter().map(|r| id_at(r, "id")).collect();
            let ok = group.get("name").and_then(Value::as_str) == Some(plan.name.as_str())
                && members.len() == plan.approver_ids.len()
                && members.iter().copied().collect::<BTreeSet<_>>() == want
                && records.keys().copied().collect::<BTreeSet<_>>() == exact_ids
                && exact.iter().all(|r| {
                    records.get(&id_at(r, "id")).is_some_and(|rec| {
                        id_at(rec, "member_id") == id_at(r, "member_id")
                            && rec.get("status") == r.get("status")
                    })
                });
            if ok {
                result.insert("group_id".into(), json!(id_at(group, "id")));
                result.insert("group_name_verified".into(), json!(true));
            }
        }
    }
    journal.record_plan("done", &hash, ACTION, Some(&target), None);
    Ok(Value::Object(result))
}

// ---------------------------------------------------------------------------
// approval decisions and cancellation (`change_approval_actions.py`)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPlan {
    pub change_id: i64,
    pub target_id: i64,
    /// `approve` | `reject` | `cancel`.
    pub operation: String,
    pub remark: Option<String>,
    pub tenant: String,
}

impl ActionPlan {
    fn cancel(&self) -> bool {
        self.operation == "cancel"
    }

    pub fn action(&self) -> &'static str {
        if self.cancel() {
            "change_approval_cancel"
        } else {
            "change_approval_respond"
        }
    }

    pub fn path(&self) -> String {
        let suffix = if self.cancel() {
            format!("approval-groups/{}/cancel", self.target_id)
        } else {
            format!("approvals/{}", self.target_id)
        };
        format!("changes/{}/{suffix}", self.change_id)
    }

    /// `None` for cancel: no body at all, not JSON null or `{}`.
    pub fn payload(&self) -> Option<Value> {
        if self.cancel() {
            return None;
        }
        let mut body =
            obj(json!({"approval_status": if self.operation == "approve" { 1 } else { 2 }}));
        if let Some(r) = &self.remark {
            body.insert("remark".into(), json!(r));
        }
        Some(Value::Object(body))
    }

    fn payload_json(&self) -> Value {
        self.payload().unwrap_or(Value::Null)
    }

    pub fn hash(&self) -> String {
        plan_hash(
            &json!({"tenant": self.tenant, "method": "PUT", "path": self.path(), "payload": self.payload_json()}),
        )
    }

    pub fn review(&self) -> Map<String, Value> {
        let mut out = obj(json!({
            "action": self.action(), "operation": self.operation, "tenant": self.tenant,
            "change_id": self.change_id, "method": "PUT", "path": format!("/api/_/{}", self.path()),
            "payload": self.payload_json(), "body_present": self.payload().is_some(),
            "plan_hash": self.hash(), "warning": ACTION_WARNING,
            "contract": "Approve accepted live (requested=0, approved=1); reject captured aborted (read-back mapping unobserved).",
            "verification": "Complete approvals list; exact record ID and changed raw status. Named decision checked if exposed; no numeric mapping inferred.",
        }));
        out.insert(
            if self.cancel() {
                "group_id"
            } else {
                "approval_id"
            }
            .into(),
            json!(self.target_id),
        );
        if self.cancel() {
            out.insert("send_enabled".into(), json!(true));
            out.insert("contract".into(), json!("Bodyless JSON-media-type cancel accepted live; chains exposed cancelled status {id:3,name:cancelled}."));
            out.insert("verification".into(), json!("Complete approval chains: exact group disappears or all original approvals change away from requested; report raw statuses."));
        }
        out
    }
}

/// `build_plan` of `change_approval_actions`.
pub fn build_action_plan(
    change_id: &str,
    target_id: &Value,
    operation: &str,
    remark: Option<&str>,
    tenant: &str,
) -> Result<ActionPlan> {
    let cid = parse_id(change_id, &CHANGES)?;
    if cid <= 0 {
        return Err(Error::value("change ID must be positive"));
    }
    let Some(tid) = positive_id(target_id) else {
        return Err(Error::value(
            "approval record/group ID must be a positive integer",
        ));
    };
    if !["approve", "reject", "cancel"].contains(&operation) {
        return Err(Error::value(
            "choose exactly one of approve, reject or cancel",
        ));
    }
    if remark.is_some_and(|r| has_control(r, "\r\n\t")) {
        return Err(Error::value(
            "remark must be plain text without unsafe control characters",
        ));
    }
    if operation == "reject" && remark.is_none_or(|r| fsv_core::pystr::strip(r).is_empty()) {
        return Err(Error::value("reject requires a nonempty --remark"));
    }
    if operation == "cancel" && remark.is_some() {
        return Err(Error::value(
            "cancel sends no body and does not accept a remark",
        ));
    }
    Ok(ActionPlan {
        change_id: cid,
        target_id: tid,
        operation: operation.into(),
        remark: remark.map(str::to_string),
        tenant: tenant.into(),
    })
}

/// `_status`: the raw status, validated without inventing a mapping.
pub fn status(row: &Value) -> Result<Value> {
    let v = row.get("status");
    if valid_record_status(v) {
        return Ok(v.cloned().unwrap_or(Value::Null));
    }
    Err(refused("approval status unreadable"))
}

/// `_requested`.
pub fn requested(status: &Value) -> bool {
    *status == json!(0)
        || status.as_object().is_some_and(|m| {
            m.get("id") == Some(&json!(0)) || m.get("name") == Some(&json!("requested"))
        })
}

/// `_cancel_group`: the target group from complete chains, with a requested record.
pub fn cancel_group(c: &dyn Api, plan: &ActionPlan) -> Result<Value> {
    let groups = get_change_approval_groups(c, &json!(plan.change_id))
        .map_err(|_| refused("could not read complete approval chains; no cancel PUT attempted"))?;
    let Some(group) = groups
        .into_iter()
        .find(|g| id_at(g, "id") == Some(plan.target_id))
    else {
        return Err(refused(
            "approval group missing from complete chains inventory; no cancel PUT attempted",
        ));
    };
    let rows = group["approvals"].as_array().cloned().unwrap_or_default();
    let mut any_requested = false;
    for r in &rows {
        if requested(&status(r)?) {
            any_requested = true;
            break;
        }
    }
    if rows.is_empty() || !any_requested {
        return Err(refused(
            "approval group has no requested approval records; no cancel PUT attempted",
        ));
    }
    Ok(group)
}

fn status_rows(rows: &[Value]) -> Result<Vec<Value>> {
    rows.iter()
        .map(|r| {
            Ok(json!({"approval_id": r["id"], "member_id": r["member_id"], "status": status(r)?}))
        })
        .collect()
}

/// `_verify_cancel`: the group is gone, or every record left `requested`.
pub fn verify_cancel(c: &dyn Api, plan: &ActionPlan, before: &Value) -> Result<Value> {
    let groups = get_change_approval_groups(c, &json!(plan.change_id))?;
    let after = groups
        .into_iter()
        .find(|g| id_at(g, "id") == Some(plan.target_id));
    let before_rows = before["approvals"].as_array().cloned().unwrap_or_default();
    let mut statuses = Value::Null;
    if let Some(after) = &after {
        let after_rows = after["approvals"].as_array().cloned().unwrap_or_default();
        let baseline: BTreeMap<Option<i64>, &Value> =
            before_rows.iter().map(|r| (id_at(r, "id"), r)).collect();
        let current: BTreeMap<Option<i64>, &Value> =
            after_rows.iter().map(|r| (id_at(r, "id"), r)).collect();
        if baseline.keys().collect::<Vec<_>>() != current.keys().collect::<Vec<_>>() {
            return Err(refused(ACTION_UNKNOWN));
        }
        for (rid, row) in &current {
            if id_at(row, "member_id") != id_at(baseline[rid], "member_id")
                || requested(&status(row)?)
            {
                return Err(refused(ACTION_UNKNOWN));
            }
        }
        let mut changed = false;
        for (rid, row) in &baseline {
            if status(current[rid])? != status(row)? {
                changed = true;
            }
        }
        if !changed {
            return Err(refused(ACTION_UNKNOWN));
        }
        statuses = Value::Array(status_rows(&after_rows)?);
    }
    Ok(json!({
        "status": "submitted", "operation": "cancel", "change_id": plan.change_id,
        "group_id": plan.target_id, "verified": true,
        "verification_basis": if after.is_none() { "group_gone" } else { "changed_raw_statuses_only" },
        "before_status": status_rows(&before_rows)?,
        "observed_status": statuses, "readback": after, "plan_hash": plan.hash(),
        "warning": "Observed group disappearance/status transition only; cancellation status names and notification delivery are not inferred.",
    }))
}

/// Baseline of a respond action: the exact requested record.
fn respond_baseline(c: &dyn Api, plan: &ActionPlan) -> Result<(Vec<Value>, Value, Value, i64)> {
    let rows = inventory(c, plan.change_id)?;
    let before = rows
        .iter()
        .find(|r| id_at(r, "id") == Some(plan.target_id))
        .cloned()
        .ok_or_else(|| refused("target missing"))?;
    let before_status = status(&before)?;
    if before_status != json!({"id": 0, "name": "requested"}) {
        return Err(refused("target approval is not requested"));
    }
    let Some(member_id) = id_at(&before, "member_id") else {
        return Err(refused("approval member identity unavailable"));
    };
    if let Some(member) = before.get("member").filter(|m| !m.is_null())
        && (!member.is_object() || id_at(member, "id") != Some(member_id))
    {
        return Err(refused("approval member identity conflicting"));
    }
    if let Some(cid) = before.get("change_id")
        && positive_id(cid) != Some(plan.change_id)
    {
        return Err(refused("approval change identity conflicting"));
    }
    Ok((rows, before, before_status, member_id))
}

/// `send` of `change_approval_actions`: one reviewed PUT, then read-back.
pub fn send_action(
    plan: &ActionPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    opts: SendOpts<'_>,
) -> Result<Value> {
    let rebuilt = build_action_plan(
        &plan.change_id.to_string(),
        &json!(plan.target_id),
        &plan.operation,
        plan.remark.as_deref(),
        &c.domain(),
    );
    if rebuilt.ok().as_ref() != Some(plan) {
        return Err(refused(
            "approval action plan or tenant changed; review a new plan",
        ));
    }
    if opts.dry_run {
        let mut review = plan.review();
        if plan.cancel() {
            review.insert("group".into(), cancel_group(c, plan)?);
        }
        let mut out = obj(json!({"status": "dry_run"}));
        out.extend(review);
        return Ok(Value::Object(out));
    }
    if opts.read_only {
        return Err(refused("read-only mode: no approval action was sent"));
    }
    let Some(reviewed) = opts.reviewed_hash else {
        return Ok(json!({"status": "cancelled", "change_id": plan.change_id}));
    };
    let hash = plan.hash();
    if reviewed != hash {
        return Err(refused(
            "approval action review hash does not match; review the exact action again",
        ));
    }
    if plan.tenant.is_empty() {
        return Err(refused("configure a tenant before an approval action"));
    }
    let action = plan.action();
    if matches!(
        last_event(journal, action, &hash).as_deref(),
        Some("started" | "done")
    ) {
        return Err(refused(
            "this approval action was sent or its outcome is unknown; inspect the change and journal; do not resubmit",
        ));
    }
    let generic = "could not prepare complete approvals inventory/session with the exact requested record; no approval PUT attempted";
    let (before_rows, before, before_status, member_id) = if plan.cancel() {
        let group = cancel_group(c, plan)?;
        let rows = group["approvals"].as_array().cloned().unwrap_or_default();
        (rows, group, Value::Null, 0)
    } else {
        respond_baseline(c, plan).map_err(|_| refused(generic))?
    };
    let csrf = c.get_csrf().map_err(|_| refused(generic))?;
    let target = json!(plan.change_id);
    journal.record_plan("started", &hash, action, Some(&target), None);
    if last_event(journal, action, &hash).as_deref() != Some("started") {
        return Err(refused(
            "could not persist approval action journal; no approval PUT attempted",
        ));
    }
    let body = match plan.payload() {
        Some(p) => dumps_bytes(&p, false),
        None => Vec::new(),
    };
    if let Err(e) = c.send_once("PUT", &plan.path(), Body::Raw(body), None, &csrf) {
        if let Some(code) = definite_rejection(&e) {
            journal.record_plan("failed", &hash, action, Some(&target), None);
            return Err(refused(format!(
                "approval action rejected (HTTP {code}); no automatic retry"
            )));
        }
        return Err(refused(ACTION_UNKNOWN));
    }
    if plan.cancel() {
        let result = verify_cancel(c, plan, &before).map_err(|_| refused(ACTION_UNKNOWN))?;
        journal.record_plan("done", &hash, action, Some(&target), None);
        return Ok(result);
    }
    let expected = if plan.operation == "approve" {
        "approved"
    } else {
        "rejected"
    };
    let check = || -> Result<(Value, Value, Option<String>)> {
        let after_rows = inventory(c, plan.change_id)?;
        let exact = after_rows
            .iter()
            .find(|r| id_at(r, "id") == Some(plan.target_id))
            .cloned()
            .ok_or_else(|| refused(ACTION_UNKNOWN))?;
        if !row_ids(&before_rows).is_subset(&row_ids(&after_rows))
            || id_at(&exact, "member_id") != id_at(&before, "member_id")
        {
            return Err(refused(ACTION_UNKNOWN));
        }
        if let Some(member) = exact.get("member").filter(|m| !m.is_null())
            && (!member.is_object() || id_at(member, "id") != Some(member_id))
        {
            return Err(refused(ACTION_UNKNOWN));
        }
        if let Some(cid) = exact.get("change_id")
            && positive_id(cid) != Some(plan.change_id)
        {
            return Err(refused(ACTION_UNKNOWN));
        }
        let raw = status(&exact)?;
        let raw_id = if raw.is_object() {
            raw.get("id").cloned().unwrap_or(Value::Null)
        } else {
            raw.clone()
        };
        let name = raw.get("name").and_then(Value::as_str).map(str::to_string);
        if raw == before_status || raw_id == json!(0) || name.as_deref() == Some("requested") {
            return Err(refused(ACTION_UNKNOWN));
        }
        if name.as_deref().is_some_and(|n| {
            ["approved", "rejected", "cancelled", "canceled"].contains(&n) && n != expected
        }) {
            return Err(refused(ACTION_UNKNOWN));
        }
        Ok((exact, raw, name))
    };
    let (exact, raw, name) = check().map_err(|_| refused(ACTION_UNKNOWN))?;
    journal.record_plan("done", &hash, action, Some(&target), None);
    let decided = name.as_deref() == Some(expected);
    let mut result = obj(json!({
        "status": "submitted", "change_id": plan.change_id, "approval_id": plan.target_id,
        "operation": plan.operation, "verified": true, "decision_verified": decided,
        "verification_basis": if decided { "named_decision" } else { "changed_raw_status_only" },
        "before_status": before_status, "observed_status": raw, "readback": exact, "plan_hash": hash,
    }));
    if !decided {
        result.insert("warning".into(), json!("Raw status changed; decision name/mapping unobserved. Only the status transition is verified; do not resend."));
    }
    Ok(Value::Object(result))
}

/// Flags of `_run`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunOpts<'a> {
    pub dry_run: bool,
    pub read_only: bool,
    pub yes: bool,
    pub expected: Option<&'a str>,
    pub no_input: bool,
    pub json_out: bool,
}

/// What `_run` echoed: `(to_stderr, text)` lines in order.
pub type Echo = Vec<(bool, String)>;

fn indent2(v: &Value) -> String {
    fsv_py::Dumps::default().indent(2).dump(v)
}

/// `_run`: the CLI review/confirm/send sequence of an approval action.
/// `confirm` answers the final prompt (`typer.confirm`). On error the
/// lines already echoed are dropped; use [`run_action_detailed`] to keep them.
pub fn run_action(
    plan: &ActionPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    opts: RunOpts<'_>,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<Echo> {
    run_action_detailed(plan, c, journal, opts, confirm).map_err(|e| e.error)
}

/// Port of fsv.change_approvals._run failing after it already echoed:
/// `echo` holds the lines Python printed before raising `error`.
#[derive(Debug)]
pub struct RunActionError {
    pub echo: Echo,
    pub error: Error,
}

impl std::fmt::Display for RunActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for RunActionError {}

impl From<RunActionError> for Error {
    fn from(e: RunActionError) -> Error {
        e.error
    }
}

/// Port of fsv.change_approvals._run keeping the echoed review on error.
pub fn run_action_detailed(
    plan: &ActionPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    opts: RunOpts<'_>,
    confirm: &dyn Fn(&str) -> bool,
) -> std::result::Result<Echo, RunActionError> {
    let mut echo = Echo::new();
    match run_into(&mut echo, plan, c, journal, opts, confirm) {
        Ok(()) => Ok(echo),
        Err(error) => Err(RunActionError { echo, error }),
    }
}

fn run_into(
    echo: &mut Echo,
    plan: &ActionPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    opts: RunOpts<'_>,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<()> {
    if opts.read_only && !opts.dry_run {
        return Err(refused("read-only mode: no approval action was sent"));
    }
    if opts.dry_run {
        let out = send_action(
            plan,
            c,
            journal,
            SendOpts {
                dry_run: true,
                ..Default::default()
            },
        )?;
        echo.push((false, indent2(&out)));
        return Ok(());
    }
    let hash = plan.hash();
    if opts.expected.is_some_and(|e| e != hash) {
        return Err(refused(
            "approval action review hash does not match; review the exact action again",
        ));
    }
    if opts.yes {
        if opts.expected != Some(hash.as_str()) {
            return Err(refused(
                "--yes requires the full --expect-plan from a prior approval action review",
            ));
        }
    } else if opts.no_input {
        return Err(refused(
            "sending requires --yes and --expect-plan in no-input mode",
        ));
    }
    let mut review = plan.review();
    if plan.cancel() {
        review.insert("group".into(), cancel_group(c, plan)?);
    }
    echo.push((true, indent2(&Value::Object(review))));
    if !opts.yes
        && !confirm(
            "Send this exact approval action (notifies people; may move workflow; cancel irreversible)?",
        )
    {
        echo.push((false, "Cancelled; no approval action was sent.".into()));
        return Ok(());
    }
    let result = send_action(
        plan,
        c,
        journal,
        SendOpts {
            reviewed_hash: Some(&hash),
            read_only: opts.read_only,
            dry_run: false,
        },
    )?;
    if opts.json_out {
        echo.push((false, indent2(&result)));
    } else {
        let what = if plan.cancel() { "group" } else { "record" };
        echo.push((
            false,
            format!(
                "Approval {what} {} submitted on change {}; observed raw status: {}; verification: {}",
                plan.target_id,
                plan.change_id,
                fsv_py::Dumps::default().dump(&result["observed_status"]),
                fsv_py::py_str(&result["verification_basis"])
            ),
        ));
        if let Some(w) = result.get("warning").filter(|w| fsv_py::truthy(w)) {
            echo.push((true, fsv_py::py_str(w)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    const RUN_DECLINE_REVIEW: &str = r#"{
  "action": "change_approval_respond",
  "operation": "reject",
  "tenant": "acme.freshservice.com",
  "change_id": 12,
  "method": "PUT",
  "path": "/api/_/changes/12/approvals/101",
  "payload": {
    "approval_status": 2,
    "remark": "no"
  },
  "body_present": true,
  "plan_hash": "b3e8da8d89f5333a78ad68b31f7715d4acac7eb1bb62193fe425ce9f8e324b34",
  "warning": "Approval actions notify people; approval/rejection may move the change workflow. Cancellation is irreversible. Review the exact tenant and identifiers.",
  "contract": "Approve accepted live (requested=0, approved=1); reject captured aborted (read-back mapping unobserved).",
  "verification": "Complete approvals list; exact record ID and changed raw status. Named decision checked if exposed; no numeric mapping inferred.",
  "approval_id": 101
}"#;
    const RUN_DRY: &str = r#"{
  "status": "dry_run",
  "action": "change_approval_respond",
  "operation": "reject",
  "tenant": "acme.freshservice.com",
  "change_id": 12,
  "method": "PUT",
  "path": "/api/_/changes/12/approvals/101",
  "payload": {
    "approval_status": 2,
    "remark": "no"
  },
  "body_present": true,
  "plan_hash": "b3e8da8d89f5333a78ad68b31f7715d4acac7eb1bb62193fe425ce9f8e324b34",
  "warning": "Approval actions notify people; approval/rejection may move the change workflow. Cancellation is irreversible. Review the exact tenant and identifiers.",
  "contract": "Approve accepted live (requested=0, approved=1); reject captured aborted (read-back mapping unobserved).",
  "verification": "Complete approvals list; exact record ID and changed raw status. Named decision checked if exposed; no numeric mapping inferred.",
  "approval_id": 101
}"#;
    const RUN_CANCEL_REVIEW: &str = r#"{
  "action": "change_approval_cancel",
  "operation": "cancel",
  "tenant": "acme.freshservice.com",
  "change_id": 12,
  "method": "PUT",
  "path": "/api/_/changes/12/approval-groups/50/cancel",
  "payload": null,
  "body_present": false,
  "plan_hash": "1f4a2a31268a841edcf67ddefe9782a5f6f295ff9d8fd546cbf49eb7eed1e201",
  "warning": "Approval actions notify people; approval/rejection may move the change workflow. Cancellation is irreversible. Review the exact tenant and identifiers.",
  "contract": "Bodyless JSON-media-type cancel accepted live; chains exposed cancelled status {id:3,name:cancelled}.",
  "verification": "Complete approval chains: exact group disappears or all original approvals change away from requested; report raw statuses.",
  "group_id": 50,
  "send_enabled": true,
  "group": {
    "id": 50,
    "name": "CAB",
    "approval_status": 0,
    "approvals": [
      {
        "id": 1,
        "member_id": 7,
        "status": {
          "id": 0,
          "name": "requested"
        }
      }
    ]
  }
}"#;
    const RUN_CANCEL_RESULT: &str = r#"{
  "status": "submitted",
  "operation": "cancel",
  "change_id": 12,
  "group_id": 50,
  "verified": true,
  "verification_basis": "group_gone",
  "before_status": [
    {
      "approval_id": 1,
      "member_id": 7,
      "status": {
        "id": 0,
        "name": "requested"
      }
    }
  ],
  "observed_status": null,
  "readback": null,
  "plan_hash": "1f4a2a31268a841edcf67ddefe9782a5f6f295ff9d8fd546cbf49eb7eed1e201",
  "warning": "Observed group disappearance/status transition only; cancellation status names and notification delivery are not inferred."
}"#;

    use super::*;
    use crate::service::testing::*;

    const T: &str = "acme.freshservice.com";
    const AP_HASH: &str = "f29a229eb1b6c509f36896fdb131c314bb0eb444f68531059b6ab065230d113f";
    const RJ_HASH: &str = "b3e8da8d89f5333a78ad68b31f7715d4acac7eb1bb62193fe425ce9f8e324b34";
    const CN_HASH: &str = "1f4a2a31268a841edcf67ddefe9782a5f6f295ff9d8fd546cbf49eb7eed1e201";
    const CANCEL_WARNING: &str = "Observed group disappearance/status transition only; cancellation status names and notification delivery are not inferred.";
    const CONFIRM: &str = "Send this exact approval action (notifies people; may move workflow; cancel irreversible)?";

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    fn req() -> Value {
        json!({"id": 0, "name": "requested"})
    }

    fn row(id: i64, member: i64) -> Value {
        json!({"id": id, "member_id": member, "status": req(), "type": {"id": 1, "name": "everyone"}})
    }

    fn with(mut v: Value, key: &str, val: Value) -> Value {
        v[key] = val;
        v
    }

    fn grp(id: i64, rows: Value) -> Value {
        json!({"id": id, "name": "CAB", "approval_status": 0, "approvals": rows})
    }

    fn chains(groups: &[Value]) -> Value {
        json!({"approval_chains": [{"id": 1, "approval_groups": groups}]})
    }

    fn before_group() -> Value {
        grp(50, json!([{"id": 1, "member_id": 7, "status": req()}]))
    }

    fn ap() -> ActionPlan {
        build_action_plan("CHN-12", &json!("101"), "approve", Some("fine"), T).unwrap()
    }

    fn rj() -> ActionPlan {
        build_action_plan("12", &json!(101), "reject", Some("no"), T).unwrap()
    }

    fn cn() -> ActionPlan {
        build_action_plan("12", &json!(50), "cancel", None, T).unwrap()
    }

    fn chain_api(groups: &[Value]) -> FakeApi {
        FakeApi::new().ok("GET changes/12/approval-chains", chains(groups))
    }

    #[test]
    fn recipient_rule() {
        assert!(recipient("7").is_ok() && recipient("a@b.co").is_ok());
        assert_eq!(
            err(recipient("a\x01")),
            "approver must be a positive agent ID or exact email"
        );
        assert_eq!(
            err(recipient("me")),
            "approver must be a positive agent ID or exact email (no implicit self)"
        );
    }

    #[test]
    fn resolve_approvers_uses_agent_id() {
        let api = FakeApi::new().ok("GET autocomplete/agents", json!([{"email": "Ann@x.io", "id": 21, "user_id": 99},
                                                                      {"email": "old@x.io", "id": 22, "value": "Old (Deactivated)"}]));
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            resolve_approvers(&api, &s(&["5", "ann@X.io"])).unwrap(),
            [5, 21]
        );
        assert_eq!(
            err(resolve_approvers(&api, &[])),
            "at least one explicit approver is required"
        );
        assert_eq!(
            err(resolve_approvers(&api, &s(&["21", "ann@x.io"]))),
            "duplicate approver; specify each agent exactly once"
        );
        assert_eq!(
            err(resolve_approvers(&api, &s(&["old@x.io"]))),
            "approver email must resolve to exactly one active agent ID; no POST attempted; use an explicit agent ID if lookup is unavailable"
        );
        assert_eq!(
            err(resolve_approvers(&api, &s(&["me"]))),
            "approver must be a positive agent ID or exact email (no implicit self)"
        );
    }

    #[test]
    fn request_plan_matches_python() {
        let p = build_plan(
            "CHN-12",
            "CAB",
            "Please\r\nreview <now>",
            &[7, 8],
            "anyone",
            T,
        )
        .unwrap();
        let payload = json!({"approval_groups": [{"name": "CAB", "approval_type": 2, "email_content": "<p dir=\"ltr\">Please<br>review &lt;now&gt;</p>", "notified_to": [7, 8]}]});
        let h = "7f34fb9adce9ce2a92cbf6f9ff6c4239eb8cac53e6c4a70b2d1dc23496111c07";
        assert_eq!(
            (p.path(), p.payload(), p.hash()),
            (
                "changes/12/approval-groups".to_string(),
                payload.clone(),
                h.to_string()
            )
        );
        assert_eq!(
            Value::Object(p.review()),
            json!({"action": "change_approval_request", "tenant": T, "change_id": 12, "method": "POST", "path": "/api/_/changes/12/approval-groups",
                   "payload": payload, "recipients": [7, 8], "approval_rule": "Anyone", "message_text": "Please\r\nreview <now>", "plan_hash": h,
                   "warning": WARNING,
                   "verification": "Complete before/after approval record IDs, exact new recipients, requested status and exact selected rule ID. When complete chains identify exactly one new matching group, verify its exact name and ID too.",
                   "contract": "Per-member approvals and named approval-chain groups observed; POST response not trusted."})
        );
        assert_eq!(
            err(build_plan("12", " ", "m", &[1], "everyone", T)),
            "approval group name must be nonempty text without control characters"
        );
        assert_eq!(
            err(build_plan("12", "n", "a\x07", &[1], "everyone", T)),
            "message must be nonempty plain text without unsafe control characters"
        );
        assert_eq!(
            err(build_plan("12", "n", "m", &[0], "everyone", T)),
            "at least one explicit positive agent ID is required"
        );
        assert_eq!(
            err(build_plan("12", "n", "m", &[1, 1], "everyone", T)),
            "duplicate approver; specify each agent exactly once"
        );
        assert_eq!(
            err(build_plan("12", "n", "m", &[1], "all", T)),
            "rule must be everyone, anyone, majority or first-responder"
        );
        assert_eq!(
            err(build_plan("0", "n", "m", &[1], "everyone", T)),
            "change ID must be positive"
        );
    }

    #[test]
    fn inventory_strict_pages() {
        let api = FakeApi::new().ok("GET changes/12/approvals", json!([row(1, 7)]));
        assert_eq!(inventory(&api, 12).unwrap(), [row(1, 7)]);
        let env = FakeApi::new().ok(
            "GET changes/12/approvals",
            json!({"approvals": [row(1, 7)], "meta": {"total_count": 1}}),
        );
        assert_eq!(inventory(&env, 12).unwrap(), [row(1, 7)]);
        for bad in [
            json!({"approvals": [row(1, 7)], "meta": {"total_count": 2}}),
            json!([row(1, 7), row(1, 8)]),
            json!({"approvals": [], "partial": true}),
        ] {
            let api = FakeApi::new().ok("GET changes/12/approvals", bad);
            assert_eq!(err(inventory(&api, 12)), "approval inventory is incomplete");
        }
        assert_eq!(
            err(inventory(&FakeApi::new(), 12)),
            "approval inventory is incomplete"
        );
    }

    #[test]
    fn approval_groups_complete_chains() {
        let g = grp(50, json!([{"id": 1, "member_id": 7, "status": req()}]));
        let api = chain_api(std::slice::from_ref(&g));
        assert_eq!(get_change_approval_groups(&api, &json!(12)).unwrap(), [g]);
        assert_eq!(
            api.calls.lock().unwrap()[0].params,
            Some(obj(json!({"page": 1, "per_page": 100})))
        );
        assert_eq!(
            err(get_change_approval_groups(&api, &json!(0))),
            "change ID must be positive"
        );
        let one = |reply: Value| FakeApi::new().ok("GET changes/12/approval-chains", reply);
        let cases = [
            (json!({"x": 1}), "approval chains inventory is unreadable"),
            (
                json!({"approval_chains": [{"id": 1, "approval_groups": []}], "meta": {"total_count": 2}}),
                "approval chains inventory does not match the declared total",
            ),
            (
                json!({"approval_chains": [{"id": 1, "approval_groups": []}], "links": {"next": "x"}}),
                "unsupported approval chains pagination",
            ),
            (
                json!({"approval_chains": [], "has_more": true}),
                "approval chains inventory is incomplete",
            ),
            (
                chains(&[with(
                    grp(50, json!([])),
                    "approval_group_meta",
                    json!({"count": 3}),
                )]),
                "nested approval chains inventory total mismatch",
            ),
            (
                chains(&[grp(50, json!([{"id": 1, "member_id": 7, "status": "x"}]))]),
                "approval group record status unreadable",
            ),
            (
                chains(&[grp(
                    50,
                    json!([{"id": 1, "member_id": 7, "member": {"id": 8}, "status": 0}]),
                )]),
                "approval group member identity conflicting",
            ),
            (
                chains(&[grp(
                    50,
                    json!([{"id": 1, "member_id": 7, "change_id": 13, "status": 0}]),
                )]),
                "approval group change identity conflicting",
            ),
        ];
        for (reply, msg) in cases {
            assert_eq!(
                err(get_change_approval_groups(&one(reply), &json!(12))),
                msg
            );
        }
        let two = FakeApi::new()
            .ok("GET changes/12/approval-chains", json!({"approval_chains": [{"id": 1, "approval_groups": [grp(50, json!([]))]}], "meta": {"has_next": true, "total_count": 2}}))
            .ok("GET changes/12/approval-chains", json!({"approval_chains": [{"id": 2, "approval_groups": [grp(51, json!([]))]}], "meta": {"has_next": false, "total_count": 2}}));
        let ids: Vec<Value> = get_change_approval_groups(&two, &json!(12))
            .unwrap()
            .iter()
            .map(|g| g["id"].clone())
            .collect();
        assert_eq!(ids, [json!(50), json!(51)]);
    }

    #[test]
    fn verify_new_exact_recipients() {
        let p = build_plan(
            "CHN-12",
            "CAB",
            "Please\r\nreview <now>",
            &[7, 8],
            "anyone",
            T,
        )
        .unwrap();
        let anyone = json!({"id": 2, "name": "anyone"});
        let r = |i, m| with(row(i, m), "type", anyone.clone());
        assert!(verify_new(&[r(3, 7), r(4, 8)], &p).is_ok());
        assert_eq!(
            err(verify_new(&[r(3, 7)], &p)),
            "new approval records do not exactly match the recipients"
        );
        assert_eq!(
            err(verify_new(&[row(3, 7), row(4, 8)], &p)),
            "approval status or rule does not match the request"
        );
        assert_eq!(
            err(verify_new(
                &[with(r(3, 7), "member", json!({"id": 9})), r(4, 8)],
                &p
            )),
            "approval member identity unavailable or conflicting"
        );
    }

    #[test]
    fn send_request_verifies_new_records_and_group() {
        let e = build_plan("12", "CAB", "ok", &[7], "everyone", T).unwrap();
        let h = e.hash();
        assert_eq!(
            h,
            "43c116a9e47dc8584187d2dceb020c6d4bbef2ce5f79c360e6a8e0f7cce011e6"
        );
        let api = FakeApi::new()
            .ok("GET changes/12/approvals", json!([row(1, 5)]))
            .ok("GET changes/12/approvals", json!([row(1, 5), row(2, 7)]))
            .ok("GET changes/12/approval-chains", chains(&[]))
            .ok(
                "GET changes/12/approval-chains",
                chains(&[grp(60, json!([{"id": 2, "member_id": 7, "status": req()}]))]),
            )
            .ok("POST changes/12/approval-groups", json!({"ok": true}));
        let j = MemJournal::default();
        let opts = SendOpts {
            reviewed_hash: Some(&h),
            ..Default::default()
        };
        assert_eq!(
            send(&e, &api, &j, opts).unwrap(),
            json!({"status": "submitted", "change_id": 12, "approval_ids": [2], "group_name_verified": true, "verified": true,
                   "readback": [row(2, 7)], "plan_hash": h, "group_id": 60})
        );
        assert_eq!(j.events(), ["started", "done"]);
        let w = api.writes();
        assert_eq!(
            (w[0].method.as_str(), w[0].path.as_str()),
            ("POST", "changes/12/approval-groups")
        );
        let sent: Value = serde_json::from_slice(w[0].raw.as_deref().unwrap()).unwrap();
        assert_eq!(
            sent,
            json!({"approval_groups": [{"name": "CAB", "approval_type": 1, "email_content": "<p dir=\"ltr\">ok</p>", "notified_to": [7]}]})
        );
        assert_eq!(
            err(send(&e, &api, &j, opts)),
            "this approval request was sent or its outcome is unknown; inspect the change and journal; do not resubmit"
        );
    }

    #[test]
    fn send_request_unknown_rejected_and_guards() {
        let e = build_plan("12", "CAB", "ok", &[7], "everyone", T).unwrap();
        let h = e.hash();
        let opts = SendOpts {
            reviewed_hash: Some(&h),
            ..Default::default()
        };
        let j = MemJournal::default();
        let api = FakeApi::new()
            .ok("GET changes/12/approvals", json!([row(1, 5)]))
            .ok("POST changes/12/approval-groups", json!({}));
        assert_eq!(err(send(&e, &api, &j, opts)), UNKNOWN);
        assert_eq!(j.events(), ["started"]);
        let j = MemJournal::default();
        let api = FakeApi::new()
            .ok("GET changes/12/approvals", json!([]))
            .on("POST changes/12/approval-groups", Reply::Status(400));
        assert_eq!(
            err(send(&e, &api, &j, opts)),
            "approval request rejected (HTTP 400); no automatic retry"
        );
        assert_eq!(j.events(), ["started", "failed"]);
        let (none, j) = (FakeApi::new(), MemJournal::default());
        assert_eq!(
            send(&e, &none, &j, SendOpts::default()).unwrap(),
            json!({"status": "cancelled", "change_id": 12})
        );
        assert_eq!(
            send(
                &e,
                &none,
                &j,
                SendOpts {
                    dry_run: true,
                    ..Default::default()
                }
            )
            .unwrap()["status"],
            "dry_run"
        );
        assert_eq!(
            err(send(
                &e,
                &none,
                &j,
                SendOpts {
                    read_only: true,
                    ..Default::default()
                }
            )),
            "read-only mode: no approval request was sent"
        );
        assert_eq!(
            err(send(
                &e,
                &none,
                &j,
                SendOpts {
                    reviewed_hash: Some("x"),
                    ..Default::default()
                }
            )),
            "approval review hash does not match; review the exact request again"
        );
        assert_eq!(
            err(send(&e, &none, &j, opts)),
            "could not prepare a complete approval inventory/session; no approval POST was attempted"
        );
    }

    #[test]
    fn action_plans_match_python() {
        let common = |op: &str| json!({"operation": op, "tenant": T, "change_id": 12, "method": "PUT", "warning": ACTION_WARNING});
        let mut a = common("approve");
        let extra = json!({"action": "change_approval_respond", "path": "/api/_/changes/12/approvals/101", "payload": {"approval_status": 1, "remark": "fine"},
                           "body_present": true, "plan_hash": AP_HASH,
                           "contract": "Approve accepted live (requested=0, approved=1); reject captured aborted (read-back mapping unobserved).",
                           "verification": "Complete approvals list; exact record ID and changed raw status. Named decision checked if exposed; no numeric mapping inferred.",
                           "approval_id": 101});
        a.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let p = ap();
        assert_eq!(
            (p.action(), p.path(), p.payload(), p.hash()),
            (
                "change_approval_respond",
                "changes/12/approvals/101".to_string(),
                Some(json!({"approval_status": 1, "remark": "fine"})),
                AP_HASH.to_string()
            )
        );
        assert_eq!(Value::Object(p.review()), a);
        let r = rj();
        assert_eq!(
            (r.payload(), r.hash()),
            (
                Some(json!({"approval_status": 2, "remark": "no"})),
                RJ_HASH.to_string()
            )
        );
        let c = cn();
        assert_eq!(
            (c.action(), c.path(), c.payload(), c.hash()),
            (
                "change_approval_cancel",
                "changes/12/approval-groups/50/cancel".to_string(),
                None,
                CN_HASH.to_string()
            )
        );
        let review = c.review();
        assert_eq!(
            (
                review["body_present"].clone(),
                review["group_id"].clone(),
                review["send_enabled"].clone()
            ),
            (json!(false), json!(50), json!(true))
        );
        assert_eq!(
            review["contract"],
            "Bodyless JSON-media-type cancel accepted live; chains exposed cancelled status {id:3,name:cancelled}."
        );
        let e = |id: Value, op: &str, remark: Option<&str>| {
            err(build_action_plan("12", &id, op, remark, T))
        };
        assert_eq!(
            e(json!("x"), "approve", None),
            "approval record/group ID must be a positive integer"
        );
        assert_eq!(
            e(json!(1), "maybe", None),
            "choose exactly one of approve, reject or cancel"
        );
        assert_eq!(
            e(json!(1), "approve", Some("a\x07")),
            "remark must be plain text without unsafe control characters"
        );
        assert_eq!(
            e(json!(1), "reject", Some(" ")),
            "reject requires a nonempty --remark"
        );
        assert_eq!(
            e(json!(1), "cancel", Some("r")),
            "cancel sends no body and does not accept a remark"
        );
        assert_eq!(
            err(build_action_plan("INC-1", &json!(1), "approve", None, T)),
            "prefix INC not valid for changes (expect CHN)"
        );
    }

    #[test]
    fn status_and_requested() {
        assert_eq!(status(&json!({"status": 0})).unwrap(), json!(0));
        assert_eq!(
            status(&json!({"status": {"id": 1, "name": "approved"}})).unwrap(),
            json!({"id": 1, "name": "approved"})
        );
        assert_eq!(
            status(&json!({"status": {"id": 3}})).unwrap(),
            json!({"id": 3})
        );
        assert_eq!(
            err(status(&json!({"status": {"id": 1, "name": " "}}))),
            "approval status unreadable"
        );
        assert_eq!(
            err(status(&json!({"status": -1}))),
            "approval status unreadable"
        );
        let got: Vec<bool> = [
            json!(0),
            json!(1),
            json!({"id": 0}),
            json!({"id": 2, "name": "requested"}),
            json!({"id": 1, "name": "approved"}),
        ]
        .iter()
        .map(requested)
        .collect();
        assert_eq!(got, [true, false, true, true, false]);
    }

    #[test]
    fn cancel_group_and_verify_cancel() {
        let c = cn();
        assert_eq!(
            cancel_group(&chain_api(&[before_group()]), &c).unwrap(),
            before_group()
        );
        assert_eq!(
            err(cancel_group(&chain_api(&[]), &c)),
            "approval group missing from complete chains inventory; no cancel PUT attempted"
        );
        assert_eq!(
            err(cancel_group(
                &chain_api(&[grp(50, json!([{"id": 1, "member_id": 7, "status": 1}]))]),
                &c
            )),
            "approval group has no requested approval records; no cancel PUT attempted"
        );
        assert_eq!(
            err(cancel_group(&FakeApi::new(), &c)),
            "could not read complete approval chains; no cancel PUT attempted"
        );
        let before_status = json!([{"approval_id": 1, "member_id": 7, "status": req()}]);
        assert_eq!(
            verify_cancel(&chain_api(&[]), &c, &before_group()).unwrap(),
            json!({"status": "submitted", "operation": "cancel", "change_id": 12, "group_id": 50, "verified": true, "verification_basis": "group_gone",
                   "before_status": before_status, "observed_status": null, "readback": null, "plan_hash": CN_HASH, "warning": CANCEL_WARNING})
        );
        let cancelled = json!({"id": 3, "name": "cancelled"});
        let after = grp(50, json!([{"id": 1, "member_id": 7, "status": cancelled}]));
        assert_eq!(
            verify_cancel(
                &chain_api(std::slice::from_ref(&after)),
                &c,
                &before_group()
            )
            .unwrap(),
            json!({"status": "submitted", "operation": "cancel", "change_id": 12, "group_id": 50, "verified": true, "verification_basis": "changed_raw_statuses_only",
                   "before_status": before_status, "observed_status": [{"approval_id": 1, "member_id": 7, "status": cancelled}], "readback": after,
                   "plan_hash": CN_HASH, "warning": CANCEL_WARNING})
        );
        assert_eq!(
            err(verify_cancel(
                &chain_api(&[before_group()]),
                &c,
                &before_group()
            )),
            ACTION_UNKNOWN
        );
        assert_eq!(
            err(verify_cancel(
                &chain_api(&[grp(50, json!([{"id": 2, "member_id": 7, "status": 3}]))]),
                &c,
                &before_group()
            )),
            ACTION_UNKNOWN
        );
    }

    #[test]
    fn send_action_approve_and_reject() {
        let p = ap();
        let j = MemJournal::default();
        let approved = json!({"id": 1, "name": "approved"});
        let api = FakeApi::new()
            .ok("GET changes/12/approvals", json!([row(101, 7)]))
            .ok(
                "GET changes/12/approvals",
                json!([with(row(101, 7), "status", approved.clone())]),
            )
            .ok("PUT changes/12/approvals/101", json!({}));
        assert_eq!(
            send_action(
                &p,
                &api,
                &j,
                SendOpts {
                    reviewed_hash: Some(AP_HASH),
                    ..Default::default()
                }
            )
            .unwrap(),
            json!({"status": "submitted", "change_id": 12, "approval_id": 101, "operation": "approve", "verified": true, "decision_verified": true,
                   "verification_basis": "named_decision", "before_status": req(), "observed_status": approved,
                   "readback": with(row(101, 7), "status", approved.clone()), "plan_hash": AP_HASH})
        );
        assert_eq!(j.events(), ["started", "done"]);
        let sent: Value = serde_json::from_slice(api.writes()[0].raw.as_deref().unwrap()).unwrap();
        assert_eq!(sent, json!({"approval_status": 1, "remark": "fine"}));

        let api = FakeApi::new()
            .ok("GET changes/12/approvals", json!([row(101, 7)]))
            .ok(
                "GET changes/12/approvals",
                json!([with(row(101, 7), "status", json!(2))]),
            )
            .ok("PUT changes/12/approvals/101", json!({}));
        assert_eq!(
            send_action(
                &rj(),
                &api,
                &MemJournal::default(),
                SendOpts {
                    reviewed_hash: Some(RJ_HASH),
                    ..Default::default()
                }
            )
            .unwrap(),
            json!({"status": "submitted", "change_id": 12, "approval_id": 101, "operation": "reject", "verified": true, "decision_verified": false,
                   "verification_basis": "changed_raw_status_only", "before_status": req(), "observed_status": 2,
                   "readback": with(row(101, 7), "status", json!(2)), "plan_hash": RJ_HASH,
                   "warning": "Raw status changed; decision name/mapping unobserved. Only the status transition is verified; do not resend."})
        );
    }

    #[test]
    fn send_action_failures() {
        let p = ap();
        let opts = SendOpts {
            reviewed_hash: Some(AP_HASH),
            ..Default::default()
        };
        let j = MemJournal::default();
        let api = FakeApi::new()
            .ok("GET changes/12/approvals", json!([row(101, 7)]))
            .ok(
                "GET changes/12/approvals",
                json!([with(
                    row(101, 7),
                    "status",
                    json!({"id": 2, "name": "rejected"})
                )]),
            )
            .ok("PUT changes/12/approvals/101", json!({}));
        assert_eq!(err(send_action(&p, &api, &j, opts)), ACTION_UNKNOWN);
        assert_eq!(j.events(), ["started"]);
        let api = FakeApi::new().ok(
            "GET changes/12/approvals",
            json!([with(row(101, 7), "status", json!(1))]),
        );
        assert_eq!(
            err(send_action(&p, &api, &MemJournal::default(), opts)),
            "could not prepare complete approvals inventory/session with the exact requested record; no approval PUT attempted"
        );
        let j = MemJournal::default();
        let api = FakeApi::new()
            .ok("GET changes/12/approvals", json!([row(101, 7)]))
            .on("PUT changes/12/approvals/101", Reply::Status(409));
        assert_eq!(
            err(send_action(&p, &api, &j, opts)),
            "approval action rejected (HTTP 409); no automatic retry"
        );
        assert_eq!(j.events(), ["started", "failed"]);
        let none = FakeApi::new();
        assert_eq!(
            send_action(&p, &none, &j, SendOpts::default()).unwrap(),
            json!({"status": "cancelled", "change_id": 12})
        );
        assert_eq!(
            err(send_action(
                &p,
                &none,
                &j,
                SendOpts {
                    read_only: true,
                    ..Default::default()
                }
            )),
            "read-only mode: no approval action was sent"
        );
        assert_eq!(
            err(send_action(
                &p,
                &none,
                &j,
                SendOpts {
                    reviewed_hash: Some("x"),
                    ..Default::default()
                }
            )),
            "approval action review hash does not match; review the exact action again"
        );
        assert_eq!(
            err(send_action(
                &cn(),
                &none,
                &MemJournal::default(),
                SendOpts {
                    reviewed_hash: Some(CN_HASH),
                    ..Default::default()
                }
            )),
            "could not read complete approval chains; no cancel PUT attempted"
        );
    }

    #[test]
    fn send_action_cancel_bodyless() {
        let c = cn();
        let j = MemJournal::default();
        let api = FakeApi::new()
            .ok("GET changes/12/approval-chains", chains(&[before_group()]))
            .ok("GET changes/12/approval-chains", chains(&[]))
            .ok("PUT changes/12/approval-groups/50/cancel", json!({}));
        let out = send_action(
            &c,
            &api,
            &j,
            SendOpts {
                reviewed_hash: Some(CN_HASH),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out["verification_basis"], "group_gone");
        assert_eq!(out["observed_status"], Value::Null);
        assert_eq!(j.events(), ["started", "done"]);
        let w = api.writes();
        assert_eq!(
            (w[0].method.as_str(), w[0].path.as_str(), w[0].body.clone()),
            ("PUT", "changes/12/approval-groups/50/cancel", None)
        );
        assert!(w[0].raw.as_deref().is_none_or(<[u8]>::is_empty));
        let dry = send_action(
            &c,
            &chain_api(&[before_group()]),
            &MemJournal::default(),
            SendOpts {
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            (dry["status"].clone(), dry["group"].clone()),
            (json!("dry_run"), before_group())
        );
    }

    #[test]
    fn run_action_flags_and_output() {
        let j = MemJournal::default();
        let none = FakeApi::new();
        let yes = |_: &str| true;
        let no = |_: &str| false;
        let run =
            |p: &ActionPlan, api: &FakeApi, opts: RunOpts<'_>, confirm: &dyn Fn(&str) -> bool| {
                run_action(p, api, &j, opts, confirm)
            };
        assert_eq!(
            err(run(
                &ap(),
                &none,
                RunOpts {
                    read_only: true,
                    ..Default::default()
                },
                &no
            )),
            "read-only mode: no approval action was sent"
        );
        assert_eq!(
            err(run(
                &ap(),
                &none,
                RunOpts {
                    expected: Some("x"),
                    ..Default::default()
                },
                &no
            )),
            "approval action review hash does not match; review the exact action again"
        );
        assert_eq!(
            err(run(
                &ap(),
                &none,
                RunOpts {
                    yes: true,
                    ..Default::default()
                },
                &no
            )),
            "--yes requires the full --expect-plan from a prior approval action review"
        );
        assert_eq!(
            err(run(
                &ap(),
                &none,
                RunOpts {
                    no_input: true,
                    ..Default::default()
                },
                &no
            )),
            "sending requires --yes and --expect-plan in no-input mode"
        );
        let asked = std::cell::RefCell::new(Vec::new());
        let record = |q: &str| {
            asked.borrow_mut().push(q.to_string());
            false
        };
        assert_eq!(
            run(&rj(), &none, RunOpts::default(), &record).unwrap(),
            [
                (true, RUN_DECLINE_REVIEW.to_string()),
                (false, "Cancelled; no approval action was sent.".to_string())
            ]
        );
        assert_eq!(asked.borrow().as_slice(), [CONFIRM]);
        assert_eq!(
            run(
                &rj(),
                &none,
                RunOpts {
                    dry_run: true,
                    ..Default::default()
                },
                &no
            )
            .unwrap(),
            [(false, RUN_DRY.to_string())]
        );

        let api = FakeApi::new()
            .ok("GET changes/12/approvals", json!([row(101, 7)]))
            .ok(
                "GET changes/12/approvals",
                json!([with(row(101, 7), "status", json!(2))]),
            )
            .ok("PUT changes/12/approvals/101", json!({}));
        let echo = run(
            &rj(),
            &api,
            RunOpts {
                yes: true,
                expected: Some(RJ_HASH),
                ..Default::default()
            },
            &no,
        )
        .unwrap();
        assert_eq!(echo, [
            (true, RUN_DECLINE_REVIEW.to_string()),
            (false, "Approval record 101 submitted on change 12; observed raw status: 2; verification: changed_raw_status_only".to_string()),
            (true, "Raw status changed; decision name/mapping unobserved. Only the status transition is verified; do not resend.".to_string()),
        ]);

        let api = FakeApi::new()
            .ok("GET changes/12/approval-chains", chains(&[before_group()]))
            .ok("GET changes/12/approval-chains", chains(&[before_group()]))
            .ok("GET changes/12/approval-chains", chains(&[]))
            .ok("PUT changes/12/approval-groups/50/cancel", json!({}));
        let echo = run_action(
            &cn(),
            &api,
            &MemJournal::default(),
            RunOpts {
                json_out: true,
                ..Default::default()
            },
            &yes,
        )
        .unwrap();
        assert_eq!(
            echo,
            [
                (true, RUN_CANCEL_REVIEW.to_string()),
                (false, RUN_CANCEL_RESULT.to_string())
            ]
        );
    }

    /// `_run` echoes the review to stderr before `send` raises.
    #[test]
    fn run_action_detailed_keeps_review_echo_on_error() {
        let api = || {
            FakeApi::new()
                .ok("GET changes/12/approvals", json!([row(101, 7)]))
                .on("PUT changes/12/approvals/101", Reply::Status(500))
        };
        let opts = RunOpts {
            yes: true,
            expected: Some(RJ_HASH),
            ..Default::default()
        };
        let no = |_: &str| false;
        let e = run_action_detailed(&rj(), &api(), &MemJournal::default(), opts, &no).unwrap_err();
        assert_eq!(e.echo, [(true, RUN_DECLINE_REVIEW.to_string())]);
        let flat = run_action(&rj(), &api(), &MemJournal::default(), opts, &no).unwrap_err();
        assert_eq!(e.to_string(), flat.to_string());
        assert_eq!(Error::from(e).exit_code(), flat.exit_code());
        let early = run_action_detailed(
            &ap(),
            &FakeApi::new(),
            &MemJournal::default(),
            RunOpts {
                read_only: true,
                ..Default::default()
            },
            &no,
        )
        .unwrap_err();
        assert!(early.echo.is_empty());
    }
}
