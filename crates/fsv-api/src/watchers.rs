//! Captured ticket/change self-watch contracts; other subscribers are
//! unsupported (`watchers.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::Result;
use fsv_core::resource::{CHANGES, TICKETS, parse_id};
use fsv_py::ValueExt;

use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, plan_hash};

/// Port of fsv.watchers.positive_id: a positive decimal int or digit string,
/// never a bool.
pub fn positive_id(value: &Value) -> Result<i64> {
    crate::service::positive_id(value)
        .ok_or_else(|| refused("expected a positive numeric identity"))
}

/// Port of fsv.watchers.self_id: the signed-in agent's id, read fresh.
pub fn self_id(c: &dyn Api) -> Result<i64> {
    let me = c.me()?;
    if !me.is_object() {
        return Err(refused("signed-in agent identity is unavailable"));
    }
    positive_id(me.get("id").unwrap_or(&Value::Null))
}

/// Python `raw["total"] != n` for a JSON total.
pub(crate) fn total_differs(total: &Value, n: usize) -> bool {
    match total {
        Value::Null => false,
        Value::Bool(b) => usize::from(*b) != n,
        Value::Number(x) => x.as_f64() != Some(n as f64),
        _ => true,
    }
}

/// `has_more or has_next or next_page`.
pub(crate) fn has_continuation(raw: &Value) -> bool {
    ["has_more", "has_next", "next_page"]
        .iter()
        .any(|k| raw.get_truthy(k).is_some())
}

/// Port of fsv.watchers.inventory: the complete, sorted watcher ids.
pub fn inventory(c: &dyn Api, module: &str, target: i64) -> Result<Vec<i64>> {
    let raw = c.int_get(&format!("{module}/{target}/watchers"), None)?;
    let Some(list) = raw
        .get("watcher_ids")
        .and_then(Value::as_array)
        .filter(|_| raw.is_object() && !has_continuation(&raw))
    else {
        return Err(refused("complete watcher membership is unavailable"));
    };
    let mut ids = list.iter().map(positive_id).collect::<Result<Vec<i64>>>()?;
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != n {
        return Err(refused("watcher membership contains duplicate identities"));
    }
    if total_differs(raw.get("total").unwrap_or(&Value::Null), n) {
        return Err(refused("watcher membership is incomplete"));
    }
    Ok(ids)
}

/// Port of fsv.watchers.build_plan: self watch/unwatch of a ticket or change.
pub fn build_plan(c: &dyn Api, module: &str, target: &str, watch: bool) -> Result<WritePlan> {
    let res = match module {
        "tickets" => &TICKETS,
        "changes" => &CHANGES,
        _ => {
            return Err(refused(
                "only ticket/change self-watch contracts are proven",
            ));
        }
    };
    let cid = positive_id(&json!(parse_id(target, res)?))?;
    let user = self_id(c)?;
    let before = inventory(c, module, cid)?;
    let (method, path, payload) = match (module, watch) {
        ("tickets", true) => (
            "POST",
            format!("tickets/{cid}/watch"),
            json!({"user_id": user}),
        ),
        ("tickets", false) => (
            "PUT",
            format!("tickets/{cid}/unwatch"),
            json!({"user_id": user}),
        ),
        (_, true) => (
            "POST",
            format!("changes/{cid}/watchers"),
            json!({"user_id": user}),
        ),
        (_, false) => (
            "DELETE",
            format!("changes/{cid}/unwatch?user_id={user}"),
            Value::Null,
        ),
    };
    let context = json!({"self": user, "before": before, "watch": watch});
    Ok(WritePlan {
        action: format!("{module}_self_{}", if watch { "watch" } else { "unwatch" }),
        target: cid,
        tenant: c.domain(),
        method: method.into(),
        path,
        payload_json: fsv_py::Dumps::default().sort_keys(true).dump(&payload),
        context_hash: plan_hash(&context),
    })
}

/// Port of fsv.watchers.send: rebuilds (even for no-ops), then one write and
/// a full-membership readback.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let module = plan.path.split('/').next().unwrap_or_default().to_string();
    let watch = plan.action.ends_with("_watch");
    let user = self_id(c)?;
    let before = inventory(c, &module, plan.target)?;
    let target = plan.target.to_string();
    let fresh = build_plan(c, &module, &target, watch)?;
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    if fresh.hash() != hash || reviewed != hash || plan.tenant != c.domain() {
        return Err(refused("target or review changed; review again"));
    }
    if before.contains(&user) == watch {
        return Ok(
            json!({"status": "already_in_requested_state", "verified": true, "plan_hash": hash}),
        );
    }
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let after = inventory(c, &module, plan.target)?;
        let expected: Vec<i64> = if watch {
            let mut v = before.clone();
            v.push(user);
            v.sort_unstable();
            v.dedup();
            v
        } else {
            before.iter().copied().filter(|v| *v != user).collect()
        };
        Ok(obj(
            json!({"watcher_ids": after, "verified": after == expected}),
        ))
    };
    let rebuild = || build_plan(c, &module, &target, watch);
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;
    use fsv_core::ApiError;
    use fsv_core::error::Error;

    struct World {
        user: i64,
        members: Vec<i64>,
        failure: Option<u16>,
        read_failure: bool,
        fail_reads_after_write: bool,
        listing: Option<Value>,
    }

    fn client(world: &Arc<Mutex<World>>) -> FnApi {
        let w = world.clone();
        FnApi::new(move |call| {
            let mut w = w.lock().unwrap();
            match call.method.as_str() {
                "ME" => Ok(json!({"id": w.user})),
                "GET" => {
                    assert!(call.path.ends_with("/watchers"));
                    if w.read_failure {
                        return Err(ApiError::new(403, json!("denied")).into());
                    }
                    Ok(w.listing
                        .clone()
                        .unwrap_or(json!({"watcher_ids": w.members})))
                }
                _ => {
                    if let Some(s) = w.failure {
                        return Err(ApiError::new(s, json!("uncertain")).into());
                    }
                    let user = w.user;
                    if call.path.ends_with("/watch") || call.path.ends_with("/watchers") {
                        w.members.push(user);
                    } else {
                        w.members.retain(|m| *m != user);
                    }
                    if w.fail_reads_after_write {
                        w.read_failure = true;
                    }
                    Ok(json!({}))
                }
            }
        })
    }

    fn world() -> Arc<Mutex<World>> {
        Arc::new(Mutex::new(World {
            user: 901,
            members: vec![902],
            failure: None,
            read_failure: false,
            fail_reads_after_write: false,
            listing: None,
        }))
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    /// `test_watchers.py::test_self_watch_unwatch_preserves_other_members_and_exact_methods`.
    #[test]
    fn watch_unwatch_exact_methods() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        for module in ["tickets", "changes"] {
            let w = world();
            let c = client(&w);
            let j = MemJournal::default();
            let p = build_plan(&c, module, "123", true).unwrap();
            assert_eq!(p.payload(), json!({"user_id": 901}));
            assert_eq!(p.hash(), oracle["watchers"][module]["watch_hash"]);
            assert_eq!(send(&p, &c, &j, Some(&p.hash())).unwrap()["verified"], true);
            let q = build_plan(&c, module, "123", false).unwrap();
            assert_eq!(q.hash(), oracle["watchers"][module]["unwatch_hash"]);
            assert_eq!(
                q.payload_json,
                oracle["watchers"][module]["unwatch_payload_json"]
            );
            let out = send(&q, &c, &j, Some(&q.hash())).unwrap();
            assert_eq!(out["verified"], true);
            assert_eq!(out["watcher_ids"], json!([902]));
            assert_eq!(w.lock().unwrap().members, [902]);
            let writes = c.writes();
            assert_eq!(writes[0].method, "POST");
            if module == "tickets" {
                assert_eq!(
                    (
                        writes[1].method.as_str(),
                        writes[1].path.as_str(),
                        &writes[1].body
                    ),
                    ("PUT", "tickets/123/unwatch", &Some(json!({"user_id": 901})))
                );
            } else {
                assert_eq!(
                    (
                        writes[1].method.as_str(),
                        writes[1].path.as_str(),
                        &writes[1].body
                    ),
                    ("DELETE", "changes/123/unwatch?user_id=901", &None)
                );
            }
        }
    }

    /// `test_ambiguous_write_is_never_replayed`.
    #[test]
    fn ambiguous_write_never_replayed() {
        for status in [403, 429, 500] {
            let w = world();
            let c = client(&w);
            let j = MemJournal::default();
            let p = build_plan(&c, "tickets", "123", true).unwrap();
            w.lock().unwrap().failure = Some(status);
            for _ in 0..2 {
                assert!(err(send(&p, &c, &j, Some(&p.hash()))).contains("unknown"));
            }
            assert_eq!(c.writes().len(), 1);
            assert_eq!(j.events(), ["started"]);
        }
    }

    /// `test_stale_identity_partial_membership_and_noop`.
    #[test]
    fn stale_identity_and_partial_membership() {
        let w = world();
        let c = client(&w);
        let j = MemJournal::default();
        let p = build_plan(&c, "tickets", "123", true).unwrap();
        w.lock().unwrap().user = 903;
        assert_eq!(
            err(send(&p, &c, &j, Some(&p.hash()))),
            "target or review changed; review again"
        );
        {
            let mut g = w.lock().unwrap();
            g.user = 901;
            g.listing = Some(json!({"watcher_ids": [], "has_more": true}));
        }
        assert_eq!(
            err(build_plan(&c, "tickets", "123", true)),
            "complete watcher membership is unavailable"
        );
        assert!(c.writes().is_empty());
    }

    /// `test_readback_failure_and_journal_failure_do_not_resend`.
    #[test]
    fn readback_failure_does_not_resend() {
        let w = world();
        let c = client(&w);
        let p = build_plan(&c, "tickets", "123", true).unwrap();
        w.lock().unwrap().fail_reads_after_write = true;
        let out = send(&p, &c, &MemJournal::default(), Some(&p.hash())).unwrap();
        assert_eq!(out["verified"], false);
        assert!(out["warning"].as_str().unwrap().contains("do not resend"));
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_noop_and_missing_journal`.
    #[test]
    fn noop_and_missing_journal() {
        let w = world();
        let c = client(&w);
        let p = build_plan(&c, "tickets", "123", false).unwrap();
        let h = p.hash();
        assert_eq!(
            send(&p, &c, &MemJournal::default(), Some(&h)).unwrap(),
            json!({"status": "already_in_requested_state", "verified": true, "plan_hash": h})
        );
        let q = build_plan(&c, "tickets", "123", true).unwrap();
        assert_eq!(
            err(send(&q, &c, &DeafJournal, Some(&q.hash()))),
            "could not persist ambiguity journal; no write attempted"
        );
        assert!(c.writes().is_empty());
        assert_eq!(
            send(&q, &c, &MemJournal::default(), None).unwrap(),
            json!({"status": "cancelled", "verified": false})
        );
    }

    #[test]
    fn inventory_and_identity_checks() {
        let list = |v: Value| {
            let w = world();
            w.lock().unwrap().listing = Some(v);
            inventory(&client(&w), "tickets", 1)
        };
        assert_eq!(
            list(json!({"watcher_ids": [3, "1"], "total": 2})).unwrap(),
            [1, 3]
        );
        assert_eq!(
            err(list(json!({"watcher_ids": [1, 1]}))),
            "watcher membership contains duplicate identities"
        );
        assert_eq!(
            err(list(json!({"watcher_ids": [1], "total": 2}))),
            "watcher membership is incomplete"
        );
        assert_eq!(
            err(list(json!({"watcher_ids": [true]}))),
            "expected a positive numeric identity"
        );
        assert_eq!(
            err(list(json!([1]))),
            "complete watcher membership is unavailable"
        );
        let me_list = FnApi::new(|_| Ok(json!([1])));
        assert_eq!(
            err(self_id(&me_list)),
            "signed-in agent identity is unavailable"
        );
        let me_zero = FnApi::new(|_| Ok(json!({"id": 0})));
        assert_eq!(
            err(self_id(&me_zero)),
            "expected a positive numeric identity"
        );
        let me_fail = FnApi::new(|_| Err(Error::session("expired")));
        assert!(matches!(self_id(&me_fail), Err(Error::Session(_))));
        assert_eq!(
            err(build_plan(&me_fail, "problems", "1", true)),
            "only ticket/change self-watch contracts are proven"
        );
    }
}
