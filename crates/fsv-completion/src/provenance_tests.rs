//! Every golden fixture this crate loads records the Python commit it was
//! generated from (`port/golden/provenance.py`). The `emit_expected.*`
//! scripts are shell text, so `emit_expected.provenance.json` carries their
//! stamp. `dynamic_input.json` is input, not Python output.

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
            "dynamic_expected",
            include_str!("../tests/fixtures/dynamic_expected.json"),
        ),
        (
            "emit_expected",
            include_str!("../tests/fixtures/emit_expected.provenance.json"),
        ),
        (
            "extra_expected",
            include_str!("../tests/fixtures/extra_expected.json"),
        ),
        (
            "helpers_expected",
            include_str!("../tests/fixtures/helpers_expected.json"),
        ),
        (
            "matrix_expected",
            include_str!("../tests/fixtures/matrix_expected.json"),
        ),
        (
            "walk_expected",
            include_str!("../tests/fixtures/walk_expected.json"),
        ),
    ] {
        assert_stamped(name, text);
    }
}
