from __future__ import annotations

from io import StringIO

from rich.console import Console

from fsv.resources import TICKETS


def test_get_resource_ticket_json_includes_requested_items(monkeypatch):
    import fsv.cli as cli

    emitted = []

    class FakeClient:
        def int_get(self, path, params=None):
            if path == "tickets/1":
                return {
                    "ticket": {
                        "id": 1,
                        "display_id": 1,
                        "human_display_id": "SR-1",
                        "subject": "demo",
                        "status": 2,
                        "priority": 2,
                        "type": "Service Request",
                    }
                }
            if path == "tickets/1/requested_items":
                return {
                    "requested_items": [
                        {
                            "id": 10,
                            "stage": {"name": "Requested"},
                            "item": {"name": "Access Request"},
                        }
                    ]
                }
            if path == "tickets/1/requested_items/10":
                return {
                    "requested_item": {
                        "custom_fields": {"action": "Create group"},
                        "item": {
                            "description": "Need access",
                            "custom_fields": [],
                        },
                    }
                }
            raise AssertionError((path, params))

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli.schema_mod, "load", lambda res, c: {"fields": []})
    monkeypatch.setattr(cli, "emit_json", lambda data: emitted.append(data))

    cli.get_resource(TICKETS, "SR-1", False, True)

    assert emitted[0]["requested_items"][0]["stage"]["name"] == "Requested"
    assert emitted[0]["requested_items"][0]["item"]["name"] == "Access Request"
    assert emitted[0]["requested_items"][0]["item"]["description"] == "Need access"


def test_requested_item_rows_use_active_section_and_nested_fields():
    import fsv.render as render

    item = {
        "custom_fields": {
            "action": "Create group",
            "category": "Banking",
            "system": "AAD (Azure AD)",
            "group": "rg-demo",
            "details": "because",
        },
        "item": {
            "custom_fields": [
                {
                    "name": "action",
                    "label": "Action",
                    "field_type": "custom_dropdown",
                    "sections": [
                        {
                            "name": "Create group",
                            "fields": [
                                {
                                    "name": "category",
                                    "label": "Category",
                                    "field_type": "nested_field",
                                    "nested_fields": [
                                        {"name": "system", "label": "System", "nested_fields": []}
                                    ],
                                },
                                {"name": "group", "label": "Group", "nested_fields": []},
                                {"name": "details", "label": "Details", "nested_fields": []},
                            ],
                        }
                    ],
                }
            ]
        },
    }

    assert render._requested_item_rows(item) == [
        ("Action", "Create group"),
        ("Category", "Banking"),
        ("System", "AAD (Azure AD)"),
        ("Group", "rg-demo"),
        ("Details", "because"),
    ]


def test_requested_items_panel_renders_stage_description_and_fields(monkeypatch):
    import fsv.render as render

    buf = StringIO()
    test_console = Console(file=buf, force_terminal=False, width=120)
    monkeypatch.setattr(render, "console", test_console)

    render.requested_items_panel([
        {
            "stage": {"name": "Requested"},
            "custom_fields": {"action": "Create group", "group": "rg-demo"},
            "item": {
                "name": "Access Request",
                "description": "Need access",
                "custom_fields": [
                    {
                        "name": "action",
                        "label": "Action",
                        "sections": [
                            {
                                "name": "Create group",
                                "fields": [
                                    {"name": "group", "label": "Group", "nested_fields": []}
                                ],
                            }
                        ],
                    }
                ],
            },
        }
    ])

    out = buf.getvalue()
    assert "Requested" in out
    assert "Need access" in out
    assert "Action" in out
    assert "Group" in out
    assert "rg-demo" in out


def test_conversations_resource_uses_ticket_conversations_endpoint(monkeypatch):
    import fsv.cli as cli

    emitted = []

    class FakeClient:
        def int_get(self, path, params=None):
            assert path == "tickets/1/conversations"
            assert params == {"include": "user,phone,feedback", "page": 2, "per_page": 5}
            return {
                "conversations": [
                    {
                        "id": 9,
                        "created_at": "2026-05-29T12:34:56+07:00",
                        "private": True,
                        "incoming": False,
                        "body_text": "hello",
                        "user": {"name": "Agent"},
                    }
                ]
            }

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "emit_json", lambda data: emitted.append(data))

    cli.conversations_resource("SR-1", 2, 5, True)

    assert emitted == [[{
        "id": 9,
        "created_at": "2026-05-29T12:34:56+07:00",
        "private": True,
        "incoming": False,
        "body_text": "hello",
        "user": {"name": "Agent"},
    }]]


def test_ticket_approvals_resource_uses_internal_endpoint(monkeypatch):
    import fsv.cli as cli

    emitted = []

    class FakeClient:
        def int_get(self, path, params=None):
            assert path == "tickets/1/approvals"
            assert params is None
            return {
                "approvals": [
                    {
                        "level_id": 1,
                        "status": {"name": "approved"},
                        "type": {"name": "everyone"},
                        "member": {"name": "Approver"},
                        "remark": [{"updated_at": "2026-05-22T09:17:34Z", "data": "ok"}],
                    }
                ]
            }

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "emit_json", lambda data: emitted.append(data))

    cli.ticket_approvals_resource("SR-1", json_out=True)

    assert emitted[0][0]["member"]["name"] == "Approver"
    assert emitted[0][0]["status"]["name"] == "approved"


def test_ticket_associations_resource_uses_ticket_endpoints(monkeypatch):
    import fsv.cli as cli

    emitted = []

    class FakeClient:
        def int_get(self, path, params=None):
            if path == "tickets/1/tabs":
                return [{"name": "associations", "associated_modules": ["change", "change_cause", "problem"]}]
            if path == "tickets/1/changes" and params == {"change_type": "change"}:
                return {"changes": [{"human_display_id": "CHN-1", "subject": "demo", "status_name": "Open", "priority_name": "Medium"}]}
            if path == "tickets/1/changes" and params == {"change_type": "change_cause"}:
                return {"changes": []}
            if path == "tickets/1/problems":
                return {"problems": [{"human_display_id": "PRB-1", "subject": "problem", "status_name": "Known Error"}]}
            raise AssertionError((path, params))

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "emit_json", lambda data: emitted.append(data))

    cli.ticket_associations_resource("SR-1", json_out=True)

    assert emitted == [{
        "changes": [{"human_display_id": "CHN-1", "subject": "demo", "status_name": "Open", "priority_name": "Medium"}],
        "change_causes": [],
        "problems": [{"human_display_id": "PRB-1", "subject": "problem", "status_name": "Known Error"}],
    }]
