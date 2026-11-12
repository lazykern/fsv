"""Change lifecycle pre-checks from native state-flow shapes (synthetic Acme data)."""
from __future__ import annotations

import json
from io import StringIO
from types import SimpleNamespace

import pytest
from rich.console import Console
from typer.testing import CliRunner

import acme_change_form as A
import fsv.change_flow as flow
import fsv.cli as cli
from fsv import change_lifecycle as lc, config, journal, state_flow
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute


def native_flow():
    # Shape of GET change_management/<id>?include=state_info: transitions keyed by
    # the from-state as text; both entry-criteria shapes; per-state field locks.
    return {
        "id": 77, "name": "Acme release flow",
        "state_list": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 3, "value": "Closed"},
                       {"id": 22, "value": "Security Review"}, {"id": 23, "value": "Pending Scan"}],
        "transitions": {
            "1": [{"id": 9000081400, "to_state": 2, "entry_criteria": [
                {"fields": {"9000161342": {"name": "department_id", "value": "", "operator": "is_not"},
                            "9000161473": {"name": "customFields.cab_date", "value": "", "operator": "is_not",
                                           "custom_field": True}}}]}],
            "2": [{"id": 9000081401, "to_state": 3, "entry_criteria": [
                {"name": "itil_tasks::status", "value": 3, "operator": "is"},
                {"name": "approval_status", "value": 1, "operator": "is"}]}],
            "22": [{"id": 9000081402, "to_state": 23, "entry_criteria": [{"fields": {}}]}],
            "3": []},
        "state_properties": {"field_lock_config": {
            "22": [{"9000161333": "status"}, {"9000161445": "customFields.environment"}, {"-1": "change_reason"}],
            "1": [{"9000161342": "department"}]}},
    }


def record(**over):
    row = {"id": 900000501, "status": 1, "department_id": None, "approval_status": 1,
           "custom_fields": {"cab_date": "2030-01-02", "environment": "Staging", "notes": "x"}}
    row.update(over)
    return row


def test_disallowed_edge_is_refused_with_allowed_targets():
    errors, _ = lc.precheck(native_flow(), record(), {"status": 3})
    assert errors == ["Open → Closed is not an allowed lifecycle transition (allowed from Open: Planning)"]


def test_unmet_entry_criterion_names_the_field_and_supplied_value_meets_it():
    errors, warnings = lc.precheck(native_flow(), record(), {"status": 2})
    assert errors == ["entry criterion for Open → Planning not met: department_id must be set; supply it with --set"]
    assert not warnings
    errors, _ = lc.precheck(native_flow(), record(custom_fields={}), {"status": 2, "department_id": 9000004001})
    # Supplying the department satisfies it, but it is locked in Open; the
    # custom criterion is unknown on this record, so it is only warned about.
    assert errors == ["locked in lifecycle state Open: department (field lock); the server rejects this write"]


def test_status_lock_refuses_and_unlocked_field_update_passes():
    errors, _ = lc.precheck(native_flow(), record(status=22), {"status": 23})
    assert errors == ["locked in lifecycle state Security Review: status (field lock); the server rejects this write"]
    assert lc.precheck(native_flow(), record(status=22), {"custom_fields": {"notes": "y"}}) == ([], [])
    errors, _ = lc.precheck(native_flow(), record(status=22), {"custom_fields": {"environment": "Live"}})
    assert errors and "environment" in errors[0]


def test_unevaluable_criteria_warn_but_do_not_block():
    errors, warnings = lc.precheck(native_flow(), record(status=2), {"status": 3})
    assert errors == []
    assert warnings == ["entry criterion for Planning → Closed not pre-checked: itil_tasks::status is 3"]


def test_missing_transitions_only_warn():
    flow_data = native_flow()
    del flow_data["transitions"]
    errors, warnings = lc.precheck(flow_data, record(), {"status": 3})
    assert errors == [] and "not pre-checked" in warnings[0]


@pytest.fixture
def fake(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    c = A.FakeClient(change=A.record(state_flow_id=77, department_id=None))
    real = c.int_get
    def get(path, params=None):
        if path == "change_management/77":
            c.calls.append(("GET", path, params))
            return {"state_flow": native_flow()}
        return real(path, params)
    c.int_get = get
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    return c


@pytest.mark.parametrize("status,reason", [
    ("Closed", "Open → Closed is not an allowed lifecycle transition"),
    ("Planning", "entry criterion for Open → Planning not met: department_id must be set"),
])
def test_update_dry_run_refuses_lifecycle_violations_before_sending(fake, status, reason):
    r = CliRunner().invoke(cli.app, ["changes", "update", "CHN-501", "--set", f"Status={status}", "--yes", "--dry-run"])
    out = " ".join(r.output.split())
    assert r.exit_code == 2 and reason in out, out
    r = CliRunner().invoke(cli.app, ["changes", "update", "CHN-501", "--set", f"Status={status}", "--yes"])
    assert r.exit_code == 2 and not fake.writes()
    assert sum(1 for m, p, _ in fake.calls if p == "change_management/77") >= 1


def test_http_400_rejection_surfaces_the_server_reason(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    plan = WritePlan("change_form_update", 501, "acme.example.com", "PUT", "changes/501", json.dumps({"status": 3}), "x")
    def reject(*_a, **_k):
        raise APIError(400, {"description": "Validation failed",
                             "errors": [{"field": "status", "message": "Transition is not allowed", "code": "invalid_value"}]})
    client = SimpleNamespace(_get_csrf=lambda: "synthetic", _request=reject)
    with pytest.raises(ReviewedWriteError) as exc:
        execute(plan, client, reviewed_hash=plan.hash, rebuild=lambda: plan, verify=lambda r: {"verified": True})
    assert "HTTP 400" in str(exc.value) and "status: Transition is not allowed" in str(exc.value)
    assert "no retry" in str(exc.value)


def test_state_table_shows_transitions_criteria_and_locks(monkeypatch):
    def get(path, params=None):
        return {"state_flow": native_flow()} if path == "change_management/77" else {"approvals": []}
    monkeypatch.setattr(state_flow, "get_client", lambda: SimpleNamespace(int_get=get))
    stream = StringIO()
    state_flow.render_flow({"id": 501, "human_display_id": "CHN-501", "state_flow_id": 77, "status": 1,
                            "status_name": "Open", "state_traversal": []},
                           Console(file=stream, width=200, color_system=None))
    out = stream.getvalue()
    assert "Next (allowed transitions)" in out
    assert "Transitions from Open:" in out
    assert "→ Planning: department_id must be set; cab_date must be set" in out
    assert "Locked in Open: department" in out
