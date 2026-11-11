"""Offline incident create contracts with invented tenant metadata."""
from __future__ import annotations

import json

import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.change_flow as change_flow
import fsv.cli as cli
from fsv import journal, ticket_flow
from fsv.drafts import Draft
from fsv.errors import APIError
from fsv.formmeta import FormMeta
from fsv.forms import (FormState, LookupValue, RulesEngine, build_br_validation_excludes,
                       decode_br_validation_excludes)
from fsv.formui.session import ticket_session


def meta():
    fields = a.fields()["change_fields"]
    keep = {"requester", "subject", "description", "priority", "impact", "status", "cloud_only", "stage", "staging_start", "staging_end", "legacy_flag", "reviewer"}
    fields = [f for f in fields if f["name"] in keep]
    rules = a.rules()
    rules["business_rules"] = rules["business_rules"][:3]
    for r in rules["business_rules"]:
        r["module_type"] = "ticket"
    return FormMeta("ticket", "new_form", {"ticket_fields": fields}, rules, 2, 0, "fixture")


WHO = LookupValue(9001, "Jane Doe", "jane.doe@example.com")


class Client:
    def __init__(self):
        self.writes = []
        self.failure = None
    def _get_csrf(self):
        return "synthetic-token"
    def _request(self, method, url, **kw):
        self.writes.append((method, url, json.loads(kw["content"])))
        if self.failure:
            raise self.failure
        return {"ticket": {"id": 123}}
    def autocomplete(self, kind, query, params):
        return [{"id": 9001, "value": "Jane Doe", "email": "jane.doe@example.com"}]


def draft():
    return Draft(answers={"Subject": "Service unavailable", "Description": "Please help", "Cloud Only": "Yes", "Stage": "Live", "Legacy Flag": True})


def plan(m=None, d=None, who=WHO):
    return ticket_flow.build_create_plan(m or meta(), d or draft(), resolver=None, requester=who)


@pytest.fixture(autouse=True)
def isolated_journal(monkeypatch, tmp_path):
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal.jsonl")


def test_wire_payload_metadata_rules_and_hidden():
    p = plan()
    assert p.ok, p.errors
    assert p.payload["subject"] == "Service unavailable"
    assert p.payload["description"] == "<div>Please help</div>"
    assert p.payload["email"] == "Jane Doe <jane.doe@example.com>"
    assert p.payload["status"] == 1
    assert "legacy_flag" not in p.payload["custom_fields"]
    assert decode_br_validation_excludes(p.payload["br_validation_excludes"])["hidden"] == [
        f"legacy_flag_{a.ACCOUNT}", f"staging_start_{a.ACCOUNT}", f"staging_end_{a.ACCOUNT}"]
    assert len(p.hash) == 64


def test_required_identity_and_dynamic_validation():
    assert not plan(who=None).ok
    p = plan(d=Draft(answers={"Subject": "x", "Cloud Only": "No"}))
    assert any("Description is required" in e for e in p.errors)
    assert any("Staging Start is required" in e for e in p.errors)
    p = plan(d=Draft(answers={**draft().answers, "Requester": "other@example.com"}))
    assert any("not answers" in e for e in p.errors)


def test_form_session_matches_plan_and_supports_save(tmp_path):
    c = Client()
    s = ticket_session(c, meta=meta(), requester=WHO, dry_run=True)
    assert not s.set("subject", "Service unavailable").error
    assert not s.set("description", "Please help").error
    assert not s.set("cloud_only", "Yes").error
    assert not s.set("stage", "Live").error
    p = s.build_plan()
    assert p.ok, p.errors
    assert p.payload["email"].endswith("<jane.doe@example.com>")
    path = tmp_path / "incident.yaml"
    s.save_draft(str(path), p)
    assert "tickets create" in s.command_for(str(path), p)
    assert path.exists()
    with pytest.raises(change_flow.FlowExit):
        s.send(p)


def test_single_write_success_duplicate_and_unknown(monkeypatch):
    c, p = Client(), plan()
    ticket_flow.execute(p, c)
    assert len(c.writes) == 1 and c.writes[0][0] == "POST" and c.writes[0][1].endswith("/tickets")
    with pytest.raises(change_flow.FlowExit):
        ticket_flow.execute(p, c)
    ticket_flow.execute(p, c, allow_duplicate=True)
    assert len(c.writes) == 2
    q = plan(d=Draft(answers={**draft().answers, "Subject": "Another incident"}))
    c.failure = TimeoutError("sensitive body")
    with pytest.raises(change_flow.FlowExit, match="outcome unknown"):
        ticket_flow.execute(q, c)
    with pytest.raises(change_flow.FlowExit, match="outcome is unknown"):
        ticket_flow.execute(q, c, allow_duplicate=True)
    assert len(c.writes) == 3
    assert "sensitive body" not in journal.journal_path().read_text()


def test_403_never_retries_and_failure_is_safe():
    c = Client()
    c.failure = APIError(403, "do not print response body")
    with pytest.raises(change_flow.FlowExit, match="HTTP 403") as exc:
        ticket_flow.execute(plan(), c)
    assert "response body" not in str(exc.value)
    assert len(c.writes) == 1
    assert journal.read()[-1]["event"] == "failed"


def test_cli_dry_run_and_review_hash(monkeypatch):
    c = Client()
    m = meta()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(change_flow, "interactive", lambda: False)
    monkeypatch.setattr("fsv.ticket_cli.load_meta", lambda *args, **kw: m)
    monkeypatch.setattr(change_flow, "load_meta", lambda *args, **kw: m)
    args = ["tickets", "create", "--requester", "jane.doe@example.com", "--set", "Subject=Service unavailable", "--set", "Description=Please help", "--set", "Cloud Only=Yes", "--set", "Stage=Live"]
    runner = CliRunner()
    res = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert res.exit_code == 0, res.output
    doc = json.loads(res.output)
    assert not c.writes
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    assert runner.invoke(cli.app, args + ["--yes", "--expect-plan", "0" * 16]).exit_code == 4
    res = runner.invoke(cli.app, args + ["--yes", "--expect-plan", doc["plan_hash"][:16]])
    assert res.exit_code == 0, res.output
    assert len(c.writes) == 1


def test_missing_rule_context_and_unknown_target_fail_closed():
    m = meta()
    m.rules["business_rules"].append(a._rule(9000004990, "Portal condition",
        {"all": [{"name": "portal", "evaluate_on": "system", "operator": "is", "value": "42"}]},
        [a._a("hide", ["customFields.unknown_field"])]))
    m.rules["business_rules"][-1]["module_type"] = "ticket"
    p = plan(m)
    assert not p.ok and any("unavailable fields or context" in e for e in p.errors)
    assert not p.hash


@pytest.mark.parametrize("tenant", ["Acme", "Globex"])
@pytest.mark.parametrize("source", ["system", "current_user", "requester", "requested_for"])
def test_short_circuited_missing_context_fails_closed(tenant, source, monkeypatch):
    m = meta()
    r = a._rule(9000004991, tenant + " context", {"any": [
        {"name": "subject", "evaluate_on": "ticket", "operator": "selected"},
        {"all": [{"name": "region", "evaluate_on": source, "operator": "is", "value": "west"}]},
    ]}, [a._a("hide", ["priority"])])
    r["module_type"] = "ticket"
    m.rules["business_rules"].append(r)
    # A nonempty but incomplete mapping must not be mistaken for known context.
    context_key = "user" if source == "current_user" else source
    def engine():
        from fsv.forms import RuleContext
        context = RuleContext(module="ticket", workspace_id=2)
        from dataclasses import replace
        return RulesEngine(m.spec, m.rules, context=replace(context, **{context_key: {"other": "value"}}))
    monkeypatch.setattr(m, "engine", engine)
    p = plan(m)
    assert not any("none was given" in w for w in p.state.rules.warnings)
    assert not p.ok and not p.hash
    assert any("unavailable fields or context" in e for e in p.errors)
    with pytest.raises(change_flow.FlowExit):
        ticket_flow.execute(p, Client())


def test_supplied_context_allows_plan_and_edit_only_context_is_ignored(monkeypatch):
    from fsv.forms import RuleContext
    m = meta()
    rule = a._rule(9000004995, "Portal", {"all": [
        {"name": "portal", "evaluate_on": "system", "operator": "is", "value": "east"},
    ]}, [a._a("hide", ["priority"])])
    rule["module_type"] = "ticket"
    m.rules["business_rules"].append(rule)
    monkeypatch.setattr(m, "engine", lambda: RulesEngine(
        m.spec, m.rules, context=RuleContext(module="ticket", workspace_id=2, system={"portal": "west"})))
    assert plan(m).ok
    rule["form_type"] = "edit_form"
    monkeypatch.setattr(m, "engine", lambda: RulesEngine(m.spec, m.rules, module="ticket", workspace_id=2))
    assert plan(m).ok


@pytest.mark.parametrize("tenant", ["Acme", "Globex"])
def test_unknown_condition_and_target_independently_fail_closed(tenant):
    for unknown in ("condition", "target"):
        m = meta()
        condition = "customFields.missing" if unknown == "condition" else "subject"
        target = "customFields.missing" if unknown == "target" else "priority"
        r = a._rule(9000004992, tenant + " unknown", {"all": [
            {"name": condition, "evaluate_on": "ticket", "operator": "selected"},
        ]}, [a._a("hide", [target])])
        r["module_type"] = "ticket"
        m.rules["business_rules"].append(r)
        p = plan(m)
        assert not p.ok and not p.hash
        assert any("unavailable fields or context" in e for e in p.errors)


def test_ticket_token_preserves_answer_event_order():
    m = meta()
    m.rules["business_rules"] = []
    for rid, name, field, value, target in (
        (9000004993, "Cloud", "customFields.cloud_only", "No", "customFields.staging_start"),
        (9000004994, "Stage", "customFields.stage", "Live", "customFields.staging_end"),
    ):
        r = a._rule(rid, name, {"all": [{"name": field, "evaluate_on": "ticket",
                                           "operator": "is", "value": value}]},
                    [a._a("hide", [target])])
        r["module_type"] = "ticket"
        m.rules["business_rules"].append(r)
    st = FormState(m.spec, m.engine(), event_order="history")
    st.set("stage", "Live")
    st.set("cloud_only", "No")
    token = build_br_validation_excludes(st.evaluated, "ticket")
    assert decode_br_validation_excludes(token)["hidden"] == [
        f"staging_end_{a.ACCOUNT}", f"staging_start_{a.ACCOUNT}"]
    # Independent compact JSON encoding, not a copied token.
    import base64
    payload = {"optional": "[]", "disabled": "[]",
               "hidden": json.dumps([f"staging_end_{a.ACCOUNT}", f"staging_start_{a.ACCOUNT}"])}
    enc = lambda s: base64.b64encode(s.encode()).decode()
    assert token == enc('{"alg":"HS256","typ":"JWT"}') + "." + enc(json.dumps(payload, separators=(",", ":")))


def test_real_metadata_loader_uses_ticket_query(monkeypatch):
    from fsv.formmeta import load_meta
    m = meta()
    calls = []
    class MetadataClient:
        def int_get(self, path, params=None):
            calls.append((path, params))
            if path == "ticket_form_fields":
                return m.form_fields
            if path.endswith("business_rules"):
                return m.rules
            raise AssertionError(path)
    loaded = load_meta(MetadataClient(), "tickets", "new", fresh=True)
    assert loaded.module == "ticket" and loaded.raw_fields
    rule_query = next(params for path, params in calls if path.endswith("business_rules"))
    assert rule_query["module_type"] == "ticket" and rule_query["form_type"] == "new_form"


def test_deprecated_values_are_refused():
    m = meta()
    next(f for f in m.raw_fields if f["name"] == "subject")["deprecated"] = True
    p = plan(m)
    assert not p.ok and any("deprecated" in e for e in p.errors)


def test_metadata_change_aborts_before_write(monkeypatch):
    c, original = Client(), meta()
    changed = meta()
    changed.rules["business_rules"][0]["updated_at"] = "2031-01-01T00:00:00Z"
    next(f for f in changed.raw_fields if f["name"] == "reviewer")["required"] = True
    monkeypatch.setattr(change_flow, "load_meta", lambda *a, **kw: changed)
    p = plan(original)
    with pytest.raises(change_flow.FlowExit):
        ticket_flow.send(p, c, lambda m: plan(m))
    assert not c.writes


def test_missing_response_id_blocks_even_allowed_duplicate():
    c = Client()
    c._request = lambda *args, **kwargs: {}
    p = plan()
    with pytest.raises(change_flow.FlowExit, match="no ID"):
        ticket_flow.execute(p, c)
    with pytest.raises(change_flow.FlowExit, match="outcome is unknown"):
        ticket_flow.execute(p, c, allow_duplicate=True)


def test_verbose_client_does_not_log_body_or_retry(monkeypatch, capsys):
    import httpx
    import fsv.client as client_module
    from fsv import config
    seen = []
    monkeypatch.setattr(config, "DOMAIN", "acme.freshservice.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.freshservice.com/api/_")
    c = client_module.Client(cookies={"fw-session-id": "synthetic-cookie"})
    c._client.close()
    def handler(request):
        seen.append(json.loads(request.content))
        return httpx.Response(403, json={"message": "rejected"})
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic-token"
    monkeypatch.setattr(c, "_probe_session", lambda: True)
    monkeypatch.setattr(client_module, "_VERBOSE", True)
    with pytest.raises(change_flow.FlowExit):
        ticket_flow.execute(plan(), c)
    logs = capsys.readouterr().err
    assert len(seen) == 1
    assert "Service unavailable" not in logs and "synthetic-cookie" not in logs and "synthetic-token" not in logs
    c.close()


def test_textual_form_renders_and_reacts_to_rules():
    from formui_drive import focus, run_app, settle, type_text
    s = ticket_session(Client(), meta=meta(), requester=WHO, dry_run=True)
    async def script(app, pilot):
        await focus(app, pilot, "cloud_only")
        await type_text(pilot, "no")
        await pilot.press("enter")
        await settle(app, pilot)
        assert app.screen.rows["staging_start"].display
        assert "staging_start" in s.state.evaluated.required
    run_app(s, script)
