"""Render :mod:`fsv.views` data the way the web lays it out.

Every function returns Rich markup (``str``) or lines of it, so the CLI
prints them and the TUI drops them into its widgets unchanged. All values
coming from the server are escaped.
"""
from __future__ import annotations

import datetime as dt
from typing import Any, Iterable, Mapping

from rich.markup import escape

from fsv import views as V

DIM_EMPTY = "[dim]--[/]"


def _e(value: Any) -> str:
    return escape("" if value is None else str(value))


def section(title: str) -> str:
    return f"[bold]{_e(title)}[/]"


# --------------------------------------------------------------------------
# lists


def list_table(page: V.ListPage) -> Any:
    from rich.table import Table

    t = Table(show_lines=False, title=None, pad_edge=False, expand=False)
    for col in page.columns:
        style = {"status": "green", "assigned_to": "", "requester": ""}.get(col.key, "")
        t.add_column(col.label, style=style or None, overflow="fold", ratio=3 if col.key == "subject" else None)
    for row in page.rows:
        cells = []
        for col in page.columns:
            val = row.get(col.key, "")
            if col.key == "subject":
                cells.append(f"{_e(val)}\n[dim cyan]#{_e(row['id'])}[/]")
            elif col.key == "assigned_to" and val.endswith("(Me)"):
                cells.append(f"{_e(val[:-4])}[bold](Me)[/]")
            else:
                cells.append(_e(val) if val else DIM_EMPTY)
        t.add_row(*cells)
    return t


def list_header(page: V.ListPage) -> str:
    return f"[bold]{_e(page.title)}[/][dim] · {_e(page.header()[len(page.title) + 3:])}[/]"


# --------------------------------------------------------------------------
# detail: header, lifecycle, reporter, tab bar


def header_lines(d: V.Detail) -> list[str]:
    out = [f"[bold cyan]#{_e(d.id)}[/]  [bold]{_e(d.subject)}[/]"]
    status = f"[bold green]{_e(d.badge)}[/]"
    due = d.due_text()
    stats = d.record.get("stats") or {}
    closed = bool(stats.get("resolved_at") or stats.get("closed_at")) and d.resource.name == "tickets"
    if due and not closed:
        status += f", due {_e(due)}"
    out.append(status)
    return out


def lifecycle_markup(lc: V.Lifecycle | None, context: int | None = None) -> str:
    """``✓ Open › [Planning] › Awaiting Approval › …``; ``context`` keeps
    only that many states either side of the current one."""
    if lc is None or not lc.states:
        return ""
    states = lc.states
    cur = next((i for i, s in enumerate(states) if s.mark == "current"), 0)
    lo, hi = 0, len(states) - 1
    if context is not None:
        lo, hi = max(0, cur - context), min(len(states) - 1, cur + context)
    parts = ["[dim]…[/]"] if lo > 0 else []
    for s in states[lo:hi + 1]:
        if s.mark == "current":
            parts.append(f"[bold black on green] {_e(s.name)} [/]")
        elif s.mark == "done":
            parts.append(f"[green]✓ {_e(s.name)}[/]")
        else:
            parts.append(f"[dim]{_e(s.name)}[/]")
    if hi < len(states) - 1:
        parts.append("[dim]…[/]")
    return " [dim]›[/] ".join(parts)


def status_line(d: V.Detail) -> str:
    """The lifecycle bar; ``""`` when the account has none (the header's
    status line then stands alone, as on the web)."""
    return lifecycle_markup(d.lifecycle)


def reporter_markup(d: V.Detail) -> str:
    r = d.reporter()
    if not r.get("name") and not r.get("created_at"):
        return ""
    text = f"[bold]{_e(r.get('name') or 'Someone')}[/] reported {_e(r.get('relative'))}"
    if r.get("when"):
        text += f" [dim]({_e(r['when'])})[/]"
    if r.get("via"):
        text += f" via [bold]{_e(r['via'])}[/]"
    if r.get("request_for"):
        text += f"   [dim]Request for:[/] {_e(r['request_for'])}"
    return text


def tab_bar(d: V.Detail, active: str = "details", numbered: bool = False) -> str:
    parts = []
    for i, t in enumerate(d.tabs, start=1):
        label = t.label + (f" ({t.count})" if t.count else "")
        if numbered:
            label = f"{i}:{label}"
        parts.append(f"[bold reverse] {_e(label)} [/]" if t.name == active else f"[dim]{_e(label)}[/]")
    return "  ".join(parts)


# --------------------------------------------------------------------------
# detail: Details tab (main column + right column)


def _indent(text: str, pad: str = "  ") -> list[str]:
    return [pad + _e(ln) if ln else "" for ln in text.splitlines()] or [pad]


def main_lines(d: V.Detail) -> list[str]:
    out: list[str] = []
    if d.requested_items is not None:
        out.append(section("Requested items"))
        if not d.requested_items:
            out.append("  [dim]none[/]")
        for item in d.requested_items:
            j = V.requested_item_json(item)
            qty = f" × {j['quantity']}" if isinstance(j.get("quantity"), int) and j["quantity"] > 1 else ""
            out.append(f"  [bold]{_e(j['name'] or 'Requested item')}[/]{qty}   [dim]Stage:[/] {_e(j['stage'] or '-')}")
            for a in j["answers"]:
                out.append(f"    [cyan]{_e(a['label'])}[/]  {_e(a['value'])}")
        out.append("")
    elif d.requested_items_missing:
        out += [f"[dim]{_e(d.requested_items_missing.note)}[/]", ""]
    out.append(section("Description"))
    desc = d.description()
    out += _indent(desc) if desc else ["  [dim]No description[/]"]
    atts = d.attachments()
    if atts:
        out.append("")
        out.append(section("Attachments"))
        for a in atts:
            size = f" [dim]({_human_size(a['size'])})[/]" if a.get("size") else ""
            out.append(f"  📎 {_e(a['name'])}{size}")
    if d.planning is not None:
        out.append("")
        out.append(section("Planning"))
        for p in d.planning:
            help_ = f"  [dim]{_e(p.help)}[/]" if p.help and p.help != p.label else ""
            out.append(f"  [cyan]{_e(p.label)}[/]{help_}")
            if p.empty:
                out.append("    [dim]empty[/]")
                continue
            if p.text:
                out += _indent(p.text, "    ")
            for f in p.files:
                out.append(f"    📎 {_e(f)}")
    elif d.planning_missing:
        out += ["", f"[dim]{_e(d.planning_missing.note)}[/]"]
    if d.conversations is not None:
        out.append("")
        total = d.conversations_meta.get("count")
        shown = len(d.conversations)
        title = "Conversations" + (f" ({total})" if total else "")
        out.append(section(title))
        if isinstance(total, int) and total > shown:
            out.append(f"  [dim]… {total - shown} earlier (use --all-conversations)[/]")
        if not d.conversations:
            out.append("  [dim]no conversations[/]")
        for cv in d.conversations:
            out += conversation_lines(cv, d.tz)
        if d.conversations_missing:  # a partial fetch: never shown as complete
            out.append(f"  [yellow]incomplete: {_e(d.conversations_missing.note)}[/]")
    elif d.conversations_missing:
        out += ["", f"[dim]{_e(d.conversations_missing.note)}[/]"]
    return out


def conversation_lines(cv: Mapping[str, Any], tz: dt.tzinfo | None) -> list[str]:
    j = V.conversation_json(cv, tz)
    kind = j["kind"]
    mark = "[yellow]🔒 private[/]" if j["private"] else "[green]public[/]"
    when = V.fmt_when(j["created_at"], tz, year=False)
    out = [f"  [bold]{_e(j['author'] or 'Someone')}[/] {_e(kind)} · {mark} · {_e(j['relative'])} [dim]({_e(when)})[/]"]
    body = j["body"]
    out += _indent(body, "    ") if body else ["    [dim](empty)[/]"]
    for f in j["attachments"]:
        out.append(f"    📎 {_e(f)}")
    out.append("")
    return out


def _human_size(n: Any) -> str:
    try:
        size = float(n)
    except (TypeError, ValueError):
        return str(n)
    for unit in ("B", "KB", "MB", "GB"):
        if size < 1024 or unit == "GB":
            return f"{size:.0f} {unit}" if unit == "B" else f"{size:.1f} {unit}"
        size /= 1024
    return str(n)


def side_lines(d: V.Detail) -> list[str]:
    out = []
    rows = d.summary()
    lw = max((len(k) for k, _ in rows), default=6) + 2
    for label, value in rows:
        out.append(f"[cyan]{_e(label)}[/]{' ' * (lw - len(label))}{_e(value) if value else DIM_EMPTY}")
    out.append("")
    out.append(section("Properties"))
    if not d.properties:
        out.append("  [dim]no fields[/]")
    width = min(28, max((len(p.label) for p in d.properties), default=8) + 2)
    for p in d.properties:
        star = "[red]*[/]" if p.required else " "
        label = p.label if len(p.label) <= width else p.label[: width - 1] + "…"
        out.append(f"  [cyan]{_e(label)}[/]{star}{' ' * max(1, width - len(label))}{_e(p.value) if p.value else DIM_EMPTY}")
    return out


def notes_lines(d: V.Detail) -> list[str]:
    return [f"[dim]· {_e(n)}[/]" for n in d.notes()]


# --------------------------------------------------------------------------
# other tabs


def _who(obj: Any) -> str:
    return V._name(obj) if isinstance(obj, Mapping) else ""


def _status_text(v: Any) -> str:
    if isinstance(v, Mapping):
        return str(v.get("name") or v.get("value") or v.get("id") or "")
    return "" if v is None else str(v)


TASK_STATUS = {1: "Open", 2: "In Progress", 3: "Completed"}


def tab_lines(td: V.TabData, tz: dt.tzinfo | None = None, schema: Mapping[str, Any] | None = None) -> list[str]:
    if td.missing:
        return [f"[dim]{_e(td.missing.note)}[/]"]
    fn = _TAB_RENDERERS.get(td.name, _generic_tab)
    out = fn(td, tz, schema)
    out += [f"[dim]· {_e(n)}[/]" for n in td.notes]
    return out


def _notes_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    rows = td.data or []
    if not rows:
        return ["[dim]no notes[/]"]
    out = []
    for n in rows:
        lock = "[yellow]🔒 private[/]" if n.get("private") else "[green]public[/]"
        who = _who(n.get("user")) or str(n.get("user_id") or "")
        out.append(f"[bold]{_e(who or 'Someone')}[/] added a note · {lock} · {_e(V.relative(n.get('created_at')))}"
                   f" [dim]({_e(V.fmt_when(n.get('created_at'), tz, year=False))})[/]")
        body = V.html_to_text(n.get("body") or "") or str(n.get("body_text") or "")
        out += _indent(body)
        out.append("")
    return out


def _tasks_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    rows = td.data or []
    if not rows:
        return ["[dim]no tasks[/]"]
    out = []
    for t in rows:
        st = t.get("status_name") or _status_text(t.get("status_info")) or TASK_STATUS.get(t.get("status"), _status_text(t.get("status")))
        icon = "[green]✓[/]" if t.get("status") == 3 else "[yellow]◐[/]" if t.get("status") == 2 else "○"
        tid = t.get("human_display_id") or (f"#{t.get('display_id') or t.get('id')}")
        out.append(f"{icon} [bold]{_e(t.get('title') or '-')}[/] [dim]{_e(tid)}[/]")
        bits = [st]
        grp = _who(t.get("group")) or str(t.get("group_name") or "")
        agent = _who(t.get("agent")) or str(t.get("owner_name") or "")
        if grp or agent:
            bits.append(f"{grp or 'None'} / {agent or 'None'}")
        if t.get("due_date"):
            bits.append("due " + V.fmt_when(t["due_date"], tz))
        out.append("    [dim]" + _e(" · ".join(b for b in bits if b)) + "[/]")
    return out


def _approvals_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    rows = td.data or []
    if not rows:
        return ["[dim]no approvals requested[/]"]
    out = []
    levels: dict[Any, list[Mapping[str, Any]]] = {}
    for a in rows:
        levels.setdefault(a.get("level_id") or a.get("level") or 1, []).append(a)
    for lvl in sorted(levels, key=lambda x: (str(type(x)), x)):
        out.append(section(f"Level {lvl}"))
        for a in levels[lvl]:
            st = _status_text(a.get("status")) or str(a.get("approval_status") or "")
            icon = {"approved": "[green]✓[/]", "rejected": "[red]✗[/]"}.get(st.lower(), "[yellow]◷[/]")
            who = _who(a.get("member") or a.get("approver")) or str(a.get("member_id") or "")
            out.append(f"  {icon} [bold]{_e(who)}[/]  {_e(st.capitalize())}  [dim]{_e(V.relative(a.get('updated_at') or a.get('created_at')))}[/]")
            for r in a.get("remark") or []:
                if isinstance(r, Mapping) and r.get("data"):
                    out.append(f"      [dim]“{_e(r['data'])}”[/]")
    return out


def _assets_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    rows = td.data or []
    if not rows:
        return ["[dim]none associated[/]"]
    out = []
    for a in rows:
        it = a.get("config_item") if isinstance(a.get("config_item"), Mapping) else a
        typ = it.get("ci_type_name") or _who(it.get("ci_type")) or it.get("asset_type_name") or ""
        aid = it.get("display_id") or it.get("id")
        out.append(f"[bold]{_e(it.get('name') or it.get('display_name') or '-')}[/]  [dim]{_e(typ)} · #{_e(aid)}[/]")
    return out


def _activity_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    rows = td.data or []
    if not rows:
        return ["[dim]no activity[/]"]
    out = []
    day = None
    for a in rows:
        when = V.parse_dt(a.get("created_at"))
        local = when.astimezone(tz) if (when and tz) else when
        d = f"{local:%a}, {local.day} {local:%b %Y}" if local else "?"
        if d != day:
            out.append(section(d))
            day = d
        actor = _who(a.get("actor")) or "System"
        out.append(f"  [dim]{local:%H:%M}[/]  [bold]{_e(actor)}[/] {_e(V.one_line(a.get('content')))}" if local
                   else f"  [bold]{_e(actor)}[/] {_e(V.one_line(a.get('content')))}")
        for s in a.get("sub_contents") or []:
            out.append(f"         [dim]· {_e(V.one_line(s))}[/]")
    return out


def _item_line(x: Mapping[str, Any]) -> str:
    did = x.get("human_display_id") or x.get("display_id") or x.get("id")
    st = x.get("status_name") or _status_text(x.get("status"))
    return f"  [cyan]#{_e(did)}[/] {_e(x.get('subject') or x.get('name') or x.get('title') or '')}" + (f"  [dim]{_e(st)}[/]" if st else "")


def _parts_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    out = []
    any_rows = False
    for label, rows in td.parts.items():
        if label == "survey":
            continue
        out.append(section(f"{label} ({len(rows)})"))
        any_rows = any_rows or bool(rows)
        out += [_item_line(r) for r in rows] or ["  [dim]none[/]"]
    if not out:
        return ["[dim]nothing associated[/]"]
    return out


def _risk_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    data = td.data or {}
    out = []
    score = data.get("overall_score")
    computed = data.get("computed_risk")
    out.append(f"[cyan]Overall score[/]  {_e(score) if score is not None else DIM_EMPTY}")
    out.append(f"[cyan]Computed risk[/]  {_e(_status_text(computed)) if computed not in (None, '') else DIM_EMPTY}")
    factors = data.get("basic_risk_awareness") or []
    if factors:
        out.append("")
        out.append(section(f"{len(factors)} factors considered"))
        for f in factors:
            if isinstance(f, Mapping):
                name = V.choice_name(schema, "", None) or str(f.get("name") or "").replace("_", " ").capitalize()
                out.append(f"  {_e(name)} {_e(f.get('operator') or '')} [bold]{_e(f.get('value'))}[/]")
    survey = td.parts.get("survey")
    if isinstance(survey, Mapping) and survey:
        out.append("")
        out.append(section("Risk assessment"))
        for k, v in survey.items():
            if isinstance(v, (str, int, float)):
                out.append(f"  [cyan]{_e(k)}[/]  {_e(v)}")
    return out


def _resolution_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    data = td.data or {}
    out = []
    if data.get("resolved_at"):
        out.append(f"[cyan]Resolved[/]  {_e(V.fmt_when(data['resolved_at'], tz))}")
    if data.get("closed_at"):
        out.append(f"[cyan]Closed[/]  {_e(V.fmt_when(data['closed_at'], tz))}")
    if data.get("note"):
        out.append("")
        out.append(section("Resolution note"))
        out += _indent(data["note"])
    return out or ["[dim]no resolution yet[/]"]


def _generic_tab(td: V.TabData, tz: Any, schema: Any) -> list[str]:
    if td.parts:
        return _parts_tab(td, tz, schema)
    rows = td.data or []
    if isinstance(rows, list):
        return [_item_line(r) for r in rows if isinstance(r, Mapping)] or ["[dim]nothing here[/]"]
    return [_e(rows)]


_TAB_RENDERERS = {
    "notes": _notes_tab, "tasks": _tasks_tab, "approvals": _approvals_tab, "assets": _assets_tab,
    "impacted_services": _assets_tab, "activity": _activity_tab, "associations": _parts_tab,
    "devops": _parts_tab, "related_tickets": _parts_tab, "risk": _risk_tab, "resolution_notes": _resolution_tab,
}


# --------------------------------------------------------------------------
# CLI printing


def print_detail(console: Any, d: V.Detail, *, tab: str | None = None, all_tabs: bool = False) -> None:
    from rich.table import Table
    from rich.text import Text

    for ln in header_lines(d):
        console.print(ln, highlight=False)
    bar = status_line(d)
    if bar:
        console.print(bar, highlight=False)
    rep = reporter_markup(d)
    if rep:
        console.print(rep, highlight=False)
    console.print()
    active = tab or "details"
    console.print(tab_bar(d, active), highlight=False)
    console.rule(style="dim")
    if active == "details" or all_tabs:
        main = "\n".join(main_lines(d))
        side = "\n".join(side_lines(d))
        if console.width >= 100:
            grid = Table.grid(expand=True, padding=(0, 2))
            grid.add_column(ratio=3)
            grid.add_column(ratio=2)
            grid.add_row(Text.from_markup(main), Text.from_markup(side))
            console.print(grid)
        else:
            console.print(main, highlight=False)
            console.print()
            console.print(side, highlight=False)
    names: Iterable[str] = [t.name for t in d.tabs if t.name != "details"] if all_tabs else ([] if active == "details" else [active])
    for name in names:
        td = d.tab_data.get(name)
        if td is None:
            continue
        if all_tabs:
            console.print()
            console.rule(f"[bold]{_e(td.label)}[/]", align="left", style="dim")
        for ln in tab_lines(td, d.tz, d.schema):
            console.print(ln, highlight=False)
    shown = {m.note for m in (d.requested_items_missing, d.planning_missing, d.conversations_missing) if m}
    notes = [f"[dim]· {_e(n)}[/]" for n in d.notes() if n not in shown]
    if notes:
        console.print()
        for ln in notes:
            console.print(ln, highlight=False)


__all__ = [
    "conversation_lines", "header_lines", "lifecycle_markup", "list_header", "list_table", "main_lines",
    "notes_lines", "print_detail", "reporter_markup", "side_lines", "status_line", "tab_bar", "tab_lines",
]

