//! Expected requests and results are from the Python TUI's calls
//! (`service.*`, `views.*`, `ticket_notes`, `change_approvals`,
//! `change_approval_actions`, `query.build_query_hash_from_schema`) run
//! over a recording client with the same fixtures.

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::testkit::{Call, MemJournal, Tenant, ok, pairs, status, tenant};
use fsv_core::client::Response;
use fsv_core::resource::{CHANGES, TICKETS};

fn backend(t: &Tenant) -> LiveBackend {
    let schema: SchemaLoader = Arc::new(|r: Res| Ok(json!({"fields": [], "for": r.name})));
    LiveBackend::new(t.client.clone(), schema)
        .with_journal(Arc::new(MemJournal::default()))
        .with_store(Arc::new(fsv_api::formmeta::MemStore::default()))
        .with_config(Map::new())
}

fn route(c: &Call) -> Option<Response> {
    let p = c.path.strip_prefix("/api/_/").unwrap_or(&c.path);
    match (c.method.as_str(), p) {
        ("GET", "changes") => ok(
            json!({"changes": [{"id": 5, "display_id": 5, "subject": "A"}], "meta": {"total_count": 1}}),
        ),
        ("GET", "changes/5") => ok(
            json!({"change": {"id": 5, "display_id": 5, "subject": "A", "description": "<p>d</p>"}}),
        ),
        ("GET", "changes/5/tabs") => {
            ok(json!({"tabs": [{"name": "details"}, {"name": "notes", "count": 2}]}))
        }
        ("GET", "changes/5/notes") => ok(json!({"notes": [{"id": 1, "body": "n"}]})),
        ("GET", "change-filters") => {
            ok(json!({"change_filters": [{"id": 3, "name": "Mine", "default": true}]}))
        }
        ("GET", "change-filters/3") => {
            ok(json!({"change_filter": {"order_by": "updated_at", "order_type": "asc"}}))
        }
        ("GET", "tickets/headers") => ok(json!({"headers": [{"name": "subject"}]})),
        ("GET", "/api/v2/agents/me") => ok(
            json!({"agent": {"id": 1, "first_name": "Ann", "last_name": "Lee", "time_zone": "Tokyo"}}),
        ),
        ("GET", "/search/changes") => ok(
            json!({"results": [{"itil_module_path": "/a/changes/5", "itil_module_display_id": "CHN-5", "subject": "<b>A</b>"}], "total_entries": 1}),
        ),
        ("GET", "bootstrap/agents_groups") => ok(json!({"groups": []})),
        ("GET", "/search/autocomplete/agents") => ok(json!({"results": [{"id": 3}]})),
        ("PUT", "changes/5") => ok(json!({"change": {"id": 5}})),
        ("POST", "changes/5/notes") | ("POST", "tickets/12/notes") => {
            ok(json!({"note": {"id": 1}}))
        }
        ("POST", "tickets/12/reply") => ok(json!({"conversation": {"id": 2}})),
        _ => None,
    }
}

fn lines(t: &Tenant) -> Vec<String> {
    t.lines()
}

#[test]
fn list_page_sends_the_web_list_query() {
    let t = tenant(route);
    let req = ListRequest {
        res: &CHANGES,
        page: 2,
        per_page: 30,
        query_hash: Some(String::new()),
        filter_name: Some("3".into()),
        order_by: "updated_at".into(),
        order_type: "asc".into(),
    };
    let (items, data) = backend(&t).list_page(&req).unwrap();
    assert_eq!(
        items,
        [json!({"id": 5, "display_id": 5, "subject": "A", "_resource": "changes"})]
    );
    assert_eq!(data["meta"], json!({"total_count": 1}));
    let call = &t.calls()[0];
    assert_eq!(call.path, "/api/_/changes");
    assert_eq!(
        call.query,
        pairs(&[
            ("per_page", "30"),
            ("page", "2"),
            ("include", "change_status,agent,requester,group,department"),
            ("cache", "true"),
            ("filter", "3"),
            ("order_by", "updated_at"),
            ("order_type", "asc")
        ])
    );
}

#[test]
fn saved_views_and_their_settings() {
    let t = tenant(route);
    let b = backend(&t);
    let views = b.load_views(&CHANGES).unwrap();
    assert_eq!(
        views
            .iter()
            .map(|v| (v.id.as_str(), v.name.as_str(), v.system, v.marked_default))
            .collect::<Vec<_>>(),
        [("3", "Mine", true, true)]
    );
    assert_eq!(
        b.view_settings(&CHANGES, &views[0]).unwrap(),
        json!({"order_by": "updated_at", "order_type": "asc"})
    );
    assert_eq!(
        lines(&t),
        ["GET /api/_/change-filters", "GET /api/_/change-filters/3"]
    );
    let none = View {
        id: "9".into(),
        name: "x".into(),
        system: false,
        marked_default: false,
        raw: json!({}),
    };
    assert_eq!(b.view_settings(&CHANGES, &none).unwrap(), json!({}));
}

#[test]
fn ticket_headers_are_the_header_rows() {
    let t = tenant(route);
    assert_eq!(
        backend(&t).ticket_headers().unwrap(),
        [json!({"name": "subject"})]
    );
    assert_eq!(lines(&t), ["GET /api/_/tickets/headers"]);
}

#[test]
fn current_user_is_read_once_and_named() {
    let t = tenant(route);
    let b = backend(&t);
    let me = b.current_user().unwrap();
    assert_eq!(
        me,
        json!({"id": 1, "first_name": "Ann", "last_name": "Lee", "time_zone": "Tokyo", "name": "Ann Lee"})
    );
    b.current_user().unwrap();
    assert_eq!(lines(&t), ["GET /api/v2/agents/me"]);
    if std::env::var_os("FSV_TZ").is_none() {
        assert_eq!(b.user_tz(&me).unwrap().iana_name(), Some("Asia/Tokyo"));
    }
    let denied = tenant(|c| {
        (c.path == "/api/v2/agents/me")
            .then(|| status(403, json!({})))
            .flatten()
    });
    assert_eq!(backend(&denied).current_user().unwrap(), json!({}));
}

#[test]
fn detail_pages_load_like_views_load_detail_and_render_through_pages() {
    let t = tenant(route);
    let b = backend(&t);
    let page = b
        .load_detail(&CHANGES, &json!(5), Some(&json!({"fields": []})))
        .unwrap();
    let calls = t.calls();
    assert_eq!(
        calls.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        [
            "/api/_/changes/5",
            "/api/v2/agents/me",
            "/api/_/changes/5/tabs"
        ]
    );
    assert_eq!(calls[0].query, pairs(&[("include", "requester,stats")]));
    let tabs: Vec<(&str, &str, Option<i64>)> = page
        .tabs
        .iter()
        .map(|t| (t.name.as_str(), t.label.as_str(), t.count))
        .collect();
    assert_eq!(
        tabs,
        [("details", "Details", None), ("notes", "Notes", Some(2))]
    );
    assert_eq!(
        (page.badge.as_str(), page.extra["notes"].clone()),
        ("", json!(["no form fields"]))
    );
    let pages = b.pages();
    let d = pages.details.get(&page).unwrap();
    assert_eq!(pages.main_lines(&page), fsv_views::pages::main_lines(&d));
    assert_eq!(pages.side_lines(&page), fsv_views::pages::side_lines(&d));
    assert_eq!(pages.notes_lines(&page), fsv_views::pages::notes_lines(&d));
    assert_eq!(
        pages.reporter_markup(&page),
        fsv_views::pages::reporter_markup(&d)
    );
    assert_eq!(
        pages.lifecycle_markup(&page, 3),
        fsv_views::pages::lifecycle_markup(None, Some(3))
    );
    assert_eq!(
        pages.tab_bar(&page, "notes", true),
        fsv_views::pages::tab_bar(&d, "notes", true)
    );
    let foreign = DetailPage {
        extra: json!({}),
        ..page
    };
    assert_eq!(pages.main_lines(&foreign), BasicPages.main_lines(&foreign));
}

#[test]
fn detail_without_a_schema_uses_the_loader_and_tickets_ask_for_the_responder() {
    let t = tenant(|c| match c.path.as_str() {
        "/api/_/tickets/12" => ok(json!({"ticket": {"id": 12, "display_id": 12}})),
        _ => None,
    });
    let b = backend(&t);
    let page = b.load_detail(&TICKETS, &json!(12), None).unwrap();
    assert_eq!(page.schema.as_ref().unwrap()["for"], json!("tickets"));
    assert_eq!(
        t.calls()[0].query,
        pairs(&[("include", TICKET_DETAIL_INCLUDES)])
    );
}

#[test]
fn tabs_load_and_round_trip_to_the_renderer() {
    let t = tenant(route);
    let b = backend(&t);
    let tab = b
        .load_tab(&CHANGES, &json!({"id": 5, "display_id": 5}), "notes")
        .unwrap();
    assert_eq!(
        (tab.label.as_str(), tab.data.clone()),
        ("Notes", json!([{"id": 1, "body": "n"}]))
    );
    let call = &t.calls()[0];
    assert_eq!(
        (call.path.as_str(), call.query.clone()),
        (
            "/api/_/changes/5/notes",
            pairs(&[("include", "user"), ("page", "1"), ("per_page", "30")])
        )
    );
    let mut src = ClientSource::new(t.client.clone());
    let direct = fsv_views::load_tab(
        &mut src,
        &CHANGES,
        &json!({"id": 5, "display_id": 5}),
        "notes",
    )
    .unwrap();
    assert_eq!(views_tab(&tab), direct);
    assert_eq!(
        b.pages().tab_lines(&tab, None, None),
        fsv_views::pages::tab_lines(&direct, None, None)
    );
    let missing = tui_tab(fsv_views::TabData {
        missing: Some(site::Missing::new("Notes", Some(404))),
        ..fsv_views::TabData::new("notes")
    });
    assert_eq!(
        (missing.data.clone(), views_tab(&missing).data),
        (Value::Null, None)
    );
}

#[test]
fn search_reads_the_fulltext_endpoint() {
    let t = tenant(route);
    let (items, totals) = backend(&t)
        .search_items("x", Some("changes"), "modified", 1)
        .unwrap();
    assert_eq!(
        t.calls()[0].line(),
        "GET /search/changes?term=x&page=1&search_sort=updated_at"
    );
    assert_eq!(items.len(), 1);
    assert_eq!(
        (items[0].res.name, items[0].id(), items[0].str("subject")),
        ("changes", json!(5), "A".to_string())
    );
    assert_eq!(totals, [("changes".to_string(), 1)]);
}

#[test]
fn record_writes_use_the_service_calls() {
    let t = tenant(route);
    let b = backend(&t);
    assert_eq!(
        b.update_item(
            &CHANGES,
            &json!(5),
            &crate::testkit::obj(json!({"subject": "x"}))
        )
        .unwrap(),
        json!({"id": 5})
    );
    assert_eq!(
        b.add_note(&CHANGES, &json!(5), "hi", false).unwrap(),
        json!({"id": 1})
    );
    assert_eq!(
        b.add_note(&TICKETS, &json!(12), "hi", true).unwrap(),
        json!({"id": 1})
    );
    assert_eq!(
        b.add_reply(&TICKETS, &json!(12), "yo").unwrap(),
        json!({"id": 2})
    );
    let got: Vec<(String, Value)> = t
        .calls()
        .iter()
        .map(|c| (c.line(), c.json().unwrap()))
        .collect();
    assert_eq!(
        got,
        [
            ("PUT /api/_/changes/5".to_string(), json!({"subject": "x"})),
            (
                "POST /api/_/changes/5/notes".to_string(),
                json!({"body": "hi", "private": true})
            ),
            (
                "POST /api/_/tickets/12/notes".to_string(),
                json!({"body": "hi"})
            ),
            (
                "POST /api/_/tickets/12/reply".to_string(),
                json!({"body": "yo"})
            ),
        ]
    );
    assert!(b.update_item(&CHANGES, &json!("x"), &Map::new()).is_err());
}

const NOTE_HASH: &str = "295518245b2b1b390113d89c464e14322ed879299d9c1a1f141924a63a547b79";

#[test]
fn ticket_notes_review_then_send_once_and_read_back() {
    let t = tenant(|c| match (c.method.as_str(), c.path.as_str()) {
        ("POST", "/api/_/tickets/12/notes") => {
            ok(json!({"conversation": {"id": 70, "body": "<div>hi &amp; bye</div>"}}))
        }
        ("GET", "/api/_/tickets/12/conversations") => ok(
            json!({"conversations": [{"id": 70, "body": "<div>hi &amp; bye</div>", "private": true, "notify_emails": []}], "meta": {"count": 1}}),
        ),
        _ => None,
    });
    let b = backend(&t);
    let plan = b.ticket_note_plan(&json!(12), "hi & bye", false).unwrap();
    assert_eq!(
        (plan.hash.as_str(), plan.public, plan.target_id.clone()),
        (NOTE_HASH, false, json!(12))
    );
    assert_eq!(
        plan.review,
        json!({"action": "ticket_note", "tenant": "acme.freshservice.com", "ticket_id": 12, "visibility": "private note", "path": "/api/_/tickets/12/notes",
               "method": "POST", "payload": {"body": "<div>hi &amp; bye</div>", "conversation_from_summary": false, "private": true, "notify_emails": []},
               "body_text": "hi & bye", "plan_hash": NOTE_HASH, "risks": [],
               "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications."})
    );
    let out = b.send_ticket_note(&plan, false, false).unwrap();
    let calls = t.calls();
    assert_eq!(calls[0].line(), "POST /api/_/tickets/12/notes");
    assert_eq!(
        calls[0].body.as_deref(),
        Some(
            r#"{"body": "<div>hi &amp; bye</div>", "conversation_from_summary": false, "private": true, "notify_emails": []}"#
        )
    );
    assert_eq!(
        (calls[1].path.as_str(), calls[1].query.clone()),
        (
            "/api/_/tickets/12/conversations",
            pairs(&[
                ("include", "user,phone,feedback"),
                ("per_page", "30"),
                ("page", "1")
            ])
        )
    );
    assert_eq!(
        out,
        json!({"status": "submitted", "ticket_id": 12, "note_id": 70, "note": {"id": 70, "body": "<div>hi &amp; bye</div>"}, "plan_hash": NOTE_HASH,
               "verified": true, "readback": {"id": 70, "body": "<div>hi &amp; bye</div>", "private": true, "notify_emails": []},
               "identity_verified": true, "fields_verified": true, "readback_complete": true,
               "fields": {"body": "matched", "private": "matched", "notify_emails": "matched"}})
    );
    let tampered = ReviewPlan {
        hash: "0".repeat(64),
        ..plan
    };
    assert!(b.send_ticket_note(&tampered, false, false).is_err());
}

#[test]
fn approval_requests_are_planned_from_explicit_agent_ids() {
    let t = tenant(route);
    let plan = backend(&t)
        .approval_request_plan(
            &json!(5),
            "CAB",
            "please\nlook",
            &["3".into(), "4".into()],
            "anyone",
        )
        .unwrap();
    assert_eq!(
        plan.hash,
        "58615d09edd54cce03218e009e6cacb622bf75a92489d29a31358a4dff3868a5"
    );
    assert_eq!(
        plan.review,
        json!({"action": "change_approval_request", "tenant": "acme.freshservice.com", "change_id": 5, "method": "POST", "path": "/api/_/changes/5/approval-groups",
               "payload": {"approval_groups": [{"name": "CAB", "approval_type": 2, "email_content": "<p dir=\"ltr\">please<br>look</p>", "notified_to": [3, 4]}]},
               "recipients": [3, 4], "approval_rule": "Anyone", "message_text": "please\nlook", "plan_hash": "58615d09edd54cce03218e009e6cacb622bf75a92489d29a31358a4dff3868a5",
               "warning": "Approval emails are irreversible; cancel/revoke cannot recall them. This requests approval; it does not approve the change. Tenant workflows may also notify people.",
               "verification": "Complete before/after approval record IDs, exact new recipients, requested status and exact selected rule ID. When complete chains identify exactly one new matching group, verify its exact name and ID too.",
               "contract": "Per-member approvals and named approval-chain groups observed; POST response not trusted."})
    );
    assert!(t.calls().is_empty());
    assert_eq!(approval_plan(&plan.raw).hash(), plan.hash);
}

const ACT_HASH: &str = "17dd426733d6aadc540302d059b0de7734e2a86cdb92a4f57b45beb9b898d537";

fn approvals_server() -> Tenant {
    let decided = Arc::new(AtomicBool::new(false));
    tenant(move |c| match (c.method.as_str(), c.path.as_str()) {
        ("GET", "/api/_/changes/5/approvals") => {
            let st = if decided.load(Ordering::SeqCst) {
                json!({"id": 1, "name": "approved"})
            } else {
                json!({"id": 0, "name": "requested"})
            };
            ok(json!([{"id": 7, "member_id": 3, "status": st, "change_id": 5}]))
        }
        ("PUT", "/api/_/changes/5/approvals/7") => {
            decided.store(true, Ordering::SeqCst);
            ok(json!({}))
        }
        _ => None,
    })
}

#[test]
fn approval_actions_send_once_and_verify_the_decision() {
    let t = approvals_server();
    let b = backend(&t);
    let rows = b.approval_targets("approval-respond", &json!(5)).unwrap();
    assert_eq!(
        rows,
        [
            json!({"id": 7, "member_id": 3, "status": {"id": 0, "name": "requested"}, "change_id": 5})
        ]
    );
    let plan = b
        .approval_action_plan(&json!(5), &json!(7), "approve", Some("ok"))
        .unwrap();
    assert_eq!(plan.hash, ACT_HASH);
    let n = t.calls().len();
    let out = b.send_approval(&plan, false).unwrap();
    let calls = t.calls().split_off(n);
    assert_eq!(
        calls.iter().map(|c| c.line()).collect::<Vec<_>>(),
        [
            "GET /api/_/changes/5/approvals",
            "PUT /api/_/changes/5/approvals/7",
            "GET /api/_/changes/5/approvals"
        ]
    );
    assert_eq!(
        calls[1].body.as_deref(),
        Some(r#"{"approval_status": 1, "remark": "ok"}"#)
    );
    assert_eq!(
        out,
        json!({"status": "submitted", "change_id": 5, "approval_id": 7, "operation": "approve", "verified": true, "decision_verified": true,
               "verification_basis": "named_decision", "before_status": {"id": 0, "name": "requested"}, "observed_status": {"id": 1, "name": "approved"},
               "readback": {"id": 7, "member_id": 3, "status": {"id": 1, "name": "approved"}, "change_id": 5}, "plan_hash": ACT_HASH})
    );
    let other = ReviewPlan {
        raw: json!({"kind": "ticket_note"}),
        ..plan
    };
    assert!(b.send_approval(&other, false).unwrap_err().is_value());
}

#[test]
fn query_hashes_use_the_schema_and_directory() {
    let t = tenant(route);
    let schema = json!({"fields": [{"name": "priority", "label": "Priority", "type": "dropdown", "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "High"}]}]});
    let got = backend(&t)
        .build_query_hash(&CHANGES, &schema, &["priority=High".into()])
        .unwrap();
    assert_eq!(
        got,
        r#"[{"value":["High"],"condition":"priority","operator":"is_in","type":"custom_field"}]"#
    );
    assert!(t.calls().is_empty());
    assert!(
        backend(&t)
            .build_query_hash(&CHANGES, &schema, &["priority:High".into()])
            .unwrap_err()
            .is_value()
    );
}

#[test]
fn directory_lookups_use_autocomplete_and_bootstrap() {
    let t = tenant(route);
    let dir = ClientDirectory(t.client.clone());
    assert_eq!(
        dir.autocomplete("agents", "ann", &[]).unwrap(),
        [json!({"id": 3})]
    );
    assert_eq!(dir.agent_groups().unwrap(), json!({"groups": []}));
    assert_eq!(t.calls()[0].query, pairs(&[("q", "ann"), ("qf", "value")]));
    assert_eq!(t.calls()[1].line(), "GET /api/_/bootstrap/agents_groups");
}

#[test]
fn settings_urls_and_schemas() {
    let t = tenant(route);
    let b = backend(&t).with_config(crate::testkit::obj(json!({"tui": {"read_only": "yes"}})));
    assert_eq!(b.read_only_setting(), json!("yes"));
    assert_eq!(
        backend(&t)
            .with_config(crate::testkit::obj(json!({"tui": "x"})))
            .read_only_setting(),
        Value::Null
    );
    assert_eq!(
        b.item_url(&CHANGES, &json!(5)),
        "https://acme.freshservice.com/a/changes/5"
    );
    assert_eq!(b.load_schema(&CHANGES).unwrap()["for"], json!("changes"));
    assert_eq!(
        b.default_where("changes"),
        fsv_core::config::get_default_where("changes")
    );
}

#[test]
fn the_detail_cache_keeps_the_latest_pages() {
    let cache = DetailCache::default();
    let mk = || {
        Detail::new(
            &CHANGES,
            json!({"id": 1}),
            json!({"fields": []}),
            json!({}),
            TimeZone::UTC,
            vec![],
        )
    };
    let first = cache.put(mk());
    for _ in 0..DETAIL_CACHE_SIZE {
        cache.put(mk());
    }
    let page = |key: &str| DetailPage {
        res: &CHANGES,
        record: json!({}),
        tabs: vec![],
        badge: String::new(),
        due: String::new(),
        tz: None,
        schema: None,
        extra: json!({"detail_key": key}),
    };
    assert!(cache.get(&page(&first)).is_none());
    assert!(
        cache
            .get(&page(&format!("changes:{}", DETAIL_CACHE_SIZE + 1)))
            .is_some()
    );
}
