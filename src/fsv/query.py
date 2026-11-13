"""Query-hash building for Freshservice filter API — shared by CLI and TUI."""
from __future__ import annotations

import json
import re
from typing import Any
from urllib.parse import unquote


def _isdigit(text: str) -> bool:
    """ASCII-only digits (str.isdigit accepts e.g. superscripts and Thai digits)."""
    return text.isascii() and text.isdigit()


def _strip_quotes(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in ('"', "'"):
        return value[1:-1]
    return value


def _parse_where(expr: str) -> tuple[str, str, str]:
    if not isinstance(expr, str) or any(ord(c) < 32 for c in expr):
        raise ValueError("invalid filter; controls and multiline expressions are not supported")
    m = re.fullmatch(r"([^<>=!~]+?)(>=|<=|!=|~=|=|>|<)(.*)", expr.strip())
    if not m:
        raise ValueError(f"invalid filter {expr!r}; expected FIELD followed by = != >= <= > < or ~=")
    field, op, value = m.groups()
    field, value = field.strip(), value.strip()
    if not field or not value:
        raise ValueError("invalid filter; field and value are required (quote literal empty text)")
    if value[0] in ("\"", "'"):
        if len(value) < 2 or value[-1] != value[0]:
            raise ValueError("invalid filter; unterminated quoted value")
    elif value[0] in "<>=!~":
        raise ValueError("invalid filter operator sequence; quote a literal value starting with an operator")
    return field, op, _strip_quotes(value)


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
    if _isdigit(value):
        return {"id": int(value), "name": value}
    item = _pick_lookup(
        "requester",
        value,
        c.autocomplete("requesters", value, {"all_users": "true"}),
        ("value", "email", "details"),
    )
    return {"id": item["id"], "name": item.get("value") or str(item["id"])}


def _resolve_agent(c: Any, value: str) -> str:
    if _isdigit(value):
        return value
    from fsv.agent_lookup import exact_agent_id, is_email
    if is_email(value):
        return str(exact_agent_id(c, value))
    item = _pick_lookup("agent", value, c.autocomplete("agents", value), ("value", "id", "email", "details"))
    user_id = item.get("user_id")
    if user_id is None:
        raise ValueError(f"agent match has no user_id: {value}")
    return str(user_id)


def _resolve_group(c: Any, value: str) -> str:
    if _isdigit(value):
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
        return int(raw) if _isdigit(raw) else raw
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
    raise ValueError(f"unknown filter operator {op!r}; no equality fallback")


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


# ---------------------------------------------------------------------------
# Captured list-filter wires (dates, numbers, multi-select, text).
#
# * Dates: the filter panel and the advanced builder send ``is_in_the_range``
#   (or ``is_greater_than``) with ``{"from": D, "to": D}``. The server reads
#   whole days with BOTH ends inclusive; a one-sided ``{from}`` or ``{to}``
#   answers HTTP 500, so a range must carry both ends. There is no strict
#   ``>``/``<`` operator: ``> D`` is sent as ``from D+1`` and ``< D`` as
#   ``to D-1`` (day-granular shift, shown in --debug). ``on`` + a relative
#   token and ``in_the_last``/``in_the_next`` + a period exist only in the
#   advanced builder (tickets, changes). The old ``custom_date`` operator is
#   not a server operator (206 search_error / silently empty on problems).
# * Numbers (custom_number): ``is``/``is_not``/``is_greater_than_or_equal``/
#   ``is_less_than_or_equal`` with a STRING value, advanced builder only.
# * Multi-select dropdowns: ``is_in`` with the INTEGER choice id, advanced.
# * Text (subject, description, custom text/paragraph): no list filter exists
#   in any list UI; refuse rather than inventing one.
# ---------------------------------------------------------------------------
import datetime as _dt

DATE_FIELDS = frozenset({"created_at", "updated_at", "due_by", "planned_start_date", "planned_end_date",
                         "fr_due_by", "closed_at", "resolved_at"})
ON_TOKENS = ("today", "yesterday", "tomorrow", "week", "month", "year")
RANGE_TOKENS = ("five_minutes", "one_week", "one_month", "two_months", "six_months", "one_year")
ADVANCED_MODULES = frozenset({"tickets", "changes"})
# Date conditions offered by the advanced builder, keyed by fsv's field name.
# fsv reads no filter-field metadata that names them: the builder's field list
# ships in the web bundle (created_at, updated_at, planned_start_date,
# planned_end_date, due_by_date, fr_due_by_date, closed_date, resolved_date),
# and neither the form fields nor the saved views carry the advanced name, so
# this table is the source. Record fields keep their names in the basic
# query_hash panel; the due, first-response-due, closed and resolved dates
# differ.
ADVANCED_DATE_CONDITIONS = {"created_at": "created_at", "updated_at": "updated_at",
                            "due_by": "due_by_date", "fr_due_by": "fr_due_by_date",
                            "closed_at": "closed_date", "resolved_at": "resolved_date",
                            "planned_start_date": "planned_start_date", "planned_end_date": "planned_end_date"}
# Advanced dates that exist only in the ticket builder.
TICKET_ONLY_DATES = frozenset({"fr_due_by", "closed_at", "resolved_at"})
# Dates whose range goes through the advanced builder under its advanced
# name: the basic ``due_by`` range answers 206 + meta.search_error (unfiltered
# rows), and the closed/resolved/first-response dates have no basic panel.
ADVANCED_RANGE_DATES = frozenset({"due_by", "fr_due_by", "closed_at", "resolved_at"})
# Problems: only the created range is proven; other shapes returned 0 rows silently.
PROBLEM_DATE_FIELDS = frozenset({"created_at"})
NUMBER_OPERATORS = {"=": "is", "!=": "is_not", ">=": "is_greater_than_or_equal", "<=": "is_less_than_or_equal"}
TEXT_FIELD_TYPES = frozenset({"default_subject", "default_description", "default_text", "custom_text",
                              "custom_paragraph", "custom_url", "default_email"})
_ADV = "_advanced"  # internal marker, never sent
_ISO_DAY = re.compile(r"\d{4}-\d{2}-\d{2}")


def _advanced(item: dict[str, Any]) -> dict[str, Any]:
    item[_ADV] = True
    return item


def is_text_field(f: dict[str, Any]) -> bool:
    ftype = str(f.get("field_type") or "")
    return ftype in TEXT_FIELD_TYPES or str(f.get("name") or "") in {"subject", "description"}


def text_filter_refusal(res_name: str, f: dict[str, Any]) -> ValueError:
    label = f.get("label") or f.get("name")
    return ValueError(f"no list filter exists for text field {label!r}: the web's filter panels and builders offer "
                      f"no subject/description/text condition, so nothing was sent; use `fsv {res_name} search TEXT` "
                      "for a keyword search")


def date_condition(res_name: str, field: str, op: str, raw: str, explain: list[dict[str, Any]],
                   shown: str) -> dict[str, Any]:
    """One captured date condition (see the module notes). Raises ValueError."""
    text = raw.strip()
    low = text.casefold()
    for prefix, operator, tokens in (("on:", "on", ON_TOKENS), ("last:", "in_the_last", RANGE_TOKENS),
                                     ("next:", "in_the_next", RANGE_TOKENS)):
        if not low.startswith(prefix):
            continue
        token = "_".join(low[len(prefix):].split())
        if op != "=":
            raise ValueError(f"relative date {prefix}PERIOD takes only =")
        if token not in tokens:
            raise ValueError(f"unknown {operator} period {token!r}; use one of: {', '.join(tokens)}")
        if res_name not in ADVANCED_MODULES:
            raise ValueError(f"{operator} date filters exist only in the tickets/changes advanced filter; "
                             f"{res_name} has none, nothing was sent")
        return _advanced(_query_item(advanced_date_condition(res_name, field), operator, token, "default",
                                     explain, shown, text))
    if not _ISO_DAY.fullmatch(text):
        raise ValueError("date filters take a whole day YYYY-MM-DD (the server ignores time of day) or a "
                         "relative on:/last:/next: period; nothing was sent")
    try:
        day = _dt.date.fromisoformat(text)
    except ValueError:
        raise ValueError(f"invalid date {text!r}") from None
    if res_name == "problems" and field not in PROBLEM_DATE_FIELDS:
        raise ValueError(f"{field} date filters are unproven on problems (the list returned 0 rows silently); "
                         "only created_at ranges are supported there, nothing was sent")
    note = ""
    if op == "=":
        value: Any = {"from": text, "to": text}
    elif op == ">=":
        value = {"from": text}
    elif op == "<=":
        value = {"to": text}
    elif op == ">":
        shifted = (day + _dt.timedelta(days=1)).isoformat()
        value, note = {"from": shifted}, f" (no strict operator exists: sent as from {shifted}, whole days)"
    elif op == "<":
        shifted = (day - _dt.timedelta(days=1)).isoformat()
        value, note = {"to": shifted}, f" (no strict operator exists: sent as to {shifted}, whole days)"
    else:
        raise ValueError(f"date operator {op!r} is not supported; use = > >= < <= (whole days)")
    if field in ADVANCED_RANGE_DATES:
        if res_name not in ADVANCED_MODULES:
            raise ValueError(f"{field} date ranges exist only in the tickets/changes advanced filter; "
                             f"{res_name} has none, nothing was sent")
        return _advanced(_query_item(advanced_date_condition(res_name, field), "is_in_the_range", value, "default",
                                     explain, shown, text + note))
    return _query_item(field, "is_in_the_range", value, "default", explain, shown, text + note)


def advanced_date_condition(res_name: str, field: str) -> str:
    """The advanced builder's condition name for an fsv date field."""
    cond = ADVANCED_DATE_CONDITIONS.get(field)
    if cond is None or (field in TICKET_ONLY_DATES and res_name != "tickets"):
        raise ValueError(f"{field} is not a date in the {res_name} advanced filter builder; nothing was sent")
    return cond


def _int_choice_id(value: Any) -> int | None:
    if isinstance(value, int) and not isinstance(value, bool) and value > 0:
        return value
    if isinstance(value, str) and _isdigit(value) and int(value) > 0:
        return int(value)
    return None


def typed_condition(res_name: str, f: dict[str, Any], op: str, raw: str, explain: list[dict[str, Any]],
                    shown: str) -> dict[str, Any] | None:
    """Text refusal, number and multi-select wires; None when not one of these."""
    ftype = str(f.get("field_type") or "")
    name = str(f.get("name") or shown)
    label = f.get("label") or name
    if is_text_field(f):
        raise text_filter_refusal(res_name, f)
    if ftype == "custom_number":
        if op not in NUMBER_OPERATORS:
            raise ValueError(f"operator {op!r} not supported for number field {label!r}; use = != >= <= "
                             "(the filter builder has no strict > or <)")
        value = raw.strip()
        if not re.fullmatch(r"-?[0-9]+", value):
            raise ValueError(f"number filter {label!r} takes an integer value")
        if res_name not in ADVANCED_MODULES:
            raise ValueError(f"number filters exist only in the tickets/changes advanced filter; nothing was sent")
        return _advanced(_query_item(name, NUMBER_OPERATORS[op], value, "custom_field", explain, shown, value))
    if "multi_select" in ftype:
        if op not in ("=", "!="):
            raise ValueError(f"operator {op!r} not supported for multi-select {label!r}; use = or !=")
        if res_name not in ADVANCED_MODULES:
            raise ValueError("multi-select filters exist only in the tickets/changes advanced filter; nothing was sent")
        folded = raw.strip().casefold()
        found = []
        for ch in f.get("choices") or []:
            if not isinstance(ch, dict):
                continue
            labels = (ch.get("value"), ch.get("name"), ch.get("label"))
            if str(ch.get("id")) == raw.strip() or any(str(x or "").strip().casefold() == folded for x in labels):
                found.append(ch)
        ids = {_int_choice_id(ch.get("id")) for ch in found}
        if len(found) < 1 or len(ids) != 1 or None in ids:
            shown_choices = ", ".join(str(ch.get("value") or ch.get("id")) for ch in (f.get("choices") or [])[:12]
                                      if isinstance(ch, dict))
            raise ValueError(f"no unique integer choice id for {raw!r} in {label!r} runtime metadata; "
                             f"options: {shown_choices}")
        cid = ids.pop()
        return _advanced(_query_item(name, "is_in" if op == "=" else "is_not_in", [cid], "custom_field",
                                     explain, shown, f"{raw} ({cid})"))
    return None


def merge_date_ranges(query: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Join ``>=``/``<=`` halves per condition; refuse duplicate or missing ends."""
    ranges: dict[tuple[Any, Any], dict[str, Any]] = {}
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
            for end in value:
                if end in ranges[key]["value"]:
                    raise ValueError(f"two '{end}' bounds for {key[0]}; give one lower and one upper bound")
            ranges[key]["value"].update(value)
        else:
            out.append(item)
    for (cond, _t), item in ranges.items():
        value = item["value"]
        if set(value) != {"from", "to"}:
            raise ValueError(f"a date range on {cond} needs both ends (e.g. {cond}>=2030-01-01 and "
                             f"{cond}<=2030-01-31): the server answers a one-sided range with HTTP 500, "
                             "so nothing was sent")
        if str(value["from"]) > str(value["to"]):
            raise ValueError(f"empty date range on {cond}: from {value['from']} is after to {value['to']}")
    return out


def workspace_item(res_name: str, ws: Any) -> dict[str, Any] | None:
    ws_id = _int_choice_id(ws)
    if ws_id is None:
        return None
    if res_name == "tickets":
        return {"condition": "workspace_id", "operator": "is_in", "value": [{"id": ws_id}], "type": "default"}
    return {"condition": "workspace_id", "operator": "is", "value": ws_id, "type": "default"}


def runtime_workspace(sch: dict[str, Any] | None) -> Any:
    from fsv import formmeta
    return formmeta.configured_workspace() or formmeta.workspace_from_fields((sch or {}).get("fields") or [])


def serialize(res_name: str, query: list[dict[str, Any]], or_grouping: bool = False,
              workspace: Any = None) -> tuple[str | None, bool, list[dict[str, Any]]]:
    """(payload, advanced, clean items). Advanced payloads go in advanced_query_hash."""
    advanced = or_grouping or any(isinstance(i, dict) and i.get(_ADV) for i in query)
    clean = [{k: v for k, v in i.items() if k != _ADV} if isinstance(i, dict) else i for i in query]
    if not clean:
        return None, False, clean
    if not advanced:
        return json.dumps(clean, separators=(",", ":")), False, clean
    if res_name not in ADVANCED_MODULES:
        raise ValueError(f"{res_name} has no advanced filter (OR, relative on/last/next dates, numbers, "
                         "multi-select); nothing was sent")
    ws = workspace_item(res_name, workspace)
    extra = [ws] if ws else []
    body = {"any": clean, "all": extra} if or_grouping else {"all": clean + extra, "any": []}
    return json.dumps(body, separators=(",", ":")), True, clean


# List filters a module's list endpoint is proven to ignore: the ticket list
# answers ``approval_status`` with 206 + ``meta.search_error`` and returns
# UNFILTERED rows. Refuse them instead of showing a silently broadened list.
UNSUPPORTED_LIST_FILTERS = {"tickets": frozenset({"approval_status"})}


def refuse_unsupported_filter(res: Any, condition: Any) -> None:
    if str(condition) in UNSUPPORTED_LIST_FILTERS.get(getattr(res, "name", ""), ()):
        raise ValueError(f"{condition} is not a {res.name} list filter (the server ignores it and returns "
                         "unfiltered rows); read approval_status per record instead (e.g. `get`)")


def _check_raw_query(res: Any, items: Any) -> None:
    for item in items if isinstance(items, list) else [items]:
        if isinstance(item, dict):
            if "condition" in item:
                refuse_unsupported_filter(res, item.get("condition"))
            for key in ("any", "all"):
                if key in item:
                    _check_raw_query(res, item[key])


def _resolve_where(
    c: Any, res: Any, sch: dict[str, Any], expr: str, explain: list[dict[str, Any]]
) -> dict[str, Any]:
    field_text, op, raw = _parse_where(expr)
    refuse_unsupported_filter(res, field_text)
    if field_text == "approval_status" and getattr(res, "name", "") == "changes":
        # Proven read-only on the change list: query_hash approval_status is_in ["<n>"].
        # Values are raw platform numbers; no name mapping is inferred.
        if op not in ("=", "!=") or not _isdigit(raw):
            raise ValueError("approval_status takes a numeric value with = or != (e.g. approval_status=0)")
        return _query_item("approval_status", "is_in" if op == "=" else "is_not_in", [raw], "default",
                           explain, field_text, raw)
    if field_text in DATE_FIELDS:
        return date_condition(getattr(res, "name", ""), field_text, op, raw, explain, field_text)
    if field_text.endswith("_id"):
        if op in (">", "<", ">=", "<=", "~="):
            raise ValueError(f"operator {op!r} not supported on ID fields; use = or !=")
        operator = "is_in" if op == "=" else "is_not_in"
        val: Any = int(raw) if _isdigit(raw) else raw
        return _query_item(field_text, operator, [val], "default", explain, field_text, raw)

    f = _find_schema_field(sch, field_text)
    if not f:
        raise ValueError(f"field not found: {field_text!r}")
    name = f.get("name") or field_text
    refuse_unsupported_filter(res, name)
    type_ = _field_api_type(f)
    ftype = str(f.get("field_type") or "")
    if name in DATE_FIELDS:
        return date_condition(getattr(res, "name", ""), str(name), op, raw, explain, field_text)
    typed = typed_condition(getattr(res, "name", ""), f, op, raw, explain, field_text)
    if typed is not None:
        return typed
    from fsv.filter_proof import require_filterable_field
    require_filterable_field(f)
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

    from fsv.filter_proof import name_valued_choice
    named = name_valued_choice(f, raw)
    if named is not None:
        return _query_item(name, operator, [named], type_, explain, name, named)
    if f.get("choices") or "dropdown" in ftype or ftype in ("default_status", "default_priority"):
        val = _choice_value(f, raw, type_)
        return _query_item(name, operator, [val], type_, explain, name, str(val))
    val = int(raw) if _isdigit(raw) and ftype.endswith("number") else raw
    return _query_item(name, operator, [val], type_, explain, name, raw)


_merge_date_ranges = merge_date_ranges


def _load_query_hash(raw: str) -> list[dict[str, Any]]:
    try:
        # Decode percent-escapes at most once, and only when the text is not
        # already JSON (otherwise a literal "%25" inside a value would be
        # decoded twice).
        text = raw if raw.lstrip().startswith(("[", "{")) else unquote(raw)
        loaded = json.loads(text)
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
    query = merge_date_ranges(query)
    payload, _adv, _clean = serialize(getattr(res, "name", ""), query, False,
                                      runtime_workspace(sch) if _adv_needed(query) else None)
    return payload


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
        loaded = _load_query_hash(raw)
        _check_raw_query(res, loaded)
        query.extend(loaded)
        explain.append({"field": "query_hash", "condition": "raw", "operator": "raw", "type": "raw", "value": raw})
    sch: dict[str, Any] | None = None
    if where:
        sch = schema_mod.load(res, c)
        for expr in where:
            query.append(_resolve_where(c, res, sch, expr, explain))
    query = merge_date_ranges(query)
    payload, _adv, clean = serialize(getattr(res, "name", ""), query, or_grouping,
                                     runtime_workspace(sch) if (or_grouping or _adv_needed(query)) else None)
    return payload, clean, explain


def _adv_needed(query: list[dict[str, Any]]) -> bool:
    return any(isinstance(i, dict) and i.get(_ADV) for i in query)
