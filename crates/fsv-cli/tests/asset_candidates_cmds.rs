//! `MODULE assets candidates` against canned routes (Python dd2c480,
//! `tests/golden/cmp.py --emit`): legacy 403/404 and problems use the
//! integrated inventory search.

mod common;
use common::run_fixture;

#[test]
fn asset_candidates_problems() {
    let r = run_fixture(
        "asset_candidates",
        &[
            "problems",
            "assets",
            "candidates",
            "PRB-41",
            "--search",
            "acme",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "id": 81,
      "name": "Acme laptop",
      "associated": null,
      "inventory": {
        "device_pk": 81,
        "name": {
          "id": 81,
          "label": "Acme laptop"
        },
        "type": "Laptop"
      }
    },
    {
      "id": 82,
      "name": "Globex phone",
      "associated": null,
      "inventory": {
        "device_pk": 82,
        "name": {
          "id": 82,
          "label": "Globex phone"
        },
        "type": "Phone"
      }
    }
  ],
  "driver": "integrated",
  "kind": "devices",
  "query": "acme",
  "page": 1,
  "total_rows": 2,
  "complete": true,
  "id_meaning": "inventory IDs (use with assets add)",
  "associated_note": "no association list for this record; association state unknown"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_candidates_tickets() {
    let r = run_fixture(
        "asset_candidates",
        &[
            "tickets",
            "assets",
            "candidates",
            "INC-41",
            "--search",
            "acme",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "id": 81,
      "name": "Acme laptop",
      "associated": null,
      "inventory": {
        "device_pk": 81,
        "name": {
          "id": 81,
          "label": "Acme laptop"
        },
        "type": "Laptop"
      }
    },
    {
      "id": 82,
      "name": "Globex phone",
      "associated": null,
      "inventory": {
        "device_pk": 82,
        "name": {
          "id": 82,
          "label": "Globex phone"
        },
        "type": "Phone"
      }
    }
  ],
  "driver": "integrated",
  "kind": "devices",
  "query": "acme",
  "page": 1,
  "total_rows": 2,
  "complete": true,
  "id_meaning": "inventory IDs (use with assets add)",
  "associated_note": "no association list for this record; association state unknown",
  "note": "legacy assets-to-associate answered HTTP 403; listed from the integrated inventory search instead"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_candidates_sr() {
    let r = run_fixture(
        "asset_candidates",
        &[
            "sr",
            "assets",
            "candidates",
            "SR-41",
            "--search",
            "acme",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "id": 81,
      "name": "Acme laptop",
      "associated": null,
      "inventory": {
        "device_pk": 81,
        "name": {
          "id": 81,
          "label": "Acme laptop"
        },
        "type": "Laptop"
      }
    },
    {
      "id": 82,
      "name": "Globex phone",
      "associated": null,
      "inventory": {
        "device_pk": 82,
        "name": {
          "id": 82,
          "label": "Globex phone"
        },
        "type": "Phone"
      }
    }
  ],
  "driver": "integrated",
  "kind": "devices",
  "query": "acme",
  "page": 1,
  "total_rows": 2,
  "complete": true,
  "id_meaning": "inventory IDs (use with assets add)",
  "associated_note": "no association list for this record; association state unknown",
  "note": "legacy assets-to-associate answered HTTP 403; listed from the integrated inventory search instead"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_candidates_changes() {
    let r = run_fixture(
        "asset_candidates",
        &[
            "changes",
            "assets",
            "candidates",
            "CHN-41",
            "--search",
            "acme",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "id": 81,
      "name": "Acme laptop",
      "associated": null,
      "inventory": {
        "device_pk": 81,
        "name": {
          "id": 81,
          "label": "Acme laptop"
        },
        "type": "Laptop"
      }
    },
    {
      "id": 82,
      "name": "Globex phone",
      "associated": null,
      "inventory": {
        "device_pk": 82,
        "name": {
          "id": 82,
          "label": "Globex phone"
        },
        "type": "Phone"
      }
    }
  ],
  "driver": "integrated",
  "kind": "devices",
  "query": "acme",
  "page": 1,
  "total_rows": 2,
  "complete": true,
  "id_meaning": "inventory IDs (use with assets add)",
  "associated_note": "no association list for this record; association state unknown",
  "note": "legacy assets-to-associate answered HTTP 403; listed from the integrated inventory search instead"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_candidates_bad_kind() {
    let r = run_fixture(
        "asset_candidates",
        &[
            "changes",
            "assets",
            "candidates",
            "CHN-41",
            "--kind",
            "other",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"candidate kind must be devices or services
"#
    );
}
