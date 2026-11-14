//! Forms, catalog and approvals CLI cases (batch "forms"). Expected output
//! was captured from the Python fsv on the same routes (`cmp.py --emit`).

mod common;
use common::{run_files, run_fixture, run_steps};

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

/// Python's `ItemMeta.spec` is lazy: `form dump sr` prints an item whose
/// `section_info` the spec refuses (`sr fields` still fails on it).
#[test]
fn form_dump_sr_does_not_build_the_spec() {
    let mut routes = common::fixture("catalog");
    let item = routes
        .iter_mut()
        .find(|r| {
            r["path"]
                .as_str()
                .unwrap_or("")
                .ends_with("service-items/78")
        })
        .unwrap();
    let si = if item["json"].get("service_item").is_some() {
        &mut item["json"]["service_item"]
    } else {
        &mut item["json"]
    };
    si["custom_fields"][0]["field_options"] = serde_json::json!({"section_info": "{bad"});
    let env = common::Env::new(routes);
    let r = env.run(&["form", "dump", "sr", "--item", "78"]);
    assert_eq!((r.code, r.err.as_str()), (0, ""));
    assert!(r.out.contains("\"section_info\": \"{bad\""), "{}", r.out);
    let r = env.run(&["sr", "fields", "78"]);
    assert_eq!(r.code, 2);
    assert!(
        r.err
            .ends_with("error: item field 'reason_200': unreadable section_info\n"),
        "{}",
        r.err
    );
}

/// `find_item`: `s.isdigit()` then `int(s)`, so a superscript is the
/// `ValueError` text and a display id past 64 bits is fetched as written
/// (Python runs on the same routes).
#[test]
fn sr_item_digit_refs_are_python_int() {
    let r = run_fixture("catalog", &["sr", "fields", "\u{b2}"], None);
    assert_eq!(
        (r.code, r.out.as_str(), r.err.as_str()),
        (
            2,
            "",
            "error: invalid literal for int() with base 10: '\u{b2}'\n"
        )
    );
    let r = run_fixture(
        "catalog",
        &["sr", "fields", "99999999999999999999999"],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert!(
        r.calls()
            .iter()
            .any(|(_, p)| p == "/api/_/catalog/service-items/99999999999999999999999"),
        "{:?}",
        r.calls()
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

// Python commit: ebfbdd5

#[test]
fn tickets_approvals_flat_rows_json() {
    let r = run_fixture(
        "approvals_flat",
        &["tickets", "approvals", "SR-123", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 710000000001,
    "member_id": 9000003001,
    "user_id": 9000004001,
    "level_id": 1,
    "approval_level_status": null,
    "remark": [],
    "status": {
      "id": 0,
      "name": "requested"
    },
    "type": {
      "id": 1,
      "name": "everyone"
    },
    "last_reminded_at": null,
    "change_id": 123,
    "member": {
      "id": 9000003001,
      "name": "Acme Approver",
      "email": "approver@example.com"
    },
    "level": 1,
    "approver": "Acme Approver",
    "approver_id": 9000003001,
    "status_name": "requested",
    "status_id": 0,
    "type_name": "everyone",
    "decided": "",
    "comment": ""
  },
  {
    "id": 710000000002,
    "member_id": 9000003001,
    "user_id": 9000004001,
    "level_id": 2,
    "approval_level_status": null,
    "remark": [
      {
        "delegate_id": null,
        "status": 1,
        "updated_at": "2030-01-02T03:04:05Z",
        "data": "ok [x]"
      }
    ],
    "status": {
      "id": 1,
      "name": "Genehmigt"
    },
    "type": {
      "id": 1,
      "name": "everyone"
    },
    "last_reminded_at": null,
    "change_id": 123,
    "member": {
      "id": 9000003001,
      "name": "Acme Approver",
      "email": "approver@example.com"
    },
    "level": 2,
    "approver": "Acme Approver",
    "approver_id": 9000003001,
    "status_name": "Genehmigt",
    "status_id": 1,
    "type_name": "everyone",
    "decided": "2030-01-02T03:04:05",
    "comment": "ok [x]"
  },
  {
    "id": 710000000003,
    "member_id": 9000003002,
    "level_id": 2,
    "status": {
      "id": 4
    },
    "type": 2,
    "updated_at": "2030-01-03T00:00:00Z",
    "level": 2,
    "approver": "9000003002",
    "approver_id": 9000003002,
    "status_name": "4",
    "status_id": 4,
    "type_name": "2",
    "decided": "2030-01-03T00:00:00",
    "comment": ""
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_approvals_flat_rows_table() {
    let r = run_fixture("approvals_flat", &["tickets", "approvals", "SR-123"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┓
┃ Level ┃ Approver      ┃ Status    ┃ Decided             ┃ Type     ┃ Comment ┃
┡━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━┩
│ 1     │ Acme Approver │ requested │ -                   │ everyone │ -       │
│ 2     │ Acme Approver │ Genehmigt │ 2030-01-02T03:04:05 │ everyone │ ok [x]  │
│ 2     │ 9000003002    │ 4         │ 2030-01-03T00:00:00 │ 2        │ -       │
└───────┴───────────────┴───────────┴─────────────────────┴──────────┴─────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_approvals_flat_rows_csv() {
    let r = run_fixture(
        "approvals_flat",
        &["tickets", "approvals", "SR-123", "-o", "csv"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "level,approver,approver_id,status_name,status_id,type_name,decided,comment\r\n1,Acme Approver,9000003001,requested,0,everyone,,\r\n2,Acme Approver,9000003001,Genehmigt,1,everyone,2030-01-02T03:04:05,ok [x]\r\n2,9000003002,9000003002,4,4,2,2030-01-03T00:00:00,\r\n"
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_approvals_list_flat_rows_json() {
    let r = run_fixture(
        "approvals_flat",
        &["sr", "approvals", "list", "SR-123", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "model": "legacy",
  "note": "this tenant uses legacy flat approvals (approval-chains answered HTTP 400)",
  "approvals": [
    {
      "id": 710000000001,
      "member_id": 9000003001,
      "user_id": 9000004001,
      "level_id": 1,
      "approval_level_status": null,
      "remark": [],
      "status": {
        "id": 0,
        "name": "requested"
      },
      "type": {
        "id": 1,
        "name": "everyone"
      },
      "last_reminded_at": null,
      "change_id": 123,
      "member": {
        "id": 9000003001,
        "name": "Acme Approver",
        "email": "approver@example.com"
      },
      "level": 1,
      "approver": "Acme Approver",
      "approver_id": 9000003001,
      "status_name": "requested",
      "status_id": 0,
      "type_name": "everyone",
      "decided": "",
      "comment": ""
    },
    {
      "id": 710000000002,
      "member_id": 9000003001,
      "user_id": 9000004001,
      "level_id": 2,
      "approval_level_status": null,
      "remark": [
        {
          "delegate_id": null,
          "status": 1,
          "updated_at": "2030-01-02T03:04:05Z",
          "data": "ok [x]"
        }
      ],
      "status": {
        "id": 1,
        "name": "Genehmigt"
      },
      "type": {
        "id": 1,
        "name": "everyone"
      },
      "last_reminded_at": null,
      "change_id": 123,
      "member": {
        "id": 9000003001,
        "name": "Acme Approver",
        "email": "approver@example.com"
      },
      "level": 2,
      "approver": "Acme Approver",
      "approver_id": 9000003001,
      "status_name": "Genehmigt",
      "status_id": 1,
      "type_name": "everyone",
      "decided": "2030-01-02T03:04:05",
      "comment": "ok [x]"
    },
    {
      "id": 710000000003,
      "member_id": 9000003002,
      "level_id": 2,
      "status": {
        "id": 4
      },
      "type": 2,
      "updated_at": "2030-01-03T00:00:00Z",
      "level": 2,
      "approver": "9000003002",
      "approver_id": 9000003002,
      "status_name": "4",
      "status_id": 4,
      "type_name": "2",
      "decided": "2030-01-03T00:00:00",
      "comment": ""
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_approvals_list_flat_rows_table() {
    let r = run_fixture(
        "approvals_flat",
        &["sr", "approvals", "list", "SR-123"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━┳━━━━━━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┓
┃ Level ┃ Approver      ┃ Status    ┃ Decided             ┃ Type     ┃ Comment ┃
┡━━━━━━━╇━━━━━━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━┩
│ 1     │ Acme Approver │ requested │ -                   │ everyone │ -       │
│ 2     │ Acme Approver │ Genehmigt │ 2030-01-02T03:04:05 │ everyone │ ok [x]  │
│ 2     │ 9000003002    │ 4         │ 2030-01-03T00:00:00 │ 2        │ -       │
└───────┴───────────────┴───────────┴─────────────────────┴──────────┴─────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"this tenant uses legacy flat approvals (approval-chains answered HTTP 400)
"#
    );
}

// Python commit: ebfbdd5 (fb2b13c behavior: record workspace notes)

#[test]
fn form_explain_record_workspace_notes() {
    let r = run_steps(
        "form_record_workspace",
        &[&["config", "set", "forms.workspace_id", "5"]],
        &["form", "explain", "problems", "--id", "PRB-77"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                        problem form (edit)                         
┏━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━┓
┃ Field       ┃ State             ┃ Value          ┃ Why           ┃
┡━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━┩
│ Subject     │ visible, required │ Globex problem │ form metadata │
│ Description │ visible, required │                │ form metadata │
│ Requester   │ visible, required │                │ form metadata │
│ Status      │ visible, required │                │ form metadata │
│ Priority    │ visible, required │                │ form metadata │
│ Impact      │ visible, required │                │ form metadata │
│ Acme count  │ visible           │                │ form metadata │
└─────────────┴───────────────────┴────────────────┴───────────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"note: the record is in workspace 9 but the default problem form's fields belong to workspace 5; business rules come from workspace 9, the field list is the default form's
note: forms.workspace_id=5 does not apply to this record: it carries its own workspace 9
"#
    );
}

#[test]
fn form_explain_record_workspace_notes_json() {
    let r = run_steps(
        "form_record_workspace",
        &[&["config", "set", "forms.workspace_id", "5"]],
        &[
            "form", "explain", "problems", "--id", "PRB-77", "-o", "json",
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
      "value": "Globex problem",
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
      "value": "",
      "populated_by": null,
      "why": []
    },
    {
      "field": "priority",
      "label": "Priority",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "",
      "populated_by": null,
      "why": []
    },
    {
      "field": "impact",
      "label": "Impact",
      "visible": true,
      "required": true,
      "disabled": false,
      "value": "",
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
    "fingerprint": "552f641fd20f9acd"
  },
  "form_notes": [
    "the record is in workspace 9 but the default problem form's fields belong to workspace 5; business rules come from workspace 9, the field list is the default form's",
    "forms.workspace_id=5 does not apply to this record: it carries its own workspace 9"
  ]
}
"#
    );
    assert_eq!(
        r.err,
        r#"note: the record is in workspace 9 but the default problem form's fields belong to workspace 5; business rules come from workspace 9, the field list is the default form's
note: forms.workspace_id=5 does not apply to this record: it carries its own workspace 9
"#
    );
}
