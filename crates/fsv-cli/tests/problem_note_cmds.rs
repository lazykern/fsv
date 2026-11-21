//! Legacy `changes add-note` keeps its visibility key (Python e80a922). The
//! problem note is a reviewed write now (see `up_b_cmds.rs`).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: e80a92200bdd87666359ec6c21ab437351d05209

mod common;
use common::run_fixture;

#[test]
fn change_note_keeps_visibility_key() {
    let r = run_fixture(
        "problem_add_note",
        &["changes", "add-note", "CHN-3", "Acme change note"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, "added private note 9000000078\n");
    assert_eq!(r.err, "");
    assert_eq!(r.write_paths(), ["POST /api/_/changes/3/notes"]);
    assert_eq!(
        r.writes()[0].2,
        serde_json::json!({"body": "Acme change note", "private": true})
    );
}
