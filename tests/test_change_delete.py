"""Reviewed change soft-delete: captured bodyless DELETE and exact deleted readback."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import change_delete as flow, cli, config, journal
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=[("acme.example.com", 123), ("globex.example.com", 742)])
def api(request, monkeypatch, tmp_path):
    domain, cid = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"cid": cid, "row": {"id": cid, "subject": "Acme rollout", "deleted": False}, "writes": [],
         "lag": 0, "fail": None, "reads_after": 0}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            assert path == f"changes/{cid}", path
            row = dict(s["row"])
            if s["writes"]:
                s["reads_after"] += 1
                if s["lag"]:
                    s["lag"] -= 1
                    row["deleted"] = False
            return httpx.Response(200, json={"change": row})
        s["writes"].append(req)
        if s["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=req)
        if s["fail"]:
            return httpx.Response(s["fail"], json={"errors": "synthetic"})
        s["row"]["deleted"] = True
        return httpx.Response(204, headers={"content-type": "application/json"})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s
    c.close()


def test_exact_bodyless_delete_wire_and_independent_readback(api):
    c, s = api
    p = flow.build_plan(c, f"CHN-{s['cid']}")
    review = flow.review(p, c)
    assert review["method"] == "DELETE" and review["path"] == f"/api/_/changes/{s['cid']}"
    assert review["payload"] is None and review["subject"] == "Acme rollout"
    assert not s["writes"]
    result = flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert result["verified"] and result["fields"] == {"deleted": "matched"}
    assert len(s["writes"]) == 1
    sent = s["writes"][0]
    assert sent.method == "DELETE" and sent.url.path == f"/api/_/changes/{s['cid']}" and sent.content == b""
    assert sent.headers["content-type"] == "application/json" and sent.headers["x-csrf-token"] == "synthetic"
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert len(s["writes"]) == 1


def test_destructive_confirmation_is_required(api):
    c, s = api
    p = flow.build_plan(c, s["cid"])
    with pytest.raises(ReviewedWriteError, match="confirm-destructive"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert not s["writes"]


@pytest.mark.parametrize("row", [{"deleted": True}, {"deleted": None}, {"deleted": "false"}, {"subject": None}])
def test_already_deleted_or_unproven_target_never_writes(api, row):
    c, s = api
    s["row"].update(row)
    if "deleted" in row and row["deleted"] is None:
        s["row"].pop("deleted")
    with pytest.raises(ReviewedWriteError):
        flow.build_plan(c, s["cid"])
    assert not s["writes"]


def test_target_drift_after_review_blocks_send(api):
    c, s = api
    p = flow.build_plan(c, s["cid"])
    s["row"]["subject"] = "Globex changed"
    with pytest.raises(ReviewedWriteError, match="changed since review"):
        flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert not s["writes"]


@pytest.mark.parametrize("lag,verified,attempts", [(2, True, 3), (100, False, 4)])
def test_lagging_soft_delete_rereads_only(api, lag, verified, attempts):
    c, s = api
    p = flow.build_plan(c, s["cid"])
    s["lag"] = lag
    result = flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert result["verified"] is verified and result["readback_attempts"] == attempts
    assert len(s["writes"]) == 1 and s["reads_after"] >= attempts


@pytest.mark.parametrize("fail,match", [(403, "unknown"), (429, "unknown"), (422, "rejected"), ("disconnect", "unknown")])
def test_failures_are_never_replayed(api, fail, match):
    c, s = api
    p = flow.build_plan(c, s["cid"])
    s["fail"] = fail
    with pytest.raises(ReviewedWriteError, match=match):
        flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    if match == "unknown":
        with pytest.raises(ReviewedWriteError, match="was sent"):
            flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert len(s["writes"]) == 1


def test_cli_review_then_confirmed_send(api):
    c, s = api
    runner = CliRunner()
    base = ["changes", "delete", f"CHN-{s['cid']}", "--json"]
    review = runner.invoke(cli.app, base + ["--dry-run"])
    assert review.exit_code == 0, review.output
    plan = json.loads(review.stdout)
    unconfirmed = runner.invoke(cli.app, base + ["--yes", "--expect-plan", plan["plan_hash"]])
    assert unconfirmed.exit_code == 2 and not s["writes"]
    wrong = runner.invoke(cli.app, base + ["--confirm-destructive", "--yes", "--expect-plan", "0" * 64])
    assert wrong.exit_code == 2 and not s["writes"]
    sent = runner.invoke(cli.app, base + ["--confirm-destructive", "--yes", "--expect-plan", plan["plan_hash"]])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.stdout)["verified"] and len(s["writes"]) == 1
