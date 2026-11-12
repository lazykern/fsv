import json
from types import SimpleNamespace
import pytest
from typer.testing import CliRunner
from fsv import change_tasks as a, cli, config, ticket_tasks
from fsv.reviewed_write import ReviewedWriteError


def fields():
    return [{"name": "title", "label": "Title", "default": True, "field_options": {"required_for_create": "true"}},
            {"name": "workspace_id", "label": "Workspace", "default": True, "field_options": {"required_for_create": "true"}},
            {"name": "cf_system", "label": "System", "default": False, "field_options": {"required_for_create": "true"}, "choices": [{"id": "17", "value": "Acme"}]},
            {"name": "cf_environment", "label": "Environment", "default": False, "field_options": {"required_for_create": "true"}, "choices": [{"id": "29", "value": "Test"}]},
            {"name": "planned_start_date", "label": "Planned Start", "default": True, "field_options": {"required_for_create": "true"}},
            {"name": "planned_end_date", "label": "Planned End", "default": True, "field_options": {"required_for_create": "true"}},
            {"name": "status", "choices": [{"id": "7", "value": "Open"}]},
            {"name": "notify_before", "choices": [{"id": "0", "value": "Never"}]}]


def test_change_cli_required_nested_metadata_and_custom_wire(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(a, "runtime_fields", fields)
    monkeypatch.setattr(a, "resolve_status", lambda *a: 7)
    base = ["changes", "tasks-add", "CHN-23", "--title", "Acme review", "--dry-run", "--json"]
    runner = CliRunner()
    r = runner.invoke(cli.app, base)
    assert r.exit_code == 2 and "System, Environment, Planned Start, Planned End" in r.output
    r = runner.invoke(cli.app, base + ["--system", "Acme", "--environment", "Test", "--planned-start", "2026-12-01T10:00:00Z", "--planned-end", "2026-12-01T11:00:00Z"])
    assert r.exit_code == 0, r.output
    payload = json.loads(r.stdout)["payload"]
    assert payload["custom_fields"] == {"system": 17, "environment": 29}
    assert payload["status"] == 7 and "workspace_id" not in payload


def test_change_required_validation_and_metadata_bound(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    with pytest.raises(a.TaskError, match="required task fields"):
        a.build_plan(23, "Acme", status_id=7, fields=fields())
    kw = dict(status_id=7, fields=fields(), custom_fields={"system": 17, "environment": 29}, planned_start="2026-12-01T10:00:00Z", planned_end="2026-12-01T11:00:00Z")
    plan = a.build_plan(23, "Acme", **kw)
    changed = fields()
    changed[2]["choices"].append({"id": "31", "value": "Globex"})
    assert a.build_plan(23, "Acme", **{**kw, "fields": changed}).hash != plan.hash


def test_ticket_existing_required_guard_and_explicit_value(monkeypatch):
    schema = [f for f in fields() if f["name"] not in {"cf_environment", "planned_start_date", "planned_end_date"}]
    c = SimpleNamespace(int_get=lambda path, params=None: {"ticket_task_fields": schema} if path == "ticket_task_fields" else {"tasks": [], "meta": {"has_next": False}})
    with pytest.raises(ReviewedWriteError, match="required task fields missing: System"):
        ticket_tasks.build_plan(c, 23, action="add", title="Acme")
    plan = ticket_tasks.build_plan(c, 23, action="add", title="Acme", values={"cf_system": "Acme"})
    assert plan.payload["custom_fields"] == {"system": 17}


def test_metadata_hash_upgrade_keeps_legacy_ambiguity_guard(monkeypatch):
    from dataclasses import replace
    from fsv import journal
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    plan = a.build_plan(23, "Acme", status_id=7, fields=[])
    legacy = replace(plan, fields_json="")
    journal.record_plan("started", legacy.hash, action="change_task_create", target=23)
    c = SimpleNamespace(_request=lambda *a, **k: pytest.fail("must not write"))
    with pytest.raises(a.TaskError, match="do not resubmit"):
        a.send(plan, c, reviewed_hash=plan.hash)

