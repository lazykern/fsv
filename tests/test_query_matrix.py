"""Query payload contracts: scoped ambiguity, typed choices and inclusive dates."""
import json
from types import SimpleNamespace
from urllib.parse import quote

import pytest

from fsv import query, schema
from fsv.resources import CHANGES, TICKETS


@pytest.fixture
def sch():
    return {"fields": [
        {"name": "status", "label": "State", "default_field": True, "field_type": "default_status",
         "choices": [{"id": 47, "value": "Awaiting review", "requester_display_value": "In review"}, {"id": 83, "name": "Done"}]},
        {"name": "cf_tier", "label": "Tier", "field_type": "custom_dropdown", "choices": [{"id": 701, "value": "Gold"}]},
        {"name": "cf_count", "label": "Count", "field_type": "custom_number"},
        {"name": "subject", "label": "Subject", "default_field": True, "field_type": "default_text"},
        {"name": "requester", "label": "Requester", "default_field": True},
        {"name": "agent", "label": "Agent", "default_field": True},
        {"name": "group", "label": "Group", "default_field": True, "choices": [{"id": 93, "value": "Acme Ops"}]},
    ]}


@pytest.mark.parametrize("expression,condition,value,kind,operator", [
    ("State=In review", "status", [47], "default", "is_in"),
    ("status!=83", "status", [83], "default", "is_not_in"),
    ("custom:Tier=701", "cf_tier", ["Gold"], "custom_field", "is_in"),
    ("cf_tier=gold", "cf_tier", ["Gold"], "custom_field", "is_in"),
    ("Count=12", "cf_count", [12], "custom_field", "is_in"),
    ("Count=-12", "cf_count", ["-12"], "custom_field", "is_in"),
    ("Subject='Acme = ready'", "subject", ["Acme = ready"], "default", "is_in"),
    ("group=Acme Ops", "group_id", ["93"], "default", "is_in"),
    ("group_id!=93", "group_id", [93], "default", "is_not_in"),
    ("asset_id=external", "asset_id", ["external"], "default", "is_in"),
    ("requester=71", "requester_id", [{"id": 71, "name": "71"}], "default", "is_in"),
    ("agent=72", "agent_id", ["72"], "default", "is_in"),
])
def test_schema_filters_produce_exact_wire_items(sch, expression, condition, value, kind, operator):
    payload = query.build_query_hash_from_schema(None, CHANGES, sch, [expression])
    assert json.loads(payload) == [{"condition": condition, "value": value, "type": kind, "operator": operator}]


@pytest.mark.parametrize("expression,message", [
    ("status", "invalid filter"), ("unknown=x", "field not found"),
    ("status=missing", "no choice"), ("status~=review", "contains"),
    *[("status" + op + "47", "not supported") for op in (">", "<", ">=", "<=")],
    *[("group_id" + op + "93", "ID fields") for op in (">", "<", ">=", "<=", "~=")],
    ("created_at!=2026-01-01", "not supported for date"),
    ("created_at~=2026", "unsupported date operator"),
])
def test_invalid_filter_fails_locally_with_actionable_message(sch, expression, message):
    with pytest.raises(ValueError, match=message):
        query.build_query_hash_from_schema(None, CHANGES, sch, [expression])


@pytest.mark.parametrize("field", ["created_at", "updated_at", "due_by", "planned_start_date", "planned_end_date"])
@pytest.mark.parametrize("lower,upper", [(">", "<"), (">=", "<=")])
def test_date_bounds_are_inclusive_merged_and_do_not_erase_other_fields(sch, field, lower, upper):
    result = json.loads(query.build_query_hash_from_schema(None, CHANGES, sch, [
        f"{field}{lower}2026-01-01", "status=47", f"{field}{upper}2026-12-31"]))
    assert result == [
        {"condition": field, "type": "default", "operator": "custom_date", "value": {"from": "2026-01-01", "to": "2026-12-31"}},
        {"condition": "status", "type": "default", "operator": "is_in", "value": [47]}]


def test_exact_date_and_last_repeated_bound_are_preserved(sch):
    result = json.loads(query.build_query_hash_from_schema(None, CHANGES, sch, [
        "due_by=2026-01-01", "due_by>=2026-02-01"]))
    assert result[0]["value"] == {"from": "2026-02-01", "to": "2026-01-01"}
    # Builder serializes the supplied range; it does not validate date ordering.


@pytest.mark.parametrize("fields,text,match", [
    ([{"name": "status", "label": "State"}, {"name": "other", "label": "Status"}], "status", "status"),
    ([{"name": "cf_zone", "label": "Acme Zone"}], "zone", "cf_zone"),
    ([{"name": "state", "default_field": True}, {"name": "state"}], "default:state", "state"),
    ([{"name": "state", "default_field": True}, {"name": "state"}], "custom:state", "state"),
])
def test_field_name_precedence_scope_and_unique_substrings(fields, text, match):
    assert query._find_schema_field({"fields": fields}, text)["name"] == match


@pytest.mark.parametrize("fields,text", [
    ([{"name": "alpha", "label": "Zone"}, {"name": "beta", "label": "Zone"}], "Zone"),
    ([{"name": "cf_zone_a"}, {"name": "cf_zone_b"}], "zone"),
])
def test_duplicate_labels_and_fuzzy_matches_are_not_guessed(fields, text):
    with pytest.raises(ValueError, match="ambiguous field") as exc:
        query._find_schema_field({"fields": fields}, text)
    assert all(f["name"] in str(exc.value) for f in fields)


@pytest.mark.parametrize("res,agent_condition", [(CHANGES, "agent_id"), (TICKETS, "responder_id")])
def test_named_people_resolve_server_ids_not_display_names(monkeypatch, sch, res, agent_condition):
    calls = []
    def autocomplete(kind, value, params=None):
        calls.append((kind, value, params))
        return [{"id": 71, "user_id": 171, "value": "Alex Acme", "email": "alex@example.com"}]
    c = SimpleNamespace(autocomplete=autocomplete)
    monkeypatch.setattr(schema, "load", lambda resource, client: sch)
    payload, items, explain = query.build_query_hash(c, res, ["requester=alex@example.com", "agent=Alex Acme"], or_grouping=True)
    assert json.loads(payload) == {"any": items}
    assert items[0]["value"] == [{"id": 71, "name": "Alex Acme"}]
    assert items[1]["condition"] == agent_condition and items[1]["value"] == ["171"]
    assert explain[0]["value"] == "Alex Acme (71)"
    assert calls == [("requesters", "alex@example.com", {"all_users": "true"}), ("agents", "Alex Acme", None)]


@pytest.mark.parametrize("rows,message", [([], "no agent match"), ([{"id": 71, "value": "Alex"}], "no user_id"),
    ([{"id": 71, "value": "Alex Acme"}, {"id": 72, "value": "Alex Globex"}], "multiple agent matches")])
def test_agent_lookup_errors_never_fall_back_to_wrong_id(rows, message):
    c = SimpleNamespace(autocomplete=lambda *a: rows)
    with pytest.raises(ValueError, match=message):
        query._resolve_agent(c, "Alex")


def test_group_lookup_exact_then_unique_and_missing_id():
    calls = []
    c = SimpleNamespace(int_get=lambda path: calls.append(path) or {"groups": [
        {"id": 91, "name": "Acme"}, {"id": 92, "name": "Acme Ops"}]})
    assert query._resolve_group(c, "Acme") == "91"
    assert query._resolve_group(c, "Ops") == "92"
    assert query._resolve_group(c, "93") == "93"
    assert calls == ["bootstrap/agents_groups"] * 2
    with pytest.raises(ValueError, match="no id"):
        query._resolve_group(SimpleNamespace(int_get=lambda p: {"groups": [{"name": "Acme"}]}), "Acme")


@pytest.mark.parametrize("encoded", [False, True])
def test_raw_query_hash_decodes_only_once_and_combines_with_schema(monkeypatch, sch, encoded):
    raw_items = [{"condition": "subject", "operator": "is_in", "type": "default", "value": ["100%25 Acme"]}]
    raw = json.dumps(raw_items)
    if encoded:
        raw = quote(raw)
    monkeypatch.setattr(schema, "load", lambda *a: sch)
    payload, items, explain = query.build_query_hash(None, CHANGES, ["status=47"], raw=raw)
    assert json.loads(payload) == items and items[0] == raw_items[0]
    assert items[1]["value"] == [47] and explain[0]["value"] == raw


@pytest.mark.parametrize("raw,message", [("%ZZ", "invalid query-hash"), ("%255B%255D", "invalid query-hash"), ("{}", "JSON array"), ("null", "JSON array")])
def test_malformed_raw_query_hash_is_rejected(raw, message):
    with pytest.raises(ValueError, match=message):
        query.build_query_hash(None, CHANGES, [], raw=raw)


def test_empty_filters_do_not_load_schema(monkeypatch, sch):
    monkeypatch.setattr(schema, "load", lambda *a: pytest.fail("unneeded schema request"))
    assert query.build_query_hash(None, CHANGES, []) == (None, [], [])
    assert query.build_query_hash_from_schema(None, CHANGES, sch, []) is None
