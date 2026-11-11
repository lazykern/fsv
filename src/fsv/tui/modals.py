"""Modal screens for TUI write actions."""
from __future__ import annotations

from typing import Any

from rich.markup import escape
from rich.text import Text
from textual import on
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import Button, OptionList, Static, TextArea
from textual.widgets.option_list import Option


class StatusScreen(ModalScreen["Any | None"]):
    """Pick a new status. Dismisses with (id, label) or None."""

    BINDINGS = [Binding("escape", "cancel", "Cancel")]

    def __init__(self, choices: list[tuple[Any, str]], current_label: str) -> None:
        super().__init__()
        self._choices = choices
        self._current = current_label

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog", id="status-dialog"):
            yield Static(f"[bold]Change status[/]  [dim]current: {escape(self._current)}[/]", classes="dialog-title")
            yield OptionList(*[Option(escape(label), id=str(i)) for i, (_, label) in enumerate(self._choices)],
                             id="status-options")
            yield Static("[dim]Enter select · Esc cancel[/]", classes="dialog-hint")

    def on_mount(self) -> None:
        ol = self.query_one(OptionList)
        for i, (_, label) in enumerate(self._choices):
            if label == self._current:
                ol.highlighted = i
                break
        else:
            ol.highlighted = 0 if self._choices else None
        ol.focus()

    @on(OptionList.OptionSelected)
    def _selected(self, event: OptionList.OptionSelected) -> None:
        event.stop()
        self.dismiss(self._choices[event.option_index])

    def action_cancel(self) -> None:
        self.dismiss(None)



class ConfirmScreen(ModalScreen[bool]):
    BINDINGS = [Binding("escape", "cancel", "Cancel"), Binding("y", "confirm", "Confirm", show=False)]

    def __init__(self, title: str, lines: list[str]) -> None:
        super().__init__()
        self._title = title
        self._lines = lines

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog", id="confirm-dialog"):
            yield Static(f"[bold]{escape(self._title)}[/]", classes="dialog-title")
            yield Static(Text("\n".join(self._lines)), id="confirm-body")
            with Horizontal(classes="dialog-buttons"):
                yield Button("Confirm", variant="warning", id="confirm")
                yield Button("Cancel", id="cancel")
            yield Static("[dim]y confirm · Esc cancel[/]", classes="dialog-hint")

    def on_mount(self) -> None:
        self.query_one("#cancel", Button).focus()

    @on(Button.Pressed, "#confirm")
    def _yes(self, event: Button.Pressed) -> None:
        event.stop()
        self.dismiss(True)

    @on(Button.Pressed, "#cancel")
    def _no(self, event: Button.Pressed) -> None:
        event.stop()
        self.dismiss(False)

    def action_confirm(self) -> None:
        self.dismiss(True)

    def action_cancel(self) -> None:
        self.dismiss(False)



class NoteScreen(ModalScreen["tuple[str, bool] | None"]):
    """Compose a note or reply. Dismisses with (body, public) or None."""

    BINDINGS = [
        Binding("escape", "cancel", "Cancel"),
        Binding("ctrl+t", "toggle_public", "Private/Public"),
        Binding("ctrl+s", "submit", "Continue"),
    ]

    def __init__(self, target: str, *, reply: bool = False, can_toggle: bool = True) -> None:
        super().__init__()
        self._target = target
        self._reply = reply
        self._can_toggle = can_toggle and not reply
        self.public = reply

    def compose(self) -> ComposeResult:
        what = "Reply" if self._reply else "Note"
        with Vertical(classes="dialog", id="note-dialog"):
            yield Static(f"[bold]Add {what.lower()}[/]  [dim]{escape(self._target)}[/]", classes="dialog-title")
            yield TextArea(id="note-text")
            yield Static(id="note-visibility")
            yield Static(id="note-preview")
            with Horizontal(classes="dialog-buttons"):
                yield Button("Continue", variant="primary", id="submit")
                yield Button("Cancel", id="cancel")
            yield Static("[dim]Ctrl+S continue · Ctrl+T toggle · Esc cancel[/]", classes="dialog-hint")

    def on_mount(self) -> None:
        self.query_one(TextArea).focus()
        self._refresh()

    def _refresh(self) -> None:
        if self._reply:
            vis = "[b]Visibility: PUBLIC reply[/] [dim](sent to the requester)[/]"
        elif self._can_toggle:
            vis = ("[b]Visibility: PUBLIC[/] [dim](Ctrl+T: private)[/]" if self.public
                   else "[b]Visibility: Private[/] [dim](Ctrl+T: public)[/]")
        else:
            vis = "[b]Visibility: Private[/] [dim](use r for a public reply)[/]"
        self.query_one("#note-visibility", Static).update(Text.from_markup(vis))
        body = self.query_one(TextArea).text.strip()
        preview = Text()
        preview.append("Preview: ", style="bold")
        preview.append(body if body else "(empty)", style="not bold" if body else "dim")
        self.query_one("#note-preview", Static).update(preview)

    @on(TextArea.Changed)
    def _changed(self, event: TextArea.Changed) -> None:
        event.stop()
        self._refresh()

    def action_toggle_public(self) -> None:
        if self._can_toggle:
            self.public = not self.public
            self._refresh()

    def action_submit(self) -> None:
        body = self.query_one(TextArea).text.strip()
        if not body:
            self.app.notify("Note is empty", severity="warning")
            return
        self.dismiss((body, self.public))

    def action_cancel(self) -> None:
        self.dismiss(None)

    @on(Button.Pressed, "#submit")
    def _submit(self, event: Button.Pressed) -> None:
        event.stop()
        self.action_submit()

    @on(Button.Pressed, "#cancel")
    def _cancel(self, event: Button.Pressed) -> None:
        event.stop()
        self.dismiss(None)

