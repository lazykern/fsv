//! Every golden fixture this crate loads records the Python commit it was
//! generated from (`port/golden/provenance.py`). A fixture that is a JSON
//! list records it in its sibling `<stem>.provenance.json`.

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
            "attachment_rendering",
            include_str!("../tests/fixtures/attachment_rendering.json"),
        ),
        (
            "properties_expected",
            include_str!("../tests/fixtures/properties_expected.provenance.json"),
        ),
        (
            "views_expected",
            include_str!("../tests/fixtures/views_expected.json"),
        ),
        (
            "views_extra",
            include_str!("../tests/fixtures/views_extra.json"),
        ),
    ] {
        assert_stamped(name, text);
    }
}
