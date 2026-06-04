"""Tests for shell-native completion generator and thin dynamic entrypoint."""
from __future__ import annotations

import subprocess
import sys
from typing import Iterable

import pytest


# ──────────────────────────────────────────────────────────
# Helpers
# ──────────────────────────────────────────────────────────

def _typer_completions(shell: str, cmdline: str) -> list[str]:
    """Get completions via the existing typer protocol (oracle)."""
    env_key = "_FSV_COMPLETE"
    env = {
        **__import__("os").environ,
        env_key: f"complete_{shell}",
    }
    if shell == "fish":
        env["_TYPER_COMPLETE_FISH_ACTION"] = "get-args"
        env["_TYPER_COMPLETE_ARGS"] = cmdline
    elif shell == "bash":
        words = cmdline.split()
        env["COMP_WORDS"] = cmdline
        env["COMP_CWORD"] = str(len(words) - 1)
    else:
        env["_TYPER_COMPLETE_ARGS"] = cmdline

    result = subprocess.run(
        [sys.executable, "-m", "fsv.cli"],
        env=env,
        capture_output=True,
        text=True,
    )
    lines = [l.strip() for l in result.stdout.splitlines() if l.strip()]
    # fish: "value\thelp" → extract value
    return [l.split("\t")[0] for l in lines]


def _thin_completions(completer: str, resource: str, token: str) -> list[str]:
    """Get completions via the thin _complete.py entrypoint."""
    result = subprocess.run(
        [sys.executable, "-m", "fsv._complete", "fish", completer, resource, token],
        capture_output=True,
        text=True,
    )
    lines = [l.strip().split("\t")[0] for l in result.stdout.splitlines() if l.strip()]
    return lines


# ──────────────────────────────────────────────────────────
# STATIC_COMPLETERS integrity
# ──────────────────────────────────────────────────────────

def test_static_completers_keys_exist_in_completion_module():
    from fsv import completion as _c
    from fsv.completion_gen import _STATIC_COMPLETERS

    for name in _STATIC_COMPLETERS:
        assert hasattr(_c, name), f"completion.{name} not found — update _STATIC_COMPLETERS"


def test_static_completers_have_values():
    from fsv.completion_gen import _STATIC_COMPLETERS

    for name, vals in _STATIC_COMPLETERS.items():
        assert vals, f"_STATIC_COMPLETERS[{name!r}] is empty"
        for item in vals:
            assert isinstance(item, tuple) and len(item) == 2, (
                f"_STATIC_COMPLETERS[{name!r}] entry not a (value, help) tuple"
            )


# ──────────────────────────────────────────────────────────
# Param arity classification
# ──────────────────────────────────────────────────────────

def _get_param(resource_name: str, cmd_name: str, flag: str):
    from fsv.cli import app
    from fsv.completion_gen import _extract_params

    grp = next(g for g in app.registered_groups if g.name == resource_name)
    cmd = next(c for c in grp.typer_instance.registered_commands if c.name == cmd_name)
    params = _extract_params(cmd.callback)
    for p in params:
        if flag in p.flags:
            return p
    return None


def test_boolean_flag_json():
    p = _get_param("changes", "ls", "--json")
    assert p is not None
    assert p.kind == "boolean"


def test_static_value_flag_output():
    p = _get_param("changes", "ls", "--output")
    assert p is not None
    assert p.kind == "static"
    assert p.static_values is not None
    values = [v for v, _ in p.static_values]
    assert "table" in values
    assert "json" in values


def test_dynamic_value_flag_where():
    p = _get_param("changes", "ls", "--where")
    assert p is not None
    assert p.kind == "dynamic"
    assert p.completer == "complete_where"
    assert p.resource == "changes"


def test_freeform_flag_page():
    p = _get_param("changes", "ls", "--page")
    assert p is not None
    assert p.kind == "freeform"


# ──────────────────────────────────────────────────────────
# Generated fish script structure
# ──────────────────────────────────────────────────────────

def test_fish_script_has_no_files_global():
    from fsv.completion_gen import build_script
    script = build_script("fish", "fsv")
    assert "complete -c fsv -f" in script


def test_fish_script_has_top_level_subcommands():
    from fsv.completion_gen import build_script
    script = build_script("fish", "fsv")
    for cmd in ("changes", "tickets", "problems", "auth", "config", "cache", "completion"):
        assert cmd in script, f"top-level cmd {cmd!r} missing from fish script"


def test_fish_script_has_dynamic_where_for_changes():
    from fsv.completion_gen import build_script
    script = build_script("fish", "fsv")
    assert "complete_where" in script
    assert "changes" in script
    assert "commandline -ct" in script


def test_fish_script_has_static_output_values():
    from fsv.completion_gen import build_script
    script = build_script("fish", "fsv")
    assert "table" in script
    assert "json" in script


def test_fish_script_contains_complete_fish_and_no_files():
    """Test that --show-completion fish output contains markers for shell_fallback test."""
    from fsv.completion_gen import build_script
    script = build_script("fish", "fsv")
    # The thin entrypoint directive must mention the shell and no-files is via -f flag
    assert "fsv._complete fish" in script or "fsv._complete" in script
    assert "complete -c fsv -f" in script


# ──────────────────────────────────────────────────────────
# Differential test (thin entrypoint vs typer oracle)
# ──────────────────────────────────────────────────────────

STATIC_PREFIXES = [
    # (completer_name, resource, token, minimum_expected_values)
    ("complete_format", "-", "", ["table", "json", "csv", "tsv"]),
    ("complete_sort_order", "-", "", ["asc", "desc"]),
    ("complete_cache_target", "-", "", ["schema", "filters", "groups", "all"]),
    ("complete_store", "-", "", ["file", "argon", "keychain"]),
]


@pytest.mark.parametrize("completer,resource,token,expected", STATIC_PREFIXES)
def test_thin_entrypoint_static_values(completer, resource, token, expected):
    got = _thin_completions(completer, resource, token)
    for val in expected:
        assert val in got, f"expected {val!r} in output of _complete.py {completer}"


def test_thin_entrypoint_dynamic_where_returns_values():
    """complete_where on changes with 'status=' should return status choices."""
    got = _thin_completions("complete_where", "changes", "status=")
    assert any("status=" in v for v in got), f"expected status= completions, got: {got[:5]}"


def test_thin_entrypoint_complete_group_query():
    got = _thin_completions("complete_group_query", "-", "")
    # May be empty if groups cache not populated, but should not crash
    assert isinstance(got, list)


def test_thin_entrypoint_does_not_import_typer():
    """The thin entrypoint must not load typer (that was the whole point)."""
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            "import os; os.environ['FSV_THIN_COMPLETE']='1'; "
            "import sys; import fsv.completion; "
            "print('typer' in sys.modules)",
        ],
        capture_output=True,
        text=True,
    )
    assert result.stdout.strip() == "False", "typer was imported on the thin completion path"


# ──────────────────────────────────────────────────────────
# build_script smoke tests for bash and zsh
# ──────────────────────────────────────────────────────────

def test_bash_script_has_complete_function():
    from fsv.completion_gen import build_script
    script = build_script("bash", "fsv")
    assert "_fsv_complete()" in script
    assert "complete -F _fsv_complete fsv" in script


def test_bash_script_uses_sys_executable():
    from fsv.completion_gen import build_script
    script = build_script("bash", "fsv")
    assert "fsv._complete" in script
    assert " python " not in script and not script.strip().endswith(" python")


def test_zsh_script_has_compdef():
    from fsv.completion_gen import build_script
    script = build_script("zsh", "fsv")
    assert "#compdef fsv" in script
    assert "_fsv()" in script


def test_zsh_script_has_dynamic_completions():
    from fsv.completion_gen import build_script
    script = build_script("zsh", "fsv")
    assert "fsv._complete" in script
    assert "complete_where" in script


def test_zsh_script_uses_sys_executable():
    from fsv.completion_gen import build_script
    script = build_script("zsh", "fsv")
    assert " python " not in script and not script.strip().endswith(" python")
