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
            "session_expected",
            include_str!("../tests/fixtures/session_expected.json"),
        ),
        (
            "strip_html",
            include_str!("../tests/fixtures/strip_html.provenance.json"),
        ),
        (
            "term_wrap",
            include_str!("../tests/fixtures/term_wrap.provenance.json"),
        ),
    ] {
        assert_stamped(name, text);
    }
}
