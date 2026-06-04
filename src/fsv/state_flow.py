"""Fetch and render change state flow (approval lifecycle)."""
from __future__ import annotations

from typing import Any

from rich.console import Console
from rich.table import Table

from fsv.client import Client, get_client
from fsv.resources import format_id


def get_state_flow(state_flow_id: int, client: Client | None = None) -> dict[str, Any]:
    """Fetch state flow definition from the internal API."""
    c = client or get_client()
    data = c.int_get(f"ws/2/change_management/{state_flow_id}", params={"include": "state_info"})
    return data.get("state_flow", data)


def render_flow(
    change: dict[str, Any],
    console: Console | None = None,
) -> None:
    """Render the state flow for a change, highlighting current position."""
    out = console or Console()
    c = get_client()

    state_flow_id = change.get("state_flow_id")
    if not state_flow_id:
        out.print("[yellow]no state_flow_id on this change[/]")
        return

    flow = get_state_flow(state_flow_id, client=c)
    state_list = flow.get("state_list", [])
    if not state_list:
        out.print("[yellow]empty state_list[/]")
        return

    current_status = change.get("status")
    current_name = change.get("status_name") or str(current_status)
    traversal_ids = set(change.get("state_traversal", []) or [])

    flow_name = flow.get("name", "?")
    hid = change.get("human_display_id") or f"CHN-{change.get('id')}"
    change_type = change.get("change_type")

    t = Table(title=f"{hid} flow — [bold]{flow_name}[/] (change_type={change_type})")
    t.add_column("→", justify="center", style="dim", width=3)
    t.add_column("Status", no_wrap=True)
    t.add_column("State", justify="center", style="dim", width=7)

    current_seen = False
    for i, s in enumerate(state_list):
        sid = s["id"]
        name = s["value"]
        visited = sid in traversal_ids
        is_current = sid == current_status

        if is_current:
            current_seen = True

        # Determine icon
        if is_current:
            icon = "[bold yellow]▶[/]"
        elif visited:
            icon = "[green]✓[/]"
        else:
            icon = "[dim]·[/]"

        if visited or is_current:
            row_style = ""
        else:
            row_style = "dim"

        if is_current:
            name_display = f"[bold yellow]{name}[/]"
            state_display = "[bold yellow]current[/]"
        elif visited:
            name_display = f"[green]{name}[/]"
            state_display = "[green]passed[/]"
        else:
            name_display = f"[dim]{name}[/]"
            state_display = "[dim]future[/]"

        t.add_row(icon, name_display, state_display)

    out.print(t)
    out.print(f"  current: [bold yellow]{current_name}[/] · past: {len(traversal_ids)} · ahead: {len(state_list) - len(traversal_ids & (set(s['id'] for s in state_list))) - 1}")

    # Show approval gates
    approvals_data = _try_get_approvals(change.get("id"), c)
    if approvals_data:
        out.print()
        out.print("[bold]Approval gates:[/]")
        for a in approvals_data:
            level = a.get("level_id")
            status = (a.get("status") or {}).get("name", "?")
            member = (a.get("member") or {}).get("name") or str(a.get("member_id", ""))
            icon = "✓" if status == "approved" else "✗" if status == "rejected" else "◷"
            out.print(f"  {icon} L{level}: {member} — {status}")


def render_compact(change: dict[str, Any], console: Console | None = None, context: int = 2) -> None:
    """Context-window state line appended to `changes get` output.

    Shows `context` states before and after the current position — like
    grep -C 2. Immediately shows where the change is, what it passed,
    and what comes next, without all 19 steps.
    """
    from rich.markup import escape
    out = console or Console()
    c = get_client()

    state_flow_id = change.get("state_flow_id")
    if not state_flow_id:
        return
    try:
        flow = get_state_flow(state_flow_id, client=c)
    except Exception:
        return

    state_list = flow.get("state_list", [])
    if not state_list:
        return

    current = change.get("status")
    traversal = set(change.get("state_traversal") or [])
    total = len(state_list)
    done = len(traversal)

    cur_idx = next((i for i, s in enumerate(state_list) if s["id"] == current), None)
    if cur_idx is None:
        return

    lo = max(0, cur_idx - context)
    hi = min(total - 1, cur_idx + context)
    window = state_list[lo : hi + 1]

    parts: list[str] = []
    if lo > 0:
        parts.append("[dim]…[/]")
    for s in window:
        sid, name = s["id"], s["value"]
        if sid == current:
            parts.append(f"[bold yellow]▶ {escape(name)}[/]")
        elif sid in traversal:
            parts.append(f"[green]{escape(name)}[/]")
        else:
            parts.append(f"[dim]{escape(name)}[/]")
    if hi < total - 1:
        parts.append("[dim]…[/]")

    sep = " [dim]→[/] "
    hid = change.get("human_display_id", "")
    out.print("state:    " + sep.join(parts) + f"  [dim][{done}/{total}][/]")
    out.print(f"[dim]          `fsv changes state {hid}` for full flow[/]")


def render_state_diagram(change: dict[str, Any], console: Console | None = None, context: int = 3) -> None:
    """Render state flow diagram showing context around current state.

    Shows: ... → [past] → [CURRENT] → [future] → ...
    Current state highlighted boldly. Progress shown at top.
    """
    from rich.markup import escape

    out = console or Console()
    c = get_client()

    state_flow_id = change.get("state_flow_id")
    if not state_flow_id:
        return

    try:
        flow = get_state_flow(state_flow_id, client=c)
    except Exception:
        return

    state_list = flow.get("state_list", [])
    if not state_list:
        return

    current = change.get("status")
    traversal = set(change.get("state_traversal") or [])

    cur_idx = next((i for i, s in enumerate(state_list) if s["id"] == current), None)
    if cur_idx is None:
        return

    total = len(state_list)
    done = len(traversal)
    pct = round((done / total) * 100) if total > 0 else 0

    lo = max(0, cur_idx - context)
    hi = min(total - 1, cur_idx + context)
    window = state_list[lo : hi + 1]

    parts: list[str] = []
    if lo > 0:
        parts.append("[dim]…[/]")

    for i, state in enumerate(window):
        sid, name = state["id"], escape(state["value"])
        state_abs_idx = lo + i
        visited = sid in traversal
        is_current = state_abs_idx == cur_idx

        if is_current:
            parts.append(f"[bold yellow on blue] ▶ {name} [/]")
        elif visited:
            parts.append(f"[green]✓ {name}[/]")
        else:
            parts.append(f"[dim]{name}[/]")

    if hi < total - 1:
        parts.append("[dim]…[/]")

    sep = " [dim]→[/] "
    flow_line = sep.join(parts)

    out.print(f"[bold cyan]state flow:[/] [bold]{pct}%[/] complete  [dim]({done}/{total})[/]")
    out.print(flow_line)
    if lo > 0 or hi < total - 1:
        out.print(f"[dim]  use `fsv changes state {change.get('human_display_id')}` for full flow[/]")


def _try_get_approvals(change_id: int, c: Client) -> list[dict[str, Any]]:
    try:
        return c.int_get(f"changes/{change_id}/approvals").get("approvals", [])
    except Exception:
        return []
