//! The generated entity table records the Python commit it was generated
//! from (`port/golden/provenance.py`).

#[test]
fn html_entities_record_their_python_commit() {
    let commit = include_str!("html_entities.rs")
        .lines()
        .find_map(|l| l.strip_prefix("//! Python commit: "))
        .expect("no Python commit header")
        .trim_end_matches(" (dirty)");
    assert!(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "{commit}"
    );
}
