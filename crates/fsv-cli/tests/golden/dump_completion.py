"""Dump the Python completion tree and reference scripts for fsv-completion.

Run from the Python worktree:
    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/crates/fsv-cli/tests/golden/dump_completion.py \
        ../fsv-rust/crates/fsv-cli/tests/fixtures

Writes completion_tree.json and completion.{fish,bash,zsh}. Two deliberate
divergences are applied before emitting so the scripts match the Rust port:
`--x/--no-x` declarations become two flags (Python completes the literal
`--pager/--no-pager`), and the dynamic helper is `<exe> _complete` instead of
`<python> -m fsv._complete`.
"""
import json
import pathlib
import sys

from fsv import completion_gen as g
from fsv.cli import _ROOT_FLAGS_ANYWHERE, app

EXE = "@FSV_EXE@"


def split_flags(cmd):
    for p in cmd.params:
        flags = []
        for f in p.flags:
            flags.extend(f.split("/") if "/" in f else [f])
        p.flags = flags
    for s in cmd.subcommands:
        split_flags(s)


def to_json(cmd):
    return {
        "path": cmd.path,
        "help": cmd.help,
        "params": [
            {"flags": p.flags, "kind": p.kind, "completer": p.completer, "resource": p.resource}
            for p in cmd.params
        ],
        "subcommands": [to_json(s) for s in cmd.subcommands],
    }


out = pathlib.Path(sys.argv[1])
root = g._walk(app, ["fsv"], inherited_flags=set(_ROOT_FLAGS_ANYWHERE))
split_flags(root)
(out / "completion_tree.json").write_text(json.dumps(to_json(root), indent=1, ensure_ascii=False) + "\n")
sys.executable = EXE
for shell, emit in (("fish", g._emit_fish), ("bash", g._emit_bash), ("zsh", g._emit_zsh)):
    text = emit(root, "fsv").replace(f"{EXE} -m fsv._complete", f"{EXE} _complete")
    (out / f"completion.{shell}").write_text(text)
