//! Forms, catalog and approvals CLI cases (batch "forms"). Expected output
//! was captured from the Python fsv on the same routes (`cmp.py --emit`).

mod common;
use common::{run_files, run_fixture};

/// `metadata.fetched_at` is the run's clock.
fn masked(out: &str) -> String {
    out.lines()
        .map(|l| match l.find("\"fetched_at\": \"") {
            Some(i) => format!("{}\"fetched_at\": \"<fetched>\",", &l[..i]),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

// Python commit: 18d91aa

#[test]
fn sr_catalog_truncated_json() {
    let r = run_fixture("catalog_truncated", &["sr", "catalog", "-o", "json"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "items": [
    {
      "display_id": 78,
      "id": 4400002,
      "name": "VPN Access",
      "short_description": "Remote access to the office network",
      "category_id": 7002,
      "category": "Access"
    },
    {
      "display_id": 77,
      "id": 4400001,
      "name": "Feature Request",
      "short_description": "Ask for a product change",
      "category_id": 7001,
      "category": "Engineering"
    }
  ],
  "truncated": true,
  "notice": "catalog list truncated: read 50 of 99 pages of 30 items (limit 50 pages); more items exist and are not shown or searched",
  "pages_read": 50,
  "per_page": 30,
  "max_pages": 50,
  "total_pages": 99
}
"#
    );
    assert_eq!(
        r.err,
        r#"warning: catalog list truncated: read 50 of 99 pages of 30 items (limit 50 pages); more items exist and are not shown or searched
"#
    );
}

#[test]
fn sr_catalog_truncated_table() {
    let r = run_fixture("catalog_truncated", &["sr", "catalog"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                              Service catalog                               
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
        r#"warning: catalog list truncated: read 50 of 99 pages of 30 items (limit 50 pages); more items exist and are not shown or searched
2 item(s); see one with: fsv sr fields ID
"#
    );
}

#[test]
fn sr_fields_name_lookup_says_truncated() {
    let r = run_fixture(
        "catalog_truncated",
        &["sr", "fields", "no such item anywhere"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no catalog item matches 'no such item anywhere'; list them with `fsv sr catalog --search TEXT` (catalog list truncated: read 50 of 99 pages of 30 items (limit 50 pages); more items exist and 
are not shown or searched; use the item's display id)
"#
    );
}

// Python commit: cc81f93

#[test]
fn form_fields_sr_item_json() {
    let r = run_fixture(
        "catalog",
        &["form", "fields", "sr", "--item", "78", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "label": "Reason",
    "name": "reason_200",
    "kind": "text",
    "custom": true,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Access level",
    "name": "access_level",
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
    "label": "Justification",
    "name": "justification",
    "kind": "textarea",
    "custom": true,
    "required_static": false,
    "visible_on_load": false,
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
fn form_fields_sr_item_table() {
    let r = run_fixture("catalog", &["form", "fields", "sr", "--item", "78"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"       service_request form fields (web order; state on a new form)        
┏━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━┓
┃ Label         ┃ Name          ┃ Type         ┃ On load           ┃ Help ┃
┡━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━┩
│ Reason        │ reason_200    │ text         │ visible, required │      │
│ Access level  │ access_level  │ dropdown (3) │ visible, required │      │
│ Justification │ justification │ textarea     │ hidden            │      │
└───────────────┴───────────────┴──────────────┴───────────────────┴──────┘
choices: fsv form fields sr --item 78 --choices LABEL
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_fields_sr_item_choices_json() {
    let r = run_fixture(
        "catalog",
        &[
            "form",
            "fields",
            "sr",
            "--item",
            "78",
            "--choices",
            "Access level",
            "-o",
            "json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "field": "access_level",
  "label": "Access level",
  "choices": [
    {
      "label": "Read only",
      "value": "Read only",
      "id": "l1",
      "description": ""
    },
    {
      "label": "Read  write",
      "value": "Read  write",
      "id": "l2",
      "description": ""
    },
    {
      "label": "Admin",
      "value": "Admin",
      "id": "l3",
      "description": ""
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_sr_item_json() {
    let r = run_fixture(
        "catalog",
        &[
            "form",
            "explain",
            "sr",
            "--item",
            "78",
            "--set",
            "Access level=Admin",
            "-o",
            "json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        masked(&r.out),
        r#"{
  "rows": [
    {
      "field": "reason_200",
      "label": "Reason",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "",
      "populated_by": null,
      "why": []
    },
    {
      "field": "access_level",
      "label": "Access level",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "Admin",
      "populated_by": null,
      "why": []
    },
    {
      "field": "justification",
      "label": "Justification",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "",
      "populated_by": null,
      "why": [
        "visible (show by rule 'Admin needs a justification' (id 9100001) on change of access_level)",
        "required (mandate by rule 'Admin needs a justification' (id 9100001) on change of access_level)"
      ]
    }
  ],
  "messages": [],
  "conflicts": [],
  "errors": [],
  "metadata": {
    "updated_at": "2030-01-10T10:00:00+09:00",
    "fetched_at": "<fetched>",
    "source": "server",
    "fingerprint": "26af10946d8965d2"
  },
  "form_notes": []
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_sr_item_table() {
    let r = run_fixture(
        "catalog",
        &[
            "form",
            "explain",
            "sr",
            "--item",
            "78",
            "--set",
            "Access level=Admin",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                                                                                       service_request form (new)                                                                                       
┏━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Field         ┃ State             ┃ Value ┃ Why                                                                                                                                                      ┃
┡━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┩
│ Reason        │ visible, required │       │ form metadata                                                                                                                                            │
│ Access level  │ visible, required │ Admin │ form metadata                                                                                                                                            │
│ Justification │ visible, required │       │ visible (show by rule 'Admin needs a justification' (id 9100001) on change of access_level); required (mandate by rule 'Admin needs a justification' (id │
│               │                   │       │ 9100001) on change of access_level)                                                                                                                      │
└───────────────┴───────────────────┴───────┴──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_fields_sr_needs_item() {
    let r = run_fixture("catalog", &["form", "fields", "sr"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: sr forms are per catalog item: pass --item ID (the display id or exact name)
"#
    );
}

#[test]
fn form_explain_sr_refuses_id() {
    let r = run_fixture(
        "catalog",
        &["form", "explain", "sr", "--item", "78", "--id", "SR-1"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --id does not apply to sr item forms; use --item
"#
    );
}

#[test]
fn form_fields_item_only_for_sr() {
    let r = run_fixture(
        "catalog",
        &["form", "fields", "changes", "--item", "78"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --item applies to sr forms only
"#
    );
}

#[test]
fn form_dump_sr_refuses_edit() {
    let r = run_fixture(
        "catalog",
        &["form", "dump", "sr", "--item", "78", "--mode", "edit"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: sr item forms have no edit form here; use --item without --id/--mode edit
"#
    );
}

#[test]
fn form_diff_sr_same() {
    let r = run_files(
        "catalog",
        "form_sr_snapshots",
        &["form", "diff", "sr78.json", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_diff_sr_added() {
    let r = run_files(
        "catalog",
        "form_sr_snapshots",
        &["form", "diff", "sr78_nojust.json", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "kind": "field",
    "change": "added",
    "name": "justification",
    "label": "Justification"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_diff_sr_added_table() {
    let r = run_files(
        "catalog",
        "form_sr_snapshots",
        &["form", "diff", "sr78_nojust.json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"+ field Justification (justification)
1 difference(s)
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_diff_sr_needs_item() {
    let r = run_files(
        "catalog",
        "form_sr_snapshots",
        &["form", "diff", "sr_noitem.json"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: sr_noitem.json is an sr snapshot without its catalog item; dump it again with --item
"#
    );
}

#[test]
fn form_fields_problems_json() {
    let r = run_fixture(
        "problem_flows",
        &["form", "fields", "problems", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
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
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Requester",
    "name": "requester",
    "kind": "lookup_user",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
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
    "choices": 1,
    "choice_descriptions": []
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
    "choices": 1,
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
    "choices": 1,
    "choice_descriptions": []
  },
  {
    "label": "Acme count",
    "name": "acme_count",
    "kind": "number",
    "custom": true,
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
fn form_fields_problems_table() {
    let r = run_fixture("problem_flows", &["form", "fields", "problems"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"         problem form fields (web order; state on a new form)          
┏━━━━━━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━┓
┃ Label       ┃ Name        ┃ Type         ┃ On load           ┃ Help ┃
┡━━━━━━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━┩
│ Subject     │ subject     │ text         │ visible, required │      │
│ Description │ description │ richtext     │ visible, required │      │
│ Requester   │ requester   │ lookup_user  │ visible, required │      │
│ Status      │ status      │ dropdown (1) │ visible, required │      │
│ Priority    │ priority    │ dropdown (1) │ visible, required │      │
│ Impact      │ impact      │ dropdown (1) │ visible, required │      │
│ Acme count  │ acme_count  │ number       │ visible           │      │
└─────────────┴─────────────┴──────────────┴───────────────────┴──────┘
choices: fsv form fields problems --choices LABEL
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_problems_json() {
    let r = run_fixture(
        "problem_flows",
        &[
            "form",
            "explain",
            "problems",
            "--set",
            "Subject=Acme",
            "-o",
            "json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        masked(&r.out),
        r#"{
  "rows": [
    {
      "field": "subject",
      "label": "Subject",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "Acme",
      "populated_by": null,
      "why": []
    },
    {
      "field": "description",
      "label": "Description",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "",
      "populated_by": null,
      "why": []
    },
    {
      "field": "requester",
      "label": "Requester",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "",
      "populated_by": null,
      "why": []
    },
    {
      "field": "status",
      "label": "Status",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "Open",
      "populated_by": null,
      "why": []
    },
    {
      "field": "priority",
      "label": "Priority",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "Low",
      "populated_by": null,
      "why": []
    },
    {
      "field": "impact",
      "label": "Impact",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "Low",
      "populated_by": null,
      "why": []
    },
    {
      "field": "acme_count",
      "label": "Acme count",
      "visible": true,
      "required": false,
      "disabled": false,
      "value": "",
      "populated_by": null,
      "why": []
    }
  ],
  "messages": [],
  "conflicts": [],
  "errors": [],
  "metadata": {
    "updated_at": null,
    "fetched_at": "<fetched>",
    "source": "server",
    "fingerprint": "5f69ea44344a3ffc"
  },
  "form_notes": []
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_diff_problems_added() {
    let r = run_files(
        "problem_flows",
        "form_problem_snapshots",
        &["form", "diff", "problem.json", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "kind": "field",
    "change": "added",
    "name": "acme_count",
    "label": "Acme count"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_fields_problems_rules_unavailable() {
    let r = run_fixture(
        "problem_flows_rules500",
        &["form", "fields", "problems", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
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
    "help": "",
    "choices": 0,
    "choice_descriptions": []
  },
  {
    "label": "Requester",
    "name": "requester",
    "kind": "lookup_user",
    "custom": false,
    "required_static": true,
    "visible_on_load": true,
    "required_on_load": true,
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
    "choices": 1,
    "choice_descriptions": []
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
    "choices": 1,
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
    "choices": 1,
    "choice_descriptions": []
  },
  {
    "label": "Acme count",
    "name": "acme_count",
    "kind": "number",
    "custom": true,
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
    assert_eq!(
        r.err,
        r#"note: business rules not loaded (the workspace-less path answered HTTP 500 (server error)); checking static required/visible flags only (override the workspace with `fsv config set forms.workspace_id 
N`)
"#
    );
}
