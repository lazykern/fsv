"""Ticket trash: list filter, restore (deleted flag readback), delete forever
(exact 404 readback, explicit IDs, bound and destructive confirmation)."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, ticket_trash as trash
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client


@pytest.fixture
def tenant(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"rows": {31: True, 32: True, 33: False}, "writes": [], "lists": [], "fail": {}, "effect": True}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        q = dict(req.url.params)
        if req.method == "GET":
            if path == "tickets":
                s["lists"].append(q)
                rows = [{"id": t, "subject": f"Acme {t}", "deleted": True} for t, d in s["rows"].items() if d]
                return httpx.Response(200, json={"tickets": rows, "meta": {"offset": 0, "has_next": False,
                                                                           "count": len(rows), "editable_ids": []}})
            tid = int(path.split("/")[1])
            if tid not in s["rows"]:
                return httpx.Response(404, json={})
            return httpx.Response(200, json={"ticket": {"id": tid, "subject": f"Acme {tid}", "deleted": s["rows"][tid]}})
        s["writes"].append((req.method, path, q, req.content))
        if path == "tickets/delete_forever":
            tid = int(q["ids"])
            if tid in s["fail"]:
                return httpx.Response(s["fail"][tid], json={"errors": "synthetic"})
            if s["effect"]:
                del s["rows"][tid]
            return httpx.Response(204)
        tid = int(path.split("/")[1])
        s["rows"][tid] = False
        return httpx.Response(204)

    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: c)
    return c, s


def test_trash_list_uses_deleted_filter(tenant):
    c, s = tenant
    out = trash.trash(c)
    assert [r["id"] for r in out["tickets"]] == [31, 32]
    assert s["lists"][0]["filter"] == "deleted" and s["lists"][0]["order_by"] == "created_at"


def test_restore_bodyless_put_and_deleted_flag_readback(tenant):
    c, s = tenant
    p = trash.build_restore(c, "INC-32")
    assert (p.method, p.path, p.payload) == ("PUT", "tickets/32/restore", None)
    r = trash.send_restore(p, c, reviewed_hash=p.hash)
    assert r["verified"] and r["deleted"] is False
    assert s["writes"][0][3] == b""
    with pytest.raises(ReviewedWriteError, match="not in the trash"):
        trash.build_restore(c, 32)
    with pytest.raises(ReviewedWriteError, match="not found"):
        trash.build_restore(c, 99)


def test_delete_forever_one_request_per_id_with_404_readback(tenant):
    c, s = tenant
    with pytest.raises(ValueError, match="max-records"):
        trash.build_forever(c, ["31", "32"], max_records=None)
    with pytest.raises(ValueError, match="more IDs"):
        trash.build_forever(c, ["31", "32"], max_records=1)
    with pytest.raises(ReviewedWriteError, match="not in the trash"):
        trash.build_forever(c, ["31", "33"], max_records=5)
    p = trash.build_forever(c, ["INC-31", "32"], max_records=2)
    assert p.review()["requests"] == ["/api/_/tickets/delete_forever?ids=31", "/api/_/tickets/delete_forever?ids=32"]
    with pytest.raises(ReviewedWriteError, match="confirm-destructive"):
        trash.send_forever(p, c, reviewed_hash=p.hash)
    assert not s["writes"]
    r = trash.send_forever(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert r["verified"] and set(r["tickets"]) == {"31", "32"}
    assert [w[:2] for w in s["writes"]] == [("DELETE", "tickets/delete_forever")] * 2


def test_delete_forever_stops_at_first_failure_and_unverified(tenant):
    c, s = tenant
    s["fail"][31] = 500
    p = trash.build_forever(c, ["31", "32"], max_records=2)
    r = trash.send_forever(p, c, reviewed_hash=p.hash, confirm_destructive=True)
    assert r["status"] == "failed" and r["not_attempted"] == [32] and len(s["writes"]) == 1
    s["fail"].clear()
    s["effect"] = False
    q = trash.build_forever(c, ["32"], max_records=1)
    r = trash.send_forever(q, c, reviewed_hash=q.hash, confirm_destructive=True)
    assert r["verified"] is False and len(s["writes"]) == 2


def test_cli_lists_ids_and_gates(tenant):
    c, s = tenant
    run = CliRunner().invoke
    args = ["tickets", "delete-forever", "31", "32", "--max-records", "2"]
    review = run(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    assert "Tickets to delete PERMANENTLY: 31, 32" in review.stderr
    digest = json.loads(review.stdout)["plan_hash"]
    refused = run(cli.app, args + ["--yes", "--expect-plan", digest])
    assert refused.exit_code == 2 and not s["writes"]
    done = run(cli.app, args + ["--yes", "--expect-plan", digest, "--confirm-destructive"])
    assert done.exit_code == 0 and json.loads(done.stdout)["verified"]
    listed = run(cli.app, ["tickets", "trash", "list", "--json"])
    assert listed.exit_code == 0 and json.loads(listed.stdout)["tickets"] == []
    r = run(cli.app, ["tickets", "restore", "33", "--dry-run"])
    assert r.exit_code == 2 and "not in the trash" in r.output
