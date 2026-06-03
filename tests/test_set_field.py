"""Tests for --set FIELD=VALUE resolution on changes update."""
from __future__ import annotations

import pytest

from fsv.cli import _resolve_set_field

# Minimal schema mimicking KKP change fields
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
                {"id": 16000100001, "value": "Production"},
                {"id": 16000100002, "value": "UAT"},
            ],
        },
        {
            "name": "msf_application_code",
            "label": "Application Code",
            "field_type": "custom_multi_select_dropdown",
            "default_field": False,
            "choices": [
                {"id": 16000200001, "value": "EDP"},
                {"id": 16000200002, "value": "ETL"},
                {"id": 16000200003, "value": "IDM"},
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
                {"id": 16000182336, "value": "Information Technology"},
                {"id": 16000182337, "value": "Finance"},
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
        if kind == "agents" and "pong" in query.casefold():
            return [{"value": "Pong Potcharapol", "user_id": 16003029099, "email": "pong@example.com"}]
        return []

    def int_get(self, path):
        if "bootstrap/agents_groups" in path:
            return {"groups": [{"id": 16000309867, "name": "CD Data Platform"}]}
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
    key, val = _resolve_set_field(C, SCHEMA, "Application Code", "EDP")
    assert key == "msf_application_code" and val == ["EDP"]


def test_multi_select_multiple():
    key, val = _resolve_set_field(C, SCHEMA, "Application Code", "EDP,ETL")
    assert key == "msf_application_code" and val == ["EDP", "ETL"]


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
    key, val = _resolve_set_field(C, SCHEMA, "agent", "16003029057")
    assert key == "agent_id" and val == 16003029057


def test_agent_name_resolved():
    key, val = _resolve_set_field(C, SCHEMA, "agent", "Pong")
    assert key == "agent_id" and val == 16003029099


# ── FK remap: group → group_id ────────────────────────────────────────────────
def test_group_name_resolved():
    key, val = _resolve_set_field(C, SCHEMA, "group", "CD Data Platform")
    assert key == "group_id" and val == 16000309867


def test_group_id_passthrough():
    key, val = _resolve_set_field(C, SCHEMA, "group", "16000309867")
    assert key == "group_id" and val == 16000309867


# ── FK remap: department → department_id ─────────────────────────────────────
def test_department_label_to_id():
    key, val = _resolve_set_field(C, SCHEMA, "department", "Information Technology")
    assert key == "department_id" and val == 16000182336


# ── subject (text core field) ─────────────────────────────────────────────────
def test_subject_passthrough():
    key, val = _resolve_set_field(C, SCHEMA, "subject", "New deployment")
    assert key == "subject" and val == "New deployment"


# ── unknown field → raises SystemExit (via _err) ─────────────────────────────
def test_unknown_field_raises():
    import click
    with pytest.raises((SystemExit, click.exceptions.Exit)):
        _resolve_set_field(C, SCHEMA, "nonexistent_xyz", "value")
