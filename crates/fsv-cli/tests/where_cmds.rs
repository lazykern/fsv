//! `--where` / `--query-hash` resolution (`cli._resolve_where` and helpers)
//! against canned schemas. Expected output was captured from the Python fsv
//! on the same routes (`tests/golden/cmp.py --emit`) unless a test says so.

mod common;
use common::run_fixture;

/// Python's `json.JSONDecodeError` text after the prefix.
#[test]
fn query_hash_bad_json() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--query-hash", "[{", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(
        r.err,
        "error: invalid --query-hash JSON: Expecting property name enclosed in double quotes: line 1 column 3 (char 2)\n"
    );
}

/// `query_hash` sent to `/api/_/RES`, and how many `/api/v2/agents/me` calls.
fn sent(r: &common::Run, res: &str) -> (String, usize) {
    sent_param(r, res, "query_hash")
}

/// `PARAM` sent to `/api/_/RES`, and how many `/api/v2/agents/me` calls.
fn sent_param(r: &common::Run, res: &str, param: &str) -> (String, usize) {
    let list = format!("/api/_/{res}?");
    let key = format!("{param}=");
    let hash = r
        .calls()
        .iter()
        .find_map(|(_, p)| p.strip_prefix(&list).map(str::to_string))
        .and_then(|q| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix(key.as_str()).map(str::to_string))
        })
        .map(|v| fsv_core::pystr::unquote(&v.replace('+', " ")))
        .unwrap_or_default();
    let me = r
        .calls()
        .iter()
        .filter(|(_, p)| p == "/api/v2/agents/me")
        .count();
    (hash, me)
}

/// Payloads from the Python run's request log on the same routes.
#[test]
fn where_payload_merges_dates_and_choices() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "created>=2026-01-01",
            "--where",
            "created_at<=2026-02-01",
            "--where",
            "Agents=me",
            "--where",
            "group=(empty)",
            "--where",
            "status=Open",
            "--where",
            "status=Resolved",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        sent(&r, "tickets"),
        (
            r#"[{"value":{"from":"2026-01-01","to":"2026-02-01"},"condition":"created_at","operator":"is_in_the_range","type":"default"},{"value":["0"],"condition":"responder_id","operator":"is_in","type":"default"},{"value":["-1"],"condition":"group_id","operator":"is_in","type":"default"},{"value":[2,4],"condition":"status","operator":"is_in","type":"default"}]"#.into(),
            1
        )
    );
}

#[test]
fn where_payload_requester_me_custom_fields() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "requester=me",
            "--where",
            "Count=5",
            "--where",
            "Environment=Prod",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(sent(&r, "tickets"), (String::new(), 1));
    assert_eq!(
        sent_param(&r, "tickets", "advanced_query_hash"),
        (
            r#"{"all":[{"value":[{"id":99,"name":"Me Agent"}],"condition":"requester_id","operator":"is_in","type":"default"},{"value":"5","condition":"cf_count","operator":"is","type":"custom_field"},{"value":["Prod"],"condition":"cf_env","operator":"is_in","type":"custom_field"}],"any":[]}"#.into(),
            1
        )
    );
}

#[test]
fn where_payload_change_agent_and_id() {
    let r = run_fixture(
        "where",
        &[
            "changes",
            "ls",
            "--where",
            "agent=me",
            "--where",
            "requester_id=12",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        sent(&r, "changes"),
        (
            r#"[{"value":["0"],"condition":"agent_id","operator":"is_in","type":"default"},{"value":[12],"condition":"requester_id","operator":"is_in","type":"default"}]"#.into(),
            1
        )
    );
}

#[test]
fn where_period_alias() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "created>=last 7 days",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                              query                               
┏━━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━━┓
┃ field   ┃ condition  ┃ operator        ┃ type    ┃ value       ┃
┡━━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━━┩
│ created │ created_at │ is_greater_than │ default │ last 7 days │
└─────────┴────────────┴─────────────────┴─────────┴─────────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_label_date_period() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Created=Last 6 months",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                               query                                
┏━━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━━━━┓
┃ field   ┃ condition  ┃ operator        ┃ type    ┃ value         ┃
┡━━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━━━━┩
│ Created │ created_at │ is_greater_than │ default │ Last 6 months │
└─────────┴────────────┴─────────────────┴─────────┴───────────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_period_token_lt() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "created<today", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: relative date periods support only = or >=; no inverse period wire is proven
"#
    );
}

#[test]
fn where_date_strict_gt_rejected() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Created>2026-01-01", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: a date range on created_at needs both ends (e.g. created_at>=2030-01-01 and created_at<=2030-01-31): the server answers a one-sided range with HTTP 500, so nothing was sent
"#
    );
}

#[test]
fn where_date_period_gt_rejected() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "created>today", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: relative date periods support only = or >=; no inverse period wire is proven
"#
    );
}

#[test]
fn where_change_date_lt_rejected() {
    let r = run_fixture(
        "where",
        &["changes", "ls", "--where", "created<2026-01-01", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: a date range on created_at needs both ends (e.g. created_at>=2030-01-01 and created_at<=2030-01-31): the server answers a one-sided range with HTTP 500, so nothing was sent
"#
    );
}

#[test]
fn where_date_range_merged() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "created>=2026-01-01",
            "--where",
            "created_at<=2026-02-01",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                               query                                
┏━━━━━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━┓
┃ field      ┃ condition  ┃ operator        ┃ type    ┃ value      ┃
┡━━━━━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━┩
│ created    │ created_at │ is_in_the_range │ default │ 2026-01-01 │
│ created_at │ created_at │ is_in_the_range │ default │ 2026-02-01 │
└────────────┴────────────┴─────────────────┴─────────┴────────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_date_not_equal() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "updated_at!=2026-01-01",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: date operator '!=' is not supported; use = > >= < <= (whole days)
"#
    );
}

#[test]
fn where_date_contains_rejected() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "created~=x", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: date filters take a whole day YYYY-MM-DD (the server ignores time of day) or a relative on:/last:/next: period; nothing was sent
"#
    );
}

#[test]
fn where_due_label() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Due Date>=2026-02-01",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: a date range on due_by_date needs both ends (e.g. due_by_date>=2030-01-01 and due_by_date<=2030-01-31): the server answers a one-sided range with HTTP 500, so nothing was sent
"#
    );
}

/// Python `3ad6041` through the CLI on the same schema: the due range is advanced.
#[test]
fn where_due_label_range() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Due Date>=2026-02-01",
            "--where",
            "Due Date<=2026-02-28",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                               query                               
┏━━━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━┓
┃ field    ┃ condition   ┃ operator        ┃ type    ┃ value      ┃
┡━━━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━┩
│ Due Date │ due_by_date │ is_in_the_range │ default │ 2026-02-01 │
│ Due Date │ due_by_date │ is_in_the_range │ default │ 2026-02-28 │
└──────────┴─────────────┴─────────────────┴─────────┴────────────┘
api: internal /api/_/ with advanced_query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

/// Python `406ca71` through the CLI on the same schema: relative `on` and
/// number conditions go to `advanced_query_hash`.
#[test]
fn where_advanced_debug() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "created=on:yesterday",
            "--where",
            "Count=3",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                              query                              
┏━━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━┓
┃ field   ┃ condition  ┃ operator ┃ type         ┃ value        ┃
┡━━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━┩
│ created │ created_at │ on       │ default      │ on:yesterday │
│ Count   │ cf_count   │ is       │ custom_field │ 3            │
└─────────┴────────────┴──────────┴──────────────┴──────────────┘
api: internal /api/_/ with advanced_query_hash (2 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

/// Python `406ca71`: text fields have no list filter.
#[test]
fn where_text_field_refused() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Note=hi", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no list filter exists for text field 'Note': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword 
search
"#
    );
    assert!(
        r.calls()
            .iter()
            .all(|(_, p)| !p.starts_with("/api/_/tickets?"))
    );
}

#[test]
fn where_agent_me_plural() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Agents=me", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                        query                        
┏━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field ┃ condition    ┃ operator ┃ type    ┃ value ┃
┡━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ agent │ responder_id │ is_in    │ default │ Me    │
└───────┴──────────────┴──────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_agent_unassigned_alias() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Assigned to=unassigned",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                          query                           
┏━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━┓
┃ field ┃ condition    ┃ operator ┃ type    ┃ value      ┃
┡━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━┩
│ agent │ responder_id │ is_in    │ default │ Unassigned │
└───────┴──────────────┴──────────┴─────────┴────────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_agent_not_me() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Agents!=me", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                        query                         
┏━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field ┃ condition    ┃ operator  ┃ type    ┃ value ┃
┡━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ agent │ responder_id │ is_not_in │ default │ Me    │
└───────┴──────────────┴───────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_agent_by_name() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "agent=Bob Ray", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                        query                        
┏━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field ┃ condition    ┃ operator ┃ type    ┃ value ┃
┡━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ agent │ responder_id │ is_in    │ default │ 141   │
└───────┴──────────────┴──────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_agent_no_user_id() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "agent=nouid", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: agent match has no user_id: nouid
"#
    );
}

#[test]
fn where_agent_no_match() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "agent=zed", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no agent match: zed
"#
    );
}

#[test]
fn where_change_agent_condition() {
    let r = run_fixture(
        "where",
        &["changes", "ls", "--where", "agent=me", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                      query                       
┏━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field ┃ condition ┃ operator ┃ type    ┃ value ┃
┡━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ agent │ agent_id  │ is_in    │ default │ Me    │
└───────┴───────────┴──────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_requester_me() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "requester=me", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                           query                           
┏━━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┓
┃ field     ┃ condition    ┃ operator ┃ type    ┃ value   ┃
┡━━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━┩
│ requester │ requester_id │ is_in    │ default │ Me (99) │
└───────────┴──────────────┴──────────┴─────────┴─────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_requester_me_unreadable() {
    let r = run_fixture(
        "where_noid",
        &["tickets", "ls", "--where", "requester=me", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: cannot tell who you are (agents/me is not readable); use your email instead of 'me'
"#
    );
}

#[test]
fn where_requester_email() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Requesters!=sue@acme.com",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                              query                               
┏━━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━━━━┓
┃ field     ┃ condition    ┃ operator  ┃ type    ┃ value         ┃
┡━━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━━━━┩
│ requester │ requester_id │ is_not_in │ default │ Sue Park (51) │
└───────────┴──────────────┴───────────┴─────────┴───────────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_requester_ambiguous() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "requester=ann", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no exact requester match for 'ann': Ann Lee <ann@acme.com>; Ann Lee <ann2@acme.com>. Use the exact name/email or a numeric ID.
"#
    );
}

#[test]
fn where_group_plural() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Groups=DBA", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                      query                       
┏━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field ┃ condition ┃ operator ┃ type    ┃ value ┃
┡━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ group │ group_id  │ is_in    │ default │ 9     │
└───────┴───────────┴──────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_group_empty() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "group=(empty)", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                       query                        
┏━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┓
┃ field ┃ condition ┃ operator ┃ type    ┃ value   ┃
┡━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━┩
│ group │ group_id  │ is_in    │ default │ (empty) │
└───────┴───────────┴──────────┴─────────┴─────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_department_none_is_choice() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "department=none", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                           query                           
┏━━━━━━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field      ┃ condition     ┃ operator ┃ type    ┃ value ┃
┡━━━━━━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ department │ department_id │ is_in    │ default │ 22    │
└────────────┴───────────────┴──────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_department_dash_empty() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Departments=-", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                            query                            
┏━━━━━━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┓
┃ field      ┃ condition     ┃ operator ┃ type    ┃ value   ┃
┡━━━━━━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━┩
│ department │ department_id │ is_in    │ default │ (empty) │
└────────────┴───────────────┴──────────┴─────────┴─────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_dropdown_label() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Environment=Prod", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                         query                          
┏━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━┓
┃ field  ┃ condition ┃ operator ┃ type         ┃ value ┃
┡━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━┩
│ cf_env │ cf_env    │ is_in    │ custom_field │ Prod  │
└────────┴───────────┴──────────┴──────────────┴───────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_dropdown_empty() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "cf_env=empty", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                          query                           
┏━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━┓
┃ field  ┃ condition ┃ operator ┃ type         ┃ value   ┃
┡━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━┩
│ cf_env │ cf_env    │ is_in    │ custom_field │ (empty) │
└────────┴───────────┴──────────┴──────────────┴─────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_dropdown_bad_choice() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Environment=Nope", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no choice 'Nope' for Environment; choices: Prod, UAT
"#
    );
}

#[test]
fn where_plural_cut() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Statuses=Open",
            "--where",
            "Prioritys=Low",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                        query                        
┏━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field    ┃ condition ┃ operator ┃ type    ┃ value ┃
┡━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ status   │ status    │ is_in    │ default │ 2     │
│ priority │ priority  │ is_in    │ default │ 1     │
└──────────┴───────────┴──────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (2 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_number_and_text() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Count=5",
            "--where",
            "cf_count=x",
            "--where",
            "Note=hi",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: number filter 'Count' takes an integer value
"#
    );
}

#[test]
fn where_id_field() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "requester_id=12",
            "--where",
            "group_id!=x",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                            query                            
┏━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field        ┃ condition    ┃ operator  ┃ type    ┃ value ┃
┡━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ requester_id │ requester_id │ is_in     │ default │ 12    │
│ group_id     │ group_id     │ is_not_in │ default │ x     │
└──────────────┴──────────────┴───────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (2 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn where_id_inequality_rejected() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "requester_id>3", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: operator '>' not supported on ID fields; use = or !=
"#
    );
}

#[test]
fn where_inequality_rejected() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "status>2", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: operator '>' not supported; inequalities only work for date fields (created_at, updated_at, etc.). Use = or !=
"#
    );
}

#[test]
fn where_contains_rejected() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Status~=x", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: operator ~= (contains) not supported; use = or !=
"#
    );
}

#[test]
fn where_unparsable() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "bad", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: invalid filter 'bad'; expected FIELD followed by = != >= <= > < or ~=
"#
    );
}

#[test]
fn where_unknown_field_lists_labels() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "nosuch=1", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: field not found: nosuch; filter by a label shown on the form (Agent, Count, Department, Due Date, Environment, Group, Note, Priority, Requester, Status) or see `fsv tickets fields`
"#
    );
}

#[test]
fn where_or_grouping() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "status=Open",
            "--where",
            "Priority!=Low",
            "--or",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                        query                         
┏━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field    ┃ condition ┃ operator  ┃ type    ┃ value ┃
┡━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ status   │ status    │ is_in     │ default │ 2     │
│ priority │ priority  │ is_not_in │ default │ 1     │
└──────────┴───────────┴───────────┴─────────┴───────┘
api: internal /api/_/ with advanced_query_hash (2 conditions, OR)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn query_hash_not_array() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--query-hash", "{\"a\":1}", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --query-hash must be a JSON array
"#
    );
}

#[test]
fn query_hash_percent_with_where() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--query-hash",
            "%5B%7B%22condition%22%3A%22status%22%2C%22operator%22%3A%22is_in%22%2C%22type%22%3A%22default%22%2C%22value%22%3A%5B2%5D%7D%5D",
            "--where",
            "status=Resolved",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                                                     query                                                                                      
┏━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ field      ┃ condition ┃ operator ┃ type    ┃ value                                                                                                                          ┃
┡━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ query_hash │ raw       │ raw      │ raw     │ %5B%7B%22condition%22%3A%22status%22%2C%22operator%22%3A%22is_in%22%2C%22type%22%3A%22default%22%2C%22value%22%3A%5B2%5D%7D%5D │
│ status     │ status    │ is_in    │ default │ 4                                                                                                                              │
└────────────┴───────────┴──────────┴─────────┴────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

/// `test_filter_wires.test_cli_refusals_send_no_list_request` (a text field
/// of this schema stands in for `Subject`).
#[test]
fn where_refusals_send_no_list_request() {
    for args in [
        ["tickets", "ls", "--where", "Note=Acme"],
        ["changes", "ls", "--where", "created_at>=2030-01-01"],
        ["tickets", "ls", "--where", "Created=on:fortnight"],
    ] {
        let r = run_fixture("where", &[&args[..], &["--json"]].concat(), None);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        let list = format!("/api/_/{}?", args[0]);
        assert!(
            r.calls().iter().all(|(_, p)| !p.starts_with(&list)),
            "{args:?}"
        );
        assert!(r.writes().is_empty(), "{args:?}");
    }
}

/// `test_filter_wires.test_cli_strict_date_shift_is_sent_and_noted`.
#[test]
fn where_strict_date_shift_is_sent_and_noted() {
    let r = run_fixture(
        "where",
        &[
            "changes",
            "ls",
            "--where",
            "created_at>2030-01-31",
            "--where",
            "created_at<2030-03-01",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        sent(&r, "changes").0,
        r#"[{"value":{"from":"2030-02-01","to":"2030-02-28"},"condition":"created_at","operator":"is_in_the_range","type":"default"}]"#
    );
    assert_eq!(
        r.err,
        "note: created_at 2030-01-31 (no strict operator exists: sent as from 2030-02-01, whole days)\n\
         note: created_at 2030-03-01 (no strict operator exists: sent as to 2030-02-28, whole days)\n"
    );
}

/// Python `406ca71`: `--or` on tickets sends `advanced_query_hash` (no
/// workspace is configured or named by this schema, so `all` is empty).
#[test]
fn where_or_sends_advanced_query_hash() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "status=Open",
            "--where",
            "Priority!=Low",
            "--or",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(sent(&r, "tickets").0, "");
    assert_eq!(
        sent_param(&r, "tickets", "advanced_query_hash").0,
        r#"{"any":[{"value":[2],"condition":"status","operator":"is_in","type":"default"},{"value":[1],"condition":"priority","operator":"is_not_in","type":"default"}],"all":[]}"#
    );
}

/// Python `3ad6041` (`test_cli_panel_labels_for_due_closed_resolved_ranges`):
/// panel labels for closed, resolved and first-response-due ranges send the
/// advanced builder's condition names.
#[test]
fn where_closed_resolved_first_response_labels_are_advanced() {
    for (label, cond) in [
        ("Closed", "closed_date"),
        ("Resolved date", "resolved_date"),
        ("First response due", "fr_due_by_date"),
        ("Due By", "due_by_date"),
    ] {
        let lo = format!("{label}>=2030-11-13");
        let hi = format!("{label}<=2030-11-16");
        let r = run_fixture(
            "where",
            &["tickets", "ls", "--where", &lo, "--where", &hi, "--json"],
            None,
        );
        assert_eq!(r.code, 0, "{label}: {}", r.err);
        assert_eq!(sent(&r, "tickets").0, "", "{label}");
        assert_eq!(
            sent_param(&r, "tickets", "advanced_query_hash").0,
            format!(
                r#"{{"all":[{{"value":{{"from":"2030-11-13","to":"2030-11-16"}},"condition":"{cond}","operator":"is_in_the_range","type":"default"}}],"any":[]}}"#
            ),
            "{label}"
        );
    }
}

/// Python `3ad6041` through the CLI on the same schema.
#[test]
fn where_resolved_label_debug() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Resolved date>=2030-11-13",
            "--where",
            "Resolved date<=2030-11-16",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                  query                                   
┏━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━┓
┃ field         ┃ condition     ┃ operator        ┃ type    ┃ value      ┃
┡━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━┩
│ Resolved date │ resolved_date │ is_in_the_range │ default │ 2030-11-13 │
│ Resolved date │ resolved_date │ is_in_the_range │ default │ 2030-11-16 │
└───────────────┴───────────────┴─────────────────┴─────────┴────────────┘
api: internal /api/_/ with advanced_query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, "");
}

/// Python `3ad6041`: ticket-only dates have no panel period and no change builder name.
#[test]
fn where_ticket_only_dates_refuse() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Closed=Today", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        r.err,
        "error: no filter-panel period exists for closed_at; use on:/last:/next: PERIOD or a YYYY-MM-DD range (nothing was sent)\n"
    );
    let r = run_fixture(
        "where",
        &[
            "changes",
            "ls",
            "--where",
            "Closed>=2030-01-05",
            "--where",
            "Closed<=2030-01-06",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        r.err,
        "error: closed_at is not a date in the changes advanced filter builder; nothing was sent\n"
    );
}

/// Python `f081c97` (`test_cli_change_where_approval_status_uses_the_proven_wire`).
#[test]
fn where_change_approval_status_uses_the_proven_wire() {
    for (expr, operator, value) in [
        ("approval_status=0", "is_in", "0"),
        ("approval_status=1", "is_in", "1"),
        ("approval_status=3", "is_in", "3"),
        ("Approval Status!=0", "is_not_in", "0"),
    ] {
        let r = run_fixture(
            "where",
            &["changes", "ls", "--where", expr, "--debug"],
            None,
        );
        assert_eq!(r.code, 0, "{expr}: {}", r.err);
        assert!(
            r.out.contains(&format!(
                "│ approval_status │ approval_status │ {operator} "
            )),
            "{expr}: {}",
            r.out
        );
        assert!(
            r.out.contains(&format!(" │ default │ {value}     │")),
            "{}",
            r.out
        );
        assert_eq!(r.err, "");
    }
}

/// Python `f081c97` (`test_cli_ticket_where_approval_status_names_the_real_reason`).
#[test]
fn where_ticket_approval_status_names_the_real_reason() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "approval_status=1"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        r.err,
        "error: approval_status is not a tickets list filter (the server ignores it and returns unfiltered rows); read approval_status per record instead (e.g. `get`)\n"
    );
    assert!(sent(&r, "tickets").0.is_empty());
}

/// Python `f081c97` (`test_cli_raw_query_hash_bad_shapes_refuse_cleanly`), plus
/// the other `validate_raw_query` refusals; messages from the Python run.
#[test]
fn query_hash_bad_shapes_refuse_cleanly() {
    for (raw, msg) in [
        (
            r#"[{"condition":"status","operator":"is_in","value":2,"type":"default"}]"#,
            r#"--query-hash item 1 (status): operator is_in takes a JSON list value, e.g. "value": [2]; nothing was sent"#,
        ),
        (
            r#"[{"condition":"status","operator":"is_not_in","value":"2","type":"default"}]"#,
            r#"--query-hash item 1 (status): operator is_not_in takes a JSON list value, e.g. "value": ["2"]; nothing was sent"#,
        ),
        (
            "[2]",
            "--query-hash item 1 must be a JSON object with condition/operator/value; nothing was sent",
        ),
        (
            r#"[{"any":[{"condition":"status","operator":"is_in","value":2}]}]"#,
            r#"--query-hash item 1 any item 1 (status): operator is_in takes a JSON list value, e.g. "value": [2]; nothing was sent"#,
        ),
        (
            r#"[{"condition":"approval_status","operator":"is_in","value":["1"],"type":"default"}]"#,
            "approval_status is not a tickets list filter (the server ignores it and returns unfiltered rows); read approval_status per record instead (e.g. `get`)",
        ),
        (
            r#"[{"any":3}]"#,
            "--query-hash item 1: 'any' must be a JSON list of conditions; nothing was sent",
        ),
        (
            r#"[{"operator":"is"}]"#,
            "--query-hash item 1 has no condition; nothing was sent",
        ),
        (
            r#"[{"condition":1,"operator":"is"}]"#,
            "--query-hash item 1: condition and operator must be strings; nothing was sent",
        ),
        (
            r#"[{"condition":"x","operator":"is_in","value":{"a":"é"}}]"#,
            r#"--query-hash item 1 (x): operator is_in takes a JSON list value, e.g. "value": [{"a": "\u00e9"}]; nothing was sent"#,
        ),
    ] {
        let r = run_fixture("where", &["tickets", "ls", "--query-hash", raw], None);
        assert_eq!(r.code, 2, "{raw}: {}", r.err);
        assert_eq!(r.err, format!("error: {msg}\n"), "{raw}");
        assert!(r.calls().is_empty(), "{raw}: {:?}", r.calls());
    }
}

const DUE_BY_NOTE: &str = "Note: Pending tickets have no Due By on this tenant type (the field is hidden and empty while Pending), so a Due By filter never returns them; list them with --where Status=Pending.\n";

/// Python `38ee07d` (`test_due_by_filter_without_status_prints_one_stderr_note`).
#[test]
fn due_by_filter_without_status_prints_one_stderr_note() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "due_by>=2030-01-01",
            "--where",
            "due_by<=2030-02-01",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.err, DUE_BY_NOTE);
    assert!(!r.out.contains("Pending tickets have no Due By"));
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Due By>=2030-01-01",
            "--where",
            "Due By<=2030-02-01",
            "--or",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.err, format!("{DUE_BY_NOTE}2 rows\n"));
}

/// Python `38ee07d` (`test_no_note_with_status_filter_other_fields_or_other_modules`).
#[test]
fn no_due_by_note_with_status_filter_other_fields_or_other_modules() {
    for args in [
        &[
            "tickets",
            "ls",
            "--where",
            "due_by>=2030-01-01",
            "--where",
            "due_by<=2030-02-01",
            "--where",
            "status=Open",
        ][..],
        &[
            "tickets",
            "ls",
            "--where",
            "created_at>=2030-01-01",
            "--where",
            "created_at<=2030-02-01",
        ],
        &[
            "changes",
            "ls",
            "--where",
            "created_at>=2030-01-01",
            "--where",
            "created_at<=2030-02-01",
        ],
    ] {
        let mut argv = args.to_vec();
        argv.push("--json");
        let r = run_fixture("where", &argv, None);
        assert_eq!(r.code, 0, "{args:?}: {}", r.err);
        assert!(
            !r.err.contains("Pending tickets have no Due By"),
            "{args:?}"
        );
    }
}

/// Python `cli._resolve_where`: an `_id` digit string is `int(raw)`, exact
/// past 64 bits and for Unicode decimal digits; a superscript digit that
/// `int()` rejects is the `ValueError` text, exit 2. A `--query-hash` big
/// int is sent as written. Expected values from the Python run.
#[test]
fn where_id_ints_match_python_int() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "group_id=99999999999999999999999",
            "--where",
            "group_id!=\u{661}\u{662}",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        sent(&r, "tickets").0,
        r#"[{"value":[99999999999999999999999],"condition":"group_id","operator":"is_in","type":"default"},{"value":[12],"condition":"group_id","operator":"is_not_in","type":"default"}]"#
    );
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "group_id=\u{b2}"],
        None,
    );
    assert_eq!(
        (r.code, r.out.as_str(), r.err.as_str()),
        (
            2,
            "",
            "error: invalid literal for int() with base 10: '\u{b2}'\n"
        )
    );
    let raw = r#"[{"condition":"group_id","operator":"is_in","type":"default","value":[99999999999999999999999]}]"#;
    let r = run_fixture("where", &["tickets", "ls", "--query-hash", raw], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(sent(&r, "tickets").0, raw);
}
