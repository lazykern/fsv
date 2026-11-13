"""Explicit bulk-priority single-attempt contract and per-ticket readback."""
import json
from types import SimpleNamespace

import httpx
import pytest
from typer.testing import CliRunner

from fsv import bulk_priority as bulk, cli, config, formmeta
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client


@pytest.fixture(params=[("acme.example.com", 123, 87), ("globex.example.com", 742, 26)])
def wire(request, monkeypatch):
    domain, cid, high = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr(formmeta, "configured_workspace", lambda: 1)
    w = SimpleNamespace(ids=[cid, cid + 1], high=high, seen=[], failure=None, partial=False,
        rows={cid: {"id": cid, "subject": "Acme one", "priority": 1},
              cid + 1: {"id": cid + 1, "subject": "Globex two", "priority": 1}})
    def handler(req):
        w.seen.append(req)
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "PUT":
            assert path == "bulk_ticket_actions/update_multiple"
            if w.failure:
                return httpx.Response(w.failure, json={"error": "synthetic denial"})
            body = json.loads(req.content)
            for tid in body["ids"][:1] if w.partial else body["ids"]:
                w.rows[tid]["priority"] = body["priority"]
            return httpx.Response(200, json={})
        if path == "ticket_form_fields":
            return httpx.Response(200, json={"ticket_fields": [
                {"id": cid + 9000000000, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1},
                {"id": cid + 9000000001, "name": "priority", "label": "Priority", "field_type": "default_priority", "position": 2,
                 "choices": [{"id": 1, "value": "Low"}, {"id": high, "value": "High"}]}]})
        if path.endswith("business_rules"):
            return httpx.Response(200, json={"business_rules": []})
        assert path.startswith("tickets/"), path
        return httpx.Response(200, json={"ticket": w.rows[int(path.split("/")[1])]})
    w.client = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: w.client)
    yield w
    w.client.close()


def test_exact_wire_and_fresh_per_target_readback(wire):
    w = wire
    p = bulk.build_plan(w.client, w.ids, "High")
    assert not [r for r in w.seen if r.method == "PUT"]
    assert p.write.payload == {"priority": w.high, "ids": w.ids, "reply": {}}
    assert len(p.review()["before"]) == 2
    result = bulk.send(p, w.client, reviewed_hash=p.hash)
    assert result["verified"] and set(result["tickets"].values()) == {"matched"}
    writes = [r for r in w.seen if r.method == "PUT"]
    assert len(writes) == 1 and json.loads(writes[0].content) == p.write.payload
    assert writes[0].headers["x-csrf-token"] == "synthetic-token"


def test_partial_is_not_success_and_is_not_resent(wire):
    w = wire
    w.partial = True
    p = bulk.build_plan(w.client, w.ids, "High")
    result = bulk.send(p, w.client, reviewed_hash=p.hash)
    assert result["verified"] is False and result["tickets"][str(w.ids[1])] == "mismatched"
    with pytest.raises(ReviewedWriteError, match="already sent at"):
        bulk.send(p, w.client, reviewed_hash=p.hash)
    assert len([r for r in w.seen if r.method == "PUT"]) == 1


@pytest.mark.parametrize("failure", [403, 429, 500])
def test_bulk_no_error_replay(wire, failure):
    p = bulk.build_plan(wire.client, wire.ids, "High")
    wire.failure = failure
    for _ in range(2):
        with pytest.raises(ReviewedWriteError, match="unknown"):
            bulk.send(p, wire.client, reviewed_hash=p.hash)
    assert len([r for r in wire.seen if r.method == "PUT"]) == 1


def test_cli_requires_exact_hash_and_never_expands_filter(wire):
    w = wire
    runner = CliRunner()
    args = ["tickets", "bulk-priority", *map(str, w.ids), "--priority", "High"]
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    assert not [r for r in w.seen if r.method == "PUT"]
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert result.exit_code == 0, result.output
    assert len([r for r in w.seen if r.method == "PUT"]) == 1


def test_stale_target_and_duplicates_fail_before_send(wire):
    w = wire
    p = bulk.build_plan(w.client, w.ids, "High")
    w.rows[w.ids[1]]["subject"] = "changed"
    with pytest.raises(ReviewedWriteError, match="changed"):
        bulk.send(p, w.client, reviewed_hash=p.hash)
    with pytest.raises(ValueError, match="unique"):
        bulk.build_plan(w.client, [w.ids[0], w.ids[0]], "High")
    assert not [r for r in w.seen if r.method == "PUT"]


@pytest.mark.parametrize("lag,verified", [(1, True), (100, False)])
def test_lagging_readback_rereads_only_and_never_resends(wire, monkeypatch, lag, verified):
    w = wire
    p = bulk.build_plan(w.client, w.ids, "High")
    real = dict(w.rows)
    stale = {"left": lag}
    def lagging(cid):
        def get(*_a, **_k):
            if stale["left"]:
                stale["left"] -= 1
                return {**real[cid], "priority": 1}
            return real[cid]
        return get
    from fsv import ticket_properties
    original = ticket_properties._record
    writes = lambda: [r for r in w.seen if r.method != "GET"]
    monkeypatch.setattr(ticket_properties, "_record",
                        lambda client, cid: lagging(cid)() if writes() else original(client, cid))
    result = bulk.send(p, w.client, reviewed_hash=p.hash)
    assert result["verified"] is verified
    assert result["readback_attempts"] == (2 if verified else 4)
    assert len(writes()) == 1
    with pytest.raises(ReviewedWriteError):
        bulk.send(p, w.client, reviewed_hash=p.hash)
    assert len(writes()) == 1
