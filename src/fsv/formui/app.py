"""Run the form as a standalone app (the CLI face)."""
from __future__ import annotations

import os
import sys
from typing import Any

from textual.app import App

from fsv import config

from .screens import FormScreen
from .session import FormResult, FormSession, open_session


def form_available() -> bool:
    """Can the full-screen form run here? (a terminal on stdin and stdout;
    ``FSV_NO_FORM=1`` or ``TERM=dumb`` turn it off)."""
    if os.environ.get("FSV_NO_FORM") or os.environ.get("TERM") == "dumb":
        return False
    try:
        return sys.stdin.isatty() and sys.stdout.isatty()
    except (AttributeError, ValueError):
        return False


class FormApp(App[FormResult]):
    """A one-screen app around :class:`FormScreen`."""

    TITLE = "fsv"

    def __init__(self, session: FormSession, **kw: Any) -> None:
        super().__init__(**kw)
        self.session = session
        self.result: FormResult | None = None

    def on_mount(self) -> None:
        theme = (config.load_config().get("forms") or {}).get("theme")
        if theme:
            try:
                self.theme = str(theme)
            except Exception:  # unknown theme name: keep the default
                pass
        self.push_screen(FormScreen(self.session), self._done)

    def _done(self, result: FormResult | None) -> None:
        self.result = result
        self.exit(result)


def run_form(kind: str | FormSession, **ctx: Any) -> FormResult:
    """Open the form full-screen and return what the person did.

    ``kind`` is ``"change"`` (``mode="new"|"edit"``, ``target``, ``record``,
    ``meta``, ``requester``, ``dry_run``) or ``"sr"`` (``item_ref`` or
    ``meta``, ``requester``, ``dry_run``) plus ``client=``; or a ready
    :class:`FormSession`. Nothing is sent here: a ``submit`` result carries
    the reviewed plan for the caller's send path.
    """
    session = kind if isinstance(kind, FormSession) else open_session(kind, **ctx)
    result = FormApp(session).run()
    return result or FormResult("cancel", session.state, None, session=session)
