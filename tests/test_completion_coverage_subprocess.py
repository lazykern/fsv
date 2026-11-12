"""Thin entrypoint subprocess protocol; collect branch data during coverage runs.

Each child has synthetic tenant caches, denied DNS, no keychain and an import
blocker proving completion does not load Typer. Parallel data files sit beside
the active coverage data, so pytest-cov's final combine includes the children.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from fsv import cache, config


@pytest.fixture
def thin(tmp_path):
    cache.save(config.schema_cache_path("changes"), "schema", [
        {"name": "status", "label": "State", "field_type": "default_status", "default_field": True,
         "choices": [{"id": 47, "value": "Review"}]},
        {"name": "test_plan", "label": "Test Plan", "field_type": "planning_field"}])
    cache.save(config.groups_cache_path(), "groups", {"groups": [{"id": 91, "name": "Acme Ops", "description": "Acme support"}]})
    cache.save(config.filters_cache_path("changes"), "filters", {"filters": [{"id": "all", "name": "All Changes"}]})
    guard = tmp_path / "guard"
    guard.mkdir()
    (guard / "sitecustomize.py").write_text(
        "import socket,sys\n"
        "def denied(*a, **k):\n    raise AssertionError('network disabled in completion child')\n"
        "socket.getaddrinfo = denied\n"
        "class NoTyper:\n"
        "    def find_spec(self, fullname, path=None, target=None):\n"
        "        if fullname == 'typer' or fullname.startswith('typer.'):\n"
        "            raise AssertionError('thin completion imported Typer')\n"
        "sys.meta_path.insert(0, NoTyper())\n")
    env = {**os.environ, "FSV_COMPLETION_NETWORK": "0", "FSV_NO_KEYCHAIN": "1",
           "PYTHONPATH": str(guard) + os.pathsep + str(Path(__file__).resolve().parents[1] / "src")}
    env.pop("_FSV_COMPLETE", None)
    env.pop("FSV_THIN_COMPLETE", None)
    prefix = [sys.executable]
    if importlib.util.find_spec("coverage") is not None:
        import coverage
        active = coverage.Coverage.current()
        env["COVERAGE_FILE"] = active.config.data_file if active else str(tmp_path / ".coverage")
        prefix += ["-m", "coverage", "run", "--parallel-mode", "--branch", "--source=fsv"]
    def run(*args):
        result = subprocess.run([*prefix, "-m", "fsv._complete", *args], env=env, text=True, capture_output=True, timeout=20)
        assert result.returncode == 0, result.stderr
        assert result.stderr == "", result.stderr
        return result.stdout.splitlines()
    return run


@pytest.mark.parametrize("shell,expected", [("fish", ["json\tJSON array"]), ("bash", ["json"]), ("zsh", ["json"])])
def test_thin_static_shell_protocol_has_no_raw_token_or_typer(thin, shell, expected):
    assert thin(shell, "complete_format", "-", "j") == expected


@pytest.mark.parametrize("name,token,expected", [
    ("complete_where", "sta", ["status=\tState · default_status · default"]),
    ("complete_set", "status=R", ["status=Review\tstatus"]),
    ("complete_filter_name", "All", ["all\tAll Changes"]),
    ("complete_field_names", "sta", ["status\tState · default_status · default"]),
    ("complete_choice_field_names", "sta", ["status\tState · default_status · default · 1 choices"]),
    ("complete_planning_field_names", "Test", ["Test Plan\ttest_plan", "test_plan\tTest Plan"]),
    ("complete_lookup_kind", "gr", ["group\tgroup lookup"]),
    ("complete_update_group_id", "Ac", ["Acme Ops\tid=91 · Acme support"]),
    ("complete_update_agent_id", "al", []),
])
def test_thin_dynamic_factory_uses_synthetic_cache(thin, name, token, expected):
    assert thin("fish", name, "changes", token) == expected


@pytest.mark.parametrize("tokens", [["--kind", "group"], ["-k", "group"], ["--kind=group"], ["-k=group"]])
def test_lookup_kind_context_extracts_separate_and_equals_flags(thin, tokens):
    assert thin("fish", "complete_lookup_query", "changes", "Ac", "--", "changes", "lookup", *tokens) == ["Acme Ops\tAcme support"]


@pytest.mark.parametrize("args", [[], ["fish"], ["fish", "complete_where", "changes"],
    ["fish", "unknown", "changes", ""], ["fish", "complete_where", "not-a-resource", "st"],
    ["fish", "complete_format", "-", "json"], ["fish", "complete_config_value", "-", "off"],
    ["fish", "complete_lookup_query", "changes", "Ac", "--", "--kind"],
    ["fish", "complete_lookup_query", "changes", "Ac", "--", "unrelated"]])
def test_thin_empty_invalid_and_exact_token_requests_are_silent(thin, args):
    assert thin(*args) == []


def test_thin_malformed_optional_config_is_silent_not_a_traceback(thin):
    config.CONFIG_FILE.write_text(json.dumps({"domain": config.DOMAIN, "completion": None}))
    assert thin("fish", "complete_update_agent_id", "-", "al") == []
