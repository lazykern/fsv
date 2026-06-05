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
         "choices": [{"id": 16000100001, "value": "Production"}, {"id": 16000100002, "value": "UAT"}]},
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


def _dry(args):
    result = runner.invoke(cli.app, args)
    assert result.exit_code == 0, result.output
    return json.loads(result.output)


def test_create_flags_resolve_labels():
    body = _dry(["changes", "create", "--subject", "Patch DB",
                 "--status", "Open", "--priority", "High",
                 "--set", "environment=Production", "--dry-run"])["body"]
    assert body == {"subject": "Patch DB", "status": 1, "priority": 3, "environment": "Production"}


def test_create_no_flags_emits_template():
    out = _dry(["changes", "create", "--dry-run"])
    assert "subject" in out and "dry_run" not in out


def test_create_no_input_without_flags_errors():
    result = runner.invoke(cli.app, ["changes", "create", "--no-input"])
    assert result.exit_code != 0
    assert "no-input" in result.output


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
    # --no-input + flags, NOT --dry-run → reaches preflight, must error before POST
    monkeypatch.setattr(schema_mod, "load", lambda res, c=None: {"fields": [
        {"name": "requester", "label": "Requester", "field_type": "default_requester", "required": True},
        {"name": "subject", "label": "Subject", "field_type": "default_subject", "required": True},
    ]})
    result = runner.invoke(cli.app, ["changes", "create", "--subject", "x", "--no-input"])
    assert result.exit_code != 0
    assert "Requester" in result.output and "required" in result.output


def test_update_resolves_labels():
    body = _dry(["changes", "update", "CHN-1", "--status", "Open",
                 "--priority", "High", "--dry-run"])["body"]
    assert body == {"status": 1, "priority": 3}
