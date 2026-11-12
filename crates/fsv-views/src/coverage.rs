//! `detail.rs` / `source.rs` public API against `fsv.views` (and
//! `service.get_requested_items`): expected JSON comes from running the
//! Python loaders over the same fake client responses.

use std::sync::Mutex;

use jiff::tz::{self, TimeZone};
use serde_json::{Map, Value, json};

use fsv_api::site::{self, Lifecycle, LifecycleState, Missing, PlanningEntry, Prop, Tab};
use fsv_core::resource::{CHANGES, Resource, TICKETS};

use crate::detail::{
    Detail, DetailInput, TabData, load_detail, load_lifecycle, load_planning, load_tab,
};
use crate::source::fake::{raised, read_only_api};
use crate::source::{
    AssociationReader, FetchError, SessionError, collect_pages, get_requested_items, int_read,
    load_conversations, name_of, obj, py_int, rows_of, v2_read,
};
use fsv_api::service::Api;

type Replies = Vec<(String, Vec<Result<Value, FetchError>>)>;

/// Responses per path, served in order (the last one repeats); unknown paths 404.
#[derive(Default)]
struct Seq {
    int: Mutex<Replies>,
    v2: Mutex<Replies>,
    calls: Mutex<Vec<Value>>,
}

impl Seq {
    fn int(self, path: &str, replies: Vec<Result<Value, FetchError>>) -> Self {
        self.int.lock().unwrap().push((path.into(), replies));
        self
    }
    fn v2(self, path: &str, reply: Result<Value, FetchError>) -> Self {
        self.v2.lock().unwrap().push((path.into(), vec![reply]));
        self
    }
    fn calls(&self) -> Vec<Value> {
        self.calls.lock().unwrap().clone()
    }
    fn next(table: &Mutex<Replies>, path: &str) -> fsv_core::Result<Value> {
        let mut table = table.lock().unwrap();
        raised(match table.iter_mut().find(|(p, _)| p == path) {
            Some((_, replies)) if replies.len() > 1 => replies.remove(0),
            Some((_, replies)) => replies[0].clone(),
            None => Err(FetchError::Status(Some(404))),
        })
    }
}

impl Api for Seq {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> fsv_core::Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push(json!([path, params.map(Value::Object)]));
        Seq::next(&self.int, path)
    }
    fn v2_get(&self, path: &str, _: Option<Map<String, Value>>) -> fsv_core::Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push(json!([format!("v2:{path}"), null]));
        Seq::next(&self.v2, path)
    }
    read_only_api!();
}

fn status(code: i64) -> Result<Value, FetchError> {
    Err(FetchError::Status(Some(code)))
}

fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap()
}

/// Key-order-insensitive: `fsv_api::site::collect_pages` writes `partial`
/// before `continuation`, Python the reverse.
fn assert_same_keys(got: &Value, python: &str) {
    assert_eq!(
        *got,
        serde_json::from_str::<Value>(python).unwrap(),
        "{}",
        compact(got)
    );
}

fn tz7() -> TimeZone {
    TimeZone::fixed(tz::offset(7))
}

fn rel(s: &str) -> String {
    s.replace(
        "<REL>",
        &site::relative(&json!("2026-01-02T03:04:00Z"), None),
    )
}

fn note(m: &Option<Missing>) -> Value {
    m.as_ref().map_or(Value::Null, |m| json!(m.note()))
}

// ------------------------------------------------------------------ source.rs

#[test]
fn session_error_displays_message() {
    assert_eq!(SessionError("expired".into()).to_string(), "expired");
}

#[test]
fn collect_pages_dedups_across_pages() {
    let src = Seq::default().int(
        "changes/1/notes",
        vec![
            Ok(json!({"notes": [{"id": 1}, {"id": 2}], "meta": {"has_more": true}})),
            Ok(json!({"notes": [{"id": 2}, {"id": 3}], "meta": {"has_more": false}})),
        ],
    );
    let got = collect_pages(
        &src,
        "changes/1/notes",
        "notes",
        &obj(json!({"page": 1, "per_page": 2})),
        "",
        50,
    )
    .unwrap();
    assert_eq!(compact(&json!(got.rows)), r#"[{"id":1},{"id":2},{"id":3}]"#);
    assert_eq!(
        compact(&Value::Object(got.meta)),
        r#"{"has_more":false,"loaded_count":3,"pages":2,"complete":true,"has_next":false,"continuation":null}"#
    );
    assert!(got.failure.is_none());
    assert_eq!(
        compact(&json!(src.calls())),
        r#"[["changes/1/notes",{"page":1,"per_page":2}],["changes/1/notes",{"page":2,"per_page":2}]]"#
    );
}

#[test]
fn collect_pages_keeps_rows_on_later_failure() {
    let src = Seq::default().int(
        "changes/1/notes",
        vec![
            Ok(json!({"notes": [{"id": 1}], "meta": {"has_more": true}})),
            status(403),
        ],
    );
    let got = collect_pages(
        &src,
        "changes/1/notes",
        "notes",
        &obj(json!({"page": 1})),
        "Notes",
        50,
    )
    .unwrap();
    assert_eq!(compact(&json!(got.rows)), r#"[{"id":1}]"#);
    assert_same_keys(
        &Value::Object(got.meta),
        r#"{"has_more":true,"loaded_count":1,"pages":1,"complete":false,"has_next":true,"continuation":{"page":2},"partial":"Notes: incomplete (1 loaded; Notes: not permitted for your role (403))"}"#,
    );
    assert_eq!(
        note(&got.failure),
        "Notes: not permitted for your role (403)"
    );
}

#[test]
fn collect_pages_raises_session_errors() {
    let src = Seq::default().int("x", vec![Err(FetchError::Session("expired".into()))]);
    assert_eq!(
        collect_pages(&src, "x", "x", &Map::new(), "", 50)
            .unwrap_err()
            .0,
        "expired"
    );
}

#[test]
fn rows_of_keeps_mappings() {
    let got: Vec<Value> = [
        rows_of(&json!({"k": [{"a": 1}, 2, {"b": 2}]}), "k"),
        rows_of(&json!([{"a": 1}, "x"]), "k"),
        rows_of(&json!({"k": 3}), "k"),
        rows_of(&Value::Null, "k"),
    ]
    .into_iter()
    .map(Value::from)
    .collect();
    assert_eq!(
        compact(&json!(got)),
        r#"[[{"a":1},{"b":2}],[{"a":1}],[],[]]"#
    );
}

#[test]
fn name_of_first_truthy_stripped() {
    let got: Vec<String> = [
        json!({"name": " Ann "}),
        json!({"value": "V"}),
        json!({"email": "e@x"}),
        json!({}),
        json!({"name": ""}),
        json!("x"),
    ]
    .iter()
    .map(name_of)
    .collect();
    assert_eq!(got, ["Ann", "V", "e@x", "", "", ""]);
}

#[test]
fn obj_and_py_int_helpers() {
    assert_eq!(obj(json!({"a": 1})).get("a"), Some(&json!(1)));
    assert!(obj(json!([1])).is_empty());
    let got: Vec<Option<i64>> = [
        json!("12"),
        json!(" 7 "),
        json!(2.9),
        json!(true),
        json!("1_000"),
        json!("x"),
        Value::Null,
    ]
    .iter()
    .map(py_int)
    .collect();
    assert_eq!(
        got,
        [Some(12), Some(7), Some(2), Some(1), Some(1000), None, None]
    );
}

#[test]
fn association_reader_passes_v2_through() {
    let inner = Seq::default().v2("tickets/1/resolution_note", Ok(json!({"ok": 1})));
    let reader = AssociationReader(&inner);
    assert_eq!(
        v2_read(&reader, "tickets/1/resolution_note"),
        Ok(json!({"ok": 1}))
    );
    assert_eq!(
        int_read(&reader, "changes/1/tickets", None),
        Err(FetchError::Status(Some(404)))
    );
}

#[test]
fn load_conversations_latest_page() {
    let conv = json!({"conversations": [{"id": 2, "created_at": "2026-01-02T00:00:00Z"}, {"id": 1, "created_at": "2026-01-01T00:00:00Z"}],
                      "meta": {"count": 7}});
    let src = Seq::default().int("tickets/5/conversations", vec![Ok(conv)]);
    let got = load_conversations(&src, &json!(5), false, 3).unwrap();
    let ids: Vec<&Value> = got.rows.iter().map(|r| &r["id"]).collect();
    assert_eq!(compact(&json!(ids)), "[1,2]");
    assert_eq!(compact(&Value::Object(got.meta)), r#"{"count":7}"#);
    assert!(got.missing.is_none());
    assert_eq!(
        compact(&src.calls()[0][1]),
        r#"{"include":"user,phone,feedback","per_page":3}"#
    );

    let got = load_conversations(&Seq::default(), &json!(5), false, 3).unwrap();
    assert!(got.rows.is_empty() && got.meta.is_empty());
    assert_eq!(
        note(&got.missing),
        "conversations: not available on this account (404)"
    );
}

#[test]
fn load_conversations_all_pages_partial_and_complete() {
    let src = Seq::default().int(
        "tickets/5/conversations",
        vec![
            Ok(json!({"conversations": [{"id": 1}], "meta": {"has_more": true, "count": 4}})),
            status(500),
        ],
    );
    let got = load_conversations(&src, &json!(5), true, 3).unwrap();
    assert_eq!(got.rows.len(), 1);
    assert_eq!(got.meta["count"], json!(4));
    assert_eq!(
        note(&got.missing),
        "conversations after the first 1 (page 2): unavailable (500)"
    );
    assert_eq!(
        compact(&src.calls()[0][1]),
        r#"{"include":"user,phone,feedback","per_page":30,"page":1}"#
    );

    let src = Seq::default().int(
        "tickets/5/conversations",
        vec![Ok(
            json!({"conversations": [{"id": 1}, {"id": 2}], "meta": {"has_more": false}}),
        )],
    );
    let got = load_conversations(&src, &json!(5), true, 3).unwrap();
    assert_eq!(got.rows.len(), 2);
    assert_eq!(got.meta["count"], json!(2));
    assert!(got.missing.is_none());
}

#[test]
fn get_requested_items_merges_more_info() {
    let src = Seq::default()
        .int(
            "tickets/9/requested_items",
            vec![Ok(json!({"requested_items": [
                {"id": 1, "quantity": 1, "item": {"name": "Laptop", "a": 1}},
                {"id": 2, "item": {"name": "Phone"}},
                {"quantity": 3}
            ]}))],
        )
        .int(
            "tickets/9/requested_items/1",
            vec![Ok(
                json!({"requested_item": {"stage": "Done", "item": {"a": 2, "b": 3}}}),
            )],
        );
    let got = get_requested_items(&src, &json!(9)).unwrap();
    assert_eq!(
        compact(&json!(got)),
        r#"[{"id":1,"quantity":1,"item":{"name":"Laptop","a":2,"b":3},"stage":"Done"},{"id":2,"item":{"name":"Phone"}},{"quantity":3}]"#
    );
}

// ------------------------------------------------------------------ detail.rs

#[test]
fn tab_data_to_json_order() {
    let mut td = TabData::new("notes");
    td.data = Some(json!([]));
    td.missing = Some(Missing::new("Notes", Some(403)));
    td.parts = obj(json!({"A": []}));
    td.notes = vec!["n".into()];
    td.meta = obj(json!({"pages": 1}));
    td.parts_meta = obj(json!({"A": {"pages": 0}}));
    assert_eq!(
        compact(&td.to_json()),
        r#"{"name":"notes","label":"Notes","meta":{"pages":1},"parts_meta":{"A":{"pages":0}},"unavailable":true,"status":403,"note":"Notes: not permitted for your role (403)","data":[],"parts":{"A":[]},"notes":["n"]}"#
    );
    assert_eq!(
        compact(&TabData::new("tasks").to_json()),
        r#"{"name":"tasks","label":"Tasks"}"#
    );
}

fn tab_value(res: &'static Resource, rec: Value, name: &str, mut src: Seq) -> Value {
    let td = load_tab(&mut src, res, &rec, name).unwrap();
    json!({"tab": td.to_json(), "calls": src.calls()})
}

fn tab_case(res: &'static Resource, rec: Value, name: &str, src: Seq) -> String {
    compact(&tab_value(res, rec, name, src))
}

#[test]
fn load_tab_assets_and_impacted_services() {
    let src = Seq::default().int(
        "tickets/3/associated_assets",
        vec![Ok(json!({"associated_assets": [{"id": 1, "name": "srv"}]}))],
    );
    assert_eq!(
        tab_case(&TICKETS, json!({"id": 3}), "assets", src),
        r#"{"tab":{"name":"assets","label":"Assets","meta":{"loaded_count":1,"pages":1,"complete":true,"has_more":false,"has_next":false,"continuation":null},"data":[{"id":1,"name":"srv"}]},"calls":[["tickets/3/associated_assets",null]]}"#
    );
    let src = Seq::default().int(
        "changes/4/assets",
        vec![Ok(json!({"assets": [{"id": 2}], "associated_assets": []}))],
    );
    assert_eq!(
        tab_case(&CHANGES, json!({"display_id": 4}), "assets", src),
        r#"{"tab":{"name":"assets","label":"Assets","data":[{"id":2}]},"calls":[["changes/4/assets",null]]}"#
    );
    let src = Seq::default().int(
        "changes/4/assets",
        vec![Ok(json!({"associated_impacted_services": [{"id": 5}]}))],
    );
    assert_eq!(
        tab_case(&CHANGES, json!({"display_id": 4}), "impacted_services", src),
        r#"{"tab":{"name":"impacted_services","label":"Impacted services","data":[{"id":5}]},"calls":[["changes/4/assets",{"filter":"impacted_services"}]]}"#
    );
}

#[test]
fn load_tab_ticket_associations_partial_failure() {
    let src = Seq::default()
        .int(
            "tickets/3/changes",
            vec![Ok(json!({"changes": [{"id": 1}]}))],
        )
        .int("tickets/3/problems", vec![status(403)]);
    assert_same_keys(
        &tab_value(&TICKETS, json!({"id": 3}), "associations", src),
        r#"{"tab":{"name":"associations","label":"Associations","parts_meta":{"Changes":{"loaded_count":1,"pages":1,"complete":true,"has_more":false,"has_next":false,"continuation":null},"Changes that caused this":{"loaded_count":1,"pages":1,"complete":true,"has_more":false,"has_next":false,"continuation":null},"Problems":{"loaded_count":0,"pages":0,"complete":false,"has_more":true,"has_next":true,"continuation":{},"partial":"Problems: incomplete (0 loaded; Problems: not permitted for your role (403))"}},"parts":{"Changes":[{"id":1}],"Changes that caused this":[{"id":1}]},"notes":["Problems: incomplete (0 loaded; Problems: not permitted for your role (403))","Problems: not permitted for your role (403)"]},"calls":[["tickets/3/changes",{"change_type":"change"}],["tickets/3/changes",{"change_type":"change_cause"}],["tickets/3/problems",null]]}"#,
    );
}

#[test]
fn load_tab_devops_all_missing() {
    assert_same_keys(
        &tab_value(&CHANGES, json!({"display_id": 4}), "devops", Seq::default()),
        r#"{"tab":{"name":"devops","label":"Devops","parts_meta":{"Work items":{"loaded_count":0,"pages":0,"complete":false,"has_more":true,"has_next":true,"continuation":{"page":1,"per_page":5},"partial":"Work items: incomplete (0 loaded; Work items: not available on this account (404))"},"Pull requests":{"loaded_count":0,"pages":0,"complete":false,"has_more":true,"has_next":true,"continuation":{"page":1,"per_page":5},"partial":"Pull requests: incomplete (0 loaded; Pull requests: not available on this account (404))"},"Pipeline runs":{"loaded_count":0,"pages":0,"complete":false,"has_more":true,"has_next":true,"continuation":{"page":1,"per_page":5},"partial":"Pipeline runs: incomplete (0 loaded; Pipeline runs: not available on this account (404))"}},"unavailable":true,"status":404,"note":"Devops: not available on this account (404)","notes":["Work items: incomplete (0 loaded; Work items: not available on this account (404))","Pull requests: incomplete (0 loaded; Pull requests: not available on this account (404))","Pipeline runs: incomplete (0 loaded; Pipeline runs: not available on this account (404))"]},"calls":[["changes/4/work-items",{"page":1,"per_page":5}],["changes/4/pull-requests",{"page":1,"per_page":5}],["changes/4/pipeline-runs",{"page":1,"per_page":5}]]}"#,
    );
}

#[test]
fn load_tab_risk_related_approvals() {
    let src = Seq::default()
        .int(
            "changes/4/risk-analysis",
            vec![Ok(json!({"overall_score": 3}))],
        )
        .int("changes/4/survey", vec![Ok(json!({"q": "yes"}))]);
    assert_eq!(
        tab_case(
            &CHANGES,
            json!({"display_id": 4, "workspace_id": 2}),
            "risk",
            src
        ),
        r#"{"tab":{"name":"risk","label":"Risk","data":{"overall_score":3},"parts":{"survey":{"q":"yes"}}},"calls":[["changes/4/risk-analysis",null],["changes/4/survey",{"module":"risk","workspace_id":2}]]}"#
    );
    let src = Seq::default().int(
        "tickets/3/mappings",
        vec![Ok(
            json!({"predecessor_ticket": {"id": 1}, "successor_tickets": [{"id": 2}, "x"]}),
        )],
    );
    assert_eq!(
        tab_case(&TICKETS, json!({"id": 3}), "related_tickets", src),
        r#"{"tab":{"name":"related_tickets","label":"Related tickets","parts":{"Parent":[{"id":1}],"Child tickets":[{"id":2}]}},"calls":[["tickets/3/mappings",{"successor_type":"active"}]]}"#
    );
    let src = Seq::default().int(
        "changes/4/approvals",
        vec![Ok(json!({"approvals": [{"id": 1}]}))],
    );
    assert_eq!(
        tab_case(&CHANGES, json!({"display_id": 4}), "approvals", src),
        r#"{"tab":{"name":"approvals","label":"Approvals","meta":{"loaded_count":1,"pages":1,"complete":true,"has_more":false,"has_next":false,"continuation":null},"data":[{"id":1}]},"calls":[["changes/4/approvals",null]]}"#
    );
}

#[test]
fn load_tab_resolution_record_denied_null() {
    assert_eq!(
        tab_case(
            &TICKETS,
            json!({"id": 3, "resolution_notes": "<p>Done</p>"}),
            "resolution_notes",
            Seq::default()
        ),
        r#"{"tab":{"name":"resolution_notes","label":"Resolution","data":{"note":"Done"}},"calls":[]}"#
    );
    let src = Seq::default().v2("tickets/3/resolution_note", status(403));
    assert_eq!(
        tab_case(&TICKETS, json!({"id": 3}), "resolution_notes", src),
        r#"{"tab":{"name":"resolution_notes","label":"Resolution","unavailable":true,"status":403,"note":"Resolution: not permitted for your role (403)","data":{}},"calls":[["v2:tickets/3/resolution_note",null]]}"#
    );
    let src = Seq::default().v2(
        "tickets/3/resolution_note",
        Ok(json!({"resolution_note": null})),
    );
    assert_eq!(
        tab_case(&TICKETS, json!({"id": 3}), "resolution_notes", src),
        r#"{"tab":{"name":"resolution_notes","label":"Resolution","meta":{"absent":true},"data":{}},"calls":[["v2:tickets/3/resolution_note",null]]}"#
    );
}

fn ticket_detail() -> Detail {
    let record = json!({"id": 12, "subject": "Disk", "status": 2, "priority": 3, "source": 2,
        "due_by": "2026-03-01T10:00:00Z", "stats": {"resolved_at": "x", "resolution_time_in_secs": 3700},
        "requester": {"name": "Ann", "job_title": "Eng"}, "created_at": "2026-01-02T03:04:00Z",
        "description": "", "description_text": " plain ",
        "attachments": [{"name": "a", "size": 1}, {"content_file_name": "b", "content_file_size": 2}, "x"],
        "cloud_files": [{"id": 9}]});
    let schema = json!({"fields": [{"name": "status", "choices": [{"id": 2, "value": "Open"}]},
                                   {"name": "source", "choices": [{"id": 2, "value": "Portal"}]}]});
    let tabs = vec![
        Tab {
            name: "details".into(),
            label: "Details".into(),
            count: None,
        },
        Tab {
            name: "notes".into(),
            label: "Notes".into(),
            count: Some(2),
        },
    ];
    let mut d = Detail::new(&TICKETS, record, schema, json!({"id": 77}), tz7(), tabs);
    d.requested_for = Some("Bob".into());
    d.properties = vec![Prop {
        name: "p".into(),
        label: "P".into(),
        value: "v".into(),
        required: true,
    }];
    d.properties_note = Some("note".into());
    d.conversations = Some(vec![
        json!({"id": 1, "body": "hi", "user": {"name": "Ann"}}),
    ]);
    d.conversations_meta = obj(json!({"count": 1}));
    d.requested_items = Some(vec![
        json!({"item": {"name": "Laptop"}, "custom_fields": {}}),
    ]);
    d.lifecycle = Some(Lifecycle {
        name: "F".into(),
        states: vec![LifecycleState {
            id: json!(1),
            name: "Open".into(),
            mark: "current".into(),
        }],
    });
    d.planning = Some(vec![PlanningEntry {
        name: "p".into(),
        label: "Reason".into(),
        help: String::new(),
        text: "t".into(),
        files: vec![],
    }]);
    let mut notes = TabData::new("notes");
    notes.data = Some(json!([]));
    d.set_tab(notes);
    d
}

#[test]
fn detail_accessors_ticket() {
    let d = ticket_detail();
    assert_eq!(d.id(), "INC-12");
    assert_eq!(d.subject(), "Disk");
    assert_eq!(d.badge(), "Open");
    assert_eq!(d.me_id(), Some(&json!(77)));
    assert_eq!(d.due_text(), "Sun, 1 Mar 2026 17:00");
    assert_eq!(d.tab_names(), ["details", "notes"]);
    assert_eq!(
        compact(&Value::Object(d.reporter())),
        rel(
            r#"{"name":"Ann","created_at":"2026-01-02T03:04:00Z","relative":"<REL>","when":"Fri, 2 Jan 10:04","via":"Portal","request_for":"Bob"}"#
        )
    );
    assert_eq!(d.description(), "plain");
    assert_eq!(
        compact(&json!(d.attachments())),
        r#"[{"name":"a","size":1},{"name":"b","size":2},{"name":"9","size":null}]"#
    );
    assert_eq!(d.notes(), ["note"]);
    let summary = d.summary();
    let summary: Vec<(&str, &str)> = summary
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            ("Status", "Open"),
            ("Priority", "High"),
            ("Due by", "Sun, 1 Mar 2026 17:00"),
            ("Time taken", "1h 2m"),
            ("Requester title", "Eng")
        ]
    );
}

#[test]
fn detail_tab_and_set_tab() {
    let mut d = ticket_detail();
    assert_eq!(d.tab("notes").unwrap().data, Some(json!([])));
    assert!(d.tab("tasks").is_none());
    let mut replaced = TabData::new("notes");
    replaced.data = Some(json!([{"id": 1}]));
    d.set_tab(replaced);
    assert_eq!(d.tab_data.len(), 1);
    assert_eq!(d.tab("notes").unwrap().data, Some(json!([{"id": 1}])));
    d.set_tab(TabData::new("tasks"));
    assert_eq!(
        d.tab_data
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["notes", "tasks"]
    );
}

#[test]
fn detail_to_json_ticket() {
    assert_eq!(
        compact(&ticket_detail().to_json()),
        rel(
            r#"{"id":"INC-12","subject":"Disk","status":"Open","due_by":"2026-03-01T10:00:00Z","reporter":{"name":"Ann","created_at":"2026-01-02T03:04:00Z","relative":"<REL>","when":"Fri, 2 Jan 10:04","via":"Portal","request_for":"Bob"},"tabs":[{"name":"details","label":"Details","count":null},{"name":"notes","label":"Notes","count":2}],"lifecycle":{"name":"F","states":[{"id":1,"name":"Open","mark":"current"}]},"details":{"description":"plain","attachments":[{"name":"a","size":1},{"name":"b","size":2},{"name":"9","size":null}],"planning":[{"name":"p","label":"Reason","help":"","content":"t","files":[],"empty":false}],"requested_items":[{"name":"Laptop","stage":"","quantity":null,"answers":[]}],"conversations":[{"id":1,"author":"Ann","kind":"replied","private":false,"created_at":null,"relative":"","body":"hi","attachments":[]}],"conversations_total":1},"summary":[{"label":"Status","value":"Open"},{"label":"Priority","value":"High"},{"label":"Due by","value":"Sun, 1 Mar 2026 17:00"},{"label":"Time taken","value":"1h 2m"},{"label":"Requester title","value":"Eng"}],"properties":[{"name":"p","label":"P","value":"v","required":true}],"notes":["note"],"tab_data":{"notes":{"name":"notes","label":"Notes","data":[]}}}"#
        )
    );
}

#[test]
fn detail_change_missing_sections() {
    let mut d = Detail::new(
        &CHANGES,
        json!({"display_id": 4}),
        json!({"fields": []}),
        json!({}),
        tz7(),
        vec![Tab {
            name: "details".into(),
            label: "Details".into(),
            count: None,
        }],
    );
    d.tabs_missing = Some(Missing::new("tabs", Some(404)));
    d.lifecycle_missing = Some(Missing::new("lifecycle", Some(403)));
    d.planning_missing = Some(Missing::new("planning", Some(500)));
    d.requested_items_missing = Some(Missing::new("requested items", None));
    d.conversations_missing = Some(Missing::new("conversations", Some(404)));
    assert_eq!(
        (
            d.id().as_str(),
            d.subject().as_str(),
            d.badge().as_str(),
            d.me_id(),
            d.due_text().as_str()
        ),
        ("CHN-4", "", "", None, "")
    );
    assert_eq!(
        d.notes(),
        [
            "tabs: not available on this account (404)",
            "lifecycle: not permitted for your role (403)",
            "planning: unavailable (500)",
            "requested items: unavailable",
            "conversations: not available on this account (404)",
        ]
    );
    assert_eq!(d.summary(), [("Status".to_string(), String::new())]);
    assert_eq!(
        compact(&d.to_json()),
        r#"{"id":"CHN-4","subject":"","status":"","due_by":null,"reporter":{"name":"","created_at":null,"relative":"","when":""},"tabs":[{"name":"details","label":"Details","count":null}],"lifecycle":null,"details":{"description":"","attachments":[]},"summary":[{"label":"Status","value":""}],"properties":[],"notes":["tabs: not available on this account (404)","lifecycle: not permitted for your role (403)","planning: unavailable (500)","requested items: unavailable","conversations: not available on this account (404)"]}"#
    );
}

fn lc_json(lc: &Option<Lifecycle>) -> Value {
    lc.as_ref().map_or(Value::Null, Lifecycle::to_json)
}

#[test]
fn load_lifecycle_workspace_denied_none() {
    let flow = json!({"state_flow": {"name": "Std", "state_list": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Plan"}]}});
    let src = Seq::default().int("ws/3/change_management/8", vec![Ok(flow)]);
    let (lc, miss) = load_lifecycle(
        &src,
        &json!({"state_flow_id": 8, "workspace_id": 3, "status": 2, "state_traversal": [1, 2]}),
    )
    .unwrap();
    assert_eq!(
        compact(&lc_json(&lc)),
        r#"{"name":"Std","states":[{"id":1,"name":"Open","mark":"done"},{"id":2,"name":"Plan","mark":"current"}]}"#
    );
    assert!(miss.is_none());
    assert_eq!(
        compact(&json!(src.calls())),
        r#"[["ws/3/change_management/8",{"include":"state_info"}]]"#
    );

    let src = Seq::default().int("change_management/8", vec![status(403)]);
    let (lc, miss) = load_lifecycle(&src, &json!({"state_flow_id": 8, "status": 2})).unwrap();
    assert!(lc.is_none());
    assert_eq!(note(&miss), "lifecycle: not permitted for your role (403)");

    let (lc, miss) = load_lifecycle(&Seq::default(), &json!({})).unwrap();
    assert!(lc.is_none() && miss.is_none());
}

#[test]
fn load_planning_rows_missing_none_unidentified() {
    let schema = json!({"fields": [{"name": "planning_reason", "label": "Reason &amp; why", "field_type": "planning_field", "helptext": "<b>Why</b> now"}]});
    let src = Seq::default().int(
        "changes/4/planning-fields",
        vec![Ok(json!({"change_planning_fields": [{"name": "planning_reason", "description": "<p>Because</p>", "attachments": [{"name": "p.pdf"}]}]}))],
    );
    let (rows, miss) = load_planning(&src, &json!({"display_id": 4}), &schema, "changes").unwrap();
    let rows: Vec<Value> = rows.unwrap().iter().map(PlanningEntry::to_json).collect();
    assert_eq!(
        compact(&json!(rows)),
        r#"[{"name":"planning_reason","label":"Reason & why","help":"Why now","content":"Because","files":["p.pdf"],"empty":false}]"#
    );
    assert!(miss.is_none());

    let (rows, miss) = load_planning(
        &Seq::default(),
        &json!({"display_id": 4}),
        &schema,
        "changes",
    )
    .unwrap();
    assert!(rows.is_none());
    assert_eq!(note(&miss), "planning: not available on this account (404)");

    let (rows, miss) = load_planning(
        &Seq::default(),
        &json!({"display_id": 4}),
        &json!({"fields": []}),
        "changes",
    )
    .unwrap();
    assert!(rows.is_none() && miss.is_none());

    let src = Seq::default().int(
        "changes/4/planning-fields",
        vec![Ok(json!([{"description": "orphan"}]))],
    );
    let (rows, miss) = load_planning(&src, &json!({"display_id": 4}), &schema, "changes").unwrap();
    assert!(rows.is_none());
    assert_eq!(note(&miss), "unidentified planning rows: unavailable");
}

/// `test_problem_details.py`: the native analysis sections, in either
/// stored shape, with `files`; values from `uv run python` on `load_planning`.
#[test]
fn load_planning_problem_analysis_sections() {
    for shape in ["content", "description"] {
        let src = Seq::default().int(
            "problems/123/planning-fields",
            vec![Ok(json!({"problem_planning_fields": [
                {"name": "problem_cause", shape: "<div>Acme cause &amp; evidence</div>", "files": []},
                {"name": "problem_impact", shape: "Acme impact", "files": [{"name": "evidence.txt"}]},
                {"name": "problem_symptom", shape: "Acme symptoms", "files": []}
            ]}))],
        );
        let (rows, miss) =
            load_planning(&src, &json!({"id": 123}), &json!({}), "problems").unwrap();
        let got: Vec<Value> = rows
            .unwrap()
            .iter()
            .map(|r| json!({"name": r.name, "label": r.label, "help": r.help, "text": r.text, "files": r.files}))
            .collect();
        assert_eq!(
            json!(got),
            json!([
                {"name": "problem_cause", "label": "Root Cause", "help": "", "text": "Acme cause & evidence", "files": []},
                {"name": "problem_impact", "label": "Impact", "help": "", "text": "Acme impact", "files": ["evidence.txt"]},
                {"name": "problem_symptom", "label": "Symptoms", "help": "", "text": "Acme symptoms", "files": []}
            ]),
            "{shape}"
        );
        assert!(miss.is_none());
    }
    for payload in [
        json!({}),
        json!({"problem_planning_fields": null}),
        json!({"problem_planning_fields": [42]}),
        json!({"change_planning_fields": [{"name": "x"}]}),
    ] {
        let src = Seq::default().int("problems/123/planning-fields", vec![Ok(payload.clone())]);
        let (rows, miss) =
            load_planning(&src, &json!({"id": 123}), &json!({}), "problems").unwrap();
        assert!(rows.is_none(), "{payload}");
        assert_eq!(
            note(&miss),
            "planning rows unavailable: unavailable",
            "{payload}"
        );
    }
}

#[test]
fn load_detail_ticket_tabs_items_conversations() {
    let mut src = Seq::default()
        .int(
            "tickets/12/tabs",
            vec![Ok(
                json!({"tabs": [{"name": "requested_items"}, {"name": "notes", "count": 2}]}),
            )],
        )
        .int("users/5", vec![Ok(json!({"user": {"name": "Rae"}}))])
        .int(
            "tickets/12/requested_items",
            vec![Ok(
                json!({"requested_items": [{"item": {"name": "Laptop"}}]}),
            )],
        )
        .int(
            "tickets/12/conversations",
            vec![Ok(
                json!({"conversations": [{"id": 1, "body": "hi"}], "meta": {"count": 4}}),
            )],
        )
        .int(
            "tickets/12/notes",
            vec![Ok(json!({"notes": [], "meta": {"has_more": false}}))],
        );
    let tabs = ["details".to_string(), "notes".to_string()];
    let input = DetailInput {
        record: json!({"id": 12, "requested_for_id": 5, "requester_id": 2}),
        schema: Some(json!({"fields": []})),
        user: json!({}),
        tz: tz7(),
        item_id: json!(12),
        all_conversations: false,
        tabs: &tabs,
    };
    let d = load_detail(&mut src, &TICKETS, input, None).unwrap();
    let got = json!({
        "tabs": d.tabs.iter().map(Tab::to_json).collect::<Vec<_>>(),
        "requested_for": d.requested_for,
        "requested_items": d.requested_items,
        "conversations": d.conversations,
        "conversations_meta": d.conversations_meta,
        "notes": d.notes(),
        "tab_data": d.tab_data.iter().map(|t| (t.name.clone(), t.to_json())).collect::<Map<_, _>>(),
    });
    assert_eq!(
        compact(&got),
        r#"{"tabs":[{"name":"details","label":"Details","count":null},{"name":"notes","label":"Notes","count":2}],"requested_for":"Rae","requested_items":[{"item":{"name":"Laptop"}}],"conversations":[{"id":1,"body":"hi"}],"conversations_meta":{"count":4},"notes":["no form fields"],"tab_data":{"notes":{"name":"notes","label":"Notes","meta":{"has_more":false,"loaded_count":0,"pages":1,"complete":true,"has_next":false,"continuation":null},"data":[]}}}"#
    );
}
