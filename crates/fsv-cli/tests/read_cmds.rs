//! Read commands against canned routes. Expected output was captured from
//! the Python fsv run on the same routes (`tests/golden/cmp.py --emit`).

mod common;
use common::{run_fixture, run_steps};

#[test]
fn fields_table() {
    let r = run_fixture("change_fields", &["changes", "fields"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━┳━━━━━━━━━┓
┃ name     ┃ label       ┃ type             ┃ scope   ┃ req ┃ choices ┃
┡━━━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━╇━━━━━━━━━┩
│ subject  │ Subject     │ default_subject  │ default │  *  │ -       │
│ status   │ Status      │ default_status   │ default │     │ 3       │
│ priority │ Priority    │ default_priority │ default │     │ 2       │
│ agent    │ Agent       │ default_agent    │ default │     │ -       │
│ group    │ Group       │ default_group    │ default │     │ -       │
│ cf_env   │ Environment │ custom_dropdown  │ custom  │     │ 3       │
└──────────┴─────────────┴──────────────────┴─────────┴─────┴─────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn fields_json_filtered() {
    let r = run_fixture(
        "change_fields",
        &["changes", "fields", "stat", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "name": "status",
    "label": "Status",
    "field_type": "default_status",
    "default_field": true,
    "choices": [
      {
        "id": 1,
        "value": "Open"
      },
      {
        "id": 2,
        "value": "Planning"
      },
      {
        "id": 5,
        "value": "Closed"
      }
    ]
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn fields_choices_unknown() {
    let r = run_fixture(
        "change_fields",
        &["changes", "fields", "--choices", "nope"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: field not found: nope; run `fsv changes fields`
"#
    );
}

#[test]
fn fields_ambiguous() {
    let r = run_fixture(
        "change_fields",
        &["changes", "fields", "--choices", "s"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: ambiguous field 's':
  subject  Subject  default_subject  default
  status  Status  default_status  default
"#
    );
}

#[test]
fn lookup_field_choices() {
    let r = run_fixture("change_fields", &["changes", "lookup", "env", "pr"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━┳━━━━━━━┳━━━━━━━━┓
┃ id ┃ value ┃ detail ┃
┡━━━━╇━━━━━━━╇━━━━━━━━┩
│ 11 │ Prod  │        │
└────┴───────┴────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn lookup_unknown_kind() {
    let r = run_fixture("change_fields", &["changes", "lookup", "nope", "x"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: unknown lookup kind or field: nope
"#
    );
}

#[test]
fn fields_choices_csv() {
    let r = run_fixture(
        "change_fields",
        &["changes", "fields", "--choices", "cf_env", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "id,value,detail\r\n11,Prod,\r\n12,UAT,\r\n13,DEV,\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn conversations_table() {
    let r = run_fixture("ticket_reads", &["tickets", "conversations", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━┳━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━┓
┃ ID  ┃ Created             ┃ Author ┃ Kind         ┃ Body          ┃
┡━━━━━╇━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━┩
│ 101 │ 2026-01-02T03:04:05 │ Ann    │ private note │ internal note │
│ 102 │ 2026-01-03T03:04:05 │ 9      │ incoming     │ Hello  there  │
│ 103 │ 2026-01-04T03:04:05 │ Bob    │ public reply │ reply         │
└─────┴─────────────────────┴────────┴──────────────┴───────────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn conversations_csv() {
    let r = run_fixture(
        "ticket_reads",
        &["tickets", "conversations", "5", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "id,created_at,author,kind,body\r\n101,2026-01-02T03:04:05,Ann,private note,internal note\r\n102,2026-01-03T03:04:05,9,incoming,Hello  there\r\n103,2026-01-04T03:04:05,Bob,public reply,reply\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_notes_table() {
    let r = run_fixture("ticket_reads", &["problems", "notes", "7"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━┳━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━┓
┃ ID  ┃ Created             ┃ Author ┃ Visibility ┃ Body ┃
┡━━━━━╇━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━┩
│ 201 │ 2026-02-02T03:04:05 │ Cy     │ private    │ pn   │
│ 202 │ 2026-02-03T03:04:05 │ 4      │ public     │ pub  │
└─────┴─────────────────────┴────────┴────────────┴──────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_notes_tsv() {
    let r = run_fixture(
        "ticket_reads",
        &["problems", "notes", "7", "-o", "tsv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "id\tcreated_at\tauthor\tvisibility\tbody\r\n201\t2026-02-02T03:04:05\tCy\tprivate\tpn\r\n202\t2026-02-03T03:04:05\t4\tpublic\tpub\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_notes_forbidden() {
    let r = run_fixture("ticket_reads", &["problems", "notes", "8"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"notes: not permitted for your role (403)
"#
    );
}

#[test]
fn ticket_approvals_table() {
    let r = run_fixture("ticket_reads", &["tickets", "approvals", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┓
┃ Level ┃ Approver ┃ Status    ┃ Decided             ┃ Type     ┃ Comment  ┃
┡━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━┩
│ 1     │ Dee      │ approved  │ 2026-03-01T10:00:00 │ everyone │ ok by me │
│ 2     │ 44       │ requested │ 2026-03-02T11:00:00 │ -        │          │
└───────┴──────────┴───────────┴─────────────────────┴──────────┴──────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ticket_approvals_csv() {
    let r = run_fixture(
        "ticket_reads",
        &["tickets", "approvals", "5", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "level,approver,status,decided,type,comment\r\n1,Dee,approved,2026-03-01T10:00:00,everyone,ok by me\r\n2,44,requested,2026-03-02T11:00:00,-,\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ticket_approvals_inc_404() {
    let r = run_fixture("ticket_reads", &["tickets", "approvals", "INC-6"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: ticket approvals not available for this ticket; INC tickets often do not expose approvals, try SR or another approval-capable ticket
"#
    );
}

#[test]
fn ticket_associations_table() {
    let r = run_fixture("ticket_reads", &["tickets", "associations", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                Changes                 
┏━━━━━━━━┳━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━┓
┃ ID     ┃ Subject ┃ Status ┃ Priority ┃
┡━━━━━━━━╇━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━┩
│ CHN-12 │ Patch   │ Open   │ Medium   │
└────────┴─────────┴────────┴──────────┘
          Problems          
┏━━━━━━━┳━━━━━━━━━┳━━━━━━━━┓
┃ ID    ┃ Subject ┃ Status ┃
┡━━━━━━━╇━━━━━━━━━╇━━━━━━━━┩
│ PRB-3 │ Root    │ 1      │
└───────┴─────────┴────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ticket_associations_json() {
    let r = run_fixture(
        "ticket_reads",
        &["tickets", "associations", "5", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "changes": [
    {
      "id": 12,
      "subject": "Patch",
      "status_name": "Open",
      "priority": 2
    }
  ],
  "change_causes": [],
  "problems": [
    {
      "id": 3,
      "human_display_id": "PRB-3",
      "subject": "Root",
      "status": 1
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ticket_associations_none() {
    let r = run_fixture("ticket_reads", &["tickets", "associations", "6"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"no associations
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn download_ticket_main() {
    let r = run_fixture("downloads", &["tickets", "download", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"      Downloaded attachments for INC-5       
┏━━━━━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━┓
┃ Status     ┃ Source      ┃ File    ┃ Size ┃
┡━━━━━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━┩
│ downloaded │ attachments │ log.txt │    7 │
└────────────┴─────────────┴─────────┴──────┘
output downloads/INC-5
"#
    );
    assert_eq!(
        r.err,
        r#"output directory: downloads/INC-5 (default ./downloads/<ID>; choose with --dir)
  downloading log.txt...
"#
    );
}

#[test]
fn download_ticket_all_json() {
    let r = run_fixture(
        "downloads",
        &["tickets", "download", "5", "--all", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "name": "log.txt",
    "path": "downloads/INC-5/log.txt",
    "size": 7,
    "skipped": false,
    "source": "attachments",
    "status": "downloaded"
  },
  {
    "name": "reply.png",
    "path": "downloads/INC-5/reply.png",
    "size": 4,
    "skipped": false,
    "source": "conversations",
    "status": "downloaded"
  },
  {
    "name": "attachment-93",
    "path": "downloads/INC-5/attachment-93",
    "size": 5,
    "skipped": false,
    "source": "description",
    "status": "downloaded"
  },
  {
    "name": "attachment-94",
    "path": "downloads/INC-5/attachment-94",
    "size": 5,
    "skipped": false,
    "source": "description",
    "status": "downloaded"
  }
]
"#
    );
    assert_eq!(
        r.err,
        r#"output directory: downloads/INC-5 (default ./downloads/<ID>; choose with --dir)
  downloading log.txt...
  downloading reply.png...
  downloading attachment-93...
  downloading attachment-94...
"#
    );
}

#[test]
fn download_ticket_threads_dir() {
    let r = run_fixture(
        "downloads",
        &[
            "tickets",
            "download",
            "5",
            "--conversations",
            "--dir",
            "./out/",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"        Downloaded attachments for INC-5         
┏━━━━━━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━┓
┃ Status     ┃ Source        ┃ File      ┃ Size ┃
┡━━━━━━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━┩
│ downloaded │ conversations │ reply.png │    4 │
└────────────┴───────────────┴───────────┴──────┘
output out
"#
    );
    assert_eq!(
        r.err,
        r#"output directory: out (--dir)
  downloading reply.png...
"#
    );
}

#[test]
fn download_problem_api_error() {
    let r = run_fixture("downloads", &["problems", "download", "9"], None);
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"output directory: downloads/PRB-9 (default ./downloads/<ID>; choose with --dir)
error: HTTP 500 fetching problem: boom
"#
    );
}

#[test]
fn download_change_planning() {
    let r = run_fixture("downloads", &["changes", "download", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"            Downloaded attachments for CHN-5            
┏━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━┓
┃ Status     ┃ Source                ┃ File     ┃ Size ┃
┡━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━┩
│ downloaded │ planning:rollout_plan │ plan.pdf │    9 │
└────────────┴───────────────────────┴──────────┴──────┘
output downloads/CHN-5
"#
    );
    assert_eq!(
        r.err,
        r#"output directory: downloads/CHN-5 (default ./downloads/<ID>; choose with --dir)
  downloading plan.pdf...
"#
    );
}

#[test]
fn download_change_all_json() {
    let r = run_fixture(
        "downloads",
        &["changes", "download", "5", "--all", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "name": "plan.pdf",
    "path": "downloads/CHN-5/plan.pdf",
    "size": 9,
    "skipped": false,
    "source": "planning:rollout_plan",
    "status": "downloaded"
  },
  {
    "name": "log.txt",
    "path": "downloads/CHN-5/log.txt",
    "size": 7,
    "skipped": false,
    "source": "attachments",
    "status": "downloaded"
  },
  {
    "name": "attachment-93",
    "path": "downloads/CHN-5/attachment-93",
    "size": 5,
    "skipped": false,
    "source": "description",
    "status": "downloaded"
  }
]
"#
    );
    assert_eq!(
        r.err,
        r#"output directory: downloads/CHN-5 (default ./downloads/<ID>; choose with --dir)
  downloading plan.pdf...
  downloading log.txt...
  downloading attachment-93...
"#
    );
}

#[test]
fn download_change_named_planning() {
    let r = run_fixture(
        "downloads",
        &[
            "changes",
            "download",
            "5",
            "--planning",
            "Backout plan",
            "--planning",
            "rollout",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"output directory: downloads/CHN-5 (default ./downloads/<ID>; choose with --dir)
error: planning field not found: 'rollout'; choices: Rollout plan, Backout plan
"#
    );
}

#[test]
fn download_change_unknown_planning() {
    let r = run_fixture(
        "downloads",
        &["changes", "download", "5", "--planning", "nope"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"output directory: downloads/CHN-5 (default ./downloads/<ID>; choose with --dir)
error: planning field not found: 'nope'; choices: Rollout plan, Backout plan
"#
    );
}

#[test]
fn change_approvals_table() {
    let r = run_fixture("change_helpers", &["changes", "approvals", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━━━━┓
┃ Level ┃ Approver ┃ Status        ┃ Decided    ┃ Message ┃
┡━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━━━━┩
│ 1     │ Ann Lee  │ approved      │ 2026-02-03 │ ok <b>  │
│ 2     │ 33       │ peer approved │ -          │         │
└───────┴──────────┴───────────────┴────────────┴─────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_approvals_csv() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "approvals", "5", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "level,approver,status,decided,message\r\n1,Ann Lee,approved,2026-02-03,ok <b>\r\n2,33,peer approved,-,\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_approvals_json() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "approvals", "5", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 70,
    "level_id": 1,
    "member_id": 31,
    "member": {
      "name": "Ann Lee"
    },
    "status": {
      "name": "approved"
    },
    "remark": [
      {
        "updated_at": "2026-02-03T04:05:06Z",
        "data": "ok <b>"
      }
    ]
  },
  {
    "id": 71,
    "level_id": 2,
    "member_id": 33,
    "status": {
      "name": "peer_responded"
    }
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_approvals_api_error() {
    let r = run_fixture("change_helpers", &["changes", "approvals", "6"], None);
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 500 listing approvals: boom
"#
    );
}

#[test]
fn change_approval_groups_table() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "approvals", "5", "--groups"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━━━━━━━┓
┃ Group ID ┃ Raw status ┃ Member IDs ┃
┡━━━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━━━━━━━┩
│ 3        │ 0          │ 31, 34     │
└──────────┴────────────┴────────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_approval_groups_tsv() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "approvals", "5", "--groups", "-o", "tsv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, "group_id\tstatus\tmember_ids\r\n3\t0\t31, 34\r\n");
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_approval_groups_unreadable() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "approvals", "6", "--groups"],
        None,
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: approval chains inventory is unreadable
"#
    );
}

#[test]
fn change_state_table() {
    let r = run_fixture("change_helpers", &["changes", "state", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"          CHN-5 flow — Standard (change_type=1)          
┏━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃  →  ┃ Status   ┃  State  ┃ Next (allowed transitions) ┃
┡━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│  ✓  │ Open     │ passed  │ Planning                   │
│  ▶  │ Planning │ current │ Done                       │
│  ·  │ Done     │ future  │ -                          │
└─────┴──────────┴─────────┴────────────────────────────┘
  current: Planning · past: 1 · ahead: 1

Transitions from Planning:
  → Done: no entry criteria

Approval gates:
  ✓ L1: Ann Lee — approved
  ◷ L2: 33 — peer_responded
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_state_json() {
    let r = run_fixture("change_helpers", &["changes", "state", "5", "--json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "change": {
    "id": 5,
    "human_display_id": "CHN-5",
    "status": 2,
    "status_name": "Planning",
    "state_flow_id": 11,
    "workspace_id": 2,
    "change_type": 1,
    "state_traversal": [
      1,
      1
    ]
  },
  "flow": {
    "name": "Standard",
    "state_list": [
      {
        "id": 1,
        "value": "Open"
      },
      {
        "id": 2,
        "value": "Planning"
      },
      {
        "id": 3,
        "value": "Done"
      }
    ],
    "transitions": {
      "1": [
        {
          "to_state": 2
        }
      ],
      "2": [
        {
          "to_state": 3
        }
      ],
      "3": []
    }
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_state_no_flow() {
    let r = run_fixture("change_helpers", &["changes", "state", "6"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"no state_flow_id on this change
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_state_no_flow_json() {
    let r = run_fixture("change_helpers", &["changes", "state", "6", "--json"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no state_flow_id on this change
"#
    );
}

#[test]
fn change_state_csv() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "state", "5", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "id,human_display_id,status,status_name,state_flow_id,workspace_id,change_type,state_traversal\r\n5,CHN-5,2,Planning,11,2,1,\"[1, 1]\"\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn associations_ls_table() {
    let r = run_fixture("links", &["changes", "associations", "ls", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"            Tickets            
┏━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━┓
┃ ID     ┃ Subject   ┃ Status ┃
┡━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━┩
│ SR-100 │ Disk full │ Open   │
└────────┴───────────┴────────┘
        Problems         
┏━━━━┳━━━━━━━━━┳━━━━━━━━┓
┃ ID ┃ Subject ┃ Status ┃
┡━━━━╇━━━━━━━━━╇━━━━━━━━┩
│ 7  │ Root    │ 1      │
└────┴─────────┴────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"releases: incomplete (0 loaded; releases: not permitted for your role (403))
"#
    );
}

#[test]
fn associations_ls_json() {
    let r = run_fixture(
        "links",
        &["changes", "associations", "ls", "5", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "meta": {
    "tickets": {
      "total_count": 1,
      "loaded_count": 1,
      "pages": 1,
      "complete": true,
      "has_more": false,
      "has_next": false,
      "continuation": null
    },
    "problems": {
      "total_count": 1,
      "loaded_count": 1,
      "pages": 1,
      "complete": true,
      "has_more": false,
      "has_next": false,
      "continuation": null
    },
    "releases": {
      "loaded_count": 0,
      "pages": 0,
      "complete": false,
      "has_more": true,
      "has_next": true,
      "continuation": {
        "per_page": 50,
        "page": 1
      },
      "partial": "releases: incomplete (0 loaded; releases: not permitted for your role (403))"
    }
  },
  "warnings": [
    "releases: incomplete (0 loaded; releases: not permitted for your role (403))"
  ],
  "tickets": [
    {
      "id": 100,
      "human_display_id": "SR-100",
      "subject": "Disk full",
      "status_name": "Open"
    }
  ],
  "problems": [
    {
      "id": 7,
      "subject": "Root",
      "status": 1
    }
  ],
  "releases": [],
  "complete": false
}
"#
    );
    assert_eq!(
        r.err,
        r#"releases: incomplete (0 loaded; releases: not permitted for your role (403))
"#
    );
}

#[test]
fn associations_ls_csv() {
    let r = run_fixture(
        "links",
        &["changes", "associations", "ls", "5", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "type,id,subject,status\r\ntickets,SR-100,Disk full,Open\r\nproblems,7,Root,1\r\n"
    );
    assert_eq!(
        r.err,
        r#"releases: incomplete (0 loaded; releases: not permitted for your role (403))
"#
    );
}

#[test]
fn associations_ls_none() {
    let r = run_fixture("links", &["changes", "associations", "ls", "6"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"no associations
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn associations_search_table() {
    let r = run_fixture(
        "links",
        &["changes", "associations", "search", "5", "disk"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"         Ticket search          
┏━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━┓
┃ ID      ┃ Subject   ┃ Status ┃
┡━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━┩
│ SR-100  │ Disk full │ Open   │
│ INC-101 │ Disk slow │ 2      │
└─────────┴───────────┴────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn associations_search_tsv() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "search",
            "5",
            "disk",
            "-o",
            "tsv",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "id\tsubject\tstatus\r\nSR-100\tDisk full\tOpen\r\nINC-101\tDisk slow\t2\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn activity_table() {
    let r = run_fixture("tabs", &["tickets", "activity", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"
2024-03-01
03:15  Ann [x]  Set status to Open
      · a b
      · c
23:30  ?  second

Unknown
--:--  Bob  
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn activity_limit() {
    let r = run_fixture("tabs", &["tickets", "activity", "5", "--limit", "2"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"
2024-03-01
03:15  Ann [x]  Set status to Open
      · a b
      · c
23:30  ?  second

truncated: 1 more (showing 2 of 3); use --limit N or --all
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn activity_negative_limit_json() {
    let r = run_fixture(
        "tabs",
        &["tickets", "activity", "5", "--limit", "-1", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, "[]\n");
    assert_eq!(
        r.err,
        "truncated: 3 more (showing 0 of 3); use --limit N or --all\n"
    );
}

#[test]
fn activity_all_overrides_limit() {
    let r = run_fixture(
        "tabs",
        &["tickets", "activity", "5", "--limit", "1", "--all"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"
2024-03-01
03:15  Ann [x]  Set status to Open
      · a b
      · c
23:30  ?  second

Unknown
--:--  Bob  
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn activity_csv_limit_notice() {
    let r = run_fixture(
        "tabs",
        &["tickets", "activity", "5", "--limit", "2", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "day,time,actor,content\r\n2024-03-01,03:15,Ann [x],Set status to Open\r\n2024-03-01,23:30,?,second\r\n"
    );
    assert_eq!(
        r.err,
        "truncated: 1 more (showing 2 of 3); use --limit N or --all\n"
    );
}

#[test]
fn activity_json_limit_notice() {
    let r = run_fixture(
        "tabs",
        &["tickets", "activity", "5", "--limit", "2", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "created_at": "2024-03-01T10:15:00+07:00",
    "actor": {
      "name": "Ann [x]"
    },
    "content": "<b>Set</b>  status\n to Open",
    "sub_contents": [
      "<i>a</i>  b",
      "c"
    ]
  },
  {
    "created_at": "2024-03-01T23:30:00Z",
    "actor": null,
    "content": "second"
  }
]
"#
    );
    assert_eq!(
        r.err,
        "truncated: 1 more (showing 2 of 3); use --limit N or --all\n"
    );
}

#[test]
fn activity_csv() {
    let r = run_fixture("tabs", &["tickets", "activity", "5", "-o", "csv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "day,time,actor,content\r\n2024-03-01,03:15,Ann [x],Set status to Open\r\n2024-03-01,23:30,?,second\r\nUnknown,--:--,Bob,\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn activity_forbidden() {
    let r = run_fixture("tabs", &["tickets", "activity", "6"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Activity: not permitted for your role (403)
"#
    );
}

#[test]
fn tasks_ticket_table() {
    let r = run_fixture("tabs", &["tickets", "tasks-legacy", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━┳━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ TSK   ┃ Status ┃ Group/Agent ┃ Due        ┃ Title                                                        ┃
┡━━━━━━━╇━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ TSK-1 │ Open   │ Ops         │ 2024-01-01 │ A very long title that goes on and on and on past sixty char │
│ 2     │ 2      │ Ann         │ -          │                                                              │
└───────┴────────┴─────────────┴────────────┴──────────────────────────────────────────────────────────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tasks_ticket_tsv() {
    let r = run_fixture("tabs", &["tickets", "tasks-legacy", "5", "-o", "tsv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "id\tstatus\tgroup_agent\tdue\ttitle\r\nTSK-1\tOpen\tOps\t2024-01-01\tA very long title that goes on and on and on past sixty char\r\n2\t2\tAnn\t-\t\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tasks_change_table() {
    let r = run_fixture("tabs", &["changes", "tasks", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━┳━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━━━┳━━━━━━━┓
┃ TSK ┃ Status ┃ Group/Agent ┃ Due     ┃ System/Env ┃ Title ┃
┡━━━━━╇━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━━━╇━━━━━━━┩
│ 3   │ Done   │ -           │ overdue │ SAP / -    │ t     │
└─────┴────────┴─────────────┴─────────┴────────────┴───────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tasks_change_json() {
    let r = run_fixture("tabs", &["changes", "tasks", "5", "--json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 3,
    "status_name": "Done",
    "custom_fields": {
      "system": "SAP",
      "environment": null
    },
    "task_due_status": "overdue",
    "title": "t"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tasks_change_csv() {
    let r = run_fixture("tabs", &["changes", "tasks", "5", "-o", "csv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "id,status,group_agent,due,system_env,title\r\n3,Done,-,overdue,SAP / -,t\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tasks_change_missing() {
    let r = run_fixture("tabs", &["changes", "tasks", "6"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Tasks: not available on this account (404)
"#
    );
}

#[test]
fn get_ticket_page() {
    let r = run_fixture("get", &["tickets", "get", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"#SR-5  Printer [down]
Open
Ann Lee reported 

 Details   Tasks  Activity
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
Requested items                                                                                                         Status    Open                                                                  
  Requested item   Stage: Requested                                                                                     Priority  Low                                                                   
                                                                                                                                                                                                        
Description                                                                                                             Properties                                                                      
  It is broken                                                                                                            no fields                                                                     
                                                                                                                                                                                                        
Attachments                                                                                                                                                                                             
  📎 a.txt (2.0 KB)                                                                                                                                                                                     
                                                                                                                                                                                                        
Conversations (1)                                                                                                                                                                                       
  Bob replied · public ·  ()                                                                                                                                                                            
    hi                                                                                                                                                                                                  
                                                                                                                                                                                                        

· static field visibility (business rules: not available on this account (404))
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_ticket_json() {
    let r = run_fixture("get", &["tickets", "get", "5", "--json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 5,
  "display_id": 5,
  "subject": "Printer [down]",
  "status": 2,
  "priority": 1,
  "group_id": 7,
  "human_display_id": "SR-5",
  "description": "<p>It is <b>broken</b></p>",
  "requester": {
    "name": "Ann Lee",
    "email": "ann@acme.com"
  },
  "cf_env": 11,
  "attachments": [
    {
      "id": 1,
      "name": "a.txt",
      "size": 2048
    }
  ],
  "requested_items": [
    {
      "id": 3,
      "service_item": {
        "name": "Laptop"
      },
      "quantity": 1,
      "stage": "Requested"
    }
  ],
  "view": {
    "id": "SR-5",
    "subject": "Printer [down]",
    "status": "Open",
    "due_by": null,
    "reporter": {
      "name": "Ann Lee",
      "created_at": null,
      "relative": "",
      "when": "",
      "via": ""
    },
    "tabs": [
      {
        "name": "details",
        "label": "Details",
        "count": null
      },
      {
        "name": "tasks",
        "label": "Tasks",
        "count": null
      },
      {
        "name": "activity",
        "label": "Activity",
        "count": null
      }
    ],
    "lifecycle": null,
    "details": {
      "description": "It is broken",
      "attachments": [
        {
          "name": "a.txt",
          "size": 2048
        }
      ],
      "requested_items": [
        {
          "name": "",
          "stage": "Requested",
          "quantity": 1,
          "answers": []
        }
      ],
      "conversations": [
        {
          "id": 1,
          "author": "Bob",
          "kind": "replied",
          "private": false,
          "created_at": null,
          "relative": "",
          "body": "hi",
          "attachments": []
        }
      ],
      "conversations_total": 1
    },
    "summary": [
      {
        "label": "Status",
        "value": "Open"
      },
      {
        "label": "Priority",
        "value": "Low"
      }
    ],
    "properties": [],
    "notes": [
      "static field visibility (business rules: not available on this account (404))"
    ]
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_ticket_csv() {
    let r = run_fixture("get", &["tickets", "get", "5", "-o", "csv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "section,label,value\r\nsummary,Status,Open\r\nsummary,Priority,Low\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_ticket_tsv() {
    let r = run_fixture("get", &["tickets", "get", "5", "-o", "tsv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "section\tlabel\tvalue\r\nsummary\tStatus\tOpen\r\nsummary\tPriority\tLow\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_ticket_tab() {
    let r = run_fixture("get", &["tickets", "get", "5", "--tab", "tasks"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"#SR-5  Printer [down]
Open
Ann Lee reported 

Details   Tasks   Activity
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
○ Check #1
    Open

· static field visibility (business rules: not available on this account (404))
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_ticket_bad_tab() {
    let r = run_fixture("get", &["tickets", "get", "5", "--tab", "nope"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: tab 'nope' is not shown for SR-5; tabs: details, tasks, activity
"#
    );
}

/// `get_resource`: `elif tab:` / `if tab else None`, so `--tab ""` is no tab.
#[test]
fn get_ticket_empty_tab_is_no_tab() {
    let r = run_fixture("get", &["tickets", "get", "5", "--tab", ""], None);
    let plain = run_fixture("get", &["tickets", "get", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, plain.out);
    assert_eq!(r.err, plain.err);
}

#[test]
fn get_ticket_all_tabs() {
    let r = run_fixture("get", &["tickets", "get", "5", "--all-tabs"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"#SR-5  Printer [down]
Open
Ann Lee reported 

 Details   Tasks  Activity
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
Requested items                                                                                                         Status    Open                                                                  
  Requested item   Stage: Requested                                                                                     Priority  Low                                                                   
                                                                                                                                                                                                        
Description                                                                                                             Properties                                                                      
  It is broken                                                                                                            no fields                                                                     
                                                                                                                                                                                                        
Attachments                                                                                                                                                                                             
  📎 a.txt (2.0 KB)                                                                                                                                                                                     
                                                                                                                                                                                                        
Conversations (1)                                                                                                                                                                                       
  Bob replied · public ·  ()                                                                                                                                                                            
    hi                                                                                                                                                                                                  
                                                                                                                                                                                                        

Tasks ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
○ Check #1
    Open

Activity ───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
Fri, 1 Mar 2024
  17:15  Ann created

· static field visibility (business rules: not available on this account (404))
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_ticket_not_found() {
    let r = run_fixture("get", &["tickets", "get", "9"], None);
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 404 fetching ticket: not found
"#
    );
}

#[test]
fn get_change_lifecycle() {
    let r = run_fixture("change_helpers", &["changes", "get", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"#CHN-5  
Planning
✓ Open ›  Planning  › Done

 Details   Notes  Tasks  Approvals  Activity
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
Description                                                                                                             Status  Planning                                                                
  No description                                                                                                                                                                                        
                                                                                                                        Properties                                                                      
planning: not available on this account (404)                                                                             no fields                                                                     

· tabs: not available on this account (404)
· static field visibility (business rules: not available on this account (404))
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_change_json() {
    let r = run_fixture("change_helpers", &["changes", "get", "7", "--json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 7,
  "human_display_id": "CHN-7",
  "subject": "Patch",
  "status": 1,
  "priority": 2,
  "description": "<p>x</p>",
  "created_at": "2026",
  "custom_fields": {
    "cf_env": "prod"
  },
  "attachments": [
    {
      "id": 1,
      "name": "a.txt"
    }
  ],
  "rollout_plan_planning_fields": {},
  "view": {
    "id": "CHN-7",
    "subject": "Patch",
    "status": "1",
    "due_by": null,
    "reporter": {
      "name": "",
      "created_at": "2026",
      "relative": "",
      "when": "2026"
    },
    "tabs": [
      {
        "name": "details",
        "label": "Details",
        "count": null
      },
      {
        "name": "notes",
        "label": "Notes",
        "count": null
      },
      {
        "name": "tasks",
        "label": "Tasks",
        "count": null
      },
      {
        "name": "approvals",
        "label": "Approvals",
        "count": null
      },
      {
        "name": "activity",
        "label": "Activity",
        "count": null
      }
    ],
    "lifecycle": null,
    "details": {
      "description": "x",
      "attachments": [
        {
          "name": "a.txt",
          "size": null
        }
      ],
      "planning": [
        {
          "name": "rollout_plan",
          "label": "Rollout plan",
          "help": "",
          "content": "do",
          "files": [
            "9"
          ],
          "empty": false
        },
        {
          "name": "backout_plan",
          "label": "Backout plan",
          "help": "",
          "content": "",
          "files": [],
          "empty": true
        }
      ]
    },
    "summary": [
      {
        "label": "Status",
        "value": "1"
      },
      {
        "label": "Priority",
        "value": "Medium"
      }
    ],
    "properties": [],
    "notes": [
      "tabs: not available on this account (404)",
      "static field visibility (business rules: not available on this account (404))"
    ]
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_change_stats_json() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "get", "7", "--stats", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 7,
  "human_display_id": "CHN-7",
  "subject": "Patch",
  "status": 1,
  "priority": 2,
  "description": "<p>x</p>",
  "created_at": "2026",
  "custom_fields": {
    "cf_env": "prod"
  },
  "attachments": [
    {
      "id": 1,
      "name": "a.txt"
    }
  ],
  "rollout_plan_planning_fields": {},
  "planning_fields": {
    "rollout_plan": {
      "name": "rollout_plan",
      "description": "<p>do</p>",
      "attachments": [
        {
          "id": 9
        }
      ]
    },
    "backout_plan": {
      "name": "backout_plan",
      "description": ""
    }
  },
  "change_planning_fields": [
    {
      "name": "rollout_plan",
      "description": "<p>do</p>",
      "attachments": [
        {
          "id": 9
        }
      ]
    },
    {
      "name": "backout_plan",
      "description": ""
    }
  ],
  "planning_field_definitions": [
    {
      "name": "rollout_plan",
      "label": "Rollout plan",
      "field_type": "default_change_plan",
      "default_field": true
    },
    {
      "name": "backout_plan",
      "label": "Backout plan",
      "field_type": "default_backout_plan",
      "default_field": true
    }
  ],
  "main_attachments": [
    {
      "id": 1,
      "name": "a.txt"
    }
  ],
  "description_attachment_urls": []
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_change_csv() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "get", "7", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "section,label,value\r\nsummary,Status,1\r\nsummary,Priority,Medium\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn get_change_page() {
    let r = run_fixture("change_helpers", &["changes", "get", "7"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"#CHN-7  Patch
1
Someone reported  (2026)

 Details   Notes  Tasks  Approvals  Activity
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
Description                                                                                                             Status    1                                                                     
  x                                                                                                                     Priority  Medium                                                                
                                                                                                                                                                                                        
Attachments                                                                                                             Properties                                                                      
  📎 a.txt                                                                                                                no fields                                                                     
                                                                                                                                                                                                        
Planning                                                                                                                                                                                                
  Rollout plan                                                                                                                                                                                          
    do                                                                                                                                                                                                  
    📎 9                                                                                                                                                                                                
  Backout plan                                                                                                                                                                                          
    empty                                                                                                                                                                                               

· tabs: not available on this account (404)
· static field visibility (business rules: not available on this account (404))
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ls_default_view() {
    let r = run_fixture("ls", &["tickets", "ls"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Open in my groups · 1-2 of 3 · Sort by: Created Date
┏━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━┓
┃Subject    ┃ Requester ┃ State ┃ Status   ┃ Priority ┃ Assigned to┃
┡━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━┩
│Item 1     │ Ann       │ --    │ Open     │ Low      │ None       │
│#INC-1     │           │       │          │          │            │
│Second [x] │ Ann       │ --    │ Resolved │ Low      │ None       │
│#INC-2     │           │       │          │          │            │
└───────────┴───────────┴───────┴──────────┴──────────┴────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"2 rows
"#
    );
}

#[test]
fn ls_json() {
    let r = run_fixture("ls", &["tickets", "ls", "--json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 1,
    "display_id": 1,
    "subject": "Item 1",
    "status": 2,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  },
  {
    "id": 2,
    "display_id": 2,
    "subject": "Second [x]",
    "status": 4,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ls_csv() {
    let r = run_fixture("ls", &["tickets", "ls", "-o", "csv"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "id,subject,requester,state,status,priority,assigned_to\r\nINC-1,Item 1,Ann,,Open,Low,None\r\nINC-2,Second [x],Ann,,Resolved,Low,None\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ls_saved_view_settings() {
    let r = run_fixture("ls", &["tickets", "ls", "--view", "My Saved"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"My Saved · 1-2 of 3 · Sort by: Priority
┏━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━┓
┃Subject    ┃ Requester ┃ State ┃ Status   ┃ Priority ┃ Assigned to┃
┡━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━┩
│Item 1     │ Ann       │ --    │ Open     │ Low      │ None       │
│#INC-1     │           │       │          │          │            │
│Second [x] │ Ann       │ --    │ Resolved │ Low      │ None       │
│#INC-2     │           │       │          │          │            │
└───────────┴───────────┴───────┴──────────┴──────────┴────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"2 rows
"#
    );
}

#[test]
fn ls_unknown_view() {
    let r = run_fixture("ls", &["tickets", "ls", "--view", "nope"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no view named 'nope'; views: 'Open in my groups', 'My Saved'
"#
    );
}

#[test]
fn ls_all_pages() {
    let r = run_fixture("ls", &["tickets", "ls", "--per-page", "2", "-a"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Open in my groups · 1-2 of 3 · Sort by: Created Date
┏━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━┓
┃Subject    ┃ Requester ┃ State ┃ Status   ┃ Priority ┃ Assigned to┃
┡━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━┩
│Item 1     │ Ann       │ --    │ Open     │ Low      │ None       │
│#INC-1     │           │       │          │          │            │
│Second [x] │ Ann       │ --    │ Resolved │ Low      │ None       │
│#INC-2     │           │       │          │          │            │
└───────────┴───────────┴───────┴──────────┴──────────┴────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"  fetched 2 so far...
2 rows
"#
    );
}

#[test]
fn ls_npages() {
    let r = run_fixture("ls", &["tickets", "ls", "--npages", "1"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Open in my groups · 1-2 of 3 · Sort by: Created Date
┏━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━┓
┃Subject    ┃ Requester ┃ State ┃ Status   ┃ Priority ┃ Assigned to┃
┡━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━┩
│Item 1     │ Ann       │ --    │ Open     │ Low      │ None       │
│#INC-1     │           │       │          │          │            │
│Second [x] │ Ann       │ --    │ Resolved │ Low      │ None       │
│#INC-2     │           │       │          │          │            │
└───────────┴───────────┴───────┴──────────┴──────────┴────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"  fetched 2 so far...
2 rows
"#
    );
}

#[test]
fn ls_debug_merged_in() {
    let r = run_fixture(
        "ls",
        &[
            "tickets",
            "ls",
            "--where",
            "Status=Open",
            "--where",
            "Status=Resolved",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                       query                       
┏━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field  ┃ condition ┃ operator ┃ type    ┃ value ┃
┡━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ status │ status    │ is_in    │ default │ 2     │
│ status │ status    │ is_in    │ default │ 4     │
└────────┴───────────┴──────────┴─────────┴───────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ls_where_filtered() {
    let r = run_fixture("ls", &["tickets", "ls", "--where", "Status=Open"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"All tickets · 1-2 of 3 · Sort by: Created Date · filtered: Status=Open
┏━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━┓
┃Subject    ┃ Requester ┃ State ┃ Status   ┃ Priority ┃ Assigned to┃
┡━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━┩
│Item 1     │ Ann       │ --    │ Open     │ Low      │ None       │
│#INC-1     │           │       │          │          │            │
│Second [x] │ Ann       │ --    │ Resolved │ Low      │ None       │
│#INC-2     │           │       │          │          │            │
└───────────┴───────────┴───────┴──────────┴──────────┴────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"Warning: returned records contradict the requested predicate; do not use this list for writes. IDs: 2
2 rows
"#
    );
}

/// Python `406ca71`: `--or` works on tickets too; with no filters it is the
/// plain default-view list.
#[test]
fn ls_or_tickets_without_filters() {
    let r = run_fixture("ls", &["tickets", "ls", "--or"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Open in my groups · 1-2 of 3 · Sort by: Created Date
┏━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━┓
┃Subject    ┃ Requester ┃ State ┃ Status   ┃ Priority ┃ Assigned to┃
┡━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━┩
│Item 1     │ Ann       │ --    │ Open     │ Low      │ None       │
│#INC-1     │           │       │          │          │            │
│Second [x] │ Ann       │ --    │ Resolved │ Low      │ None       │
│#INC-2     │           │       │          │          │            │
└───────────┴───────────┴───────┴──────────┴──────────┴────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"2 rows
"#
    );
}

#[test]
fn ls_changes_no_views() {
    let r = run_fixture("ls", &["changes", "ls"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"All changes · 1-1 of 1 · Sort by: Created Date
┏━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━━━┓
┃Subject ┃ Requester ┃ Status ┃ Priority ┃ Assigned to ┃ Department┃
┡━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━━━┩
│Chg     │ --        │ Open   │ --       │ None        │ --        │
│#CHN-4  │           │        │          │             │           │
└────────┴───────────┴────────┴──────────┴─────────────┴───────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"1 rows
"#
    );
}

#[test]
fn ls_changes_view_passthrough() {
    let r = run_fixture("ls", &["changes", "ls", "--view", "x"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"x · 1-1 of 1 · Sort by: Created Date
┏━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━━━┓
┃Subject ┃ Requester ┃ Status ┃ Priority ┃ Assigned to ┃ Department┃
┡━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━━━┩
│Chg     │ --        │ Open   │ --       │ None        │ --        │
│#CHN-4  │           │        │          │             │           │
└────────┴───────────┴────────┴──────────┴─────────────┴───────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"1 rows
"#
    );
}

#[test]
fn ls_changes_or_debug() {
    let r = run_fixture(
        "ls",
        &["changes", "ls", "--or", "--where", "Status=Open", "--debug"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                       query                       
┏━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━┓
┃ field  ┃ condition ┃ operator ┃ type    ┃ value ┃
┡━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━┩
│ status │ status    │ is_in    │ default │ 1     │
└────────┴───────────┴──────────┴─────────┴───────┘
api: internal /api/_/ with advanced_query_hash (1 conditions, OR)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ls_raw_query_debug() {
    let r = run_fixture(
        "ls",
        &[
            "tickets",
            "ls",
            "--query-hash",
            "[{\"condition\":\"status\",\"operator\":\"is_in\",\"type\":\"default\",\"value\":[2]}]",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                         query                                                         
┏━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ field      ┃ condition ┃ operator ┃ type ┃ value                                                                    ┃
┡━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ query_hash │ raw       │ raw      │ raw  │ [{"condition":"status","operator":"is_in","type":"default","value":[2]}] │
└────────────┴───────────┴──────────┴──────┴──────────────────────────────────────────────────────────────────────────┘
api: internal /api/_/ with query_hash (1 conditions, AND)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_catalog_items() {
    let r = run_fixture("catalog", &["sr", "catalog"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                 Service catalog                                  
┏━━━━┳━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ ID ┃ Item            ┃ Category          ┃ Short description                   ┃
┡━━━━╇━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ 78 │ VPN Access      │ Access            │ Remote access to the office network │
│ 77 │ Feature Request │ Engineering       │ Ask for a product change            │
│ 80 │ Build Agent     │ Engineering Tools │ A CI build agent                    │
│ 79 │ Laptop          │ Hardware          │ A new or replacement laptop         │
└────┴─────────────────┴───────────────────┴─────────────────────────────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"4 item(s); see one with: fsv sr fields ID
"#
    );
}

#[test]
fn sr_catalog_categories() {
    let r = run_fixture("catalog", &["sr", "catalog", "--categories"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"              Service catalog categories               
┏━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ ID   ┃ Category          ┃ Description              ┃
┡━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ 7001 │ Engineering       │ Product work             │
│ 7004 │ Engineering Tools │ Build tools              │
│ 7002 │ Access            │ Accounts and permissions │
│ 7003 │ Hardware          │ Devices                  │
└──────┴───────────────────┴──────────────────────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_catalog_categories_json() {
    let r = run_fixture(
        "catalog",
        &["sr", "catalog", "--categories", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 7001,
    "name": "Engineering",
    "description": "Product work",
    "position": 1,
    "workspace_id": 3,
    "hierarchy_info": {
      "has_sub_category": true,
      "order": "az"
    }
  },
  {
    "id": 7004,
    "name": "Engineering Tools",
    "description": "Build tools",
    "position": 2,
    "workspace_id": 3,
    "parent_id": 7001,
    "hierarchy_info": {
      "has_sub_category": null,
      "order": null
    }
  },
  {
    "id": 7002,
    "name": "Access",
    "description": "Accounts and permissions",
    "position": 3,
    "workspace_id": 3,
    "hierarchy_info": {
      "has_sub_category": null,
      "order": "az"
    }
  },
  {
    "id": 7003,
    "name": "Hardware",
    "description": "Devices",
    "position": 4,
    "workspace_id": 3,
    "hierarchy_info": {
      "has_sub_category": null,
      "order": null
    }
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_catalog_popular() {
    let r = run_fixture("catalog", &["sr", "catalog", "--popular"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                   Service catalog (frequently requested)                   
┏━━━━┳━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ ID ┃ Item            ┃ Category    ┃ Short description                   ┃
┡━━━━╇━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ 78 │ VPN Access      │ Access      │ Remote access to the office network │
│ 77 │ Feature Request │ Engineering │ Ask for a product change            │
└────┴─────────────────┴─────────────┴─────────────────────────────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"2 item(s); see one with: fsv sr fields ID
"#
    );
}

#[test]
fn sr_catalog_category_json() {
    let r = run_fixture(
        "catalog",
        &["sr", "catalog", "-c", "Engineering", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "display_id": 77,
    "id": 4400001,
    "name": "Feature Request",
    "short_description": "Ask for a product change",
    "category_id": 7001,
    "category": "Engineering"
  },
  {
    "display_id": 80,
    "id": 4400004,
    "name": "Build Agent",
    "short_description": "A CI build agent",
    "category_id": 7004,
    "category": "Engineering Tools"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_catalog_search() {
    let r = run_fixture("catalog", &["sr", "catalog", "-s", "laptop"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                    Service catalog                     
┏━━━━┳━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ ID ┃ Item   ┃ Category ┃ Short description           ┃
┡━━━━╇━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ 79 │ Laptop │ Hardware │ A new or replacement laptop │
└────┴────────┴──────────┴─────────────────────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"1 item(s); see one with: fsv sr fields ID
"#
    );
}

#[test]
fn sr_catalog_unknown_category() {
    let r = run_fixture("catalog", &["sr", "catalog", "-c", "nope"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no category matches 'nope'; categories: Engineering, Engineering Tools, Access, Hardware
"#
    );
}

#[test]
fn sr_catalog_no_match() {
    let r = run_fixture("catalog", &["sr", "catalog", "-s", "zzz"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"              Service catalog               
┏━━━━┳━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┓
┃ ID ┃ Item ┃ Category ┃ Short description ┃
┡━━━━╇━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━┩
└────┴──────┴──────────┴───────────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"no items match
"#
    );
}

#[test]
fn sr_fields_sections() {
    let r = run_fixture("catalog", &["sr", "fields", "77"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Feature Request  #77 · Engineering
Ask for a product change
Use this form for feature work.
Attachments: required: at least one general file
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field                      ┃ Required         ┃ Type     ┃ Details                                                    ┃
┡━━━━━━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Notice                     │                  │ note     │ Attach the overview deck.                                  │
│ Title                      │ yes              │ text     │ e.g. Short name                                            │
│ Details                    │ yes              │ textarea │                                                            │
│ Affected system (if known) │ no               │ textarea │                                                            │
│ Needed by                  │ yes              │ date     │                                                            │
│ 1. Expected benefit        │ yes              │ dropdown │ • More revenue  (Total USD) → reveals: 1.1 Revenue target  │
│                            │                  │          │ • Lower cost → reveals: 1.2 Cost target                    │
│                            │                  │          │ • Other                                                    │
│   ↳ 1.1 Revenue target     │ yes (when shown) │ text     │ shown when 1. Expected benefit = More revenue  (Total USD) │
│   ↳ 1.2 Cost target        │ yes (when shown) │ text     │ shown when 1. Expected benefit = Lower cost                │
│ Priority                   │ yes              │ dropdown │ • Critical - blocks customers                              │
│                            │                  │          │ • Medium - planned work                                    │
│                            │                  │          │ • Low - cosmetic, reports                                  │
│ Overview deck (.pptx)      │ yes              │ file(s)  │ e.g. Upload files or drop here                             │
│                            │                  │          │ types: .pptx · at most 1 file(s)                           │
└────────────────────────────┴──────────────────┴──────────┴────────────────────────────────────────────────────────────┘
item form as of 2030-01-15T10:00:00+09:00; 0 business rule(s) apply. Create with: fsv sr create 77 (interactive form)
"#
    );
    assert_eq!(
        r.err,
        r#"warning: Separate requested-for is available via --requested-for; requester remains unchanged.
"#
    );
}

#[test]
fn sr_fields_rules() {
    let r = run_fixture("catalog", &["sr", "fields", "78"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"VPN Access  #78 · Access
Remote access to the office network
Attachments: not accepted
┏━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field         ┃ Required ┃ Type     ┃ Details                                            ┃
┡━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Reason        │ yes      │ text     │                                                    │
│ Access level  │ yes      │ dropdown │ • Read only                                        │
│               │          │          │ • Read  write                                      │
│               │          │          │ • Admin                                            │
│ Justification │ no       │ textarea │ hidden at load by rule Admin needs a justification │
└───────────────┴──────────┴──────────┴────────────────────────────────────────────────────┘
item form as of 2030-01-10T10:00:00+09:00; 1 business rule(s) apply. Create with: fsv sr create 78 (interactive form)
"#
    );
    assert_eq!(
        r.err,
        r#"warning: Separate requested-for is available via --requested-for; requester remains unchanged.
"#
    );
}

#[test]
fn sr_fields_by_name() {
    let r = run_fixture("catalog", &["sr", "fields", "VPN Access"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"VPN Access  #78 · Access
Remote access to the office network
Attachments: not accepted
┏━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field         ┃ Required ┃ Type     ┃ Details                                            ┃
┡━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Reason        │ yes      │ text     │                                                    │
│ Access level  │ yes      │ dropdown │ • Read only                                        │
│               │          │          │ • Read  write                                      │
│               │          │          │ • Admin                                            │
│ Justification │ no       │ textarea │ hidden at load by rule Admin needs a justification │
└───────────────┴──────────┴──────────┴────────────────────────────────────────────────────┘
item form as of 2030-01-10T10:00:00+09:00; 1 business rule(s) apply. Create with: fsv sr create 78 (interactive form)
"#
    );
    assert_eq!(
        r.err,
        r#"warning: Separate requested-for is available via --requested-for; requester remains unchanged.
"#
    );
}

#[test]
fn sr_fields_not_found() {
    let r = run_fixture("catalog", &["sr", "fields", "99"], None);
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 404 fetching service-item: no route
"#
    );
}

#[test]
fn sr_get_default() {
    let r = run_fixture("sr_get", &["sr", "get", "SR-123"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"────────────────────────────────────────────────────────────────────────────────────────── SR-123  Dark mode ───────────────────────────────────────────────────────────────────────────────────────────
Feature Request  #77 · stage Requested
Title                  Dark mode                
Details                Please add dark mode.    
1. Expected benefit    More revenue  (Total USD)
1.1 Revenue target     +5%                      
Needed by              2030-12-03               
Overview deck (.pptx)  deck.pptx                
Attachments
  deck.pptx  11 B  id 801 · Overview deck (.pptx)
  ../escape.sh  5 B  id 802 · request
  brief.pdf  9 B  id 803 · request
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_get_json() {
    let r = run_fixture("sr_get", &["sr", "get", "SR-123", "-o", "json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": "SR-123",
  "subject": "Dark mode",
  "items": [
    {
      "item": "Feature Request",
      "display_id": 77,
      "stage": "Requested",
      "values": [
        {
          "label": "Title",
          "name": "title_100",
          "value": "Dark mode"
        },
        {
          "label": "Details",
          "name": "paragraph_101",
          "value": "Please add dark mode."
        },
        {
          "label": "1. Expected benefit",
          "name": "goal",
          "value": "More revenue  (Total USD)"
        },
        {
          "label": "1.1 Revenue target",
          "name": "goal_1_kpi",
          "value": "+5%"
        },
        {
          "label": "Needed by",
          "name": "date_103",
          "value": "2030-12-03"
        },
        {
          "label": "Overview deck (.pptx)",
          "name": "overview_deck",
          "value": "deck.pptx"
        }
      ]
    }
  ],
  "attachments": [
    {
      "id": 801,
      "name": "deck.pptx",
      "size": 11,
      "source": "Overview deck (.pptx)"
    },
    {
      "id": 802,
      "name": "../escape.sh",
      "size": 5,
      "source": "request"
    },
    {
      "id": 803,
      "name": "brief.pdf",
      "size": 9,
      "source": "request"
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_get_items() {
    let r = run_fixture("sr_get", &["sr", "get", "123", "--items"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"────────────────────────────────────────────────────────────────────────────────────────── SR-123  Dark mode ───────────────────────────────────────────────────────────────────────────────────────────
Feature Request  #77 · stage Requested
Title                  Dark mode                
Details                Please add dark mode.    
1. Expected benefit    More revenue  (Total USD)
1.1 Revenue target     +5%                      
Needed by              2030-12-03               
Overview deck (.pptx)  deck.pptx                
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_get_attachments_json() {
    let r = run_fixture(
        "sr_get",
        &["sr", "get", "123", "--attachments", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": "SR-123",
  "subject": "Dark mode",
  "attachments": [
    {
      "id": 801,
      "name": "deck.pptx",
      "size": 11,
      "source": "Overview deck (.pptx)"
    },
    {
      "id": 802,
      "name": "../escape.sh",
      "size": 5,
      "source": "request"
    },
    {
      "id": 803,
      "name": "brief.pdf",
      "size": 9,
      "source": "request"
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_get_download_mixed() {
    let r = run_fixture("sr_get", &["sr", "get", "123", "--download", "ref"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        r.out,
        r#"────────────────────────────────────────────────────────────────────────────────────────── SR-123  Dark mode ───────────────────────────────────────────────────────────────────────────────────────────
Attachments
  deck.pptx  11 B  id 801 · Overview deck (.pptx)
  ../escape.sh  5 B  id 802 · request
  brief.pdf  9 B  id 803 · request
  downloaded deck.pptx
  rejected ../escape.sh  unsafe file name '../escape.sh': contains '..'
  error brief.pdf  HTTP 500
Saved to ref
"#
    );
    assert_eq!(r.err, r#""#);
}

/// `console.print(f"Saved to {download}", soft_wrap=True)`: a long DIR
/// stays on one line.
#[test]
fn sr_get_download_long_dir_not_wrapped() {
    let dir = "segment0123456789/".repeat(14);
    let r = run_fixture("sr_get", &["sr", "get", "123", "--download", &dir], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.out.ends_with(&format!("\nSaved to {dir}\n")), "{}", r.out);
}

#[test]
fn sr_get_no_items() {
    let r = run_fixture("sr_get", &["sr", "get", "124"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"────────────────────────────────────────────────────────────────────────────────────────── SR-124  Dark mode ───────────────────────────────────────────────────────────────────────────────────────────
no requested items (not a service request?)
Attachments
  (none)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_get_no_items_json() {
    let r = run_fixture("sr_get", &["sr", "get", "124", "-o", "json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": "SR-124",
  "subject": "Dark mode",
  "items": [],
  "attachments": []
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_get_not_found() {
    let r = run_fixture("sr_get", &["sr", "get", "125"], None);
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 404 fetching ticket: Record not found
"#
    );
}

#[test]
fn sr_get_wrong_prefix() {
    let r = run_fixture("sr_get", &["sr", "get", "CHN-1"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: prefix CHN not valid for tickets (expect INC or SR)
"#
    );
}

#[test]
fn form_fields_table() {
    let r = run_fixture("form", &["form", "fields"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                          change form fields (web order; state on a new form)                                                           
┏━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Label                 ┃ Name               ┃ Type             ┃ On load           ┃ Help                                                                             ┃
┡━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Requester             │ requester          │ lookup_user      │ visible, required │ e.g. jane.doe@acme.example                                                       │
│ Request for           │ requested_for      │ lookup_user      │ visible           │ The user this change is for, if not the requester (shown in fsv only: not sent … │
│ Department            │ department         │ department (2)   │ visible           │                                                                                  │
│ Team                  │ team               │ dropdown (3)     │ visible, required │                                                                                  │
│ Group                 │ group              │ lookup_group     │ visible, required │                                                                                  │
│ Agent                 │ agent              │ lookup_agent     │ visible, required │                                                                                  │
│ Change Type           │ change_type        │ dropdown (4)     │ visible, required │                                                                                  │
│ Services              │ services           │ multi_select (3) │ visible, required │                                                                                  │
│ Cloud Only            │ cloud_only         │ dropdown (2)     │ visible, required │ Only the cloud team performs this change                                         │
│ Stage                 │ stage              │ dropdown (2)     │ visible, required │                                                                                  │
│ Rollout Tool          │ rollout_tool       │ dropdown (3)     │ hidden            │                                                                                  │
│ Reviewers             │ reviewers          │ lookup_user      │ hidden            │                                                                                  │
│ Staging Start         │ staging_start      │ datetime         │ hidden            │                                                                                  │
│ Staging End           │ staging_end        │ datetime         │ hidden            │                                                                                  │
│ Subject               │ subject            │ text             │ visible, required │                                                                                  │
│ Description           │ description        │ richtext         │ visible, required │ What changes and why                                                             │
│ Emergency Reason      │ emergency_reason   │ dropdown (2)     │ hidden            │                                                                                  │
│ Needs Security Review │ needs_review       │ checkbox         │ visible           │                                                                                  │
│ Review Scope          │ review_scope       │ multi_select (2) │ hidden            │                                                                                  │
│ Planned Start Date    │ planned_start_date │ datetime         │ visible, required │                                                                                  │
│ Planned End Date      │ planned_end_date   │ datetime         │ visible, required │                                                                                  │
│ Downtime              │ downtime           │ dropdown (2)     │ visible, required │                                                                                  │
│ Downtime Window       │ downtime_window    │ text             │ visible           │                                                                                  │
│ Sign-off Date         │ signoff_date       │ dropdown (3)     │ visible, required │                                                                                  │
│ Estimated Hours       │ estimated_hours    │ number           │ visible           │                                                                                  │
│ Impact                │ impact             │ dropdown (3)     │ visible, required │ • Low — Sandbox only                                                             │
│                       │                    │                  │                   │ • Medium — Internal tools                                                        │
│                       │                    │                  │                   │ • High — Customer facing                                                         │
│ Priority              │ priority           │ dropdown (4)     │ visible, required │ • Low — Nice to have                                                             │
│                       │                    │                  │                   │ • Medium — Planned work                                                          │
│                       │                    │                  │                   │ • High — Revenue at risk                                                         │
│ Risk                  │ risk               │ dropdown (4)     │ visible, required │                                                                                  │
│ Category              │ category           │ nested (2)       │ hidden            │                                                                                  │
│ Sub-Category          │ sub_category       │ nested           │ hidden            │                                                                                  │
│ Item                  │ item_category      │ nested           │ hidden            │                                                                                  │
│ Status                │ status             │ dropdown (3)     │ visible, required │                                                                                  │
│ Legacy Flag           │ legacy_flag        │ checkbox         │ hidden            │                                                                                  │
│ Maintenance Window    │ change_window      │ lookup           │ visible           │                                                                                  │
└───────────────────────┴────────────────────┴──────────────────┴───────────────────┴──────────────────────────────────────────────────────────────────────────────────┘
1 information block(s) on the form, not inputs: Read the change guide first.
choices: fsv form fields changes --choices LABEL
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_fields_json() {
    let r = run_fixture("form", &["form", "fields", "-o", "json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "label": "Requester",
    "name": "requester",
    "kind": "lookup_user",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "e.g. jane.doe@acme.example",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Request for",
    "name": "requested_for",
    "kind": "lookup_user",
    "custom": false,
    "required_static": false,
    "visible_on_load": true,
    "required_on_load": false,
    "help": "The user this change is for, if not the requester (shown in fsv only: not sent yet).",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Department",
    "name": "department",
    "kind": "department",
    "custom": false,
    "required_static": false,
    "visible_on_load": true,
    "required_on_load": false,
    "help": "",
    "choices": 2,
    "choice_descriptions": []
  },
  {
    "label": "Team",
    "name": "team",
    "kind": "dropdown",
    "custom": true,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 3,
    "choice_descriptions": []
  },
  {
    "label": "Group",
    "name": "group",
    "kind": "lookup_group",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Agent",
    "name": "agent",
    "kind": "lookup_agent",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Change Type",
    "name": "change_type",
    "kind": "dropdown",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 4,
    "choice_descriptions": []
  },
  {
    "label": "Services",
    "name": "services",
    "kind": "multi_select",
    "custom": true,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 3,
    "choice_descriptions": []
  },
  {
    "label": "Cloud Only",
    "name": "cloud_only",
    "kind": "dropdown",
    "custom": true,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "Only the cloud team performs this change",
    "choices": 2,
    "choice_descriptions": []
  },
  {
    "label": "Stage",
    "name": "stage",
    "kind": "dropdown",
    "custom": true,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 2,
    "choice_descriptions": []
  },
  {
    "label": "Rollout Tool",
    "name": "rollout_tool",
    "kind": "dropdown",
    "custom": true,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 3,
    "choice_descriptions": []
  },
  {
    "label": "Reviewers",
    "name": "reviewers",
    "kind": "lookup_user",
    "custom": true,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Staging Start",
    "name": "staging_start",
    "kind": "datetime",
    "custom": true,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Staging End",
    "name": "staging_end",
    "kind": "datetime",
    "custom": true,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Subject",
    "name": "subject",
    "kind": "text",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Description",
    "name": "description",
    "kind": "richtext",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "What changes and why",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Emergency Reason",
    "name": "emergency_reason",
    "kind": "dropdown",
    "custom": true,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 2,
    "choice_descriptions": []
  },
  {
    "label": "Needs Security Review",
    "name": "needs_review",
    "kind": "checkbox",
    "custom": true,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Review Scope",
    "name": "review_scope",
    "kind": "multi_select",
    "custom": true,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 2,
    "choice_descriptions": []
  },
  {
    "label": "Planned Start Date",
    "name": "planned_start_date",
    "kind": "datetime",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Planned End Date",
    "name": "planned_end_date",
    "kind": "datetime",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Downtime",
    "name": "downtime",
    "kind": "dropdown",
    "custom": true,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 2,
    "choice_descriptions": []
  },
  {
    "label": "Downtime Window",
    "name": "downtime_window",
    "kind": "text",
    "custom": true,
    "required_static": false,
    "visible_on_load": true,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Sign-off Date",
    "name": "signoff_date",
    "kind": "dropdown",
    "custom": true,
    "required_static": false,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 3,
    "choice_descriptions": []
  },
  {
    "label": "Estimated Hours",
    "name": "estimated_hours",
    "kind": "number",
    "custom": true,
    "required_static": false,
    "visible_on_load": true,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Impact",
    "name": "impact",
    "kind": "dropdown",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 3,
    "choice_descriptions": [
      {
        "label": "Low",
        "description": "Sandbox only"
      },
      {
        "label": "Medium",
        "description": "Internal tools"
      },
      {
        "label": "High",
        "description": "Customer facing"
      }
    ]
  },
  {
    "label": "Priority",
    "name": "priority",
    "kind": "dropdown",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 4,
    "choice_descriptions": [
      {
        "label": "Low",
        "description": "Nice to have"
      },
      {
        "label": "Medium",
        "description": "Planned work"
      },
      {
        "label": "High",
        "description": "Revenue at risk"
      }
    ]
  },
  {
    "label": "Risk",
    "name": "risk",
    "kind": "dropdown",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 4,
    "choice_descriptions": []
  },
  {
    "label": "Category",
    "name": "category",
    "kind": "nested",
    "custom": false,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 2,
    "choice_descriptions": []
  },
  {
    "label": "Sub-Category",
    "name": "sub_category",
    "kind": "nested",
    "custom": false,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Item",
    "name": "item_category",
    "kind": "nested",
    "custom": false,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Status",
    "name": "status",
    "kind": "dropdown",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 3,
    "choice_descriptions": []
  },
  {
    "label": "Legacy Flag",
    "name": "legacy_flag",
    "kind": "checkbox",
    "custom": true,
    "required_static": false,
    "visible_on_load": false,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Maintenance Window",
    "name": "change_window",
    "kind": "lookup",
    "custom": false,
    "required_static": false,
    "visible_on_load": true,
    "required_on_load": false,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_fields_choices() {
    let r = run_fixture(
        "form",
        &["form", "fields", "--choices", "Change Type"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"      Change Type      
   (change_type): 3    
        choices        
┏━━━━━━━━━━━┳━━━━━━━━━┓
┃ Choice    ┃ Meaning ┃
┡━━━━━━━━━━━╇━━━━━━━━━┩
│ Normal    │         │
│ Standard  │         │
│ Emergency │         │
└───────────┴─────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_fields_choices_json() {
    let r = run_fixture(
        "form",
        &["form", "fields", "--choices", "team", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "field": "team",
  "label": "Team",
  "choices": [
    {
      "label": "Platform",
      "value": "Platform",
      "id": 1101,
      "description": ""
    },
    {
      "label": "Payments",
      "value": "Payments",
      "id": 1102,
      "description": ""
    },
    {
      "label": "Labs",
      "value": "Labs",
      "id": 1103,
      "description": ""
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_fields_unknown_choice() {
    let r = run_fixture("form", &["form", "fields", "--choices", "nope"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: unknown field: 'nope'
"#
    );
}

#[test]
fn form_explain_new() {
    let r = run_fixture("form", &["form", "explain"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                                                           change form (new)                                                                                            
┏━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field                 ┃ State             ┃ Value  ┃ Why                                                                                                                                             ┃
┡━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Requester             │ visible, required │        │ form metadata                                                                                                                                   │
│ Request for           │ visible           │        │ form metadata                                                                                                                                   │
│ Department            │ visible           │        │ not disabled (enable by rule 'Labs team' (id 14) (reversed: rule does not match))                                                               │
│ Team                  │ visible, required │        │ form metadata                                                                                                                                   │
│ Group                 │ visible, required │        │ form metadata                                                                                                                                   │
│ Agent                 │ visible, required │        │ form metadata                                                                                                                                   │
│ Change Type           │ visible, required │ Normal │ form metadata                                                                                                                                   │
│ Services              │ visible, required │        │ form metadata                                                                                                                                   │
│ Cloud Only            │ visible, required │        │ required (mandate by rule 'Mandatory unless labs (new only)' (id 15))                                                                           │
│ Stage                 │ visible, required │        │ required (mandate by rule 'Mandatory unless labs (new only)' (id 15)); not choices restricted (restore by rule 'Cloud only no' (id 5)           │
│                       │                   │        │ (reversed: rule does not match))                                                                                                                │
│ Rollout Tool          │ hidden            │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                               │
│ Reviewers             │ hidden            │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                               │
│ Staging Start         │ hidden            │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                               │
│ Staging End           │ hidden            │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                               │
│ Subject               │ visible, required │        │ form metadata                                                                                                                                   │
│ Description           │ visible, required │        │ form metadata                                                                                                                                   │
│ Emergency Reason      │ hidden            │        │ not visible (hide by rule 'Normal needs sign-off' (id 9))                                                                                       │
│ Needs Security Review │ visible           │        │ not required (unmandate by rule 'Not labs: optional extras' (id 1))                                                                             │
│ Review Scope          │ hidden            │        │ not visible (hide by rule 'No review: hide scope' (id 11))                                                                                      │
│ Planned Start Date    │ visible, required │        │ form metadata                                                                                                                                   │
│ Planned End Date      │ visible, required │        │ form metadata                                                                                                                                   │
│ Downtime              │ visible, required │        │ visible (show by rule 'Labs team' (id 14) (reversed: rule does not match))                                                                      │
│ Downtime Window       │ visible           │        │ form metadata                                                                                                                                   │
│ Sign-off Date         │ visible, required │        │ required (mandate by rule 'Normal needs sign-off' (id 9)); not choices restricted (restore by rule 'Emergency' (id 10) (reversed: rule does not │
│                       │                   │        │ match))                                                                                                                                         │
│ Estimated Hours       │ visible           │        │ form metadata                                                                                                                                   │
│ Impact                │ visible, required │ Low    │ form metadata                                                                                                                                   │
│ Priority              │ visible, required │ Low    │ form metadata                                                                                                                                   │
│ Risk                  │ visible, required │ Low    │ form metadata                                                                                                                                   │
│ Category              │ hidden            │        │ not visible (hide by rule 'Normal needs sign-off' (id 9))                                                                                       │
│ Status                │ visible, required │ Open   │ form metadata                                                                                                                                   │
│ Legacy Flag           │ hidden            │        │ not visible (hide by rule 'Hide legacy flag' (id 8))                                                                                            │
│ Maintenance Window    │ visible           │        │ form metadata                                                                                                                                   │
└───────────────────────┴───────────────────┴────────┴─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
conflict cloud_only.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
conflict stage.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_set() {
    let r = run_fixture(
        "form",
        &["form", "explain", "--set", "Change Type=Emergency"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                                                           change form (new)                                                                                            
┏━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field                 ┃ State             ┃ Value     ┃ Why                                                                                                                                          ┃
┡━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Requester             │ visible, required │           │ form metadata                                                                                                                                │
│ Request for           │ visible           │           │ form metadata                                                                                                                                │
│ Department            │ visible           │           │ not disabled (enable by rule 'Labs team' (id 14) (reversed: rule does not match) on change of change_type)                                   │
│ Team                  │ visible, required │           │ form metadata                                                                                                                                │
│ Group                 │ visible, required │           │ form metadata                                                                                                                                │
│ Agent                 │ visible, required │           │ form metadata                                                                                                                                │
│ Change Type           │ visible, required │ Emergency │ form metadata                                                                                                                                │
│ Services              │ visible, required │           │ form metadata                                                                                                                                │
│ Cloud Only            │ visible, required │           │ required (mandate by rule 'Mandatory unless labs (new only)' (id 15) on change of change_type)                                               │
│ Stage                 │ visible, required │           │ required (mandate by rule 'Mandatory unless labs (new only)' (id 15) on change of change_type); not choices restricted (restore by rule      │
│                       │                   │           │ 'Cloud only no' (id 5) (reversed: rule does not match) on change of change_type)                                                             │
│ Rollout Tool          │ hidden            │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of change_type)                                   │
│ Reviewers             │ hidden            │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of change_type)                                   │
│ Staging Start         │ hidden            │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of change_type)                                   │
│ Staging End           │ hidden            │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of change_type)                                   │
│ Subject               │ visible, required │           │ form metadata                                                                                                                                │
│ Description           │ visible, required │           │ form metadata                                                                                                                                │
│ Emergency Reason      │ visible, required │           │ visible (show by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match) on change of change_type); required (mandate by rule    │
│                       │                   │           │ 'Emergency' (id 10) on change of change_type)                                                                                                │
│ Needs Security Review │ visible           │           │ not required (unmandate by rule 'Not labs: optional extras' (id 1) on change of change_type)                                                 │
│ Review Scope          │ hidden            │           │ not visible (hide by rule 'No review: hide scope' (id 11) on change of change_type)                                                          │
│ Planned Start Date    │ visible, required │           │ form metadata                                                                                                                                │
│ Planned End Date      │ visible, required │           │ form metadata                                                                                                                                │
│ Downtime              │ visible, required │           │ visible (show by rule 'Labs team' (id 14) (reversed: rule does not match) on change of change_type)                                          │
│ Downtime Window       │ visible           │           │ form metadata                                                                                                                                │
│ Sign-off Date         │ visible           │           │ not required (unmandate by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match) on change of change_type); choices restricted │
│                       │                   │           │ (populate by rule 'Emergency' (id 10) on change of change_type)                                                                              │
│ Estimated Hours       │ visible           │           │ form metadata                                                                                                                                │
│ Impact                │ visible, required │ Low       │ form metadata                                                                                                                                │
│ Priority              │ visible, required │ Low       │ form metadata                                                                                                                                │
│ Risk                  │ visible, required │ Low       │ form metadata                                                                                                                                │
│ Category              │ visible           │           │ visible (show by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match) on change of change_type)                               │
│ Status                │ visible, required │ Open      │ form metadata                                                                                                                                │
│ Legacy Flag           │ hidden            │           │ not visible (hide by rule 'Hide legacy flag' (id 8) on change of change_type)                                                                │
│ Maintenance Window    │ visible           │           │ form metadata                                                                                                                                │
└───────────────────────┴───────────────────┴───────────┴──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
message Tell the payments on-call before you start (rule Payments heads-up)
conflict cloud_only.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
conflict stage.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_all() {
    let r = run_fixture(
        "form",
        &["form", "explain", "--set", "Team=Labs", "--all"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                                                           change form (new)                                                                                            
┏━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field                 ┃ State              ┃ Value  ┃ Why                                                                                                                                            ┃
┡━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Requester             │ visible, required  │        │ form metadata                                                                                                                                  │
│ Request for           │ visible            │        │ form metadata                                                                                                                                  │
│ Department            │ visible, read-only │        │ disabled (disable by rule 'Labs team' (id 14) on change of team)                                                                               │
│ Team                  │ visible, required  │ Labs   │ form metadata                                                                                                                                  │
│ Group                 │ visible, required  │        │ form metadata                                                                                                                                  │
│ Agent                 │ visible, required  │        │ form metadata                                                                                                                                  │
│ Change Type           │ visible, required  │ Normal │ form metadata                                                                                                                                  │
│ Services              │ visible, required  │        │ form metadata                                                                                                                                  │
│ Cloud Only            │ visible            │        │ not required (unmandate by rule 'Mandatory unless labs (new only)' (id 15) (reversed: rule does not match) on change of team)                  │
│ Stage                 │ visible            │        │ not required (unmandate by rule 'Mandatory unless labs (new only)' (id 15) (reversed: rule does not match) on change of team); not choices     │
│                       │                    │        │ restricted (restore by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of team)                                          │
│ Rollout Tool          │ hidden             │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of team)                                            │
│ Reviewers             │ hidden             │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of team)                                            │
│ Staging Start         │ hidden             │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of team)                                            │
│ Staging End           │ hidden             │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match) on change of team)                                            │
│ Subject               │ visible, required  │        │ form metadata                                                                                                                                  │
│ Description           │ visible, required  │        │ form metadata                                                                                                                                  │
│ Emergency Reason      │ hidden             │        │ not visible (hide by rule 'Normal needs sign-off' (id 9) on change of team)                                                                    │
│ Needs Security Review │ visible, required  │        │ required (mandate by rule 'Not labs: optional extras' (id 1) (reversed: rule does not match) on change of team)                                │
│ Review Scope          │ hidden             │        │ not visible (hide by rule 'No review: hide scope' (id 11) on change of team)                                                                   │
│ Planned Start Date    │ visible, required  │        │ form metadata                                                                                                                                  │
│ Planned End Date      │ visible, required  │        │ form metadata                                                                                                                                  │
│ Downtime              │ hidden             │        │ not visible (hide by rule 'Labs team' (id 14) on change of team)                                                                               │
│ Downtime Window       │ visible            │        │ form metadata                                                                                                                                  │
│ Sign-off Date         │ visible, required  │        │ required (mandate by rule 'Normal needs sign-off' (id 9) on change of team); not choices restricted (restore by rule 'Emergency' (id 10)       │
│                       │                    │        │ (reversed: rule does not match) on change of team)                                                                                             │
│ Estimated Hours       │ visible            │        │ form metadata                                                                                                                                  │
│ Impact                │ visible, required  │ Low    │ form metadata                                                                                                                                  │
│ Priority              │ visible, required  │ Low    │ form metadata                                                                                                                                  │
│ Risk                  │ visible, required  │ Low    │ form metadata                                                                                                                                  │
│ Category              │ hidden             │        │ not visible (hide by rule 'Normal needs sign-off' (id 9) on change of team)                                                                    │
│ Sub-Category          │ hidden             │        │ not visible (the field it belongs to is hidden)                                                                                                │
│ Item                  │ hidden             │        │ not visible (the field it belongs to is hidden)                                                                                                │
│ Status                │ visible, required  │ Open   │ form metadata                                                                                                                                  │
│ Legacy Flag           │ hidden             │        │ not visible (hide by rule 'Hide legacy flag' (id 8) on change of team)                                                                         │
│ Maintenance Window    │ visible            │        │ form metadata                                                                                                                                  │
└───────────────────────┴────────────────────┴────────┴────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_bad_answer() {
    let r = run_fixture("form", &["form", "explain", "--set", "Bogus=1"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                                                           change form (new)                                                                                            
┏━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field                 ┃ State             ┃ Value  ┃ Why                                                                                                                                             ┃
┡━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Requester             │ visible, required │        │ form metadata                                                                                                                                   │
│ Request for           │ visible           │        │ form metadata                                                                                                                                   │
│ Department            │ visible           │        │ not disabled (enable by rule 'Labs team' (id 14) (reversed: rule does not match))                                                               │
│ Team                  │ visible, required │        │ form metadata                                                                                                                                   │
│ Group                 │ visible, required │        │ form metadata                                                                                                                                   │
│ Agent                 │ visible, required │        │ form metadata                                                                                                                                   │
│ Change Type           │ visible, required │ Normal │ form metadata                                                                                                                                   │
│ Services              │ visible, required │        │ form metadata                                                                                                                                   │
│ Cloud Only            │ visible, required │        │ required (mandate by rule 'Mandatory unless labs (new only)' (id 15))                                                                           │
│ Stage                 │ visible, required │        │ required (mandate by rule 'Mandatory unless labs (new only)' (id 15)); not choices restricted (restore by rule 'Cloud only no' (id 5)           │
│                       │                   │        │ (reversed: rule does not match))                                                                                                                │
│ Rollout Tool          │ hidden            │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                               │
│ Reviewers             │ hidden            │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                               │
│ Staging Start         │ hidden            │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                               │
│ Staging End           │ hidden            │        │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                               │
│ Subject               │ visible, required │        │ form metadata                                                                                                                                   │
│ Description           │ visible, required │        │ form metadata                                                                                                                                   │
│ Emergency Reason      │ hidden            │        │ not visible (hide by rule 'Normal needs sign-off' (id 9))                                                                                       │
│ Needs Security Review │ visible           │        │ not required (unmandate by rule 'Not labs: optional extras' (id 1))                                                                             │
│ Review Scope          │ hidden            │        │ not visible (hide by rule 'No review: hide scope' (id 11))                                                                                      │
│ Planned Start Date    │ visible, required │        │ form metadata                                                                                                                                   │
│ Planned End Date      │ visible, required │        │ form metadata                                                                                                                                   │
│ Downtime              │ visible, required │        │ visible (show by rule 'Labs team' (id 14) (reversed: rule does not match))                                                                      │
│ Downtime Window       │ visible           │        │ form metadata                                                                                                                                   │
│ Sign-off Date         │ visible, required │        │ required (mandate by rule 'Normal needs sign-off' (id 9)); not choices restricted (restore by rule 'Emergency' (id 10) (reversed: rule does not │
│                       │                   │        │ match))                                                                                                                                         │
│ Estimated Hours       │ visible           │        │ form metadata                                                                                                                                   │
│ Impact                │ visible, required │ Low    │ form metadata                                                                                                                                   │
│ Priority              │ visible, required │ Low    │ form metadata                                                                                                                                   │
│ Risk                  │ visible, required │ Low    │ form metadata                                                                                                                                   │
│ Category              │ hidden            │        │ not visible (hide by rule 'Normal needs sign-off' (id 9))                                                                                       │
│ Status                │ visible, required │ Open   │ form metadata                                                                                                                                   │
│ Legacy Flag           │ hidden            │        │ not visible (hide by rule 'Hide legacy flag' (id 8))                                                                                            │
│ Maintenance Window    │ visible           │        │ form metadata                                                                                                                                   │
└───────────────────────┴───────────────────┴────────┴─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
conflict cloud_only.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
conflict stage.required: unmandate by 'Not labs: optional extras' (id 1); mandate by 'Mandatory unless labs (new only)' (id 15) -> mandate wins ('Mandatory unless labs (new only)')
answer error unknown field: 'Bogus'
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_edit() {
    let r = run_fixture("form", &["form", "explain", "--id", "12"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                                                           change form (edit)                                                                                           
┏━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field                 ┃ State                        ┃ Value     ┃ Why                                                                                                                               ┃
┡━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Requester             │ visible, required            │           │ form metadata                                                                                                                     │
│ Request for           │ visible                      │           │ form metadata                                                                                                                     │
│ Department            │ visible, read-only           │           │ disabled (disable by rule 'Labs team' (id 14))                                                                                    │
│ Team                  │ visible, required            │ Labs      │ form metadata                                                                                                                     │
│ Group                 │ visible, required            │           │ form metadata                                                                                                                     │
│ Agent                 │ visible, required            │           │ form metadata                                                                                                                     │
│ Change Type           │ visible, required            │ Emergency │ form metadata                                                                                                                     │
│ Services              │ visible, required            │           │ form metadata                                                                                                                     │
│ Cloud Only            │ visible, required            │           │ required (mandate by rule 'Not labs: optional extras' (id 1) (reversed: rule does not match))                                     │
│ Stage                 │ visible, required            │ Live      │ required (mandate by rule 'Not labs: optional extras' (id 1) (reversed: rule does not match)); not choices restricted (restore by │
│                       │                              │           │ rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                                      │
│ Rollout Tool          │ hidden                       │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                 │
│ Reviewers             │ hidden                       │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                 │
│ Staging Start         │ hidden                       │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                 │
│ Staging End           │ hidden                       │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                 │
│ Subject               │ visible, required, read-only │ Patch     │ disabled (disable by rule 'Edit: lock subject' (id 19))                                                                           │
│ Description           │ visible, required            │           │ form metadata                                                                                                                     │
│ Emergency Reason      │ visible, required            │           │ visible (show by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match)); required (mandate by rule 'Emergency' (id  │
│                       │                              │           │ 10))                                                                                                                              │
│ Needs Security Review │ visible, required            │           │ required (mandate by rule 'Not labs: optional extras' (id 1) (reversed: rule does not match))                                     │
│ Review Scope          │ hidden                       │           │ not visible (hide by rule 'No review: hide scope' (id 11))                                                                        │
│ Planned Start Date    │ visible, required            │           │ form metadata                                                                                                                     │
│ Planned End Date      │ visible, required            │           │ form metadata                                                                                                                     │
│ Downtime              │ hidden                       │           │ not visible (hide by rule 'Labs team' (id 14))                                                                                    │
│ Downtime Window       │ visible                      │           │ form metadata                                                                                                                     │
│ Sign-off Date         │ visible                      │           │ not required (unmandate by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match)); choices restricted (populate by  │
│                       │                              │           │ rule 'Emergency' (id 10))                                                                                                         │
│ Estimated Hours       │ visible                      │           │ form metadata                                                                                                                     │
│ Impact                │ visible, required            │           │ form metadata                                                                                                                     │
│ Priority              │ visible, required            │           │ form metadata                                                                                                                     │
│ Risk                  │ visible, required            │           │ form metadata                                                                                                                     │
│ Category              │ visible                      │           │ visible (show by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match))                                             │
│ Status                │ visible, required            │           │ form metadata                                                                                                                     │
│ Legacy Flag           │ hidden                       │           │ not visible (hide by rule 'Hide legacy flag' (id 8))                                                                              │
│ Maintenance Window    │ visible                      │           │ form metadata                                                                                                                     │
└───────────────────────┴──────────────────────────────┴───────────┴───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
message Tell the payments on-call before you start (rule Payments heads-up)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_unsupported() {
    let r = run_fixture("form", &["form", "explain", "assets"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: unsupported form 'assets'; supported: changes, tickets, problems
"#
    );
}

#[test]
fn form_dump_bad_mode() {
    let r = run_fixture("form", &["form", "dump", "--mode", "x"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --mode must be new or edit
"#
    );
}

#[test]
fn form_diff_against() {
    let r = run_fixture(
        "form",
        &[
            "form",
            "diff",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/form_snaps/old.json"
            ),
            "--against",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/form_snaps/new.json"
            ),
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"+ field Added (added)
- field Gone (gone)
~ field Stage (stage): choices reordered
~ field Team name (team): label: 'Team' -> 'Team name'; required: False -> True; choices +['Payments'] -['Labs']
+ rule R3 (3/fixed)
- rule R2 (2/fixed)
~ rule R1 (1/rev): state
7 difference(s)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_diff_against_json() {
    let r = run_fixture(
        "form",
        &[
            "form",
            "diff",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/form_snaps/old.json"
            ),
            "--against",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/form_snaps/new.json"
            ),
            "-o",
            "json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "kind": "field",
    "change": "added",
    "name": "added",
    "label": "Added"
  },
  {
    "kind": "field",
    "change": "removed",
    "name": "gone",
    "label": "Gone"
  },
  {
    "kind": "field",
    "change": "changed",
    "name": "stage",
    "label": "Stage",
    "what": [
      "choices reordered"
    ]
  },
  {
    "kind": "field",
    "change": "changed",
    "name": "team",
    "label": "Team name",
    "what": [
      "label: 'Team' -> 'Team name'",
      "required: False -> True",
      "choices +['Payments'] -['Labs']"
    ]
  },
  {
    "kind": "rule",
    "change": "added",
    "id": "3/fixed",
    "name": "R3"
  },
  {
    "kind": "rule",
    "change": "removed",
    "id": "2/fixed",
    "name": "R2"
  },
  {
    "kind": "rule",
    "change": "changed",
    "id": "1/rev",
    "name": "R1",
    "what": [
      "state"
    ]
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_diff_no_changes() {
    let r = run_fixture(
        "form",
        &[
            "form",
            "diff",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/form_snaps/old.json"
            ),
            "--against",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/form_snaps/same.json"
            ),
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"no changes
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_diff_missing() {
    let r = run_fixture("form", &["form", "diff", "missing.json"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: cannot read snapshot missing.json: [Errno 2] No such file or directory: 'missing.json'
"#
    );
}

#[test]
fn lookup_refresh_refetches_schema() {
    let r = run_steps(
        "change_fields",
        &[&["changes", "lookup", "env", "pr"]],
        &["changes", "lookup", "env", "pr", "--refresh"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━┳━━━━━━━┳━━━━━━━━┓
┃ id ┃ value ┃ detail ┃
┡━━━━╇━━━━━━━╇━━━━━━━━┩
│ 11 │ Prod  │        │
└────┴───────┴────────┘
"#
    );
    assert_eq!(r.err, r#""#);
    assert!(
        r.calls()
            .iter()
            .any(|(_, p)| p == "/api/_/change_form_fields")
    );
}

#[test]
fn lookup_uses_cached_schema() {
    let r = run_steps(
        "change_fields",
        &[&["changes", "lookup", "env", "pr"]],
        &["changes", "lookup", "env", "pr"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━┳━━━━━━━┳━━━━━━━━┓
┃ id ┃ value ┃ detail ┃
┡━━━━╇━━━━━━━╇━━━━━━━━┩
│ 11 │ Prod  │        │
└────┴───────┴────────┘
"#
    );
    assert_eq!(r.err, r#""#);
    assert!(!r.calls().iter().any(|(_, p)| p.contains("form_fields")));
}

#[test]
fn form_explain_ticket_record() {
    let r = run_fixture(
        "ticket_update",
        &["form", "explain", "tickets", "--id", "5"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"      ticket form (edit)       
┏━━━━━━━┳━━━━━━━┳━━━━━━━┳━━━━━┓
┃ Field ┃ State ┃ Value ┃ Why ┃
┡━━━━━━━╇━━━━━━━╇━━━━━━━╇━━━━━┩
└───────┴───────┴───────┴─────┘
"#
    );
    assert_eq!(
        r.err,
        r#"note: business rules not loaded (no workspace found in the form metadata or the account, and /api/_/business_rules answered HTTP 404); checking static required/visible flags only (override the 
workspace with `fsv config set forms.workspace_id N`)
"#
    );
    assert!(
        r.calls()
            .iter()
            .any(|(m, p)| m == "GET" && p == "/api/_/tickets/5")
    );
}

#[test]
fn form_explain_change_display_id() {
    let r = run_fixture("form", &["form", "explain", "--id", "CHN-12"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                                                           change form (edit)                                                                                           
┏━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field                 ┃ State                        ┃ Value     ┃ Why                                                                                                                               ┃
┡━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Requester             │ visible, required            │           │ form metadata                                                                                                                     │
│ Request for           │ visible                      │           │ form metadata                                                                                                                     │
│ Department            │ visible, read-only           │           │ disabled (disable by rule 'Labs team' (id 14))                                                                                    │
│ Team                  │ visible, required            │ Labs      │ form metadata                                                                                                                     │
│ Group                 │ visible, required            │           │ form metadata                                                                                                                     │
│ Agent                 │ visible, required            │           │ form metadata                                                                                                                     │
│ Change Type           │ visible, required            │ Emergency │ form metadata                                                                                                                     │
│ Services              │ visible, required            │           │ form metadata                                                                                                                     │
│ Cloud Only            │ visible, required            │           │ required (mandate by rule 'Not labs: optional extras' (id 1) (reversed: rule does not match))                                     │
│ Stage                 │ visible, required            │ Live      │ required (mandate by rule 'Not labs: optional extras' (id 1) (reversed: rule does not match)); not choices restricted (restore by │
│                       │                              │           │ rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                                      │
│ Rollout Tool          │ hidden                       │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                 │
│ Reviewers             │ hidden                       │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                 │
│ Staging Start         │ hidden                       │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                 │
│ Staging End           │ hidden                       │           │ not visible (hide by rule 'Cloud only no' (id 5) (reversed: rule does not match))                                                 │
│ Subject               │ visible, required, read-only │ Patch     │ disabled (disable by rule 'Edit: lock subject' (id 19))                                                                           │
│ Description           │ visible, required            │           │ form metadata                                                                                                                     │
│ Emergency Reason      │ visible, required            │           │ visible (show by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match)); required (mandate by rule 'Emergency' (id  │
│                       │                              │           │ 10))                                                                                                                              │
│ Needs Security Review │ visible, required            │           │ required (mandate by rule 'Not labs: optional extras' (id 1) (reversed: rule does not match))                                     │
│ Review Scope          │ hidden                       │           │ not visible (hide by rule 'No review: hide scope' (id 11))                                                                        │
│ Planned Start Date    │ visible, required            │           │ form metadata                                                                                                                     │
│ Planned End Date      │ visible, required            │           │ form metadata                                                                                                                     │
│ Downtime              │ hidden                       │           │ not visible (hide by rule 'Labs team' (id 14))                                                                                    │
│ Downtime Window       │ visible                      │           │ form metadata                                                                                                                     │
│ Sign-off Date         │ visible                      │           │ not required (unmandate by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match)); choices restricted (populate by  │
│                       │                              │           │ rule 'Emergency' (id 10))                                                                                                         │
│ Estimated Hours       │ visible                      │           │ form metadata                                                                                                                     │
│ Impact                │ visible, required            │           │ form metadata                                                                                                                     │
│ Priority              │ visible, required            │           │ form metadata                                                                                                                     │
│ Risk                  │ visible, required            │           │ form metadata                                                                                                                     │
│ Category              │ visible                      │           │ visible (show by rule 'Normal needs sign-off' (id 9) (reversed: rule does not match))                                             │
│ Status                │ visible, required            │           │ form metadata                                                                                                                     │
│ Legacy Flag           │ hidden                       │           │ not visible (hide by rule 'Hide legacy flag' (id 8))                                                                              │
│ Maintenance Window    │ visible                      │           │ form metadata                                                                                                                     │
└───────────────────────┴──────────────────────────────┴───────────┴───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
message Tell the payments on-call before you start (rule Payments heads-up)
"#
    );
    assert_eq!(r.err, r#""#);
}
