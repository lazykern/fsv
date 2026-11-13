"""`ls --no-defaults` with no filter must not silently keep the server's
default view: the list endpoint applies that view when no filter is sent, so
fsv asks for the module's system 'All' view, or names the view that still
filters."""
import json

import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import views as V

VIEWS = {
    "tickets": [V.SavedView("new_and_my_open", "New & My Open Tickets", True, True),
                V.SavedView("9001", "All Acme Tickets", False),
                V.SavedView("all_tickets", "All Tickets", True)],
    "changes": [V.SavedView("my_open", "My Open Changes", True, True)],
    "problems": [V.SavedView("my_open", "My Open Problems", True, True),
                 V.SavedView("all", "All Problems", True)],
}


@pytest.fixture
def sent(monkeypatch):
    calls = []

    class FakeClient:
        def int_get(self, path, params=None):
            calls.append((path, dict(params or {})))
            return {path: [{"id": 1, "status": 2}]}

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "_saved_views", lambda c, res: VIEWS[res.name])
    monkeypatch.setattr(V, "ticket_headers", lambda c: [])
    monkeypatch.setattr("fsv.schema.load", lambda res, c: {"fields": []})
    monkeypatch.setattr(cli.config, "get_default_where", lambda name: [])
    return calls


def _list_filter(calls, module):
    return [p.get("filter") for path, p in calls if path == module]


@pytest.mark.parametrize("module,view_id", [("tickets", "all_tickets"), ("problems", "all")])
def test_no_defaults_lists_the_system_all_view(sent, module, view_id):
    r = CliRunner().invoke(cli.app, [module, "ls", "--no-defaults", "--json"])
    assert r.exit_code == 0, r.output
    assert _list_filter(sent, module) == [view_id]
    assert f"({view_id})" in r.output and "--no-defaults" in r.output


def test_without_all_view_the_default_view_is_named(sent):
    r = CliRunner().invoke(cli.app, ["changes", "ls", "--no-defaults", "--json"])
    assert r.exit_code == 0, r.output
    assert _list_filter(sent, "changes") == ["my_open"]
    assert "My Open Changes (my_open) still filters this list" in r.output


def test_custom_all_named_view_is_not_taken_for_all():
    assert V.all_records_view([V.SavedView("9001", "All Acme Tickets", False)], "tickets") is None
    assert V.all_records_view([V.SavedView("all", "All", False)], "changes") is None


def test_default_list_still_uses_the_server_default_view(sent):
    r = CliRunner().invoke(cli.app, ["tickets", "ls", "--json"])
    assert r.exit_code == 0, r.output
    assert _list_filter(sent, "tickets") == ["new_and_my_open"]
    assert "--no-defaults" not in r.output


def test_explicit_view_wins_over_no_defaults(sent):
    r = CliRunner().invoke(cli.app, ["tickets", "ls", "--no-defaults", "--view", "New & My Open Tickets", "--json"])
    assert r.exit_code == 0, r.output
    assert _list_filter(sent, "tickets") == ["new_and_my_open"]
