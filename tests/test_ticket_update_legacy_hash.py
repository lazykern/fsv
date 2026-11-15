"""Legacy (non --reviewed) ticket update: the dry run shows a review hash bound
to the tenant, the exact PUT body and the ticket snapshot; --expect-plan
refuses a changed body or record and sends nothing."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli

FIELDS = {"fields": [
    {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": True},
    {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": True,
     "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}]},
]}


@pytest.fixture
def state(monkeypatch):
    s = {"row": {"id": 11, "human_display_id": "SR-11", "subject": "Acme laptop", "priority": 1,
                 "updated_at": "2030-01-01T00:00:00Z"}, "puts": [], "gets": 0, "get_fails": False}

    class Client:
        def int_get(self, path, params=None):
            assert path == "tickets/11"
            s["gets"] += 1
            if s["get_fails"]:
                raise RuntimeError("synthetic read failure")
            return {"ticket": dict(s["row"])}

        def int_put(self, path, body):
            s["puts"].append((path, body))
            return {"ticket": {**s["row"], **body}}

    monkeypatch.setattr(cli, "_client", lambda: Client())
    monkeypatch.setattr("fsv.schema.load", lambda *a: FIELDS)
    return s


ARGS = ["tickets", "update", "SR-11", "--priority", "Medium"]


def _review(args=ARGS):
    r = CliRunner().invoke(cli.app, args + ["--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    return json.loads(r.output)["plan_hash"]


def test_dry_run_shows_hash_and_matching_send_goes_once(state):
    h = _review()
    assert len(h) == 64 and not state["puts"]
    r = CliRunner().invoke(cli.app, ARGS + ["--yes", "--expect-plan", h, "--json"])
    assert r.exit_code == 0, r.output
    assert state["puts"] == [("tickets/11", {"priority": 2})]


def test_hash_is_stable_and_bound_to_body(state):
    assert _review() == _review()
    assert _review(["tickets", "update", "SR-11", "--priority", "Low"]) != _review()
    assert _review(ARGS + ["--set", "subject=Acme desk"]) != _review()


@pytest.mark.parametrize("change", ["record", "body", "prefix"])
def test_stale_or_partial_hash_refuses_and_sends_nothing(state, change):
    h = _review()
    args = list(ARGS)
    if change == "record":
        state["row"]["updated_at"] = "2030-01-02T00:00:00Z"
        state["row"]["subject"] = "Acme laptop (edited)"
    elif change == "body":
        args = ["tickets", "update", "SR-11", "--priority", "Low"]
    else:
        h = h[:16]
    r = CliRunner().invoke(cli.app, args + ["--yes", "--expect-plan", h])
    assert r.exit_code == 2, r.output
    assert "changed since review" in " ".join(r.output.split()) and not state["puts"]


def test_unreadable_record_has_no_hash_and_expect_plan_refuses(state):
    h = _review()
    state["get_fails"] = True
    r = CliRunner().invoke(cli.app, ARGS + ["--dry-run", "--json"])
    assert r.exit_code == 0 and "plan_hash" not in json.loads(r.output)
    r = CliRunner().invoke(cli.app, ARGS + ["--yes", "--expect-plan", h])
    assert r.exit_code == 2 and not state["puts"]


def test_plain_send_without_hash_keeps_the_legacy_path_and_reads_nothing(state):
    r = CliRunner().invoke(cli.app, ARGS + ["--yes"])
    assert r.exit_code == 0, r.output
    assert state["gets"] == 0 and state["puts"] == [("tickets/11", {"priority": 2})]


def test_hash_is_tenant_bound(state, monkeypatch):
    from fsv import config
    h = _review()
    monkeypatch.setattr(config, "DOMAIN", "globex.freshservice.com")
    assert _review() != h
