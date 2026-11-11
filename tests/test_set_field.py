"""Tests for --set FIELD=VALUE resolution on changes update."""
from __future__ import annotations

import pytest

from fsv.cli import _resolve_set_field

# Minimal schema of change fields
SCHEMA = {
    "fields": [
        {
            "name": "status",
            "label": "Status",
            "field_type": "default_status",
            "default_field": True,
            "choices": [
                {"id": 1, "value": "Open"},
                {"id": 2, "value": "Pending"},
                {"id": 3, "value": "Resolved"},
            ],
        },
        {
            "name": "priority",
            "label": "Priority",
            "field_type": "default_priority",
            "default_field": True,
            "choices": [
                {"id": 1, "value": "Low"},
                {"id": 2, "value": "Medium"},
                {"id": 3, "value": "High"},
                {"id": 4, "value": "Urgent"},
            ],
        },
        {
            "name": "environment",
            "label": "Environment",
            "field_type": "custom_dropdown",
            "default_field": False,
            "choices": [
                {"id": 9000000101, "value": "Production"},
                {"id": 9000000102, "value": "UAT"},
            ],
        },
        {
            "name": "app_code",
            "label": "Application Code",
            "field_type": "custom_multi_select_dropdown",
            "default_field": False,
            "choices": [
                {"id": 9000000105, "value": "APP1"},
                {"id": 9000000106, "value": "APP2"},
                {"id": 9000000107, "value": "APP3"},
            ],
        },
        {
            "name": "require_security_review",
            "label": "Require Security Review",
            "field_type": "custom_checkbox",
            "default_field": False,
            "choices": [],
        },
        {
            "name": "remark",
            "label": "Remark",
            "field_type": "custom_text",
            "default_field": False,
            "choices": [],
        },
        {
            "name": "planned_start_date",
            "label": "Planned Start Date",
            "field_type": "default_planned_start_date",
            "default_field": True,
            "choices": [],
        },
        {
            "name": "agent",
            "label": "Agent",
            "field_type": "default_agent",
            "default_field": True,
            "choices": [],
        },
        {
            "name": "group",
            "label": "Group",
            "field_type": "default_group",
            "default_field": True,
            "choices": [],
        },
        {
            "name": "department",
            "label": "Department",
            "field_type": "default_department",
            "default_field": True,
            "choices": [
                {"id": 9000000103, "value": "Information Technology"},
                {"id": 9000000104, "value": "Finance"},
            ],
        },
        {
            "name": "subject",
            "label": "Subject",
            "field_type": "default_subject",
            "default_field": True,
            "choices": [],
        },
    ]
}


class _FakeClient:
    def autocomplete(self, kind, query, params=None):
        if kind == "agents" and "pat" in query.casefold():
            return [{"value": "Pat Example", "user_id": 9000000118, "email": "pat@example.com"}]
        return []

    def int_get(self, path):
        if "bootstrap/agents_groups" in path:
            return {"groups": [{"id": 9000000108, "name": "Acme Platform Team"}]}
        return {}


C = _FakeClient()


# ── default dropdown (status) → returns ID int ─────────────────────────────
def test_status_label_to_id():
    key, val = _resolve_set_field(C, SCHEMA, "status", "Open")
    assert key == "status" and val == 1


def test_status_id_passthrough():
    key, val = _resolve_set_field(C, SCHEMA, "status", "2")
    assert key == "status" and val == 2


# ── default dropdown by label (case-insensitive) ────────────────────────────
def test_priority_case_insensitive():
    key, val = _resolve_set_field(C, SCHEMA, "Priority", "urgent")
    assert key == "priority" and val == 4


# ── custom dropdown → returns value string ──────────────────────────────────
def test_custom_dropdown_returns_string():
    key, val = _resolve_set_field(C, SCHEMA, "environment", "Production")
    assert key == "environment" and val == "Production"


def test_custom_dropdown_by_label():
    key, val = _resolve_set_field(C, SCHEMA, "Environment", "UAT")
    assert key == "environment" and val == "UAT"


# ── multi-select: comma-split → list of strings ─────────────────────────────
def test_multi_select_single():
    key, val = _resolve_set_field(C, SCHEMA, "Application Code", "APP1")
    assert key == "app_code" and val == ["APP1"]


def test_multi_select_multiple():
    key, val = _resolve_set_field(C, SCHEMA, "Application Code", "APP1,APP2")
    assert key == "app_code" and val == ["APP1", "APP2"]


# ── checkbox ─────────────────────────────────────────────────────────────────
@pytest.mark.parametrize("raw", ["true", "True", "1", "yes", "on"])
def test_checkbox_truthy(raw):
    key, val = _resolve_set_field(C, SCHEMA, "require_security_review", raw)
    assert key == "require_security_review" and val is True


@pytest.mark.parametrize("raw", ["false", "False", "0", "no", "off"])
def test_checkbox_falsy(raw):
    key, val = _resolve_set_field(C, SCHEMA, "require_security_review", raw)
    assert key == "require_security_review" and val is False


# ── text field passthrough ────────────────────────────────────────────────────
def test_text_passthrough():
    key, val = _resolve_set_field(C, SCHEMA, "remark", "network capture test")
    assert key == "remark" and val == "network capture test"


# ── date passthrough ──────────────────────────────────────────────────────────
def test_date_passthrough():
    key, val = _resolve_set_field(C, SCHEMA, "planned_start_date", "2026-05-27T12:00:00+07:00")
    assert key == "planned_start_date" and val == "2026-05-27T12:00:00+07:00"


# ── FK remap: agent → agent_id ────────────────────────────────────────────────
def test_agent_id_passthrough():
    key, val = _resolve_set_field(C, SCHEMA, "agent", "9000000117")
    assert key == "agent_id" and val == 9000000117


def test_agent_name_resolved():
    key, val = _resolve_set_field(C, SCHEMA, "agent", "Pat Example")
    assert key == "agent_id" and val == 9000000118


# ── FK remap: group → group_id ────────────────────────────────────────────────
def test_group_name_resolved():
    key, val = _resolve_set_field(C, SCHEMA, "group", "Acme Platform Team")
    assert key == "group_id" and val == 9000000108


def test_group_id_passthrough():
    key, val = _resolve_set_field(C, SCHEMA, "group", "9000000108")
    assert key == "group_id" and val == 9000000108


# ── FK remap: department → department_id ─────────────────────────────────────
def test_department_label_to_id():
    key, val = _resolve_set_field(C, SCHEMA, "department", "Information Technology")
    assert key == "department_id" and val == 9000000103


# ── subject (text core field) ─────────────────────────────────────────────────
def test_subject_passthrough():
    key, val = _resolve_set_field(C, SCHEMA, "subject", "New deployment")
    assert key == "subject" and val == "New deployment"


# ── unknown field → raises SystemExit (via _err) ─────────────────────────────
def test_unknown_field_raises():
    import fsv.cli as cli
    with pytest.raises((SystemExit, cli.typer.Exit)):
        _resolve_set_field(C, SCHEMA, "nonexistent_xyz", "value")
