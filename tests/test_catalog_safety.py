"""Offline catalog review and uploaded-receipt safety boundaries."""
from __future__ import annotations

import json
import time

import pytest
from typer.testing import CliRunner

import acme_catalog as acme
import fsv.change_flow as flow
import fsv.cli_sr as cli_sr
from fsv import catalog
from fsv.drafts import Draft, draft_from_data
from fsv.errors import APIError
from fsv.formmeta import RULES_UNAVAILABLE_KEY
from fsv.forms import FormState, LookupValue, PayloadError, build_sr_payload


def item():
    return {
        "id": 9000000001, "display_id": 9000000002, "name": "Acme request",
        "allow_attachments": True, "configs": {"attachment_mandatory": False},
        "custom_fields": [
            {"id": 9000000003, "name": "answer", "label": "Answer", "type": 1001,
             "required": False, "account_id": 9000000004},
            {"id": 9000000005, "name": "deck", "label": "Deck", "type": 1014,
             "required": False, "account_id": 9000000004,
             "field_options": {"allowed_file_types": '[".pptx"]', "max_attachment_limit": "1"}},
        ],
    }


def rule(condition, actions):
    return {"id": 9000000006, "name": "Acme rule", "module_type": "service_request",
            "item_id": 9000000001, "state": "active", "user_type": "both", "form_type": "both",
            "rule_type": "client_side", "condition_data": {"all": condition},
            "action_data": {"action": actions}}


def meta(rules=None, *, source="server", raw=None):
    return catalog.ItemMeta(raw or item(), rules if rules is not None else {"business_rules": []},
                            None, time.time(), source=source)


def plan(m):
    return catalog.build_sr_plan(m, Draft(answers={"Answer": "ok"}), resolver=None,
                                 requester=LookupValue(id=9000000007, display="Acme", email="a@example.com"))


def test_unknown_negative_condition_and_target_block_and_are_reported():
    cond = {"name": "customField.cf_missing", "operator": "is_not", "value": ["x"], "evaluate_on": "change"}
    action = {"name": ["customField.cf_missing_target"], "property": "hide"}
    m = meta({"business_rules": [rule([cond], [action])]})
    p = plan(m)
    assert not p.ok and not p.hash and p.to_json()["payload"] is None
    assert any("unknown field" in w for w in p.review.warnings)
    assert any("unknown target" in w for w in p.review.warnings)
    assert any("unavailable fields or context" in e for e in p.errors)
    doc = cli_sr.fields_doc(m)
    assert doc["item"]["attachments"]["effective_mandatory"] is None
    assert any("unknown field" in w for w in doc["warnings"])


@pytest.mark.parametrize("source", ["current_user", "requester", "requested_for", "system"])
def test_missing_context_in_short_circuited_group_is_blocking(source):
    condition = [
        {"name": "customField.answer", "operator": "is", "value": ["not selected"], "evaluate_on": "change"},
        {"name": "department_id", "operator": "is_not", "value": ["x"], "evaluate_on": source},
    ]
    m = meta({"business_rules": [rule(condition, [{"name": ["attachment_ids"], "property": "unmandate"}])],
              "user": {"first_name": "SensitiveName", "secret": "SensitiveToken"}})
    p = plan(m)
    assert not p.ok
    assert any("context" in w for w in p.review.warnings)
    combined = json.dumps({"review": p.review.to_dict(), "fields": cli_sr.fields_doc(m)})
    assert "SensitiveName" not in combined and "SensitiveToken" not in combined


def test_empty_rules_differ_from_unavailable_and_explicit_static():
    verified = plan(meta())
    assert verified.ok
    unavailable = {"business_rules": [], RULES_UNAVAILABLE_KEY: "HTTP 403"}
    blocked = plan(meta(unavailable))
    assert not blocked.ok and any("unavailable" in e for e in blocked.errors)
    assert any("static" in w for w in blocked.review.warnings)
    static = plan(meta(unavailable, source="static"))
    assert static.ok and any("static" in w for w in static.review.warnings)
    assert cli_sr.fields_doc(meta(unavailable))["item"]["attachments"]["effective_mandatory"] is None
    assert any("requested-for" in w for w in verified.review.warnings)
    with pytest.raises(ValueError, match="requested_for"):
        draft_from_data({"requested_for": "person@example.com"})


def test_conditional_attachment_description_uses_evaluated_user_without_disclosure():
    raw = item()
    raw["configs"]["attachment_mandatory"] = True
    condition = [{"name": "department_id", "operator": "includes", "value": ["9000000008"],
                  "evaluate_on": "current_user"}]
    actions = [{"name": ["attachment_ids"], "property": "unmandate"}]
    rules = {"business_rules": [rule(condition, actions)],
             "user": {"department_id": ["9000000008"], "secret": "SensitiveToken"}}
    m = meta(rules, raw=raw)
    doc = cli_sr.fields_doc(m)
    attachments = doc["item"]["attachments"]
    assert attachments["mandatory"] is True and attachments["effective_mandatory"] is False
    assert "effective at load: optional" in attachments["rule"]
    assert plan(m).ok
    assert "SensitiveToken" not in json.dumps(doc)
    rules["user"] = {"department_id": ["9000000009"]}
    m = meta(rules, raw=raw)
    assert cli_sr.fields_doc(m)["item"]["attachments"]["effective_mandatory"] is True
    assert not plan(m).ok
    rules["user"] = {}
    m = meta(rules, raw=raw)
    assert cli_sr.fields_doc(m)["item"]["attachments"]["effective_mandatory"] is None
    assert not plan(m).ok


@pytest.mark.parametrize("incomplete", [False, True])
def test_uploaded_receipt_count_always_enforced(incomplete):
    m = meta()
    state = FormState(m.spec, m.engine())
    with pytest.raises(PayloadError, match="at most 1"):
        build_sr_payload(m.spec.item, state, "a@example.com",
                         field_attachments={"deck": [9000000010, 9000000011]}, allow_incomplete=incomplete)
    body = build_sr_payload(m.spec.item, state, "a@example.com",
                            field_attachments={"deck": [9000000010]}, allow_incomplete=incomplete)
    assert body["custom_fields"]["deck"] == [9000000010]


def test_benign_warnings_do_not_block():
    m = meta()
    p = plan(m)
    assert p.ok and p.review.warnings


def test_rules_unavailable_fake_client_is_not_verified_empty(monkeypatch):
    class Fake:
        def int_get(self, path, params=None):
            if path.endswith("business_rules"):
                raise APIError(403, "denied", "GET", path)
            return {}
    monkeypatch.setattr(catalog, "configured_workspace", lambda: None)
    m = catalog.load_item_meta(Fake(), 9000000002, item=item())
    assert m.rules.get(RULES_UNAVAILABLE_KEY)
    p = plan(m)
    assert not p.ok and any("unavailable" in e for e in p.errors)


def test_freshness_rechecks_rules_before_an_upload(monkeypatch):
    original = plan(meta())
    assert original.ok
    changed = meta({"business_rules": [], RULES_UNAVAILABLE_KEY: "HTTP 403"})
    monkeypatch.setattr(catalog, "load_item_meta", lambda client, ref: changed)
    new, diff = catalog.freshen(original, object(), plan)
    assert diff and not new.ok and not new.hash


def test_known_cf_alias_resolves_without_blocking():
    condition = [{"name": "customField.cf_answer", "operator": "is", "value": ["ok"],
                  "evaluate_on": "change"}]
    actions = [{"name": ["customField.cf_answer"], "property": "show"}]
    p = plan(meta({"business_rules": [rule(condition, actions)]}))
    assert p.ok, p.errors


@pytest.mark.parametrize("visible", [True, False])
@pytest.mark.parametrize("code", [501, 502])
def test_answered_unverified_lookup_never_creates_sendable_plan(visible, code):
    raw = item()
    raw["custom_fields"].append({"id": 9000000020, "name": "asset", "label": "Asset", "type": code,
                                 "account_id": 9000000004, "visible": visible, "required": False,
                                 "field_options": {"link": "/lookup_choices"}})
    m = meta(raw=raw)
    blank = plan(m)
    assert blank.ok and any("lookup submission is unverified" in w for w in blank.review.warnings)
    value = 9000000021 if code == 501 else [9000000021]
    p = catalog.build_sr_plan(m, Draft(answers={"Asset": value}), resolver=None,
                              requester=LookupValue(id=9000000007, email="a@example.com"))
    assert not p.ok and not p.hash and p.to_json()["payload"] is None
    assert any("lookup submission is unverified" in e for e in p.errors)


def test_required_unverified_lookup_cannot_be_omitted():
    raw = item()
    raw["custom_fields"].append({"id": 9000000020, "name": "asset", "label": "Asset", "type": 501,
                                 "account_id": 9000000004, "required": True})
    assert any("Asset is required" in e for e in plan(meta(raw=raw)).errors)


def test_execute_rechecks_unverified_lookup_before_any_write(tmp_path):
    raw = item()
    raw["custom_fields"].append({"id": 9000000020, "name": "asset", "label": "Asset", "type": 501,
                                 "account_id": 9000000004, "required": False})
    file = tmp_path / "doc.pdf"
    file.write_bytes(b"doc")
    m = meta(raw=raw)
    p = catalog.build_sr_plan(m, Draft(attachments=[str(file)]), resolver=None,
                              requester=LookupValue(id=9000000007, email="a@example.com"))
    assert p.ok
    p.draft.answers["Asset"] = 9000000021
    writes = []
    with pytest.raises(flow.FlowExit, match="lookup submission is unverified"):
        catalog.execute(p, object(), upload_fn=lambda *a: (writes.append(a), 9000000022)[1])
    assert not writes


def test_execute_rechecks_file_digest_before_upload(tmp_path):
    file = tmp_path / "doc.pdf"
    file.write_bytes(b"doc")
    p = catalog.build_sr_plan(meta(), Draft(attachments=[str(file)]), resolver=None,
                              requester=LookupValue(id=9000000007, email="a@example.com"))
    assert p.ok
    file.write_bytes(b"changed")
    writes = []
    with pytest.raises(flow.FlowExit, match="changed"):
        catalog.execute(p, object(), upload_fn=lambda *a: (writes.append(a), 9000000022)[1])
    assert not writes


def test_cli_dry_run_never_uploads_and_does_not_leak_rule_user(monkeypatch, tmp_path):
    fake = acme.AcmeCatalog()
    monkeypatch.setattr(cli_sr, "_client", lambda: fake.client())
    monkeypatch.setattr(flow, "interactive", lambda: False)
    original = acme.rules_for
    def rules(item_id):
        result = original(item_id)
        result["user"]["secret"] = "SensitiveToken"
        return result
    monkeypatch.setattr(acme, "rules_for", rules)
    file = tmp_path / "deck.pptx"
    file.write_bytes(b"file")
    draft = tmp_path / "request.yaml"
    draft.write_text("requester: jane.doe@example.com\nanswers:\n  Title: ok\n  Details: ok\n"
                     "  Needed by: 2030-12-03\n  1. Expected benefit: Other\n  Priority: Medium\n"
                     f"  Overview deck (.pptx): {file}\nattachments: [{file}]\n")
    result = CliRunner().invoke(cli_sr.app, ["create", "77", "--from", str(draft), "--dry-run", "-o", "json"])
    assert result.exit_code == 0, result.output
    assert "SensitiveToken" not in result.output
    assert not fake.writes()
