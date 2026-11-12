"""Synthetic accepted bulk envelopes and frozen selection failure modes."""
import copy
import json
from types import SimpleNamespace

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, ticket_bulk as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError, WritePlan


@pytest.fixture
def api(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"rows": {i: {"id": i, "status": 2, "spam": False, "deleted": False} for i in [71, 72]},
         "writes": [], "failure": False, "omit": False, "partial": False}
    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "tickets":
                return httpx.Response(200, json={"tickets": list(s["rows"].values()), "has_more": s["partial"]})
            row = copy.deepcopy(s["rows"][int(path.rsplit("/", 1)[1])])
            if s["omit"] and s["writes"]:
                row.pop("spam", None)
            return httpx.Response(200, json={"ticket": row})
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, path, body, dict(req.url.params)))
        if s["failure"]:
            raise httpx.ReadError("disconnect", request=req)
        ids = body["ids"] if body else [int(i) for i in req.url.params["ids"].split(",")]
        if "update_multiple" in path:
            fields = {k: v for k, v in body.items() if k not in {"ids", "reply"}}
        elif path.endswith("delete_multiple"):
            fields = {"deleted": True}
        else:
            fields = {"spam": path.endswith("/spam_multiple")}
        for i in ids:
            s["rows"][i].update(fields)
        return httpx.Response(204) if req.method == "DELETE" else httpx.Response(200, json={"success": True})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "build_query_hash", lambda *a: ('[{"synthetic":true}]', [], []))
    def prop(client, i, answers):
        payload = {"status": 3}
        return SimpleNamespace(payload=payload, hash=journal.snapshot_hash({"row": s["rows"][i], "answers": answers}))
    monkeypatch.setattr(flow.ticket_properties, "build_plan", prop)
    def assign(client, ids, **kw):
        write = WritePlan("ticket_assign", ids[0], config.DOMAIN, "PUT", f"tickets/{ids[0]}",
                          json.dumps({"group_id": 81, "responder_id": 91}), journal.snapshot_hash(s["rows"][ids[0]]))
        return SimpleNamespace(write=write, hash=write.hash)
    monkeypatch.setattr(flow.ticket_assignment, "build_plan", assign)
    yield c, s
    c.close()


@pytest.mark.parametrize("action,options,method,path", [
    ("status", {"status": "Pending"}, "PUT", "bulk_ticket_actions/update_multiple"),
    ("assign", {"agent": "91", "group": "81"}, "PUT", "bulk_ticket_actions/update_multiple"),
    ("spam", {}, "POST", "tickets/spam_multiple"),
    ("unspam", {}, "POST", "tickets/unspam_multiple"),
    ("delete", {}, "DELETE", "tickets/delete_multiple"),
])
@pytest.mark.parametrize("filtered", [False, True])
def test_exact_bulk_wire_and_per_ticket_readback(api, action, options, method, path, filtered):
    c, s = api
    selection = {"where": ["Priority=Low"], "select_all": True} if filtered else {}
    p = flow.build_plan(c, [] if filtered else ["INC-72", "INC-71"], action=action, **options, **selection)
    assert p.review()["ticket_ids"] == [71, 72] and not s["writes"]
    result = flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert result["verified"] and len(s["writes"]) == 1
    m, actual_path, body, query = s["writes"][0]
    assert (m, actual_path) == (method, path)
    if action == "delete":
        assert body is None and query == {"ids": "71,72"}
    else:
        assert body["ids"] == [71, 72] and all(type(i) is int for i in body["ids"])
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert len(s["writes"]) == 1


@pytest.mark.parametrize("action", ["spam", "delete"])
def test_destructive_confirmation_and_printed_ids(api, action):
    c, s = api
    args = ["tickets", "bulk", action, "INC-71", "INC-72", "--json"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    assert "Selected ticket IDs: 71, 72" in review.stderr
    h = json.loads(review.stdout)["plan_hash"]
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", h])
    assert sent.exit_code == 2 and "--confirm-destructive" in sent.output
    assert not s["writes"]


def test_filter_drift_prevents_send(api):
    c, s = api
    p = flow.build_plan(c, [], action="spam", where=["Priority=Low"], select_all=True)
    del s["rows"][72]
    with pytest.raises(ReviewedWriteError, match="selection"):
        flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert not s["writes"]


@pytest.mark.parametrize("failure", ["disconnect", "missing-flag"])
def test_unknown_or_unverified_never_replays(api, failure):
    c, s = api
    p = flow.build_plan(c, ["71", "72"], action="spam")
    if failure == "disconnect":
        s["failure"] = True
        with pytest.raises(ReviewedWriteError, match="unknown"):
            flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    else:
        s["omit"] = True
        assert not flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)["verified"]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert len(s["writes"]) == 1


@pytest.mark.parametrize("ids,kw", [([], {}), (["71", "71"], {}), ([], {"select_all": True}),
                                    (["71"], {"where": ["Priority=Low"], "select_all": True}),
                                    ([], {"where": ["Priority=Low"]})])
def test_invalid_selection_never_writes(api, ids, kw):
    c, s = api
    with pytest.raises(ValueError):
        flow.build_plan(c, ids, action="spam", **kw)
    assert not s["writes"]
