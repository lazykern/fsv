//! Generated completion scripts match `completion_gen.py` output
//! (`tests/golden/dump_completion.py`, executable replaced by `@FSV_EXE@`).

fn check(shell: &str, want: &str) {
    let tree = fsv_cli::completion_tree("fsv");
    let got = fsv_completion::build_script(shell, &tree, "fsv", "@FSV_EXE@").unwrap();
    if got != want {
        let line = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(got.lines().count().min(want.lines().count()));
        panic!(
            "{shell} differs at line {}:\n  rust:   {:?}\n  python: {:?}",
            line + 1,
            got.lines().nth(line),
            want.lines().nth(line)
        );
    }
}

#[test]
fn fish_matches_python() {
    check("fish", include_str!("fixtures/completion.fish"));
}

#[test]
fn bash_matches_python() {
    check("bash", include_str!("fixtures/completion.bash"));
}

#[test]
fn zsh_matches_python() {
    check("zsh", include_str!("fixtures/completion.zsh"));
}

#[test]
fn unknown_shell_is_an_error() {
    let tree = fsv_cli::completion_tree("fsv");
    assert_eq!(
        fsv_completion::build_script("pwsh", &tree, "fsv", "x").unwrap_err(),
        "unsupported shell: pwsh"
    );
}
