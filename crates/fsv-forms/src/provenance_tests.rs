//! Every golden fixture this crate loads records the Python commit it was
//! generated from (`port/golden/provenance.py`); the generated tables carry
//! it as a header line.

use serde_json::Value;

fn assert_commit(name: &str, commit: &str) {
    assert!(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "{name}: {commit}"
    );
}

fn assert_stamped(name: &str, text: &str) {
    let doc: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let rev = &doc["_python"];
    let commit = rev["commit"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: no _python.commit"));
    assert_commit(name, commit);
    assert!(rev["dirty"].is_boolean(), "{name}: _python.dirty");
}

#[test]
fn fixtures_record_their_python_commit() {
    for (name, text) in [
        ("acme", include_str!("../testdata/acme.json")),
        (
            "problem_expected",
            include_str!("../testdata/problem_expected.json"),
        ),
        ("py_golden", include_str!("../testdata/py_golden.json")),
        (
            "resolver_expected",
            include_str!("../testdata/resolver_expected.json"),
        ),
    ] {
        assert_stamped(name, text);
    }
}

#[test]
fn generated_tables_record_their_python_commit() {
    for (name, text) in [
        ("casefold.rs", include_str!("tables/casefold.rs")),
        ("digits.rs", include_str!("tables/digits.rs")),
    ] {
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix("// Python commit: "))
            .unwrap_or_else(|| panic!("{name}: no Python commit header"));
        assert_commit(name, line.trim_end_matches(" (dirty)"));
    }
}
