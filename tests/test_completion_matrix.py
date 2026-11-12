"""Completion contracts against real synthetic cache envelopes and fake clients."""
import json
from types import SimpleNamespace

import pytest

from fsv import cache, client, completion as C, config
from fsv.resources import CHANGES


FIELDS = [
    {"name": "status", "label": "State", "default_field": True, "field_type": "default_status",
     "choices": [{"id": 47, "value": "Review"}, {"id": 83, "name": "Done"}, {"id": 47, "value": "Review"}]},
    {"name": "cf_enabled", "label": "Enabled", "field_type": "custom_checkbox"},
    {"name": "planned_start_date", "label": "Start", "field_type": "default_date"},
    {"name": "cf_plan", "label": "Test Plan", "field_type": "planning_field"},
    {"name": "cf_owner", "label": "Owner", "field_type": "custom_lookup", "lookup_config": {"link": "/lookups/people"}},
    {"name": "requester", "label": "Requester", "field_type": "custom_lookup"},
    {"name": "agent", "label": "Agent", "field_type": "default_agent"},
    {"name": "cf_title", "label": "Title", "field_type": "custom_text"},
    {"label": "Nameless"},
]
GROUPS = [{"id": 91, "name": "Acme Ops", "description": "Acme support"}, {"id": 92, "name": "Globex"}, {"id": 93}]


@pytest.fixture
def cached():
    cache.save(config.schema_cache_path("changes"), "schema", FIELDS)
    cache.save(config.groups_cache_path(), "groups", {"groups": GROUPS})
    cache.save(config.filters_cache_path("changes"), "filters", {"filters": [
        {"id": 17, "name": "Acme active"}, {"id": "all", "name": "All Changes"}, None, {}]})
    return FIELDS


def pairs(factory, token):
    return list(factory(CHANGES)(token))


def test_cache_envelopes_supply_scoped_fields_views_and_groups(cached):
    assert C._cached_fields(CHANGES) == FIELDS
    assert C._cached_filters(CHANGES) == [{"id": "17", "name": "Acme active"}, {"id": "all", "name": "All Changes"}]
    assert C._cached_groups() == GROUPS
    assert pairs(C.complete_filter_name, "Ac") == [("17", "Acme active"), ("Ac", "")]
    assert list(C.complete_group_query("ac")) == [("Acme Ops", "Acme support"), ("ac", "")]
    assert list(C.complete_update_group_id("9")) == [("91", "Acme Ops"), ("92", "Globex"), ("93", ""), ("9", "")]
    assert list(C.complete_update_group_id("Ac")) == [("Acme Ops", "id=91 · Acme support"), ("Ac", "")]


@pytest.mark.parametrize("shape", ["legacy", "list", "dict", "fallback"])
def test_group_and_filter_cache_response_families(shape):
    groups = [{"id": 91, "name": "Acme"}]
    filters = [{"id": 17, "name": "Acme active"}]
    for path, key, rows in [(config.groups_cache_path(), "groups", groups), (config.filters_cache_path("changes"), "filters", filters)]:
        path.parent.mkdir(parents=True, exist_ok=True)
        doc = {key: rows} if shape == "legacy" else {"data": rows} if shape == "list" else {"data": {key: rows}} if shape == "dict" else {"data": "invalid", key: rows}
        path.write_text(json.dumps(doc))
    assert C._cached_groups() == groups
    assert C._cached_filters(CHANGES) == [{"id": "17", "name": "Acme active"}]


@pytest.mark.parametrize("body", ["not json", "[]", "{}"])
def test_missing_or_corrupt_cache_uses_static_fallback_without_client(monkeypatch, body):
    monkeypatch.setattr(client, "get_client", lambda: pytest.fail("offline completion requested a client"))
    path = config.schema_cache_path("changes")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    assert C._cached_fields(CHANGES) == []
    assert ("status=", "Status · default_status · default") in pairs(C.complete_where, "sta")
    assert pairs(C.complete_where, "status=Pe") == [("status=Pending", "status"), ("status=Pe", "")]
    assert C._cached_groups() == [] and C._cached_filters(CHANGES) == []


def test_stale_cache_remains_usable_and_cannot_leak_other_tenant(cached, monkeypatch):
    cache.mark_stale(config.schema_cache_path("changes"))
    assert C._cached_fields(CHANGES) == FIELDS
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    assert C._cached_fields(CHANGES) == []
    assert C._cached_groups() == [] and C._cached_filters(CHANGES) == []


def test_field_scopes_and_unique_partial_resolution(cached):
    assert C._find_field(CHANGES, "default:State")["name"] == "status"
    assert C._find_field(CHANGES, "custom:Enabled")["name"] == "cf_enabled"
    assert C._find_field(CHANGES, "custom:State") is None
    assert C._find_field(CHANGES, "title")["name"] == "cf_title"
    assert C._find_field(CHANGES, "cf_") is None
    assert pairs(C.complete_field_names, "sta") == [("status", "State · default_status · default"), ("planned_start_date", "Start · default_date · custom"), ("sta", "")]
    assert pairs(C.complete_choice_field_names, "sta") == [("status", "State · default_status · default · 3 choices"), ("sta", "")]
    assert pairs(C.complete_planning_field_names, "Test") == [("Test Plan", "cf_plan"), ("cf_plan", "Test Plan"), ("Test", "")]
    assert ("group", "group lookup") in pairs(C.complete_lookup_kind, "gr")


@pytest.mark.parametrize("token,expected", [
    ("cf_enabled=t", [("cf_enabled=true", ""), ("cf_enabled=t", "")]),
    ("cf_enabled=", [("cf_enabled=true", ""), ("cf_enabled=false", ""), ("cf_enabled=", "")]),
    ("status=Re", [("status=Review", "status"), ("status=Review", "status"), ("status=Re", "")]),
    ("unknown=x", [("unknown=x", "")]),
    ("cf_title=Acme", [("cf_title=Acme", "")]),
    ("'tit", [("title=", "cf_title · custom_text · custom")]),
])
def test_set_completion_types_and_label_prefixes(cached, token, expected):
    assert pairs(C.complete_set, token) == expected


@pytest.mark.parametrize("factory,token,help_text", [(C.complete_set, "planned_start_date=2025", "ISO-8601"), (C.complete_where, "due_by>=2025", "ISO-8601 timestamp")])
def test_date_completion_keeps_operator_and_full_offset(cached, factory, token, help_text):
    prefix = token[:-4]
    assert pairs(factory, token) == [(prefix + date, help_text) for date in C.DATE_EXAMPLES] + [(token, "")]


def test_choice_update_emits_deduplicated_labels_ids_and_fallbacks(cached):
    result = pairs(lambda res: C.complete_update_choice(res, "status"), "")
    assert result[:4] == [("Review", "id=47"), ("47", "Review"), ("Done", "id=83"), ("83", "Done")]
    assert result.count(("Review", "id=47")) == 1
    assert ("Open", "status") in result and result[-1] == ("", "")


@pytest.mark.parametrize("token", ["cf_owner=al", "requester=al", "agent=al"])
def test_network_is_off_by_default_even_with_lookup_metadata(cached, monkeypatch, token):
    monkeypatch.setattr(client, "get_client", lambda: pytest.fail("completion network must be opt-in"))
    assert pairs(C.complete_set, token) == [(token, "")]
    assert pairs(C.complete_where, token) == [(token, "")]
    assert list(C.complete_update_agent_id("al")) == [("al", "")]


@pytest.mark.parametrize("setting,expected", [("on", True), (True, True), ("true", True), ("1", True), ("off", False), (False, False)])
def test_network_opt_in_config_values(setting, expected):
    config.CONFIG_FILE.write_text(json.dumps({"completion": {"network": setting}}))
    assert C._completion_network() is expected


def test_broken_config_and_env_opt_in(monkeypatch):
    config.CONFIG_FILE.unlink()
    assert C._config_load() == {} and not C._completion_network()
    config.CONFIG_FILE.write_text("broken")
    assert C._config_load() == {} and not C._completion_network()
    monkeypatch.setenv("FSV_COMPLETION_NETWORK", "1")
    assert C._completion_network()


@pytest.fixture
def remote(cached, monkeypatch):
    calls = []
    rows = [{"id": 71, "email": "Alex@example.com", "value": "Alex Acme"},
            {"id": 72, "name": "Al Globex"}, {"id": 73, "value": "Unrelated"}, {}]
    def autocomplete(kind, text, params=None):
        calls.append((kind, text, params))
        return rows
    monkeypatch.setenv("FSV_COMPLETION_NETWORK", "1")
    monkeypatch.setattr(client, "get_client", lambda: SimpleNamespace(autocomplete=autocomplete))
    return calls, rows


@pytest.mark.parametrize("kind", ["requesters", "agents"])
def test_opted_in_user_completion_preserves_typed_case_and_requester_scope(remote, kind):
    calls, _ = remote
    assert C._remote_user_values(kind, " al ") == [("alex@example.com", "Alex Acme"), ("al Globex", "Al Globex"), ("Unrelated", "Unrelated")]
    assert calls == [(kind, "al", {"all_users": "true"} if kind == "requesters" else None)]
    assert C._remote_user_values(kind, "a") == [] and len(calls) == 1


@pytest.mark.parametrize("kind", ["requester", "agent", "group", "status", ""])
def test_lookup_query_routes_kind_and_always_suppresses_file_fallback(remote, kind):
    calls, _ = remote
    token = "Re" if kind == "status" else "Ac" if kind == "group" else "al"
    result = list(C.complete_lookup_query(CHANGES)(SimpleNamespace(params={"kind": kind}), token))
    assert result[-1] == (token, "")
    if kind in ("requester", "agent"):
        assert result[0] == ("alex@example.com", "Alex Acme") and len(calls) == 1
    elif kind == "group":
        assert result == [("Acme Ops", "Acme support"), (token, "")] and calls == []
    elif kind == "status":
        assert result[0] == ("Review", "status") and calls == []
    else:
        assert result == [(token, "")] and calls == []


def test_remote_user_errors_are_silent_and_bounded(remote, monkeypatch):
    calls, rows = remote
    rows[:] = [{"email": f"alex{i}@example.com"} for i in range(20)]
    assert len(C._remote_user_values("agents", "al")) == 10
    def denied(*a, **k):
        raise RuntimeError("synthetic denial")
    monkeypatch.setattr(client, "get_client", denied)
    assert C._remote_user_values("agents", "al") == []
    assert list(C.complete_update_agent_id("al")) == []


def test_lookup_set_prefix_sort_case_limit_and_failure(cached, monkeypatch):
    monkeypatch.setenv("FSV_COMPLETION_NETWORK", "1")
    calls = []
    rows = [{"name": "Unrelated"}, {"name": "Al Globex"}, {"name": "Alex Acme", "email": "Alex@example.com"}]
    def choices(link, token):
        calls.append((link, token))
        return list(rows)
    monkeypatch.setattr(client, "get_client", lambda: SimpleNamespace(lookup_choices=choices))
    assert pairs(C.complete_set, "cf_owner=al") == [("cf_owner=al Globex", ""), ("cf_owner=alex@example.com", "Alex Acme"), ("cf_owner=al", "")]
    assert calls == [("/lookups/people", "al")]
    rows[:] = [{"name": f"Alex {i}"} for i in range(25)]
    assert len(pairs(C.complete_set, "cf_owner=al")) == 21
    assert pairs(C.complete_set, "cf_owner=a") == [("cf_owner=a", "")]
    def denied(*a):
        raise RuntimeError("synthetic denial")
    monkeypatch.setattr(client, "get_client", denied)
    assert pairs(C.complete_set, "cf_owner=al") == [("cf_owner=al", "")]


def test_unlinked_lookup_deduplicates_people_but_keeps_raw_token(remote):
    calls, _ = remote
    result = pairs(C.complete_set, "requester=al")
    assert result == [("requester=alex@example.com", "Alex Acme"), ("requester=al Globex", "Al Globex"), ("requester=Unrelated", "Unrelated"), ("requester=al", "")]
    assert [k for k, _, _ in calls] == ["agents", "requesters"]
    calls.clear()
    assert pairs(C.complete_where, "agent!=al")[0] == ("agent!=alex@example.com", "Alex Acme")
    assert pairs(C.complete_where, "requester=al")[0] == ("requester=alex@example.com", "Alex Acme")
    assert list(C.complete_update_agent_id("al"))[0] == ("alex@example.com", "Alex Acme")


@pytest.mark.parametrize("name,token,expected", [
    ("complete_format", "j", ("json", "JSON array")),
    ("complete_store", "arg", ("argon", "Argon2id + AES-256-GCM encrypted")),
    ("complete_config_key", "forms", ("forms.workspace_id", "workspace whose business rules apply")),
    ("complete_config_value", "of", ("off", "")),
    ("complete_cache_target", "v", ("views", "saved view names")),
])
def test_static_completers_filter_prefix_and_preserve_raw_token(name, token, expected):
    assert list(getattr(C, name)(token)) == [expected, (token, "")]
