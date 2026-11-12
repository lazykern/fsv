//! Every golden fixture this crate loads records the Python commit it was
//! generated from (`port/golden/provenance.py`).

use serde_json::Value;

fn assert_stamped(name: &str, text: &str) {
    let doc: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let rev = &doc["_python"];
    let commit = rev["commit"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: no _python.commit"));
    assert!(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "{name}: {commit}"
    );
    assert!(rev["dirty"].is_boolean(), "{name}: _python.dirty");
}

#[test]
fn fixtures_record_their_python_commit() {
    for (name, text) in [
        ("flows", include_str!("../tests/golden/flows.json")),
        ("sr", include_str!("../tests/golden/sr.json")),
    ] {
        assert_stamped(name, text);
    }
}
