"""Pre-send change lifecycle checks from the runtime state flow.

The state flow (``change_management/<id>?include=state_info``) carries:

* ``transitions``: ``{"<from state>": [{"to_state": int, "entry_criteria": [...]}]}``;
  a criterion is ``{"name", "operator", "value"}`` or
  ``{"fields": {"<field id>": {"name", "operator", "value", "custom_field"?}}}``;
* ``state_properties.field_lock_config``: ``{"<state>": [{"<field id>": "status" |
  "department" | "customFields.<name>" | ...}]}``: fields locked while the
  change is in that state.

The server rejects disallowed edges, unmet criteria and locked fields with a
bare HTTP 400, so the dry-run checks them first. Only criteria fsv can
evaluate from the change record refuse; the rest are reported as warnings.
"""
from __future__ import annotations

from typing import Any, Callable, Mapping

Label = Callable[[str], str]


def _plain(name: str) -> str:
    return name.removeprefix("customFields.")


def state_names(flow: Mapping[str, Any]) -> dict[str, str]:
    rows = flow.get("state_list")
    return {str(s.get("id")): str(s.get("value")) for s in rows if isinstance(s, dict)} if isinstance(rows, list) else {}


def criteria(transition: Mapping[str, Any]) -> list[dict[str, Any]]:
    """Flatten both native entry-criteria shapes into ``{name, operator, value}``."""
    out: list[dict[str, Any]] = []
    raw = transition.get("entry_criteria")
    for entry in raw if isinstance(raw, list) else []:
        if not isinstance(entry, dict):
            continue
        if isinstance(entry.get("fields"), dict):
            out.extend(c for c in entry["fields"].values() if isinstance(c, dict) and c.get("name"))
        elif entry.get("name"):
            out.append(entry)
    return out


def locks(flow: Mapping[str, Any], state: Any) -> list[str]:
    props = flow.get("state_properties")
    config = props.get("field_lock_config") if isinstance(props, dict) else None
    rows = config.get(str(state)) if isinstance(config, dict) else None
    return [str(v) for row in rows if isinstance(row, dict) for v in row.values()] if isinstance(rows, list) else []


def describe(criterion: Mapping[str, Any], label: Label = _plain) -> str:
    name, op, value = str(criterion.get("name")), criterion.get("operator"), criterion.get("value")
    if op == "is_not" and value in ("", None):
        return f"{label(name)} must be set"
    return f"{label(name)} {str(op).replace('_', ' ')} {value!r}"


def _value(name: str, record: Mapping[str, Any], body: Mapping[str, Any]) -> tuple[bool, Any]:
    """(known, value) of a criterion field after the write."""
    if name.startswith("customFields."):
        key = _plain(name)
        for source in (body.get("custom_fields"), record.get("custom_fields")):
            if isinstance(source, dict) and key in source:
                return True, source[key]
        return False, None
    for source in (body, record):
        if name in source:
            return True, source[name]
    return False, None


def _met(criterion: Mapping[str, Any], record: Mapping[str, Any], body: Mapping[str, Any]) -> bool | None:
    known, actual = _value(str(criterion.get("name")), record, body)
    if not known:
        return None
    op, value = criterion.get("operator"), criterion.get("value")
    empty = actual is None or actual == "" or actual == []
    if op == "is_not" and value in ("", None):
        return not empty
    if op == "is":
        return not empty and str(actual) == str(value)
    if op == "is_not":
        return str(actual) != str(value)
    return None


def _lock_names(body: Mapping[str, Any]) -> dict[str, str]:
    """Lock-config names for each sent key (``department_id`` -> ``department``)."""
    out = {}
    for key in body:
        if key == "custom_fields" and isinstance(body[key], dict):
            out.update({f"customFields.{k}": f"customFields.{k}" for k in body[key]})
        elif key not in {"br_validation_excludes"}:
            out[key.removesuffix("_id")] = key
            out[key] = key
    return out


def precheck(flow: Mapping[str, Any], record: Mapping[str, Any], body: Mapping[str, Any],
             label: Label = _plain) -> tuple[list[str], list[str]]:
    """(errors, warnings) for sending ``body`` to a change in ``record``'s state."""
    errors: list[str] = []
    warnings: list[str] = []
    names = state_names(flow)
    current = record.get("status")
    here = names.get(str(current), str(current))
    locked = set(locks(flow, current))
    sent = _lock_names(body)
    hit = sorted({label(_plain(n)) for n in sent if n in locked})
    if hit:
        errors.append(f"locked in lifecycle state {here}: {', '.join(hit)} (field lock); the server rejects this write")
    if "status" not in body or str(body["status"]) == str(current):
        return errors, warnings
    target = body["status"]
    there = names.get(str(target), str(target))
    transitions = flow.get("transitions")
    if not isinstance(transitions, dict) or str(current) not in transitions:
        warnings.append(f"lifecycle transitions from {here} unavailable; the edge {here} → {there} was not pre-checked")
        return errors, warnings
    edges = [t for t in transitions[str(current)] or [] if isinstance(t, dict)]
    match = [t for t in edges if str(t.get("to_state")) == str(target)]
    if not match:
        allowed = ", ".join(names.get(str(t.get("to_state")), str(t.get("to_state"))) for t in edges) or "none"
        errors.append(f"{here} → {there} is not an allowed lifecycle transition (allowed from {here}: {allowed})")
        return errors, warnings
    for criterion in criteria(match[0]):
        met = _met(criterion, record, body)
        if met is False:
            errors.append(f"entry criterion for {here} → {there} not met: {describe(criterion, label)}"
                          + ("; supply it with --set" if criterion.get("operator") == "is_not" else ""))
        elif met is None:
            warnings.append(f"entry criterion for {here} → {there} not pre-checked: {describe(criterion, label)}")
    return errors, warnings


def outgoing(flow: Mapping[str, Any], state: Any, label: Label = _plain) -> list[tuple[str, list[str]]]:
    """[(to-state name, [criterion descriptions])] for display."""
    names = state_names(flow)
    transitions = flow.get("transitions")
    edges = transitions.get(str(state)) if isinstance(transitions, dict) else None
    return [(names.get(str(t.get("to_state")), str(t.get("to_state"))), [describe(c, label) for c in criteria(t)])
            for t in edges or [] if isinstance(t, dict)]
