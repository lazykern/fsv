//! Forms, catalog and approvals CLI cases (batch "forms"). Expected output
//! was captured from the Python fsv on the same routes (`cmp.py --emit`).

mod common;
use common::run_fixture;

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
