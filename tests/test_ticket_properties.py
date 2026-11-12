"""Offline ticket property reviews and single-attempt writes."""
import json

import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.cli as cli
from fsv import config, journal, ticket_properties as props
from fsv.errors import APIError
from fsv.formmeta import FormMeta


class Client:
    def __init__(self):
        self.row = {"id": 123, "subject": "Test", "tags": ["old"], "planned_start_date": None,
                    "planned_end_date": None}
        self.writes = []
        self.failure = None
    def int_get(self, path):
        return {"ticket": dict(self.row)}
    def _get_csrf(self):
        return "example-token"
    def _request(self, method, path, **kw):
        body = json.loads(kw["content"])
        self.writes.append((method, path, body))
        if self.failure:
            raise self.failure
        self.row.update({k: v for k, v in body.items() if k != "br_validation_excludes"})
        return {"ticket": self.row}


def metadata():
    fields = [a._f("subject", "Subject", "default_subject", 1),
              a._f("planned_start_date", "Start", "default_planned_start_date", 2),
              a._f("planned_end_date", "End", "default_planned_end_date", 3)]
    return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 1, 0, "fixture", 123)


@pytest.fixture(autouse=True)
def setup(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: metadata())


@pytest.mark.parametrize("tags,start", [(["Acme", "one"], "2030-01-02T09:00:00-05:00"),
                                       (["Globex"], "2031-03-04T18:00:00+09:00")])
def test_review_wire_and_readback(tags, start):
    c = Client()
    p = props.build_plan(c, "INC-123", tags=tags, planned_start=start)
    assert p.before == {"tags": ["old"], "planned_start_date": None}
    assert p.payload["tags"] == tags and p.payload["planned_start_date"] == start
    assert p.review()["after"]["tags"] == tags
    result = props.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] and len(c.writes) == 1 and c.writes[0][0] == "PUT"
    with pytest.raises(props.PropertyError, match="was sent"):
        props.send(p, c, reviewed_hash=p.hash)


def test_stale_record_and_ambiguous_403():
    c = Client()
    p = props.build_plan(c, 123, tags=["new"])
    c.row["subject"] = "changed"
    with pytest.raises(props.PropertyError, match="changed"):
        props.send(p, c, reviewed_hash=p.hash)
    assert not c.writes
    q = props.build_plan(c, 123, tags=["new"])
    c.failure = APIError(403, "hidden")
    with pytest.raises(props.PropertyError, match="outcome unknown"):
        props.send(q, c, reviewed_hash=q.hash)
    with pytest.raises(props.PropertyError, match="outcome is unknown"):
        props.send(q, c, reviewed_hash=q.hash)
    assert len(c.writes) == 1 and journal.read()[-1]["event"] == "started"


def test_inputs_require_offsets_and_edit_rules(monkeypatch):
    c = Client()
    with pytest.raises(ValueError, match="offset"):
        props.build_plan(c, 123, planned_start="2030-01-02T09:00")
    with pytest.raises(ValueError):
        props.build_plan(c, 123, tags=[" bad "])
    m = metadata()
    m.rules["__rules_unavailable__"] = "test"
    from fsv.formmeta import RULES_UNAVAILABLE_KEY
    m.rules[RULES_UNAVAILABLE_KEY] = "test"
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: m)
    with pytest.raises(props.PropertyError, match="unavailable"):
        props.build_plan(c, 123, tags=["ok"])
    assert not c.writes


def test_cli_exact_hash_and_no_mixing(monkeypatch):
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    runner = CliRunner()
    args = ["tickets", "update", "123", "--tag", "one", "--tag", "two", "--planned-end", "2030-01-02T10:00:00-05:00"]
    r = runner.invoke(cli.app, args + ["--dry-run"])
    assert r.exit_code == 0, r.output
    doc = json.loads(r.output)
    assert doc["before"]["tags"] == ["old"] and doc["after"]["tags"] == ["one", "two"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    r = runner.invoke(cli.app, args + ["--yes", "--expect-plan", doc["plan_hash"]])
    assert r.exit_code == 0, r.output
    assert len(c.writes) == 1
    assert runner.invoke(cli.app, args + ["--status", "Open", "--dry-run"]).exit_code == 2


def rich_metadata():
    m = metadata()
    m.form_fields["ticket_fields"].extend([
        a._f("priority", "Priority", "default_priority", 4, choices=[{"id": 73, "value": "High"}]),
        a._f("status", "Status", "default_status", 5, choices=[{"id": 81, "value": "Open"}, {"id": 91, "value": "Closed"}]),
        a._f("cost_code", "Cost Code", "custom_text", 6),
    ])
    for index, field in enumerate(m.form_fields["ticket_fields"], 9000008001):
        field["id"] = index
    return m


def test_full_properties_runtime_enums_and_custom_readback(monkeypatch):
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: rich_metadata())
    c = Client()
    c.row.update(priority=1, status=81, custom_fields={"cost_code": "old", "other": "retained"})
    plan = props.build_plan(c, "SR-123", answers={"Subject": "Globex", "Priority": "High", "Cost Code": "new"})
    assert plan.payload["priority"] == 73
    assert plan.payload["subject"] == "Globex"
    assert plan.payload["custom_fields"] == {"cost_code": "new"}
    result = props.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] and len(c.writes) == 1


def test_reviewed_cli_uses_full_hash_and_refuses_unproven_closure(monkeypatch):
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: rich_metadata())
    c = Client()
    c.row.update(status=81, priority=1, custom_fields={"cost_code": "old"})
    monkeypatch.setattr(cli, "_client", lambda: c)
    runner = CliRunner()
    args = ["tickets", "update", "INC-123", "--reviewed", "--set", "Subject=Acme", "--priority", "High"]
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert sent.exit_code == 0, sent.output
    assert len(c.writes) == 1
    with pytest.raises(props.PropertyError, match="closure") as refused:
        props.build_plan(c, 123, answers={"Status": "Closed"})
    # The refusal names the commands that can close, not only the web form.
    assert "fsv tickets close ID" in str(refused.value) and "--ack" in str(refused.value)


def test_reviewed_update_blocking_message_needs_ack(monkeypatch):
    m = rich_metadata()
    cond = {"all": [{"name": "priority", "operator": "is", "value": "73", "evaluate_on": "ticket"}]}
    rule = a._rule(9000008101, "High needs lead", cond, [a._a("message", ["High priority needs the Acme lead"])],
                   validation=True)
    rule["module_type"], rule["workspace_id"] = "ticket", 1
    m.rules["business_rules"] = [rule]
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: m)
    c = Client()
    c.row.update(status=81, priority=1, custom_fields={"cost_code": "old"})
    monkeypatch.setattr(cli, "_client", lambda: c)
    runner = CliRunner()
    args = ["tickets", "update", "INC-123", "--reviewed", "--priority", "High"]
    refused = runner.invoke(cli.app, args + ["--dry-run"])
    assert refused.exit_code == 2 and "acknowledge to continue" in refused.output and "--ack" in refused.output
    review = runner.invoke(cli.app, args + ["--ack", "--dry-run"])
    assert review.exit_code == 0, review.output
    body = json.loads(review.output)
    assert body["acknowledged_rule_messages"] == ["High priority needs the Acme lead"]
    assert runner.invoke(cli.app, args + ["--yes", "--expect-plan", body["plan_hash"]]).exit_code == 2
    sent = runner.invoke(cli.app, args + ["--ack", "--yes", "--expect-plan", body["plan_hash"]])
    assert sent.exit_code == 0, sent.output
    assert len(c.writes) == 1
    assert runner.invoke(cli.app, ["tickets", "update", "INC-123", "--priority", "High", "--ack", "--dry-run"]).exit_code != 0


def test_legacy_property_hash_identity_is_unchanged():
    c = Client()
    plan = props.build_plan(c, 123, tags=["Acme"])
    expected = journal.plan_hash({"tenant": plan.tenant, "ticket_id": plan.ticket_id,
                                  "payload": plan.payload, "before": plan.before,
                                  "record_hash": plan.record_hash})
    assert plan.hash == expected
    journal.record_plan("started", expected, action="ticket_properties", target=123)
    with pytest.raises(props.PropertyError, match="outcome is unknown"):
        props.send(plan, c, reviewed_hash=plan.hash)
    assert not c.writes

