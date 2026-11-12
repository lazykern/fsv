"""Server-shaped mail readbacks and retained legacy create guards."""
import json
from dataclasses import replace

import httpx
import pytest
from typer.testing import CliRunner

from fsv import attachments as flow, cli, config, journal
from fsv.reviewed_write import ReviewedWriteError
from test_attachments_reviewed import api, builder  # noqa: F401


@pytest.mark.parametrize("event", ["started", "done"])
def test_prior_create_guard_blocks_new_attachment_hash(api, event):
    c, s, paths, _ = api
    legacy = "a" * 64
    journal.record_plan(event, legacy, action="create", target=0)
    build = lambda: replace(builder(c, paths, "change")(), prior_guards=(("create", legacy),))
    p = build()
    with pytest.raises(ReviewedWriteError, match="prior attachment/create"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert not s["writes"]


def test_new_transaction_also_persists_legacy_guard_before_upload(api):
    c, s, paths, _ = api
    legacy = "b" * 64
    build = lambda: replace(builder(c, paths, "change")(), prior_guards=(("create", legacy),))
    p = build()
    s["fail"] = "disconnect"
    with pytest.raises(ReviewedWriteError, match="outcome unknown"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    events = [e for e in journal.read() if e.get("kind") == "plan:create" and e.get("plan") == legacy]
    assert events[-1]["event"] == "started"
    assert len(s["writes"]) == 1


@pytest.mark.parametrize("kind", ["reply", "note"])
def test_attached_mail_native_formatted_sender_and_notification_readback(api, kind):
    c, s, paths, _ = api
    original = c._client._transport.handler
    def handler(req):
        response = original(req)
        if req.method == "GET" and req.url.path.endswith("/conversations"):
            raw = response.json()
            for row in raw["conversations"]:
                if "from_email" in row:
                    row["from_email"] = '"Acme Support" <SUPPORT@example.com>'
                if "to_emails" in row:
                    row["to_emails"] = ["Acme Person <PERSON@example.com>"]
            return httpx.Response(200, json=raw)
        return response
    c._client._transport.handler = handler
    build = lambda: flow.conversation_plan(c, "INC-123", "Acme evidence", paths, kind=kind,
                sender="support@example.com" if kind == "reply" else None,
                notify=("person@example.com",) if kind == "note" else ())
    p = build()
    result = flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert result["verified"], result
    assert len(s["writes"]) == 3


def test_change_initial_upload_cli_full_review_and_exact_owner(api, monkeypatch):
    import acme_change_form as A
    from fsv import change_flow
    from test_ticket_create import WHO
    c, s, paths, _ = api
    metadata = change_flow.load_meta(A.FakeClient(), "change", "new", fresh=True)
    monkeypatch.setattr("fsv.change_cli.load_meta", lambda *a, **kw: metadata)
    monkeypatch.setattr("fsv.change_cli.resolve_requester", lambda *a, **kw: WHO)
    monkeypatch.setattr(change_flow, "interactive", lambda: False)
    original_get = c.int_get
    monkeypatch.setattr(c, "int_get", lambda path, params=None: {"attachment_settings": {"max_attachment_size_bytes": 4096}}
                        if path == "change_form_fields" else original_get(path, params))
    args = ["changes", "create", "--requester", "jane.doe@example.com", "--set", "Subject=Acme",
            "--description", "Acme work", "--set", "Cloud Only=Yes", "--set", "Stage=Live",
            "--set", "Planned Start Date=2030-11-26T19:00:00+00:00",
            "--set", "Planned End Date=2030-11-26T22:00:00+00:00", "--attach", paths[0],
            "--attach-flow", "before-create", "--max-bytes", "4096", "--json"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    p = json.loads(review.stdout)
    assert p["path"] == "/api/_/changes" and not s["writes"]
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", p["plan_hash"]])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.stdout)["verified"]
    assert [w[:2] for w in s["writes"]] == [("POST", "attachments"), ("POST", "changes")]
    assert all(type(i) is int for i in s["writes"][-1][2]["attachments"])

@pytest.mark.parametrize("mode", ["normalized", "wrong-requester", "conflicting-requester"])
def test_initial_owner_requester_and_timestamp_readback(api, mode):
    from fsv.reviewed_write import WritePlan
    from test_ticket_create import WHO
    c, s, paths, _ = api
    original = c._client._transport.handler
    def handler(req):
        response = original(req)
        if req.method == "GET" and req.url.path.endswith("/tickets/123"):
            raw = response.json()
            row = raw["ticket"]
            row["planned_start_date"] = "2030-01-02T15:00:00Z"
            if mode == "wrong-requester":
                row["requester_id"] = 999
            elif mode == "conflicting-requester":
                row["requester"] = {"id": WHO.id, "email": "wrong@example.com"}
            return httpx.Response(200, json=raw)
        return response
    c._client._transport.handler = handler
    def build():
        base = WritePlan("ticket_create_attachments", 0, config.DOMAIN, "POST", "tickets",
                         json.dumps({"subject": "Acme", "email": WHO.email,
                                     "planned_start_date": "2030-01-02T10:00:00-05:00"}), "synthetic")
        return replace(flow.wrap(base, paths, {"max_attachment_size_bytes": 4096}, client=c), requester_id=int(WHO.id))
    p = build()
    result = flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert result["verified"] is (mode == "normalized")
    assert result["fields"]["planned_start_date"] == "matched"
    assert result["fields"]["email"] == ("matched" if mode == "normalized" else "mismatched")
    assert len(s["writes"]) == 3


def test_legacy_after_create_does_not_ignore_explicit_size_limit(api):
    c, s, paths, _ = api
    result = CliRunner().invoke(cli.app, ["changes", "create", "--subject", "Acme", "--attach", paths[0],
                                         "--max-bytes", "4096", "--yes"])
    assert result.exit_code == 2 and "--attach-flow before-create" in result.output
    assert not s["writes"]
