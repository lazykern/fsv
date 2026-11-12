"""Write gate for business rules fsv cannot fully model (synthetic Acme rules)."""
from __future__ import annotations

import json

import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.change_flow as change_flow
import fsv.cli as cli
from fsv import ticket_flow, ticket_lifecycle
from fsv.drafts import Draft
from test_ticket_create import WHO, Client, draft, meta


def _plan(m, d=None, **kw):
    return ticket_flow.build_create_plan(m, d or draft(), resolver=None, requester=WHO, **kw)


def _add(m, rid, cond, actions, *, rev=False):
    r = a._rule(rid, f"Acme rule {rid}", cond, actions, rev=rev)
    r["module_type"] = "ticket"
    m.rules["business_rules"].append(r)
    return r


def _c(name, source, op, value=None):
    out = {"name": name, "evaluate_on": source, "operator": op}
    if value is not None:
        out["value"] = value
    return out


def test_agent_form_models_portal_false():
    m = meta()
    _add(m, 9000006001, {"all": [_c("portal", "system", "is", "true")]}, [a._a("hide", ["subject"])])
    p = _plan(m)
    assert p.ok, p.errors  # the portal-only rule does not match on the agent form
    assert m.engine().context.system == {"portal": False}


def test_current_user_groups_come_from_runtime_profile():
    m = meta()
    _add(m, 9000006002, {"all": [_c("group_ids", "current_user", "includes", ["77"])]}, [a._a("hide", ["priority"])])
    m.rules["user"] = {"group_ids": [77], "department_ids": []}
    p = _plan(m)
    assert p.ok, p.errors
    assert 9000006002 in p.state.evaluated.matched
    del m.rules["user"]["group_ids"]  # incomplete profile: the rule is unknown and could hide priority
    p = _plan(m)
    assert not p.ok and any("rule 9000006002" in e and "group_ids" in e for e in p.errors)


def test_tags_and_approval_state_are_empty_on_create():
    m = meta()
    _add(m, 9000006003, {"any": [_c("tags", "ticket", "selected"), _c("approval_status", "ticket", "selected")]},
         [a._a("hide", ["subject"])])
    p = _plan(m)
    assert p.ok, p.errors
    assert 9000006003 not in p.state.evaluated.matched


@pytest.mark.parametrize("widget", ["tags", "approval_request", "add_child_ticket", "edit_service_item"])
def test_widget_hide_warns_with_rule_id(widget):
    m = meta()
    _add(m, 9000006004, {"all": None}, [a._a("hide", [widget])])
    p = _plan(m)
    assert p.ok, p.errors
    assert any("rule 9000006004" in w and widget in w for w in p.review.warnings)


def test_widget_hide_blocks_when_the_write_sends_it():
    m = meta()
    _add(m, 9000006005, {"all": None}, [a._a("hide", ["config_item_ids"])])
    assert _plan(m).ok
    p = _plan(m, devices=(9000000101,))
    assert not p.ok and any("rule 9000006005" in e and "config_item_ids" in e for e in p.errors)


def test_widget_mandate_blocks():
    m = meta()
    _add(m, 9000006006, {"all": None}, [a._a("mandate", ["tags"])])
    p = _plan(m)
    assert not p.ok and any("rule 9000006006" in e and "could require" in e for e in p.errors)


def test_unknown_context_rule_blocks_only_when_it_touches_the_write():
    m = meta()
    # system key fsv does not know; the target is an empty, optional field
    _add(m, 9000006007, {"all": [_c("channel", "system", "is", "email")]}, [a._a("hide", ["customFields.legacy_flag"])])
    p = _plan(m)
    assert p.ok, p.errors
    assert any("rule 9000006007" in w and "channel" in w for w in p.review.warnings)
    m.rules["business_rules"][-1]["action_data"]["action"] = [a._a("hide", ["subject"])]
    p = _plan(m)
    assert not p.ok and any("rule 9000006007" in e and "Subject" in e for e in p.errors)


def test_unknown_context_rule_could_require_unfilled_field_blocks():
    m = meta()
    _add(m, 9000006008, {"all": [_c("channel", "system", "is", "email")]},
         [a._a("mandate", ["customFields.legacy_flag"])])
    p = _plan(m)
    assert not p.ok and any("rule 9000006008" in e for e in p.errors)


def test_hidden_answer_is_refused_with_rule_id():
    m = meta()  # the fixture's own rule hides Legacy Flag
    p = _plan(m, Draft(answers={**draft().answers, "Legacy Flag": True}))
    assert not p.ok
    assert any(e.startswith("business rules: Legacy Flag: hidden by rule") for e in p.errors)


def test_dry_run_json_lists_rule_warnings(monkeypatch):
    c = Client()
    m = meta()
    _add(m, 9000006010, {"all": None}, [a._a("hide", ["tags"])])
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(change_flow, "interactive", lambda: False)
    monkeypatch.setattr("fsv.ticket_cli.load_meta", lambda *args, **kw: m)
    monkeypatch.setattr(change_flow, "load_meta", lambda *args, **kw: m)
    args = ["tickets", "create", "--requester", "jane.doe@example.com", "--set", "Subject=Service unavailable",
            "--set", "Description=Please help", "--set", "Cloud Only=Yes", "--set", "Stage=Live"]
    res = CliRunner().invoke(cli.app, args + ["--dry-run", "--json"])
    assert res.exit_code == 0, res.output
    assert "9000006010" in json.dumps(json.loads(res.output))
    assert not c.writes


# -- transition readback -----------------------------------------------------

NOTE = "<div>Fixed by restarting the Acme service</div>"


def _row(status, **kw):
    row = {"status": status, "group_id": 9000000201, "responder_id": None, "resolution_notes": NOTE,
           "custom_fields": {}}
    row.update(kw)
    return row


def test_readback_exact_status_verifies():
    payload = {"status": 4, "group_id": 9000000201, "responder_id": None, "br_validation_excludes": "x"}
    assert ticket_lifecycle.readback(_row(2), _row(4), payload, NOTE) == (True, [])


def test_readback_routing_rewrite_is_verified_with_note():
    payload = {"status": 4, "group_id": 9000000201, "responder_id": None, "br_validation_excludes": "x"}
    ok, notes = ticket_lifecycle.readback(_row(2), _row(4, group_id=9000000202), payload, NOTE)
    assert ok and any("group_id" in n for n in notes)


def test_readback_workflow_status_rewrite_is_verified_with_note():
    payload = {"status": 4, "group_id": 9000000201, "responder_id": None, "br_validation_excludes": "x"}
    ok, notes = ticket_lifecycle.readback(_row(2), _row(5), payload, NOTE)
    assert ok and any("rewritten" in n for n in notes)


def test_readback_unchanged_status_or_lost_note_is_a_mismatch():
    payload = {"status": 4, "group_id": 9000000201, "responder_id": None, "br_validation_excludes": "x"}
    assert ticket_lifecycle.readback(_row(2), _row(2), payload, NOTE)[0] is False
    assert ticket_lifecycle.readback(_row(2), _row(4, resolution_notes=""), payload, NOTE)[0] is False
    assert ticket_lifecycle.readback(None, _row(5), payload, NOTE)[0] is False
