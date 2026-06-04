from __future__ import annotations

from fsv.resources import CHANGES


def test_get_change_evidence_fetches_planning_and_attachment_sources(monkeypatch):
    from fsv import config, service

    monkeypatch.setattr(config, "DOMAIN", "fresh.example")

    class FakeClient:
        def int_get(self, path, params=None):
            if path == "changes/1":
                return {
                    "change": {
                        "id": 1,
                        "attachments": [{"id": 7, "name": "main.txt"}],
                        "description": '<a href="/helpdesk/attachments/9">evidence</a>',
                    }
                }
            if path == "changes/1/planning-fields":
                return {
                    "change_planning_fields": [
                        {"name": "change_plan", "attachments": [{"id": 8, "name": "plan.docx"}]}
                    ]
                }
            raise AssertionError((path, params))

    evidence = service.get_change_evidence(1, client=FakeClient())

    assert evidence["change"]["_resource"] == CHANGES
    assert evidence["planning_fields_by_name"]["change_plan"]["attachments"][0]["name"] == "plan.docx"
    assert evidence["main_attachments"][0]["name"] == "main.txt"
    assert evidence["description_attachment_urls"] == ["https://fresh.example/helpdesk/attachments/9"]


def test_get_resource_change_stats_json_includes_planning(monkeypatch):
    import fsv.cli as cli

    emitted = []

    class FakeClient:
        def int_get(self, path, params=None):
            if path == "changes/1":
                return {"change": {"id": 1, "human_display_id": "CHN-1", "attachments": []}}
            if path == "changes/1/planning-fields":
                return {"change_planning_fields": [{"name": "change_reason", "attachments": []}]}
            raise AssertionError((path, params))

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli.schema_mod, "load", lambda res, c: {"fields": [{"name": "change_reason", "label": "Reason", "field_type": "default_change_reason"}]})
    monkeypatch.setattr(cli, "emit_json", lambda data: emitted.append(data))

    cli.get_resource(CHANGES, "CHN-1", True, True)

    assert emitted[0]["planning_fields"] == {"change_reason": {"name": "change_reason", "attachments": []}}
    assert emitted[0]["change_planning_fields"] == [{"name": "change_reason", "attachments": []}]
    assert emitted[0]["planning_field_definitions"] == [{"name": "change_reason", "label": "Reason", "field_type": "default_change_reason"}]
    assert emitted[0]["main_attachments"] == []
    assert emitted[0]["description_attachment_urls"] == []
