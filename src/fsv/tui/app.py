from __future__ import annotations

import json
import shlex
import subprocess
import webbrowser
from typing import Any

from rich.markup import escape
from rich.text import Text

from textual import work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.reactive import var
from textual.screen import ModalScreen
from textual.widgets import DataTable, Input, Static
from textual.widgets._data_table import RowKey

from fsv import config, pages, schema as schema_mod, service
from fsv import views as V
from fsv.client import get_client
from fsv.errors import SessionError
from fsv.query import build_query_hash_from_schema
from fsv.render import strip_html
from fsv.resources import CHANGES, PROBLEMS, TICKETS, Resource, format_id
from fsv.tui import actions
from fsv.tui.modals import (ChoiceScreen, ConfirmScreen, FailureScreen, NoteScreen,
                            StatusScreen, ValueScreen, ViewPickerScreen)


ALL_RESOURCES = (TICKETS, CHANGES, PROBLEMS)

_OPERATORS = (">=", "<=", "!=", "~=", "=", ">", "<")


def _split_filter_token(token: str) -> tuple[str, str, str] | None:
    for op in _OPERATORS:
        if op in token:
            left, right = token.split(op, 1)
            return left.strip(), op, right
    return None

ENTITY_TABS = ("tickets", "problems", "changes")

DETAIL_TAB_LABELS: dict[str, list[str]] = {
    "changes": ["details", "notes", "tasks", "assets", "associations", "approvals", "activity"],
    "tickets": ["details", "conversations", "tasks", "assets", "associations", "approvals", "activity", "resolution"],
    "problems": ["details", "notes", "tasks", "assets", "associations", "activity"],
}

ENTITY_TAB_KEY = {"t": "tickets", "p": "problems", "c": "changes"}


def _resource_for_entity(name: str) -> Resource | None:
    return {"tickets": TICKETS, "problems": PROBLEMS, "changes": CHANGES}.get(name)


def _form_session(mode: str, item: dict | None, page: Any = None) -> Any:
    """The ``fsv.formui`` session for a new change, or for editing ``item``
    (``change_session(client, mode=..., target=..., record=...)``)."""
    import fsv.formui as formui  # noqa: PLC0415 - optional

    record = dict(page.record) if page is not None else (dict(item) if item else None)
    if record is not None:
        record.pop("_resource", None)
    client = get_client()
    factory = getattr(formui, "change_session", None)
    if callable(factory):
        if mode == "new":
            return factory(client)
        return factory(client, mode="edit", target=(record or {}).get("display_id") or (record or {}).get("id"),
                       record=record)
    fn = getattr(formui, "open_new_change" if mode == "new" else "open_edit_change", None)
    if callable(fn):
        return fn(client) if mode == "new" else fn(client, record)
    raise ImportError("fsv.formui has no change_session")


def _clip(value: Any, width: int) -> str:
    text = "" if value is None else str(value)
    return text if len(text) <= width else text[: width - 1] + "…"


def _markup(value: Any) -> str:
    return escape("" if value is None else str(value))


class HelpScreen(ModalScreen[None]):
    DEFAULT_CSS = """
    HelpScreen {
        align: center middle;
    }

    #help-dialog {
        width: 76;
        height: auto;
        max-height: 90%;
        border: heavy $accent;
        padding: 1 2;
    }
    """

    HELP_TEXT = """[bold cyan]fsv[/] help

[bold]Navigation[/]
  [cyan]Tab[/]          switch pane
  [cyan]j/k[/]          move list or scroll detail
  [cyan]J/K[/]          move or scroll ×5
  [cyan]G/gg[/]         bottom/top in active pane
  [cyan]h/l[/]          entity in list pane, tabs in detail pane
  [cyan]gt/gp/gc[/]     jump to tickets/problems/changes
  [cyan]1-9[/]          switch detail tab (the tabs the web shows for the item)
  [cyan]v[/]            pick a saved view by name (the list opens with the default view)
  [cyan]\\[][/cyan]         resize list/detail pane

[bold]Search[/]
  [cyan]f[/]            fulltext search across all entities
  [cyan]h/l[/]          narrow results: all → tickets → problems → changes
  [cyan]s[/]            cycle sort: relevance → date created → last modified
  [cyan]Esc[/]          exit search mode
  pagination auto-loads at list bottom (per-entity only)

[bold]Filter[/]
  [cyan]/[/]            open filter input (FIELD=VALUE, space-separated)
  [cyan]↑/↓[/] or [cyan]Ctrl+P/N[/]  browse filter history
  [cyan]Tab[/]          autocomplete field/value
  [cyan]Esc[/]          cancel filter input
  [cyan]Enter[/]        apply filter and reload
  empty input clears active filter

[bold]Selection[/]
  moving list cursor loads detail automatically
  [cyan]y[/]            copy selected display id
  [cyan]o[/]            open selected item in browser

[bold]Forms[/] (off with --read-only)
  [cyan]N[/]            choose Change / Incident / Service Request
  [cyan]e[/]            edit the selected change

[bold]Write actions[/] (changes/tickets; off with --read-only)
  [cyan]s[/]            change status: pick, then confirm
  [cyan]n[/]            add note (private by default; Ctrl+T toggles public)
  [cyan]r[/]            reply to ticket (detail pane; public, confirmed)
  [cyan]a[/]            add/update/delete task / initiating-ticket / attachment actions
  [cyan]![/]            reopen the last persistent failure

[bold]App[/]
  [cyan]r[/]            reload list and clear caches (list pane)
  [cyan]q[/]/[cyan]ctrl+c[/]     quit
  [cyan]?[/]            close/open this help

Press any key to close."""

    def compose(self) -> ComposeResult:
        yield Static(Text.from_markup(self.HELP_TEXT), id="help-dialog")

    def on_key(self, event) -> None:
        self.dismiss()
        event.stop()
        event.prevent_default()


class FsvApp(App):
    CSS_PATH = "app.tcss"
    TITLE = "fsv"
    BINDINGS = [Binding("ctrl+c", "quit", "Quit", priority=True, show=False)]

    entity = var("tickets")
    detail_tab_idx = var(0)

    def __init__(self, read_only: bool = False) -> None:
        super().__init__(ansi_color=True)
        self.read_only = actions.resolve_read_only(read_only)
        self._items: list[dict[str, Any]] = []
        self._total = 0
        self._schemas: dict[str, dict] = {}
        self._selected: dict[str, Any] | None = None
        self._detail_cache: dict[str, dict] = {}
        self._sub_cache: dict[str, Any] = {}
        self._g_pending = False
        self._g_pending_pane = "list"
        self._pending_detail_key: str | None = None
        self._pending_sub_key: str | None = None
        self._detail_debounce_timer = None
        self._entity_debounce_timer = None
        self._list_fr = 11
        self._detail_fr = 9
        self._page = 1
        self._list_epoch = 0
        self._write_failure: str | None = None
        self._per_page = 100
        self._has_more = False
        self._loading_more = False
        self._active_filters: list[str] = self._get_default_filters("tickets")
        self._filter_error: str | None = None
        self._suggestions: list[str] = []
        self._suggestion_idx: int = 0
        self._tab_lock: int = 0
        self._tab_base: str | None = None
        self._completion_lookup_timer = None
        self._completion_lookup_key: str = ""
        self._filter_history: list[str] = []
        self._history_idx: int = -1
        self._history_saved: str = ""
        self._history_nav_lock: int = 0
        self._search_mode: bool = False
        self._search_term: str = ""
        self._search_totals: dict[str, int] = {}
        self._search_entity: str | None = None
        self._search_sort: str = "relevance"
        self._search_page: int = 1
        self._search_has_more: bool = False
        self._search_seq: int = 0
        self._search_cache: dict[str | None, tuple[list[dict], dict[str, int], int, bool]] = {}
        self._state_flow_cache: dict[int, list[dict]] = {}
        # site-like views: saved views per entity, the current one, the
        # signed-in agent, list meta, and the loaded detail pages
        self._views: dict[str, list[Any]] = {}
        self._view: dict[str, Any] = {}
        self._headers: dict[str, list[dict[str, Any]]] = {}
        self._me: dict[str, Any] | None = None
        self._tz: Any = None
        self._list_total: int | None = None
        self._list_sort: str = "created_at"
        self._columns: list[Any] = []
        self._details: dict[str, Any] = {}
        self._load_filter_history()

    def compose(self) -> ComposeResult:
        yield Static(id="header")
        with Vertical(id="main"):
            with Vertical(id="list-pane"):
                yield DataTable(id="list")
                yield Input(id="filter-input", placeholder="status=Open priority=High")
                yield Input(id="search-input", placeholder="search freshservice...")
                yield Static(id="suggestion-bar")
                yield Static(id="filter-bar")
            with Vertical(id="detail"):
                yield Static(id="detail-bar")
                yield Static(id="state-flow-bar")
                yield Static(id="reporter-line")
                yield Static(id="detail-tabs")
                with Horizontal(id="detail-body"):
                    with VerticalScroll(id="detail-scroll"):
                        yield Static(id="detail-content")
                    with VerticalScroll(id="detail-side"):
                        yield Static(id="detail-props")
        yield Static(id="status-bar")
        yield Static(id="key-hints")

    def on_mount(self) -> None:
        self.screen.styles.background = "transparent"
        for widget in self.query("Vertical, VerticalScroll, Static"):
            widget.styles.background = "transparent"
        self.query_one("#state-flow-bar", Static).display = False
        self.query_one("#reporter-line", Static).display = False
        self.query_one("#detail-side").display = False
        filter_input = self.query_one("#filter-input", Input)
        filter_input.display = False
        filter_input.styles.background = "transparent"
        search_input = self.query_one("#search-input", Input)
        search_input.display = False
        search_input.styles.background = "transparent"
        self.query_one("#suggestion-bar", Static).display = False
        table = self.query_one("#list", DataTable)
        table.cursor_type = "row"
        table.zebra_stripes = False
        table.styles.background = "transparent"
        table.focus()
        self._render_header()
        self._render_key_hints()
        self._clear_detail("[dim]loading rows...[/]")
        self._load_list()

    # ── Data loading ────────────────────────────────────────

    @work(thread=True, exclusive=True, group="list")
    def _load_list(self, append: bool = False) -> None:
        epoch, entity = self._list_epoch, self.entity
        requested_page = self._page + 1 if append else 1
        try:
            self._fetch_list(append, epoch, entity, requested_page)
        except Exception as exc:  # noqa: BLE001 - retain loaded rows for retry
            self.call_from_thread(self._list_failed, epoch, entity, str(exc))

    def _list_failed(self, epoch: int, entity: str, message: str) -> None:
        if (epoch, entity) != (self._list_epoch, self.entity):
            return
        self._loading_more = False
        self.notify(message, title="List load failed; reload or retry", severity="error")
        self._render_status()

    def _fetch_list(self, append: bool, epoch: int, entity: str, requested_page: int) -> None:
        c = get_client()
        for res in ALL_RESOURCES:
            if res.name not in self._schemas:
                try:
                    self._schemas[res.name] = schema_mod.load(res, c)
                except SessionError:
                    raise
                except Exception:  # noqa: BLE001 - module off on this account (e.g. no problems)
                    self._schemas[res.name] = {"fields": []}
        if self._me is None:
            self._me = V.current_user(c)
            self._tz = V.user_tz(self._me)

        res = _resource_for_entity(entity)
        if res.name not in self._views:
            views, _miss = V.load_views(c, res)
            self._views[res.name] = views
            self._view.setdefault(res.name, V.default_view(views))
        if res == TICKETS and res.name not in self._headers:
            self._headers[res.name] = V.ticket_headers(c)
        query_hash: str | None = None
        if self._active_filters:
            try:
                sch = self._schemas.get(res.name, {"fields": []})
                query_hash = build_query_hash_from_schema(c, res, sch, self._active_filters)
            except ValueError as exc:
                self.call_from_thread(self._list_failed, epoch, entity, str(exc))
                return
        view = None if self._active_filters else self._view.get(res.name)
        order_by, order_type = "created_at", "desc"
        if view is not None and view.raw:
            settings = V.view_settings(c, res, view)
            if settings.get("order_by"):
                order_by, order_type = settings["order_by"], settings.get("order_type") or "desc"
        self._list_sort = order_by
        items, data = service.list_page(
            res, client=c, page=requested_page, per_page=self._per_page, query_hash=query_hash,
            filter_name=view.id if view is not None else None, order_by=order_by, order_type=order_type,
        )
        total, more = V.list_meta(data, items, requested_page, self._per_page)
        self.call_from_thread(self._accept_list, epoch, entity, requested_page, items, total, more, append)

    def _accept_list(self, epoch, entity, page, items, total, more, append) -> None:
        if (epoch, entity) != (self._list_epoch, self.entity) or self._search_mode:
            return
        if append and page != self._page + 1:
            return
        self._page = page
        self._list_total = total
        lower_bound = len(self._items) + len(items) if append else len(items)
        self._populate_list(items, total if total is not None else lower_bound, append, has_more=more)

    def _list_columns(self, res: Resource) -> list[Any]:
        return V.list_columns(res, self._schemas.get(res.name, {"fields": []}), self._headers.get(res.name))

    def _populate_list(self, items: list[dict], total: int, append: bool = False,
                       has_more: bool | None = None) -> None:
        self._loading_more = False
        self._total = total

        self._has_more = has_more if has_more is not None else len(items) == self._per_page
        seen = {self._item_key(row, row.get("_resource", TICKETS)) for row in self._items} if append else set()
        unique = []
        for row in items:
            key = self._item_key(row, row.get("_resource", TICKETS))
            if key not in seen:
                seen.add(key)
                unique.append(row)
        items = unique

        table = self.query_one("#list", DataTable)
        res_now = _resource_for_entity(self.entity) or TICKETS

        if not append:
            self._items = items
            self._columns = self._list_columns(res_now) if not self._search_mode else []
            table.clear(columns=True)
            if self._columns:
                table.add_columns("", *[col.label for col in self._columns])
            else:
                table.add_columns("", "ID", "STATUS", "PRI", "SUBJECT", "REQUESTER")
            offset = 0
        else:
            offset = len(self._items)
            self._items.extend(items)

        me_id = (self._me or {}).get("id")
        for idx, item in enumerate(items):
            row_num = offset + idx
            res: Resource = item.get("_resource", TICKETS)
            schema = self._schemas.get(res.name, {"fields": []})
            did = format_id(item, res)
            sel = f"{row_num + 1:03d}"
            if self._columns:
                row = V.list_row(item, res, self._columns, schema, me_id, self._tz)
                cells: list[Any] = []
                for col in self._columns:
                    val = row.get(col.key, "")
                    if col.key == "subject":
                        t = Text(f"#{did} ", style="dim cyan")
                        t.append(_clip(val, 60))
                        cells.append(t)
                    elif col.key == "status":
                        cells.append(Text(_clip(val, 24), style="green"))
                    elif col.key == "assigned_to" and val.endswith(" (Me)") and len(val) > 40:
                        cells.append(Text(_clip(val[:-5], 35) + " (Me)"))
                    else:
                        cells.append(Text(_clip(val, 40 if col.key == "assigned_to" else 34) or "--",
                                          style="" if val else "dim"))
                table.add_row(sel, *cells, key=did)
                continue
            status = service.resolve_status(item, res, schema)
            pri = service.resolve_priority(item)
            subject = (item.get("subject") or "")[:80]
            requester = ((item.get("requester") or {}).get("name") if isinstance(item.get("requester"), dict) else "") or ""
            table.add_row(sel, did, status, pri, subject, requester, key=did)

        if not append and items:
            self._clear_detail("[dim]loading details...[/]")
            table.move_cursor(row=0)
        elif not append and not items:
            self._clear_detail("[dim]no rows[/]")

        self._render_header()
        self._render_filter_bar()
        self._render_status()

    @work(thread=True, exclusive=True, group="detail")
    def _load_detail(self, item: dict, detail_key: str) -> None:
        res: Resource = item.get("_resource", TICKETS)
        item_id = item.get("id")
        cache_key = f"{res.name}:{item_id}"

        if cache_key in self._detail_cache and cache_key in self._details:
            detail = self._detail_cache[cache_key]
        else:
            c = get_client()
            try:
                page = V.load_detail(c, res, item_id, schema=self._schemas.get(res.name))
            except Exception as exc:  # noqa: BLE001 - shown in the pane
                if cache_key not in self._detail_cache:
                    self.call_from_thread(self._detail_error, detail_key, actions.clean_error(exc))
                    return
                page = None
            if page is not None:
                page.record["_resource"] = res
                self._details[cache_key] = page
                detail = page.record
                self._detail_cache[cache_key] = detail
            else:
                detail = self._detail_cache[cache_key]

        self.call_from_thread(self._show_detail, detail, res, detail_key)

    def _detail_error(self, detail_key: str, message: str) -> None:
        if self._pending_detail_key != detail_key:
            return
        self.query_one("#detail-content", Static).update(Text(f"could not load: {message}", style="red"))

    def _page_for(self, item: dict | None) -> Any:
        if not item:
            return None
        res: Resource = item.get("_resource", TICKETS)
        return self._details.get(f"{res.name}:{item.get('id')}")

    def _show_detail(self, item: dict, resource: Resource, detail_key: str) -> None:
        if self._pending_detail_key != detail_key:
            return
        self._selected = item
        self._pending_sub_key = None
        tabs = self._tab_names(resource)
        if self.detail_tab_idx >= len(tabs):
            self.detail_tab_idx = 0
        self._render_detail_bar(item, resource)
        self._trigger_state_flow(item, resource)
        self._render_detail_tabs(resource)
        self._render_detail_content(item, resource)
        self._render_filter_bar()
        self._render_status()

    @work(thread=True, exclusive=True, group="sub")
    def _load_sub_resource(self, tab_name: str, sub_key: str) -> None:
        if not self._selected:
            return
        item = self._selected
        res: Resource = item.get("_resource", TICKETS)
        item_id = item.get("id")
        cache_key = f"{res.name}:{item_id}:{tab_name}"

        if cache_key in self._sub_cache:
            data = self._sub_cache[cache_key]
        else:
            c = get_client()
            page = self._page_for(item)
            record = page.record if page is not None else item
            try:
                data = V.load_tab(c, res, record, tab_name)
            except Exception as exc:  # noqa: BLE001 - shown in the pane
                data = V.TabData(tab_name, V.tab_label(tab_name), missing=V.Missing(actions.clean_error(exc)))
            self._sub_cache[cache_key] = data

        self.call_from_thread(self._render_sub_content, tab_name, data, res, sub_key)

    def _item_key(self, item: dict, resource: Resource) -> str:
        return f"{resource.name}:{item.get('id')}"

    def _selected_key(self) -> str | None:
        if not self._selected:
            return None
        resource: Resource = self._selected.get("_resource", TICKETS)
        return self._item_key(self._selected, resource)

    def _tab_names(self, resource: Resource | None = None) -> list[str]:
        """The tabs the web shows for the selected item (from the tabs API),
        else fsv's fallback list."""
        page = self._page_for(self._selected)
        if page is not None and (resource is None or page.resource.name == resource.name):
            return page.tab_names()
        if resource is None:
            resource = (self._selected or {}).get("_resource", TICKETS)
        return DETAIL_TAB_LABELS.get(resource.name, ["details"])

    def _current_tab_name(self, resource: Resource | None = None) -> str:
        if resource is None and not self._selected:
            return "details"
        tabs = self._tab_names(resource)
        return tabs[min(self.detail_tab_idx, len(tabs) - 1)]

    def _maybe(self, selector: str) -> Any:
        try:
            return self.query_one(selector)
        except Exception:  # noqa: BLE001 - optional widget (tests stub query_one)
            return None

    def _show_side(self, lines: list[str] | None) -> None:
        side, props = self._maybe("#detail-side"), self._maybe("#detail-props")
        if side is None or props is None:
            return
        if lines is None:
            side.display = False
            props.update("")
            return
        props.update(Text.from_markup("\n".join(lines)))
        side.display = True

    def _active_pane(self) -> str:
        if self.query_one("#detail-scroll", VerticalScroll).has_focus_within:
            return "detail"
        return "list"

    def _focus_pane(self, pane: str) -> None:
        if pane == "detail":
            self.query_one("#detail-scroll", VerticalScroll).focus()
        else:
            self.query_one("#list", DataTable).focus()
        self._render_filter_bar()
        self._render_status()

    def _toggle_pane(self) -> None:
        self._focus_pane("detail" if self._active_pane() == "list" else "list")

    def _resize_panes(self, delta: int) -> None:
        self._list_fr = max(3, min(17, self._list_fr + delta))
        self._detail_fr = max(3, min(17, self._detail_fr - delta))
        self.query_one("#list-pane").styles.height = f"{self._list_fr}fr"
        self.query_one("#detail").styles.height = f"{self._detail_fr}fr"

    def _scroll_detail_home(self) -> None:
        self.query_one("#detail-scroll", VerticalScroll).scroll_home(animate=False, immediate=True)

    def _reset_detail_scroll(self) -> None:
        self.call_after_refresh(self._scroll_detail_home)

    def _clear_detail(self, message: str = "[dim]select row[/]") -> None:
        self._selected = None
        self._pending_detail_key = None
        self._pending_sub_key = None
        self.query_one("#detail-bar", Static).update("")
        self.query_one("#state-flow-bar", Static).update("")
        self.query_one("#state-flow-bar", Static).display = False
        rep_line = self._maybe("#reporter-line")
        if rep_line is not None:
            rep_line.update("")
            rep_line.display = False
        self._show_side(None)
        self.query_one("#detail-tabs", Static).update("")
        self.query_one("#detail-content", Static).update(Text.from_markup(message))
        self._reset_detail_scroll()
        self._render_filter_bar()
        self._render_status()

    def _set_pending_detail(self, detail_key: str, message: str = "[dim]loading details...[/]") -> None:
        self._selected = None
        self._pending_detail_key = detail_key
        self._pending_sub_key = None
        self.query_one("#detail-bar", Static).update("")
        self.query_one("#detail-tabs", Static).update("")
        self._show_side(None)
        self.query_one("#detail-content", Static).update(Text.from_markup(message))
        self._reset_detail_scroll()
        self._render_filter_bar()
        self._render_status()

    def _set_filter_error(self, error: str) -> None:
        self._filter_error = error
        self._render_filter_bar()

    def _get_default_filters(self, entity: str) -> list[str]:
        try:
            return config.get_default_where(entity)
        except Exception:
            return []

    def _load_filter_history(self) -> None:
        try:
            from fsv.config import filters_cache_path
            path = filters_cache_path("tui_history")
            if path.exists():
                self._filter_history = json.loads(path.read_text()).get("history", [])[:100]
        except Exception:
            pass

    def _save_filter_history(self) -> None:
        try:
            from fsv.config import filters_cache_path, ensure_dirs
            ensure_dirs()
            path = filters_cache_path("tui_history")
            path.write_text(json.dumps({"history": self._filter_history[:100]}))
        except Exception:
            pass

    def _history_navigate(self, direction: int) -> None:
        """direction: +1 = older (up), -1 = newer (down)."""
        if not self._filter_history and direction > 0:
            return
        inp = self.query_one("#filter-input", Input)
        if self._history_idx == -1:
            self._history_saved = inp.value
        new_idx = self._history_idx + direction
        if new_idx >= len(self._filter_history):
            return
        if new_idx < -1:
            return
        self._history_idx = new_idx
        self._history_nav_lock += 1
        self._suggestion_idx = 0
        self._tab_base = None
        if self._history_idx == -1:
            inp.value = self._history_saved
        else:
            inp.value = self._filter_history[self._history_idx]
        inp.cursor_position = len(inp.value)
        self._update_suggestion_bar(inp.value)

    _PSEUDO_DATE_FIELDS = ("created_at", "updated_at", "due_by")

    def _extract_item_values(self, fname: str, val_prefix: str) -> list[str]:
        seen: set[str] = set()
        result: list[str] = []
        val_lower = val_prefix.lower()
        for item in self._items:
            raw = item.get(fname)
            if raw is None and fname == "responder":
                raw = item.get("agent")
            val: str | None = None
            if isinstance(raw, dict):
                val = raw.get("name")
            elif isinstance(raw, str) and raw:
                val = raw
            if val is None:
                cf = item.get("custom_fields") or {}
                raw_cf = cf.get(fname)
                if isinstance(raw_cf, str) and raw_cf:
                    val = raw_cf
            if val and val.lower().startswith(val_lower) and val not in seen:
                seen.add(val)
                result.append(val)
                if len(result) >= 8:
                    break
        return result

    def _get_suggestions(self, value: str) -> tuple[list[str], str | None]:
        """Returns (completable_suggestions, hint_or_None)."""
        sch = self._schemas.get(self.entity, {"fields": []})
        fields_list = sch.get("fields") or []

        def _all_fields() -> list[str]:
            seen: set[str] = set()
            out: list[str] = []
            for name in self._PSEUDO_DATE_FIELDS:
                if name not in seen:
                    seen.add(name)
                    out.append(name)
            for f in fields_list:
                name = f.get("name") or ""
                if name and name not in seen:
                    seen.add(name)
                    out.append(name)
            return out

        if not value:
            return _all_fields(), None
        # If inside a quoted value, suppress suggestions (can't split safely)
        try:
            tokens = shlex.split(value)
            in_quote = False
        except ValueError:
            in_quote = True
        if in_quote:
            return [], None
        if value.endswith(" "):
            return _all_fields(), None
        current = value.rsplit(" ", 1)[-1]
        if not current:
            return [], None
        current_lower = current.lower()
        split = _split_filter_token(current)
        if split is None:
            matches: list[str] = []
            for name in self._PSEUDO_DATE_FIELDS:
                if name.startswith(current_lower) and name not in matches:
                    matches.append(name)
            for f in fields_list:
                name = f.get("name") or ""
                label = f.get("label") or ""
                if (name.lower().startswith(current_lower) or label.lower().startswith(current_lower)) and name not in matches:
                    matches.append(name)
            return matches, None
        field_part, _op, val_prefix = split
        field_lower = field_part.lower()
        val_lower = val_prefix.lower()
        if field_lower in self._PSEUDO_DATE_FIELDS:
            return [], "YYYY-MM-DD  (operators: = != > >= < <=)"
        matched = None
        for f in fields_list:
            if (f.get("name") or "").lower() == field_lower:
                matched = f
                break
        if not matched:
            for f in fields_list:
                if (f.get("name") or "").lower().startswith(field_lower):
                    matched = f
                    break
        if not matched:
            return [], None
        ftype = (matched.get("field_type") or "").lower()
        fname = (matched.get("name") or "").lower()
        choices = matched.get("choices") or []
        if not choices:
            if fname in ("requester",) or "requester" in ftype:
                vals = self._extract_item_values("requester", val_prefix)
                return (vals, None) if vals else ([], "name or email")
            if fname in ("agent", "responder") or "agent" in ftype:
                vals = self._extract_item_values("responder", val_prefix)
                return (vals, None) if vals else ([], "agent name")
            if fname == "group" or "group" in ftype:
                vals = self._extract_item_values("group", val_prefix)
                if not vals:
                    try:
                        from fsv.completion import _cached_groups
                        vals = [g["name"] for g in _cached_groups() if str(g.get("name", "")).lower().startswith(val_lower)]
                    except Exception:
                        pass
                return (vals, None) if vals else ([], "group name")
            if fname == "department" or "department" in ftype:
                vals = self._extract_item_values("department", val_prefix)
                return (vals, None) if vals else ([], "department name")
            if "lookup" in ftype:
                vals = self._extract_item_values(fname, val_prefix)
                return (vals, None) if vals else ([], "name")
            if "checkbox" in ftype:
                return ["true", "false"], None
            if "number" in ftype:
                return [], "number"
            if "date" in ftype:
                return [], "YYYY-MM-DD"
            return [], "text"
        suggestions: list[str] = []
        for c in choices:
            v = c.get("value") or c.get("name") or ""
            if v and v.lower().startswith(val_lower):
                suggestions.append(v)
        return suggestions, None

    def _update_suggestion_bar(self, value: str) -> None:
        suggestions, hint = self._get_suggestions(value)
        self._suggestions = suggestions
        bar = self.query_one("#suggestion-bar", Static)
        if not suggestions:
            if hint:
                bar.update(Text.from_markup(f"[dim]{escape(hint)}[/]"))
                bar.display = True
            else:
                bar.display = False
            self._maybe_schedule_network_lookup(value)
            return
        _WIN = 8
        visible = suggestions[:_WIN]
        overflow = len(suggestions) - _WIN
        parts = [f"[bold cyan]{escape(visible[0])}[/]"]
        for s in visible[1:]:
            parts.append(f"[dim]{escape(s)}[/]")
        suffix = f"  [dim](+{overflow} more)  tab→[/]" if overflow > 0 else "  [dim]tab→[/]"
        bar.update(Text.from_markup("  ".join(parts) + suffix))
        bar.display = True
        self._maybe_schedule_network_lookup(value)

    def _show_suggestions_cycling(self, suggestions: list[str], active_idx: int) -> None:
        bar = self.query_one("#suggestion-bar", Static)
        _WIN = 8
        total = len(suggestions)
        if total <= _WIN:
            lo, hi = 0, total
        else:
            lo = max(0, min(active_idx - _WIN // 2, total - _WIN))
            hi = lo + _WIN
        parts = []
        if lo > 0:
            parts.append(f"[dim]…[/]")
        for i in range(lo, hi):
            s = suggestions[i]
            if i == active_idx:
                parts.append(f"[bold cyan]{escape(s)}[/]")
            else:
                parts.append(f"[dim]{escape(s)}[/]")
        if hi < total:
            parts.append(f"[dim]…[/]")
        bar.update(Text.from_markup("  ".join(parts) + "  [dim]tab→[/]"))
        bar.display = True

    def _complete_suggestion(self) -> None:
        inp = self.query_one("#filter-input", Input)
        value = inp.value

        if self._tab_base is None:
            self._tab_base = value
            self._suggestion_idx = 0

        suggestions = self._suggestions if self._suggestions else self._get_suggestions(self._tab_base)[0]
        if not suggestions:
            self._tab_base = None
            return

        last_space = self._tab_base.rfind(" ")
        prefix = self._tab_base[:last_space + 1] if last_space >= 0 else ""
        current = self._tab_base[last_space + 1:] if last_space >= 0 else self._tab_base
        split = _split_filter_token(current)

        idx = self._suggestion_idx % len(suggestions)
        suggestion = suggestions[idx]

        if split is None:
            if len(suggestions) == 1:
                new_value = prefix + suggestion + "="
                self._tab_base = None
                self._suggestion_idx = 0
                if new_value != value:
                    self._tab_lock += 1
                    inp.value = new_value
                    inp.cursor_position = len(new_value)
                self._update_suggestion_bar(new_value)
                return
            else:
                new_value = prefix + suggestion
                self._suggestion_idx = (idx + 1) % len(suggestions)
        else:
            field_part, op, _ = split
            val = f'"{suggestion}"' if " " in suggestion else suggestion
            new_value = prefix + field_part + op + val
            self._suggestion_idx = (idx + 1) % len(suggestions)

        if new_value != value:
            self._tab_lock += 1
            inp.value = new_value
            inp.cursor_position = len(new_value)

        self._show_suggestions_cycling(suggestions, idx)

    def _maybe_schedule_network_lookup(self, value: str) -> None:
        try:
            from fsv.completion import _completion_network
            if not _completion_network():
                return
        except Exception:
            return
        if self._tab_base is not None:
            return
        if value.endswith(" "):
            return
        current = value.rsplit(" ", 1)[-1]
        split = _split_filter_token(current)
        if split is None:
            return
        field_part, _, val_prefix = split
        if len(val_prefix.strip()) < 2:
            if self._completion_lookup_timer is not None:
                self._completion_lookup_timer.stop()
                self._completion_lookup_timer = None
            return
        field_lower = field_part.lower()
        sch = self._schemas.get(self.entity, {"fields": []})
        matched = None
        for f in (sch.get("fields") or []):
            if (f.get("name") or "").lower() == field_lower:
                matched = f
                break
        if not matched:
            return
        fname = (matched.get("name") or "").lower()
        ftype = (matched.get("field_type") or "").lower()
        if fname in ("requester",) or "requester" in ftype:
            kind, link = "requesters", None
        elif fname in ("agent", "responder") or "agent" in ftype:
            kind, link = "agents", None
        elif "lookup" in ftype:
            link = (matched.get("lookup_config") or {}).get("link")
            if not link:
                return
            kind = "lookup"
        else:
            return
        lookup_key = f"{kind}:{val_prefix}"
        if self._completion_lookup_key == lookup_key:
            return
        if self._completion_lookup_timer is not None:
            self._completion_lookup_timer.stop()
        self._completion_lookup_key = lookup_key
        self._completion_lookup_timer = self.set_timer(
            0.3, lambda: self._run_network_lookup(lookup_key, val_prefix, kind, link)
        )

    @work(thread=True, exclusive=True, group="completion")
    def _run_network_lookup(self, lookup_key: str, val_prefix: str, kind: str, link: str | None) -> None:
        if lookup_key != self._completion_lookup_key:
            return
        try:
            if kind == "lookup" and link:
                c = get_client()
                rows = c.lookup_choices(link, val_prefix)
                r = val_prefix.casefold()
                rows.sort(key=lambda x: 0 if str(x.get("email") or "").casefold().startswith(r) or str(x.get("name") or "").casefold().startswith(r) else 1)
                suggestions: list[str] = []
                for row in rows[:8]:
                    name_ = str(row.get("name") or "")
                    email_ = str(row.get("email") or "")
                    val = email_ if email_ and email_.casefold().startswith(r) else name_
                    if val and val not in suggestions:
                        suggestions.append(val)
            else:
                from fsv.completion import _remote_user_values
                results = _remote_user_values(kind, val_prefix)
                suggestions = [v for v, _ in results[:8]]
        except Exception:
            return
        self.call_from_thread(self._apply_network_suggestions, lookup_key, suggestions)

    def _apply_network_suggestions(self, lookup_key: str, suggestions: list[str]) -> None:
        if lookup_key != self._completion_lookup_key:
            return
        if self._tab_base is not None:
            return
        inp = self.query_one("#filter-input", Input)
        if not inp.display or not inp.has_focus:
            return
        if not suggestions:
            return
        self._suggestions = suggestions
        parts = [f"[bold cyan]{escape(suggestions[0])}[/]"]
        for s in suggestions[1:]:
            parts.append(f"[dim]{escape(s)}[/]")
        bar = self.query_one("#suggestion-bar", Static)
        bar.update(Text.from_markup("  ".join(parts) + "  [dim]tab→[/]"))
        bar.display = True

    def _open_filter_input(self) -> None:
        inp = self.query_one("#filter-input", Input)
        current_val = " ".join(self._active_filters)
        inp.value = current_val
        inp.display = True
        inp.focus()
        end = len(current_val)
        self.call_after_refresh(lambda: setattr(inp, "cursor_position", end))
        if self._filter_history and self._filter_history[0] == current_val.strip():
            self._history_idx = 0
        else:
            self._history_idx = -1
        self._history_saved = current_val
        self._suggestion_idx = 0
        self._tab_base = None
        self._update_suggestion_bar(inp.value)

    def _apply_filter(self, raw: str) -> None:
        inp = self.query_one("#filter-input", Input)
        inp.display = False
        self.query_one("#suggestion-bar", Static).display = False
        self._suggestions = []
        self._suggestion_idx = 0
        self._tab_base = None
        if self._completion_lookup_timer is not None:
            self._completion_lookup_timer.stop()
            self._completion_lookup_timer = None
        self._completion_lookup_key = ""
        self._history_idx = -1
        self.query_one("#list", DataTable).focus()
        if raw.strip():
            try:
                self._active_filters = shlex.split(raw)
            except ValueError:
                self._active_filters = raw.split()
            entry = raw.strip()
            self._filter_history = [entry] + [h for h in self._filter_history if h != entry]
            self._filter_history = self._filter_history[:100]
            self._save_filter_history()
        else:
            self._active_filters = []
        self._reload_list("[dim]filtering...[/]")

    def _cancel_filter_input(self) -> None:
        inp = self.query_one("#filter-input", Input)
        inp.display = False
        self.query_one("#suggestion-bar", Static).display = False
        self._suggestions = []
        self._suggestion_idx = 0
        self._tab_base = None
        if self._completion_lookup_timer is not None:
            self._completion_lookup_timer.stop()
            self._completion_lookup_timer = None
        self._completion_lookup_key = ""
        self._history_idx = -1
        self.query_one("#list", DataTable).focus()
        self._render_filter_bar()

    def _open_search_input(self) -> None:
        inp = self.query_one("#search-input", Input)
        inp.value = self._search_term if self._search_mode else ""
        inp.display = True
        inp.focus()

    def _apply_search(self, raw: str) -> None:
        inp = self.query_one("#search-input", Input)
        inp.display = False
        term = raw.strip()
        if not term:
            if self._search_mode:
                self._exit_search_mode()
            else:
                self.query_one("#list", DataTable).focus()
            return
        self._search_seq += 1
        self._search_cache.clear()
        self._search_mode = True
        self._search_term = term
        self._search_entity = None
        self._search_sort = "relevance"
        self._search_page = 1
        self._search_has_more = False
        self._active_filters = []
        self._filter_error = None
        self._items = []
        self._total = 0
        self._page = 1
        self._has_more = False
        self._detail_cache.clear()
        self._sub_cache.clear()
        self.query_one("#list", DataTable).clear(columns=True)
        self._focus_pane("list")
        self._render_header()
        self._clear_detail("[dim]searching...[/]")
        self._load_search()

    def _cancel_search_input(self) -> None:
        inp = self.query_one("#search-input", Input)
        inp.display = False
        self.query_one("#list", DataTable).focus()

    def _exit_search_mode(self) -> None:
        self._search_seq += 1
        self._search_cache.clear()
        self._search_mode = False
        self._search_term = ""
        self._search_totals = {}
        self._search_entity = None
        self._search_sort = "relevance"
        self._search_page = 1
        self._search_has_more = False
        self._active_filters = self._get_default_filters(self.entity)
        self._filter_error = None
        self._reload_list("[dim]loading rows...[/]")

    def _step_search_entity(self, delta: int) -> None:
        tabs: list[str | None] = [None, "tickets", "problems", "changes"]
        try:
            idx = tabs.index(self._search_entity)
        except ValueError:
            idx = 0
        new_idx = (idx + delta) % len(tabs)
        new_entity = tabs[new_idx]
        if new_entity == self._search_entity:
            return
        self._save_search_cache()
        if new_entity in self._search_cache:
            self._restore_search_cache(new_entity)
            return
        if new_entity is None:
            self._search_seq += 1
            self._search_entity = None
            self._search_page = 1
            self._search_has_more = False
            self._items = []
            self._total = 0
            self._page = 1
            self._has_more = False
            self._detail_cache.clear()
            self._sub_cache.clear()
            self.query_one("#list", DataTable).clear(columns=True)
            self._clear_detail("[dim]searching...[/]")
            self._render_header()
            self._load_search()
        else:
            self._search_narrow_entity(new_entity)

    def _search_narrow_entity(self, entity: str) -> None:
        if not self._search_mode or not self._search_term:
            return
        self._search_seq += 1
        self._search_entity = entity
        self._search_page = 1
        self._search_has_more = False
        self._items = []
        self._total = 0
        self._page = 1
        self._has_more = False
        self._detail_cache.clear()
        self._sub_cache.clear()
        self.query_one("#list", DataTable).clear(columns=True)
        self._clear_detail("[dim]searching...[/]")
        self._render_header()
        self._load_search()

    def _save_search_cache(self) -> None:
        if self._items:
            self._search_cache[self._search_entity] = (
                list(self._items),
                dict(self._search_totals),
                self._search_page,
                self._search_has_more,
            )

    def _restore_search_cache(self, entity: str | None) -> None:
        items, totals, page, has_more = self._search_cache[entity]
        self._search_seq += 1
        self._search_entity = entity
        self._search_page = page
        self._search_has_more = has_more
        self._items = list(items)
        self._search_totals = totals
        if entity and entity in totals:
            self._total = totals[entity]
        else:
            self._total = sum(totals.values())
        table = self.query_one("#list", DataTable)
        table.clear(columns=True)
        table.add_columns("", "ID", "STATUS", "SUBJECT", "AGENT", "GROUP")
        for idx, item in enumerate(self._items):
            res: Resource = item.get("_resource", TICKETS)
            did = format_id(item, res)
            status = item.get("status") or ""
            sel = f"{idx + 1:03d}"
            subject = (item.get("subject") or "")[:80]
            agent = item.get("_agent") or ""
            group = item.get("_group") or ""
            table.add_row(sel, did, status, subject, agent, group, key=did)
        if self._items:
            table.move_cursor(row=0)
            self._clear_detail("[dim]loading details...[/]")
        else:
            self._clear_detail("[dim]no results[/]")
        self._render_header()
        self._render_filter_bar()
        self._render_status()

    @work(thread=True, exclusive=True, group="list")
    def _load_search(self, append: bool = False) -> None:
        seq = self._search_seq
        c = get_client()
        for res in ALL_RESOURCES:
            if res.name not in self._schemas:
                self._schemas[res.name] = schema_mod.load(res, c)
        items, totals = service.search_items(
            self._search_term,
            entity=self._search_entity,
            sort=self._search_sort,
            page=self._search_page,
            client=c,
        )
        self.call_from_thread(self._populate_search, items, totals, append, seq)

    def _populate_search(self, items: list[dict], totals: dict[str, int], append: bool = False, seq: int = -1) -> None:
        if seq != self._search_seq:
            return
        self._search_totals = totals
        self._loading_more = False
        entity = self._search_entity
        if entity and entity in totals:
            self._total = totals[entity]
            self._search_has_more = len(items) == 30
        else:
            self._total = sum(totals.values())
            self._search_has_more = False

        table = self.query_one("#list", DataTable)
        if not append:
            self._items = items
            table.clear(columns=True)
            table.add_columns("", "ID", "STATUS", "SUBJECT", "AGENT", "GROUP")
            offset = 0
        else:
            offset = len(self._items)
            self._items.extend(items)

        existing_keys = set(table.rows)
        for idx, item in enumerate(items):
            row_num = offset + idx
            res: Resource = item.get("_resource", TICKETS)
            did = format_id(item, res)
            if RowKey(did) in existing_keys:
                continue
            status = item.get("status") or ""
            sel = f"{row_num + 1:03d}"
            subject = (item.get("subject") or "")[:80]
            agent = item.get("_agent") or ""
            group = item.get("_group") or ""
            table.add_row(sel, did, status, subject, agent, group, key=did)
        if not append and items:
            self._clear_detail("[dim]loading details...[/]")
            table.move_cursor(row=0)
        elif not append and not items:
            self._clear_detail("[dim]no results[/]")
        self._render_header()
        self._render_filter_bar()
        self._render_status()

    def _search_load_more(self) -> None:
        if not self._search_has_more or self._loading_more:
            return
        self._loading_more = True
        self._search_page += 1
        self._render_filter_bar()
        self._render_status()
        self._load_search(append=True)

    def _cycle_search_sort(self) -> None:
        self._search_seq += 1
        self._search_cache.clear()
        order = ["relevance", "created", "modified"]
        idx = order.index(self._search_sort) if self._search_sort in order else 0
        self._search_sort = order[(idx + 1) % len(order)]
        self._search_page = 1
        self._search_has_more = False
        self._items = []
        self._total = 0
        self._detail_cache.clear()
        self._sub_cache.clear()
        self.query_one("#list", DataTable).clear(columns=True)
        self._clear_detail("[dim]searching...[/]")
        self._render_header()
        self._load_search()

    def _load_more(self) -> None:
        if self._loading_more or not self._has_more:
            return
        self._loading_more = True
        self._render_filter_bar()
        self._render_status()
        self._load_list(append=True)

    def _reload_list(self, message: str = "[dim]reloading...[/]") -> None:
        self._list_epoch += 1
        self._list_total = None
        self._items = []
        self._total = 0
        self._page = 1
        self._has_more = False
        self._loading_more = False
        self._g_pending = False
        self._filter_error = None
        self._detail_cache.clear()
        self._sub_cache.clear()
        self.query_one("#list", DataTable).clear(columns=True)
        self._focus_pane("list")
        self._render_header()
        self._clear_detail(message)
        self._load_list()

    # ── Rendering ───────────────────────────────────────────

    def _render_header(self) -> None:
        parts = []
        if self._search_mode:
            parts.append(f"[bold cyan]fsv[/] :: [yellow bold]search[/] [yellow]{escape(self._search_term)}[/]  ")
            all_label = "ALL"
            if self._search_entity is None:
                parts.append(f"[bold reverse] {all_label} [/] ")
            else:
                parts.append(f"{all_label}  ")
            for tab in ENTITY_TABS:
                label = tab.upper()
                cnt = self._search_totals.get(tab)
                cnt_str = f"({cnt})" if cnt is not None else ""
                if tab == self._search_entity:
                    parts.append(f"[bold reverse] {label}{cnt_str} [/] ")
                else:
                    parts.append(f"{label}{cnt_str}  ")
            rows = len(self._items)
            total = self._total
            count = f"{rows}/{total}" if total > rows else str(rows)
            sort_label = {"relevance": "relevance", "created": "date created", "modified": "last modified"}.get(self._search_sort, self._search_sort)
            parts.append(f"  [dim]{count} shown  sort:{sort_label}  Esc exit[/]")
        else:
            parts.append("[bold cyan]fsv[/] :: freshservice  ")
            for tab in ENTITY_TABS:
                label = tab.upper()
                if tab == self.entity:
                    parts.append(f"[bold reverse] {label} [/] ")
                else:
                    parts.append(f"{label}  ")
            parts.append("  " + self._view_header())
            parts.append("  [dim]v views  ? help[/]")
        self.query_one("#header", Static).update(Text.from_markup("".join(parts)))

    def _view_header(self) -> str:
        """``<view> · 1-100 of N · Sort by: Created Date`` like the web."""
        res = _resource_for_entity(self.entity) or TICKETS
        view = None if self._active_filters else self._view.get(res.name)
        name = view.name if view is not None else ("Filtered" if self._active_filters else f"All {res.name}")
        rows = len(self._items)
        rng = f"1-{rows}" if rows else "0"
        total = self._list_total if self._list_total is not None else (self._total or rows)
        of = f" of {total}" if self._list_total is not None else f" of {rows}{'+' if self._has_more else ''}"
        sort = V.sort_label(self._list_sort, self._schemas.get(res.name))
        return f"[bold]{escape(name)}[/][dim] · {rng}{of} · Sort by: {escape(sort)}[/]"

    def _pick_view(self) -> None:
        if self._search_mode:
            return
        res = _resource_for_entity(self.entity) or TICKETS
        views = self._views.get(res.name) or []
        if not views:
            self.notify("No saved views on this account for " + res.name, severity="warning")
            return

        def picked(view: Any) -> None:
            if view is None:
                return
            self._view[res.name] = view
            self._active_filters = []
            self._reload_list(f"[dim]loading {escape(view.name)}...[/]")

        self.push_screen(ViewPickerScreen(views, self._view.get(res.name), title=f"{res.name.capitalize()} views"), picked)

    def _open_form(self, mode: str) -> None:
        """``N`` new change / ``e`` edit the selected change in the full-screen
        form (``fsv.formui``), when that module is installed."""
        if self.read_only:
            self.notify("Read-only mode: write actions are disabled", severity="warning")
            return
        if mode == "new":
            self.push_screen(ChoiceScreen([("change", "Change"), ("incident", "Incident"),
                                          ("sr", "Service Request")], "Create"), self._create_choice)
            return
        item = self._selected
        if mode == "edit" and (not item or item.get("_resource", TICKETS).name != "changes"):
            self.notify("Select a change to edit", severity="warning")
            return
        try:
            import fsv.formui as formui  # noqa: PLC0415 - optional, built separately
            screen_cls = formui.FormScreen
        except (ImportError, AttributeError):
            self.notify("form UI not available", severity="warning")
            return
        self.notify("opening the form…", timeout=2)
        self._build_form(mode, item, screen_cls)

    def _create_choice(self, kind: str | None) -> None:
        if kind is None or self.read_only:
            return
        if kind == "sr":
            self._load_catalog()
        else:
            self._launch_create(kind)

    def _launch_create(self, kind: str, item: dict | None = None) -> None:
        if self.read_only:
            return
        try:
            from fsv.formui import FormScreen
            self._build_form("new" if kind == "change" else kind, item, FormScreen)
        except (ImportError, AttributeError):
            self.notify("form UI not available", severity="warning")

    @work(thread=True, exclusive=True, group="catalog")
    def _load_catalog(self) -> None:
        from fsv import catalog
        try:
            client = get_client()
            items = catalog.list_items(client)
            categories = catalog.list_categories(client)
        except Exception as exc:
            self.call_from_thread(self._show_failure, f"Catalog unavailable: {exc}")
            return
        self.call_from_thread(self._pick_catalog, items, categories)

    def _pick_catalog(self, items: list[dict], categories: list[dict]) -> None:
        from fsv import catalog

        def category_picked(category) -> None:
            if category is None or self.read_only:
                return
            ids = None if category == "all" else [c["id"] for c in catalog.resolve_category(categories, str(category))]
            rows = catalog.filter_items(items, category_ids=ids, categories=categories)
            names = {c["id"]: c.get("name", "") for c in categories}
            choices = [(r, f"#{r.get('display_id')} {r.get('name', '')} · {names.get(r.get('category_id'), '')}")
                       for r in rows if r.get("display_id") is not None]
            self.push_screen(ChoiceScreen(choices, "Service catalog — search items"),
                             lambda row: self._launch_create("sr", row) if row is not None else None)

        self.push_screen(ChoiceScreen([("all", "All categories")] +
                                      [(c["id"], c.get("name", str(c["id"]))) for c in categories],
                                      "Service catalog — category"), category_picked)

    @work(thread=True, exclusive=True, group="form")
    def _build_form(self, mode: str, item: dict | None, screen_cls: Any) -> None:
        try:
            if mode in ("incident", "sr"):
                from fsv.formui.session import sr_session, ticket_session
                session = (ticket_session(get_client()) if mode == "incident" else
                           sr_session(get_client(), item_ref=(item or {})["display_id"]))
            else:
                session = _form_session(mode, item, self._page_for(item))
        except Exception as exc:  # noqa: BLE001 - shown as a toast
            self.call_from_thread(self._show_failure, f"Form unavailable: {exc}")
            return
        self.call_from_thread(self._push_form, screen_cls, session, mode, item)

    def _push_form(self, screen_cls: Any, session: Any, mode: str, item: dict | None) -> None:
        def done(result: Any) -> None:
            action = getattr(result, "action", None)
            if action == "save_draft":
                self.notify(f"Draft saved: {getattr(result, 'saved_path', '')}")
            elif action == "submit" and getattr(result, "plan", None) is not None:
                self._send_form(session, result, mode, item)

        try:
            self.push_screen(screen_cls(session), done)
        except Exception as exc:  # noqa: BLE001
            self.notify(f"form UI not available: {actions.clean_error(exc)}", severity="warning")

    @work(thread=True, group="write")
    def _send_form(self, session: Any, result: Any, mode: str, item: dict | None) -> None:
        if self.read_only:
            return
        try:
            out = session.send(result.plan)
        except Exception as exc:  # noqa: BLE001 - never truncate a partial/ambiguous outcome
            self.call_from_thread(self._show_failure, actions.failure_detail(exc))
            return
        label, done = actions.form_result(mode, out)
        if mode == "edit" and item is not None:
            self.call_from_thread(self._after_write, item.get("_resource", CHANGES), item, "Change updated")
        else:
            self.call_from_thread(self.notify, f"{label} created {done}".strip())
            self.call_from_thread(self._reload_list)

    def _show_failure(self, message: str) -> None:
        self._write_failure = message
        self.push_screen(FailureScreen(message))

    def _render_filter_bar(self) -> None:
        sel = format_id(self._selected, self._selected["_resource"]) if self._selected else "-"
        pane = self._active_pane()
        rows = len(self._items)
        total = self._total
        count = f"{rows}/{total}" if total > rows else str(rows)
        loading = "  [yellow]loading more…[/]" if self._loading_more else ""
        if self._search_mode:
            filter_part = f"[yellow]search: {escape(self._search_term)}[/]  [dim]h/l narrow  s sort  Esc exit  f new search[/]"
        elif self._filter_error:
            filter_part = f"[red]filter error: {escape(self._filter_error)}[/]"
        elif self._active_filters:
            filter_part = f"[yellow]filter: {escape(' '.join(self._active_filters))}[/]"
        else:
            filter_part = "[dim]/ to filter[/]"
        self.query_one("#filter-bar", Static).update(
            Text.from_markup(f"{filter_part}  [dim]focus={pane}  sel={sel}  {count} rows[/]{loading}")
        )

    def _trigger_state_flow(self, item: dict, resource: Resource) -> None:
        """Lifecycle bar + reporter line from the loaded page (no extra calls;
        accounts without a lifecycle just show none)."""
        bar = self.query_one("#state-flow-bar", Static)
        page = self._page_for(item)
        markup = pages.lifecycle_markup(page.lifecycle, context=3) if page is not None else ""
        bar.update(Text.from_markup(markup) if markup else "")
        bar.display = bool(markup)
        rep_line = self._maybe("#reporter-line")
        if rep_line is not None:
            text = pages.reporter_markup(page) if page is not None else ""
            rep_line.update(Text.from_markup(text) if text else "")
            rep_line.display = bool(text)

    def _render_detail_bar(self, item: dict, resource: Resource) -> None:
        bar = self.query_one("#detail-bar", Static)
        page = self._page_for(item)
        did = format_id(item, resource)
        subject = item.get("subject") or ""
        text = Text()
        if page is not None:
            text.append(f"#{did} ", style="bold cyan")
            text.append(f"{subject[:90]}  ", style="bold")
            text.append(page.badge, style="bold green")
            due = page.due_text()
            stats = page.record.get("stats") or {}
            if due and not (resource.name == "tickets" and (stats.get("resolved_at") or stats.get("closed_at"))):
                text.append(f", due {due}", style="dim")
            bar.update(text)
            return
        schema = self._schemas.get(resource.name, {"fields": []})
        status = service.resolve_status(item, resource, schema)
        pri = service.resolve_priority(item)
        if resource.name != "changes":
            text.append(f"[{status}]", style="bold green")
            text.append("  ")
        text.append(f"{did} {subject[:80]}  ")
        text.append(str(pri), style="dim")
        bar.update(text)

    def _render_detail_tabs(self, resource: Resource) -> None:
        page = self._page_for(self._selected)
        if page is not None and page.resource.name == resource.name:
            active = self._current_tab_name(resource)
            self.query_one("#detail-tabs", Static).update(
                Text.from_markup(pages.tab_bar(page, active, numbered=True)))
            return
        tabs = DETAIL_TAB_LABELS.get(resource.name, ["details"])
        parts = []
        for idx, tab in enumerate(tabs):
            num = idx + 1
            if idx == self.detail_tab_idx:
                parts.append(f"[bold reverse] {num}:{tab} [/]")
            else:
                parts.append(f"[dim]{num}:{tab}[/]")
        self.query_one("#detail-tabs", Static).update(
            Text.from_markup("  ".join(parts))
        )

    def _render_detail_content(self, item: dict, resource: Resource) -> None:
        tab_name = self._current_tab_name(resource)

        if tab_name == "details":
            self._pending_sub_key = None
            page = self._page_for(item)
            if page is not None:
                content = "\n".join(pages.main_lines(page) + [""] + pages.notes_lines(page))
                self._show_side(pages.side_lines(page))
            else:
                content = self._format_details(item, resource)
                self._show_side(None)
            self.query_one("#detail-content", Static).update(Text.from_markup(content))
            self._reset_detail_scroll()
        else:
            self._show_side(None)
            sub_key = f"{self._item_key(item, resource)}:{tab_name}"
            self._pending_sub_key = sub_key
            self.query_one("#detail-content", Static).update(
                Text.from_markup(f"[dim]loading {tab_name}...[/]")
            )
            self._reset_detail_scroll()
            self._load_sub_resource(tab_name, sub_key)

    def _render_sub_content(self, tab_name: str, data: Any, resource: Resource, sub_key: str) -> None:
        if self._pending_sub_key != sub_key:
            return
        current_key = self._selected_key()
        if current_key is None or f"{current_key}:{self._current_tab_name(resource)}" != sub_key:
            return
        if isinstance(data, V.TabData):
            page = self._page_for(self._selected)
            content = "\n".join([""] + pages.tab_lines(data, page.tz if page else self._tz,
                                                         page.schema if page else None) + [""])
        elif tab_name in ("notes", "conversations"):
            content = self._format_notes(data)
        elif tab_name == "tasks":
            content = self._format_tasks(data)
        elif tab_name == "approvals":
            content = self._format_approvals(data)
        elif tab_name == "assets":
            content = self._format_assets(data)
        elif tab_name == "associations":
            content = self._format_associations(data, resource)
        elif tab_name == "activity":
            content = self._format_activity(data)
        elif tab_name == "resolution":
            content = self._format_resolution(data)
        else:
            content = "[dim]no data[/]"
        self.query_one("#detail-content", Static).update(Text.from_markup(content))
        self._reset_detail_scroll()

    def _render_status(self) -> None:
        if self._search_mode:
            label = f"SEARCH: {self._search_term}"
            if self._search_entity:
                label += f" > {self._search_entity.upper()}"
        else:
            label = self.entity.upper()
        sel = ""
        if self._selected:
            did = format_id(self._selected, self._selected["_resource"])
            tab_name = self._current_tab_name(self._selected["_resource"])
            sel = f" > {did} > {tab_name}"
        rows = len(self._items)
        total = self._total
        count = f"{rows}/{total}" if total > rows else str(rows)
        pane = self._active_pane()
        bar = self.query_one("#status-bar", Static)
        text = Text(f"{label}{sel}    ")
        suffix = "loading more…" if self._loading_more else f"{count} rows"
        text.append(f"pane={pane} · {suffix}", style="dim")
        bar.update(text)

    def _render_key_hints(self) -> None:
        write = "" if self.read_only else "[bold]s[/] status  [bold]n[/] note  [bold]N[/]/[bold]e[/] create/edit  [bold]a[/] actions  [bold]![/] failure  "
        rkey = "[bold]r[/] reload  " if self.read_only else "[bold]r[/] reload/reply  "
        hints = (
            "[bold]Tab[/] pane  "
            "[bold]j/k[/] move or scroll  "
            "[bold]J/K[/] ×5  "
            "[bold]G/gg[/] ↕  "
            "[bold]h/l[/] entity/tab  "
            "[bold]gw/gt/gp/gc[/] entity  "
            "[bold]1-9[/] detail tab  "
            "[bold]v[/] views  "
            "[bold]\\[][/bold] resize  "
            "[bold]f[/] search  "
            "[bold]/[/] filter  "
            "[bold]o[/] browser  "
            f"{write}{rkey}"
            "[bold]y[/] yank  "
            "[bold]q[/]/[bold]Ctrl+C[/] quit  "
            "[bold]?[/] help"
        )
        self.query_one("#key-hints", Static).update(Text.from_markup(hints))

    # ── Formatters ──────────────────────────────────────────

    def _format_details(self, item: dict, resource: Resource) -> str:
        schema = self._schemas.get(resource.name, {"fields": []})
        lines: list[str] = []

        lines.append("")
        lines.append("[dim]── CORE ──[/]")
        lines.append(f"  [cyan]Module[/]       {_markup(resource.name.upper())}")
        lines.append(f"  [cyan]ID[/]           {_markup(format_id(item, resource))}")
        lines.append(f"  [cyan]Subject[/]      {_markup(item.get('subject', ''))}")

        desc = strip_html(item.get("description"))
        if desc:
            lines.append(f"  [cyan]Description[/]  {_markup(desc[:200])}")
            if len(desc) > 200:
                lines.append(f"               {_markup(desc[200:500])}")

        status = service.resolve_status(item, resource, schema)
        lines.append(f"  [cyan]Status[/]       [green]{_markup(status)}[/]")
        lines.append(f"  [cyan]Priority[/]     {_markup(service.resolve_priority(item))}")

        if resource.name == "changes":
            ct = schema_mod.choice_label("change_type", item.get("change_type"), schema)
            lines.append(f"  [cyan]Type[/]         {_markup(ct)}")
            risk = schema_mod.RISK.get(item.get("risk"), "-")
            lines.append(f"  [cyan]Risk[/]         {_markup(risk)}")
        elif resource.name == "tickets":
            lines.append(f"  [cyan]Type[/]         {_markup(item.get('type', '-'))}")
        if item.get("impact") is not None:
            lines.append(f"  [cyan]Impact[/]       {_markup(schema_mod.IMPACT.get(item.get('impact'), '-'))}")

        if item.get("planned_start_date") or item.get("planned_end_date"):
            lines.append(
                f"  [cyan]Planned[/]      {_markup(item.get('planned_start_date', '-'))}  →  {_markup(item.get('planned_end_date', '-'))}"
            )

        lines.append("")
        lines.append("[dim]── REQUEST ──[/]")
        req = item.get("requester")
        req_name = req.get("name") if isinstance(req, dict) else "-"
        lines.append(f"  [cyan]Requester[/]    {_markup(req_name)}")
        agent = item.get("agent") or item.get("responder")
        agent_name = agent.get("name") if isinstance(agent, dict) else "-"
        lines.append(f"  [cyan]Agent[/]        {_markup(agent_name)}")
        group = item.get("group")
        group_name = group.get("name") if isinstance(group, dict) else "-"
        lines.append(f"  [cyan]Group[/]        {_markup(group_name)}")

        dept = item.get("department")
        if isinstance(dept, dict) and dept.get("name"):
            lines.append(f"  [cyan]Department[/]   {_markup(dept['name'])}")

        cf = item.get("custom_fields") or {}
        visible = {k: v for k, v in cf.items() if v not in (None, "", [], False)}
        if visible:
            lines.append("")
            lines.append("[dim]── CUSTOM FIELDS ──[/]")
            for k, v in visible.items():
                label = (schema_mod.field(k, schema) or {}).get("label") or k
                if isinstance(v, list):
                    v = ", ".join(str(x) for x in v)
                elif isinstance(v, dict):
                    v = v.get("name") or v.get("value") or str(v)
                lines.append(f"  [cyan]{_markup(label)}[/]  {_markup(v)}")

        pf = item.get("planning_fields") or {}
        if pf:
            lines.append("")
            lines.append("[dim]── PLANNING FIELDS ──[/]")
            for k, v in pf.items():
                if isinstance(v, dict):
                    desc_text = v.get("description_text") or v.get("name") or ""
                    if desc_text:
                        lines.append(f"  [cyan]{_markup(k)}[/]  {_markup(desc_text[:200])}")

        lines.append("")
        return "\n".join(lines)

    def _format_notes(self, notes: list[dict]) -> str:
        if not notes:
            return "\n[dim]no notes[/]\n"
        lines: list[str] = [""]
        for note in notes:
            user = (note.get("user") or {}).get("name") or "?"
            created = (note.get("created_at") or "")[:19]
            private = note.get("private", False)
            lock = " [yellow]🔒[/]" if private else ""
            lines.append(f"  [bold]{_markup(user)}[/]{lock}")
            lines.append(f"  [dim]{_markup(created)}[/]")
            body = strip_html(note.get("body") or note.get("body_text") or "")
            if body:
                for line in body[:500].split("\n"):
                    lines.append(f"    {_markup(line)}")
            lines.append("")
        return "\n".join(lines)

    def _format_tasks(self, tasks: list[dict]) -> str:
        if not tasks:
            return "\n[dim]no tasks[/]\n"
        lines: list[str] = [""]
        for task in tasks:
            tid = task.get("id", "?")
            title = task.get("title") or task.get("description") or "-"
            status_val = task.get("status")
            from fsv.task_status import display as task_status_display
            status = task_status_display(task.get("status_name") or status_val)
            group = (task.get("group") or {}).get("name") if isinstance(task.get("group"), dict) else ""
            agent = (task.get("agent") or {}).get("name") if isinstance(task.get("agent"), dict) else ""
            due = (task.get("due_date") or "")[:10]

            icon = "✓" if status == "Completed" else "○"
            color = "green" if status == "Completed" else "yellow" if status == "In Progress" else "white"
            lines.append(f"  [{color}]{icon}[/] [bold]#TSK-{_markup(tid)}[/] {_markup(title)}")
            parts = []
            if group:
                parts.append(group)
            if agent:
                parts.append(agent)
            if due:
                parts.append(f"Due: {due}")
            lines.append(f"    [{color}]{_markup(status)}[/]  [dim]{_markup(' / '.join(parts))}[/]")
            lines.append("")
        return "\n".join(lines)

    def _format_approvals(self, approvals: list[dict]) -> str:
        if not approvals:
            return "\n[dim]no approvals[/]\n"
        lines: list[str] = [""]
        for a in approvals:
            member = (a.get("member") or {}).get("name") or str(a.get("member_id", "?"))
            status_obj = a.get("status") or {}
            status = status_obj.get("name") if isinstance(status_obj, dict) else str(status_obj)
            level = a.get("level_id", "?")
            created = (a.get("created_at") or "")[:19]

            if status and status.lower() == "approved":
                icon = "[green]✓[/]"
            elif status and status.lower() == "rejected":
                icon = "[red]✗[/]"
            else:
                icon = "[yellow]⏳[/]"
            lines.append(f"  {icon} [bold]{_markup(member)}[/]  L{_markup(level)}")
            lines.append(f"    {_markup(status)}  [dim]{_markup(created)}[/]")
            lines.append("")
        return "\n".join(lines)

    def _format_assets(self, assets: list[dict]) -> str:
        if not assets:
            return "\n[dim]no assets[/]\n"
        lines: list[str] = [""]
        for asset in assets:
            item = asset.get("config_item") if isinstance(asset.get("config_item"), dict) else asset
            name = item.get("name") or item.get("display_name") or "-"
            display_id = item.get("display_id") or item.get("id")
            atype = item.get("asset_type") or item.get("ci_type") or item.get("ci_type_name") or ""
            dept = (item.get("department") or {}).get("name") if isinstance(item.get("department"), dict) else ""
            location = item.get("location_name") or item.get("location") or ""
            lines.append(f"  [bold]{_markup(name)}[/]")
            parts = []
            if display_id:
                parts.append(f"ID: {display_id}")
            if atype:
                parts.append(f"Type: {atype}")
            if dept:
                parts.append(f"Dept: {dept}")
            if location:
                parts.append(f"Location: {location}")
            if parts:
                lines.append(f"    [dim]{_markup(' · '.join(parts))}[/]")
            lines.append("")
        return "\n".join(lines)

    def _format_associations(self, data: dict, resource: Resource) -> str:
        if not data:
            return "\n[dim]no associations[/]\n"
        lines: list[str] = [""]
        lines.extend(_markup(w) for w in data.get("warnings", []))
        for key, items in data.items():
            if key in ("meta", "warnings", "complete") or not isinstance(items, list) or not items:
                continue
            lines.append(f"  [dim]── {_markup(key.upper())} ──[/]")
            for item in items:
                did = item.get("human_display_id") or item.get("display_id") or item.get("id")
                subject = (item.get("subject") or "")[:60]
                status = item.get("status_name") or str(item.get("status", ""))
                pri = schema_mod.PRIORITY.get(item.get("priority", 0), "-")
                lines.append(f"    [cyan]{_markup(did)}[/]  {_markup(subject)}")
                lines.append(f"      Status: {_markup(status)}  Priority: {_markup(pri)}")
            lines.append("")
        return "\n".join(lines)

    def _format_activity(self, activities: list[dict]) -> str:
        if not activities:
            return "\n[dim]no activity[/]\n"
        lines: list[str] = [""]
        current_date = ""
        for act in activities:
            created = act.get("created_at") or ""
            date_part = created[:10]
            time_part = created[11:16] if len(created) > 16 else ""

            if date_part != current_date:
                current_date = date_part
                lines.append(f"  [dim]── {_markup(date_part)} ──[/]")

            actor = (act.get("actor") or {}).get("name") if isinstance(act.get("actor"), dict) else "System"
            content = strip_html(act.get("content") or act.get("note_content") or "")
            lines.append(f"  [dim]{_markup(time_part)}[/]  [bold]{_markup(actor)}[/]")
            if content:
                for line in content[:300].split("\n"):
                    lines.append(f"         {_markup(line)}")
            lines.append("")
        return "\n".join(lines)

    def _format_resolution(self, item: dict) -> str:
        lines: list[str] = [""]
        resolved = item.get("stats", {}).get("resolved_at") or item.get("resolved_at")
        if resolved:
            lines.append(f"  [cyan]Resolved at[/]  {_markup(resolved)}")

        resolution = item.get("resolution_notes") or ""
        if resolution:
            lines.append("")
            lines.append("  [dim]── Resolution Note ──[/]")
            text = strip_html(resolution)
            for line in text.split("\n"):
                lines.append(f"    {_markup(line)}")

        if not resolved and not resolution:
            lines.append("  [dim]no resolution[/]")

        lines.append("")
        return "\n".join(lines)

    # ── Key handling ────────────────────────────────────────

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id == "filter-input":
            self._apply_filter(event.value)
            event.stop()
        elif event.input.id == "search-input":
            self._apply_search(event.value)
            event.stop()

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id == "filter-input":
            if self._history_nav_lock > 0:
                self._history_nav_lock -= 1
                self._update_suggestion_bar(event.value)
            elif self._tab_lock > 0:
                self._tab_lock -= 1
                # bar already updated by _complete_suggestion
            else:
                self._history_idx = -1
                self._suggestion_idx = 0
                self._tab_base = None
                self._update_suggestion_bar(event.value)

    def on_key(self, event) -> None:
        key = event.key

        if isinstance(self.screen, ModalScreen):
            return

        search_inp = self.query_one("#search-input", Input)
        if search_inp.display and search_inp.has_focus:
            if key == "escape":
                self._cancel_search_input()
                event.stop()
                event.prevent_default()
            return

        inp = self.query_one("#filter-input", Input)
        if inp.display and inp.has_focus:
            if key == "escape":
                self._cancel_filter_input()
                event.stop()
                event.prevent_default()
                return
            if key == "tab":
                self._complete_suggestion()
                event.stop()
                event.prevent_default()
                return
            if key in ("up", "ctrl+p"):
                self._history_navigate(1)
                event.stop()
                event.prevent_default()
                return
            if key in ("down", "ctrl+n"):
                self._history_navigate(-1)
                event.stop()
                event.prevent_default()
                return
            return

        table = self.query_one("#list", DataTable)
        detail = self.query_one("#detail-scroll", VerticalScroll)
        pane = self._active_pane()

        if self._g_pending:
            self._g_pending = False
            pending_pane = self._g_pending_pane
            if key == "g":
                if pending_pane == "detail":
                    detail.scroll_home(animate=False, immediate=True)
                else:
                    table.move_cursor(row=0)
            elif key in ENTITY_TAB_KEY:
                if self._search_mode:
                    self._search_narrow_entity(ENTITY_TAB_KEY[key])
                else:
                    self._switch_entity(ENTITY_TAB_KEY[key])
            event.stop()
            event.prevent_default()
            return

        if key == "g":
            self._g_pending = True
            self._g_pending_pane = pane
            event.stop()
            event.prevent_default()
            return

        if key == "f":
            self._open_search_input()
            event.stop()
            event.prevent_default()
            return
        if key == "escape" and self._search_mode:
            self._exit_search_mode()
            event.stop()
            event.prevent_default()
            return
        if key == "s" and self._search_mode:
            self._cycle_search_sort()
            event.stop()
            event.prevent_default()
            return

        if key == "slash":
            if self._search_mode:
                return
            self._open_filter_input()
            event.stop()
            event.prevent_default()
        elif key == "tab":
            self._toggle_pane()
            event.stop()
            event.prevent_default()
        elif key == "ctrl+j":
            self._focus_pane("detail")
            event.stop()
            event.prevent_default()
        elif key == "ctrl+k":
            self._focus_pane("list")
            event.stop()
            event.prevent_default()
        elif key in ("question_mark", "?"):
            self.push_screen(HelpScreen())
            event.stop()
            event.prevent_default()
        elif key == "j" or key == "down":
            if pane == "detail":
                detail.scroll_down(animate=False, immediate=True)
            else:
                table.action_cursor_down()
            event.stop()
            event.prevent_default()
        elif key == "k" or key == "up":
            if pane == "detail":
                detail.scroll_up(animate=False, immediate=True)
            else:
                table.action_cursor_up()
            event.stop()
            event.prevent_default()
        elif key in ("J", "right_curly_bracket"):
            if pane == "detail":
                for _ in range(5):
                    detail.scroll_down(animate=False, immediate=True)
            else:
                for _ in range(5):
                    table.action_cursor_down()
            event.stop()
            event.prevent_default()
        elif key in ("K", "left_curly_bracket"):
            if pane == "detail":
                for _ in range(5):
                    detail.scroll_up(animate=False, immediate=True)
            else:
                for _ in range(5):
                    table.action_cursor_up()
            event.stop()
            event.prevent_default()
        elif key == "G":
            if pane == "detail":
                detail.scroll_end(animate=False, immediate=True)
            elif table.row_count > 0:
                table.move_cursor(row=table.row_count - 1)
            event.stop()
            event.prevent_default()
        elif key == "h" or key == "left":
            if pane == "detail":
                self._step_detail_tab(-1)
            elif self._search_mode:
                self._step_search_entity(-1)
            else:
                self._prev_entity()
            event.stop()
            event.prevent_default()
        elif key == "l" or key == "right":
            if pane == "detail":
                self._step_detail_tab(1)
            elif self._search_mode:
                self._step_search_entity(1)
            else:
                self._next_entity()
            event.stop()
            event.prevent_default()
        elif key == "o":
            self._open_browser()
            event.stop()
            event.prevent_default()
        elif key == "a":
            self._action_menu()
            event.stop()
            event.prevent_default()
        elif key == "!" or key == "exclamation_mark":
            if self._write_failure:
                self.push_screen(FailureScreen(self._write_failure))
            event.stop()
            event.prevent_default()
        elif key in ("s", "n") or (key == "r" and pane == "detail" and self._reply_possible()):
            self._write_key(key)
            event.stop()
            event.prevent_default()
        elif key == "r":
            if self._search_mode:
                self._apply_search(self._search_term)
            else:
                self._reload_list()
            event.stop()
            event.prevent_default()
        elif key == "y":
            self._yank()
            event.stop()
            event.prevent_default()
        elif key == "q":
            self.exit()
            event.stop()
            event.prevent_default()
        elif key == "left_square_bracket":
            self._resize_panes(-1)
            event.stop()
            event.prevent_default()
        elif key == "right_square_bracket":
            self._resize_panes(1)
            event.stop()
            event.prevent_default()
        elif key == "v":
            self._pick_view()
            event.stop()
            event.prevent_default()
        elif key in ("N", "shift+n"):
            self._open_form("new")
            event.stop()
            event.prevent_default()
        elif key == "e":
            self._open_form("edit")
            event.stop()
            event.prevent_default()
        elif key in ("1", "2", "3", "4", "5", "6", "7", "8", "9"):
            idx = int(key) - 1
            if self._selected:
                res = self._selected.get("_resource", TICKETS)
                tabs = self._tab_names(res)
                if idx < len(tabs):
                    self.detail_tab_idx = idx
                    self._render_detail_tabs(res)
                    self._render_detail_content(self._selected, res)
                    self._render_filter_bar()
                    self._render_status()
            event.stop()
            event.prevent_default()

    def on_data_table_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        if event.row_key is None:
            return
        did = str(event.row_key.value)
        item = self._find_item(did)
        if not item:
            return
        res: Resource = item.get("_resource", TICKETS)
        self.detail_tab_idx = 0
        detail_key = self._item_key(item, res)
        cache_key = f"{res.name}:{item.get('id')}"
        if cache_key in self._detail_cache:
            self._pending_detail_key = detail_key
            self._pending_sub_key = None
            self._show_detail(self._detail_cache[cache_key], res, detail_key)
        else:
            self._set_pending_detail(detail_key)
        if self._detail_debounce_timer is not None:
            self._detail_debounce_timer.stop()
        self._detail_debounce_timer = self.set_timer(
            0.15, lambda: self._load_detail(item, detail_key)
        )
        table = self.query_one("#list", DataTable)
        if table.cursor_row == table.row_count - 1 and not self._loading_more:
            if self._search_mode and self._search_has_more:
                self._search_load_more()
            elif self._has_more:
                self._load_more()

    def _find_item(self, display_id: str) -> dict | None:
        for item in self._items:
            res: Resource = item.get("_resource", TICKETS)
            if format_id(item, res) == display_id:
                return item
        return None

    # ── Actions ─────────────────────────────────────────────

    def _switch_entity(self, name: str) -> None:
        if name == self.entity:
            return
        self.entity = name
        self.detail_tab_idx = 0
        self._list_epoch += 1
        self._has_more = False
        self._loading_more = False
        self._active_filters = self._get_default_filters(name)
        self._filter_error = None
        self._render_header()
        if self._entity_debounce_timer is not None:
            self._entity_debounce_timer.stop()
        self._entity_debounce_timer = self.set_timer(
            0.2, lambda: self._reload_list("[dim]loading rows...[/]")
        )

    def _next_entity(self) -> None:
        idx = ENTITY_TABS.index(self.entity)
        next_idx = (idx + 1) % len(ENTITY_TABS)
        self._switch_entity(ENTITY_TABS[next_idx])

    def _prev_entity(self) -> None:
        idx = ENTITY_TABS.index(self.entity)
        prev_idx = (idx - 1) % len(ENTITY_TABS)
        self._switch_entity(ENTITY_TABS[prev_idx])

    def _step_detail_tab(self, delta: int) -> None:
        if not self._selected:
            return
        res: Resource = self._selected.get("_resource", TICKETS)
        tabs = self._tab_names(res)
        next_idx = (self.detail_tab_idx + delta) % len(tabs)
        self.detail_tab_idx = next_idx
        self._render_detail_tabs(res)
        self._render_detail_content(self._selected, res)
        self._render_filter_bar()
        self._render_status()

    # ── Write actions ───────────────────────────────────────

    def _action_menu(self) -> None:
        if self.read_only:
            self.notify("Read-only mode: actions are disabled", severity="warning")
            return
        if not self._selected:
            self.notify("No row selected", severity="warning")
            return
        item = dict(self._selected)
        choices = [("download", "Download selected attachment")]
        if item.get("_resource", TICKETS) == CHANGES:
            choices = [("task-add", "Add task"), ("task-update", "Update selected task"), ("task-delete", "Delete selected task"),
                       ("link", "Link initiating ticket"), ("unlink", "Unlink initiating ticket")] + choices
        self.push_screen(ChoiceScreen(choices, "Selected record actions"),
                         lambda kind: self._prepare_action(kind, item) if kind is not None else None)

    def _prepare_action(self, kind: str, item: dict) -> None:
        if self.read_only:
            return
        if kind == "task-add":
            self._compose_change_task(item)
        elif kind == "link":
            self.push_screen(ValueScreen("Search initiating tickets (ID or subject)"),
                             lambda query: self._load_action_targets(kind, item, query) if query else None)
        else:
            self._load_action_targets(kind, item)

    def _compose_change_task(self, item: dict) -> None:
        if self.read_only or item.get("_resource", TICKETS) != CHANGES:
            return

        def titled(title):
            if title is None or self.read_only:
                return

            def owner_mode(mode):
                if mode is None or self.read_only:
                    return
                if mode == "none":
                    self._change_task_options(item, title, None)
                else:
                    self.push_screen(ValueScreen("Explicit task owner user ID or exact email (no implicit self)"),
                                     lambda owner: self._change_task_options(item, title, owner) if owner is not None else None)
            self.push_screen(ChoiceScreen([("none", "No owner (default)"),
                                           ("explicit", "Explicit owner user ID or email")], "Task owner"), owner_mode)
        self.push_screen(ValueScreen("New change task title — Open; reminder Never"), titled)

    def _change_task_options(self, item: dict, title: str, owner: str | None,
                             options: dict | None = None) -> None:
        """Reuse the existing pickers/inputs; no separate form engine or sender."""
        from fsv import change_tasks
        if self.read_only:
            return
        values = dict(options or {})
        fields = {"group": "Group ID or exact name", "status": "Status", "note": "Note (plain text)",
                  "notify_before": "Notify Before", "planned_start": "Planned start (ISO with offset)",
                  "planned_end": "Planned end (ISO with offset)", "planned_effort": "Planned effort (1h30m or 01:30)"}

        def set_value(key, value):
            if value is None or self.read_only:
                return
            values[key] = value
            self._change_task_options(item, title, owner, values)

        def selected(key):
            if key is None or self.read_only:
                return
            if key == "review":
                self._prepare_change_task(item, title, owner, values)
            elif key == "clear":
                self._change_task_options(item, title, owner)
            elif key == "status":
                self.push_screen(ChoiceScreen([(v, v) for v in change_tasks.STATUS_LABELS], "Task status"),
                                 lambda value: set_value(key, value))
            elif key == "notify_before":
                self.push_screen(ChoiceScreen([(v, v) for v in change_tasks.NOTIFY_BEFORE], "Notify Before"),
                                 lambda value: set_value(key, value))
            else:
                self.push_screen(ValueScreen(fields[key], values.get(key, "")), lambda value: set_value(key, value))

        choices = [("review", "Review exact task"), *[(key, f"{label}: {values.get(key, 'default / omitted')}")
                    for key, label in fields.items()], ("clear", "Clear optional fields")]
        self.push_screen(ChoiceScreen(choices, "Task optional fields — defaults Open / Never; others omitted"), selected)

    @work(thread=True, exclusive=True, group="task-create-review")
    def _prepare_change_task(self, item: dict, title: str, owner: str | None, options: dict | None = None) -> None:
        from fsv import change_tasks
        if self.read_only:
            return
        try:
            values = dict(options or {})
            group = values.pop("group", None)
            status = values.pop("status", "Open")
            cid = item.get("display_id") or item["id"]
            change_tasks.status_label(status)
            change_tasks.build_plan(cid, title, status_id=1, **values)
            owner_id = change_tasks.resolve_owner(owner)
            plan = change_tasks.build_plan(cid, title, owner_id=owner_id,
                                           group_id=change_tasks.resolve_group(group), status=status,
                                           status_id=change_tasks.resolve_status(status), **values)
            self.call_from_thread(self._review_change_task, item, plan)
        except Exception as exc:
            self.call_from_thread(self._show_failure, actions.failure_detail(exc))

    def _review_change_task(self, item: dict, plan: Any) -> None:
        if self.read_only:
            return
        self.push_screen(ConfirmScreen("Review exact new change task", [json.dumps(plan.review(), indent=2)]),
                         lambda ok: self._send_change_task(item, plan) if ok else None)

    @work(thread=True, group="write")
    def _send_change_task(self, item: dict, plan: Any) -> None:
        from fsv import change_tasks
        if self.read_only:
            return
        try:
            result = change_tasks.send(plan, reviewed_hash=plan.hash, read_only=self.read_only)
        except Exception as exc:
            self.call_from_thread(self._show_failure, actions.failure_detail(exc))
            return
        if not result.get("verified") or result.get("unverified_fields"):
            self.call_from_thread(self._show_failure,
                                  f"Task ID {result.get('task_id')} submitted; readback incomplete or unverified.\n{result.get('warning', '')}\nDo not resend.")
            return
        self.call_from_thread(self._after_write, CHANGES, item, f"Task {result['task_id']} verified")

    @work(thread=True, exclusive=True, group="action-read")
    def _load_action_targets(self, kind: str, item: dict, query: str = "") -> None:
        from fsv import create
        res = item.get("_resource", TICKETS)
        warnings = []
        try:
            client = get_client()
            if kind == "download":
                rows, warnings = actions.download_choices(client, res, item)
            elif kind == "link":
                rows = create.search_change_tickets(query, c=client)
            else:
                tab = V.load_tab(client, CHANGES, item, "tasks" if kind.startswith("task-") else "associations")
                rows, warnings, _ = actions.target_inventory(tab, kind.startswith("task-"))
            self.call_from_thread(self._pick_action_target, kind, item, rows, warnings)
        except Exception as exc:
            self.call_from_thread(self._show_failure, f"Could not load action targets: {exc}")

    def _pick_action_target(self, kind: str, item: dict, rows: list[dict], warnings: list[str]) -> None:
        if self.read_only:
            return
        if not rows:
            self._show_failure("No available targets.\n" + "\n".join(map(str, warnings)))
            return
        choices = []
        for row in rows:
            if warnings:
                row = {**row, "_inventory_warnings": list(map(str, warnings))}
            identity = row.get("human_display_id") or row.get("display_id") or row.get("id")
            label = row.get("title") or row.get("subject") or row.get("name") or row.get("filename") or "attachment"
            choices.append((row, f"{identity}: {label} {row.get('source', '')}"))

        def picked(target):
            if target is None or self.read_only:
                return
            if kind == "task-update":
                self._load_task_editor(item, target)
            elif kind == "download":
                self.push_screen(ValueScreen("Download destination directory", "."),
                                 lambda path: self._confirm_action(kind, item, target, {"destination": path}) if path else None)
            else:
                self._confirm_action(kind, item, target, {})

        title = kind + (" — partial inventory; inspect warnings" if warnings else "")
        if warnings:
            self.notify("\n".join(map(str, warnings)), severity="warning", timeout=15)
        self.push_screen(ChoiceScreen(choices, title), picked)

    @work(thread=True, exclusive=True, group="task-read")
    def _load_task_editor(self, item: dict, target: dict) -> None:
        from fsv import create
        try:
            current = create.get_task_for_edit(item.get("display_id") or item["id"], target["id"], c=get_client())
            from fsv.task_status import fields_from
            target = {**target, "_status_fields": fields_from(get_client())}
            self.call_from_thread(self._edit_task_field, item, target, current)
        except Exception as exc:
            self.call_from_thread(self._show_failure, f"Task unavailable: {exc}")

    def _edit_task_field(self, item: dict, target: dict, current: dict) -> None:
        def field_picked(field):
            if field is None:
                return
            def value_picked(value):
                if value is None:
                    return
                try:
                    body = actions.task_patch(current, field, value, target.get("_status_fields"))
                except ValueError as exc:
                    self._show_failure(str(exc))
                    return
                if body:
                    self._confirm_action("task-update", item, {**target, "_before": {k: current.get(k) for k in body}}, body)
                else:
                    self.notify("No change; nothing sent")
            title = f"Task {target['id']}: {field}"
            self.push_screen(ValueScreen(title, str(current.get(field) or "")), value_picked)
        self.push_screen(ChoiceScreen([("title", "Title"), ("status", "Status"),
                                      ("group_id", "Group ID"), ("owner_id", "Agent ID")],
                                      f"Update task {target['id']} — choose one changed field"), field_picked)

    def _confirm_action(self, kind: str, item: dict, target: dict, body: dict) -> None:
        if self.read_only:
            return
        res = item.get("_resource", TICKETS)
        lines = [f"Record: {format_id(item, res)} (ID {item['id']})",
                 f"Target: {target.get('human_display_id') or target.get('id')} — {target.get('title') or target.get('subject') or target.get('name', '')}",
                 f"Action: {kind}", json.dumps(body, ensure_ascii=False),
                 "Assignment/status changes may notify people. Deletion cannot be undone." if kind.startswith("task-") else
                 "Initiating-ticket relationship only." if kind in ("link", "unlink") else
                 f"Source: {target.get('source', 'request')}. Existing files are skipped; no overwrite."]
        if "_before" in target:
            lines.insert(3, "Before: " + json.dumps(target["_before"], ensure_ascii=False))
        if kind in ("link", "unlink"):
            from fsv.change_associations import build_plan
            try:
                plan = build_plan(item.get("display_id") or item["id"], [target["id"]], unlink=kind == "unlink")
            except ValueError as exc:
                self._show_failure(str(exc))
                return
            target = {**target, "_association_plan": plan}
            lines.append(json.dumps(plan.review(), indent=2))
        lines.extend(target.get("_inventory_warnings", []))
        self.push_screen(ConfirmScreen(f"Confirm {kind}", lines),
                         lambda ok: self._execute_action(kind, item, target, body) if ok else None)

    @work(thread=True, group="write")
    def _execute_action(self, kind: str, item: dict, target: dict, body: dict) -> None:
        if self.read_only:
            return
        res = item.get("_resource", TICKETS)
        try:
            client = get_client()
            if kind == "download":
                result = actions.download_selected(client, target, body["destination"])
                if result.get("status") in ("error", "rejected"):
                    raise ValueError(json.dumps(result))
                self.call_from_thread(self.notify, json.dumps(result), title="Download result", timeout=15)
                return
            result = actions.apply_change_action(kind, item.get("display_id") or item["id"], target, body, client)
            if kind in ("link", "unlink"):
                if not isinstance(result, dict) or not result.get("verified"):
                    raise ValueError("Association outcome unknown/unverified; do not resubmit")
                self.call_from_thread(self._after_write, res, item, f"{kind} complete inventory verified")
                return
        except Exception as exc:
            self.call_from_thread(self._show_failure, actions.failure_detail(exc))
            return
        # Readback failure is not write failure: never repeat the send to recover it.
        try:
            tab = V.load_tab(client, CHANGES, item, "tasks" if kind.startswith("task-") else "associations")
            rows, _, complete = actions.target_inventory(tab, kind.startswith("task-"))
            found = next((r for r in rows if str(r.get("id")) == str(target["id"])), None)
            removing = kind in ("task-delete", "unlink")
            if removing and (found or not complete):
                raise ValueError("Removal is not verified in a complete list")
            if not removing and (not found or any(found.get(k) != v for k, v in body.items())):
                raise ValueError("Exact target state was not verified")
        except Exception as exc:
            self.call_from_thread(self._show_failure, f"Write sent once; readback unverified: {exc}. Do not resend; inspect the record.")
            return
        self.call_from_thread(self._after_write, res, item, f"{kind} verified")

    def _reply_possible(self) -> bool:
        return bool(self._selected) and not self.read_only and actions.can_reply(
            self._selected.get("_resource", TICKETS))

    def _write_key(self, key: str) -> None:
        if self.read_only:
            self.notify("Read-only mode: write actions are disabled", severity="warning")
            return
        if not self._selected:
            self.notify("No row selected", severity="warning")
            return
        res: Resource = self._selected.get("_resource", TICKETS)
        if key == "s":
            self._start_status(res)
        elif key == "n":
            self._start_note(res, reply=False)
        elif key == "r":
            self._start_note(res, reply=True)

    def _target_label(self, res: Resource) -> str:
        return format_id(self._selected, res)

    def _start_status(self, res: Resource) -> None:
        if not actions.can_change_status(res):
            self.notify(f"Status change is not available for {res.name}", severity="warning")
            return
        sch = self._schemas.get(res.name, {"fields": []})
        choices = actions.status_choices(sch)
        if not choices:
            self.notify("No status choices in schema", severity="warning")
            return
        item = self._selected
        current = service.resolve_status(item, res, sch)

        def picked(choice) -> None:
            if not choice:
                return
            new_id, label = choice
            plan = actions.status_plan(res, item, sch, new_id, label)

            def confirmed(ok: bool | None) -> None:
                if ok:
                    self._do_write("status", res, item, new_id, label)

            self.push_screen(ConfirmScreen(plan.title, plan.lines), confirmed)

        self.push_screen(StatusScreen(choices, current), picked)

    def _start_note(self, res: Resource, reply: bool) -> None:
        ok = actions.can_reply(res) if reply else actions.can_note(res)
        if not ok:
            self.notify("Not available for this item type", severity="warning")
            return
        item = dict(self._selected or {})
        # Ticket notes use the reviewed sender; public replies remain separate.
        screen = NoteScreen(self._target_label(res), reply=reply, can_toggle=True)

        def composed(result) -> None:
            if not result:
                return
            body, public = result
            if res == TICKETS and not reply:
                self._review_ticket_note(item, body, public)
                return
            plan = actions.note_plan(res, public, reply)

            def confirmed(ok2: bool | None) -> None:
                if ok2:
                    self._do_write("reply" if reply else "note", res, item, body, public)

            preview = ["", "> " + body]
            self.push_screen(ConfirmScreen(plan.title, plan.lines + preview), confirmed)

        self.push_screen(screen, composed)

    def _review_ticket_note(self, item: dict, body: str, public: bool) -> None:
        from fsv import ticket_notes
        if self.read_only:
            return
        try:
            plan = ticket_notes.build_plan(item.get("display_id") or item["id"], body, public=public,
                                           notify_emails=[], body_format="plain")
            review = json.dumps(plan.review(), ensure_ascii=False, indent=2)
        except Exception as exc:
            self._show_failure(str(exc))
            return

        def reviewed(ok):
            if not ok:
                return
            if plan.public:
                self.push_screen(ConfirmScreen("Confirm PUBLIC visibility", [
                    "This is a public note visible to the customer, not a reply.",
                    "No explicit notification recipients. Tenant workflows may still notify people."]),
                    lambda confirmed: self._send_ticket_note(plan, True) if confirmed else None)
            else:
                self._send_ticket_note(plan, False)
        self.push_screen(ConfirmScreen("Review exact ticket note", [review]), reviewed)

    @work(thread=True, group="write")
    def _send_ticket_note(self, plan: Any, public_confirmed: bool) -> None:
        from fsv import ticket_notes
        if self.read_only:
            return
        try:
            result = ticket_notes.send(plan, get_client(), reviewed_hash=plan.hash,
                                       public_confirmed=public_confirmed, read_only=self.read_only)
        except Exception as exc:
            self.call_from_thread(self._show_failure, actions.failure_detail(exc))
            return
        if not result.get("verified"):
            self.call_from_thread(self._show_failure,
                                  f"Note ID {result.get('note_id')} submitted; readback unverified.\n{result.get('warning', '')}\nDo not resend.")
            return
        item = {"id": plan.ticket_id, "display_id": plan.ticket_id, "_resource": TICKETS}
        selected = self._selected
        if selected and selected.get("_resource", TICKETS) == TICKETS and (selected.get("display_id") or selected.get("id")) == plan.ticket_id:
            item = selected
        self.call_from_thread(self._after_write, TICKETS, item, f"Note {result.get('note_id')} verified")

    @work(thread=True, group="write")
    def _do_write(self, kind: str, res: Resource, item: dict, arg: Any, extra: Any) -> None:
        if self.read_only:
            return
        item_id = item.get("id")
        try:
            if kind == "status":
                actions.apply_status(res, item_id, arg)
                msg = f"Status set to {extra}"
            elif kind == "reply":
                actions.apply_reply(res, item_id, arg)
                msg = "Reply sent"
            else:
                actions.apply_note(res, item_id, arg, bool(extra))
                msg = "Public note added" if extra else "Private note added"
        except Exception as exc:  # noqa: BLE001 - preserve ambiguous failures
            self.call_from_thread(self._show_failure, actions.failure_detail(exc))
            return
        self.call_from_thread(self._after_write, res, item, msg)

    def _after_write(self, res: Resource, item: dict, msg: str) -> None:
        self.notify(msg)
        prefix = f"{res.name}:{item.get('id')}"
        self._detail_cache.pop(prefix, None)
        for k in [k for k in self._sub_cache if k.startswith(prefix + ":")]:
            self._sub_cache.pop(k, None)
        if self._selected and self._selected.get("id") == item.get("id") and self._selected.get("_resource", TICKETS) == res:
            key = self._item_key(item, res)
            self._pending_detail_key = key
            self._load_detail(item, key)

    def _open_browser(self) -> None:
        if not self._selected:
            self.notify("No row selected", severity="warning")
            return
        res: Resource = self._selected.get("_resource", TICKETS)
        item_id = self._selected.get("id")
        url = service.item_url(res, item_id)
        opened = webbrowser.open(url)
        if not opened:
            self.notify("Browser open failed", severity="warning")

    def _yank(self) -> None:
        if not self._selected:
            self.notify("No row selected", severity="warning")
            return
        res: Resource = self._selected.get("_resource", TICKETS)
        did = format_id(self._selected, res)
        try:
            subprocess.run(["pbcopy"], input=did.encode(), check=True, capture_output=True)
            self.notify(f"Copied {did}")
        except Exception as exc:
            self.notify(f"Copy failed: {exc}", severity="warning")
