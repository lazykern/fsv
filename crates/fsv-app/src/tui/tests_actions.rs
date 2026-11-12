//! Expected requests, plans and results are from the Python TUI helpers
//! (`app._prepare_change_task`, `change_tasks.send`,
//! `actions.apply_change_action`, `actions.download_choices`,
//! `actions.download_selected`, `create.get_task_for_edit`,
//! `task_status.fields_from`, `catalog.*`, `completion._remote_user_values`,
//! `app._save_filter_history`) run over a recording client with the same
//! fixtures. The recording client's `me()` (`GET /api/v2/agents/me`,
//! `views.current_user`) is the real client's.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::testkit::{Call, MemJournal, Tenant, ok, pairs, tenant};
use fsv_core::client::Response;
use fsv_core::resource::{CHANGES, TICKETS};

fn backend(t: &Tenant) -> LiveBackend {
    let schema: SchemaLoader = Arc::new(|_r: Res| Ok(json!({"fields": []})));
    LiveBackend::new(t.client.clone(), schema)
        .with_journal(Arc::new(MemJournal::default()))
        .with_store(Arc::new(fsv_api::formmeta::MemStore::default()))
        .with_config(Map::new())
}

fn api(c: &Call) -> &str {
    c.path.strip_prefix("/api/_/").unwrap_or(&c.path)
}

fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap()
}

const TASK_HASH: &str = "4006c935db19ef56dffb5214360286bf26ec442442ad0b5a7647cc9ec2a48cbc";
const LINK_HASH: &str = "e0106201862cb1f3c6b8a02df97718fa086c768edd9a6070c54534bfce138fea";
const UNLINK_HASH: &str = "6c735e98775a076ee990b4dfe474d1404cb2df23c2e3c1a005e5349ca884e3d7";

fn status_fields() -> Value {
    json!({"change_task_fields": [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"}, {"id": 3, "value": "Completed"}]}]})
}

fn task_tenant() -> Tenant {
    let reads = Arc::new(AtomicUsize::new(0));
    tenant(move |c| match (c.method.as_str(), api(c)) {
        ("GET", "/search/autocomplete/agents") => {
            ok(json!({"results": [{"email": "ann@example.com", "user_id": 11, "value": "Ann"}]}))
        }
        ("GET", "bootstrap/agents_groups") => ok(json!({"groups": [{"id": 4, "name": "Desk"}]})),
        ("GET", "change_task_fields") => ok(status_fields()),
        ("GET", "changes/5/tasks") => {
            if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                ok(json!({"tasks": [], "meta": {"total_pages": 1}}))
            } else {
                ok(
                    json!({"tasks": [{"id": 9, "title": "Deploy", "owner_id": 11, "group_id": 4, "status": 2, "notify_before": 3600,
                                     "description": "<div><p>n</p></div>", "planned_effort": "1h 30m"}], "meta": {"total_pages": 1}}),
                )
            }
        }
        ("POST", "changes/5/tasks") => ok(json!({"task": {"id": 9}})),
        _ => None,
    })
}

fn task_options() -> Vec<(String, String)> {
    [
        ("group", "Desk"),
        ("status", "In Progress"),
        ("note", "n"),
        ("notify_before", "1 Hour"),
        ("planned_effort", "1h30m"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

#[test]
fn change_tasks_resolve_owner_group_and_status_then_post_once() {
    let t = task_tenant();
    let b = backend(&t);
    let plan = b
        .change_task_plan(
            &json!(5),
            "Deploy",
            Some("ann@example.com"),
            &task_options(),
        )
        .unwrap();
    assert_eq!(plan.hash, TASK_HASH);
    assert_eq!(
        compact(&plan.review["payload"]),
        r#"{"title":"Deploy","status":2,"notify_before":3600,"description":"<div><p>n</p></div>","group_id":4,"planned_effort":"1h 30m","owner_id":11}"#
    );
    assert_eq!(plan.review["status_label"], json!("In Progress"));
    assert_eq!(plan.review["reminder"], json!("1 Hour"));
    assert_eq!(plan.review["plan_hash"], json!(TASK_HASH));
    assert_eq!(
        t.lines(),
        [
            "GET /search/autocomplete/agents?q=ann@example.com&qf=value",
            "GET /api/_/bootstrap/agents_groups",
            "GET /api/_/change_task_fields?include_deleted_choices=true",
        ]
    );

    let out = b.send_change_task(&plan, false).unwrap();
    let calls = t.calls();
    let sent: Vec<String> = calls[3..].iter().map(Call::line).collect();
    assert_eq!(
        sent,
        [
            "GET /api/_/changes/5/tasks?page=1&per_page=100",
            "POST /api/_/changes/5/tasks",
            "GET /api/_/changes/5/tasks?page=1&per_page=100",
        ]
    );
    assert_eq!(
        calls[4].body.as_deref(),
        Some(
            r#"{"title": "Deploy", "status": 2, "notify_before": 3600, "description": "<div><p>n</p></div>", "group_id": 4, "planned_effort": "1h 30m", "owner_id": 11}"#
        )
    );
    assert_eq!(
        compact(&out),
        r#"{"status":"submitted","change_id":5,"task_id":9,"verified":true,"identity_verified":true,"fields_verified":true,"verified_fields":["title","status","notify_before","description","group_id","planned_effort","owner_id"],"unverified_fields":[],"mismatched_fields":[],"readback":{"id":9,"title":"Deploy","owner_id":11,"group_id":4,"status":2,"notify_before":3600,"description":"<div><p>n</p></div>","planned_effort":"1h 30m"},"plan_hash":"4006c935db19ef56dffb5214360286bf26ec442442ad0b5a7647cc9ec2a48cbc"}"#
    );
    let again = b.send_change_task(&plan, false).unwrap_err();
    assert!(again.to_string().contains("do not resubmit"), "{again}");
    assert_eq!(t.calls().len(), 6);
}

#[test]
fn change_task_plans_validate_before_any_lookup() {
    let t = task_tenant();
    let b = backend(&t);
    let bad = |opts: &[(&str, &str)]| {
        let opts: Vec<(String, String)> = opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        b.change_task_plan(&json!(5), "Deploy", Some("ann@example.com"), &opts)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        bad(&[("planned_effort", "soon")]),
        "planned effort must be duration text (1h30m) or HH:MM (01:30)"
    );
    assert_eq!(
        bad(&[("colour", "red")]),
        "build_plan() got an unexpected keyword argument 'colour'"
    );
    assert!(t.calls().is_empty());
    let read_only = b
        .send_change_task(
            &b.change_task_plan(&json!(5), "Deploy", None, &[]).unwrap(),
            true,
        )
        .unwrap_err();
    assert_eq!(read_only.to_string(), "read-only mode: no task was sent");
    assert!(
        b.send_change_task(
            &ReviewPlan {
                raw: json!({"kind": "association"}),
                ..b.change_task_plan(&json!(5), "Deploy", None, &[]).unwrap()
            },
            false
        )
        .is_err()
    );
}

fn assoc_tenant(unlink: bool) -> Tenant {
    let reads = Arc::new(AtomicUsize::new(0));
    tenant(move |c| match (c.method.as_str(), api(c)) {
        ("GET", "changes/5/tickets") => {
            let first = reads.fetch_add(1, Ordering::SeqCst) == 0;
            let linked = if unlink { first } else { !first };
            let rows = if linked {
                json!([{"id": 77}])
            } else {
                json!([])
            };
            ok(json!({"tickets": rows, "meta": {"total_pages": 1}}))
        }
        ("PUT", "changes/5/tickets/associate") | ("DELETE", "changes/5/tickets/77") => {
            ok(json!({}))
        }
        _ => None,
    })
}

#[test]
fn link_sends_the_reviewed_association_once_and_reads_it_back() {
    let t = assoc_tenant(false);
    let b = backend(&t);
    let plan = b.association_plan(&json!(5), &[json!(77)], false).unwrap();
    assert_eq!(plan.hash, LINK_HASH);
    let out = fsv_tui::actions::apply_change_action(
        &b,
        "link",
        &json!(5),
        &json!({"id": 77}),
        &Map::new(),
        Some(&plan),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        t.lines(),
        [
            "GET /api/_/changes/5/tickets?per_page=50&page=1",
            "PUT /api/_/changes/5/tickets/associate",
            "GET /api/_/changes/5/tickets?per_page=50&page=1",
        ]
    );
    assert_eq!(
        t.calls()[1].body.as_deref(),
        Some(r#"{"item_ids": [77], "module_type": "ticket"}"#)
    );
    assert_eq!(
        compact(&out),
        r#"{"status":"submitted","verified":true,"action":"change_ticket_association","tenant":"acme.freshservice.com","change_id":5,"ticket_ids":[77],"method":"PUT","path":"/api/_/changes/5/tickets/associate","payload":{"item_ids":[77],"module_type":"ticket"},"params":null,"plan_hash":"e0106201862cb1f3c6b8a02df97718fa086c768edd9a6070c54534bfce138fea","warning":"Initiating-ticket relationship only. Tenant workflows may notify people. One attempt; never automatically resubmit."}"#
    );
}

#[test]
fn unlink_deletes_one_ticket_and_a_changed_plan_is_refused() {
    let t = assoc_tenant(true);
    let b = backend(&t);
    let plan = b.association_plan(&json!(5), &[json!(77)], true).unwrap();
    assert_eq!(plan.hash, UNLINK_HASH);
    let out = b.send_association(&plan).unwrap();
    let calls = t.calls();
    assert_eq!(
        calls[1].line(),
        "DELETE /api/_/changes/5/tickets/77?module_type=ticket"
    );
    assert_eq!(calls[1].body.as_deref(), Some(r#"{"type": "unlink"}"#));
    assert_eq!(out["status"], json!("submitted"));
    assert_eq!(out["verified"], json!(true));

    let other = b.association_plan(&json!(5), &[json!(78)], true).unwrap();
    let err = fsv_tui::actions::apply_change_action(
        &b,
        "unlink",
        &json!(5),
        &json!({"id": 77}),
        &Map::new(),
        Some(&other),
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "association requires the exact reviewed plan"
    );
    assert_eq!(t.calls().len(), 3);
    assert!(
        b.send_association(&ReviewPlan {
            raw: json!({"kind": "change_task"}),
            ..plan
        })
        .is_err()
    );
}

fn task_edit_tenant() -> Tenant {
    tenant(|c| match (c.method.as_str(), api(c)) {
        ("GET", "changes/5/tasks/9") => ok(
            json!({"task": {"id": 9, "title": "T", "status": 1, "created_at": "x", "owner_id": null, "group_id": 4}}),
        ),
        ("GET", "change_task_fields") => ok(status_fields()),
        ("PUT", "changes/5/tasks/9") => ok(json!({"task": {"id": 9}})),
        ("PUT", "changes/5/tasks/update") => ok(json!({})),
        _ => None,
    })
}

#[test]
fn task_edits_read_the_task_and_status_fields_then_put_one_field() {
    let t = task_edit_tenant();
    let b = backend(&t);
    let (current, fields) = b.task_for_edit(&json!(5), &json!(9)).unwrap();
    assert_eq!(
        compact(&current),
        r#"{"title":"T","status":1,"group_id":4}"#
    );
    assert_eq!(
        Value::Array(fields.clone()),
        status_fields()["change_task_fields"]
    );
    let patch = fsv_tui::actions::task_patch(&current, "status", "In Progress", &fields).unwrap();
    assert_eq!(compact(&Value::Object(patch.clone())), r#"{"status":2}"#);
    fsv_tui::actions::apply_change_action(
        &b,
        "task-update",
        &json!(5),
        &json!({"id": 9}),
        &patch,
        None,
    )
    .unwrap();
    fsv_tui::actions::apply_change_action(
        &b,
        "task-delete",
        &json!(5),
        &json!({"id": 9}),
        &Map::new(),
        None,
    )
    .unwrap();
    let calls = t.calls();
    assert_eq!(
        calls.iter().map(Call::line).collect::<Vec<_>>(),
        [
            "GET /api/_/changes/5/tasks/9",
            "GET /api/_/change_task_fields?include_deleted_choices=true",
            "PUT /api/_/changes/5/tasks/9",
            "PUT /api/_/changes/5/tasks/update",
        ]
    );
    assert_eq!(calls[2].json(), Some(json!({"status": 2})));
    assert_eq!(
        calls[3].json(),
        Some(json!({"id": "9", "action": "delete"}))
    );
}

#[test]
fn unreadable_task_status_metadata_is_refused() {
    let t = tenant(|c| match api(c) {
        "changes/5/tasks/9" => ok(json!({"task": {"id": 9, "title": "T"}})),
        "change_task_fields" => ok(json!({"other": []})),
        _ => None,
    });
    let err = backend(&t).task_for_edit(&json!(5), &json!(9)).unwrap_err();
    assert_eq!(err.to_string(), "unreadable runtime task-status metadata");
}

#[test]
fn catalog_lists_items_then_categories_and_filters_by_category_tree() {
    let t = tenant(|c| match api(c) {
        "catalog/service-items" => ok(
            json!({"service_items": [{"id": 1, "display_id": 10, "name": "Laptop", "category_id": 2}, {"id": 2, "display_id": 11, "name": "Mouse", "category_id": 3}], "meta": {"total_pages": 1}}),
        ),
        "catalog/service-categories" => ok(
            json!({"service_categories": [{"id": 2, "name": "Hardware", "sub_categories": [{"id": 3, "name": "Peripherals"}]}]}),
        ),
        _ => None,
    });
    let b = backend(&t);
    let (items, cats) = b.catalog().unwrap();
    assert_eq!(
        t.lines(),
        [
            "GET /api/_/catalog/service-items?aurora_search=true&page=1&per_page=30",
            "GET /api/_/catalog/service-categories?category_hierarchy=true",
        ]
    );
    assert_eq!(
        compact(&json!(cats)),
        r#"[{"id":2,"name":"Hardware"},{"id":3,"name":"Peripherals","parent_id":2}]"#
    );
    let ids: Vec<Value> = b
        .resolve_category(&cats, "2")
        .iter()
        .map(|c| c["id"].clone())
        .collect();
    assert_eq!(ids, [json!(2), json!(3)]);
    assert_eq!(b.filter_catalog_items(&items, Some(&ids)), items);
    assert_eq!(
        b.filter_catalog_items(&items, Some(&[json!("3")])),
        [items[1].clone()]
    );
    assert_eq!(b.filter_catalog_items(&items, None), items);
    assert!(b.resolve_category(&cats, "nope").is_empty());
}

#[test]
fn link_targets_come_from_the_change_ticket_search() {
    let t = tenant(|c| match api(c) {
        "changes/tickets/search" => ok(json!({"tickets": [{"id": 77, "subject": "Down"}]})),
        _ => None,
    });
    let rows = backend(&t).search_change_tickets("down").unwrap();
    assert_eq!(rows, [json!({"id": 77, "subject": "Down"})]);
    assert_eq!(
        t.lines(),
        ["GET /api/_/changes/tickets/search?module_type=change&q=down"]
    );
}

fn detail_routes(c: &Call) -> Option<Response> {
    match api(c) {
        "changes/5" => ok(json!({"change": {"id": 5, "display_id": 5, "subject": "A",
            "attachments": [{"id": 1, "name": "a.txt", "attachment_url": "https://acme.freshservice.com/helpdesk/attachments/1"}],
            "cloud_files": [{"id": 2, "name": "c.doc", "url": "https://drive/x"}]}})),
        "changes/5/planning-fields" => ok(
            json!({"change_planning_fields": [{"label": "Rollout", "attachments": [{"id": 3, "name": "p.pdf"}]}, {"name": "backout"}]}),
        ),
        "tickets/12" => ok(
            json!({"ticket": {"id": 12, "display_id": 12, "subject": "T", "type": "Incident", "attachments": [{"id": 4, "name": "t.png"}]}}),
        ),
        "tickets/12/conversations" => ok(
            json!({"conversations": [{"id": 70, "attachments": [{"id": 5, "name": "r.log"}]}, {"id": 71}], "meta": {"count": 2}}),
        ),
        _ => None,
    }
}

fn without_resource(rows: Vec<Value>) -> Vec<Value> {
    rows.into_iter()
        .map(|mut r| {
            if let Value::Object(m) = &mut r {
                m.shift_remove("_resource");
            }
            r
        })
        .collect()
}

#[test]
fn change_downloads_list_request_cloud_and_planning_files() {
    let t = tenant(detail_routes);
    let b = backend(&t);
    let item = Item::new(&CHANGES, json!({"id": 5, "display_id": 5}));
    let (rows, warnings) = fsv_tui::actions::download_choices(&b, &item).unwrap();
    assert_eq!(
        compact(&json!(without_resource(rows))),
        r#"[{"id":1,"name":"a.txt","attachment_url":"https://acme.freshservice.com/helpdesk/attachments/1","source":"request"},{"id":2,"name":"c.doc","url":"https://drive/x","source":"cloud file"},{"id":3,"name":"p.pdf","source":"planning: Rollout"}]"#
    );
    assert_eq!(
        warnings,
        [
            "tabs: not available on this account (404)",
            "no form fields"
        ]
    );
    assert_eq!(
        t.lines(),
        [
            "GET /api/_/changes/5?include=requester,stats",
            "GET /api/v2/agents/me",
            "GET /api/_/changes/5/tabs",
            "GET /api/_/changes/5/planning-fields",
        ]
    );
}

#[test]
fn an_unreadable_planning_inventory_is_a_warning() {
    let t = tenant(|c| {
        if api(c) == "changes/5/planning-fields" {
            None
        } else {
            detail_routes(c)
        }
    });
    let item = Item::new(&CHANGES, json!({"id": 5, "display_id": 5}));
    let (rows, warnings) = backend(&t).download_choices(&item).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        warnings.last().unwrap(),
        "Planning attachment inventory unavailable: HTTP 404 listing planning-fields: not found"
    );
}

#[test]
fn ticket_downloads_add_every_conversation_attachment() {
    let t = tenant(detail_routes);
    let item = Item::new(&TICKETS, json!({"id": 12, "display_id": 12}));
    let (rows, warnings) = backend(&t).download_choices(&item).unwrap();
    assert_eq!(
        compact(&json!(without_resource(rows))),
        r#"[{"id":4,"name":"t.png","source":"request"},{"id":5,"name":"r.log","source":"conversation 70"}]"#
    );
    assert_eq!(
        warnings,
        [
            "tabs: not available on this account (404)",
            "no form fields"
        ]
    );
    assert_eq!(
        t.lines(),
        [
            "GET /api/_/tickets/12?include=requester,stats,phone,feedback,ticket_status,responder",
            "GET /api/v2/agents/me",
            "GET /api/_/tickets/12/tabs",
            "GET /api/_/tickets/12/requested_items",
            "GET /api/_/tickets/12/conversations?include=user,phone,feedback&per_page=3",
            "GET /api/_/tickets/12/conversations?include=user,phone,feedback&per_page=30&page=1",
        ]
    );
}

#[test]
fn requested_item_files_take_the_field_name_and_dedupe_by_id() {
    let record = json!({"attachments": [{"id": 1, "name": "a"}, {"name": "noid"}]});
    let items = [
        json!({"custom_fields": {"laptop_photo": [{"id": 1, "name": "a"}, {"id": 8, "name": "b", "attachment_url": "u"}], "proof": [9], "note": "x"}}),
    ];
    assert_eq!(
        compact(&json!(files::attachment_rows(&record, &items))),
        r#"[{"id":1,"name":"a","source":"laptop_photo"},{"name":"noid","source":"request"},{"id":8,"name":"b","attachment_url":"u","source":"laptop_photo"},{"id":9,"name":"attachment-9","source":"proof"}]"#
    );
}

fn file_tenant() -> Tenant {
    tenant(|c| match c.path.as_str() {
        "/helpdesk/attachments/1" => Some(Response::new(200).with_body(b"hello".to_vec())),
        "/helpdesk/attachments/6" => Some(Response::new(200).with_body(b"six".to_vec())),
        "/helpdesk/attachments/7" => Some(Response::new(500)),
        _ => None,
    })
}

#[test]
fn downloads_write_new_files_only_like_cli_sr_download() {
    let t = file_tenant();
    let b = backend(&t);
    let out = tempfile::tempdir().unwrap();
    let dir = out.path().to_string_lossy().into_owned();
    let real = std::fs::canonicalize(out.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let get = |att: Value| {
        compact(&fsv_tui::actions::download_selected(&b, &att, &dir).unwrap())
            .replace(&real, "<DIR>")
    };
    let att = json!({"id": 1, "name": "a.txt", "attachment_url": "https://acme.freshservice.com/helpdesk/attachments/1", "source": "request"});
    assert_eq!(
        get(att.clone()),
        r#"{"id":1,"name":"a.txt","source":"request","path":"<DIR>/a.txt","size":5,"status":"downloaded"}"#
    );
    assert_eq!(
        get(att),
        r#"{"id":1,"name":"a.txt","source":"request","path":"<DIR>/a.txt","status":"exists"}"#
    );
    assert_eq!(
        get(json!({"id": 2, "name": "../x"})),
        r#"{"id":2,"name":"../x","source":null,"status":"rejected","error":"unsafe file name '../x': contains '..'"}"#
    );
    assert_eq!(
        get(json!({"id": 6, "name": "six.bin", "source": "cloud file"})),
        r#"{"id":6,"name":"six.bin","source":"cloud file","path":"<DIR>/six.bin","size":3,"status":"downloaded"}"#
    );
    assert_eq!(
        get(json!({"name": "z"})),
        r#"{"id":null,"name":"z","source":null,"status":"error","error":"no download URL"}"#
    );
    assert_eq!(
        get(
            json!({"id": 7, "name": "e", "attachment_url": "https://acme.freshservice.com/helpdesk/attachments/7"})
        ),
        r#"{"id":7,"name":"e","source":null,"status":"error","error":"HTTP 500"}"#
    );
    assert_eq!(
        get(
            json!({"id": 6, "canonical_url": "https://acme.freshservice.com/helpdesk/attachments/6"})
        ),
        r#"{"id":6,"name":"attachment-6","source":null,"path":"<DIR>/attachment-6","size":3,"status":"downloaded"}"#
    );
    let fetched: Vec<String> = t
        .mock
        .requests()
        .iter()
        .filter(|r| r.url.contains("/helpdesk/"))
        .map(|r| r.url.clone())
        .collect();
    assert_eq!(
        fetched,
        [
            "https://acme.freshservice.com/helpdesk/attachments/1?download=true",
            "https://acme.freshservice.com/helpdesk/attachments/6?download=true",
            "https://acme.freshservice.com/helpdesk/attachments/7?download=true",
            "https://acme.freshservice.com/helpdesk/attachments/6?download=true",
        ]
    );
    let mut names: Vec<String> = std::fs::read_dir(out.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "attachment-6", "six.bin"]);
    assert_eq!(std::fs::read(out.path().join("a.txt")).unwrap(), b"hello");
}

#[test]
fn filter_history_uses_the_python_file_and_format() {
    let t = tenant(|_| None);
    let b = backend(&t);
    let path = b.history_path();
    assert_eq!(
        path,
        t.client
            .ctx()
            .paths
            .filters_dir
            .join("acme.freshservice.com--tui_history.json")
    );
    assert!(b.load_filter_history().is_empty());
    b.save_filter_history(&["a".into(), "é".into()]);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        r#"{"history": ["a", "\u00e9"]}"#
    );
    assert_eq!(b.load_filter_history(), ["a", "é"]);
    let many: Vec<String> = (0..150).map(|i| i.to_string()).collect();
    b.save_filter_history(&many);
    assert_eq!(b.load_filter_history(), many[..100]);
    std::fs::write(&path, r#"{"history": ["x"], "other": 1}"#).unwrap();
    assert_eq!(b.load_filter_history(), ["x"]);
    std::fs::write(&path, "[1]").unwrap();
    assert!(b.load_filter_history().is_empty());
}

#[test]
fn group_names_come_from_the_groups_cache() {
    let t = tenant(|_| None);
    let b = backend(&t);
    assert!(b.cached_group_names().is_empty());
    let path = t
        .client
        .ctx()
        .paths
        .cache_dir
        .join("groups--acme.freshservice.com.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"saved_at": 1, "_kind": "groups", "data": {"groups": [{"id": 1, "name": "Desk"}, {"id": 2}, {"id": 3, "name": "Network Ops"}]}}"#).unwrap();
    assert_eq!(b.cached_group_names(), ["Desk", "Network Ops"]);
    std::fs::write(&path, r#"{"data": [{"name": "Flat"}]}"#).unwrap();
    assert_eq!(b.cached_group_names(), ["Flat"]);
    std::fs::write(&path, r#"{"data": ["bad"]}"#).unwrap();
    assert!(b.cached_group_names().is_empty());
}

fn people_tenant() -> Tenant {
    tenant(|c| match c.path.as_str() {
        "/search/autocomplete/requesters" => ok(
            json!({"results": [{"email": "Jane@Example.com", "value": "Jane Doe", "id": 4}, {"name": "jack", "id": 5}]}),
        ),
        "/search/autocomplete/agents" => {
            ok(json!({"results": [{"details": "ja@x.io", "value": "Ja"}]}))
        }
        "/api/_/lookup/assets" => ok(json!({"results": [{"name": "Mac", "id": 1}]})),
        _ => None,
    })
}

#[test]
fn remote_people_suggestions_follow_completion_remote_user_values() {
    let t = people_tenant();
    let on = backend(&t).with_config(
        json!({"completion": {"network": "on"}})
            .as_object()
            .unwrap()
            .clone(),
    );
    assert!(on.completion_network());
    assert_eq!(
        on.remote_user_values("requesters", " ja ").unwrap(),
        ["jane@Example.com", "jack"]
    );
    assert_eq!(on.remote_user_values("agents", "ja").unwrap(), ["ja@x.io"]);
    assert!(on.remote_user_values("agents", "j").unwrap().is_empty());
    assert_eq!(
        t.lines(),
        [
            "GET /search/autocomplete/requesters?q=ja&qf=value&all_users=true",
            "GET /search/autocomplete/agents?q=ja&qf=value",
        ]
    );
    assert_eq!(
        user_values(
            &[
                json!({"email": "Jane@Example.com", "value": "Jane Doe", "id": 4}),
                json!({"name": "jack", "id": 5})
            ],
            "ja"
        ),
        [
            ("jane@Example.com".to_string(), "Jane Doe".to_string()),
            ("jack".to_string(), "jack".to_string())
        ]
    );
    if std::env::var("FSV_COMPLETION_NETWORK").as_deref() != Ok("1") {
        let off = backend(&t);
        assert!(!off.completion_network());
        assert!(off.remote_user_values("agents", "ja").unwrap().is_empty());
        assert_eq!(t.lines().len(), 2);
        for (v, want) in [
            (json!(true), true),
            (json!(1), true),
            (json!(1.0), true),
            (json!("true"), true),
            (json!([]), false),
            (json!("off"), false),
            (json!(0), false),
        ] {
            let b = backend(&t).with_config(
                json!({"completion": {"network": v}})
                    .as_object()
                    .unwrap()
                    .clone(),
            );
            assert_eq!(b.completion_network(), want);
        }
    }
}

#[test]
fn lookup_choices_query_the_fields_link() {
    let t = people_tenant();
    let rows = backend(&t)
        .lookup_choices("/api/_/lookup/assets?type=x", "ma")
        .unwrap();
    assert_eq!(rows, [json!({"name": "Mac", "id": 1})]);
    assert_eq!(t.lines(), ["GET /api/_/lookup/assets?type=x&q=ma"]);
}

#[test]
fn yank_and_open_go_through_the_injected_os_hooks() {
    let t = tenant(|_| None);
    let copied = Arc::new(Mutex::new(Vec::<String>::new()));
    let opened = Arc::new(Mutex::new(Vec::<String>::new()));
    let (c2, o2) = (copied.clone(), opened.clone());
    let b = backend(&t)
        .with_clipboard(Arc::new(move |s: &str| {
            c2.lock().unwrap().push(s.into());
            Err("no pasteboard".into())
        }))
        .with_browser(Arc::new(move |u: &str| {
            o2.lock().unwrap().push(u.into());
            true
        }));
    assert_eq!(
        b.copy_to_clipboard("CHN-5"),
        Err("no pasteboard".to_string())
    );
    assert!(b.open_url("https://acme.freshservice.com/a/changes/5"));
    assert_eq!(*copied.lock().unwrap(), ["CHN-5"]);
    assert_eq!(
        *opened.lock().unwrap(),
        ["https://acme.freshservice.com/a/changes/5"]
    );
}

#[test]
fn clipboard_commands_fail_like_subprocess_run() {
    assert_eq!(desktop::run_with_input(&["cat"], b"x"), Ok(()));
    assert_eq!(
        desktop::run_with_input(&["false"], b"x"),
        Err("Command '['false']' returned non-zero exit status 1.".into())
    );
    assert_eq!(
        desktop::run_with_input(&["/nonexistent/pbcopy"], b"x"),
        Err("[Errno 2] No such file or directory: '/nonexistent/pbcopy'".into())
    );
    #[cfg(unix)]
    assert_eq!(
        desktop::run_with_input(&["sh", "-c", "kill -9 $$"], b"x"),
        Err("Command '['sh', '-c', 'kill -9 $$']' died with <Signals.SIGKILL: 9>.".into())
    );
}

#[test]
fn browser_order_follows_webbrowser() {
    use desktop::Launch;
    assert_eq!(
        desktop::osascript_script("default", "https://a/\"b"),
        "open location \"https://a/%22b\""
    );
    let mac = desktop::browser_plan("u", Some("w3m:lynx"), true, false);
    assert_eq!(mac[0], Launch::Wait(vec!["w3m".into(), "u".into()]));
    assert_eq!(mac[1], Launch::Wait(vec!["lynx".into(), "u".into()]));
    assert_eq!(mac[2], Launch::Osascript("open location \"u\"".into()));
    assert_eq!(mac.len(), 6);
    assert_eq!(
        desktop::browser_plan("u", None, false, true),
        [
            Launch::Background(vec!["xdg-open".into(), "u".into()]),
            Launch::Background(vec!["gio".into(), "open".into(), "u".into()]),
        ]
    );
    assert!(desktop::browser_plan("u", None, false, false).is_empty());
}

#[test]
fn ticket_search_failures_and_pairs_helper() {
    let t = tenant(|_| None);
    let err = backend(&t).search_change_tickets("x").unwrap_err();
    assert_eq!(err.to_string(), "HTTP 404 listing search: not found");
    assert_eq!(
        t.calls()[0].query,
        pairs(&[("module_type", "change"), ("q", "x")])
    );
}

/// `views.load_detail` ends with `load_properties(c, res, rec, sch, tz, user)`;
/// the expected properties are Python's for the same record, schema and
/// rules (`testdata/properties_change_501.json`, written by
/// `port/golden/oracles/fsv_api_views/oracle.py`).
#[test]
fn detail_properties_are_the_python_form_evaluation_loaded_last() {
    let case: Value =
        serde_json::from_str(include_str!("testdata/properties_change_501.json")).unwrap();
    let (record, user, routes) = (
        case["record"].clone(),
        case["user"].clone(),
        case["routes"].clone(),
    );
    let t = tenant(move |c| match api(c) {
        "changes/501" => ok(json!({"change": record})),
        "/api/v2/agents/me" => ok(json!({"agent": user})),
        p => routes.get(p).cloned().and_then(ok),
    });
    let b = backend(&t);
    let page = b
        .load_detail(&CHANGES, &json!(501), Some(&case["schema"]))
        .unwrap();
    let d = b.details.get(&page).unwrap();
    let props: Vec<Value> = d.properties.iter().map(|p| p.to_json()).collect();
    assert_eq!(json!(props), case["result"]["ok"]["props"]);
    assert_eq!(json!(d.properties_note), case["result"]["ok"]["note"]);
    let lines = t.lines();
    let rules = lines
        .iter()
        .position(|l| l.starts_with("GET /api/_/ws/2/business_rules?"))
        .unwrap();
    assert_eq!(rules, lines.len() - 1, "{lines:#?}");
    assert!(lines[rules].contains("form_type=edit_form") && lines[rules].contains("module_id=501"));
}

#[test]
fn schemas_load_once_into_the_profile_schema_cache() {
    let t = tenant(|c| match c.path.as_str() {
        "/api/_/change_form_fields" => {
            ok(json!({"change_fields": [{"name": "subject", "label": "Subject"}]}))
        }
        "/api/v2/change_form_fields" => ok(json!({"change_fields": []})),
        _ => None,
    });
    let load = schema_loader(t.client.clone());
    let first = load(&CHANGES).unwrap();
    assert_eq!(
        first["fields"],
        json!([{"name": "subject", "label": "Subject"}])
    );
    assert_eq!(
        t.lines(),
        [
            "GET /api/_/change_form_fields",
            "GET /api/v2/change_form_fields"
        ]
    );
    assert!(
        t.client
            .ctx()
            .paths
            .schema_dir
            .join("acme.freshservice.com--changes.json")
            .exists()
    );
    assert_eq!(load(&CHANGES).unwrap()["fields"], first["fields"]);
    assert_eq!(t.lines().len(), 2);
    let live = LiveBackend::live(t.client.clone())
        .with_store(Arc::new(fsv_api::formmeta::MemStore::default()));
    assert_eq!(
        live.load_schema(&CHANGES).unwrap()["fields"],
        first["fields"]
    );
    assert_eq!(t.lines().len(), 2);
}
