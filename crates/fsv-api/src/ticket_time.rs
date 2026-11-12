//! Captured manual, nonbillable, self-owned ticket time-entry CRUD
//! (`ticket_time.py`).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_py::{py_str, truthy};

use crate::readback::{duration_seconds, same_duration, same_instant};
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, plan_hash};
use crate::watchers::{positive_id, self_id};

/// Port of fsv.ticket_time.inventory: every time entry of the ticket,
/// sorted by id; an incomplete listing is refused.
pub fn inventory(c: &dyn Api, target: i64) -> Result<Vec<Value>> {
    let params = obj(json!({"page": 1, "per_page": 30}));
    let pages = collect_pages(
        |path, params| c.int_get(path, params),
        &format!("tickets/{target}/time_entries"),
        "time_entries",
        Some(&params),
        "time entries",
        MAX_DETAIL_PAGES,
    )?;
    if !pages.meta.get("complete").is_some_and(truthy) {
        return Err(refused("complete time-entry inventory is unavailable"));
    }
    let mut keyed = pages
        .rows
        .into_iter()
        .map(|row| Ok((positive_id(row.get("id").unwrap_or(&Value::Null))?, row)))
        .collect::<Result<Vec<(i64, Value)>>>()?;
    keyed.sort_by_key(|(id, _)| *id);
    if keyed.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err(refused(
            "time-entry inventory contains duplicate identities",
        ));
    }
    Ok(keyed.into_iter().map(|(_, row)| row).collect())
}

/// Port of fsv.ticket_time._duration: a positive `HH:MM` manual time.
pub fn duration(value: &str) -> Result<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{2,}:[0-5]\d$").expect("regex"));
    let zero = value
        .chars()
        .all(|c| c == ':' || fsv_core::pystr::decimal_value(c) == Some(0));
    if !RE.is_match(value) || zero {
        return Err(Error::value(
            "manual time must be a positive HH:MM duration",
        ));
    }
    Ok(value.to_string())
}

/// Keyword arguments of [`build_plan`].
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeArgs<'a> {
    /// `create` | `update` | `delete`.
    pub action: &'a str,
    pub entry_id: Option<i64>,
    pub spent: Option<&'a str>,
    pub executed_at: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// Port of fsv.ticket_time.build_plan.
pub fn build_plan(c: &dyn Api, target: &str, args: &TimeArgs<'_>) -> Result<WritePlan> {
    let action = args.action;
    if !matches!(action, "create" | "update" | "delete") {
        return Err(Error::value("unsupported time-entry action"));
    }
    let cid = positive_id(&json!(parse_id(target, &TICKETS)?))?;
    let user = self_id(c)?;
    let rows = inventory(c, cid)?;
    if args
        .note
        .is_some_and(|n| fsv_core::pystr::strip(n).is_empty())
    {
        return Err(Error::value("note must be nonempty text when provided"));
    }
    let (method, path, payload) = if action == "create" {
        let (None, Some(spent), Some(executed_at)) = (args.entry_id, args.spent, args.executed_at)
        else {
            return Err(Error::value(
                "create requires --spent and --executed-at, without an entry ID",
            ));
        };
        let Some(when) = crate::tasks::date(Some(executed_at))? else {
            return Err(Error::value("executed-at needs an explicit offset"));
        };
        let payload = json!({"executed_at": when, "timer_running": false, "billable": false,
                             "time_spent": duration(spent)?, "note": args.note.unwrap_or(""), "agent_id": user});
        ("POST", format!("tickets/{cid}/time_entries"), payload)
    } else {
        let eid = positive_id(&json!(args.entry_id))?;
        let row = rows
            .iter()
            .find(|r| positive_id(&r["id"]).ok() == Some(eid))
            .filter(|r| {
                py_str(r.get("agent_id").unwrap_or(&Value::Null)) == user.to_string()
                    && r.get("billable") == Some(&Value::Bool(false))
                    && r.get("timer_running") == Some(&Value::Bool(false))
            })
            .ok_or_else(|| refused("exact self-owned nonbillable manual entry is unavailable"))?;
        if args.executed_at.is_some() {
            return Err(Error::value(
                "changing executed-at is not a captured update contract",
            ));
        }
        let path = format!("tickets/{cid}/time_entries/{eid}");
        if action == "delete" {
            if args.spent.is_some() || args.note.is_some() {
                return Err(Error::value("delete does not accept time or note changes"));
            }
            ("DELETE", path, Value::Null)
        } else {
            if args.spent.is_none() && args.note.is_none() {
                return Err(Error::value("update requires --spent or --note"));
            }
            let (Some(current), Some(old_note)) = (row.get("time_spent"), row.get("note")) else {
                return Err(refused("entry time/note is unavailable"));
            };
            // The server returns HH:MM:SS even for minute-only writes.
            let spent = match args.spent {
                Some(s) => s.to_string(),
                None => {
                    let seconds = duration_seconds(current)?;
                    if seconds % 60 != 0 {
                        return Err(refused(
                            "entry has sub-minute precision; provide --spent explicitly",
                        ));
                    }
                    format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60)
                }
            };
            let note = args.note.map_or_else(|| old_note.clone(), |n| json!(n));
            (
                "PUT",
                path,
                json!({"time_spent": duration(&spent)?, "note": note}),
            )
        }
    };
    Ok(WritePlan {
        action: format!("ticket_time_{action}"),
        target: cid,
        tenant: c.domain(),
        method: method.into(),
        path,
        payload_json: fsv_py::Dumps::default()
            .sort_keys(true)
            .ensure_ascii(false)
            .dump(&payload),
        context_hash: plan_hash(&json!({"self": user, "entries": rows})),
    })
}

/// A payload string field as a rebuild argument; `None` when absent/null.
fn text_arg(payload: &Value, key: &str) -> Result<Option<String>> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Error::value(format!("{key} must be text"))),
    }
}

/// Port of fsv.ticket_time.send: one write, then the exact entry read back
/// (durations and instants compared semantically, other fields by type and
/// value).
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let action = plan
        .action
        .strip_prefix("ticket_time_")
        .unwrap_or(&plan.action)
        .to_string();
    let payload = plan.payload();
    let eid = if action == "create" {
        None
    } else {
        let tail = plan.path.rsplit('/').next().unwrap_or_default();
        Some(positive_id(&json!(tail))?)
    };
    let spent = text_arg(&payload, "time_spent")?;
    let executed_at = text_arg(&payload, "executed_at")?;
    // `payload.get("note") or None`.
    let note = match payload.get("note") {
        Some(v) if truthy(v) => match v.as_str() {
            Some(s) => Some(s.to_string()),
            None => return Err(Error::value("note must be nonempty text when provided")),
        },
        _ => None,
    };
    let target = plan.target.to_string();
    let rebuild = || {
        build_plan(
            c,
            &target,
            &TimeArgs {
                action: &action,
                entry_id: eid,
                spent: spent.as_deref(),
                executed_at: executed_at.as_deref(),
                note: note.as_deref(),
            },
        )
    };
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let exact_id = match eid {
            Some(id) => id,
            None => {
                let raw = match response {
                    Value::Object(m) => m.get("time_entry").unwrap_or(response),
                    _ => &Value::Null,
                };
                positive_id(raw.get("id").unwrap_or(&Value::Null))?
            }
        };
        let rows = inventory(c, plan.target)?;
        let want = exact_id.to_string();
        let row = rows
            .iter()
            .find(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == want);
        if action == "delete" {
            return Ok(obj(
                json!({"entry_id": exact_id, "verified": row.is_none()}),
            ));
        }
        let mut fields = Map::new();
        for (key, value) in payload.as_object().into_iter().flatten() {
            let state = match row.and_then(|r| r.get(key)) {
                None => "unavailable",
                Some(actual) => {
                    let matched = match key.as_str() {
                        "time_spent" => same_duration(actual, value),
                        "executed_at" => same_instant(actual, value),
                        _ => actual == value,
                    };
                    if matched { "matched" } else { "mismatched" }
                }
            };
            fields.insert(key.clone(), json!(state));
        }
        let verified = row.is_some() && fields.values().all(|v| v == "matched");
        Ok(obj(
            json!({"entry_id": exact_id, "fields": fields, "verified": verified}),
        ))
    };
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;
    use fsv_core::ApiError;

    /// `ticket_time.inventory(c, 123)` over each listing, from
    /// `gen_fsv_api_workflows.py` (`direct.time_inventory`).
    #[test]
    fn inventory_matches_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/workflows_oracle.json")).unwrap();
        for case in oracle["direct"]["time_inventory"].as_array().unwrap() {
            let reply = match &case["reply"] {
                Value::Number(n) => Reply::Status(n.as_u64().unwrap() as u16),
                v => Reply::Ok(v.clone()),
            };
            let c = FakeApi::new().on("GET tickets/123/time_entries", reply);
            let got = match inventory(&c, 123) {
                Ok(rows) => json!({"ok": rows}),
                Err(e) => json!({"error": e.to_string()}),
            };
            let want = match case.get("ok") {
                Some(ok) => json!({"ok": ok}),
                None => json!({"error": case["error"]}),
            };
            assert_eq!(got, want, "{case}");
        }
    }

    /// `ticket_time._duration(v)` from running Python.
    #[test]
    fn duration_matches_python() {
        for ok in [
            "01:30",
            "00:01",
            "001:00",
            "100:59",
            "٠١:30",
            "01:3٠",
            "１２:00",
        ] {
            assert_eq!(duration(ok).unwrap(), ok);
        }
        for bad in [
            "00:00",
            "٠٠:00",
            "1:30",
            "10:60",
            "10:5",
            "ab:cd",
            "00:00:00",
            "",
            "12:3a",
            "٠١:٣٠",
        ] {
            assert_eq!(
                duration(bad).unwrap_err().to_string(),
                "manual time must be a positive HH:MM duration",
                "{bad}"
            );
        }
    }

    type AfterPost = Box<dyn Fn(&Value) -> Vec<Value> + Send>;

    struct World {
        rows: Vec<Value>,
        failure: Option<u16>,
        omit_id: bool,
        incomplete: bool,
        /// Server-side normalization applied after a POST.
        after_post: Option<AfterPost>,
    }

    fn client(w: &Arc<Mutex<World>>) -> FnApi {
        let w = w.clone();
        FnApi::new(move |call| {
            let mut w = w.lock().unwrap();
            match call.method.as_str() {
                "ME" => Ok(json!({"id": 901})),
                "GET" => {
                    assert_eq!(call.path, "tickets/123/time_entries");
                    if w.incomplete {
                        return Ok(json!({"time_entries": w.rows, "has_more": true}));
                    }
                    Ok(json!({"time_entries": w.rows, "has_more": false}))
                }
                method => {
                    if let Some(s) = w.failure {
                        return Err(ApiError::new(s, json!("uncertain")).into());
                    }
                    let body = call.body.clone().unwrap_or(Value::Null);
                    match method {
                        "POST" => {
                            let rows = match &w.after_post {
                                Some(f) => f(&body),
                                None => {
                                    let mut row = json!({"id": 456});
                                    row.as_object_mut()
                                        .unwrap()
                                        .extend(body.as_object().unwrap().clone());
                                    let mut rows = w.rows.clone();
                                    rows.push(row);
                                    rows
                                }
                            };
                            w.rows = rows;
                        }
                        "PUT" => {
                            let first = w.rows[0].as_object_mut().unwrap();
                            first.extend(body.as_object().unwrap().clone());
                        }
                        _ => w.rows.clear(),
                    }
                    Ok(if w.omit_id {
                        json!({})
                    } else {
                        json!({"time_entry": {"id": 456}})
                    })
                }
            }
        })
    }

    fn world() -> Arc<Mutex<World>> {
        Arc::new(Mutex::new(World {
            rows: Vec::new(),
            failure: None,
            omit_id: false,
            incomplete: false,
            after_post: None,
        }))
    }

    fn create(c: &dyn Api) -> Result<WritePlan> {
        build_plan(
            c,
            "INC-123",
            &TimeArgs {
                action: "create",
                spent: Some("00:01"),
                executed_at: Some("2030-01-02T10:00:00-05:00"),
                note: Some("Acme"),
                ..Default::default()
            },
        )
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    /// `test_ticket_time.py::test_exact_wire_crud_and_independent_readback`.
    #[test]
    fn exact_wire_crud_and_readback() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        let w = world();
        let c = client(&w);
        let j = MemJournal::default();
        let p = create(&c).unwrap();
        assert_eq!(
            p.payload(),
            json!({"executed_at": "2030-01-02T10:00:00-05:00", "timer_running": false,
                   "billable": false, "time_spent": "00:01", "note": "Acme", "agent_id": 901})
        );
        assert_eq!(p.payload_json, oracle["ticket_time"]["create_payload_json"]);
        assert_eq!(p.hash(), oracle["ticket_time"]["create_hash"]);
        let out = send(&p, &c, &j, Some(&p.hash())).unwrap();
        assert_eq!(
            (&out["verified"], &out["entry_id"]),
            (&json!(true), &json!(456))
        );
        let q = build_plan(
            &c,
            "123",
            &TimeArgs {
                action: "update",
                entry_id: Some(456),
                note: Some("Globex"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            q.payload(),
            json!({"time_spent": "00:01", "note": "Globex"})
        );
        assert_eq!(q.hash(), oracle["ticket_time"]["update_hash"]);
        assert_eq!(send(&q, &c, &j, Some(&q.hash())).unwrap()["verified"], true);
        let d = build_plan(
            &c,
            "123",
            &TimeArgs {
                action: "delete",
                entry_id: Some(456),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(d.payload(), Value::Null);
        assert_eq!(d.hash(), oracle["ticket_time"]["delete_hash"]);
        assert_eq!(send(&d, &c, &j, Some(&d.hash())).unwrap()["verified"], true);
        let methods: Vec<String> = c.writes().into_iter().map(|w| w.method).collect();
        assert_eq!(methods, ["POST", "PUT", "DELETE"]);
        assert!(w.lock().unwrap().rows.is_empty());
    }

    /// `test_unknown_response_id_never_infers_latest_or_resends`.
    #[test]
    fn unknown_response_id_never_resends() {
        let w = world();
        w.lock().unwrap().omit_id = true;
        let c = client(&w);
        let j = MemJournal::default();
        let p = create(&c).unwrap();
        let out = send(&p, &c, &j, Some(&p.hash())).unwrap();
        assert_eq!(out["verified"], false);
        assert!(out["warning"].as_str().unwrap().contains("do not resend"));
        assert!(err(send(&p, &c, &j, Some(&p.hash()))).contains("was sent"));
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_nonself_billable_or_timer_entry_refused`.
    #[test]
    fn nonself_billable_or_timer_refused() {
        for (k, v) in [
            ("agent_id", json!(902)),
            ("billable", json!(true)),
            ("timer_running", json!(true)),
        ] {
            let w = world();
            let mut row = json!({"id": 456, "agent_id": 901, "billable": false, "timer_running": false, "time_spent": "00:01", "note": "Acme"});
            row[k] = v;
            w.lock().unwrap().rows = vec![row];
            let c = client(&w);
            let args = TimeArgs {
                action: "delete",
                entry_id: Some(456),
                ..Default::default()
            };
            assert!(err(build_plan(&c, "123", &args)).contains("self-owned"));
            assert!(c.writes().is_empty());
        }
    }

    /// `test_stale_inventory_ambiguous_write_and_partial_read`.
    #[test]
    fn stale_inventory_ambiguous_write_partial_read() {
        let w = world();
        let c = client(&w);
        let j = MemJournal::default();
        let p = create(&c).unwrap();
        w.lock().unwrap().rows.push(json!({"id": 999}));
        assert!(err(send(&p, &c, &j, Some(&p.hash()))).contains("changed"));
        {
            let mut g = w.lock().unwrap();
            g.rows.clear();
            g.failure = Some(403);
        }
        for _ in 0..2 {
            assert!(err(send(&p, &c, &j, Some(&p.hash()))).contains("unknown"));
        }
        assert_eq!(c.writes().len(), 1);
        w.lock().unwrap().incomplete = true;
        assert_eq!(
            err(create(&c)),
            "complete time-entry inventory is unavailable"
        );
    }

    /// `test_duration_validation`.
    #[test]
    fn duration_validation() {
        for spent in ["1", "00:00", "00:60", "-1:00"] {
            let c = client(&world());
            let args = TimeArgs {
                action: "create",
                spent: Some(spent),
                executed_at: Some("2030-01-02T10:00:00Z"),
                ..Default::default()
            };
            assert_eq!(
                err(build_plan(&c, "123", &args)),
                "manual time must be a positive HH:MM duration"
            );
        }
    }

    /// `test_readback_normalization.py::test_normalized_exact_id_transport_readback`.
    #[test]
    fn normalized_exact_id_readback() {
        let cases: [(Value, Option<(&str, &str)>); 6] = [
            (json!({}), None),
            (
                json!({"time_spent": "00:06:00"}),
                Some(("time_spent", "mismatched")),
            ),
            (
                json!({"executed_at": "2030-01-01T20:00:01Z"}),
                Some(("executed_at", "mismatched")),
            ),
            (
                json!({"executed_at": null}),
                Some(("executed_at", "mismatched")),
            ),
            (json!({"billable": 0}), Some(("billable", "mismatched"))),
            (
                json!({"omit": "executed_at"}),
                Some(("executed_at", "unavailable")),
            ),
        ];
        for (overrides, expect) in cases {
            let w = world();
            w.lock().unwrap().after_post = Some(Box::new(move |body: &Value| {
                let mut row = json!({"id": 456});
                let m = row.as_object_mut().unwrap();
                m.extend(body.as_object().unwrap().clone());
                m.insert("time_spent".into(), json!("00:05:00"));
                m.insert("executed_at".into(), json!("2030-01-01T20:00:00Z"));
                for (k, v) in overrides.as_object().unwrap() {
                    if k == "omit" {
                        m.shift_remove(v.as_str().unwrap());
                    } else {
                        m.insert(k.clone(), v.clone());
                    }
                }
                let mut other = json!({"id": 999});
                other
                    .as_object_mut()
                    .unwrap()
                    .extend(body.as_object().unwrap().clone());
                vec![row, other]
            }));
            let c = client(&w);
            let j = MemJournal::default();
            let p = build_plan(
                &c,
                "123",
                &TimeArgs {
                    action: "create",
                    spent: Some("00:05"),
                    executed_at: Some("2030-01-02T03:00:00+07:00"),
                    ..Default::default()
                },
            )
            .unwrap();
            let out = send(&p, &c, &j, Some(&p.hash())).unwrap();
            assert_eq!(out["entry_id"], 456);
            assert_eq!(out["verified"], expect.is_none(), "{out}");
            if let Some((field, state)) = expect {
                assert_eq!(out["fields"][field], state);
            }
            let writes = c.writes();
            assert_eq!(writes.len(), 1);
            let body = writes[0].body.clone().unwrap();
            assert_eq!(
                (&body["time_spent"], &body["executed_at"]),
                (&json!("00:05"), &json!("2030-01-02T03:00:00+07:00"))
            );
            assert!(err(send(&p, &c, &j, Some(&p.hash()))).contains("was sent"));
        }
    }

    /// `test_readback_normalization.py::test_note_only_update_preserves_normalized_minutes`.
    #[test]
    fn note_only_update_preserves_minutes() {
        let c = FnApi::new(|call| {
            Ok(if call.method == "ME" {
                json!({"id": 901})
            } else {
                json!({"time_entries": [{"id": 456, "agent_id": 901, "billable": false,
                       "timer_running": false, "time_spent": "125:05:00", "note": "Acme"}], "has_more": false})
            })
        });
        let p = build_plan(
            &c,
            "123",
            &TimeArgs {
                action: "update",
                entry_id: Some(456),
                note: Some("Globex"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            p.payload(),
            json!({"time_spent": "125:05", "note": "Globex"})
        );
    }

    #[test]
    fn argument_errors() {
        let w = world();
        w.lock().unwrap().rows = vec![
            json!({"id": 456, "agent_id": 901, "billable": false, "timer_running": false, "time_spent": "00:01:30", "note": "Acme"}),
        ];
        let c = client(&w);
        let plan = |a: TimeArgs<'_>| err(build_plan(&c, "123", &a));
        let base = TimeArgs {
            entry_id: Some(456),
            ..Default::default()
        };
        assert_eq!(
            plan(TimeArgs {
                action: "stop",
                ..base
            }),
            "unsupported time-entry action"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "update",
                ..base
            }),
            "update requires --spent or --note"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "update",
                note: Some("x"),
                ..base
            }),
            "entry has sub-minute precision; provide --spent explicitly"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "update",
                spent: Some("00:00"),
                ..base
            }),
            "manual time must be a positive HH:MM duration"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "delete",
                note: Some("x"),
                ..base
            }),
            "delete does not accept time or note changes"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "update",
                executed_at: Some("2030-01-01T00:00:00Z"),
                ..base
            }),
            "changing executed-at is not a captured update contract"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "create",
                note: Some(" "),
                ..base
            }),
            "note must be nonempty text when provided"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "create",
                ..base
            }),
            "create requires --spent and --executed-at, without an entry ID"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "create",
                entry_id: None,
                spent: Some("00:01"),
                executed_at: Some(" "),
                note: None
            }),
            "executed-at needs an explicit offset"
        );
        assert_eq!(
            plan(TimeArgs {
                action: "update",
                entry_id: None,
                note: Some("x"),
                ..base
            }),
            "expected a positive numeric identity"
        );
    }
}
