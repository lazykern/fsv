from __future__ import annotations

from pathlib import Path

import pytest

from fsv import completion, create
from fsv.resources import CHANGES


FIELDS = [
    {
        "id": 9000464550,
        "name": "change_reason",
        "label": "Deployment Instructions",
        "field_type": "default_change_reason",
        "helptext": "Deployment Document",
    },
    {
        "id": 9000464553,
        "name": "backout_plan",
        "label": "Deployment Account Request",
        "field_type": "default_backout_plan",
        "helptext": "Request deployment account",
    },
    {
        "id": 9000719794,
        "name": "cfp_risk_notes",
        "label": "Other Document",
        "field_type": "planning_field",
        "export_label": "Other Document",
        "helptext": "Other Document",
    },
    {
        "id": 9000719795,
        "name": "cfp_risk_mitigation",
        "label": "Potential & Mitigation Risk",
        "field_type": "planning_field",
    },
]


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        ("cfp_risk_notes", "cfp_risk_notes"),
        ("Other Document", "cfp_risk_notes"),
        ("Other Documents", "cfp_risk_notes"),
        ("9000719794", "cfp_risk_notes"),
        ("planning-field-cf_cfp_risk_notes", "cfp_risk_notes"),
        ("cf_cfp_risk_notes", "cfp_risk_notes"),
        ("Deployment Instructions", "change_reason"),
        ("Deployment Document", "change_reason"),
        ("Deployment Account Request", "backout_plan"),
        ("Request deployment account", "backout_plan"),
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
        return {"change_planning_field": {"name": "cfp_risk_notes"}}


def test_planning_completion_includes_default_and_custom_fields(monkeypatch):
    monkeypatch.setattr(completion, "_fields_or_fallback", lambda res: FIELDS)

    assert ("Deployment Instructions", "change_reason") in list(completion.complete_planning_field_names(CHANGES)("Deploy"))
    assert ("Other Document", "cfp_risk_notes") in list(completion.complete_planning_field_names(CHANGES)("Other"))


def test_description_only_new_planning_field(monkeypatch):
    client = FakeClient()

    create.update_planning_field(16345, "cfp_risk_notes", description="hello", c=client)

    assert client.posted == (
        "changes/16345/planning-fields?id=cfp_risk_notes",
        {"description": "hello"},
    )


def test_file_only_new_planning_field_uses_filename_description(monkeypatch, tmp_path):
    file_path = tmp_path / "ERP_JOURNAL_STATUS.xlsx"
    file_path.write_text("fake")
    client = FakeClient()
    monkeypatch.setattr(create, "upload_file", lambda path, c: 123)

    create.update_planning_field(16345, "cfp_risk_notes", file_paths=[str(file_path)], c=client)

    assert client.posted == (
        "changes/16345/planning-fields?id=cfp_risk_notes",
        {"description": "ERP_JOURNAL_STATUS.xlsx", "attachments": [123]},
    )
