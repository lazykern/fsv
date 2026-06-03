"""Query-hash building for Freshservice filter API — shared by CLI and TUI."""
from __future__ import annotations

import json
import re
from typing import Any
from urllib.parse import unquote


def _strip_quotes(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in ('"', "'"):
        return value[1:-1]
    return value


def _parse_where(expr: str) -> tuple[str, str, str]:
    m = re.match(r"^(.+?)(>=|<=|!=|~=|=|>|<)(.*)$", expr.strip())
    if not m:
        raise ValueError(f"invalid filter {expr!r}; expected FIELD=VALUE")
    field, op, value = m.groups()
    return field.strip(), op, _strip_quotes(value)


def _field_api_type(f: dict[str, Any]) -> str:
    return "default" if f.get("default_field") else "custom_field"


def _field_matches(f: dict[str, Any], text: str) -> bool:
    needle = text.casefold()
    return needle in str(f.get("name") or "").casefold() or needle in str(f.get("label") or "").casefold()


def _schema_field_matches(sch: dict[str, Any], text: str, scope: str | None = None) -> list[dict[str, Any]]:
    fields = sch["fields"]
    if scope == "default":
        fields = [f for f in fields if f.get("default_field")]
    elif scope == "custom":
        fields = [f for f in fields if not f.get("default_field")]
    return [f for f in fields if _field_matches(f, text)]


def _find_schema_field(sch: dict[str, Any], text: str) -> dict[str, Any] | None:
    scope = None
    raw = text
    if ":" in text:
        maybe_scope, rest = text.split(":", 1)
        if maybe_scope in ("default", "custom"):
            scope, raw = maybe_scope, rest
    folded = raw.casefold()
    matches = _schema_field_matches(sch, raw, scope)
    exact = [
        f for f in matches
        if str(f.get("name") or "").casefold() == folded or str(f.get("label") or "").casefold() == folded
    ]
    if len(exact) == 1:
        return exact[0]
    if len(exact) > 1:
        by_name = [f for f in exact if str(f.get("name") or "").casefold() == folded]
        if len(by_name) == 1:
            return by_name[0]
        raise ValueError("ambiguous field " + repr(text) + ": " + ", ".join(str(f.get("name")) for f in exact[:5]))
    if len(matches) == 1:
        return matches[0]
    if len(matches) > 1:
        raise ValueError("ambiguous field " + repr(text) + ": " + ", ".join(str(f.get("name")) for f in matches[:5]))
    return None


def _pick_lookup(kind: str, value: str, results: list[dict[str, Any]], exact_keys: tuple[str, ...]) -> dict[str, Any]:
    if not results:
        raise ValueError(f"no {kind} match: {value}")
    folded = value.casefold()
    for item in results:
        if any(str(item.get(k) or "").casefold() == folded for k in exact_keys):
            return item
    if len(results) == 1:
        return results[0]
    choices = ", ".join(str(x.get("details") or x.get("value") or x.get("id")) for x in results[:5])
    raise ValueError(f"multiple {kind} matches for {value!r}: {choices}")


def _resolve_requester(c: Any, value: str) -> dict[str, Any]:
    if value.isdigit():
        return {"id": int(value), "name": value}
    item = _pick_lookup(
        "requester",
        value,
        c.autocomplete("requesters", value, {"all_users": "true"}),
        ("value", "email", "details"),
    )
    return {"id": item["id"], "name": item.get("value") or str(item["id"])}


def _resolve_agent(c: Any, value: str) -> str:
    if value.isdigit():
        return value
    item = _pick_lookup("agent", value, c.autocomplete("agents", value), ("value", "id", "email", "details"))
    user_id = item.get("user_id")
    if user_id is None:
        raise ValueError(f"agent match has no user_id: {value}")
    return str(user_id)


def _resolve_group(c: Any, value: str) -> str:
    if value.isdigit():
        return value
    data = c.int_get("bootstrap/agents_groups")
    folded = value.casefold()
    groups = [
        g for g in data.get("groups", [])
        if folded in str(g.get("name") or "").casefold() or str(g.get("id") or "") == value
    ]
    item = _pick_lookup("group", value, groups, ("name", "id"))
    group_id = item.get("id")
    if group_id is None:
        raise ValueError(f"group match has no id: {value}")
    return str(group_id)


def _choice_value(field: dict[str, Any], raw: str, type_: str) -> Any:
    choices = field.get("choices") or []
    if not choices:
        return int(raw) if raw.isdigit() else raw
    folded = raw.casefold()
    for c in choices:
        if str(c.get("id") or "") == raw:
            return c.get("value", c.get("id")) if type_ == "custom_field" else c.get("id")
        labels = (c.get("value"), c.get("name"), c.get("label"), c.get("requester_display_value"))
        if any(str(x or "").casefold() == folded for x in labels):
            return c.get("value", c.get("id")) if type_ == "custom_field" else c.get("id", c.get("value", raw))
    shown = ", ".join(str(c.get("value") or c.get("name") or c.get("id")) for c in choices[:12])
    raise ValueError(f"no choice {raw!r} for {field.get('label') or field.get('name')}; options: {shown}")


def _where_operator(op: str) -> str:
    if op == "=":
        return "is_in"
    if op == "!=":
        return "is_not_in"
    if op in (">", ">=", "<", "<="):
        raise ValueError(f"operator {op!r} not supported for this field; use = or !=")
    if op == "~=":
        raise ValueError("operator ~= (contains) not supported; use = or !=")
    return "is_in"


def _query_item(
    condition: str,
    operator: str,
    value: Any,
    type_: str,
    explain: list[dict[str, Any]],
    field: str,
    raw: str,
) -> dict[str, Any]:
    explain.append({"field": field, "condition": condition, "operator": operator, "type": type_, "value": raw})
    return {"value": value, "condition": condition, "operator": operator, "type": type_}


def _resolve_where(
    c: Any, res: Any, sch: dict[str, Any], expr: str, explain: list[dict[str, Any]]
) -> dict[str, Any]:
    field_text, op, raw = _parse_where(expr)
    pseudo_dates = {"created_at", "updated_at", "due_by", "planned_start_date", "planned_end_date"}
    if field_text in pseudo_dates:
        if op in (">", ">="):
            value: Any = {"from": raw}
        elif op in ("<", "<="):
            value = {"to": raw}
        elif op in ("=", "!="):
            value = {"from": raw, "to": raw}
        else:
            raise ValueError(f"unsupported date operator {op!r}; use = != >= <= > <")
        return _query_item(field_text, "custom_date", value, "default", explain, field_text, raw)
    if field_text.endswith("_id"):
        if op in (">", "<", ">=", "<=", "~="):
            raise ValueError(f"operator {op!r} not supported on ID fields; use = or !=")
        operator = "is_in" if op == "=" else "is_not_in"
        val: Any = int(raw) if raw.isdigit() else raw
        return _query_item(field_text, operator, [val], "default", explain, field_text, raw)

    f = _find_schema_field(sch, field_text)
    if not f:
        raise ValueError(f"field not found: {field_text!r}")
    name = f.get("name") or field_text
    type_ = _field_api_type(f)
    ftype = str(f.get("field_type") or "")
    operator = _where_operator(op)

    if name == "requester":
        req = _resolve_requester(c, raw)
        return _query_item(
            "requester_id", operator,
            [{"id": req["id"], "name": req["name"]}],
            "default", explain, name, f"{req['name']} ({req['id']})",
        )
    if name == "agent":
        agent_id = _resolve_agent(c, raw)
        condition = "responder_id" if res.name == "tickets" else "agent_id"
        return _query_item(condition, operator, [agent_id], "default", explain, name, agent_id)
    if name == "group":
        val = _choice_value(f, raw, "default")
        return _query_item("group_id", operator, [str(val)], "default", explain, name, str(val))

    if f.get("choices") or "dropdown" in ftype or ftype in ("default_status", "default_priority"):
        val = _choice_value(f, raw, type_)
        return _query_item(name, operator, [val], type_, explain, name, str(val))
    val = int(raw) if raw.isdigit() and ftype.endswith("number") else raw
    return _query_item(name, operator, [val], type_, explain, name, raw)


def _merge_date_ranges(query: list[dict[str, Any]]) -> list[dict[str, Any]]:
    ranges: dict[tuple[str, str], dict[str, Any]] = {}
    out: list[dict[str, Any]] = []
    for item in query:
        value = item.get("value")
        if isinstance(value, dict) and ("from" in value or "to" in value):
            key = (item.get("condition"), item.get("type"))
            if key not in ranges:
                merged = dict(item)
                merged["value"] = {}
                ranges[key] = merged
                out.append(merged)
            ranges[key]["value"].update(value)
        else:
            out.append(item)
    return out


def _load_query_hash(raw: str) -> list[dict[str, Any]]:
    try:
        loaded = json.loads(unquote(raw))
    except json.JSONDecodeError as e:
        raise ValueError(f"invalid query-hash JSON: {e}")
    if not isinstance(loaded, list):
        raise ValueError("query-hash must be a JSON array")
    return loaded


def build_query_hash_from_schema(
    c: Any,
    res: Any,
    sch: dict[str, Any],
    where: list[str],
) -> str | None:
    """Build query_hash JSON string from FIELD=VALUE expressions using a pre-loaded schema.

    Raises ValueError with a user-readable message on invalid input.
    Returns None when where is empty.
    """
    if not where:
        return None
    query: list[dict[str, Any]] = []
    explain: list[dict[str, Any]] = []
    for expr in where:
        query.append(_resolve_where(c, res, sch, expr, explain))
    query = _merge_date_ranges(query)
    if not query:
        return None
    return json.dumps(query, separators=(",", ":"))


def build_query_hash(
    c: Any,
    res: Any,
    where: list[str],
    raw: str | None = None,
    or_grouping: bool = False,
) -> tuple[str | None, list[dict[str, Any]], list[dict[str, Any]]]:
    """Build query_hash JSON string, loading schema automatically.

    Raises ValueError on invalid expressions.
    Returns (query_hash_json, query_list, explain_list).
    """
    from fsv import schema as schema_mod
    query: list[dict[str, Any]] = []
    explain: list[dict[str, Any]] = []
    if raw:
        query.extend(_load_query_hash(raw))
        explain.append({"field": "query_hash", "condition": "raw", "operator": "raw", "type": "raw", "value": raw})
    if where:
        sch = schema_mod.load(res, c)
        for expr in where:
            query.append(_resolve_where(c, res, sch, expr, explain))
    query = _merge_date_ranges(query)
    if not query:
        return None, query, explain
    if or_grouping:
        payload = json.dumps({"any": query}, separators=(",", ":"))
        return payload, query, explain
    return json.dumps(query, separators=(",", ":")), query, explain
