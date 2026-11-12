"""bulk status --set uses per-ticket reviewed property writes; --json stdout is pure JSON."""
import copy
import json

import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.cli as cli
from fsv import config, journal, ticket_properties as props
from fsv.errors import APIError
from fsv.formmeta import FormMeta


def metadata():
    fields = [a._f("status", "Status", "default_status", 1, choices=[{"id": 31, "value": "Open"}, {"id": 47, "value": "Pending"}]),
              a._f("reason", "Pending Reason", "custom_text", 2)]
    for index, field in enumerate(fields, 9000007201):
        field["id"] = index
    # Real tenant shape: a rule mandates the reason when the status becomes Pending.
    cond = {"all": [{"name": "status", "operator": "is", "value": "47", "evaluate_on": "ticket"}]}
    rule = a._rule(9000007210, "Pending needs reason", cond, [a._a("mandate", ["customFields.reason"])])
    rule["module_type"], rule["workspace_id"] = "ticket", 1
    return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": [rule]}, 1, 0, "fixture", 123)


class Client:
    def __init__(self):
        self.rows = {i: {"id": i, "subject": f"Acme {i}", "status": 31, "custom_fields": {"reason": None}} for i in (71, 72)}
        self.writes = []
        self.fail_on = None
    def int_get(self, path, params=None):
        return {"ticket": copy.deepcopy(self.rows[int(path.rsplit("/", 1)[1])])}
    def _get_csrf(self):
        return "synthetic"
    def _request(self, method, path, **kw):
        body = json.loads(kw["content"])
        ident = int(path.rsplit("/", 1)[1])
        self.writes.append((method, ident, body))
        if ident == self.fail_on:
            raise APIError(400, {"errors": [{"field": "status", "message": "Synthetic rejection"}]})
        row = self.rows[ident]
        for key, value in body.items():
            if key == "custom_fields":
                row[key].update(value)
            elif key != "br_validation_excludes":
                row[key] = value
        return {"ticket": copy.deepcopy(row)}


@pytest.fixture
def client(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: metadata())
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    return c


BASE = ["tickets", "bulk", "status", "71", "72", "--status", "Pending"]


def test_rule_required_answer_is_refused_without_set(client):
    r = CliRunner().invoke(cli.app, BASE + ["--dry-run"])
    assert r.exit_code == 2 and "Pending Reason" in r.output and not client.writes


def test_json_stdout_is_pure_json_and_set_sends_per_ticket(client):
    runner = CliRunner()
    args = BASE + ["--set", "Pending Reason=Waiting on Acme vendor", "--json"]
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    body = json.loads(review.stdout)  # the whole of stdout parses
    assert "Selected ticket IDs" not in review.stdout and "Selected ticket IDs: 71, 72" in review.stderr
    assert body["ticket_ids"] == [71, 72] and [t["id"] for t in body["selected_tickets"]] == [71, 72]
    assert all(p["custom_fields"] == {"reason": "Waiting on Acme vendor"} and p["status"] == 47 for p in body["payload"])
    assert not client.writes
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", body["plan_hash"]])
    assert sent.exit_code == 0, sent.output
    result = json.loads(sent.stdout)
    assert result["verified"] and set(result["tickets"]) == {"71", "72"}
    assert [(m, i, b["status"], b["custom_fields"]) for m, i, b in client.writes] == [
        ("PUT", 71, 47, {"reason": "Waiting on Acme vendor"}), ("PUT", 72, 47, {"reason": "Waiting on Acme vendor"})]
    # The same reviewed plan is never replayed.
    again = runner.invoke(cli.app, args + ["--yes", "--expect-plan", body["plan_hash"]])
    assert again.exit_code != 0 and len(client.writes) == 2


def test_text_mode_keeps_selection_on_stderr_only(client):
    r = CliRunner().invoke(cli.app, BASE + ["--set", "Pending Reason=Waiting", "--dry-run"])
    assert r.exit_code == 0
    assert "Selected ticket IDs: 71, 72" in r.stderr and "Selected ticket IDs" not in r.stdout
    json.loads(r.stdout)


def test_first_failure_stops_and_reports_unattempted(client):
    client.fail_on = 71
    runner = CliRunner()
    args = BASE + ["--set", "Pending Reason=Waiting", "--json"]
    digest = json.loads(runner.invoke(cli.app, args + ["--dry-run"]).stdout)["plan_hash"]
    r = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    result = json.loads(r.stdout)
    assert r.exit_code == 4 and result["status"] == "partial" and result["stopped_at"] == 71
    assert result["not_attempted"] == [72] and "HTTP 400" in result["tickets"]["71"]["error"]
    assert [i for _, i, _ in client.writes] == [71]


@pytest.mark.parametrize("extra", [["--set", "Status=Open"], ["--set", "novalue"]])
def test_set_validation_never_writes(client, extra):
    r = CliRunner().invoke(cli.app, BASE + extra + ["--dry-run"])
    assert r.exit_code == 2 and not client.writes
    r = CliRunner().invoke(cli.app, ["tickets", "bulk", "spam", "71", "--set", "Pending Reason=x", "--dry-run"])
    assert r.exit_code == 2 and not client.writes
