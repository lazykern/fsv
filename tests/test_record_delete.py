"""Offline problem/article delete, restore and delete-forever wires."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, readback, record_delete as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=[("acme.example.com", 12), ("globex.example.com", 34)])
def api(request, monkeypatch, tmp_path):
    domain, pid = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(readback, "_sleep", lambda _s: None)
    aid = 900000 + pid
    s = {"writes": [], "problem": {"id": pid, "subject": "Acme outage", "deleted": False},
         "article": {"id": aid, "title": "Globex workaround", "deleted": False}, "ignore": False, "fail": None}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == f"problems/{pid}":
                return (httpx.Response(200, json={"problem": copy.deepcopy(s["problem"])}) if s["problem"]
                        else httpx.Response(404, json={"status": 404}))
            if path == f"solutions/articles/{aid}":
                return (httpx.Response(200, json={"article": copy.deepcopy(s["article"])}) if s["article"]
                        else httpx.Response(404, json={"status": 404}))
            raise AssertionError(path)
        assert req.headers["content-type"].startswith("application/json") and not req.content
        s["writes"].append((req.method, path, dict(req.url.params)))
        if s["fail"]:
            return httpx.Response(s["fail"], json={"errors": "no"})
        if not s["ignore"]:
            for key in ("problem", "article"):
                if s[key]:
                    s[key]["updated_at"] = f"2030-01-01T00:00:{len(s['writes']):02d}Z"
            if (req.method, path) == ("DELETE", f"problems/{pid}"):
                s["problem"]["deleted"] = True
            elif (req.method, path) == ("PUT", f"problems/{pid}/restore"):
                s["problem"]["deleted"] = False
            elif (req.method, path) == ("DELETE", "problems/delete-forever"):
                assert req.url.params["ids"] == str(pid)
                s["problem"] = None
            elif (req.method, path) == ("DELETE", f"solutions/articles/{aid}"):
                s["article"]["deleted"] = True
            elif (req.method, path) == ("DELETE", f"solutions/articles/{aid}/delete_forever"):
                s["article"] = None
            else:
                raise AssertionError(path)
        return httpx.Response(204)

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s, pid, aid
    c.close()


def test_problem_trash_restore_and_forever_lifecycle(api):
    c, s, pid, _ = api
    for action, method, path, field in [("delete", "DELETE", f"problems/{pid}", "deleted"),
                                        ("restore", "PUT", f"problems/{pid}/restore", "deleted")]:
        p = flow.problem_plan(c, f"PRB-{pid}", action)
        r = flow.send(p, c, reviewed_hash=p.hash)
        assert r["verified"] and r["fields"] == {field: "matched"}
        assert s["writes"][-1][:2] == (method, path)
        with pytest.raises(ReviewedWriteError):
            flow.send(p, c, reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="only to a problem in the trash"):
        flow.problem_plan(c, pid, "delete-forever")
    # An identical record re-reviewed for the same action keeps its hash and the
    # journal refuses it; a trashed record with a different snapshot is a new plan.
    s["problem"].update(deleted=True, subject="Acme outage (trashed)")
    p = flow.problem_plan(c, pid, "delete-forever")
    assert flow.review(p, c)["requires"] == "--confirm-destructive"
    with pytest.raises(ReviewedWriteError, match="--confirm-destructive"):
        flow.send(p, c, reviewed_hash=p.hash)
    r = flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert r["verified"] and r["fields"] == {"absent": "matched"}
    assert s["writes"][-1] == ("DELETE", "problems/delete-forever", {"ids": str(pid)})
    assert len(s["writes"]) == 3


def test_ignored_delete_is_not_verified_and_never_replayed(api):
    c, s, pid, _ = api
    s["ignore"] = True
    p = flow.problem_plan(c, pid, "delete")
    r = flow.send(p, c, reviewed_hash=p.hash)
    assert not r["verified"] and r["fields"] == {"deleted": "mismatched"}
    with pytest.raises(ReviewedWriteError, match="outcome unknown"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


def test_rejected_and_drifted_targets(api):
    c, s, pid, _ = api
    p = flow.problem_plan(c, pid, "delete")
    s["problem"]["subject"] = "Acme outage (edited)"
    with pytest.raises(ReviewedWriteError, match="changed since review"):
        flow.send(p, c, reviewed_hash=p.hash)
    s["fail"] = 422
    p = flow.problem_plan(c, pid, "delete")
    with pytest.raises(ReviewedWriteError, match="HTTP 422"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


def test_article_trash_and_forever(api):
    c, s, _, aid = api
    p = flow.article_plan(c, aid, "delete")
    assert flow.review(p, c)["title"] == "Globex workaround"
    with pytest.raises(ReviewedWriteError, match="--confirm-destructive"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)["verified"]
    with pytest.raises(ReviewedWriteError, match="already in the trash"):
        flow.article_plan(c, aid, "delete")
    p = flow.article_plan(c, aid, "delete-forever")
    r = flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert r["verified"] and s["writes"][-1][1] == f"solutions/articles/{aid}/delete_forever"
    with pytest.raises(ReviewedWriteError, match="not found"):
        flow.article_plan(c, aid, "delete-forever")


def test_cli_review_then_hash_bound_send(api):
    c, s, pid, aid = api
    runner = CliRunner()
    r = runner.invoke(cli.app, ["problems", "delete", f"PRB-{pid}", "--dry-run"])
    assert r.exit_code == 0, r.output
    h = json.loads(r.output)["plan_hash"]
    assert not s["writes"]
    r = runner.invoke(cli.app, ["problems", "delete", f"PRB-{pid}", "--yes", "--expect-plan", "0" * 64])
    assert r.exit_code == 2 and not s["writes"]
    r = runner.invoke(cli.app, ["problems", "delete", f"PRB-{pid}", "--yes", "--expect-plan", h])
    assert r.exit_code == 0, r.output
    r = runner.invoke(cli.app, ["problems", "restore", str(pid), "--dry-run"])
    assert r.exit_code == 0 and json.loads(r.output)["method"] == "PUT"
    r = runner.invoke(cli.app, ["solutions", "delete-forever", str(aid), "--dry-run"])
    assert r.exit_code == 0 and "PERMANENT" in json.loads(r.output)["effect"]
    h = json.loads(r.output)["plan_hash"]
    r = runner.invoke(cli.app, ["solutions", "delete-forever", str(aid), "--yes", "--expect-plan", h])
    assert r.exit_code == 2 and len(s["writes"]) == 1
