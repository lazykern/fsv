"""Tests for flag-based `fsv changes create` and closure-scope regression.

The change commands (create/clone/update/download) close over `fsv.create`
imports bound in `_make_subapp`. A prior refactor nested those imports inside
`url()`, leaving the siblings with unbound names — every invocation raised
NameError at runtime. These tests drive the commands through CliRunner so that
class of regression fails loudly.
"""
from __future__ import annotations

import json

import pytest
from typer.testing import CliRunner

import fsv.cli as cli
import fsv.schema as schema_mod

runner = CliRunner()

SCHEMA = {
    "fields": [
        {"name": "status", "label": "Status", "field_type": "default_status", "default_field": True,
         "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Pending"}, {"id": 3, "value": "Resolved"}]},
        {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": True,
         "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"}]},
        {"name": "environment", "label": "Environment", "field_type": "custom_dropdown", "default_field": False,
         "choices": [{"id": 9000000101, "value": "Production"}, {"id": 9000000102, "value": "UAT"}]},
        {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": True, "choices": []},
    ]
}


class _FakeClient:
    def autocomplete(self, *a, **k):
        return []

    def int_get(self, path):
        return {}


@pytest.fixture(autouse=True)
def _patch(monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: _FakeClient())
    monkeypatch.setattr(schema_mod, "load", lambda res, c=None: SCHEMA)
    # change_template() builds from the schema; keep it offline/hermetic
    import fsv.create as create_mod
    monkeypatch.setattr(create_mod, "_get_change_schema_fields", lambda: [
        {**f, "required": True} if f["name"] == "subject" else f for f in SCHEMA["fields"]])


def _dry(args):
    result = runner.invoke(cli.app, args)
    assert result.exit_code == 0, result.output
    return json.loads(result.output)


def _acme(monkeypatch):
    import acme_change_form as A
    import fsv.change_flow as flow
    c = A.FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.delenv("FSV_TZ", raising=False)
    return c


def test_create_flags_resolve_labels(monkeypatch):
    # legacy flags still work; they are answers replayed through the form engine
    _acme(monkeypatch)
    out = _dry(["changes", "create", "--subject", "Patch DB", "--priority", "High",
                "--set", "Cloud Only=Yes", "--set", "Stage=Live", "--description", "x",
                "--set", "Planned Start Date=2030-01-01 10:00", "--set", "Planned End Date=2030-01-01 11:00",
                "--requester", "jane.doe@example.com", "--dry-run", "-o", "json"])
    body = out["payload"]
    assert body["subject"] == "Patch DB" and body["priority"] == 3 and body["status"] == 1
    assert body["custom_fields"] == {"cloud_only": "Yes", "stage": "Live"}


def test_create_no_flags_emits_template(monkeypatch):
    _acme(monkeypatch)
    result = runner.invoke(cli.app, ["changes", "create", "--dry-run"])
    assert result.exit_code == 0, result.output
    assert "Subject:" in result.output and "# REQUIRED" in result.output


def test_create_no_input_without_flags_errors(monkeypatch):
    _acme(monkeypatch)
    result = runner.invoke(cli.app, ["changes", "create", "--no-input"])
    assert result.exit_code == 2
    assert "--from" in result.output


def test_missing_required_lists_fk_and_custom():
    sch = {"fields": [
        {"name": "requester", "label": "Requester", "field_type": "default_requester", "required": True},
        {"name": "agent", "label": "Agent", "field_type": "default_agent", "required": True},
        {"name": "subject", "label": "Subject", "field_type": "default_subject", "required": True},
        {"name": "status", "label": "Status", "field_type": "default_status", "required": True},
        {"name": "environment", "label": "Environment", "field_type": "custom_dropdown", "required": True},
        {"name": "remark", "label": "Remark", "field_type": "custom_text", "required": False},
    ]}
    # body supplies agent (as agent_id) + subject; status is excluded from the check
    missing = cli._missing_required_change_fields(sch, {"agent_id": 5, "subject": "x"})
    assert missing == ["Requester", "Environment"]
    # nothing missing when all required present (status ignored, optional ignored)
    full = {"requester_id": 1, "agent_id": 5, "subject": "x", "environment": "Production"}
    assert cli._missing_required_change_fields(sch, full) == []


def test_create_preflight_blocks_real_submit(monkeypatch):
    # not a dry run, but incomplete: refused before any POST
    c = _acme(monkeypatch)
    result = runner.invoke(cli.app, ["changes", "create", "--subject", "x", "--requester",
                                     "jane.doe@example.com", "--no-input", "--yes"])
    assert result.exit_code == 2
    assert "required" in result.output and not c.writes()


def test_update_resolves_labels():
    body = _dry(["changes", "update", "CHN-1", "--status", "Open",
                 "--priority", "High", "--dry-run"])["body"]
    assert body == {"status": 1, "priority": 3}
