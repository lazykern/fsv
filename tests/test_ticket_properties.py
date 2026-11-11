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
