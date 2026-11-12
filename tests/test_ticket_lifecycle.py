"""Synthetic closure schemas and single-attempt transaction regressions."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.cli as cli
from fsv import config, journal, ticket_lifecycle as flow
from fsv.errors import APIError
from fsv.formmeta import FormMeta
from fsv.readback import resolution_text
from fsv.reviewed_write import ReviewedWriteError


def metadata():
    fields = [a._f("status", "Status", "default_status", 1, choices=[
        {"id": 31, "value": "Open"}, {"id": 47, "value": "Pending"},
        {"id": 62, "value": "Resolved"}, {"id": 89, "value": "Closed"}]),
        a._f("summary", "Resolution Summary", "custom_text", 2),
        a._f("reason", "Pending Reason", "custom_text", 3)]
    for index, field in enumerate(fields, 9000007101):
        field["id"] = index
    fields[1]["required_for_closure"] = True
    return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 1, 0, "fixture", 123)


class Client:
    def __init__(self):
        self.row = {"id": 123, "status": 31, "group_id": None, "responder_id": 9000001234,
                    "resolution_notes": None, "custom_fields": {"summary": "Acme fix", "reason": "", "untouched": "keep"}}
        self.writes = []
        self.fail_at = None
        self.bad_note = False
        self.bad_status = False
        self.change_after_note = False
    def int_get(self, path):
        assert path == "tickets/123"
        return {"ticket": copy.deepcopy(self.row)}
    def _get_csrf(self):
        return "synthetic-token"
    def _request(self, method, path, **kwargs):
        assert method == "PUT" and path == config.API_INT + "/tickets/123"
        body = json.loads(kwargs["content"])
        self.writes.append(body)
        if len(self.writes) == self.fail_at:
            raise APIError(403, "synthetic rejection")
        for key, value in body.items():
            if key == "resolution_notes":
                self.row[key] = {"notes_html": "wrong" if self.bad_note else value,
                                 "user": {"name": "Acme Operator", "email": "operator@example.com"}}
                if self.change_after_note:
                    self.row["group_id"] = 9000005678
            elif key == "custom_fields":
                self.row[key].update(value)
            elif key != "br_validation_excludes" and not (key == "status" and self.bad_status):
                self.row[key] = value
        return {"ticket": copy.deepcopy(self.row)}


@pytest.fixture(autouse=True)
def setup(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(flow, "load_meta", lambda *args, **kwargs: metadata())


@pytest.mark.parametrize("tenant,prefix", [("acme.example.com", "INC"), ("globex.example.com", "SR")])
def test_resolution_persists_then_transitions(tenant, prefix, monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", tenant)
    c = Client()
    p = flow.build_plan(c, prefix + "-123", "Resolved", note="Acme fix <safe>", answers={"summary": "Globex fix"})
    assert not c.writes and len(p.review()["steps"]) == 2
    assert "&lt;safe&gt;" in p.note_html and p.payload["status"] == 62
    assert p.payload["custom_fields"] == {"summary": "Globex fix"}
    result = flow.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] and len(c.writes) == 2
    assert set(c.writes[0]) == {"resolution_notes", "br_validation_excludes"}
    assert c.row["custom_fields"]["untouched"] == "keep"
    assert journal.read()[-1]["event"] == "done"
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(c.writes) == 2


@pytest.mark.parametrize("status,expected", [("Closed", 89), ("Open", 31), ("Pending", 47)])
def test_single_transition_preserves_resolution(status, expected):
    c = Client()
    c.row["resolution_notes"] = {"notes_html": "<p>Earlier fix</p>"}
    p = flow.build_plan(c, 123, status, answers={"reason": "Waiting for self"} if status == "Pending" else {})
    assert flow.send(p, c, reviewed_hash=p.hash)["verified"]
    assert len(c.writes) == 1 and c.row["status"] == expected
    assert resolution_text(c.row) == "<p>Earlier fix</p>"


@pytest.mark.parametrize("label", ["Resolved", "Closed"])
def test_required_closure_fields_and_note_fail_closed(label):
    c = Client()
    c.row["custom_fields"]["summary"] = ""
    with pytest.raises(ReviewedWriteError, match=r"Resolution Summary \(summary\)"):
        flow.build_plan(c, 123, label, note="fix" if label == "Resolved" else None)
    c.row["custom_fields"]["summary"] = "fix"
    with pytest.raises(ReviewedWriteError, match="persisted resolution"):
        flow.build_plan(c, 123, label)
    assert not c.writes


@pytest.mark.parametrize("value", [None, {"notes_html": None}, {"unexpected": "fix"}, ""])
def test_missing_resolution_is_not_a_match(value):
    c = Client()
    c.row["resolution_notes"] = value
    with pytest.raises(ReviewedWriteError, match="resolution note"):
        flow.build_plan(c, 123, "Closed")
    assert not c.writes


@pytest.mark.parametrize("bad,report", [
    ("bad_note", ["resolution note save outcome unknown", "transition not attempted"]),
    # SR shape: a workflow/approval rewrote routing when the note saved, so the
    # status write was never sent and the status stayed where it was.
    ("change_after_note", ["resolution note saved and verified; status unchanged (readback status 31)",
                           "group_id", "tickets status 123 Resolved --dry-run", "Do not resend this plan"]),
    ("bad_status", ["resolution note saved; status unchanged (readback status 31)", "held by a tenant workflow or rule",
                    "tickets activity 123"]),
    ("fail_first", ["resolution note save outcome unknown (HTTP 403: synthetic rejection)"]),
    ("fail_second", ["resolution note saved; the status write was rejected (HTTP 403: synthetic rejection)",
                     "readback status 31"]),
])
def test_partial_transactions_stay_started_and_never_replay(bad, report):
    c = Client()
    if bad.startswith("fail"):
        c.fail_at = 1 if bad == "fail_first" else 2
    else:
        setattr(c, bad, True)
    p = flow.build_plan(c, 123, "Resolved", note="fix")
    with pytest.raises(ReviewedWriteError) as exc:
        flow.send(p, c, reviewed_hash=p.hash)
    assert all(part in str(exc.value) for part in report), str(exc.value)
    count = len(c.writes)
    assert count == (2 if bad in {"bad_status", "fail_second"} else 1)
    assert journal.read()[-1]["event"] == "started"
    with pytest.raises(ReviewedWriteError, match="outcome unknown"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(c.writes) == count


def test_stale_metadata_record_hash_consent_and_journal(monkeypatch):
    c = Client()
    p = flow.build_plan(c, 123, "Open")
    assert flow.send(p, c, reviewed_hash=None)["status"] == "cancelled"
    with pytest.raises(ReviewedWriteError, match="full review"):
        flow.send(p, c, reviewed_hash=p.hash[:12])
    c.row["subject"] = "changed"
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(p, c, reviewed_hash=p.hash)
    c.row.pop("subject")
    m = metadata()
    m.form_fields["ticket_fields"][1]["label"] = "Changed requirement"
    monkeypatch.setattr(flow, "load_meta", lambda *args, **kwargs: m)
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(p, c, reviewed_hash=p.hash)
    monkeypatch.setattr(flow, "load_meta", lambda *args, **kwargs: metadata())
    monkeypatch.setattr(journal, "record_plan", lambda *args, **kwargs: None)
    with pytest.raises(ReviewedWriteError, match="journal"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert not c.writes


@pytest.mark.parametrize("args", [
    ["resolve", "SR-123", "--note", "Acme resolution", "--set", "summary=fix"],
    ["close", "123"], ["reopen", "123"], ["status", "123", "Pending", "--set", "reason=Waiting"]])
def test_cli_review_full_hash_and_send(monkeypatch, args):
    c = Client()
    c.row["resolution_notes"] = {"notes_html": "Existing fix"}
    monkeypatch.setattr(cli, "_client", lambda: c)
    runner = CliRunner()
    command = ["tickets", *args]
    reviewed = runner.invoke(cli.app, command + ["--dry-run"])
    assert reviewed.exit_code == 0, reviewed.output
    digest = json.loads(reviewed.output)["plan_hash"]
    assert not c.writes
    assert runner.invoke(cli.app, command + ["--yes"]).exit_code == 2
    assert runner.invoke(cli.app, command + ["--yes", "--expect-plan", digest[:10]]).exit_code == 2
    sent = runner.invoke(cli.app, command + ["--yes", "--expect-plan", digest])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.output)["verified"]


def test_cli_lists_missing_required_custom_fields(monkeypatch):
    c = Client()
    c.row["custom_fields"]["summary"] = None
    monkeypatch.setattr(cli, "_client", lambda: c)
    r = CliRunner().invoke(cli.app, ["tickets", "resolve", "123", "--note", "fix", "--dry-run"])
    assert r.exit_code == 2 and "summary" in r.output and "--set" in r.output
    assert not c.writes


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_real_client_transport_never_retries(failure, monkeypatch):
    from test_ticket_notes import mock_client
    requests = []
    c = Client()
    def handler(request):
        requests.append(request)
        if failure == "disconnect":
            raise httpx.ReadTimeout("offline disconnect", request=request)
        return httpx.Response(failure, json={"error": "synthetic failure"})
    actual = mock_client(monkeypatch, handler)
    actual.int_get = c.int_get
    actual._get_csrf = c._get_csrf
    p = flow.build_plan(actual, 123, "Open")
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(p, actual, reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(p, actual, reviewed_hash=p.hash)
    assert len(requests) == 1
    actual.close()


def test_pending_reason_rule_and_hidden_closure_requirements(monkeypatch):
    m = metadata()
    condition = {"all": [{"name": "status", "operator": "is", "value": "47", "evaluate_on": "ticket"}]}
    rule = a._rule(9000007107, "Pending needs reason", condition, [a._a("mandate", ["customFields.reason"])])
    rule["module_type"] = "ticket"
    rule["workspace_id"] = 1
    m.rules["business_rules"] = [rule]
    monkeypatch.setattr(flow, "load_meta", lambda *args, **kwargs: m)
    c = Client()
    with pytest.raises(ReviewedWriteError, match="Pending Reason"):
        flow.build_plan(c, 123, "Pending")
    assert flow.build_plan(c, 123, "Pending", answers={"reason": "Waiting"}).payload["custom_fields"] == {"reason": "Waiting"}
    hide = a._rule(9000007108, "Hide summary", {"all": None}, [a._a("hide", ["customFields.summary"])])
    hide["module_type"] = "ticket"
    hide["workspace_id"] = 1
    m.rules["business_rules"].append(hide)
    m.spec.account_id = "900000777"
    c.row["custom_fields"]["summary"] = None
    assert flow.build_plan(c, 123, "Resolved", note="fix").payload["status"] == 62
    assert not c.writes


def test_unknown_required_field_and_unavailable_rules(monkeypatch):
    m = metadata()
    _ = m.spec
    m.raw_fields  # materialize the known schema before the malformed addition
    m.form_fields["ticket_fields"].append({"name": "unknown_requirement", "required_for_closure": True})
    monkeypatch.setattr(flow, "load_meta", lambda *args, **kwargs: m)
    c = Client()
    with pytest.raises(ReviewedWriteError, match="unknown required fields: unknown_requirement"):
        flow.build_plan(c, 123, "Resolved", note="fix")
    m.rules["unavailable"] = "synthetic failure"
    with pytest.raises(ReviewedWriteError, match="unavailable"):
        flow.build_plan(c, 123, "Open")
    assert not c.writes


@pytest.mark.parametrize("record,expected", [
    ({"resolution_notes": {"notes_html": "<p>Acme fix</p>", "user": {"name": "Operator"}}}, "<p>Acme fix</p>"),
    ({"resolution_notes": "Globex fix"}, "Globex fix"),
    ({"resolution_notes": {"unexpected": "not note text"}}, None)])
def test_resolution_readback_shapes(record, expected):
    assert resolution_text(record) == expected


def test_digit_string_status_ids_send_integer_wire(monkeypatch):
    def string_ids():
        meta = metadata()
        fields = [{**f, "choices": [{**c, "id": str(c["id"])} for c in f["choices"]]} if f.get("choices") else f
                  for f in meta.raw_fields]
        return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 1, 0, "fixture", 123)
    monkeypatch.setattr(flow, "load_meta", lambda *args, **kwargs: string_ids())
    c = Client()
    p = flow.build_plan(c, 123, "Pending", answers={"reason": "Waiting for self"})
    assert p.payload["status"] == 47 and type(p.payload["status"]) is int
    assert flow.send(p, c, reviewed_hash=p.hash)["verified"]
    assert c.writes[-1]["status"] == 47


def _blocking_close_rule(m, status_id="89"):
    # Shape of a real tenant is_validation rule: a message shown when the
    # status becomes Closed, which the web form blocks on until dealt with.
    cond = {"all": [{"name": "status", "operator": "is", "value": status_id, "evaluate_on": "ticket"}]}
    rule = a._rule(9000007120, "Only requester closes", cond,
                   [a._a("message", ["Only the requester closes Acme tickets"])], validation=True)
    rule["module_type"] = "ticket"
    rule["workspace_id"] = 1
    m.rules["business_rules"] = [rule]
    return m


@pytest.mark.parametrize("args", [["close", "123"], ["status", "123", "Closed"]])
def test_blocking_rule_message_needs_explicit_ack_bound_to_review(monkeypatch, args):
    m = _blocking_close_rule(metadata())
    monkeypatch.setattr(flow, "load_meta", lambda *args, **kwargs: m)
    c = Client()
    c.row["resolution_notes"] = {"notes_html": "Existing fix"}
    monkeypatch.setattr(cli, "_client", lambda: c)
    runner = CliRunner()
    command = ["tickets", *args]
    refused = runner.invoke(cli.app, command + ["--dry-run"])
    assert refused.exit_code == 2
    assert "blocked by rule (acknowledge to continue): Only the requester closes" in refused.output
    assert "--ack" in refused.output
    reviewed = runner.invoke(cli.app, command + ["--ack", "--dry-run"])
    assert reviewed.exit_code == 0, reviewed.output
    review = json.loads(reviewed.output)
    assert review["acknowledged_rule_messages"] == ["Only the requester closes Acme tickets"]
    assert "blocks submit" in review["warning"] and not c.writes
    # The acknowledgement is part of the reviewed identity: a hash reviewed
    # with --ack cannot be sent without it.
    assert runner.invoke(cli.app, command + ["--yes", "--expect-plan", review["plan_hash"]]).exit_code == 2
    assert not c.writes
    sent = runner.invoke(cli.app, command + ["--ack", "--yes", "--expect-plan", review["plan_hash"]])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.output)["verified"] and len(c.writes) == 1


def test_unacknowledged_plans_keep_their_context_identity():
    plan = flow.build_plan(Client(), 123, "Pending", answers={"reason": "Waiting"})
    assert set(json.loads(plan.context_hash)) == {"record", "metadata"} and plan.acknowledged == []
