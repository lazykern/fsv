"""In-terminal interactive forms (Textual), laid out like the Freshservice
web create/edit forms: fields in web order, type-to-filter dropdowns,
people/group/agent search as you type, date pickers, a Markdown
description, file pickers, live business rules, and the review screen.

See ``docs/formui.md``. Public contract::

    from fsv.formui import FormScreen, FormSession, FormResult, run_form
    session = change_session(client)                # or sr_session(client, 77)
    app.push_screen(FormScreen(session), on_done)   # inside any Textual app
    result = run_form(session)                       # standalone (CLI)
    if result.action == "submit":
        session.send(result.plan)                    # journal, freshness, writes
"""
from __future__ import annotations

from .session import (
    FormItem,
    FormResult,
    FormSession,
    ItemView,
    Suggestion,
    change_session,
    open_session,
    sr_session,
)


def __getattr__(name: str):  # Textual is imported only when a screen is needed
    if name in ("FormScreen", "ReviewScreen"):
        from . import screens

        return getattr(screens, name)
    if name in ("FormApp", "run_form", "form_available"):
        from . import app

        return getattr(app, name)
    raise AttributeError(name)


__all__ = [
    "FormApp", "FormItem", "FormResult", "FormScreen", "FormSession", "ItemView", "ReviewScreen", "Suggestion",
    "change_session", "form_available", "open_session", "run_form", "sr_session",
]
