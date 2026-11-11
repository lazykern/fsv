"""Synthetic offline optional task fields and captured approval rule variants."""
import json
from dataclasses import replace
from types import SimpleNamespace
from typing import Any

import pytest
from typer.testing import CliRunner

from fsv import change_approvals as approvals, change_tasks as tasks, cli, journal


OPTIONS: dict[str, Any] = dict(group_id=9000004001, status="In Progress", note="<Acme> & review\nNext step",
               notify_before="30 Minutes", planned_start="2026-12-05T00:00-05:00",
               planned_end="2026-12-06T00:00:00-05:00", planned_effort="01:30")


def test_task_optional_wire_and_review():
    plan = tasks.build_plan(123, "Acme rollout", status_id=2, **OPTIONS)
    assert plan.payload == {"title": "Acme rollout", "status": 2, "notify_before": 1800,
                            "group_id": 9000004001,
                            "description": "<div><p>&lt;Acme&gt; &amp; review<br>Next step</p></div>",
                            "planned_start_date": "2026-12-05T00:00:00-05:00",
                            "planned_end_date": "2026-12-06T00:00:00-05:00",
                            "planned_effort": "1h 30m"}
    review = plan.review()
    assert review["payload"] == plan.payload and review["note_text"] == OPTIONS["note"]
    assert review["status_label"] == "In Progress" and review["reminder"] == "30 Minutes"
    assert tasks.build_plan(123, "Acme rollout", status_id=2, **{**OPTIONS, "planned_effort": "1h30m"}).hash == plan.hash


@pytest.mark.parametrize("label,seconds", tasks.NOTIFY_BEFORE.items())
def test_all_reminder_labels(label, seconds):
    plan = tasks.build_plan(123, "Acme review", status_id=1, notify_before=label.lower())
    assert plan.payload["notify_before"] == seconds
    assert plan.review()["reminder"] == label


@pytest.mark.parametrize("raw,wire", [("1h30m", "1h 30m"), ("01:30", "1h 30m"),
                                      ("2h", "2h 0m"), ("30m", "0h 30m"), ("90m", "1h 30m")])
def test_effort_normalizes(raw, wire):
    assert tasks.build_plan(123, "Acme review", status_id=1, planned_effort=raw).payload["planned_effort"] == wire


@pytest.mark.parametrize("options", [dict(group_id=0), dict(group_id=True), dict(status=""),
    dict(status="Completed", status_id=None), dict(status="In Progress", status_id=0), dict(notify_before="10 Minutes"),
    dict(note="bad\x00"), dict(planned_effort="1.5h"), dict(planned_effort="01:99"),
    dict(planned_effort="-1h"), dict(planned_start="2026-12-05"),
    dict(planned_start="2026-12-05T12:30:00"), dict(planned_start="2026-12-05T12:30:00.123Z"),
    dict(planned_start="2026-12-06T00:00Z", planned_end="2026-12-05T00:00Z")])
def test_invalid_optional_values(options):
    with pytest.raises(ValueError):
        tasks.build_plan(123, "Acme review", **{"status_id": 2, **options})


def test_blank_optional_fields_omitted_and_z_date():
    plan = tasks.build_plan(123, "Acme review", status_id=1, note=" \n", planned_effort=" ", planned_start="", planned_end="")
    assert plan.payload == {"title": "Acme review", "status": 1, "notify_before": 0}
    assert tasks.build_plan(123, "Acme review", status_id=1, planned_start="2026-12-05T00:00Z").planned_start == "2026-12-05T00:00:00+00:00"


@pytest.mark.parametrize("key,value", [("group_id", 9000004002), ("status", "Open"), ("note", "Other text"),
    ("notify_before", "Never"), ("planned_start", "2026-12-04T00:00Z"),
    ("planned_end", "2026-12-07T00:00Z"), ("planned_effort", "2h")])
def test_every_optional_field_hash_bound(monkeypatch, key, value):
    original = tasks.build_plan(123, "Acme review", status_id=2, **OPTIONS)
    changed = tasks.build_plan(123, "Acme review", **{**OPTIONS, key: value, "status_id": 1 if key == "status" else 2})
    assert original.hash != changed.hash
    monkeypatch.setattr(tasks, "get_client", lambda: pytest.fail("hash mismatch must not construct client"))
    with pytest.raises(tasks.TaskError, match="hash"):
        tasks.send(changed, reviewed_hash=original.hash)
    assert journal.read() == []


@pytest.mark.parametrize("label", ["Completed", "Blocked"])
def test_uncaptured_status_ids_require_runtime_metadata(label):
    c = SimpleNamespace(int_get=lambda *a: {"change_task_fields": [
        {"name": "status", "choices": [{"id": 87, "value": label}]}]})
    assert tasks.resolve_status(label, c) == 87  # deliberately not an inferred sequential ID
    assert tasks.build_plan(123, "Globex review", status=label, status_id=87).payload["status"] == 87
    c.int_get = lambda *a: {"change_task_fields": []}
    with pytest.raises(tasks.TaskError, match="no ID inferred"):
        tasks.resolve_status(label, c)


@pytest.mark.parametrize("rows", [[], [{"id": 9000004001, "name": "Acme operations"},
                                        {"id": 9000004002, "name": "Acme operations"}],
                                       [{"id": 9000004001, "name": "Acme operations backup"}]])
def test_group_name_no_fuzzy_or_ambiguous_matches(rows):
    c = SimpleNamespace(int_get=lambda *a: {"groups": rows})
    with pytest.raises(tasks.TaskError):
        tasks.resolve_group("Acme operations", c)


def test_group_resolution_explicit_and_exact(monkeypatch):
    monkeypatch.setattr(tasks, "get_client", lambda: pytest.fail("numeric group lookup"))
    assert tasks.resolve_group("9000004001") == 9000004001
    assert tasks.resolve_group(" ") is None
    c = SimpleNamespace(int_get=lambda *a: {"groups": [{"id": 9000004001, "name": "Acme operations"}]})
    assert tasks.resolve_group("acme OPERATIONS", c) == 9000004001


@pytest.mark.parametrize("mode", ["complete", "absent", "mismatch", "different-offset"])
def test_task_optional_readback_single_attempt(monkeypatch, mode):
    plan = tasks.build_plan(123, "Acme review", status_id=2, **OPTIONS)
    row = {"id": 9000002010, "owner_id": None, **plan.payload}
    if mode == "absent":
        row.pop("description")
        row.pop("notify_before")
    elif mode == "mismatch":
        row["planned_effort"] = "2h 0m"
    elif mode == "different-offset":
        row["planned_start_date"] = "2026-12-05T05:00:00Z"
    inventories = iter([[], [row]])
    monkeypatch.setattr(tasks, "_inventory", lambda *a: next(inventories))
    calls = []
    c = SimpleNamespace(_get_csrf=lambda: "synthetic-token",
                        _request=lambda *a, **kw: calls.append((a, kw)) or {"task": {"id": row["id"]}})
    result = tasks.send(plan, c, reviewed_hash=plan.hash)
    assert result["identity_verified"] is True
    assert result["verified"] is (mode != "mismatch")
    assert result["fields_verified"] is (mode in ("complete", "different-offset"))
    assert result["unverified_fields"] == (["notify_before", "description"] if mode == "absent" else [])
    assert result["mismatched_fields"] == (["planned_effort"] if mode == "mismatch" else [])
    if mode == "absent":
        assert "absent from GET" in result["warning"]
    assert len(calls) == 1 and json.loads(calls[0][1]["content"]) == plan.payload
    with pytest.raises(tasks.TaskError, match="do not resubmit"):
        tasks.send(plan, c, reviewed_hash=plan.hash)
    assert len(calls) == 1
    assert "Acme review" not in journal.journal_path().read_text()


def test_cli_all_optional_fields_local_review(monkeypatch):
    monkeypatch.setattr(tasks, "resolve_status", lambda value, client=None: {"Open": 1, "In Progress": 2}[value])
    result = CliRunner().invoke(cli.app, ["changes", "tasks-add", "CHN-123", "--title", "Acme review",
        "--group", "9000004001", "--status", "In Progress", "--note", "<literal>",
        "--notify-before", "2 Hours", "--planned-start", "2026-12-05T00:00-05:00",
        "--planned-end", "2026-12-06T00:00-05:00", "--planned-effort", "01:30", "--dry-run", "--json"])
    assert result.exit_code == 0, result.output
    review = json.loads(result.stdout)
    assert review["payload"]["group_id"] == 9000004001
    assert review["payload"]["planned_effort"] == "1h 30m"
    assert review["payload"]["notify_before"] == 7200
    assert review["payload"]["description"] == "<div><p>&lt;literal&gt;</p></div>"
    assert journal.read() == []


@pytest.mark.parametrize("rule,number", [("everyone", 1), ("anyone", 2), ("majority", 3), ("first-responder", 4)])
def test_approval_rule_payload_hash_readback_and_cli(monkeypatch, rule, number):
    plan = approvals.build_plan(123, "Acme review", "Please review", approver_ids=[9000003001], rule=rule)
    assert plan.payload["approval_groups"][0]["approval_type"] == number
    assert plan.review()["approval_rule"] == approvals.RULES[rule][1]
    assert len({replace(plan, rule=r).hash for r in approvals.RULES}) == 4
    row = {"id": 310000000001, "member_id": 9000003001, "status": {"id": 0, "name": "requested"},
           "type": {"id": number, "name": "everyone" if rule == "everyone" else "server rule label"}}
    inventories = iter([[], [row]])
    monkeypatch.setattr(approvals, "_inventory", lambda *a: next(inventories))
    monkeypatch.setattr(approvals, "get_change_approval_groups", lambda *a: [])
    calls = []
    c = SimpleNamespace(_get_csrf=lambda: "synthetic-token",
                        _request=lambda *a, **kw: calls.append((a, kw)) or {})
    result = approvals.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] is True and len(calls) == 1
    assert json.loads(calls[0][1]["content"]) == plan.payload
    with pytest.raises(approvals.ApprovalError, match="do not resubmit"):
        approvals.send(plan, c, reviewed_hash=plan.hash)
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("local rule draft network"))
    draft = CliRunner().invoke(cli.app, ["changes", "approval-request", "CHN-123", "--name", "Acme review",
        "--message", "Please review", "--approver", "9000003001", "--rule", rule, "--dry-run", "--json"])
    assert draft.exit_code == 0, draft.output
    assert json.loads(draft.stdout)["payload"]["approval_groups"][0]["approval_type"] == number


@pytest.mark.parametrize("rule", ["anyone", "majority", "first-responder"])
def test_approval_rule_wrong_readback_and_hash_rejected(monkeypatch, rule):
    plan = approvals.build_plan(123, "Acme review", "Please review", approver_ids=[9000003001], rule=rule)
    wrong = {"id": 310000000001, "member_id": 9000003001, "status": {"id": 0, "name": "requested"},
             "type": {"id": 1, "name": "everyone"}}
    with pytest.raises(approvals.ApprovalError, match="rule"):
        approvals._verify_new([wrong], plan)
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("changed rule constructed client"))
    with pytest.raises(approvals.ApprovalError, match="hash"):
        approvals.send(plan, reviewed_hash=replace(plan, rule="everyone").hash)
    assert journal.read() == []


def test_default_task_hash_preserves_existing_journal_guard(monkeypatch):
    plan = tasks.build_plan(123, "Acme review", status_id=1)
    legacy_hash = journal.plan_hash({"tenant": plan.tenant, "path": plan.path,
                                    "payload": {"title": "Acme review", "status": 1, "notify_before": 0}})
    assert legacy_hash is not None and plan.hash == legacy_hash
    journal.record_plan("started", legacy_hash, action="change_task_create", target=123)
    monkeypatch.setattr(tasks, "get_client", lambda: pytest.fail("legacy started plan bypassed"))
    with pytest.raises(tasks.TaskError, match="do not resubmit"):
        tasks.send(plan, reviewed_hash=plan.hash)


@pytest.mark.parametrize("label", ["Completed", "Blocked"])
def test_cli_status_metadata_and_local_validation_before_lookup(monkeypatch, label):
    calls = []
    c = SimpleNamespace(int_get=lambda *a: calls.append(a) or {"task_fields": [
        {"name": "status", "choices": [{"id": 87, "label": label}]}]})
    monkeypatch.setattr(tasks, "get_client", lambda: c)
    base = ["changes", "tasks-add", "123", "--title", "Globex review", "--status", label, "--dry-run"]
    runner = CliRunner()
    result = runner.invoke(cli.app, [*base, "--planned-effort", "bad"])
    assert result.exit_code == 2 and calls == []
    result = runner.invoke(cli.app, base)
    assert result.exit_code == 0, result.output
    assert json.loads(result.stdout)["payload"]["status"] == 87
    assert len(calls) == 1 and journal.read() == []


def test_missing_response_id_contradictory_optional_fields_unknown(monkeypatch):
    plan = tasks.build_plan(123, "Acme review", status_id=1, planned_effort="1h30m")
    inventories = iter([[], [{"id": 9000002010, "title": plan.title, "owner_id": None,
                             "planned_effort": "2h 0m"}]])
    monkeypatch.setattr(tasks, "_inventory", lambda *a: next(inventories))
    calls = []
    c = SimpleNamespace(_get_csrf=lambda: "synthetic-token",
                        _request=lambda *a, **kw: calls.append((a, kw)) or {})
    with pytest.raises(tasks.TaskError, match="outcome unknown"):
        tasks.send(plan, c, reviewed_hash=plan.hash)
    assert [r["event"] for r in journal.read()] == ["started"]
    assert len(calls) == 1


def test_invalid_rule_before_lookup(monkeypatch):
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("invalid rule lookup"))
    result = CliRunner().invoke(cli.app, ["changes", "approval-request", "CHN-123", "--name", "Acme review",
        "--message", "Review", "--approver", "agent@example.com", "--rule", "all", "--dry-run"])
    assert result.exit_code == 2 and "rule must be" in result.output
    assert journal.read() == []
