//! Ports of `tests/test_templates_windows.py` (maintenance windows, the
//! `wapi` fixture as a stateful [`FnApi`]). Hashes and reviews in
//! `testdata/windows_oracle.json` come from `uv run python` on
//! `change_windows.create_plan`/`delete_plan` with the same fakes.

use std::sync::{Arc, Mutex};

use super::*;
use crate::service::testing::{FnApi, MemJournal};

const NOW_WINDOW: &str = r#"{"id": 61, "name": "Globex freeze", "start_time": "2030-11-20T09:00:00Z", "end_time": "2030-11-20T10:00:00Z"}"#;

fn ny_a() -> Value {
    json!({"id": 71, "name": "Acme A", "start_time": "2030-11-14T02:00:00-05:00", "end_time": "2030-11-14T04:00:00-05:00"})
}

fn ny_c() -> Value {
    json!({"id": 72, "name": "Acme C", "start_time": "2030-11-14T04:00:00-05:00", "end_time": "2030-11-14T06:00:00-05:00"})
}

struct S {
    windows: Vec<Value>,
    overlaps: Vec<Value>,
    answer: Option<Value>,
    agent: Option<Value>,
    tenant_tz: String,
}

type State = Arc<Mutex<S>>;

/// The tenant stores the wall clock in ITS zone.
fn stored(day: &str, clock: &str, tz: &str) -> String {
    let (d, m, y) = (&day[0..2], &day[3..5], &day[6..10]);
    let clock = clock.to_ascii_lowercase();
    let pm = clock.ends_with("pm");
    let (h, mi) = clock[..clock.len() - 2].split_once(':').unwrap();
    let mut hour: i8 = h.parse().unwrap();
    hour = match (hour, pm) {
        (12, false) => 0,
        (12, true) => 12,
        (h, true) => h + 12,
        (h, false) => h,
    };
    let local = jiff::civil::date(y.parse().unwrap(), m.parse().unwrap(), d.parse().unwrap()).at(
        hour,
        mi.parse().unwrap(),
        0,
        0,
    );
    let z = zone(tz).unwrap();
    aware(local, fold0_offset(local, &z)).isoformat()
}

fn wapi() -> (FnApi, State) {
    let s: State = Arc::new(Mutex::new(S {
        windows: vec![serde_json::from_str(NOW_WINDOW).unwrap()],
        overlaps: Vec::new(),
        answer: None,
        agent: Some(json!({"id": 7, "time_zone": "Eastern Time (US & Canada)"})),
        tenant_tz: "America/New_York".into(),
    }));
    let st = s.clone();
    let api = FnApi::new(move |call| {
        let mut s = st.lock().unwrap();
        match (call.method.as_str(), call.path.as_str()) {
            ("ME", _) => s
                .agent
                .clone()
                .ok_or_else(|| fsv_core::ApiError::new(404, json!({})).into()),
            ("GET", "maintenance_windows") => Ok(json!({"maintenance_windows": s.windows})),
            ("POST", OVERLAP_PATH) => Ok(s
                .answer
                .clone()
                .unwrap_or_else(|| json!({"overlapping_windows": s.overlaps}))),
            ("POST", "maintenance_windows") => {
                let cw = call.body.as_ref().unwrap()["change_window"].clone();
                let tz = s.tenant_tz.clone();
                let at = |d: &str, t: &str| {
                    stored(cw[d].as_str().unwrap(), cw[t].as_str().unwrap(), &tz)
                };
                let row = json!({"id": 62, "name": cw["name"],
                    "start_time": at("start_date", "start_time"), "end_time": at("end_date", "end_time")});
                s.windows.push(row.clone());
                Ok(json!({"maintenance_window": row}))
            }
            ("DELETE", path) => {
                let wid: i64 = path.rsplit('/').next().unwrap().parse().unwrap();
                s.windows.retain(|w| w["id"] != json!(wid));
                Ok(json!({"success": true, "message": "delete_success"}))
            }
            other => panic!("unexpected {other:?}"),
        }
    });
    (api, s)
}

fn sch() -> Value {
    json!({"fields": [{"name": "change_type", "label": "Change Type", "workspace_id": 3,
                       "choices": [{"id": 1, "value": "Normal"}, {"id": 4, "value": "Emergency"}]}]})
}

/// `views.current_user` over the fake: unreadable -> `{}`.
fn me_of(c: &dyn Api) -> Result<Value> {
    match c.me() {
        Ok(v) if v.is_object() => Ok(v),
        Err(e @ Error::Session(_)) => Err(e),
        _ => Ok(json!({})),
    }
}

struct Kw<'a> {
    name: &'a str,
    description: &'a str,
    start: &'a str,
    end: &'a str,
    criteria: Vec<String>,
    timezone: Option<&'a str>,
    allow_overlap: bool,
    fsv_tz: Option<&'a str>,
}

fn kw() -> Kw<'static> {
    Kw {
        name: "Acme release",
        description: "Quarterly",
        start: "2030-11-21 10:00",
        end: "2030-11-21 13:30",
        criteria: vec!["change_type=Normal".into()],
        timezone: None,
        allow_overlap: false,
        fsv_tz: None,
    }
}

fn zkw<'a>(start: &'a str, end: &'a str) -> Kw<'a> {
    Kw {
        name: "Acme zone",
        description: "",
        start,
        end,
        ..kw()
    }
}

fn build(c: &FnApi, k: &Kw<'_>) -> Result<WindowPlan> {
    let schema = || Ok(sch());
    let me = || me_of(c);
    let env = WindowEnv {
        schema: &schema,
        configured_workspace: Value::Null,
        fsv_tz: k.fsv_tz.map(str::to_string),
        current_user: &me,
    };
    create_plan(
        c,
        &env,
        &CreateArgs {
            name: k.name,
            description: k.description,
            start: k.start,
            end: k.end,
            criteria: &k.criteria,
            alert_suppression: true,
            timezone: k.timezone,
            allow_overlap: k.allow_overlap,
        },
    )
}

fn plan(c: &FnApi) -> WindowPlan {
    build(c, &kw()).unwrap()
}

fn allowed(c: &FnApi) -> WindowPlan {
    build(
        c,
        &Kw {
            allow_overlap: true,
            ..kw()
        },
    )
    .unwrap()
}

fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
    r.unwrap_err().to_string()
}

fn write_paths(c: &FnApi) -> Vec<String> {
    c.writes().into_iter().map(|w| w.path).collect()
}

fn send(p: &WindowPlan, c: &FnApi, j: &MemJournal) -> Result<Value> {
    send_create(p, c, j, Some(&p.hash()), None)
}

fn send_rebuilt(p: &WindowPlan, c: &FnApi, j: &MemJournal, k: &Kw<'_>) -> Result<Value> {
    let rebuild = || build(c, k);
    send_create(p, c, j, Some(&p.hash()), Some(&rebuild))
}

fn oracle() -> Value {
    serde_json::from_str(include_str!("testdata/windows_oracle.json")).unwrap()
}

#[test]
fn hashes_and_reviews_match_python() {
    let o = oracle();
    let (c, s) = wapi();
    let check = |name: &str, p: &WindowPlan| {
        let want = &o[name];
        assert_eq!(p.hash(), want["hash"], "{name}");
        assert_eq!(p.legacy_hash, want["legacy"], "{name}");
        assert_eq!(p.write.payload_json, want["payload_json"], "{name}");
        assert_eq!(p.overlap_json, want["overlap_json"], "{name}");
        assert_eq!(
            (p.start.as_str(), p.end.as_str()),
            (
                want["start"].as_str().unwrap(),
                want["end"].as_str().unwrap()
            )
        );
        let review = Value::Object(p.review().unwrap());
        assert_eq!(review, want["review"], "{name}");
        let keys: Vec<&String> = review.as_object().unwrap().keys().collect();
        let want_keys: Vec<&String> = want["review"].as_object().unwrap().keys().collect();
        assert_eq!(keys, want_keys, "{name}");
    };
    check("default", &plan(&c));
    check("allow", &allowed(&c));
    s.lock().unwrap().windows = vec![ny_a(), ny_c()];
    let bkk = zkw("2030-11-14 15:00", "2030-11-14 17:00");
    check(
        "bangkok",
        &build(
            &c,
            &Kw {
                timezone: Some("Asia/Bangkok"),
                ..bkk
            },
        )
        .unwrap(),
    );
    let env = zkw("2030-11-14 15:00", "2030-11-14 17:00");
    check(
        "env",
        &build(
            &c,
            &Kw {
                fsv_tz: Some("Asia/Bangkok"),
                ..env
            },
        )
        .unwrap(),
    );
    let off = zkw("2030-11-14 15:00+07:00", "2030-11-14T16:00+07:00");
    check(
        "offset",
        &build(
            &c,
            &Kw {
                timezone: Some("Europe/London"),
                fsv_tz: Some("Asia/Bangkok"),
                ..off
            },
        )
        .unwrap(),
    );
    s.lock().unwrap().windows =
        vec![json!({"id": 76, "name": "Acme odd", "start_time": "soon", "end_time": "later"})];
    let odd = zkw("2030-11-14 08:00", "2030-11-14 09:00");
    check(
        "unknown_allow",
        &build(
            &c,
            &Kw {
                allow_overlap: true,
                ..odd
            },
        )
        .unwrap(),
    );
    s.lock().unwrap().windows = vec![serde_json::from_str(NOW_WINDOW).unwrap()];
    let d = delete_plan(&c, 61).unwrap();
    assert_eq!(d.hash(), o["delete"]["hash"]);
    assert_eq!(Value::Object(d.review()), o["delete"]["review"]);
}

/// `test_window_create_sends_overlap_check_then_captured_body`.
#[test]
fn create_sends_overlap_check_then_captured_body() {
    let (c, _) = wapi();
    let p = plan(&c);
    assert_eq!(
        p.write.payload(),
        json!({"change_window": {
            "name": "Acme release", "description": "Quarterly", "start_date": "21-11-2030", "start_time": "10:00am",
            "end_date": "21-11-2030", "end_time": "1:30pm",
            "change_event": {"additional_info": {}, "criteria": [{"name": "change_type", "operator": "is", "value": "1"}]},
            "workspace_id": 3, "alert_suppression": true}})
    );
    assert!(c.writes().is_empty());
    let j = MemJournal::default();
    let r = send_rebuilt(&p, &c, &j, &kw()).unwrap();
    assert_eq!(
        (&r["verified"], &r["window_id"]),
        (&json!(true), &json!(62))
    );
    assert_eq!(write_paths(&c), [OVERLAP_PATH, "maintenance_windows"]);
    assert_eq!(
        c.writes()[0].body.as_ref().unwrap()["type"],
        "blackout_events"
    );
    assert!(err(send(&p, &c, &j)).contains("do not replay"));
    assert_eq!(c.writes().len(), 2);
}

/// `test_window_overlap_and_invalid_input_refuse`.
#[test]
fn overlap_and_invalid_input_refuse() {
    let (c, s) = wapi();
    s.lock().unwrap().overlaps = vec![json!({"id": 61})];
    let p = plan(&c);
    assert!(err(send(&p, &c, &MemJournal::default())).contains("overlapping"));
    assert_eq!(write_paths(&c), [OVERLAP_PATH]);
    let base = || Kw {
        name: "x",
        description: "",
        start: "2030-11-21 10:00",
        end: "2030-11-21 11:00",
        ..kw()
    };
    let cases = [
        Kw {
            criteria: vec![],
            ..base()
        },
        Kw {
            criteria: vec!["priority=High".into()],
            ..base()
        },
        Kw {
            criteria: vec!["change_type=Minor".into()],
            ..base()
        },
        Kw {
            end: "2030-11-21 09:00",
            ..base()
        },
        Kw {
            start: "21-11-2030 10:00",
            ..base()
        },
    ];
    for k in &cases {
        assert!(matches!(build(&c, k), Err(Error::Exit { code: 2, .. })));
    }
}

/// `test_window_delete_and_cli` (library half; the CLI half is in fsv-cli).
#[test]
fn delete_reviewed_and_verified() {
    let (c, s) = wapi();
    let d = delete_plan(&c, 61).unwrap();
    let r = send_delete(&d, &c, &MemJournal::default(), Some(&d.hash())).unwrap();
    assert_eq!(r["fields"], json!({"absent": "matched"}));
    assert_eq!(r["verified"], json!(true));
    let w = c.writes();
    assert_eq!(
        (w[0].method.as_str(), w[0].path.as_str(), &w[0].body),
        ("DELETE", "maintenance_windows/61", &None)
    );
    assert!(s.lock().unwrap().windows.is_empty());
    assert_eq!(err(delete_plan(&c, 61)), "maintenance window 61 not found");
}

fn text(s: &str) -> Value {
    Value::String(s.into())
}

/// `test_window_overlap_answer_shapes_without_conflicts_create`.
#[test]
fn answer_shapes_without_conflicts_create() {
    let answers = [
        json!([]),
        json!({}),
        json!(false),
        json!({"overlapping_windows": []}),
        json!({"overlaps": []}),
        json!({"conflicts": [], "count": 0}),
        json!({"blackout_windows": [], "maintenance_windows": []}),
        json!({"overlap": false}),
        json!({"is_overlapping": false}),
        json!({"overlapping_windows": {"blackout_windows": [], "maintenance_windows": []}}),
        json!({"match_found": false}),
        text("false"),
    ];
    for answer in answers {
        let (c, s) = wapi();
        s.lock().unwrap().answer = Some(answer.clone());
        let p = plan(&c);
        let r = send_rebuilt(&p, &c, &MemJournal::default(), &kw()).unwrap();
        assert_eq!(
            (&r["verified"], &r["overlaps"]),
            (&json!(true), &json!(0)),
            "{answer}"
        );
        assert_eq!(write_paths(&c), [OVERLAP_PATH, "maintenance_windows"]);
    }
}

/// `test_window_overlap_answer_shapes_with_conflicts_refuse`.
#[test]
fn answer_shapes_with_conflicts_refuse() {
    let answers = [
        json!([{"id": 61}]),
        json!(true),
        json!({"has_overlap": true}),
        json!({"match_found": true}),
        json!({"overlap_count": 2}),
        json!({"conflicts": [{"id": 61}]}),
        json!({"blackout_windows": [], "maintenance_windows": [{"id": 61}]}),
        json!({"overlapping_windows": {"maintenance_windows": [{"id": 61}]}}),
    ];
    for answer in answers {
        let (c, s) = wapi();
        s.lock().unwrap().answer = Some(answer.clone());
        let p = plan(&c);
        assert!(
            err(send(&p, &c, &MemJournal::default())).contains("overlapping window"),
            "{answer}"
        );
        assert_eq!(write_paths(&c), [OVERLAP_PATH]);
    }
}

/// `test_window_overlap_unknown_answer_refuses_unless_allowed`.
#[test]
fn unknown_answer_refuses_unless_allowed() {
    let answers = [
        json!({"message": "Acme"}),
        json!({"status": "ok"}),
        json!({"overlap": false, "windows": [{"id": 61}]}),
        json!({"match_found": "no"}),
        json!({"match_found": false, "windows": [{"id": 61}]}),
        json!({"matches": true}),
        json!({"a": {"b": {"c": []}}}),
        Value::Null,
        text("Globex"),
    ];
    for answer in answers {
        let (c, s) = wapi();
        s.lock().unwrap().answer = Some(answer.clone());
        let p = plan(&c);
        let msg = err(send(&p, &c, &MemJournal::default()));
        assert!(msg.contains("nothing was created"), "{answer}: {msg}");
        assert!(!msg.contains("Acme") && !msg.contains("Globex"), "{msg}");
        assert_eq!(write_paths(&c), [OVERLAP_PATH]);
        if msg.contains("contradictory") {
            continue;
        }
        let pa = allowed(&c);
        assert_ne!(pa.hash(), p.hash());
        let r = send_rebuilt(
            &pa,
            &c,
            &MemJournal::default(),
            &Kw {
                allow_overlap: true,
                ..kw()
            },
        )
        .unwrap();
        assert_eq!(
            (&r["verified"], &r["overlaps"]),
            (&json!(true), &json!("unknown"))
        );
        assert_eq!(
            write_paths(&c),
            [OVERLAP_PATH, OVERLAP_PATH, "maintenance_windows"]
        );
    }
}

#[test]
fn answer_shape_outline_never_shows_values() {
    assert_eq!(
        err(overlaps(
            &json!({"message": "Acme", "n": 1, "x": {"y": {"z": 1}, "l": [1]}})
        )),
        "overlap check answer not understood (shape {message: text, n: number, x: {y: object, l: list}}); nothing was created. Check `fsv changes windows list` yourself; --allow-overlap skips this check"
    );
}

/// `test_window_existing_maintenance_overlap_refuses_whatever_the_check_says`.
#[test]
fn existing_maintenance_overlap_refuses_whatever_the_check_says() {
    let ranges = [
        ("2030-11-21T13:00:00-05:00", "2030-11-21T14:00:00-05:00"),
        ("2030-11-21T18:00:00Z", "2030-11-21T19:00:00Z"),
        ("2030-11-21T15:30:00", "2030-11-21T16:00:00"),
    ];
    for answer in [
        json!({"match_found": false}),
        json!([]),
        json!({"overlapping_windows": []}),
    ] {
        for (start, end) in ranges {
            let (c, s) = wapi();
            {
                let mut s = s.lock().unwrap();
                s.windows.push(json!({"id": 63, "name": "Acme window A", "start_time": start, "end_time": end}));
                s.answer = Some(answer.clone());
            }
            let p = plan(&c);
            let msg = err(send_rebuilt(&p, &c, &MemJournal::default(), &kw()));
            assert!(
                msg.contains("overlapping window(s) already listed (63)"),
                "{msg}"
            );
            assert!(c.writes().is_empty());
            let pa = allowed(&c);
            assert_ne!(pa.hash(), p.hash());
            let r = send_rebuilt(
                &pa,
                &c,
                &MemJournal::default(),
                &Kw {
                    allow_overlap: true,
                    ..kw()
                },
            )
            .unwrap();
            assert_eq!(r["verified"], json!(true));
            assert_eq!(r["overlapping_window_ids"], json!([63]));
            assert_eq!(r["overlaps"], json!(1));
            assert_eq!(write_paths(&c), [OVERLAP_PATH, "maintenance_windows"]);
        }
    }
}

/// `test_window_empty_list_and_back_to_back_windows_create`.
#[test]
fn empty_list_and_back_to_back_windows_create() {
    for answer in [json!({"match_found": false}), json!([])] {
        let (c, s) = wapi();
        {
            let mut s = s.lock().unwrap();
            s.answer = Some(answer);
            s.windows.clear();
        }
        let p = plan(&c);
        let r = send_rebuilt(&p, &c, &MemJournal::default(), &kw()).unwrap();
        assert_eq!((&r["verified"], &r["overlaps"]), (&json!(true), &json!(0)));
        assert_eq!(r["overlapping_window_ids"], json!([]));
        s.lock().unwrap().windows = vec![json!({"id": 64, "name": "Globex after",
            "start_time": "2030-11-21T18:30:00Z", "end_time": "2030-11-21T20:00:00Z"})];
        let p = build(
            &c,
            &Kw {
                name: "Acme second",
                description: "",
                ..kw()
            },
        )
        .unwrap();
        let r = send(&p, &c, &MemJournal::default()).unwrap();
        assert_eq!((&r["verified"], &r["overlaps"]), (&json!(true), &json!(0)));
        let rule = p.review().unwrap()["existing_window_check"]["rule"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(rule.starts_with("any listed window") && rule.contains("America/New_York"));
    }
}

/// `test_window_unreadable_existing_times_fail_closed`.
#[test]
fn unreadable_existing_times_fail_closed() {
    let bad = [
        json!({"start_time": "a", "end_time": "b"}),
        json!({"start_time": null}),
        json!({"start_time": "2030-11-21T10:00:00Z", "end_time": "2030-11-21T11:00:00"}),
        json!({"start_time": "10:00am", "end_time": "11:00am"}),
    ];
    for b in bad {
        let (c, s) = wapi();
        let mut row = obj(json!({"id": 65, "name": "Acme odd"}));
        row.extend(obj(b));
        s.lock().unwrap().windows = vec![Value::Object(row)];
        let p = plan(&c);
        let msg = err(send(&p, &c, &MemJournal::default()));
        assert!(
            msg.contains("unreadable start/end") && !msg.contains("10:00"),
            "{msg}"
        );
        assert!(c.writes().is_empty());
        let pa = allowed(&c);
        let r = send(&pa, &c, &MemJournal::default()).unwrap();
        assert_eq!(r["verified"], json!(true));
        assert_eq!(
            (&r["overlaps"], &r["overlapping_window_ids"]),
            (&json!("unknown"), &json!("unknown"))
        );
    }
}

/// `test_window_other_input_zone_converts_to_tenant_zone_before_check_and_send`.
#[test]
fn other_input_zone_converts_to_tenant_zone_before_check_and_send() {
    for via_option in [true, false] {
        let (c, s) = wapi();
        {
            let mut s = s.lock().unwrap();
            s.windows = vec![ny_a(), ny_c()];
            s.answer = Some(json!({"match_found": false}));
        }
        let with = |k: Kw<'static>| {
            if via_option {
                Kw {
                    timezone: Some("Asia/Bangkok"),
                    ..k
                }
            } else {
                Kw {
                    fsv_tz: Some("Asia/Bangkok"),
                    ..k
                }
            }
        };
        let p = build(&c, &with(zkw("2030-11-14 15:00", "2030-11-14 17:00"))).unwrap();
        let cw = p.write.payload()["change_window"].clone();
        assert_eq!(
            (cw["start_time"].as_str(), cw["end_time"].as_str()),
            (Some("3:00am"), Some("5:00am"))
        );
        let rv = p.review().unwrap();
        assert_eq!(
            rv["start"],
            json!({"input": "2030-11-14 15:00 Asia/Bangkok (+07:00)",
                                      "tenant": "2030-11-14 03:00 America/New_York (-05:00)"})
        );
        assert!(
            rv["time_zone"]["input"]
                .as_str()
                .unwrap()
                .starts_with("Asia/Bangkok (")
        );
        assert!(
            rv["time_zone"]["tenant"]
                .as_str()
                .unwrap()
                .starts_with("America/New_York")
        );
        assert!(err(send(&p, &c, &MemJournal::default())).contains("already listed (71, 72)"));
        assert!(c.writes().is_empty());
        let k = with(zkw("2030-11-14 03:00", "2030-11-14 05:00"));
        let p = build(&c, &k).unwrap();
        let cw = p.write.payload()["change_window"].clone();
        assert_eq!(
            [
                &cw["start_date"],
                &cw["start_time"],
                &cw["end_date"],
                &cw["end_time"]
            ],
            [
                &json!("13-11-2030"),
                &json!("3:00pm"),
                &json!("13-11-2030"),
                &json!("5:00pm")
            ]
        );
        let r = send_rebuilt(&p, &c, &MemJournal::default(), &k).unwrap();
        assert_eq!(r["verified"], json!(true));
        assert_eq!(r["overlapping_window_ids"], json!([]));
        assert_eq!(
            r["fields"],
            json!({"name": "matched", "start": "matched", "end": "matched"})
        );
    }
}

/// `test_window_timezone_option_beats_env_and_offsets_are_accepted`.
#[test]
fn timezone_option_beats_env_and_offsets_are_accepted() {
    let (c, _) = wapi();
    let k = Kw {
        timezone: Some("Europe/London"),
        fsv_tz: Some("Asia/Bangkok"),
        ..zkw("2030-11-14 08:00", "2030-11-14 09:00")
    };
    let p = build(&c, &k).unwrap();
    assert_eq!(p.write.payload()["change_window"]["start_time"], "3:00am");
    assert_eq!(
        p.review().unwrap()["time_zone"]["input"],
        "Europe/London (--timezone)"
    );
    let q = Kw {
        timezone: Some("Europe/London"),
        fsv_tz: Some("Asia/Bangkok"),
        ..zkw("2030-11-14 15:00+07:00", "2030-11-14T16:00+07:00")
    };
    assert_eq!(
        build(&c, &q).unwrap().write.payload()["change_window"]["start_time"],
        "3:00am"
    );
    let d = build(&c, &zkw("2030-11-14 08:00", "2030-11-14 09:00")).unwrap();
    assert_eq!(d.write.payload()["change_window"]["start_time"], "8:00am");
    assert_eq!(
        d.review().unwrap()["time_zone"]["input"],
        "America/New_York (tenant time zone)"
    );
    let explicit = Kw {
        timezone: Some("America/New_York"),
        ..zkw("2030-11-14 08:00", "2030-11-14 09:00")
    };
    assert_ne!(d.hash(), build(&c, &explicit).unwrap().hash());
}

/// `test_window_unknown_tenant_zone_refuses_even_with_fsv_tz`.
#[test]
fn unknown_tenant_zone_refuses_even_with_fsv_tz() {
    for agent in [
        None,
        Some(json!({"id": 7})),
        Some(json!({"id": 7, "time_zone": "Atlantis/Nowhere"})),
        Some(json!({"id": 7, "time_zone": 5})),
    ] {
        let (c, s) = wapi();
        s.lock().unwrap().agent = agent;
        let k = Kw {
            fsv_tz: Some("Asia/Bangkok"),
            ..zkw("2030-11-14 08:00", "2030-11-14 09:00")
        };
        assert!(err(build(&c, &k)).contains("tenant time zone unknown"));
        assert!(c.writes().is_empty());
    }
}

/// `test_window_dst_and_zone_edges_refuse`.
#[test]
fn dst_and_zone_edges_refuse() {
    let cases = [
        (
            "2030-03-10 02:30",
            "2030-03-10 04:00",
            None,
            "does not exist",
        ),
        (
            "2030-11-03 01:30",
            "2030-11-03 03:00",
            None,
            "happens twice",
        ),
        (
            "2030-11-03 12:30",
            "2030-11-03 14:00",
            Some("Asia/Bangkok"),
            "happens twice there",
        ),
        (
            "2030-11-14 08:00",
            "2030-11-14 09:00",
            Some("Mars/Olympus"),
            "unknown time zone",
        ),
        (
            "2030-11-14 08:00",
            "2030-11-14 08:00",
            None,
            "after its start",
        ),
        (
            "2030-11-14 09:00+00:00",
            "2030-11-14 15:30",
            Some("Asia/Bangkok"),
            "after its start",
        ),
        (
            "2030-11-14 8am",
            "2030-11-14 09:00",
            None,
            "must be 'YYYY-MM-DD HH:MM'",
        ),
    ];
    for (start, end, tz, want) in cases {
        let (c, _) = wapi();
        let msg = err(build(
            &c,
            &Kw {
                timezone: tz,
                ..zkw(start, end)
            },
        ));
        assert!(msg.contains(want), "{start}: {msg}");
        assert!(c.writes().is_empty());
    }
}

/// `test_window_dst_offsets_follow_the_date`.
#[test]
fn dst_offsets_follow_the_date() {
    let (c, _) = wapi();
    let summer = build(
        &c,
        &Kw {
            timezone: Some("Asia/Bangkok"),
            ..zkw("2030-07-01 20:00", "2030-07-01 21:00")
        },
    )
    .unwrap();
    let winter = build(
        &c,
        &Kw {
            timezone: Some("Asia/Bangkok"),
            ..zkw("2030-12-01 20:00", "2030-12-01 21:00")
        },
    )
    .unwrap();
    assert_eq!(
        summer.write.payload()["change_window"]["start_time"],
        "9:00am"
    );
    assert_eq!(
        winter.write.payload()["change_window"]["start_time"],
        "8:00am"
    );
    assert!(
        summer.review().unwrap()["start"]["tenant"]
            .as_str()
            .unwrap()
            .ends_with("(-04:00)")
    );
}

/// `test_window_readback_in_another_zone_is_not_verified`.
#[test]
fn readback_in_another_zone_is_not_verified() {
    let (c, s) = wapi();
    {
        let mut s = s.lock().unwrap();
        s.tenant_tz = "UTC".into();
        s.answer = Some(json!({"match_found": false}));
    }
    let p = build(&c, &zkw("2030-11-14 08:00", "2030-11-14 09:00")).unwrap();
    let r = send(&p, &c, &MemJournal::default()).unwrap();
    assert_eq!(r["verified"], json!(false));
    assert_eq!(
        (&r["fields"]["start"], &r["fields"]["name"]),
        (&json!("mismatched"), &json!("matched"))
    );
}

/// `test_window_legacy_hash_journal_still_guards`.
#[test]
fn legacy_hash_journal_still_guards() {
    let (c, _) = wapi();
    let p = build(&c, &zkw("2030-11-14 08:00", "2030-11-14 09:00")).unwrap();
    assert!(!p.legacy_hash.is_empty() && p.legacy_hash != p.hash());
    let j = MemJournal::default();
    j.record_plan(
        "started",
        &p.legacy_hash,
        ACTION_CREATE,
        Some(&json!(0)),
        None,
    );
    assert!(err(send(&p, &c, &j)).contains("do not replay"));
    assert!(c.writes().is_empty());
}

/// `test_window_dry_run_reports_overlapping_ids` (review half).
#[test]
fn review_reports_overlapping_ids() {
    let touch_c = json!({"id": 73, "name": "Acme C", "start_time": "2030-11-14T05:00:00-05:00", "end_time": "2030-11-14T06:00:00-05:00"});
    let touch_a = json!({"id": 74, "name": "Acme A", "start_time": "2030-11-14T01:00:00-05:00", "end_time": "2030-11-14T03:00:00-05:00"});
    let other = json!({"id": 75, "name": "Acme A", "start_time": "2030-11-14T07:59:00Z", "end_time": "2030-11-14T08:01:00Z"});
    let cases: [(Vec<Value>, Vec<i64>); 6] = [
        (vec![], vec![]),
        (vec![ny_a(), ny_c()], vec![71, 72]),
        (vec![ny_a()], vec![71]),
        (vec![touch_c], vec![]),
        (vec![touch_a], vec![]),
        (vec![other], vec![75]),
    ];
    for (windows, ids) in cases {
        let (c, s) = wapi();
        s.lock().unwrap().windows = windows;
        let p = build(
            &c,
            &Kw {
                name: "Acme",
                description: "",
                timezone: Some("Asia/Bangkok"),
                ..zkw("2030-11-14 15:00", "2030-11-14 17:00")
            },
        )
        .unwrap();
        let rv = p.review().unwrap();
        assert_eq!(rv["overlapping_window_ids"], json!(ids));
        if ids.is_empty() {
            assert!(!rv.contains_key("overlap_warning"));
        } else {
            assert_eq!(
                rv["overlap_warning"],
                json!(format!(
                    "overlapping windows exist: {}; sending refuses unless --allow-overlap",
                    join_ids(&ids)
                ))
            );
            assert!(err(send(&p, &c, &MemJournal::default())).contains("already listed"));
        }
        assert!(c.writes().is_empty());
    }
}

/// `test_window_dry_run_unreadable_times_warn_unknown`.
#[test]
fn review_unreadable_times_warn_unknown() {
    let (c, s) = wapi();
    s.lock().unwrap().windows =
        vec![json!({"id": 76, "name": "Acme odd", "start_time": "soon", "end_time": "later"})];
    let rv = build(&c, &zkw("2030-11-14 08:00", "2030-11-14 09:00"))
        .unwrap()
        .review()
        .unwrap();
    assert_eq!(rv["overlapping_window_ids"], "unknown");
    assert!(
        rv["overlap_warning"]
            .as_str()
            .unwrap()
            .starts_with("overlap unknown")
    );
    assert!(c.writes().is_empty());
}

/// `test_window_allow_overlap_changes_hash_and_warns` (library half).
#[test]
fn allow_overlap_changes_hash_and_warns() {
    let (c, s) = wapi();
    {
        let mut s = s.lock().unwrap();
        s.windows = vec![ny_a(), ny_c()];
        s.answer = Some(json!({"match_found": false}));
    }
    let base = Kw {
        name: "Acme",
        description: "",
        timezone: Some("Asia/Bangkok"),
        ..zkw("2030-11-14 15:00", "2030-11-14 17:00")
    };
    let plain = build(&c, &base).unwrap();
    let k = Kw {
        allow_overlap: true,
        ..base
    };
    let allow = build(&c, &k).unwrap();
    assert_ne!(plain.hash(), allow.hash());
    let rv = allow.review().unwrap();
    assert_eq!(
        (
            &rv["allow_overlap"],
            &plain.review().unwrap()["allow_overlap"]
        ),
        (&json!(true), &json!(false))
    );
    assert_eq!(rv["overlapping_window_ids"], json!([71, 72]));
    assert_eq!(
        rv["overlap_warning"],
        "overlapping windows exist: 71, 72; you chose to create it anyway (--allow-overlap)"
    );
    let j = MemJournal::default();
    assert!(
        err(send_create(&allow, &c, &j, Some(&plain.hash()), None))
            .contains("changed since review")
    );
    assert!(
        err(send_create(&plain, &c, &j, Some(&allow.hash()), None))
            .contains("changed since review")
    );
    assert!(c.writes().is_empty());
    let r = send_rebuilt(&allow, &c, &j, &k).unwrap();
    assert_eq!(r["verified"], json!(true));
    assert_eq!(r["overlapping_window_ids"], json!([71, 72]));
    assert_eq!(write_paths(&c), [OVERLAP_PATH, "maintenance_windows"]);
}

/// `test_window_allow_overlap_without_listed_overlap_still_says_so`.
#[test]
fn allow_overlap_without_listed_overlap_still_says_so() {
    let (c, s) = wapi();
    s.lock().unwrap().windows.clear();
    let k = Kw {
        allow_overlap: true,
        ..zkw("2030-11-14 08:00", "2030-11-14 09:00")
    };
    let rv = build(&c, &k).unwrap().review().unwrap();
    assert_eq!(rv["overlapping_window_ids"], json!([]));
    assert!(
        rv["overlap_warning"]
            .as_str()
            .unwrap()
            .starts_with("--allow-overlap:")
    );
    s.lock().unwrap().windows =
        vec![json!({"id": 76, "name": "Acme odd", "start_time": "soon", "end_time": "later"})];
    let rv = build(&c, &k).unwrap().review().unwrap();
    assert_eq!(
        rv["overlap_warning"],
        "overlap unknown: a listed window has unreadable start/end times; you chose to create it anyway (--allow-overlap)"
    );
}

#[test]
fn list_windows_refuses_malformed_lists() {
    let one = json!({"id": 61, "name": "Acme"});
    for (raw, want) in [
        (
            json!({"maintenance_windows": "x"}),
            "maintenance window list unavailable",
        ),
        (
            json!({"maintenance_windows": [one, one]}),
            "maintenance window list has duplicate ids",
        ),
        (
            json!({"maintenance_windows": [one], "meta": {"total_count": 2}}),
            "maintenance window list is incomplete; nothing selected",
        ),
    ] {
        let api = FnApi::new(move |_| Ok(raw.clone()));
        assert_eq!(err(list_windows(&api)), want);
    }
}

/// `_WHEN`'s `\d` matches any Unicode decimal digit, so such a time passes
/// the shape check and fails `fromisoformat` (texts from `_parse_when`).
#[test]
fn window_time_unicode_digits_fail_as_invalid_time() {
    for (text, why) in [
        (
            "\u{662}\u{660}\u{662}\u{664}-01-05 10:00",
            "is not a valid date and time",
        ),
        (
            "2024-01-05 10:00+0\u{667}:00",
            "is not a valid date and time",
        ),
        (
            "2024-01-05 10:00x",
            "must be 'YYYY-MM-DD HH:MM' (in --timezone; an offset such as +07:00 may be added)",
        ),
        ("2024-02-30 10:00", "is not a valid date and time"),
    ] {
        let err = parse_when(text).unwrap_err().to_string();
        assert_eq!(err, format!("window time {} {why}", py_repr_str(text)));
    }
}
