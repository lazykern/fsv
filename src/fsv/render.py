from __future__ import annotations

import csv
import html
import json
import re
import sys
from typing import Any

from rich.console import Console
from rich.markup import escape
from rich.table import Table

from fsv import schema as schema_mod
from fsv.resources import Resource, format_id

console = Console()
err = Console(stderr=True, style="dim")

_CUSTOM_FIELD_CATEGORY_ORDER = ("choices", "text", "dates", "numbers", "booleans", "other")


def _custom_field_category(name: str, schema: dict[str, Any]) -> str:
    field = schema_mod.field(name, schema)
    ftype = str((field or {}).get("field_type") or "").casefold()
    if any(token in ftype for token in ("dropdown", "choice", "nested", "lookup")):
        return "choices"
    if any(token in ftype for token in ("text", "paragraph", "description")):
        return "text"
    if "date" in ftype or "time" in ftype:
        return "dates"
    if any(token in ftype for token in ("number", "decimal", "integer")):
        return "numbers"
    if any(token in ftype for token in ("checkbox", "boolean")):
        return "booleans"
    return "other"


def emit_json(obj: Any) -> None:
    json.dump(obj, sys.stdout, indent=2, default=str)
    sys.stdout.write("\n")


def strip_html(s: str | None) -> str:
    if not s:
        return ""
    return html.unescape(re.sub(r"<[^>]+>", " ", s)).strip()


def _resolve_status_name(x: dict[str, Any], sid: Any, schema: dict[str, Any]) -> str:
    return (
        x.get("status_name")
        or (x.get("change_status") or {}).get("name")
        or (x.get("ticket_status") or {}).get("name")
        or (x.get("problem_status") or {}).get("name")
        or schema_mod.choice_label("status", sid, schema)
    )


def list_rows(items: list[dict], resource: Resource, schema: dict[str, Any]) -> list[dict[str, Any]]:
    rows = []
    for x in items:
        sid = x.get("status")
        row = {
            "id": format_id(x, resource),
            "status": _resolve_status_name(x, sid, schema),
            "status_id": sid,
            "priority": schema_mod.PRIORITY.get(x.get("priority") or 0, "-"),
            "subject": x.get("subject") or "",
            "requester": ((x.get("requester") or {}).get("name") if isinstance(x.get("requester"), dict) else "") or "",
        }
        if resource.name == "tickets":
            row["type"] = x.get("type") or ""
        elif resource.name == "changes":
            row["type"] = schema_mod.choice_label("change_type", x.get("change_type"), schema) or ""
        rows.append(row)
    return rows


def emit_delimited(rows: list[dict[str, Any]], delimiter: str) -> None:
    if not rows:
        return
    writer = csv.DictWriter(sys.stdout, fieldnames=list(rows[0]), delimiter=delimiter)
    writer.writeheader()
    writer.writerows(rows)


def list_table(items: list[dict], resource: Resource, schema: dict[str, Any]) -> Table:
    t = Table(show_lines=False)
    t.add_column("ID", style="cyan", no_wrap=True)
    t.add_column("Status", style="green")
    if resource.name == "tickets":
        t.add_column("Type")
    elif resource.name == "changes":
        t.add_column("Type")
    t.add_column("Pri")
    t.add_column("Subject")
    t.add_column("Requester", style="dim")
    for x in items:
        sid = x.get("status")
        status = _resolve_status_name(x, sid, schema)
        row = [
            format_id(x, resource),
            f"{status} ({sid})",
        ]
        if resource.name == "tickets":
            row.append(x.get("type") or "-")
        elif resource.name == "changes":
            row.append(schema_mod.choice_label("change_type", x.get("change_type"), schema) or "-")
        row += [
            schema_mod.PRIORITY.get(x.get("priority") or 0, "-"),
            x.get("subject") or "",
            ((x.get("requester") or {}).get("name") if isinstance(x.get("requester"), dict) else "-") or "-",
        ]
        t.add_row(*row)
    return t


def _state_label(value: Any, schema: dict[str, Any]) -> str:
    if isinstance(value, dict):
        for key in ("name", "display_name", "value", "status_name"):
            if value.get(key):
                return str(value[key])
        value = value.get("id") or value.get("status")
    return schema_mod.choice_label("status", value, schema) or str(value)


def _change_flow(item: dict[str, Any], schema: dict[str, Any]) -> str:
    states = item.get("state_traversal") or []
    if not states:
        return ""
    current = item.get("status")
    status_name = item.get("status_name")
    parts = []
    for state in states:
        sid = state.get("id") if isinstance(state, dict) else state
        label = _state_label(state, schema)
        text = escape(label)
        if sid == current or label == status_name:
            text = f"[bold]{text}[/]"
        parts.append(text)
    return " → ".join(parts)


def _party_display(obj: Any, id_value: Any, fallback_name: Any = None) -> str:
    if isinstance(obj, dict):
        name = obj.get("name") or obj.get("email") or obj.get("value")
        obj_id = obj.get("id") or id_value
        if name and obj_id is not None:
            return f"{name} ({obj_id})"
        if name:
            return str(name)
    if fallback_name and id_value is not None:
        return f"{fallback_name} ({id_value})"
    if fallback_name:
        return str(fallback_name)
    return str(id_value or "-")


def _field_value_text(value: Any) -> str:
    if isinstance(value, list):
        return ", ".join(_field_value_text(v) for v in value) or "-"
    if isinstance(value, dict):
        for key in ("name", "value", "label", "email"):
            if value.get(key):
                return str(value[key])
        return json.dumps(value, ensure_ascii=False)
    text = str(value).strip()
    return text or "-"


def _requested_item_field_defs(field: dict[str, Any]) -> list[dict[str, Any]]:
    defs = [field]
    for nested in field.get("nested_fields") or []:
        defs.extend(_requested_item_field_defs(nested))
    return defs


def _requested_item_rows(item: dict[str, Any]) -> list[tuple[str, str]]:
    values = item.get("custom_fields") or {}
    item_meta = item.get("item") or {}
    defs = item_meta.get("custom_fields") or []
    rows: list[tuple[str, str]] = []
    seen: set[str] = set()

    def add_field(field: dict[str, Any]) -> None:
        name = field.get("name")
        label = field.get("label")
        if not name or not label or name in seen:
            return
        if name not in values:
            return
        value = values.get(name)
        if value in (None, "", [], False):
            return
        seen.add(name)
        rows.append((str(label), _field_value_text(value)))

    for field in defs:
        add_field(field)
        selected = values.get(field.get("name"))
        if selected and field.get("sections"):
            for section in field.get("sections") or []:
                if section.get("name") != selected:
                    continue
                for section_field in section.get("fields") or []:
                    for nested in _requested_item_field_defs(section_field):
                        add_field(nested)
                break
        else:
            for nested in field.get("nested_fields") or []:
                for child in _requested_item_field_defs(nested):
                    add_field(child)

    for name, value in values.items():
        if name in seen or value in (None, "", [], False):
            continue
        rows.append((name, _field_value_text(value)))
    return rows


def requested_items_panel(items: list[dict[str, Any]]) -> None:
    for idx, item in enumerate(items, start=1):
        item_meta = item.get("item") or {}
        item_name = item_meta.get("name") or item_meta.get("short_description") or f"requested item {idx}"
        console.print()
        console.rule(f"[dim]requested_item {idx}[/]  {escape(str(item_name))}")
        stage = ((item.get("stage") or {}).get("name") if isinstance(item.get("stage"), dict) else item.get("stage")) or "-"
        console.print(f"stage     {stage}")
        description = str(item_meta.get("description") or "").strip()
        if description:
            console.print()
            console.rule("[dim]requested_item description[/]")
            console.print(description)
        rows = _requested_item_rows(item)
        if rows:
            console.print()
            console.rule("[dim]requested_item fields[/]")
            for label, value in rows:
                console.print(f"[cyan]{escape(label)}[/]  {escape(value)}")


def detail_panel(item: dict, resource: Resource, schema: dict[str, Any]) -> None:
    console.rule(f"[bold cyan]{format_id(item, resource)}[/]  {item.get('subject','')}")
    sid = item.get("status")
    console.print(f"[green]status[/]    {item.get('status_name') or schema_mod.choice_label('status', sid, schema)}  (id={sid})")
    if resource.name == "changes":
        from fsv import state_flow as _state_flow
        _state_flow.render_compact(item, console)
        console.print(f"type      {schema_mod.choice_label('change_type', item.get('change_type'), schema)}")
        console.print(f"risk      {schema_mod.RISK.get(item.get('risk'), '-')}")
    if resource.name == "tickets":
        console.print(f"type      {item.get('type', '-')}")
        dept = item.get("department") or {}
        if dept.get("name"):
            console.print(f"department {dept['name']}")
    console.print(f"priority  {schema_mod.PRIORITY.get(item.get('priority'), '-')}")
    if item.get("impact") is not None:
        console.print(f"impact    {schema_mod.IMPACT.get(item.get('impact'), '-')}")
    if item.get("planned_start_date") or item.get("planned_end_date"):
        console.print(f"planned   {item.get('planned_start_date')}  →  {item.get('planned_end_date')}")
    requester_obj = item.get("requester")
    agent_obj = item.get("agent") or item.get("responder")
    agent_id = item.get("agent_id") or item.get("responder_id")
    group_obj = item.get("group")
    console.print(
        f"requester {_party_display(requester_obj, item.get('requester_id'))}  "
        f"agent {_party_display(agent_obj, agent_id, item.get('owner_name'))}  "
        f"group {_party_display(group_obj, item.get('group_id'), item.get('group_name'))}"
    )
    description = strip_html(item.get("description"))
    if description:
        console.print()
        console.rule("[dim]description[/]")
        console.print(description)
    cf = item.get("custom_fields") or {}
    if cf:
        console.print()
        console.rule("[dim]custom_fields[/]")
        grouped: dict[str, list[tuple[str, Any]]] = {}
        for k, v in sorted((k, v) for k, v in cf.items() if v not in (None, "", [], False)):
            label = (schema_mod.field(k, schema) or {}).get("label") or k
            grouped.setdefault(_custom_field_category(k, schema), []).append((label, v))
        first = True
        for category in _CUSTOM_FIELD_CATEGORY_ORDER:
            items = grouped.get(category)
            if not items:
                continue
            if not first:
                console.print()
            first = False
            console.print(f"  [bold]{category}[/]")
            for k, v in items:
                console.print(f"    [cyan]{k}[/]: {v}")
    pf = item.get("planning_fields") or {}
    if pf:
        console.print()
        console.rule("[dim]planning_fields[/]")
        for k, v in pf.items():
            if isinstance(v, dict) and v.get("description_text"):
                console.print(f"  [cyan]{k}[/]: {v['description_text'][:200]}")
