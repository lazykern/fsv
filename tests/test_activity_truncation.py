"""`changes activity` reports truncation instead of silently cutting at --limit."""
import json
from types import SimpleNamespace

import pytest
from typer.testing import CliRunner

import fsv.cli as cli


def rows(n):
    return [{"actor": {"id": 9000001, "name": "Acme Agent"}, "content": f"set status as <b>Phase {i}</b>",
             "sub_contents": None, "created_at": f"2030-01-01T00:{i % 60:02d}:00Z"} for i in range(n)]


@pytest.fixture
def client(monkeypatch):
    data = {"rows": rows(25)}
    def get(path, params=None):
        assert path.endswith("/activities")
        page = int((params or {}).get("page", 1))
        per = int((params or {}).get("per_page", 30))
        return {"activities": data["rows"][(page - 1) * per: page * per]}
    c = SimpleNamespace(int_get=get)
    monkeypatch.setattr(cli, "_client", lambda: c)
    return data


def test_default_limit_prints_truncation_notice_in_table(client):
    r = CliRunner().invoke(cli.app, ["changes", "activity", "CHN-501"])
    assert r.exit_code == 0, r.output
    assert "truncated: 5 more (showing 20 of 25); use --limit N or --all" in " ".join(r.stdout.split())


def test_json_stdout_unchanged_and_notice_on_stderr(client):
    r = CliRunner().invoke(cli.app, ["changes", "activity", "CHN-501", "--json"])
    assert r.exit_code == 0
    assert len(json.loads(r.stdout)) == 20
    assert "truncated: 5 more" in r.stderr


@pytest.mark.parametrize("args", [["--all"], ["--limit", "25"], ["--limit", "100", "--json"]])
def test_all_or_large_limit_has_no_notice(client, args):
    r = CliRunner().invoke(cli.app, ["changes", "activity", "CHN-501", *args])
    assert r.exit_code == 0 and "truncated" not in r.output
    if "--json" in args:
        assert len(json.loads(r.stdout)) == 25
