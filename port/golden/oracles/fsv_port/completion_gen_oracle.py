"""Golden trees/scripts for fsv-completion's walk + emitters.

Usage: uv run python port/golden/oracles/fsv_port/completion_gen_oracle.py <fixtures_dir>

The emitted scripts share one `emit_expected.provenance.json`.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import List, Optional

import typer

from fsv import completion, completion_gen
from fsv.resources import CHANGES

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from provenance import stamped, write_sidecar  # noqa: E402

OUT = Path(sys.argv[1])
PY = "/opt/fsv/bin/fsv"

app = typer.Typer()


@app.callback()
def root(
    verbose: bool = typer.Option(False, "--verbose", "-v"),
    fmt: str = typer.Option("table", "--format", "-f", autocompletion=completion.complete_format),
) -> None:
    pass


@app.command("whoami")
def whoami(store: str = typer.Option("file", "--store", autocompletion=completion.complete_store)) -> None:
    """Who am I?

    Long text.
    """


changes = typer.Typer()


@changes.callback()
def changes_cb(debug: bool = typer.Option(False, "--debug")) -> None:
    pass


@changes.command("list")
def changes_list(
    where: Optional[List[str]] = typer.Option(None, "--where", "-w", autocompletion=completion.complete_where(CHANGES)),
    kind: str = typer.Option("", "--kind", "-k", autocompletion=completion.complete_lookup_query(CHANGES)),
    limit: int = typer.Option(10, "--limit"),
    yes: Optional[bool] = typer.Option(None, "--yes", "-y"),
    query: str = typer.Argument(""),
) -> None:
    """  List changes.  """


@changes.command("show")
def changes_show(id_: int = typer.Argument(...)) -> None:
    """Show one change's details."""


config = typer.Typer()


@config.command("set")
def config_set(
    key: str = typer.Option("", "--key", autocompletion=completion.complete_config_key),
    value: str = typer.Option("", "--value", autocompletion=completion.complete_config_value),
    group: str = typer.Option("", "--group", autocompletion=completion.complete_update_group_id),
) -> None:
    """Set it."""


app.add_typer(changes, name="changes")
app.add_typer(config, name="config")


def dump(cmd) -> dict:
    params = []
    for p in cmd.params:
        d = {"flags": p.flags, "kind": p.kind}
        if p.kind == "dynamic":
            d["completer"] = p.completer
            d["resource"] = p.resource
        if p.kind == "static":
            d["values"] = [list(v) for v in p.static_values]
        params.append(d)
    return {"path": cmd.path, "help": cmd.help, "params": params, "subcommands": [dump(s) for s in cmd.subcommands]}


trees = {
    "inherit_verbose": completion_gen._walk(app, ["my-fsv"], inherited_flags={"-v", "--verbose"}),
    "inherit_none": completion_gen._walk(app, ["my-fsv"]),
}
(OUT / "walk_expected.json").write_text(
    json.dumps(stamped({k: dump(v) for k, v in trees.items()}), indent=1, ensure_ascii=False) + "\n"
)

completion_gen.__dict__["_sys_executable"] = PY
sys.executable = PY
root_cmd = trees["inherit_verbose"]
for shell, fn in (("fish", completion_gen._emit_fish), ("bash", completion_gen._emit_bash), ("zsh", completion_gen._emit_zsh)):
    text = fn(root_cmd, "my-fsv")
    # Rust runs `<exe> _complete` instead of `<python> -m fsv._complete`.
    n = text.count(f"{PY} -m fsv._complete ")
    text = text.replace(f"{PY} -m fsv._complete ", f"{PY} _complete ")
    (OUT / f"emit_expected.{shell}").write_text(text)
    print(shell, "dynamic calls:", n)
write_sidecar(OUT / "emit_expected.bash")
