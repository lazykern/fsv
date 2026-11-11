"""Dump the Python fsv Click tree to JSON (fixture for the clap tree test).

Run from the Python worktree:
    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/crates/fsv-cli/tests/golden/dump_cli_tree.py \
        > ../fsv-rust/crates/fsv-cli/tests/fixtures/cli_tree.json
"""
import enum
import json


from typer.core import TyperOption
import typer.main

from fsv.cli import app


def _default(v):
    if isinstance(v, enum.Enum):
        return v.value
    if callable(v):
        return None
    if isinstance(v, (list, tuple)):
        return list(v) or None
    return v


def _param(p):
    t = p.type
    d = {
        "name": p.name,
        "kind": "option" if isinstance(p, TyperOption) else "argument",
        "opts": list(p.opts),
        "secondary_opts": list(getattr(p, "secondary_opts", [])),
        "type": "choice" if hasattr(t, "choices") else t.name,
        "choices": list(t.choices) if hasattr(t, "choices") else None,
        "required": bool(p.required),
        "multiple": bool(getattr(p, "multiple", False)),
        "nargs": p.nargs,
        "is_flag": bool(getattr(p, "is_flag", False)),
        "default": _default(p.default),
        "hidden": bool(getattr(p, "hidden", False)),
        "help": getattr(p, "help", None),
        "metavar": p.metavar,
        "envvar": p.envvar,
    }
    return d


def _walk(cmd, path):
    node = {
        "path": path,
        "help": cmd.help or "",
        "epilog": cmd.epilog,
        "hidden": bool(cmd.hidden),
        "group": hasattr(cmd, "commands"),
        "params": [_param(p) for p in cmd.params if p.name not in ("help",)],
        "commands": [],
    }
    if hasattr(cmd, "commands"):
        for name, sub in cmd.commands.items():
            node["commands"].append(_walk(sub, f"{path} {name}"))
    return node


root = typer.main.get_command(app)
print(json.dumps(_walk(root, "fsv"), indent=1, ensure_ascii=False))
