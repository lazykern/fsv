"""Active runtime task choices, shared by write resolution and presentation."""
from __future__ import annotations
from typing import Any


def active_choices(fields: list[dict[str, Any]]) -> list[tuple[int, str]]:
    statuses = [f for f in fields if isinstance(f, dict) and f.get("name") == "status"]
    if len(statuses) != 1:
        raise ValueError("exactly one runtime task-status field is required")
    result = []
    for choice in statuses[0].get("choices", []):
        if not isinstance(choice, dict):
            raise ValueError("invalid task-status choice")
        if any(choice.get(k) for k in ("deleted", "is_deleted", "deleted_at", "deactivated")):
            continue
        ident = choice.get("id")
        label = choice.get("value") or choice.get("label") or choice.get("name")
        if isinstance(ident, bool) or not str(ident).isdigit() or int(str(ident)) <= 0 or not isinstance(label, str) or not label.strip():
            raise ValueError("invalid active task-status identity")
        result.append((int(str(ident)), label.strip()))
    if len({i for i, _ in result}) != len(result) or len({v.casefold() for _, v in result}) != len(result):
        raise ValueError("ambiguous task-status choices")
    return result


def resolve(value: str, fields: list[dict[str, Any]]) -> tuple[int, str]:
    key = str(value).strip().casefold()
    matches = [(i, label) for i, label in active_choices(fields) if key in (str(i), label.casefold())]
    if len(matches) != 1:
        raise ValueError("status must match one active runtime task-status ID or label")
    return matches[0]


def fields_from(client: Any) -> list[dict[str, Any]]:
    data = client.int_get("change_task_fields", {"include_deleted_choices": "true"})
    for key in ("change_task_fields", "task_fields", "fields"):
        if isinstance(data, dict) and isinstance(data.get(key), list):
            return data[key]
    raise ValueError("unreadable runtime task-status metadata")


def display(value: Any, fields: list[dict[str, Any]] | None = None) -> str:
    if isinstance(value, dict):
        return str(value.get("name") or value.get("label") or value.get("value") or value.get("id"))
    try:
        return resolve(str(value), fields or [])[1]
    except ValueError:
        return f"{value} (unknown)"
