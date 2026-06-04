from __future__ import annotations

from rich.text import Text

from fsv.resources import CHANGES
from fsv.tui.app import FsvApp


class StubStatic:
    def __init__(self):
        self.content = None
        self.display = True

    def update(self, content, layout=True):
        self.content = content


class StubScroll:
    def __init__(self):
        self.calls = 0
        self.kwargs = None

    def scroll_home(self, **kwargs):
        self.calls += 1
        self.kwargs = kwargs


def _setup_app(monkeypatch):
    app = FsvApp()
    widgets = {
        "#detail-bar": StubStatic(),
        "#state-flow-bar": StubStatic(),
        "#detail-tabs": StubStatic(),
        "#detail-content": StubStatic(),
        "#detail-scroll": StubScroll(),
    }

    monkeypatch.setattr(app, "query_one", lambda selector, *args, **kwargs: widgets[selector])
    monkeypatch.setattr(app, "_render_filter_bar", lambda: None)
    monkeypatch.setattr(app, "_render_status", lambda: None)

    scheduled = []

    def fake_call_after_refresh(callback, *args, **kwargs):
        scheduled.append(callback)
        callback()

    monkeypatch.setattr(app, "call_after_refresh", fake_call_after_refresh)
    return app, widgets, scheduled


def test_clear_detail_resets_scroll(monkeypatch):
    app, widgets, scheduled = _setup_app(monkeypatch)

    app._clear_detail("[dim]loading[/]")

    assert len(scheduled) == 1
    assert widgets["#detail-scroll"].calls == 1
    assert widgets["#detail-scroll"].kwargs == {"animate": False, "immediate": True}
    assert isinstance(widgets["#detail-content"].content, Text)
    assert widgets["#detail-content"].content.plain == "loading"


def test_render_detail_content_resets_scroll_for_details_tab(monkeypatch):
    app, widgets, scheduled = _setup_app(monkeypatch)
    app._schemas = {"changes": {"fields": []}}
    monkeypatch.setattr(app, "_format_details", lambda *args, **kwargs: "detail body")

    item = {"id": 16375, "_resource": CHANGES}
    app._render_detail_content(item, CHANGES)

    assert len(scheduled) == 1
    assert widgets["#detail-scroll"].calls == 1
    assert widgets["#detail-content"].content.plain == "detail body"


def test_render_sub_content_resets_scroll(monkeypatch):
    app, widgets, scheduled = _setup_app(monkeypatch)
    app._selected = {"id": 16375, "_resource": CHANGES}
    app.detail_tab_idx = 1
    app._pending_sub_key = "changes:16375:notes"

    app._render_sub_content("notes", [], CHANGES, "changes:16375:notes")

    assert len(scheduled) == 1
    assert widgets["#detail-scroll"].calls == 1
    assert widgets["#detail-content"].content.plain == "\nno notes\n"
