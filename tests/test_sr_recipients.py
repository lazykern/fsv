"""SR recipient options preserve requester and participate in review."""
import json

import pytest
from typer.testing import CliRunner

import fsv.change_flow as flow
import fsv.cli_sr as cli
from fsv import catalog
from fsv.drafts import Draft
from fsv.forms import LookupValue
from test_catalog_safety import meta, rule
from test_sr_create import fake, work, dry, run, places

ACTOR = LookupValue(11, "Actor", "actor@example.com")


@pytest.mark.parametrize("target,cc", [("target@example.com", "team@example.com"),
                                       ("target@globex.example.com", "ops@globex.example.com")])
def test_plan_payload_and_hash(target, cc):
    d = Draft(answers={"Answer": "ok"})
    p = catalog.build_sr_plan(meta(), d, resolver=None, requester=ACTOR,
                              requested_for=LookupValue(12, "Target", target), cc_emails=(cc,))
    base = catalog.build_sr_plan(meta(), d, resolver=None, requester=ACTOR)
    assert p.ok, p.errors
    assert p.payload["email"] == ACTOR.email
    assert p.payload["requested_for"] == target
    assert p.payload["cc_emails"] == [cc]
    assert p.hash != base.hash
    assert ("CC", cc) in p.review.rows


def test_requested_for_rule_context_fails_closed():
    condition = [{"name": "department_id", "operator": "is", "value": ["9"], "evaluate_on": "requested_for"}]
    m = meta({"business_rules": [rule(condition, [{"name": ["answer"], "property": "hide"}])],
              "requested_for": {"department_id": "9"}})
    p = catalog.build_sr_plan(m, Draft(answers={"Answer": "ok"}), resolver=None, requester=ACTOR,
                              requested_for=LookupValue(12, "Target", "target@example.com"))
    assert not p.ok and not p.hash
    assert any("context" in e for e in p.errors)


def test_invalid_cc_and_missing_target_email_block():
    p = catalog.build_sr_plan(meta(), Draft(), resolver=None, requester=ACTOR,
                              requested_for=LookupValue(12, "Target", None), cc_emails=("bad",))
    assert not p.ok and not p.hash
    assert any("CC" in e for e in p.errors)
    assert any("requested-for" in e for e in p.errors)


class Client:
    def __init__(self, ambiguous=False):
        self.ambiguous = ambiguous
    def autocomplete(self, kind, query, params):
        if query == ACTOR.email:
            return [{"id": 11, "value": "Actor", "email": ACTOR.email}]
        if query == "team@example.com":
            return [{"id": 13, "value": "Team", "email": query}]
        if query == "target@example.com":
            rows = [{"id": 12, "value": "Target", "email": query}]
            return rows * (2 if self.ambiguous else 1)
        return []


def test_cli_exact_lookup_and_ambiguous_fail_closed(monkeypatch):
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.setattr(catalog, "load_item_meta", lambda *a, **kw: meta())
    args = ["create", "77", "--requester", ACTOR.email, "--set", "Answer=ok", "--requested-for", "target@example.com", "--cc", "team@example.com", "--dry-run", "--output", "json"]
    runner = CliRunner()
    result = runner.invoke(cli.app, args)
    assert result.exit_code == 0, result.output
    body = json.loads(result.output)["payload"]
    assert body["email"] == ACTOR.email and body["requested_for"] == "target@example.com"
    assert body["cc_emails"] == ["team@example.com"]
    c.ambiguous = True
    result = runner.invoke(cli.app, args)
    assert result.exit_code == 2 and "several exact matches" in result.output
    c.ambiguous = False
    original = c.autocomplete
    c.autocomplete = lambda kind, query, params: (original(kind, query, params) * 2 if query == "team@example.com" else original(kind, query, params))
    result = runner.invoke(cli.app, args)
    assert result.exit_code == 2 and "each CC email" in result.output


def test_submit_preserves_reviewed_recipients(fake, work):
    args = ("77", "--from", "sr.yaml", "--requested-for", "jane.doe@example.com", "--cc", "jane.doe@example.com")
    doc = dry(*args)
    result = run("create", *args, "--yes", "--expect-plan", doc["plan_hash"])
    assert result.exit_code == 0, result.output
    writes = places(fake)
    assert len(writes) == 1
    payload = writes[0][2]
    assert payload["email"] == "jane.doe@example.com"
    assert payload["requested_for"] == "jane.doe@example.com"
    assert payload["cc_emails"] == ["jane.doe@example.com"]


def test_ambiguous_failure_keeps_started_guard_even_when_duplicate_allowed(fake, work):
    from fsv import journal
    fake.fail_place = 403
    args = ("77", "--from", "sr.yaml", "--requested-for", "jane.doe@example.com", "--cc", "jane.doe@example.com")
    result = run("create", *args, "--yes")
    assert result.exit_code != 0 and "outcome is unknown" in result.flat
    assert [r for r in journal.read() if r.get("kind") == "plan:create"][-1]["event"] == "started"
    count = len(fake.writes())
    result = run("create", *args, "--yes", "--allow-duplicate")
    assert result.exit_code == 4 and "outcome is unknown" in result.flat
    assert len(fake.writes()) == count


def test_relookup_changed_identity_blocks_before_upload(monkeypatch, tmp_path):
    from fsv import journal
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "j")
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.setattr(catalog, "load_item_meta", lambda *a, **kw: meta())
    original = c.autocomplete
    counts = {"target": 0}
    def shifting(kind, query, params):
        rows = original(kind, query, params)
        if query == "target@example.com":
            counts["target"] += 1
            if counts["target"] > 1:
                rows[0]["id"] = 99
        return rows
    c.autocomplete = shifting
    args = ["create", "77", "--requester", ACTOR.email, "--set", "Answer=ok", "--requested-for", "target@example.com", "--yes"]
    result = CliRunner().invoke(cli.app, args)
    assert result.exit_code == 4 and "identities changed" in result.output
    assert not journal.read()


def test_recipient_readback_requires_exact_ticket_and_fields(monkeypatch, tmp_path):
    from fsv import journal
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "j")
    class Place:
        def int_post(self, path, body):
            self.body = body
            return {"ticket": {"id": 123}}
        def int_get(self, path):
            return {"ticket": {"id": 123, "email": ACTOR.email,
                               "requested_for": "target@example.com", "cc_emails": ["team@example.com"]}}
    c = Place()
    p = catalog.build_sr_plan(meta(), Draft(answers={"Answer": "ok"}), resolver=None, requester=ACTOR,
                              requested_for=LookupValue(12, "Target", "target@example.com"),
                              cc_emails=("team@example.com",), cc_ids=(13,))
    result = catalog.execute(p, c)
    assert result["verification"]["verified"] is True
    assert all(v == "matched" for v in result["verification"]["fields"].values())
