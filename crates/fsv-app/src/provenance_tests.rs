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
        (
            "properties_change_501",
            include_str!("tui/testdata/properties_change_501.json"),
        ),
        (
            "reviewed_expected",
            include_str!("tui/testdata/reviewed_expected.json"),
        ),
        (
            "form_lookup",
            include_str!("form/testdata/form_lookup.json"),
        ),
    ] {
        assert_stamped(name, text);
    }
}
