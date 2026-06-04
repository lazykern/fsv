from __future__ import annotations

from rich.text import Text

from fsv.resources import CHANGES
from fsv.tui import app as tui_app
from fsv.tui.app import FsvApp


class StubStatic:
    def __init__(self):
        self.content = None

    def update(self, content, layout=True):
        self.content = content


def test_render_detail_bar_treats_subject_as_plain_text(monkeypatch):
    app = FsvApp()
    app._schemas = {"changes": {"fields": []}}
    target = StubStatic()

    monkeypatch.setattr(app, "query_one", lambda *args, **kwargs: target)
    monkeypatch.setattr(tui_app.service, "resolve_status", lambda *args, **kwargs: "Pending Review")
    monkeypatch.setattr(tui_app.service, "resolve_priority", lambda *args, **kwargs: "Low")

    item = {
        "id": 123,
        "subject": "demo change [#REF-1] update [bold]param[/bold]",
    }

    app._render_detail_bar(item, CHANGES)

    assert isinstance(target.content, Text)
    assert target.content.plain == "CHN-123 demo change [#REF-1] update [bold]param[/bold]  Low"


def test_format_details_escapes_markup_like_values(monkeypatch):
    app = FsvApp()
    app._schemas = {"changes": {"fields": []}}

    monkeypatch.setattr(tui_app.service, "resolve_status", lambda *args, **kwargs: "Review [Pending]")
    monkeypatch.setattr(tui_app.service, "resolve_priority", lambda *args, **kwargs: "Low [P4]")
    monkeypatch.setattr(tui_app.schema_mod, "choice_label", lambda *args, **kwargs: "Normal [#1]")
    monkeypatch.setattr(tui_app.schema_mod, "field", lambda *args, **kwargs: {"label": "Guide [v2]"})

    item = {
        "id": 1,
        "subject": "Need [bold]fix[/bold] #tag",
        "description": "desc [#REF-1] [green]x[/]",
        "change_type": 1,
        "risk": 1,
        "requester": {"name": "User [name]"},
        "agent": {"name": "Agent [ops]"},
        "group": {"name": "Group [core]"},
        "custom_fields": {"guide": "value [danger]"},
        "planning_fields": {"step": {"description_text": "step [1]"}},
    }

    text = Text.from_markup(app._format_details(item, CHANGES))
    plain = text.plain

    assert "Need [bold]fix[/bold] #tag" in plain
    assert "desc [#REF-1] [green]x[/]" in plain
    assert "Review [Pending]" in plain
    assert "Low [P4]" in plain
    assert "Normal [#1]" in plain
    assert "User [name]" in plain
    assert "Agent [ops]" in plain
    assert "Group [core]" in plain
    assert "Guide [v2]" in plain
    assert "value [danger]" in plain
    assert "step [1]" in plain


def test_free_text_formatters_escape_user_content():
    app = FsvApp()

    notes = Text.from_markup(
        app._format_notes(
            [
                {
                    "user": {"name": "User [note]"},
                    "created_at": "2026-06-03T12:34:56+07:00",
                    "body_text": "line [one]\nline [#two]",
                }
            ]
        )
    ).plain
    tasks = Text.from_markup(
        app._format_tasks(
            [
                {
                    "id": 7,
                    "title": "Task [urgent]",
                    "status": 2,
                    "group": {"name": "Ops [G]"},
                    "agent": {"name": "Agent [A]"},
                    "due_date": "2026-06-10T00:00:00Z",
                }
            ]
        )
    ).plain
    activity = Text.from_markup(
        app._format_activity(
            [
                {
                    "created_at": "2026-06-03T12:34:56+07:00",
                    "actor": {"name": "Actor [sys]"},
                    "content": "did [thing] [#3]",
                }
            ]
        )
    ).plain

    assert "User [note]" in notes
    assert "line [one]" in notes
    assert "line [#two]" in notes
    assert "Task [urgent]" in tasks
    assert "Ops [G] / Agent [A] / Due: 2026-06-10" in tasks
    assert "Actor [sys]" in activity
    assert "did [thing] [#3]" in activity
