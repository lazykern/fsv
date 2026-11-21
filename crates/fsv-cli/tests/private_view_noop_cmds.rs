//! `private-views write ID edit --name CURRENT`: Python 84ffb0f prints
//! `nothing_to_change` and exits 0; with `--expect-plan` it refuses (exit 2).

mod common;
use common::run_fixture;
use serde_json::json;

#[test]
fn rename_to_the_current_name_is_a_no_op() {
    for extra in [&["--dry-run"][..], &["--yes"][..]] {
        let mut args = vec![
            "tickets",
            "private-views",
            "write",
            "703",
            "edit",
            "--name",
            "Acme source",
        ];
        args.extend_from_slice(extra);
        let r = run_fixture("private_views", &args, None);
        assert_eq!(r.code, 0, "{}", r.err);
        assert_eq!(
            r.json(),
            json!({
                "status": "nothing_to_change", "view_id": 703,
                "message": "the view is already named 'Acme source'; nothing to change",
                "tenant": "acme.freshservice.com", "config_dir": "<CONFIG_DIR>",
            })
        );
        assert!(r.write_paths().is_empty());
    }
}

#[test]
fn rename_to_the_current_name_with_expect_plan_is_refused() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "703",
            "edit",
            "--name",
            "Acme source",
            "--expect-plan",
            "abc",
        ],
        None,
    );
    assert_eq!(r.code, 2);
    assert_eq!(r.out, "");
    assert_eq!(
        r.err,
        "nothing to change; the reviewed plan does not exist. Nothing was sent\n"
    );
}
