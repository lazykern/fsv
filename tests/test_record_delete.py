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
    with pytest.raises(ReviewedWriteError, match="already sent at .* answered; nothing was sent now"):
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
    assert r.exit_code == 0 and "PERMANENT" in json.loads(r.stdout)["effect"]
    assert f"Articles to delete PERMANENTLY: {aid}" in r.stderr
    h = json.loads(r.stdout)["plan_hash"]
    r = runner.invoke(cli.app, ["solutions", "delete-forever", str(aid), "--yes", "--expect-plan", h])
    assert r.exit_code == 2 and len(s["writes"]) == 1


def test_replay_refusal_names_a_completed_plan_but_keeps_unknown_outcomes_ambiguous():
    from fsv import journal
    assert journal.replay_refusal([], "unknown") is None
    assert journal.replay_refusal([{"event": "failed"}], "unknown") is None
    assert journal.replay_refusal([{"event": "done"}, {"event": "started"}], "sent or outcome unknown") == \
        "sent or outcome unknown"
    text = journal.replay_refusal([{"event": "started"}, {"event": "done", "ts": "2030-01-02T03:04:05+0000"}], "x")
    assert "already sent at 2030-01-02T03:04:05+0000" in text and "nothing was sent now" in text
    assert "outcome unknown" not in text and "no override" in text and "review again" in text


def test_identical_delete_after_restore_is_refused_as_already_sent(api):
    c, s, pid, _ = api
    restored = copy.deepcopy(s["problem"])
    first = flow.problem_plan(c, pid, "delete")
    assert flow.send(first, c, reviewed_hash=first.hash)["verified"]
    s["problem"] = restored  # restored in the web UI: same record state, same plan
    again = flow.problem_plan(c, pid, "delete")
    assert again.hash == first.hash
    writes = len(s["writes"])
    with pytest.raises(ReviewedWriteError, match="already sent at .* no override") as exc:
        flow.send(again, c, reviewed_hash=again.hash)
    assert "outcome unknown" not in str(exc.value)
    assert len(s["writes"]) == writes

@pytest.fixture
def many(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(readback, "_sleep", lambda _s: None)
    s = {"writes": [], "fail_on": None,
         "problem": {i: {"id": i, "subject": f"Acme trashed {i}", "deleted": True} for i in (21, 22, 23)},
         "article": {i: {"id": i, "title": f"Globex article {i}", "deleted": False} for i in (31, 32)}}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        parts = path.split("/")
        if req.method == "GET":
            kind, store = ("problem", s["problem"]) if parts[0] == "problems" else ("article", s["article"])
            row = store.get(int(parts[-1]))
            return httpx.Response(200, json={kind: copy.deepcopy(row)}) if row else httpx.Response(404, json={})
        s["writes"].append((req.method, path, dict(req.url.params)))
        if path == "problems/delete-forever":
            pid = int(req.url.params["ids"])
            if pid == s["fail_on"]:
                return httpx.Response(500, json={"errors": "no"})
            s["problem"].pop(pid)
        else:
            s["article"].pop(int(parts[2]))
        return httpx.Response(204)

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s
    c.close()


@pytest.mark.parametrize("args,msg", [
    (["problems", "delete-forever", "21", "22"], "requires --max-records"),
    (["problems", "delete-forever", "21", "22", "--max-records", "1"], "more IDs than --max-records"),
    (["problems", "delete-forever", "21", "--max-records", "51"], "1 to 50"),
    (["problems", "delete-forever", "21", "21", "--max-records", "2"], "distinct"),
    (["solutions", "delete-forever", "31", "32"], "requires --max-records"),
])
def test_forever_bounds_refuse_before_any_write(many, args, msg):
    c, s = many
    r = CliRunner().invoke(cli.app, args + ["--dry-run"])
    assert r.exit_code != 0 and msg in r.output and not s["writes"]


@pytest.mark.parametrize("module,kind,ids", [("problems", "problem", [21, 22, 23]), ("solutions", "article", [31, 32])])
def test_forever_many_ids_review_print_and_send(many, module, kind, ids):
    c, s = many
    runner = CliRunner()
    args = [module, "delete-forever", *map(str, ids), "--max-records", str(len(ids))]
    r = runner.invoke(cli.app, args + ["--dry-run"])
    assert r.exit_code == 0, r.output
    review = json.loads(r.stdout)
    assert review[f"{kind}_ids"] == ids and len(review["requests"]) == len(ids) and not s["writes"]
    assert "PERMANENTLY: " + ", ".join(map(str, ids)) in r.stderr
    h = review["plan_hash"]
    r = runner.invoke(cli.app, args + ["--yes", "--expect-plan", h])
    assert r.exit_code != 0 and "confirm-destructive" in r.output and not s["writes"]
    r = runner.invoke(cli.app, args + ["--yes", "--expect-plan", h, "--confirm-destructive"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.stdout)["verified"] and len(s["writes"]) == len(ids) and not s[kind]


def test_forever_batch_stops_at_first_failure_and_is_bound_to_max_records(many):
    c, s = many
    a = flow.build_forever(c, "problem", ["21", "22", "23"], max_records=3)
    assert flow.build_forever(c, "problem", ["21", "22", "23"], max_records=4).hash != a.hash
    s["fail_on"] = 22
    r = flow.send_forever(a, c, reviewed_hash=a.hash, confirm_destructive=True)
    assert r["status"] == "partial" and r["stopped_at"] == 22 and r["not_attempted"] == [23]
    assert [w[2]["ids"] for w in s["writes"]] == ["21", "22"] and 23 in s["problem"]



def test_fsv_restore_then_delete_again_is_a_fresh_plan(api):
    c, s, pid, _ = api
    first = flow.problem_plan(c, pid, "delete")
    assert flow.send(first, c, reviewed_hash=first.hash)["verified"]
    restore = flow.problem_plan(c, pid, "restore")
    assert flow.send(restore, c, reviewed_hash=restore.hash)["verified"]
    again = flow.problem_plan(c, pid, "delete")
    assert again.hash != first.hash
    assert flow.send(again, c, reviewed_hash=again.hash)["verified"] and s["problem"]["deleted"] is True


def test_restore_journal_alone_moves_the_delete_hash_and_ambiguous_stays_refused(api):
    c, s, pid, _ = api
    before = flow.problem_plan(c, pid, "delete")
    journal.record_plan("done", "0" * 64, action="problem_restore", target=pid)
    after = flow.problem_plan(c, pid, "delete")
    assert after.hash != before.hash
    assert flow.problem_plan(c, pid, "delete").hash == after.hash  # stable while nothing changes
    journal.record_plan("started", after.hash, action=after.action, target=pid)
    with pytest.raises(ReviewedWriteError, match="outcome unknown"):
        flow.send(flow.problem_plan(c, pid, "delete"), c, reviewed_hash=after.hash)
    assert not s["writes"]
