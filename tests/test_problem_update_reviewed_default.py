"""`problems update` of form fields is reviewed by default (synthetic tenant):
only changed keys, plan hash with a record-state token, one journaled send,
independent readback per changed key (`verified`), replay refusals exit 2,
no-op says nothing to change; `--no-review` is the deprecated body-only path."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, formmeta, journal, problem_properties as flow
from fsv.client import Client

SCHEMA = {"fields": [
    {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": True},
    {"name": "description", "label": "Description", "field_type": "default_description", "default_field": True},
    {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": True,
     "choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]},
    {"name": "status", "label": "Status", "field_type": "default_status", "default_field": True,
     "choices": [{"id": 31, "value": "Open"}]},
    {"name": "acme_code", "label": "Account code", "field_type": "custom_text", "default_field": False},
]}
FORM = [{"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1},
        {"id": 502, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
         "choices": [{"id": 31, "value": "Open"}]},
        {"id": 503, "name": "priority", "label": "Priority", "field_type": "default_priority", "position": 3,
         "choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]},
        {"id": 504, "name": "description", "label": "Description", "field_type": "default_description",
         "position": 4},
        {"id": 505, "name": "acme_code", "label": "Account code", "field_type": "custom_text", "position": 5,
         "default_field": False}]


@pytest.fixture
def api(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(config, "CACHE_DIR", tmp_path / "cache")
    monkeypatch.setattr(formmeta, "configured_workspace", lambda: 17)
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr("fsv.schema.load", lambda *a, **k: SCHEMA)
    s = {"row": {"id": 123, "subject": "Acme problem", "description": "<div>Acme outage</div>", "status": 31,
                 "priority": 1, "custom_fields": {"acme_code": "A1"}, "deleted": False,
                 "updated_at": "2030-01-01T00:00:00Z"},
         "writes": [], "fail": None, "apply": True, "version": 0}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "problem_form_fields":
                return httpx.Response(200, json={"problem_fields": copy.deepcopy(FORM)})
            if path.endswith("business_rules"):
                return httpx.Response(200, json={"business_rules": []})
            assert path == "problems/123"
            return httpx.Response(200, json={"problem": copy.deepcopy(s["row"])})
        body = json.loads(req.content)
        s["writes"].append((req.method, path, body))
        if s["fail"]:
            return httpx.Response(s["fail"], json={"message": "synthetic outage"})
        if s["apply"]:
            body = copy.deepcopy(body)
            custom = body.pop("custom_fields", {})
            s["row"].update(body)
            s["row"]["custom_fields"].update(custom)
            if "description" in body:
                s["row"]["description"] = "<div>" + body["description"] + "</div>"  # server wraps rich text
            s["version"] += 1
            s["row"]["updated_at"] = f"2030-01-01T00:00:{s['version']:02d}Z"
        return httpx.Response(200, json={"problem": {"id": 123}})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield s
    c.close()


def run(*args):
    return CliRunner().invoke(cli.app, ["problems", "update", *args])


def review(*args):
    r = run("PRB-123", *args, "--dry-run")
    assert r.exit_code == 0, r.output
    return json.loads(r.stdout)


def send(*args):
    digest = review(*args)["plan_hash"]
    return digest, run("PRB-123", *args, "--yes", "--expect-plan", digest, "--json")


def test_dry_run_reviews_only_changed_keys_with_record_state(api):
    plan = review("--priority", "High", "--set", "Subject=Acme problem", "--set", "Account code=B2")
    assert plan["action"] == "problem_update" and len(plan["plan_hash"]) == 64
    assert plan["payload"] == {"priority": 3, "custom_fields": {"acme_code": "B2"}}  # subject unchanged: omitted
    assert plan["before"] == {"priority": 1, "custom_fields": {"acme_code": "A1"}}
    assert plan["record_state"] == {"sends": 0, "server_version": "2030-01-01T00:00:00Z"}
    assert plan["path"] == "/api/_/problems/123" and not api["writes"]


def test_reviewed_alias_is_a_no_op(api):
    assert review("--priority", "High", "--reviewed")["plan_hash"] == review("--priority", "High")["plan_hash"]


def test_send_verifies_each_changed_key_and_replay_exits_2(api):
    args = ("--priority", "High", "--set", "Account code=B2", "--set", "Description=Acme root cause")
    digest, sent = send(*args)
    assert sent.exit_code == 0, sent.output
    result = json.loads(sent.stdout)
    assert result["verified"] is True and result["plan_hash"] == digest
    assert result["fields"] == {"priority": "matched", "custom_fields.acme_code": "matched", "description": "matched"}
    assert api["writes"] == [("PUT", "problems/123", {"priority": 3, "custom_fields": {"acme_code": "B2"},
                                                      "description": "Acme root cause"})]
    replay = run("PRB-123", *args, "--yes", "--expect-plan", digest)
    assert replay.exit_code == 2 and "already sent" in replay.output and len(api["writes"]) == 1
    events = [e.get("event") for e in journal.read() if e.get("kind") == "plan:problem_update"]
    assert events == ["started", "done"]


def test_ab_alternation_four_times_plans_distinct_hashes(api):
    hashes = []
    for value in ["High", "Low", "High", "Low"]:
        digest, sent = send("--priority", value)
        assert sent.exit_code == 0, sent.output
        hashes.append(digest)
    assert len(set(hashes)) == 4 and [w[2] for w in api["writes"]] == [{"priority": p} for p in (3, 1, 3, 1)]


def test_ambiguous_outcome_is_not_replayable(api):
    api["fail"] = 503
    digest, first = send("--priority", "High")
    assert first.exit_code == 2 and "outcome unknown" in first.output and "HTTP 503" in first.output
    api["fail"] = None
    again = review("--priority", "High")
    assert again["plan_hash"] == digest  # nothing moved, nothing verified: the same plan
    second = run("PRB-123", "--priority", "High", "--yes", "--expect-plan", digest)
    assert second.exit_code == 2 and "unknown" in second.output and len(api["writes"]) == 1


def test_unverified_readback_exits_4(api):
    api["apply"] = False
    digest, sent = send("--priority", "High")
    assert sent.exit_code == 4 and json.loads(sent.stdout)["fields"] == {"priority": "mismatched"}
    assert review("--priority", "High")["plan_hash"] == digest


def test_no_op_says_nothing_to_change(api):
    r = run("PRB-123", "--priority", "Low", "--set", "Account code=A1", "--dry-run", "--json")
    out = json.loads(r.stdout)
    assert r.exit_code == 0 and out["status"] == "nothing_to_change" and out["target"] == 123 and "plan_hash" not in out
    r = run("PRB-123", "--priority", "Low", "--dry-run")
    assert r.exit_code == 0 and "nothing to change" in r.output and "plan_hash" not in r.output
    r = run("PRB-123", "--priority", "Low", "--yes", "--expect-plan", "a" * 64)
    assert r.exit_code == 2 and "does not exist" in r.output and not api["writes"]
    digest, sent = send("--priority", "High")
    assert sent.exit_code == 0
    r = run("PRB-123", "--priority", "High", "--yes", "--expect-plan", digest)
    assert r.exit_code == 2 and "already sent" in r.output and len(api["writes"]) == 1


def test_status_goes_through_its_own_command_and_other_refusals(api):
    r = run("PRB-123", "--status", "Open", "--dry-run")
    assert r.exit_code == 2 and "problems status" in r.output
    r = run("PRB-123", "--tag", "acme", "--dry-run")
    assert r.exit_code == 2 and "only for tickets" in r.output
    r = run("PRB-123", "--priority", "High", "--ack", "--dry-run")
    assert r.exit_code == 2 and "ticket" in r.output
    assert run("PRB-123", "--priority", "High", "--yes").exit_code == 2  # --yes needs the full hash
    assert not api["writes"]


def test_field_missing_from_form_metadata_refuses(api):
    global FORM
    saved = FORM
    FORM = [f for f in FORM if f["name"] != "acme_code"]
    try:
        r = run("PRB-123", "--set", "Account code=B2", "--dry-run")
    finally:
        FORM = saved
    assert r.exit_code == 2 and "no acme_code field" in r.output and not api["writes"]


def test_no_review_prints_one_notice_and_sends_body_only(api):
    dry = run("--no-review", "PRB-123", "--priority", "High", "--dry-run", "--json")
    assert dry.exit_code == 0, dry.output
    assert json.loads(dry.stdout)["body"] == {"priority": 3}
    notice = [line for line in dry.stderr.splitlines() if line.strip()]
    assert len(notice) == 1 and "problems update --no-review is deprecated" in notice[0]
    sent = run("--no-review", "PRB-123", "--priority", "High", "--status", "Open", "--yes")
    assert sent.exit_code == 0, sent.output
    assert "--no-review is deprecated" in sent.stderr
    assert api["writes"] == [("PUT", "problems/123", {"status": 31, "priority": 3})]
    assert not [e for e in journal.read() if str(e.get("kind", "")).startswith("plan:")]
    r = run("--no-review", "PRB-123", "--planned-effort", "1h", "--dry-run")
    assert r.exit_code == 2 and "drop --no-review" in r.output


def test_planning_keys_stay_on_their_own_reviewed_plan(api):
    with pytest.raises(flow.ReviewedWriteError, match="on their own"):
        flow.build_update_plan(None, 123, {"planned_effort": "1h 0m"})
