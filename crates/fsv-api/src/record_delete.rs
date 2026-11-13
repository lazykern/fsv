//! Reviewed problem and knowledge-article deletion through captured wires
//! (`record_delete.py`).
//!
//! Captured (agent UI, JSON content type, no body):
//!
//! * `DELETE /api/_/problems/<n>` -> soft delete; exact GET then shows
//!   `deleted: true` (listed in the deleted filter).
//! * `PUT /api/_/problems/<n>/restore` -> restored (`deleted: false`).
//! * `DELETE /api/_/problems/delete-forever?ids=<n>` (trash view) -> gone; the
//!   exact GET answers 404.
//! * `DELETE /api/_/solutions/articles/<id>` -> moved to the solutions trash.
//! * `DELETE /api/_/solutions/articles/<id>/delete_forever` -> exact GET 404.
//!
//! Every write is hash-bound to the exact record snapshot, single-attempt and
//! journaled; readback is the deleted flag or a 404. Project deletion lives on
//! a different product host with its own session and is not implemented.

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{PROBLEMS, parse_id};

use crate::readback::bounded_readback;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, plan_hash, snapshot_hash};
use crate::watchers::positive_id;

const PROBLEM_ACTIONS: [&str; 3] = ["delete", "restore", "delete-forever"];
const ARTICLE_ACTIONS: [&str; 2] = ["delete", "delete-forever"];
pub const MAX_FOREVER: i64 = 50;

fn destructive(kind: &str, action: &str) -> bool {
    matches!(
        (kind, action),
        ("problem", "delete-forever") | ("article", "delete" | "delete-forever")
    )
}

/// `_get`: the exact row, or `None` on an HTTP 404 (absent). Other failures
/// refuse.
fn get(c: &dyn Api, path: &str, key: &str, ident: i64) -> Result<Option<Map<String, Value>>> {
    let raw = match c.int_get(path, None) {
        Ok(raw) => raw,
        Err(e) => {
            return match e.as_api() {
                Some(api) if api.status == 404 => Ok(None),
                Some(api) => Err(refused(format!(
                    "exact {key} unavailable (HTTP {})",
                    api.status
                ))),
                None => Err(e),
            };
        }
    };
    let row = match &raw {
        Value::Object(m) => m.get(key).unwrap_or(&raw).clone(),
        _ => Value::Null,
    };
    let Value::Object(row) = row else {
        return Err(refused(format!("exact {key} unavailable")));
    };
    if positive_id(row.get("id").unwrap_or(&Value::Null))? != ident {
        return Err(refused(format!("exact {key} unavailable")));
    }
    Ok(Some(row))
}

fn problem(c: &dyn Api, pid: i64) -> Result<Option<Map<String, Value>>> {
    get(c, &format!("problems/{pid}"), "problem", pid)
}

fn article(c: &dyn Api, aid: i64) -> Result<Option<Map<String, Value>>> {
    get(c, &format!("solutions/articles/{aid}"), "article", aid)
}

type Getter = fn(&dyn Api, i64) -> Result<Option<Map<String, Value>>>;

fn getter(kind: &str) -> Getter {
    if kind == "problem" { problem } else { article }
}

/// Port of record_delete.problem_plan.
pub fn problem_plan(c: &dyn Api, target: &str, action: &str) -> Result<WritePlan> {
    if !PROBLEM_ACTIONS.contains(&action) {
        return Err(refused(format!(
            "unknown problem action {}",
            fsv_py::py_repr_str(action)
        )));
    }
    let pid = positive_id(&json!(parse_id(target, &PROBLEMS)?))?;
    let Some(row) = problem(c, pid)? else {
        return Err(refused(format!(
            "problem {pid} not found; nothing to {action}"
        )));
    };
    let Some(Value::Bool(deleted)) = row.get("deleted") else {
        return Err(refused("exact problem deleted flag unavailable"));
    };
    if action == "delete" && *deleted {
        return Err(refused(
            "problem is already in the trash; use restore or delete-forever",
        ));
    }
    if action != "delete" && !*deleted {
        return Err(refused(format!(
            "{action} applies only to a problem in the trash (delete it first)"
        )));
    }
    let (method, path) = match action {
        "delete" => ("DELETE", format!("problems/{pid}")),
        "restore" => ("PUT", format!("problems/{pid}/restore")),
        _ => ("DELETE", format!("problems/delete-forever?ids={pid}")),
    };
    Ok(WritePlan {
        action: format!("problem_{}", action.replace('-', "_")),
        target: pid,
        tenant: c.domain(),
        method: method.into(),
        path,
        payload_json: "null".into(),
        context_hash: snapshot_hash(&json!({"problem": row}))?,
    })
}

/// Port of record_delete.article_plan.
pub fn article_plan(c: &dyn Api, ident: &str, action: &str) -> Result<WritePlan> {
    if !ARTICLE_ACTIONS.contains(&action) {
        return Err(refused(format!(
            "unknown article action {}",
            fsv_py::py_repr_str(action)
        )));
    }
    let aid = positive_id(&json!(ident))?;
    let Some(row) = article(c, aid)? else {
        return Err(refused(format!(
            "article {aid} not found; nothing to delete"
        )));
    };
    if action == "delete" && row.get("deleted").is_some_and(fsv_py::truthy) {
        return Err(refused(
            "article is already in the trash; use delete-forever",
        ));
    }
    let suffix = if action == "delete-forever" {
        "/delete_forever"
    } else {
        ""
    };
    Ok(WritePlan {
        action: format!("solution_{}", action.replace('-', "_")),
        target: aid,
        tenant: c.domain(),
        method: "DELETE".into(),
        path: format!("solutions/articles/{aid}{suffix}"),
        payload_json: "null".into(),
        context_hash: snapshot_hash(&json!({"article": row}))?,
    })
}

/// `_kind`: (`problem` | `article`, the CLI action).
fn kind_of(plan: &WritePlan) -> (&'static str, String) {
    let (kind, act) = plan.action.split_once('_').unwrap_or((&plan.action, ""));
    (
        if kind == "problem" {
            "problem"
        } else {
            "article"
        },
        act.replace('_', "-"),
    )
}

/// Port of record_delete.review.
pub fn review(plan: &WritePlan, c: &dyn Api) -> Result<Map<String, Value>> {
    let (kind, action) = kind_of(plan);
    let row = getter(kind)(c, plan.target)?.unwrap_or_default();
    let title = row
        .get(if kind == "problem" {
            "subject"
        } else {
            "title"
        })
        .cloned()
        .unwrap_or(Value::Null);
    let effect = match (kind, action.as_str()) {
        ("problem", "delete") => {
            "Moves the problem to the trash; `fsv problems restore` brings it back."
        }
        ("problem", "restore") => "Restores the problem from the trash.",
        ("problem", _) => "PERMANENT: removes the trashed problem; it cannot be restored.",
        (_, "delete") => {
            "Moves the article to the solutions trash; fsv has no captured article restore."
        }
        _ => "PERMANENT: removes the article; it cannot be restored.",
    };
    let mut out = plan.review();
    out.insert(format!("{kind}_id"), json!(plan.target));
    out.insert("title".into(), title);
    out.insert(
        "deleted".into(),
        row.get("deleted").cloned().unwrap_or(Value::Null),
    );
    out.insert("effect".into(), json!(effect));
    out.insert(
        "requires".into(),
        if destructive(kind, &action) {
            json!("--confirm-destructive")
        } else {
            Value::Null
        },
    );
    out.insert(
        "warning".into(),
        json!("Single attempt; an unknown outcome must be inspected, never automatically resent."),
    );
    Ok(out)
}

fn readback(kind: &str, target: i64, field: &str, state: &str) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert(format!("{kind}_id"), json!(target));
    out.insert("fields".into(), obj(json!({field: state})).into());
    out.insert("verified".into(), json!(state == "matched"));
    out
}

/// Port of record_delete.send.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    confirm_destructive: bool,
) -> Result<Value> {
    let (kind, action) = kind_of(plan);
    if reviewed_hash.is_some() && destructive(kind, &action) && !confirm_destructive {
        return Err(refused(format!(
            "{kind} {action} requires --confirm-destructive after inspecting the review"
        )));
    }
    let target = plan.target;
    let fetch = getter(kind);
    let rebuild = || {
        if kind == "problem" {
            problem_plan(c, &target.to_string(), &action)
        } else {
            article_plan(c, &target.to_string(), &action)
        }
    };
    let verify = || -> Result<Map<String, Value>> {
        let row = fetch(c, target)?;
        if action == "delete-forever" {
            let state = if row.is_none() {
                "matched"
            } else {
                "mismatched"
            };
            return Ok(readback(kind, target, "absent", state));
        }
        let want = action == "delete";
        let state = match &row {
            None if kind == "article" && want => "matched",
            None => "mismatched",
            Some(row) => match row.get("deleted") {
                Some(Value::Bool(flag)) if *flag == want => "matched",
                Some(Value::Bool(_)) => "mismatched",
                _ => "unavailable",
            },
        };
        Ok(readback(kind, target, "deleted", state))
    };
    let bounded = |_: &Value| Ok(bounded_readback(verify));
    execute(plan, c, journal, reviewed_hash, &rebuild, &bounded)
}

/// Port of record_delete.ForeverBatch: explicit IDs to delete permanently,
/// one captured request per record, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeverBatch {
    pub kind: String,
    pub plans: Vec<WritePlan>,
    pub rows_json: String,
    pub max_records: Option<i64>,
}

impl ForeverBatch {
    pub fn ids(&self) -> Vec<i64> {
        self.plans.iter().map(|p| p.target).collect()
    }

    pub fn action(&self) -> &str {
        &self.plans[0].action
    }

    pub fn hash(&self) -> String {
        let plans: Vec<String> = self.plans.iter().map(WritePlan::hash).collect();
        plan_hash(&json!({
            "action": self.action(), "plans": plans, "ids": self.ids(),
            "max_records": self.max_records,
        }))
    }

    pub fn rows(&self) -> Vec<Value> {
        serde_json::from_str(&self.rows_json).unwrap_or_default()
    }

    pub fn review(&self) -> Map<String, Value> {
        let kind = &self.kind;
        let requests: Vec<String> = self
            .plans
            .iter()
            .map(|p| format!("/api/_/{}", p.path))
            .collect();
        let mut out = Map::new();
        out.insert("action".into(), json!(self.action()));
        out.insert("method".into(), json!("DELETE"));
        out.insert("requests".into(), json!(requests));
        out.insert(format!("{kind}_ids"), json!(self.ids()));
        out.insert("selected".into(), Value::Array(self.rows()));
        out.insert("max_records".into(), json!(self.max_records));
        out.insert(
            "effect".into(),
            json!(format!(
                "PERMANENT: removes the listed {kind}(s); they cannot be restored."
            )),
        );
        out.insert("requires".into(), json!("--confirm-destructive"));
        out.insert("plan_hash".into(), json!(self.hash()));
        out.insert(
            "warning".into(),
            json!(format!(
                "One request per {kind}, in order; stops at the first failed or unverified one. Never replay an uncertain outcome."
            )),
        );
        out
    }
}

/// Port of record_delete.build_forever: permanent delete of explicit IDs;
/// more than one ID needs `--max-records` (1 to 50).
pub fn build_forever(
    c: &dyn Api,
    kind: &str,
    idents: &[String],
    max_records: Option<i64>,
) -> Result<ForeverBatch> {
    if !matches!(kind, "problem" | "article") {
        return Err(Error::value("unsupported delete-forever kind"));
    }
    if idents.is_empty() {
        return Err(Error::value(format!(
            "give one or more explicit {kind} IDs"
        )));
    }
    if max_records.is_some_and(|n| !(1..=MAX_FOREVER).contains(&n)) {
        return Err(Error::value(format!(
            "--max-records must be 1 to {MAX_FOREVER}"
        )));
    }
    if idents.len() > 1 && max_records.is_none() {
        return Err(Error::value(format!(
            "deleting more than one {kind} permanently requires --max-records N (1 to {MAX_FOREVER})"
        )));
    }
    if idents.len() as i64 > max_records.unwrap_or(1) {
        return Err(Error::value(
            "more IDs than --max-records; refusing the whole plan; nothing was sent",
        ));
    }
    let plans = idents
        .iter()
        .map(|i| {
            if kind == "problem" {
                problem_plan(c, i, "delete-forever")
            } else {
                article_plan(c, i, "delete-forever")
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let ids: Vec<i64> = plans.iter().map(|p| p.target).collect();
    let distinct: std::collections::HashSet<i64> = ids.iter().copied().collect();
    if distinct.len() != ids.len() {
        return Err(Error::value(format!("give distinct {kind} IDs")));
    }
    let mut rows = Vec::with_capacity(ids.len());
    for id in &ids {
        let row = getter(kind)(c, *id)?.unwrap_or_default();
        let title = row
            .get(if kind == "problem" {
                "subject"
            } else {
                "title"
            })
            .cloned()
            .unwrap_or(Value::Null);
        rows.push(json!({"id": id, "title": title}));
    }
    Ok(ForeverBatch {
        kind: kind.into(),
        plans,
        rows_json: fsv_py::Dumps::default()
            .sort_keys(true)
            .ensure_ascii(false)
            .dump(&Value::Array(rows)),
        max_records,
    })
}

/// Port of record_delete.send_forever: one reviewed send per record, in
/// order, stopping at the first failed or unverified one.
pub fn send_forever(
    batch: &ForeverBatch,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    confirm_destructive: bool,
) -> Result<Value> {
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = batch.hash();
    if reviewed != hash {
        return Err(refused("full review hash changed; review again"));
    }
    let kind = &batch.kind;
    if !confirm_destructive {
        return Err(refused(format!(
            "{kind} delete-forever requires --confirm-destructive after inspecting the printed ID list"
        )));
    }
    let key = format!("{kind}_ids");
    let ids = batch.ids();
    let mut results = Map::new();
    for (index, sub) in batch.plans.iter().enumerate() {
        let result = match send(sub, c, journal, Some(&sub.hash()), true) {
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
        results.insert(sub.target.to_string(), Value::Object(kept));
        if !result.get("verified").is_some_and(fsv_py::truthy) {
            let mut out = Map::new();
            out.insert(
                "status".into(),
                json!(if index > 0 { "partial" } else { "failed" }),
            );
            out.insert("verified".into(), json!(false));
            out.insert("plan_hash".into(), json!(hash));
            out.insert(key, json!(ids));
            out.insert("results".into(), Value::Object(results));
            out.insert("stopped_at".into(), json!(sub.target));
            out.insert("not_attempted".into(), json!(ids[index + 1..]));
            out.insert(
                "warning".into(),
                json!(format!(
                    "Stopped at the first failed or unverified {kind}; inspect it and the journal; never replay."
                )),
            );
            return Ok(Value::Object(out));
        }
    }
    let mut out = Map::new();
    out.insert("status".into(), json!("submitted"));
    out.insert("verified".into(), json!(true));
    out.insert("plan_hash".into(), json!(hash));
    out.insert(key, json!(ids));
    out.insert("results".into(), Value::Object(results));
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use fsv_core::ApiError;

    use super::*;
    use crate::readback::with_readback_sleep;
    use crate::service::testing::{Call, FnApi, MemJournal};

    struct World {
        problem: Option<Map<String, Value>>,
        article: Option<Map<String, Value>>,
        writes: Vec<(String, String)>,
        ignore: bool,
        fail: Option<u16>,
    }

    const PID: i64 = 12;
    const AID: i64 = 900012;

    fn world() -> Arc<Mutex<World>> {
        Arc::new(Mutex::new(World {
            problem: Some(obj(
                json!({"id": PID, "subject": "Acme outage", "deleted": false}),
            )),
            article: Some(obj(
                json!({"id": AID, "title": "Globex workaround", "deleted": false}),
            )),
            writes: Vec::new(),
            ignore: false,
            fail: None,
        }))
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        FnApi::new(move |call: &Call| {
            let mut guard = state.lock().unwrap();
            let s = &mut *guard;
            if call.method == "GET" {
                let row = if call.path == format!("problems/{PID}") {
                    s.problem.clone().map(|r| json!({"problem": r}))
                } else if call.path == format!("solutions/articles/{AID}") {
                    s.article.clone().map(|r| json!({"article": r}))
                } else {
                    panic!("{}", call.path)
                };
                return row.ok_or_else(|| ApiError::new(404, json!({"status": 404})).into());
            }
            assert!(call.body.is_none() && call.raw.is_none());
            s.writes.push((call.method.clone(), call.path.clone()));
            if let Some(status) = s.fail {
                return Err(ApiError::new(status, json!({"errors": "no"})).into());
            }
            if !s.ignore {
                let stamp = json!(format!("2030-01-01T00:00:{:02}Z", s.writes.len()));
                for row in [s.problem.as_mut(), s.article.as_mut()]
                    .into_iter()
                    .flatten()
                {
                    row.insert("updated_at".into(), stamp.clone());
                }
                match (call.method.as_str(), call.path.as_str()) {
                    ("DELETE", "problems/12") => {
                        s.problem
                            .as_mut()
                            .unwrap()
                            .insert("deleted".into(), json!(true));
                    }
                    ("PUT", "problems/12/restore") => {
                        s.problem
                            .as_mut()
                            .unwrap()
                            .insert("deleted".into(), json!(false));
                    }
                    ("DELETE", "problems/delete-forever?ids=12") => s.problem = None,
                    ("DELETE", "solutions/articles/900012") => {
                        s.article
                            .as_mut()
                            .unwrap()
                            .insert("deleted".into(), json!(true));
                    }
                    ("DELETE", "solutions/articles/900012/delete_forever") => s.article = None,
                    (m, p) => panic!("{m} {p}"),
                }
            }
            Ok(Value::Null)
        })
    }

    fn quiet<R>(body: impl FnOnce() -> R) -> R {
        with_readback_sleep(|_| {}, body)
    }

    fn msg(r: Result<impl std::fmt::Debug>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn problem_trash_restore_and_forever_lifecycle() {
        let s = world();
        let c = api(Arc::clone(&s));
        let j = MemJournal::default();
        for (action, method, path) in [
            ("delete", "DELETE", "problems/12"),
            ("restore", "PUT", "problems/12/restore"),
        ] {
            let p = problem_plan(&c, "PRB-12", action).unwrap();
            let r = quiet(|| send(&p, &c, &j, Some(&p.hash()), false)).unwrap();
            assert_eq!(r["verified"], json!(true));
            assert_eq!(r["fields"], json!({"deleted": "matched"}));
            assert_eq!(
                s.lock().unwrap().writes.last().unwrap(),
                &(method.to_string(), path.to_string())
            );
            assert!(send(&p, &c, &j, Some(&p.hash()), false).is_err());
        }
        assert!(
            msg(problem_plan(&c, "12", "delete-forever"))
                .contains("only to a problem in the trash")
        );
        {
            let mut w = s.lock().unwrap();
            let row = w.problem.as_mut().unwrap();
            row.insert("deleted".into(), json!(true));
            row.insert("subject".into(), json!("Acme outage (trashed)"));
        }
        let p = problem_plan(&c, "12", "delete-forever").unwrap();
        assert_eq!(
            review(&p, &c).unwrap()["requires"],
            json!("--confirm-destructive")
        );
        assert!(msg(send(&p, &c, &j, Some(&p.hash()), false)).contains("--confirm-destructive"));
        let r = quiet(|| send(&p, &c, &j, Some(&p.hash()), true)).unwrap();
        assert_eq!(r["verified"], json!(true));
        assert_eq!(r["fields"], json!({"absent": "matched"}));
        let w = s.lock().unwrap();
        assert_eq!(
            w.writes.last().unwrap(),
            &(
                "DELETE".to_string(),
                "problems/delete-forever?ids=12".to_string()
            )
        );
        assert_eq!(w.writes.len(), 3);
    }

    #[test]
    fn ignored_delete_is_not_verified_and_never_replayed() {
        let s = world();
        s.lock().unwrap().ignore = true;
        let c = api(Arc::clone(&s));
        let j = MemJournal::default();
        let p = problem_plan(&c, "12", "delete").unwrap();
        let r = quiet(|| send(&p, &c, &j, Some(&p.hash()), false)).unwrap();
        assert_eq!(r["verified"], json!(false));
        assert_eq!(r["fields"], json!({"deleted": "mismatched"}));
        assert_eq!(r["readback_attempts"], json!(4));
        assert!(msg(send(&p, &c, &j, Some(&p.hash()), false)).contains("outcome unknown"));
        assert_eq!(s.lock().unwrap().writes.len(), 1);
    }

    #[test]
    fn rejected_and_drifted_targets() {
        let s = world();
        let c = api(Arc::clone(&s));
        let j = MemJournal::default();
        let p = problem_plan(&c, "12", "delete").unwrap();
        s.lock()
            .unwrap()
            .problem
            .as_mut()
            .unwrap()
            .insert("subject".into(), json!("Acme outage (edited)"));
        assert!(msg(send(&p, &c, &j, Some(&p.hash()), false)).contains("changed since review"));
        s.lock().unwrap().fail = Some(422);
        let p = problem_plan(&c, "12", "delete").unwrap();
        assert!(msg(send(&p, &c, &j, Some(&p.hash()), false)).contains("HTTP 422"));
        assert_eq!(s.lock().unwrap().writes.len(), 1);
    }

    #[test]
    fn article_trash_and_forever() {
        let s = world();
        let c = api(Arc::clone(&s));
        let j = MemJournal::default();
        let p = article_plan(&c, "900012", "delete").unwrap();
        assert_eq!(review(&p, &c).unwrap()["title"], json!("Globex workaround"));
        assert!(msg(send(&p, &c, &j, Some(&p.hash()), false)).contains("--confirm-destructive"));
        let r = quiet(|| send(&p, &c, &j, Some(&p.hash()), true)).unwrap();
        assert_eq!(r["verified"], json!(true));
        assert!(msg(article_plan(&c, "900012", "delete")).contains("already in the trash"));
        let p = article_plan(&c, "900012", "delete-forever").unwrap();
        let r = quiet(|| send(&p, &c, &j, Some(&p.hash()), true)).unwrap();
        assert_eq!(r["verified"], json!(true));
        assert_eq!(
            s.lock().unwrap().writes.last().unwrap().1,
            "solutions/articles/900012/delete_forever"
        );
        assert!(msg(article_plan(&c, "900012", "delete-forever")).contains("not found"));
    }

    #[test]
    fn cancel_and_unknown_actions() {
        let c = api(world());
        let p = problem_plan(&c, "12", "delete").unwrap();
        assert_eq!(
            send(&p, &c, &MemJournal::default(), None, false).unwrap(),
            json!({"status": "cancelled", "verified": false})
        );
        assert_eq!(
            msg(problem_plan(&c, "12", "purge")),
            "unknown problem action 'purge'"
        );
        assert_eq!(
            msg(article_plan(&c, "900012", "restore")),
            "unknown article action 'restore'"
        );
    }

    struct Many {
        problems: std::collections::BTreeMap<i64, Value>,
        writes: Vec<String>,
        fail_on: Option<i64>,
    }

    fn many() -> (Arc<Mutex<Many>>, FnApi) {
        let state = Arc::new(Mutex::new(Many {
            problems: [21, 22, 23]
                .into_iter()
                .map(|i| {
                    (
                        i,
                        json!({"id": i, "subject": format!("Acme trashed {i}"), "deleted": true}),
                    )
                })
                .collect(),
            writes: Vec::new(),
            fail_on: None,
        }));
        let s = Arc::clone(&state);
        let c = FnApi::new(move |call: &Call| {
            let mut s = s.lock().unwrap();
            if call.method == "GET" {
                let id: i64 = call.path.rsplit('/').next().unwrap().parse().unwrap();
                return s
                    .problems
                    .get(&id)
                    .map(|r| json!({"problem": r}))
                    .ok_or_else(|| ApiError::new(404, json!({})).into());
            }
            s.writes.push(call.path.clone());
            let id: i64 = call.path.rsplit('=').next().unwrap().parse().unwrap();
            if Some(id) == s.fail_on {
                return Err(ApiError::new(500, json!({"errors": "no"})).into());
            }
            s.problems.remove(&id);
            Ok(Value::Null)
        });
        (state, c)
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn forever_bounds_refuse_before_any_write() {
        let (s, c) = many();
        for (idents, max, want) in [
            (vec!["21", "22"], None, "requires --max-records"),
            (vec!["21", "22"], Some(1), "more IDs than --max-records"),
            (vec!["21"], Some(51), "1 to 50"),
            (vec!["21", "21"], Some(2), "distinct"),
            (vec![], None, "give one or more explicit problem IDs"),
        ] {
            let err = msg(build_forever(&c, "problem", &ids(&idents), max));
            assert!(err.contains(want), "{err}");
        }
        assert!(s.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn forever_batch_stops_at_first_failure_and_is_bound_to_max_records() {
        let (s, c) = many();
        let j = MemJournal::default();
        let a = build_forever(&c, "problem", &ids(&["21", "22", "23"]), Some(3)).unwrap();
        let b = build_forever(&c, "problem", &ids(&["21", "22", "23"]), Some(4)).unwrap();
        assert_ne!(a.hash(), b.hash());
        assert_eq!(a.review()["problem_ids"], json!([21, 22, 23]));
        assert!(
            msg(send_forever(&a, &c, &j, Some(&a.hash()), false)).contains("--confirm-destructive")
        );
        s.lock().unwrap().fail_on = Some(22);
        let r = quiet(|| send_forever(&a, &c, &j, Some(&a.hash()), true)).unwrap();
        assert_eq!(r["status"], json!("partial"));
        assert_eq!(r["stopped_at"], json!(22));
        assert_eq!(r["not_attempted"], json!([23]));
        let w = s.lock().unwrap();
        assert_eq!(
            w.writes,
            [
                "problems/delete-forever?ids=21",
                "problems/delete-forever?ids=22"
            ]
        );
        assert!(w.problems.contains_key(&23));
    }
}
