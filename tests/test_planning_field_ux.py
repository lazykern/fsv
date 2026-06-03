from __future__ import annotations

from pathlib import Path

import pytest

from fsv import completion, create
from fsv.resources import CHANGES


FIELDS = [
    {
        "id": 16000464550,
        "name": "change_reason",
        "label": "FM-Deployment Instruction & Go-No Go",
        "field_type": "default_change_reason",
        "helptext": "Deployment Document",
    },
    {
        "id": 16000464553,
        "name": "backout_plan",
        "label": "FM-EPV user for deploy",
        "field_type": "default_backout_plan",
        "helptext": "Request EPV user",
    },
    {
        "id": 16000719794,
        "name": "cfp_potential_risk",
        "label": "Others Document",
        "field_type": "planning_field",
        "export_label": "Others Document",
        "helptext": "Others Document",
    },
    {
        "id": 16000719795,
        "name": "cfp_mitigation_risk",
        "label": "Potential & Mitigation Risk",
        "field_type": "planning_field",
    },
]


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        ("cfp_potential_risk", "cfp_potential_risk"),
        ("Others Document", "cfp_potential_risk"),
        ("Other Documents", "cfp_potential_risk"),
        ("16000719794", "cfp_potential_risk"),
        ("planning-field-cf_cfp_potential_risk", "cfp_potential_risk"),
        ("cf_cfp_potential_risk", "cfp_potential_risk"),
        ("FM-Deployment Instruction & Go-No Go", "change_reason"),
        ("Deployment Document", "change_reason"),
        ("FM-EPV user for deploy", "backout_plan"),
        ("Request EPV user", "backout_plan"),
    ],
)
def test_resolve_planning_field_aliases(value, expected):
    assert create.resolve_planning_field(value, FIELDS) == expected


class FakeClient:
    def __init__(self):
        self.posted = None

    def int_get(self, path):
        assert path == "changes/16345/planning-fields"
        return {"change_planning_fields": []}

    def int_post(self, path, body):
        self.posted = (path, body)
        return {"change_planning_field": {"name": "cfp_potential_risk"}}


def test_planning_completion_includes_default_and_custom_fields(monkeypatch):
    monkeypatch.setattr(completion, "_fields_or_fallback", lambda res: FIELDS)

    assert ("FM-Deployment Instruction & Go-No Go", "change_reason") in list(completion.complete_planning_field_names(CHANGES)("FM"))
    assert ("Others Document", "cfp_potential_risk") in list(completion.complete_planning_field_names(CHANGES)("Other"))


def test_description_only_new_planning_field(monkeypatch):
    client = FakeClient()

    create.update_planning_field(16345, "cfp_potential_risk", description="hello", c=client)

    assert client.posted == (
        "changes/16345/planning-fields?id=cfp_potential_risk",
        {"description": "hello"},
    )


def test_file_only_new_planning_field_uses_filename_description(monkeypatch, tmp_path):
    file_path = tmp_path / "ERP_JOURNAL_STATUS.xlsx"
    file_path.write_text("fake")
    client = FakeClient()
    monkeypatch.setattr(create, "upload_file", lambda path, c: 123)

    create.update_planning_field(16345, "cfp_potential_risk", file_paths=[str(file_path)], c=client)

    assert client.posted == (
        "changes/16345/planning-fields?id=cfp_potential_risk",
        {"description": "ERP_JOURNAL_STATUS.xlsx", "attachments": [123]},
    )
