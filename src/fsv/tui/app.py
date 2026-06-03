from __future__ import annotations

import subprocess
import webbrowser
from typing import Any

from rich.markup import escape
from rich.text import Text

from textual import work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Vertical, VerticalScroll
from textual.reactive import var
from textual.screen import ModalScreen
from textual.widgets import DataTable, Static

from fsv import schema as schema_mod, service
from fsv.client import APIError, get_client
from fsv.render import strip_html
from fsv.resources import CHANGES, PROBLEMS, TICKETS, Resource, format_id


ALL_RESOURCES = (TICKETS, CHANGES, PROBLEMS)

ENTITY_TABS = ("work", "tickets", "problems", "changes")

DETAIL_TAB_LABELS: dict[str, list[str]] = {
    "changes": ["details", "notes", "tasks", "assets", "associations", "approvals", "activity"],
    "tickets": ["details", "conversations", "tasks", "assets", "associations", "approvals", "activity", "resolution"],
    "problems": ["details", "notes", "tasks", "assets", "associations", "activity"],
}

ENTITY_TAB_KEY = {"w": "work", "t": "tickets", "p": "problems", "c": "changes"}


def _resource_for_entity(name: str) -> Resource | None:
    return {"tickets": TICKETS, "problems": PROBLEMS, "changes": CHANGES}.get(name)


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
  [cyan]gw/gt/gp/gc[/]  jump to work/tickets/problems/changes
  [cyan]1-8[/]          switch detail tab
  [cyan]\\[][/cyan]         resize list/detail pane

[bold]Selection[/]
  moving list cursor loads detail automatically
  [cyan]y[/]            copy selected display id
  [cyan]o[/]            open selected item in browser

[bold]App[/]
  [cyan]r[/]            reload list and clear caches
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

    entity = var("work")
    detail_tab_idx = var(0)

    def __init__(self) -> None:
        super().__init__(ansi_color=True)
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
        self._list_fr = 11
        self._detail_fr = 9

    def compose(self) -> ComposeResult:
        yield Static(id="header")
        with Vertical(id="main"):
            with Vertical(id="list-pane"):
                yield DataTable(id="list")
                yield Static(id="filter-bar")
            with Vertical(id="detail"):
                yield Static(id="detail-bar")
                yield Static(id="detail-tabs")
                with VerticalScroll(id="detail-scroll"):
                    yield Static(id="detail-content")
        yield Static(id="status-bar")
        yield Static(id="key-hints")

    def on_mount(self) -> None:
        self.screen.styles.background = "transparent"
        for widget in self.query("Vertical, VerticalScroll, Static"):
            widget.styles.background = "transparent"
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
    def _load_list(self) -> None:
        c = get_client()
        for res in ALL_RESOURCES:
            if res.name not in self._schemas:
                self._schemas[res.name] = schema_mod.load(res, c)

        if self.entity == "work":
            items, total = service.list_work_items(client=c)
        else:
            res = _resource_for_entity(self.entity)
            items, total = service.list_items(res, client=c)

        self.call_from_thread(self._populate_list, items, total)

    def _populate_list(self, items: list[dict], total: int) -> None:
        self._items = items
        self._total = total
        table = self.query_one("#list", DataTable)
        table.clear(columns=True)

        if self.entity == "work":
            table.add_columns("", "ID", "MODULE", "SUBJECT", "STATUS", "PRI", "GROUP/OWNER", "UPDATED")
        else:
            table.add_columns("", "ID", "STATUS", "PRI", "SUBJECT", "REQUESTER")

        for idx, item in enumerate(items):
            res: Resource = item.get("_resource", TICKETS)
            schema = self._schemas.get(res.name, {"fields": []})
            did = format_id(item, res)
            status = service.resolve_status(item, res, schema)
            pri = service.resolve_priority(item)
            sel = f"{idx + 1:02d}"

            if self.entity == "work":
                module = res.name.upper()
                subject = (item.get("subject") or "")[:60]
                group = ((item.get("group") or {}).get("name") if isinstance(item.get("group"), dict) else "") or ""
                updated = (item.get("updated_at") or "")[:10]
                table.add_row(sel, did, module, subject, status, pri, group, updated, key=did)
            else:
                subject = (item.get("subject") or "")[:80]
                requester = ((item.get("requester") or {}).get("name") if isinstance(item.get("requester"), dict) else "") or ""
                table.add_row(sel, did, status, pri, subject, requester, key=did)

        if items:
            self._clear_detail("[dim]loading details...[/]")
            table.move_cursor(row=0)
        else:
            self._clear_detail("[dim]no rows[/]")

        self._render_header()
        self._render_filter_bar()
        self._render_status()

    @work(thread=True, exclusive=True, group="detail")
    def _load_detail(self, item: dict, detail_key: str) -> None:
        res: Resource = item.get("_resource", TICKETS)
        item_id = item.get("id")
        cache_key = f"{res.name}:{item_id}"

        if cache_key in self._detail_cache:
            detail = self._detail_cache[cache_key]
        else:
            c = get_client()
            detail = service.get_item(res, item_id, client=c)
            self._detail_cache[cache_key] = detail

        self.call_from_thread(self._show_detail, detail, res, detail_key)

    def _show_detail(self, item: dict, resource: Resource, detail_key: str) -> None:
        if self._pending_detail_key != detail_key:
            return
        self._selected = item
        self._pending_sub_key = None
        self._render_detail_bar(item, resource)
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
            try:
                if tab_name in ("notes", "conversations"):
                    data = service.get_notes(res, item_id, client=c)
                elif tab_name == "tasks":
                    data = service.get_tasks(res, item_id, client=c)
                elif tab_name == "approvals":
                    data = service.get_approvals(res, item_id, client=c)
                elif tab_name == "assets":
                    data = service.get_assets(res, item_id, client=c)
                elif tab_name == "associations":
                    data = service.get_associations(res, item_id, client=c)
                elif tab_name == "activity":
                    data = service.get_activities(res, item_id, client=c)
                elif tab_name == "resolution":
                    data = item
                else:
                    data = None
            except APIError as exc:
                data = [] if exc.status == 404 else None
            self._sub_cache[cache_key] = data

        self.call_from_thread(self._render_sub_content, tab_name, data, res, sub_key)

    def _item_key(self, item: dict, resource: Resource) -> str:
        return f"{resource.name}:{item.get('id')}"

    def _selected_key(self) -> str | None:
        if not self._selected:
            return None
        resource: Resource = self._selected.get("_resource", TICKETS)
        return self._item_key(self._selected, resource)

    def _current_tab_name(self, resource: Resource | None = None) -> str:
        if resource is None:
            if not self._selected:
                return "details"
            resource = self._selected.get("_resource", TICKETS)
        tabs = DETAIL_TAB_LABELS.get(resource.name, ["details"])
        return tabs[min(self.detail_tab_idx, len(tabs) - 1)]

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
        self.query_one("#detail-content", Static).update(Text.from_markup(message))
        self._reset_detail_scroll()
        self._render_filter_bar()
        self._render_status()

    def _reload_list(self, message: str = "[dim]reloading...[/]") -> None:
        self._items = []
        self._total = 0
        self._g_pending = False
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
        parts.append("[bold cyan]fsv[/] :: freshservice  ")
        for tab in ENTITY_TABS:
            label = tab.upper()
            if tab == self.entity:
                parts.append(f"[bold reverse] {label} [/] ")
            else:
                parts.append(f"{label}  ")
        rows = len(self._items)
        parts.append(f"  [dim]{rows} rows  ? help[/]")
        self.query_one("#header", Static).update(Text.from_markup("".join(parts)))

    def _render_filter_bar(self) -> None:
        sel = format_id(self._selected, self._selected["_resource"]) if self._selected else "-"
        pane = self._active_pane()
        self.query_one("#filter-bar", Static).update(
            Text(f"filter=none  focus={pane}  sel={sel}", style="dim")
        )

    def _render_detail_bar(self, item: dict, resource: Resource) -> None:
        schema = self._schemas.get(resource.name, {"fields": []})
        status = service.resolve_status(item, resource, schema)
        pri = service.resolve_priority(item)
        did = format_id(item, resource)
        subject = item.get("subject") or ""
        bar = self.query_one("#detail-bar", Static)
        text = Text()
        text.append(f"[{status}]", style="bold green")
        text.append(f"  {did} {subject[:80]}  ")
        text.append(str(pri), style="dim")
        bar.update(text)

    def _render_detail_tabs(self, resource: Resource) -> None:
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
            content = self._format_details(item, resource)
            self.query_one("#detail-content", Static).update(Text.from_markup(content))
            self._reset_detail_scroll()
        else:
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
        if tab_name in ("notes", "conversations"):
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
        entity = self.entity.upper()
        sel = ""
        if self._selected:
            did = format_id(self._selected, self._selected["_resource"])
            tab_name = self._current_tab_name(self._selected["_resource"])
            sel = f" > {did} > {tab_name}"
        rows = len(self._items)
        pane = self._active_pane()
        bar = self.query_one("#status-bar", Static)
        text = Text(f"{entity}{sel}    ")
        text.append(f"pane={pane} · {rows} rows", style="dim")
        bar.update(text)

    def _render_key_hints(self) -> None:
        hints = (
            "[bold]Tab[/] pane  "
            "[bold]j/k[/] move or scroll  "
            "[bold]J/K[/] ×5  "
            "[bold]G/gg[/] ↕  "
            "[bold]h/l[/] entity/tab  "
            "[bold]gw/gt/gp/gc[/] entity  "
            "[bold]1-8[/] detail tab  "
            "[bold]\\[][/bold] resize  "
            "[bold]o[/] browser  "
            "[bold]spc[/] mark  "
            "[bold]r[/] reload  "
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
            status_map = {1: "Open", 2: "In Progress", 3: "Completed"}
            status = status_map.get(status_val, str(status_val))
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
        for key, items in data.items():
            if not isinstance(items, list) or not items:
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

    def on_key(self, event) -> None:
        key = event.key
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

        if key == "tab":
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
        elif key == "J":
            if pane == "detail":
                for _ in range(5):
                    detail.scroll_down(animate=False, immediate=True)
            else:
                for _ in range(5):
                    table.action_cursor_down()
            event.stop()
            event.prevent_default()
        elif key == "K":
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
            else:
                self._prev_entity()
            event.stop()
            event.prevent_default()
        elif key == "l" or key == "right":
            if pane == "detail":
                self._step_detail_tab(1)
            else:
                self._next_entity()
            event.stop()
            event.prevent_default()
        elif key == "o":
            self._open_browser()
            event.stop()
            event.prevent_default()
        elif key == "r":
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
        elif key in ("1", "2", "3", "4", "5", "6", "7", "8"):
            idx = int(key) - 1
            if self._selected:
                res = self._selected.get("_resource", TICKETS)
                tabs = DETAIL_TAB_LABELS.get(res.name, ["details"])
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
        self._reload_list("[dim]loading rows...[/]")

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
        tabs = DETAIL_TAB_LABELS.get(res.name, ["details"])
        next_idx = max(0, min(len(tabs) - 1, self.detail_tab_idx + delta))
        if next_idx == self.detail_tab_idx:
            return
        self.detail_tab_idx = next_idx
        self._render_detail_tabs(res)
        self._render_detail_content(self._selected, res)
        self._render_filter_bar()
        self._render_status()

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
