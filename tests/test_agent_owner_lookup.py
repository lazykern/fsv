"""Synthetic native autocomplete envelopes and fail-closed task owners."""
import json
from types import SimpleNamespace

import httpx
import pytest
import typer
from typer.testing import CliRunner

from fsv import change_tasks, cli, query
from test_change_task_update import wire
from test_ticket_notes import mock_client
from task_meta import default_task_fields


NATIVE = {"id": "agent@example.com", "value": "Alex Acme", "user_id": 81}


def test_native_autocomplete_wire_and_change_create_dry_run(monkeypatch):
    calls = []
    def handler(req):
        calls.append(req)
        assert req.method == "GET"
        assert req.url.path == "/search/autocomplete/agents"
        assert req.url.params["q"] == "AGENT@example.com"
        return httpx.Response(200, json={"results": [NATIVE]})
    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(change_tasks, "get_client", lambda: c)
    monkeypatch.setattr(change_tasks, "runtime_fields", default_task_fields)
    monkeypatch.setattr(change_tasks, "resolve_status", lambda *a: 1)
    try:
        result = CliRunner().invoke(cli.app, ["changes", "tasks-add", "4", "--title", "Acme task",
                                              "--owner", "AGENT@example.com", "--dry-run", "--json"])
        assert result.exit_code == 0, result.output
        assert json.loads(result.stdout)["payload"]["owner_id"] == 81
        assert len(calls) == 1
    finally:
        c.close()


@pytest.mark.parametrize("key", ["id", "email", "details"])
def test_email_shapes_and_case(key):
    row = {key: "agent@example.com", "user_id": "81", "value": "Alex Acme"}
    client = SimpleNamespace(autocomplete=lambda *a: [row])
    assert change_tasks.resolve_owner("AGENT@example.com", client) == 81
    assert query._resolve_agent(client, "AGENT@example.com") == "81"
    assert cli._resolve_agent(client, "AGENT@example.com") == "81"


@pytest.mark.parametrize("rows,count", [
    ([], 0), ([NATIVE, NATIVE], 2),
    ([{**NATIVE, "deleted": True}], 0), ([{**NATIVE, "active": False}], 0),
    ([{**NATIVE, "value": "Alex Acme (deactivated)"}], 0),
    ([{**NATIVE, "user_id": None}], 1), ([{**NATIVE, "user_id": True}], 1),
    ([{**NATIVE, "email": "other@example.com"}], 0),
    ([{**NATIVE, "id": "agent@example.com.invalid"}], 0),
])
def test_fail_closed_and_report_exact_count(rows, count):
    client = SimpleNamespace(autocomplete=lambda *a: rows)
    with pytest.raises(change_tasks.TaskError, match=rf"{count} exact active candidates"):
        change_tasks.resolve_owner("agent@example.com", client)
    with pytest.raises(ValueError, match=rf"{count} exact active candidates"):
        query._resolve_agent(client, "agent@example.com")
    with pytest.raises(typer.Exit):
        cli._resolve_agent(client, "agent@example.com")


def test_name_requires_exact_unique_full_name():
    c = SimpleNamespace(autocomplete=lambda *a: [NATIVE])
    assert change_tasks.resolve_owner("ALEX ACME", c) == 81
    with pytest.raises(change_tasks.TaskError, match="0 exact active candidates"):
        change_tasks.resolve_owner("Alex Globex", c)
    with pytest.raises(ValueError):
        change_tasks.resolve_owner("me", c)
    c.autocomplete = lambda *a: [NATIVE, {**NATIVE, "user_id": 82}]
    with pytest.raises(change_tasks.TaskError, match="2 exact active candidates"):
        change_tasks.resolve_owner("Alex Acme", c)


def test_change_update_owner_dry_run_and_ambiguity(wire, monkeypatch):
    wire.autocomplete = lambda *a: [NATIVE]
    monkeypatch.setattr(cli, "_client", lambda: wire)
    args = ["changes", "tasks-update", "23", "17", "--owner", "agent@example.com", "--dry-run"]
    result = CliRunner().invoke(cli.app, args)
    assert result.exit_code == 0, result.output
    assert json.loads(result.stdout)["payload"]["owner_id"] == 81
    assert not wire.state["writes"]
    wire.autocomplete = lambda *a: [NATIVE, NATIVE]
    result = CliRunner().invoke(cli.app, args)
    assert result.exit_code == 2, result.output
    assert "2 exact active candidates" in result.output
    assert not wire.state["writes"]
