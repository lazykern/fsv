"""Drive the form app headless (Textual ``run_test``) from sync tests."""
from __future__ import annotations

import asyncio
import datetime as dt
from typing import Any, Awaitable, Callable

from rich.text import Text

from fsv.formui.app import FormApp
from fsv.formui.session import FormResult, FormSession
from fsv.forms import LookupValue

JANE = LookupValue(9000007001, "Jane Doe", "jane.doe@example.com")

Script = Callable[[FormApp, Any], Awaitable[None]]


def run_app(session: FormSession, script: Script, size: tuple[int, int] = (120, 40)) -> FormApp:
    async def main() -> FormApp:
        app = FormApp(session)
        async with app.run_test(size=size) as pilot:
            await settle(app, pilot)
            await script(app, pilot)
            await settle(app, pilot)
        return app

    return asyncio.run(main())


def drive(session: FormSession, script: Script, size: tuple[int, int] = (120, 40)) -> FormResult:
    app = run_app(session, script, size)
    return app.result or FormResult("cancel", session.state, None, session=session)


async def settle(app: FormApp, pilot: Any, t: float = 0.05) -> None:
    await pilot.pause(t)
    await app.workers.wait_for_complete()
    await pilot.pause()


async def focus(app: FormApp, pilot: Any, key: str) -> None:
    app.screen.rows[key].focus_editor()
    await pilot.pause()


async def type_text(pilot: Any, text: str) -> None:
    await pilot.press(*text)  # single characters are mapped to key events by Textual


def text_of(widget: Any) -> str:
    c = widget.content
    return c.plain if isinstance(c, Text) else Text.from_markup(str(c)).plain


def fill_change(s: FormSession, tz: dt.tzinfo | None = None) -> None:
    """A complete Acme change (Cloud Only = Yes, so no staging dates)."""
    tz = tz or s.tz
    s.set("requester", JANE)
    s.set("subject", "Billing: rotate keys")
    s.set("cloud_only", "Yes")
    s.set("description", "Rotate the **billing** keys.")
    s.set("planned_start_date", dt.datetime(2030, 11, 26, 19, 0, tzinfo=tz))
    s.set("planned_end_date", dt.datetime(2030, 11, 26, 22, 0, tzinfo=tz))
