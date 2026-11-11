"""Field widgets for the form screen.

Every editor posts :class:`FieldEdited` (key, value) when the person
commits a value; the screen applies it to the :class:`FormSession` and
re-syncs every row (visibility, required star, disabled, choices, errors).
Editors never talk to the engine themselves.
"""
from __future__ import annotations

import datetime as dt
import os
from typing import Any

from rich.markup import escape
from rich.text import Text
from textual import events, on
from textual.actions import SkipAction
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.message import Message
from textual.reactive import reactive
from textual.suggester import Suggester
from textual.widget import Widget
from textual.widgets import Button, Checkbox, Input, Markdown, OptionList, Static, TextArea
from textual.widgets.option_list import Option

from . import when
from .session import FormItem, FormSession, ItemView, Suggestion, human_size

DEBOUNCE_TEXT = 0.3
DEBOUNCE_LOOKUP = 0.25


class FieldEdited(Message):
    """An editor committed a value."""

    def __init__(self, key: str, value: Any) -> None:
        super().__init__()
        self.key = key
        self.value = value


class EditorMixin:
    """Common bits: ``key``, ``item``, ``session`` and ``sync(view)``."""

    item: FormItem
    session: FormSession

    @property
    def key(self) -> str:
        return self.item.key

    def sync(self, view: ItemView) -> None:  # pragma: no cover - overridden
        pass

    def focus_editor(self) -> None:
        self.focus()  # type: ignore[attr-defined]


# ---------------------------------------------------------------------------
# text, number, paragraph, checkbox


class TextEditor(EditorMixin, Input):
    def __init__(self, item: FormItem, session: FormSession, *, number: bool = False, **kw: Any) -> None:
        self.item, self.session = item, session
        super().__init__(placeholder=item.placeholder or ("a number" if number else ""),
                         type="number" if number else "text", **kw)
        self._committed = ""
        self._timer: Any = None

    def sync(self, view: ItemView) -> None:
        if not self.has_focus and self.value != view.text:
            self._committed = view.text
            self.value = view.text

    @on(Input.Changed)
    def _changed(self, event: Input.Changed) -> None:
        event.stop()
        if self._timer is not None:
            self._timer.stop()
        self._timer = self.set_timer(DEBOUNCE_TEXT, self.commit)

    def commit(self) -> None:
        if self.value != self._committed:
            self._committed = self.value
            self.post_message(FieldEdited(self.key, self.value))

    def on_blur(self) -> None:
        self.commit()

    @on(Input.Submitted)
    def _submitted(self, event: Input.Submitted) -> None:
        event.stop()
        self.commit()
        self.screen.focus_next()


class ParagraphEditor(EditorMixin, TextArea):
    DEFAULT_CSS = "ParagraphEditor { height: 5; }"

    def __init__(self, item: FormItem, session: FormSession, **kw: Any) -> None:
        self.item, self.session = item, session
        super().__init__(soft_wrap=True, show_line_numbers=False, tab_behavior="focus", **kw)
        self._committed = ""
        self._timer: Any = None

    def sync(self, view: ItemView) -> None:
        if not self.has_focus and self.text != view.text:
            self._committed = view.text
            self.load_text(view.text)

    @on(TextArea.Changed)
    def _changed(self, event: TextArea.Changed) -> None:
        event.stop()
        if self._timer is not None:
            self._timer.stop()
        self._timer = self.set_timer(DEBOUNCE_TEXT, self.commit)

    def commit(self) -> None:
        if self.text != self._committed:
            self._committed = self.text
            self.post_message(FieldEdited(self.key, self.text))

    def on_blur(self) -> None:
        self.commit()


class DescriptionEditor(EditorMixin, Vertical):
    """Markdown TextArea with a preview (F3). Sent as HTML."""

    DEFAULT_CSS = """
    DescriptionEditor { height: auto; }
    DescriptionEditor > TextArea { height: 10; }
    DescriptionEditor > Markdown { height: auto; max-height: 14; border: round $primary; padding: 0 1; display: none; }
    DescriptionEditor.-preview > Markdown { display: block; }
    DescriptionEditor.-preview > TextArea { display: none; }
    DescriptionEditor > .desc-bar { height: 1; color: $text-muted; }
    """
    BINDINGS = [Binding("f3", "toggle_preview", "Preview", show=True)]

    def __init__(self, item: FormItem, session: FormSession, **kw: Any) -> None:
        self.item, self.session = item, session
        super().__init__(**kw)
        self._committed = ""
        self._timer: Any = None

    def compose(self) -> ComposeResult:
        yield TextArea(soft_wrap=True, show_line_numbers=False, tab_behavior="focus", id="desc-text")
        yield Markdown("", id="desc-preview")
        yield Static(escape("Markdown: **bold**, *italic*, - lists, [link](https://…) · F3 preview"), classes="desc-bar")

    @property
    def area(self) -> TextArea:
        return self.query_one(TextArea)

    def focus_editor(self) -> None:
        self.area.focus()

    def sync(self, view: ItemView) -> None:
        ta = self.area
        if not ta.has_focus and ta.text != view.text:
            self._committed = view.text
            ta.load_text(view.text)

    @on(TextArea.Changed)
    def _changed(self, event: TextArea.Changed) -> None:
        event.stop()
        if self._timer is not None:
            self._timer.stop()
        self._timer = self.set_timer(DEBOUNCE_TEXT, self.commit)

    def commit(self) -> None:
        text = self.area.text
        if text != self._committed:
            self._committed = text
            self.post_message(FieldEdited(self.key, text))

    def on_descendant_blur(self) -> None:
        self.commit()

    def action_toggle_preview(self) -> None:
        on_ = not self.has_class("-preview")
        if on_:
            self.query_one(Markdown).update(self.area.text or "_(empty)_")
        self.set_class(on_, "-preview")
        if not on_:
            self.area.focus()


class CheckEditor(EditorMixin, Checkbox):
    def __init__(self, item: FormItem, session: FormSession, **kw: Any) -> None:
        self.item, self.session = item, session
        super().__init__("Yes", False, **kw)
        self._syncing = False

    def sync(self, view: ItemView) -> None:
        want = view.value is True
        if self.value != want:
            self._syncing = True
            self.value = want

    @on(Checkbox.Changed)
    def _changed(self, event: Checkbox.Changed) -> None:
        event.stop()
        if self._syncing:
            self._syncing = False
            return
        self.post_message(FieldEdited(self.key, bool(event.value)))


# ---------------------------------------------------------------------------
# combo box: dropdowns, multi-selects, people / group / agent lookups


class ComboBox(EditorMixin, Vertical):
    """Type to filter (choices) or to search (lookups); arrows move, Enter
    picks, Esc closes. Multi-selects toggle with Enter and stay open."""

    DEFAULT_CSS = """
    ComboBox { height: auto; }
    ComboBox > OptionList { height: auto; max-height: 9; display: none; border: round $accent; padding: 0; }
    ComboBox.-open > OptionList { display: block; }
    ComboBox.-open.-empty > OptionList { display: none; }
    ComboBox > .combo-chosen { height: auto; color: $accent; padding: 0 1; }
    ComboBox > .combo-note { height: auto; color: $text-muted; padding: 0 1; }
    """
    BINDINGS = [
        Binding("down", "cursor(1)", "Next", show=False),
        Binding("up", "cursor(-1)", "Previous", show=False),
        Binding("pagedown", "cursor(8)", show=False),
        Binding("pageup", "cursor(-8)", show=False),
        Binding("escape", "close", "Close list", show=False),
    ]

    class Opened(Message):
        def __init__(self, combo: "ComboBox") -> None:
            super().__init__()
            self.combo = combo

    def __init__(self, item: FormItem, session: FormSession, **kw: Any) -> None:
        self.item, self.session = item, session
        super().__init__(**kw)
        self.multi = item.widget == "multi"
        self.remote = item.widget == "lookup"
        self._options: list[Suggestion] = []
        self._shown_text = ""  # the input text that reflects the stored value
        self._timer: Any = None
        self._query_seen = ""
        self.searches = 0  # lookups started (tests)

    def compose(self) -> ComposeResult:
        if self.multi:
            yield Static("", classes="combo-chosen")
        hint = "type to search" if self.remote else ("type to filter, Enter toggles" if self.multi
                                                      else "type to filter, Enter picks")
        yield Input(placeholder=self.item.placeholder or hint)
        ol = OptionList()
        ol.can_focus = False
        yield ol
        yield Static("", classes="combo-note")

    @property
    def input(self) -> Input:
        return self.query_one(Input)

    @property
    def options(self) -> OptionList:
        return self.query_one(OptionList)

    @property
    def is_open(self) -> bool:
        return self.has_class("-open")

    def focus_editor(self) -> None:
        self.input.focus()

    def sync(self, view: ItemView) -> None:
        self.input.disabled = view.disabled
        if self.multi:
            self.query_one(".combo-chosen", Static).update(
                ("✓ " + escape(view.text)) if view.text else "[dim]nothing selected[/]")
            if self.is_open and not self.remote:
                self.refresh_options()
            return
        if not self.input.has_focus and self.input.value != view.text:
            self._shown_text = view.text
            self.input.value = view.text
        elif self.is_open and not self.remote:
            self.refresh_options()

    # -- list ---------------------------------------------------------------
    def _query(self) -> str:
        q = self.input.value
        return "" if (not self.multi and q == self._shown_text) else q

    def open(self) -> None:
        if self.input.disabled:
            return
        if not self.is_open:
            self.add_class("-open")
            self.post_message(self.Opened(self))
        if self.remote:
            self._schedule_search(immediate=self.item.entity in ("group", "agent"))
        else:
            self.refresh_options()

    def close(self) -> None:
        self.remove_class("-open")
        self._options = []
        st = self.query_one(".combo-note", Static)
        st.update("")
        st.display = False
        if not self.multi and self.input.value != self._shown_text and not self.input.has_focus:
            self.input.value = self._shown_text

    def refresh_options(self) -> None:
        sugg, note = self.session.suggestions(self.key, self._query())
        self.show_options(sugg, note)

    def show_options(self, sugg: list[Suggestion], note: str = "") -> None:
        self._options = sugg
        ol = self.options
        old = ol.highlighted
        ol.clear_options()
        prompts = []
        for s in sugg:
            t = Text()
            if self.multi:
                t.append("[x] " if s.selected else "[ ] ", style="bold" if s.selected else "")
            t.append(s.label)
            if s.detail:
                t.append(f"  {s.detail}", style="dim")
            prompts.append(Option(t))
        ol.add_options(prompts)
        self.set_class(not sugg, "-empty")
        if sugg:
            ol.highlighted = old if (self.multi and old is not None and old < len(sugg)) else 0
        self._note(note)

    def _note(self, note: str = "") -> None:
        ol = self.options
        desc = ""
        if ol.highlighted is not None and ol.highlighted < len(self._options):
            desc = self._options[ol.highlighted].detail
        text = " · ".join(x for x in (escape(desc) if desc and not self.remote else "", escape(note)) if x)
        st = self.query_one(".combo-note", Static)
        st.update(f"[dim]{text}[/]" if text else "")
        st.display = bool(text)

    # -- events -------------------------------------------------------------
    @on(Input.Changed)
    def _typed(self, event: Input.Changed) -> None:
        event.stop()
        if not self.input.has_focus:
            return
        if not self.multi and event.value == self._shown_text and not self.is_open:
            return  # the picked value written back into the box
        if not self.is_open:
            self.add_class("-open")
            self.post_message(self.Opened(self))
        if self.remote:
            self._schedule_search()
        else:
            self.refresh_options()

    def on_descendant_focus(self, event: events.DescendantFocus) -> None:
        if event.widget is self.input:
            self.open()

    def on_descendant_blur(self, event: events.DescendantBlur) -> None:
        self.set_timer(0.15, self._maybe_close)

    def _maybe_close(self) -> None:
        if not self.input.has_focus:
            self.close()

    def _schedule_search(self, immediate: bool = False) -> None:
        if self._timer is not None:
            self._timer.stop()
        if immediate:
            self._search()
        else:
            self._timer = self.set_timer(DEBOUNCE_LOOKUP, self._search)

    def _search(self) -> None:
        q = self._query()
        self.searches += 1
        self._note("searching…")

        def work() -> None:
            try:
                sugg, note = self.session.suggestions(self.key, q)
            except Exception as e:  # network trouble: show it, never crash the form
                sugg, note = [], " ".join(str(e).split())[:200]
            self.app.call_from_thread(self._found, q, sugg, note)

        self.run_worker(work, thread=True, exclusive=True, group=f"lookup-{id(self)}")

    def _found(self, q: str, sugg: list[Suggestion], note: str) -> None:
        if q != self._query() or not self.is_open:
            return  # stale answer
        self.show_options(sugg, note)

    @on(OptionList.OptionHighlighted)
    def _highlighted(self, event: OptionList.OptionHighlighted) -> None:
        event.stop()
        self._note()

    @on(OptionList.OptionSelected)
    def _clicked(self, event: OptionList.OptionSelected) -> None:
        event.stop()
        self.pick(event.option_index)
        self.input.focus()

    @on(Input.Submitted)
    def _enter(self, event: Input.Submitted) -> None:
        event.stop()
        if not self.is_open:
            self.open()
            return
        if not self.multi and not self.input.value.strip():
            self._shown_text = ""
            self.post_message(FieldEdited(self.key, None))
            self.close()
            return
        hl = self.options.highlighted
        if hl is not None and hl < len(self._options):
            self.pick(hl)

    def pick(self, index: int) -> None:
        s = self._options[index]
        if self.multi:
            current = list(self.session.view(self.key).value or ())
            if s.value in current:
                current.remove(s.value)
            else:
                current.append(s.value)
            self.post_message(FieldEdited(self.key, current or None))
            self.input.value = ""
            return
        self._shown_text = self.session.text_for(self.item, s.value) or s.label
        self.input.value = self._shown_text
        self.post_message(FieldEdited(self.key, s.value))
        self.close()

    def action_cursor(self, n: int) -> None:
        if not self.input.has_focus:
            raise SkipAction()
        if not self.is_open:
            self.open()
            return
        ol = self.options
        if not self._options:
            return
        cur = ol.highlighted if ol.highlighted is not None else -1
        ol.highlighted = max(0, min(len(self._options) - 1, cur + n))

    def action_close(self) -> None:
        if not self.is_open:
            raise SkipAction()
        self.close()


# ---------------------------------------------------------------------------
# dates


class CalendarGrid(Widget, can_focus=True):
    """A month grid: arrows move a day/week, PgUp/PgDn a month, Enter or a
    click picks."""

    DEFAULT_CSS = "CalendarGrid { width: 22; height: 9; padding: 0 0; } CalendarGrid:focus { background: $boost; }"
    BINDINGS = [
        Binding("left", "move(-1)", show=False), Binding("right", "move(1)", show=False),
        Binding("up", "move(-7)", show=False), Binding("down", "move(7)", show=False),
        Binding("pageup", "month(-1)", "Prev month"), Binding("pagedown", "month(1)", "Next month"),
        Binding("home", "today", "Today"), Binding("enter", "pick", "Pick", show=False),
    ]
    cursor: reactive[dt.date] = reactive(dt.date.today)

    class Picked(Message):
        def __init__(self, date: dt.date) -> None:
            super().__init__()
            self.date = date

    def __init__(self, start: dt.date, today: dt.date | None = None, **kw: Any) -> None:
        super().__init__(**kw)
        self.today = today or dt.date.today()
        self.set_reactive(CalendarGrid.cursor, start)

    def render(self) -> Text:
        # plain text first, then styles by offset: every cell keeps its width
        d = self.cursor
        head = f"{when.MONTH_NAMES[d.month - 1]} {d.year}"
        lines = ["‹" + head.center(18) + "›", " ".join(n[:2] for n in when.DAY_NAMES)]
        spans: list[tuple[int, int, str]] = [(0, 20, "bold")]
        offset = 21 + 21  # two lines of 20 chars + newline
        for week in when.month_weeks(d.year, d.month):
            cells = []
            for i, day in enumerate(week):
                cells.append(f"{day.day:2d}")
                start = offset + i * 3
                if day == d:
                    spans.append((start, start + 2, "reverse bold"))
                elif day.month != d.month:
                    spans.append((start, start + 2, "dim"))
                elif day == self.today:
                    spans.append((start, start + 2, "underline"))
            lines.append(" ".join(cells))
            offset += 21
        t = Text("\n".join(lines))
        t.stylize("dim", 21, 41)
        for a, b, style in spans:
            t.stylize(style, a, b)
        return t

    def action_move(self, n: int) -> None:
        self.cursor = self.cursor + dt.timedelta(days=n)

    def action_month(self, n: int) -> None:
        self.cursor = when.add_months(self.cursor, n)

    def action_today(self) -> None:
        self.cursor = self.today

    def action_pick(self) -> None:
        self.post_message(self.Picked(self.cursor))

    def on_click(self, event: events.Click) -> None:
        x, y = event.x, event.y
        if y == 0:
            self.action_month(-1 if x < 11 else 1)
            return
        weeks = when.month_weeks(self.cursor.year, self.cursor.month)
        row, col = y - 2, x // 3
        if 0 <= row < len(weeks) and 0 <= col < 7:
            self.cursor = weeks[row][col]
            self.action_pick()


class DateEditor(EditorMixin, Vertical):
    """Typed date / date-time with a live preview; F2 or the button opens
    the calendar (with time suggestions for date-times)."""

    DEFAULT_CSS = """
    DateEditor { height: auto; }
    DateEditor > Horizontal { height: auto; }
    DateEditor Input { width: 1fr; }
    DateEditor Button { min-width: 10; width: 12; }
    DateEditor > .date-preview { height: auto; color: $success; padding: 0 1; }
    DateEditor > .date-preview.-bad { color: $error; }
    """
    BINDINGS = [Binding("f2", "calendar", "Calendar")]

    class OpenCalendar(Message):
        def __init__(self, editor: "DateEditor") -> None:
            super().__init__()
            self.editor = editor

    def __init__(self, item: FormItem, session: FormSession, **kw: Any) -> None:
        self.item, self.session = item, session
        super().__init__(**kw)
        self.date_only = item.widget == "date"
        self._committed = ""

    def compose(self) -> ComposeResult:
        ph = "YYYY-MM-DD, 26 Nov 2026, tomorrow" if self.date_only else "YYYY-MM-DD HH:MM, tomorrow 19:00, fri 9am"
        with Horizontal():
            yield Input(placeholder=self.item.placeholder or ph)
            yield Button("Calendar", classes="date-cal")
        yield Static("", classes="date-preview")

    @property
    def input(self) -> Input:
        return self.query_one(Input)

    def focus_editor(self) -> None:
        self.input.focus()

    def parse(self, text: str) -> when.Parsed:
        return when.parse_when(text, tz=self.session.tz, tz_source=self.session.tz_source, date_only=self.date_only)

    def sync(self, view: ItemView) -> None:
        self.input.disabled = view.disabled
        self.query_one(Button).disabled = view.disabled
        if not self.input.has_focus and self.input.value != view.text:
            self._committed = view.text
            self.input.value = view.text
            self._preview(view.text)

    def _preview(self, text: str) -> when.Parsed:
        p = self.parse(text)
        st = self.query_one(".date-preview", Static)
        st.set_class(bool(p.error), "-bad")
        st.update(escape(p.error or p.preview))
        return p

    @on(Input.Changed)
    def _typed(self, event: Input.Changed) -> None:
        event.stop()
        self._preview(event.value)

    def commit(self) -> None:
        text = self.input.value
        if text == self._committed:
            return
        p = self._preview(text)
        if p.error:
            return  # shown under the field; the stored value is unchanged
        self._committed = text
        self.post_message(FieldEdited(self.key, p.value))

    def on_descendant_blur(self, event: events.DescendantBlur) -> None:
        if event.widget is self.input:
            self.commit()

    @on(Input.Submitted)
    def _enter(self, event: Input.Submitted) -> None:
        event.stop()
        self.commit()

    @on(Button.Pressed, ".date-cal")
    def _button(self, event: Button.Pressed) -> None:
        event.stop()
        self.action_calendar()

    def action_calendar(self) -> None:
        self.post_message(self.OpenCalendar(self))

    def set_value(self, value: Any) -> None:
        """From the calendar."""
        text = when.to_text(value)
        self.input.value = text
        self._preview(text)
        self._committed = text
        self.post_message(FieldEdited(self.key, value))


# ---------------------------------------------------------------------------
# files


class PathSuggester(Suggester):
    """Completes the last path of ``a.pdf; b.pn`` from the file system
    (→ accepts)."""

    def __init__(self) -> None:
        super().__init__(use_cache=False, case_sensitive=True)

    async def get_suggestion(self, value: str) -> str | None:
        return complete_path(value)


def complete_path(value: str) -> str | None:
    head, _, last = value.rpartition(";")
    frag = last.lstrip()
    if not frag:
        return None
    exp = os.path.expanduser(frag)
    d, base = os.path.split(exp)
    try:
        names = sorted(os.listdir(d or "."))
    except OSError:
        return None
    hits = [n for n in names if n.startswith(base) and (base.startswith(".") or not n.startswith("."))]
    if not hits:
        return None
    common = os.path.commonprefix(hits)
    if len(hits) == 1 and os.path.isdir(os.path.join(d or ".", hits[0])):
        common += "/"
    if common == base:
        return None
    return value + common[len(base):]


class FileEditor(EditorMixin, Vertical):
    DEFAULT_CSS = """
    FileEditor { height: auto; }
    FileEditor > .file-list { height: auto; padding: 0 1; color: $text-muted; }
    """

    def __init__(self, item: FormItem, session: FormSession, **kw: Any) -> None:
        self.item, self.session = item, session
        super().__init__(**kw)
        self._committed = ""

    def compose(self) -> ComposeResult:
        rules = []
        if self.item.allowed_types:
            rules.append(", ".join(self.item.allowed_types))
        if self.item.max_files:
            rules.append(f"max {self.item.max_files}")
        ph = "path/to/file.pdf; another.png (→ completes)" + (f"  [{' · '.join(rules)}]" if rules else "")
        yield Input(placeholder=ph, suggester=PathSuggester())
        yield Static("", classes="file-list")

    @property
    def input(self) -> Input:
        return self.query_one(Input)

    def focus_editor(self) -> None:
        self.input.focus()

    def sync(self, view: ItemView) -> None:
        self.input.disabled = view.disabled
        if not self.input.has_focus and self.input.value != view.text:
            self._committed = view.text
            self.input.value = view.text
        files = [p for p in (view.value or []) if isinstance(p, str)]
        lines = []
        for p in files:
            try:
                lines.append(f"✓ {escape(os.path.basename(p))}  {human_size(os.path.getsize(p))}")
            except OSError:
                lines.append(f"✗ {escape(os.path.basename(p))}  missing")
        if files:
            lines.append("[dim]uploaded only after you confirm the review[/]")
        self.query_one(".file-list", Static).update("\n".join(lines))

    def commit(self) -> None:
        if self.input.value != self._committed:
            self._committed = self.input.value
            self.post_message(FieldEdited(self.key, self.input.value))

    def on_descendant_blur(self, event: events.DescendantBlur) -> None:
        if event.widget is self.input:
            self.commit()

    @on(Input.Submitted)
    def _enter(self, event: Input.Submitted) -> None:
        event.stop()
        self.commit()

    @on(Input.Changed)
    def _typed(self, event: Input.Changed) -> None:
        event.stop()


# ---------------------------------------------------------------------------
# a row


EDITORS = {
    "text": TextEditor, "textarea": ParagraphEditor, "richtext": DescriptionEditor, "checkbox": CheckEditor,
    "choice": ComboBox, "multi": ComboBox, "lookup": ComboBox, "date": DateEditor, "datetime": DateEditor,
    "file": FileEditor,
}


def _safe_id(key: str) -> str:
    return "".join(ch if ch.isalnum() or ch in "_-" else "_" for ch in key)


class FieldRow(Vertical):
    """Label (+ red * when required), editor, help line, inline error."""

    DEFAULT_CSS = """
    FieldRow { height: auto; padding: 0 1; margin: 0 0 1 0; }
    FieldRow:focus-within { background: $boost; }
    FieldRow > .field-label { height: auto; text-style: bold; }
    FieldRow > .field-help { height: auto; color: $text-muted; }
    FieldRow > .field-note { height: auto; color: $warning; }
    FieldRow > .field-error { height: auto; color: $error; }
    FieldRow > .field-notice { height: auto; border: round $secondary; padding: 0 1; }
    """

    def __init__(self, item: FormItem, session: FormSession) -> None:
        super().__init__(id=f"row-{_safe_id(item.key)}")
        self.item = item
        self.session = session
        self.editor: Any = None

    @property
    def key(self) -> str:
        return self.item.key

    def compose(self) -> ComposeResult:
        it = self.item
        if it.widget == "notice":
            f = it.field
            from fsv.forms import plain_text

            yield Static(escape(plain_text((f.content if f else "") or it.label)), classes="field-notice")
            return
        yield Static(escape(it.label), classes="field-label")
        cls = EDITORS.get(it.widget, TextEditor)
        kw: dict[str, Any] = {"id": f"ed-{_safe_id(it.key)}"}
        if it.widget == "number":
            self.editor = TextEditor(it, self.session, number=True, **kw)
        else:
            self.editor = cls(it, self.session, **kw)
        yield self.editor
        first = it.help.split(". ")[0] if it.help else ""
        yield Static(escape(first if len(first) < 160 else first[:159] + "…"), classes="field-help")
        yield Static("", classes="field-note")
        yield Static("", classes="field-error")

    def sync(self) -> ItemView:
        v = self.session.view(self.key)
        self.display = v.visible
        if self.item.widget == "notice" or self.editor is None:
            return v
        star = " [bold red]*[/]" if v.required else ""
        ro = "  [dim](read-only)[/]" if v.disabled else ""
        self.query_one(".field-label", Static).update(escape(self.item.label) + star + ro)
        self.query_one(".field-help", Static).display = bool(self.item.help)
        note = v.disabled_reason if v.disabled else v.note
        for cls, text in ((".field-note", note), (".field-error", v.error)):
            st = self.query_one(cls, Static)
            st.update(escape(text))
            st.display = bool(text)
        if isinstance(self.editor, (TextEditor, ParagraphEditor, CheckEditor)):
            self.editor.disabled = v.disabled
        self.editor.sync(v)
        return v

    def focus_editor(self) -> None:
        if self.editor is not None:
            self.editor.focus_editor()
