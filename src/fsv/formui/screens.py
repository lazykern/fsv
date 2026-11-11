"""The form screen, its review screen and the small dialogs."""
from __future__ import annotations

import datetime as dt
import io
import json
import os
from typing import Any

from rich.console import Console
from rich.markup import escape
from rich.text import Text
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen, Screen
from textual.widgets import Button, Checkbox, Footer, Input, OptionList, Static
from textual.widgets.option_list import Option

from . import when
from .session import FormResult, FormSession
from .widgets import CalendarGrid, ComboBox, DateEditor, FieldEdited, FieldRow

NOTE_SECONDS = 8.0


def review_text(review: Any, width: int) -> Text:
    """The same review the CLI prints (``Review.render``), as styled text."""
    buf = io.StringIO()
    con = Console(file=buf, width=max(40, width), force_terminal=True, color_system="truecolor",
                  legacy_windows=False, highlight=False)
    review.render(con)
    return Text.from_ansi(buf.getvalue())


# ---------------------------------------------------------------------------
# dialogs


class ConfirmDialog(ModalScreen[bool]):
    BINDINGS = [Binding("escape", "no", "No"), Binding("y", "yes", "Yes", show=False)]
    DEFAULT_CSS = """
    ConfirmDialog { align: center middle; }
    ConfirmDialog > Vertical { width: 60; max-width: 95%; height: auto; border: thick $primary; padding: 1 2;
                               background: $surface; }
    ConfirmDialog Horizontal { height: auto; margin-top: 1; }
    """

    def __init__(self, question: str, yes: str = "Yes", no: str = "No") -> None:
        super().__init__()
        self.question, self.yes, self.no = question, yes, no

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Static(escape(self.question))
            with Horizontal():
                yield Button(self.yes, id="dlg-yes", variant="error")
                yield Button(self.no, id="dlg-no", variant="primary")

    def on_mount(self) -> None:
        self.query_one("#dlg-no", Button).focus()

    @on(Button.Pressed)
    def _pressed(self, event: Button.Pressed) -> None:
        self.dismiss(event.button.id == "dlg-yes")

    def action_yes(self) -> None:
        self.dismiss(True)

    def action_no(self) -> None:
        self.dismiss(False)


class SavePathDialog(ModalScreen["str | None"]):
    """Ask where to save the draft (asks again before overwriting)."""

    BINDINGS = [Binding("escape", "cancel", "Cancel")]
    DEFAULT_CSS = """
    SavePathDialog { align: center middle; }
    SavePathDialog > Vertical { width: 76; max-width: 95%; height: auto; border: thick $primary; padding: 1 2;
                                background: $surface; }
    SavePathDialog Horizontal { height: auto; margin-top: 1; }
    SavePathDialog .dlg-note { color: $warning; height: auto; }
    """

    def __init__(self, default: str, title: str = "Save draft to") -> None:
        super().__init__()
        self.default, self.title_text = default, title
        self._confirm_overwrite: str | None = None

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Static(f"[b]{escape(self.title_text)}[/b]")
            yield Input(self.default, id="dlg-path")
            yield Static("", classes="dlg-note")
            with Horizontal():
                yield Button("Save", id="dlg-save", variant="primary")
                yield Button("Cancel", id="dlg-cancel")

    def on_mount(self) -> None:
        self.query_one(Input).focus()

    @on(Input.Submitted)
    def _enter(self, event: Input.Submitted) -> None:
        event.stop()
        self._save()

    @on(Button.Pressed, "#dlg-save")
    def _save_btn(self) -> None:
        self._save()

    @on(Button.Pressed, "#dlg-cancel")
    def _cancel_btn(self) -> None:
        self.dismiss(None)

    def _save(self) -> None:
        path = os.path.abspath(os.path.expanduser(self.query_one(Input).value.strip()))
        if not path:
            return
        if os.path.exists(path) and self._confirm_overwrite != path:
            self._confirm_overwrite = path
            self.query_one(".dlg-note", Static).update(f"{escape(path)} exists: press Save again to overwrite")
            return
        self.dismiss(path)

    def action_cancel(self) -> None:
        self.dismiss(None)


class TextDialog(ModalScreen[None]):
    """Show a command (or any text) with a Copy button."""

    BINDINGS = [Binding("escape", "close", "Close")]
    DEFAULT_CSS = """
    TextDialog { align: center middle; }
    TextDialog > Vertical { width: 90%; height: auto; border: thick $primary; padding: 1 2; background: $surface; }
    TextDialog .dlg-text { height: auto; padding: 1; background: $panel; }
    TextDialog Horizontal { height: auto; margin-top: 1; }
    """

    def __init__(self, title: str, text: str, note: str = "") -> None:
        super().__init__()
        self.title_text, self.text, self.note = title, text, note

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Static(f"[b]{escape(self.title_text)}[/b]")
            yield Static(escape(self.text), classes="dlg-text", id="dlg-command")
            if self.note:
                yield Static(f"[dim]{escape(self.note)}[/]")
            with Horizontal():
                yield Button("Copy", id="dlg-copy", variant="primary")
                yield Button("Close", id="dlg-close")

    def on_mount(self) -> None:
        self.query_one("#dlg-copy", Button).focus()

    @on(Button.Pressed, "#dlg-copy")
    def _copy(self) -> None:
        self.app.copy_to_clipboard(self.text)
        self.app.notify("copied to the clipboard (terminal clipboard support permitting)", timeout=4)

    @on(Button.Pressed, "#dlg-close")
    def _close_btn(self) -> None:
        self.dismiss(None)

    def action_close(self) -> None:
        self.dismiss(None)


class DatePickerScreen(ModalScreen[Any]):
    """Calendar (+ time suggestions for date-times). Dismisses with a
    ``date`` / aware ``datetime`` or None."""

    BINDINGS = [Binding("escape", "cancel", "Cancel")]
    DEFAULT_CSS = """
    DatePickerScreen { align: center middle; }
    DatePickerScreen > Vertical { width: 60; max-width: 98%; height: auto; border: thick $primary; padding: 1 2;
                                  background: $surface; }
    DatePickerScreen #dp-body { height: auto; }
    DatePickerScreen #dp-times { width: 1fr; height: 9; margin-left: 2; }
    DatePickerScreen #dp-time { width: 1fr; }
    DatePickerScreen .dp-preview { height: auto; color: $success; margin-top: 1; }
    DatePickerScreen Horizontal.dp-buttons { height: auto; margin-top: 1; }
    """

    def __init__(self, label: str, value: Any, *, date_only: bool, tz: dt.tzinfo, tz_source: str) -> None:
        super().__init__()
        self.label, self.date_only, self.tz, self.tz_source = label, date_only, tz, tz_source
        today = dt.datetime.now(tz).date()
        if isinstance(value, dt.datetime):
            self.day, self.time = value.astimezone(tz).date() if value.tzinfo else value.date(), value.strftime("%H:%M")
        elif isinstance(value, dt.date):
            self.day, self.time = value, "09:00"
        else:
            self.day, self.time = today, "09:00"
        self.today = today

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Static(f"[b]{escape(self.label)}[/b]")
            with Horizontal(id="dp-body"):
                yield CalendarGrid(self.day, today=self.today, id="dp-cal")
                if not self.date_only:
                    with Vertical(id="dp-times"):
                        yield Input(self.time, id="dp-time", placeholder="HH:MM")
                        ol = OptionList(*[Option(t) for t in when.TIME_SUGGESTIONS], id="dp-suggest")
                        yield ol
            yield Static("", classes="dp-preview")
            yield Static("[dim]arrows move · PgUp/PgDn month · Home today · Enter picks[/]")
            with Horizontal(classes="dp-buttons"):
                yield Button("OK", id="dp-ok", variant="primary")
                yield Button("Cancel", id="dp-cancel")

    def on_mount(self) -> None:
        self.query_one(CalendarGrid).focus()
        self._update()

    def value(self) -> Any:
        day = self.query_one(CalendarGrid).cursor
        if self.date_only:
            return day
        p = when.parse_when(f"{day.isoformat()} {self.query_one('#dp-time', Input).value}", tz=self.tz,
                            tz_source=self.tz_source)
        return p.value if p.ok else None

    def _update(self) -> None:
        v = self.value()
        text = when.preview(v, self.tz_source) if v is not None else "type a time like 19:00"
        self.query_one(".dp-preview", Static).update(escape(text))

    def watch_cursor(self) -> None:  # pragma: no cover - not reactive here
        self._update()

    @on(CalendarGrid.Picked)
    def _picked(self, event: CalendarGrid.Picked) -> None:
        event.stop()
        self._update()
        if self.date_only:
            self.dismiss(event.date)
        else:
            self.query_one("#dp-time", Input).focus()

    def on_key(self) -> None:
        self.call_after_refresh(self._update)

    @on(OptionList.OptionSelected, "#dp-suggest")
    def _time(self, event: OptionList.OptionSelected) -> None:
        event.stop()
        self.query_one("#dp-time", Input).value = when.TIME_SUGGESTIONS[event.option_index]
        self._update()
        self.query_one("#dp-ok", Button).focus()

    @on(Input.Changed, "#dp-time")
    def _typed(self) -> None:
        self._update()

    @on(Input.Submitted, "#dp-time")
    def _time_enter(self, event: Input.Submitted) -> None:
        event.stop()
        self._ok()

    @on(Button.Pressed, "#dp-ok")
    def _ok(self) -> None:
        v = self.value()
        if v is not None:
            self.dismiss(v)

    @on(Button.Pressed, "#dp-cancel")
    def _cancel(self) -> None:
        self.dismiss(None)

    def action_cancel(self) -> None:
        self.dismiss(None)


# ---------------------------------------------------------------------------
# review


class ReviewScreen(Screen[str]):
    """The review (same content as the CLI review) with Submit / Back to
    edit / Save draft / Copy as command / Cancel. Dismisses with
    ``submit`` | ``back`` | ``save_draft`` | ``cancel``."""

    BINDINGS = [
        Binding("escape", "back", "Back to edit"),
        Binding("ctrl+s", "submit", "Submit"),
        Binding("f4", "save", "Save draft"),
        Binding("f6", "command", "Copy as command"),
        Binding("f7", "payload", "Show payload"),
    ]
    DEFAULT_CSS = """
    ReviewScreen #rv-title { height: 1; background: $primary; color: $text; text-style: bold; padding: 0 1; }
    ReviewScreen #rv-scroll { height: 1fr; padding: 0 1; }
    ReviewScreen #rv-banner { height: auto; padding: 0 1; color: $warning; text-style: bold; }
    ReviewScreen #rv-payload { display: none; height: auto; color: $text-muted; }
    ReviewScreen.-payload #rv-payload { display: block; }
    ReviewScreen #rv-actions { height: auto; padding: 0 1; }
    ReviewScreen #rv-actions Button { margin-right: 1; }
    ReviewScreen #rv-ack { height: auto; margin: 0 1; }
    """

    def __init__(self, session: FormSession, plan: Any) -> None:
        super().__init__()
        self.session = session
        self.plan = plan

    def compose(self) -> ComposeResult:
        yield Static(f"Review: {escape(self.session.title)}", id="rv-title")
        yield Static("", id="rv-banner")
        with VerticalScroll(id="rv-scroll"):
            yield Static("", id="rv-body")
            yield Static("", id="rv-payload")
        if self.plan.review.blocking_unacked or self.session.ack:
            yield Checkbox("I acknowledge the blocking rule message(s)", self.session.ack, id="rv-ack")
        with Horizontal(id="rv-actions"):
            yield Button("Submit", id="rv-submit", variant="success")
            yield Button("Back to edit", id="rv-back", variant="primary")
            yield Button("Save draft", id="rv-save")
            yield Button("Copy as command", id="rv-command")
            yield Button("Cancel", id="rv-cancel", variant="error")
        yield Footer()

    def on_mount(self) -> None:
        self._refresh_review()
        submit = self.query_one("#rv-submit", Button)
        (submit if not submit.disabled else self.query_one("#rv-back", Button)).focus()

    def on_resize(self) -> None:
        self._refresh_review()

    def _refresh_review(self) -> None:
        plan, s = self.plan, self.session
        width = max(40, (self.size.width or 100) - 4)
        self.query_one("#rv-body", Static).update(review_text(plan.review, width))
        if plan.payload is not None and plan.ok:
            self.query_one("#rv-payload", Static).update(
                Text("Payload\n" + json.dumps(plan.payload, indent=2, default=str, ensure_ascii=False)))
        else:
            self.query_one("#rv-payload", Static).update("No payload yet — fix the problems above")
        banner = []
        if s.kind == "sr":
            from fsv.catalog import SAFETY

            banner.append(f"⚠ {SAFETY}")
        if s.dry_run:
            banner.append("Dry run: Finish shows the plan in the terminal; nothing is sent.")
        no_changes = plan.ok and not plan.hash and s.mode == "edit"
        if no_changes:
            banner.append("No changes: nothing would be sent.")
        if plan.review.blocking_unacked:
            banner.append("A blocking rule message needs your acknowledgement (tick the box below).")
        st = self.query_one("#rv-banner", Static)
        st.update(escape("\n".join(banner)))
        st.display = bool(banner)
        submit = self.query_one("#rv-submit", Button)
        submit.label = "Finish (dry run)" if s.dry_run else ("Place request" if s.kind == "sr" else
                                                            "Update" if s.mode == "edit" else "Create")
        submit.disabled = not (plan.ok and plan.hash) or bool(plan.review.blocking_unacked)
        self.query_one("#rv-command", Button).disabled = not plan.hash

    @on(Checkbox.Changed, "#rv-ack")
    def _ack(self, event: Checkbox.Changed) -> None:
        event.stop()
        self.session.ack = bool(event.value)
        self.plan = self.session.build_plan()
        self._refresh_review()

    @on(Button.Pressed, "#rv-submit")
    def action_submit(self) -> None:
        if self.query_one("#rv-submit", Button).disabled:
            self.app.notify("fix the problems first (Back to edit)", severity="warning", timeout=4)
            return
        self.dismiss("submit")

    @on(Button.Pressed, "#rv-back")
    def action_back(self) -> None:
        self.dismiss("back")

    @on(Button.Pressed, "#rv-cancel")
    def _cancel(self) -> None:
        self.dismiss("cancel")

    @on(Button.Pressed, "#rv-save")
    def action_save(self) -> None:
        def done(path: str | None) -> None:
            if path:
                self.session.save_draft(path, self.plan)
                self.dismiss("save_draft")

        self.app.push_screen(SavePathDialog(self.session.default_draft_path()), done)

    @on(Button.Pressed, "#rv-command")
    def action_command(self) -> None:
        if not self.plan.hash:
            self.app.notify("no complete plan yet", severity="warning", timeout=4)
            return

        def done(path: str | None) -> None:
            if not path:
                return
            saved = self.session.save_draft(path, self.plan)
            self.app.push_screen(TextDialog(
                "Equivalent command", self.session.command_for(os.path.relpath(saved), self.plan),
                f"draft saved to {saved}; the command refuses to send if the plan changes"))

        self.app.push_screen(SavePathDialog(self.session.default_draft_path(), "Save the draft for the command to"),
                             done)

    def action_payload(self) -> None:
        self.toggle_class("-payload")


# ---------------------------------------------------------------------------
# form


class FormScreen(Screen[FormResult]):
    """A Freshservice create/edit form, laid out like the web's.

    Push it from any Textual app: ``app.push_screen(FormScreen(session),
    callback)``; the callback receives a :class:`FormResult`
    (``submit`` with the reviewed plan, ``save_draft`` with the path, or
    ``cancel``). Nothing is sent by the screen itself.
    """

    BINDINGS = [
        Binding("ctrl+s", "review", "Review", priority=True),
        Binding("f4", "save", "Save draft"),
        Binding("escape", "cancel", "Cancel"),
        Binding("f1", "toggle_help", "Help panel"),
    ]
    DEFAULT_CSS = """
    FormScreen #form-title { height: 1; background: $primary; color: $text; text-style: bold; padding: 0 1; }
    FormScreen #form-notes { height: auto; max-height: 4; padding: 0 1; color: $accent; }
    FormScreen #form-body { height: 1fr; }
    FormScreen #form-fields { width: 2fr; }
    FormScreen #form-help { width: 1fr; min-width: 30; max-width: 64; border-left: solid $primary; padding: 0 1; }
    FormScreen.-narrow #form-body { layout: vertical; }
    FormScreen.-narrow #form-fields { width: 100%; height: 1fr; }
    FormScreen.-narrow #form-help { width: 100%; max-width: 100%; height: 9; min-width: 0; border-left: none;
                                    border-top: solid $primary; }
    FormScreen.-hide-help #form-help { display: none; }
    FormScreen #form-actions { height: auto; padding: 0 1; margin-bottom: 1; }
    FormScreen #form-actions Button { margin-right: 1; }
    FormScreen #form-status { height: 1; padding: 0 1; background: $panel; color: $text-muted; }
    """
    NARROW = 100

    def __init__(self, session: FormSession, **kw: Any) -> None:
        super().__init__(**kw)
        self.session = session
        self._help_key: str | None = None
        self._note_timer: Any = None
        self.rows: dict[str, FieldRow] = {}

    def compose(self) -> ComposeResult:
        s = self.session
        sub = " (edit)" if s.mode == "edit" else ""
        yield Static(escape(s.title) + sub + ("  [dim]dry run[/]" if s.dry_run else ""), id="form-title")
        yield Static("", id="form-notes")
        with Horizontal(id="form-body"):
            with VerticalScroll(id="form-fields"):
                for it in s.items:
                    row = FieldRow(it, s)
                    self.rows[it.key] = row
                    yield row
                with Horizontal(id="form-actions"):
                    yield Button("Review & submit", id="btn-review", variant="primary")
                    yield Button("Save draft", id="btn-save")
                    yield Button("Cancel", id="btn-cancel", variant="error")
            with VerticalScroll(id="form-help"):
                yield Static("", id="help-text")
        yield Static("", id="form-status")
        yield Footer()

    def on_mount(self) -> None:
        self.set_class(self.app.size.width < self.NARROW, "-narrow")
        self.refresh_rows()
        if self.session.load_notes:
            self.show_notes(self.session.load_notes)
        self.call_after_refresh(self._focus_first)

    def _focus_first(self) -> None:
        for key in self.session.visible_keys():
            row = self.rows[key]
            if row.editor is not None and not self.session.view(key).disabled:
                row.focus_editor()
                return

    def on_resize(self) -> None:
        self.set_class(self.app.size.width < self.NARROW, "-narrow")

    # -- syncing -------------------------------------------------------------
    def refresh_rows(self) -> None:
        for row in self.rows.values():
            row.sync()
        problems = self.session.problems()
        missing = [self.session.by_key[k].label for k in self.session.missing_keys()]
        btn = self.query_one("#btn-review", Button)
        btn.disabled = bool(problems)
        if missing:
            shown = ", ".join(missing[:4]) + (f" +{len(missing) - 4}" if len(missing) > 4 else "")
            status = f"{len(missing)} required field(s) missing: {shown}"
        elif problems:
            status = problems[0]
        else:
            status = "Ready: Ctrl+S to review and submit"
        status += "  ·  Ctrl+S review · F4 save draft · Esc cancel"
        self.query_one("#form-status", Static).update(escape(status))
        if self._help_key:
            self.show_help(self._help_key, force=True)

    def show_notes(self, notes: list[str]) -> None:
        if not notes:
            return
        self.query_one("#form-notes", Static).update(escape("\n".join(notes[-4:])))
        self.app.notify("\n".join(notes), timeout=6)
        if self._note_timer is not None:
            self._note_timer.stop()
        self._note_timer = self.set_timer(NOTE_SECONDS, lambda: self.query_one("#form-notes", Static).update(""))

    def show_help(self, key: str, force: bool = False) -> None:
        if key == self._help_key and not force:
            return
        self._help_key = key
        h = self.session.help_for(key)
        lines = [f"[b]{escape(h['label'])}[/b]" + (" [bold red]* required[/]" if h["required"] else " [dim]optional[/]")]
        if h["help"]:
            lines.append(escape(h["help"]))
        if h["placeholder"]:
            lines.append(f"[dim]e.g. {escape(h['placeholder'])}[/]")
        if h["disabled"]:
            lines.append(f"[yellow]read-only: {escape(h['disabled'])}[/]")
        if h["note"]:
            lines.append(f"[yellow]{escape(h['note'])}[/]")
        if h["choices"]:
            lines.append("")
            lines.append("[b]Choices[/b]")
            for label, desc in h["choices"][:40]:
                lines.append(f" • {escape(label)}" + (f" [dim]— {escape(desc)}[/]" if desc else ""))
            if len(h["choices"]) > 40:
                lines.append(f" [dim]… {len(h['choices'])} in all; type to filter[/]")
        if h["effects"]:
            lines.append("")
            lines.append("[b]Rule effects[/b]")
            lines += [f" {escape(e)}" for e in h["effects"]]
        if h["why"]:
            lines.append("")
            lines += [f"[dim]{escape(w)}[/]" for w in h["why"]]
        kind_hint = {
            "choice": "Type to filter · ↑↓ move · Enter picks · Esc closes",
            "multi": "Type to filter · ↑↓ move · Enter toggles · Esc closes",
            "lookup": "Type to search · ↑↓ move · Enter picks",
            "date": "Type a date or press F2 for the calendar",
            "datetime": "Type a date and time or press F2 for the calendar",
            "file": "Paths separated by ';' · → completes the path",
            "richtext": "Markdown, sent as HTML · F3 preview",
        }.get(h["kind"])
        if kind_hint:
            lines.append("")
            lines.append(f"[dim]{kind_hint}[/]")
        self.query_one("#help-text", Static).update("\n".join(lines))

    # -- events --------------------------------------------------------------
    def on_descendant_focus(self, event: Any) -> None:
        node = event.widget
        while node is not None and not isinstance(node, FieldRow):
            node = node.parent
        if isinstance(node, FieldRow):
            self.show_help(node.key)

    @on(FieldEdited)
    def _edited(self, event: FieldEdited) -> None:
        event.stop()
        out = self.session.set(event.key, event.value)
        self.refresh_rows()
        if out.notes:
            self.show_notes(out.notes)

    @on(ComboBox.Opened)
    def _opened(self, event: ComboBox.Opened) -> None:
        event.stop()
        self.call_after_refresh(lambda: event.combo.scroll_visible(animate=False))

    @on(DateEditor.OpenCalendar)
    def _calendar(self, event: DateEditor.OpenCalendar) -> None:
        event.stop()
        ed = event.editor
        view = self.session.view(ed.key)

        def done(value: Any) -> None:
            if value is not None:
                ed.set_value(value)
            ed.input.focus()

        self.app.push_screen(DatePickerScreen(ed.item.label, view.value, date_only=ed.date_only,
                                              tz=self.session.tz, tz_source=self.session.tz_source), done)

    @on(Button.Pressed, "#btn-review")
    def _review_btn(self) -> None:
        self.action_review()

    @on(Button.Pressed, "#btn-save")
    def _save_btn(self) -> None:
        self.action_save()

    @on(Button.Pressed, "#btn-cancel")
    def _cancel_btn(self) -> None:
        self.action_cancel()

    # -- actions -------------------------------------------------------------
    def _commit_focused(self) -> None:
        f = self.focused
        node = f
        while node is not None and not isinstance(node, FieldRow):
            node = node.parent
        if isinstance(node, FieldRow) and hasattr(node.editor, "commit"):
            node.editor.commit()

    def action_review(self) -> None:
        self._commit_focused()
        self.call_after_refresh(self._open_review)

    def _open_review(self) -> None:
        s = self.session
        s.show_all_errors = True
        self.refresh_rows()
        plan = s.build_plan()
        self.app.push_screen(ReviewScreen(s, plan), self._after_review)

    def _after_review(self, action: str | None) -> None:
        s = self.session
        if action == "submit":
            self.dismiss(FormResult("submit", s.state, s.plan, session=s))
        elif action == "save_draft":
            self.dismiss(FormResult("save_draft", s.state, s.plan, saved_path=s.saved_path, session=s))
        elif action == "cancel":
            self.dismiss(FormResult("cancel", s.state, None, session=s))
        else:
            missing = s.missing_keys() or [k for k in s.errors_by_key if s.view(k).visible]
            if missing:
                self.rows[missing[0]].focus_editor()

    def action_save(self) -> None:
        self._commit_focused()

        def done(path: str | None) -> None:
            if path:
                saved = self.session.save_draft(path)
                self.dismiss(FormResult("save_draft", self.session.state, self.session.plan, saved_path=saved,
                                        session=self.session))

        self.app.push_screen(SavePathDialog(self.session.default_draft_path()), done)

    def action_cancel(self) -> None:
        s = self.session
        if not s.touched:
            self.dismiss(FormResult("cancel", s.state, None, session=s))
            return

        def done(yes: bool | None) -> None:
            if yes:
                self.dismiss(FormResult("cancel", s.state, None, session=s))

        self.app.push_screen(ConfirmDialog("Discard this form? Nothing has been sent.", "Discard", "Keep editing"),
                             done)

    def action_toggle_help(self) -> None:
        self.toggle_class("-hide-help")
