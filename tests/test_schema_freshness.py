"""Schema cache freshness: 15-minute form schemas, write-time revalidation
and one forced refetch on a validation refusal (synthetic data only)."""
from __future__ import annotations

import time

import pytest
import typer
from typer.testing import CliRunner

import fsv.cache as cache
import fsv.config as config
import fsv.schema as schema
from fsv import cli
from fsv.cache import save
from fsv.resources import TICKETS

OLD = [{"name": "status", "label": "Status", "choices": [{"id": 2, "value": "Open"}]},
       {"name": "region", "label": "Region", "field_type": "custom_dropdown",
        "choices": [{"id": 11, "value": "North"}]}]
NEW = [{"name": "status", "label": "Status", "choices": [{"id": 2, "value": "Open"}, {"id": 7, "value": "Parked"}]},
       {"name": "region", "label": "Region", "field_type": "custom_dropdown",
        "choices": [{"id": 11, "value": "North"}, {"id": 12, "value": "South"}]}]


class FieldsClient:
    def __init__(self, fields):
        self.fields, self.calls = fields, 0

    def int_get(self, path):
        self.calls += 1
        return {TICKETS.form_fields_key: [dict(f) for f in self.fields]}

    def v2_get(self, path):
        return {TICKETS.form_fields_key: []}


@pytest.fixture(autouse=True)
def _no_global_refresh(monkeypatch):
    monkeypatch.setattr(cache, "FORCE_REFRESH", False)
    monkeypatch.setattr(cache, "_REFRESHED", set())


def _seed(fields, age=0.0):
    config.ensure_dirs()
    p = config.schema_cache_path(TICKETS.name)
    save(p, "schema", fields)
    if age:
        import json
        doc = json.loads(p.read_text())
        doc["saved_at"] = time.time() - age
        p.write_text(json.dumps(doc))
    return p


def test_form_schema_and_rule_ttls_are_15_minutes():
    assert cache.TTL["schema"] == 900 and cache.TTL["form_rules"] == 900
    assert cache.WRITE_MAX_AGE == 900 and schema.CACHE_TTL == 900


def test_read_within_ttl_uses_cache_without_fetch():
    _seed(OLD, age=60)
    c = FieldsClient(NEW)
    sch = schema.load(TICKETS, c)
    assert sch["source"] == "cache" and c.calls == 0


def test_write_load_refetches_a_schema_older_than_15_minutes():
    _seed(OLD, age=16 * 60)
    c = FieldsClient(NEW)
    sch = schema.load(TICKETS, c, for_write=True)
    assert sch["source"] == "server" and c.calls == 1
    assert schema.load(TICKETS, c, for_write=True)["source"] == "cache" and c.calls == 1


def test_revalidate_for_write_refetches_only_an_old_cache():
    _seed(OLD, age=60)
    c = FieldsClient(NEW)
    schema.revalidate_for_write(TICKETS, c)
    assert c.calls == 0
    _seed(OLD, age=16 * 60)
    schema.revalidate_for_write(TICKETS, c)
    assert c.calls == 1 and schema.load(TICKETS, c)["fields"][0]["choices"][-1]["value"] == "Parked"


def test_new_choice_refused_by_cache_is_accepted_after_one_refetch(capsys):
    _seed(OLD)
    c = FieldsClient(NEW)
    sch = schema.load(TICKETS, c, for_write=True)
    assert sch["source"] == "cache" and c.calls == 0
    used, value = schema.validate_with_refetch(
        TICKETS, c, sch, lambda s: cli._resolve_update_choice(s, "status", "Parked"))
    assert value == 7 and used["source"] == "server" and c.calls == 1
    assert "refetching the tickets form schema once" in capsys.readouterr().err


def test_new_custom_choice_via_set_is_accepted_after_one_refetch():
    _seed(OLD)
    c = FieldsClient(NEW)
    sch = schema.load(TICKETS, c)
    _, (key, value) = schema.validate_with_refetch(
        TICKETS, c, sch, lambda s: cli._resolve_set_field(c, s, "Region", "South"))
    assert (key, value) == ("region", "South") and c.calls == 1


def test_still_unknown_value_refuses_after_exactly_one_refetch():
    _seed(OLD)
    c = FieldsClient(NEW)
    sch = schema.load(TICKETS, c)
    calls = []

    def build(s):
        calls.append(s["source"])
        return cli._resolve_update_choice(s, "status", "Nowhere")

    with pytest.raises(typer.Exit) as exc:
        schema.validate_with_refetch(TICKETS, c, sch, build)
    assert exc.value.exit_code == 2
    assert calls == ["cache", "server"] and c.calls == 1


def test_unknown_field_triggers_one_refetch_then_refuses():
    _seed(OLD)
    c = FieldsClient(NEW)
    with pytest.raises(typer.Exit):
        schema.validate_with_refetch(TICKETS, c, schema.load(TICKETS, c),
                                     lambda s: cli._resolve_set_field(c, s, "Planet", "Mars"))
    assert c.calls == 1


def test_server_fresh_schema_is_never_refetched():
    c = FieldsClient(OLD)
    sch = schema.load(TICKETS, c)
    assert sch["source"] == "server" and c.calls == 1
    with pytest.raises(typer.Exit):
        schema.validate_with_refetch(TICKETS, c, sch, lambda s: cli._resolve_update_choice(s, "status", "Parked"))
    assert c.calls == 1


def test_non_schema_refusal_does_not_refetch():
    _seed(OLD)
    c = FieldsClient(NEW)

    def build(s):
        cli._err("nothing to update; pass at least one field")

    with pytest.raises(typer.Exit):
        schema.validate_with_refetch(TICKETS, c, schema.load(TICKETS, c), build)
    assert c.calls == 0


def test_refusal_outside_the_retry_scope_prints_and_exits():
    with pytest.raises(typer.Exit):
        cli._resolve_update_choice({"fields": OLD}, "status", "Parked")


def test_global_refresh_refetches_once_per_process():
    _seed(OLD)
    c = FieldsClient(NEW)
    cache._REFRESHED.clear()  # the seed write counts as this process's refetch
    cache.FORCE_REFRESH = True
    assert schema.load(TICKETS, c)["source"] == "server" and c.calls == 1
    assert schema.load(TICKETS, c)["source"] == "cache" and c.calls == 1


def test_cache_status_prints_age_and_ttl_per_cache():
    _seed(OLD, age=120)
    out = CliRunner().invoke(cli.app, ["cache", "status"], env={"COLUMNS": "200"})
    assert out.exit_code == 0
    assert "tickets" in out.output and "15m" in out.output and "2m" in out.output
