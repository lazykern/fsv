//! `tree_from_clap` and the emitters against `fsv.completion_gen` run on an
//! equivalent Typer app (`/tmp/fsv_port/completion_gen_oracle.py`). Python's
//! `<python> -m fsv._complete` call is rewritten to `<exe> _complete` in the
//! fixtures; everything else is byte-for-byte Python output.

use clap::{Arg, ArgAction, Command};
use serde_json::{Value, json};

use crate::{Kind, Node, OptionHint, build_script, emit_bash, emit_fish, emit_zsh, tree_from_clap};

const WALK: &str = include_str!("../tests/fixtures/walk_expected.json");
pub(crate) const FISH: &str = include_str!("../tests/fixtures/emit_expected.fish");
const BASH: &str = include_str!("../tests/fixtures/emit_expected.bash");
const ZSH: &str = include_str!("../tests/fixtures/emit_expected.zsh");
pub(crate) const EXE: &str = "/opt/fsv/bin/fsv";
pub(crate) const PROG: &str = "my-fsv";

const fn hint(
    id: &'static str,
    flags: &'static [&'static str],
    completer: Option<&'static str>,
    resource: Option<&'static str>,
) -> OptionHint {
    OptionHint {
        id,
        flags,
        completer,
        resource,
        value_flag: false,
    }
}

pub(crate) static HINTS: &[(&str, &str, &[OptionHint])] = &[
    (
        "fsv",
        "",
        &[
            hint("verbose", &["--verbose", "-v"], None, None),
            hint("format", &["--format", "-f"], Some("complete_format"), None),
        ],
    ),
    (
        "fsv whoami",
        "Who am I?",
        &[hint("store", &["--store"], Some("complete_store"), None)],
    ),
    (
        "fsv changes",
        "",
        &[hint("debug", &["--debug"], None, None)],
    ),
    (
        "fsv changes list",
        "List changes.",
        &[
            hint(
                "where",
                &["--where", "-w"],
                Some("complete_where"),
                Some("changes"),
            ),
            hint(
                "kind",
                &["--kind", "-k"],
                Some("complete_lookup_query"),
                Some("changes"),
            ),
            hint("limit", &["--limit"], None, None),
            OptionHint {
                value_flag: true,
                ..hint("yes", &["--yes", "-y"], None, None)
            },
        ],
    ),
    ("fsv changes show", "Show one change's details.", &[]),
    (
        "fsv config set",
        "Set it.",
        &[
            hint("key", &["--key"], Some("complete_config_key"), None),
            hint("value", &["--value"], Some("complete_config_value"), None),
            hint(
                "group",
                &["--group"],
                Some("complete_update_group_id"),
                None,
            ),
        ],
    ),
];

fn flag(id: &'static str) -> Arg {
    Arg::new(id).long(id).action(ArgAction::SetTrue)
}

fn opt(id: &'static str) -> Arg {
    Arg::new(id).long(id).action(ArgAction::Set)
}

pub(crate) fn app() -> Command {
    Command::new("fsv")
        .arg(flag("verbose").short('v'))
        .arg(opt("format").short('f'))
        .subcommand(Command::new("whoami").arg(opt("store")))
        .subcommand(
            Command::new("changes")
                .arg(flag("debug"))
                .subcommand(
                    Command::new("list")
                        .arg(opt("where").short('w').action(ArgAction::Append))
                        .arg(opt("kind").short('k'))
                        .arg(opt("limit"))
                        .arg(flag("yes").short('y'))
                        .arg(Arg::new("query")),
                )
                .subcommand(Command::new("show").arg(Arg::new("id").required(true))),
        )
        .subcommand(
            Command::new("config").subcommand(
                Command::new("set")
                    .arg(opt("key"))
                    .arg(opt("value"))
                    .arg(opt("group")),
            ),
        )
}

pub(crate) fn dump(node: &Node) -> Value {
    let params: Vec<Value> = node
        .params
        .iter()
        .map(|p| match &p.kind {
            Kind::Boolean => json!({"flags": p.flags, "kind": "boolean"}),
            Kind::Freeform => json!({"flags": p.flags, "kind": "freeform"}),
            Kind::Static(values) => json!({"flags": p.flags, "kind": "static", "values": values}),
            Kind::Dynamic {
                completer,
                resource,
            } => json!({"flags": p.flags, "kind": "dynamic", "completer": completer, "resource": resource}),
        })
        .collect();
    json!({
        "path": node.path,
        "help": node.help,
        "params": params,
        "subcommands": node.subcommands.iter().map(dump).collect::<Vec<_>>(),
    })
}

pub(crate) fn expected_tree(key: &str) -> Value {
    let all: Value = serde_json::from_str(WALK).expect("walk fixture");
    all[key].clone()
}

pub(crate) fn tree() -> Node {
    tree_from_clap(&app(), PROG, HINTS, &["-v", "--verbose"])
}

#[test]
fn walk_matches_python_with_inherited_flags() {
    assert_eq!(dump(&tree()), expected_tree("inherit_verbose"));
}

#[test]
fn walk_without_inherit_flags_passes_every_option_down() {
    assert_eq!(
        dump(&tree_from_clap(&app(), PROG, HINTS, &[])),
        expected_tree("inherit_none")
    );
}

#[test]
fn walk_reads_bool_and_value_flags() {
    let root = tree();
    let list = &root.subcommands[1].subcommands[0];
    assert_eq!(list.path, ["my-fsv", "changes", "list"]);
    let kinds: Vec<&Kind> = list.params.iter().map(|p| &p.kind).collect();
    assert_eq!(kinds[0], &Kind::Boolean);
    assert_eq!(kinds[3], &Kind::Freeform);
    // `Optional[bool]` (`value_flag`) takes a value even though clap sees a switch.
    assert_eq!(kinds[4], &Kind::Freeform);
}

#[test]
fn fish_matches_python() {
    assert_eq!(emit_fish(&tree(), PROG, EXE), FISH);
}

#[test]
fn bash_matches_python() {
    assert_eq!(emit_bash(&tree(), PROG, EXE), BASH);
}

#[test]
fn zsh_matches_python() {
    assert_eq!(emit_zsh(&tree(), PROG, EXE), ZSH);
}

#[test]
fn build_script_dispatches_by_shell() {
    let root = tree();
    assert_eq!(build_script("fish", &root, PROG, EXE).unwrap(), FISH);
    assert_eq!(build_script("bash", &root, PROG, EXE).unwrap(), BASH);
    assert_eq!(build_script("zsh", &root, PROG, EXE).unwrap(), ZSH);
    assert_eq!(
        build_script("powershell", &root, PROG, EXE).unwrap_err(),
        "unsupported shell: powershell"
    );
}
