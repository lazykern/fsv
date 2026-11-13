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


NAME_VALUED_CHOICES = frozenset({"default_ticket_type"})


def name_valued_choice(field: dict[str, Any], raw: str) -> str | None:
    """Return the canonical choice name for fields whose list filter takes names.

    Saved ticket views filter type as ``ticket_type is_in ["Service Request"]``;
    a numeric choice ID is ignored by the tenant and silently returns every type.
    Resolve the user's label or ID to exactly one active runtime choice name.
    """
    if str(field.get("field_type") or "") not in NAME_VALUED_CHOICES:
        return None
    folded = raw.strip().casefold()
    found = []
    for choice in field.get("choices") or []:
        if not isinstance(choice, dict) or any(choice.get(k) for k in ("deleted", "is_deleted", "deactivated")):
            continue
        value = choice.get("value")
        ident = choice.get("id")
        if not isinstance(value, str) or not value.strip():
            continue
        if value.strip().casefold() == folded or (ident is not None and not isinstance(ident, bool) and str(ident) == raw.strip()):
            found.append(value)
    if len({v.casefold() for v in found}) != 1:
        raise ValueError(f"no unique runtime choice {raw!r} for {field.get('label') or field.get('name')}")
    return found[0]


def _scalar(value: Any) -> Any:
    if isinstance(value, dict) and set(value) <= {"id", "name"} and "id" in value:
        value = value["id"]
    if value is None or isinstance(value, (dict, list, bool, float)):
        raise ValueError("predicate value is not independently verifiable")
    return str(value)


_NUMBER_OPS = {"is", "is_not", "is_greater_than_or_equal", "is_less_than_or_equal"}


def _number_matches(row: dict[str, Any], name: str, op: str, value: Any) -> bool:
    """Captured custom_number wire: string value, inclusive comparisons."""
    from decimal import Decimal, InvalidOperation
    source = row.get("custom_fields")
    if not isinstance(source, dict) or name not in source:
        raise ValueError(f"predicate field unavailable in exact record: {name!r}")
    actual = source[name]
    if actual is None or isinstance(actual, bool) or not isinstance(value, str):
        raise ValueError(f"predicate cannot be independently verified for {name!r}")
    try:
        a, e = Decimal(str(actual)), Decimal(value)
    except InvalidOperation:
        raise ValueError(f"predicate cannot be independently verified for {name!r}") from None
    return {"is": a == e, "is_not": a != e, "is_greater_than_or_equal": a >= e,
            "is_less_than_or_equal": a <= e}[op]


def matches(row: dict[str, Any], clauses: list[dict[str, Any]]) -> bool:
    if not clauses:
        raise ValueError("empty predicate cannot authorize a filtered write")
    for clause in clauses:
        name, kind = clause.get("condition"), clause.get("type")
        op, values = clause.get("operator"), clause.get("value")
        if op in _NUMBER_OPS and kind == "custom_field" and isinstance(name, str):
            if not _number_matches(row, name, op, values):
                return False
            continue
        if not isinstance(name, str) or kind not in {"default", "custom_field"} or op not in {"is_in", "is_not_in"} or not isinstance(values, list) or not values:
            raise ValueError(f"predicate cannot be independently verified for {name!r}")
        source = row.get("custom_fields") if kind == "custom_field" else row
        if kind == "default" and name == "ticket_type":
            # Exact native ticket reads expose ``type``; reuse the strict
            # agreement check rather than trusting either key alone.
            from fsv.reviewed_write import ReviewedWriteError
            from fsv.ticket_kind import exact_ticket_type
            try:
                actual = exact_ticket_type(row)
            except ReviewedWriteError:
                raise ValueError(f"predicate field unavailable in exact record: {name!r}") from None
            expected = [str(v).casefold() for v in values if isinstance(v, str)]
            if len(expected) != len(values):
                raise ValueError(f"predicate cannot be independently verified for {name!r}")
            if (actual in expected) != (op == "is_in"):
                return False
            continue
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
