"""Independent, deliberately narrow predicate proofs for server-filtered rows."""
from __future__ import annotations

from typing import Any


def require_filterable_field(field: dict[str, Any]) -> None:
    """Form metadata is not evidence that a free-text filter operator exists.

    In particular, default_subject has no filter operators in runtime form
    metadata. Sending the choice operator for it can silently broaden a list.
    Keep unsupported types closed rather than guessing a text wire.
    """
    kind = str(field.get("field_type") or "")
    if not (field.get("choices") or "dropdown" in kind or kind.endswith("number") or kind in {
            "default_status", "default_priority", "default_requester", "default_agent",
            "default_group", "default_department"} or field.get("name") in {
            "requester", "agent", "group", "department"}):
        raise ValueError(f"filter operator unavailable in runtime metadata for {field.get('name')!r}; "
                         "refusing an unproven filter (use explicit IDs)")


def _scalar(value: Any) -> Any:
    if isinstance(value, dict) and set(value) <= {"id", "name"} and "id" in value:
        value = value["id"]
    if value is None or isinstance(value, (dict, list, bool, float)):
        raise ValueError("predicate value is not independently verifiable")
    return str(value)


def matches(row: dict[str, Any], clauses: list[dict[str, Any]]) -> bool:
    if not clauses:
        raise ValueError("empty predicate cannot authorize a filtered write")
    for clause in clauses:
        name, kind = clause.get("condition"), clause.get("type")
        op, values = clause.get("operator"), clause.get("value")
        if not isinstance(name, str) or kind not in {"default", "custom_field"} or op not in {"is_in", "is_not_in"} or not isinstance(values, list) or not values:
            raise ValueError(f"predicate cannot be independently verified for {name!r}")
        source = row.get("custom_fields") if kind == "custom_field" else row
        if not isinstance(source, dict) or name not in source:
            raise ValueError(f"predicate field unavailable in exact record: {name!r}")
        actual = _scalar(source[name])
        expected = [_scalar(v) for v in values]
        if (actual in expected) != (op == "is_in"):
            return False
    return True


def contradictions(rows: list[dict[str, Any]], clauses: list[dict[str, Any]]) -> list[Any]:
    bad = []
    for row in rows:
        try:
            if not matches(row, clauses):
                bad.append(row.get("id", "unknown"))
        except ValueError:
            continue  # Reads may display fields whose predicate cannot be proven.
    return bad
