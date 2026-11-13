"""Ticket-side associations through the real HTTP path with native read shapes:
problem link/unlink, change link/unlink in both directions, child tickets."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, ticket_associations as links
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client


@pytest.fixture
def tenant(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"problems": set(), "changes": {"change": set(), "change_cause": set()}, "children": set(),
         "writes": [], "fail": None, "effect": True,
         "tickets": {41: "Incident", 42: "Incident", 43: "Service Request", 44: "Incident"}}

    def level(t):
        if t == 41 and s["children"]:
            return 2
        return 1 if t in s["children"] else 0

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        parts = path.split("/")
        q = dict(req.url.params)
        if req.method == "GET":
            if parts[0] == "tickets" and len(parts) == 2 and int(parts[1]) in s["tickets"]:
                t = int(parts[1])
                return httpx.Response(200, json={"ticket": {"id": t, "type": s["tickets"][t], "subject": f"Acme {t}",
                                                            "human_display_id": f"{'SR' if t == 43 else 'INC'}-{t}"}})
            if path == "problems/7":
                return httpx.Response(200, json={"problem": {"id": 7, "subject": "Acme outage"}})
            if path == "changes/9":
                return httpx.Response(200, json={"change": {"id": 9, "subject": "Globex patch"}})
            if path == "tickets/41/problems" and "filter" not in q:
                return httpx.Response(200, json={"problems": [{"id": p, "human_display_id": f"PRB-{p}"} for p in s["problems"]],
                                                 "meta": {"has_next": False}, "requesters": []})
            if path == "tickets/41/changes":
                if q.get("change_type") not in ("change", "change_cause"):
                    return httpx.Response(400, json={"errors": "not a valid change_type"})
                if q.get("filter") == "list":
                    return httpx.Response(200, json={"changes": [{"id": 9, "subject": "Globex patch"}], "meta": {"has_next": False}})
                return httpx.Response(200, json={"changes": [{"id": c} for c in s["changes"][q["change_type"]]],
                                                 "meta": {"has_next": False}})
            if parts[-1] == "tabs":
                t = int(parts[1])
                return httpx.Response(200, json=[{"name": "related_tickets", "count": 0, "hierarchy": level(t)},
                                                 {"name": "associations", "count": 0}])
            if path == "tickets/41/child-tickets":
                if level(41) != 2:
                    return httpx.Response(400, json={"errors": {"parent_ticket": "not_a_major_ticket"}})
                return httpx.Response(200, json={"child_tickets": [{"id": c, "deleted": False, "spam": False}
                                                                   for c in sorted(s["children"])], "meta": {"has_next": False}})
            if path == "tickets/41/suggest_child_tickets":
                return httpx.Response(200, json={"tickets": [{"id": 42, "subject": "Acme printer"},
                                                             {"id": 44, "subject": "Globex laptop"}]})
            if path == "problems/search":
                return httpx.Response(200, json={"problems": [{"id": 7, "subject": "Acme outage", "status": 1}]})
            if path in {"problems/99", "problems/77"}:
                return httpx.Response(404, json={})
            raise AssertionError(path)
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, path, body, q))
        if s["fail"]:
            return httpx.Response(s["fail"], json={"errors": "synthetic"})
        if s["effect"]:
            if parts[2] == "problems":
                (s["problems"].add if body["type"] == "link" else s["problems"].discard)(int(parts[3]))
            elif parts[2] == "changes":
                if parts[3] == "associate":
                    s["changes"][body["change_type"]].update(body["item_ids"])
                else:
                    s["changes"][body["change_type"]].discard(int(parts[3]))
            elif parts[2] == "associate-child-tickets":
                s["children"].update(body["ticket_ids"])
                return httpx.Response(200, json={"success": True})
            elif parts[2] == "remove_child":
                s["children"].discard(int(q["ticket_id"]))
                return httpx.Response(204)
        return httpx.Response(200, json={})

    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: c)
    return c, s


def test_problem_link_unlink_exact_wires_and_readback(tenant):
    c, s = tenant
    p = links.build_plan(c, "INC-41", "problem", "PRB-7")
    assert (p.method, p.path, p.payload) == ("PUT", "tickets/41/problems/7", {"type": "link"})
    assert links.send(p, c, reviewed_hash=p.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="already"):
        links.build_plan(c, "INC-41", "problem", "PRB-7")
    u = links.build_plan(c, "INC-41", "problem", 7, unlink=True)
    assert u.payload == {"type": "unlink"}
    assert links.send(u, c, reviewed_hash=u.hash)["verified"]
    assert [w[:3] for w in s["writes"]] == [("PUT", "tickets/41/problems/7", {"type": "link"}),
                                            ("PUT", "tickets/41/problems/7", {"type": "unlink"})]


def test_problem_link_refuses_service_request_and_missing_problem(tenant):
    c, s = tenant
    with pytest.raises(ReviewedWriteError, match="incident"):
        links.build_plan(c, "SR-43", "problem", 7)
    with pytest.raises(ReviewedWriteError, match="not found"):
        links.build_plan(c, "INC-41", "problem", 99)
    assert not s["writes"]


@pytest.mark.parametrize("kind,ctype", [("initiating", "change_cause"), ("initiated-by", "change")])
def test_change_both_directions(tenant, kind, ctype):
    c, s = tenant
    p = links.build_plan(c, 41, "change", "CHN-9", ctype=kind)
    assert (p.path, p.payload) == ("tickets/41/changes/associate",
                                   {"item_ids": [9], "change_type": ctype, "type": "link"})
    assert p.review()["change_type"] == ctype
    r = links.send(p, c, reviewed_hash=p.hash)
    assert r["verified"] and r["change_type"] == ctype
    other = "change" if ctype == "change_cause" else "change_cause"
    assert not s["changes"][other]
    u = links.build_plan(c, 41, "change", 9, unlink=True, ctype=kind)
    assert (u.path, u.payload) == ("tickets/41/changes/9", {"type": "unlink", "change_type": ctype})
    assert links.send(u, c, reviewed_hash=u.hash)["verified"]


def test_child_link_unlink_hierarchy_and_404_free_readback(tenant):
    c, s = tenant
    assert links.listing(c, 41, "child")["child_tickets"] == []
    p = links.build_plan(c, 41, "child", "INC-42")
    assert (p.method, p.path, p.payload) == ("POST", "tickets/41/associate-child-tickets", {"ticket_ids": [42]})
    r = links.send(p, c, reviewed_hash=p.hash)
    assert r["verified"] and r["child_hierarchy"] == "child"
    with pytest.raises(ReviewedWriteError, match="child ticket"):
        links.build_plan(c, 42, "child", 44)
    u = links.build_plan(c, 41, "child", 42, unlink=True)
    assert (u.method, u.path, u.payload) == ("DELETE", "tickets/41/remove_child?ticket_id=42", None)
    assert links.send(u, c, reviewed_hash=u.hash)["verified"]
    with pytest.raises(ValueError):
        links.build_plan(c, 41, "child", 41)


def test_no_effect_is_unverified_and_failures_never_resend(tenant):
    c, s = tenant
    s["effect"] = False
    p = links.build_plan(c, 41, "problem", 7)
    assert links.send(p, c, reviewed_hash=p.hash)["verified"] is False
    with pytest.raises(ReviewedWriteError, match="sent"):
        links.send(p, c, reviewed_hash=p.hash)
    s.update(effect=True, fail=500)
    q = links.build_plan(c, 41, "change", 9, ctype="initiating")
    with pytest.raises(ReviewedWriteError, match="unknown"):
        links.send(q, c, reviewed_hash=q.hash)
    assert len(s["writes"]) == 2


def test_cli_candidates_review_gate_and_kind_required(tenant):
    c, s = tenant
    run = CliRunner().invoke
    r = run(cli.app, ["tickets", "children", "candidates", "INC-41", "--search", "printer"])
    assert r.exit_code == 0 and [x["id"] for x in json.loads(r.stdout)["candidates"]] == [42]
    r = run(cli.app, ["tickets", "problems", "candidates", "INC-41", "--search", "acme"])
    assert json.loads(r.stdout)["candidates"][0]["id"] == 7
    r = run(cli.app, ["tickets", "changes", "candidates", "INC-41", "--kind", "initiating"])
    assert json.loads(r.stdout)["candidates"][0]["id"] == 9
    assert run(cli.app, ["tickets", "changes", "link", "INC-41", "CHN-9", "--dry-run"]).exit_code == 2
    args = ["tickets", "changes", "link", "INC-41", "CHN-9", "--kind", "initiated-by"]
    review = run(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.stdout)["plan_hash"]
    assert run(cli.app, args + ["--yes"]).exit_code == 2 and not s["writes"]
    done = run(cli.app, args + ["--yes", "--expect-plan", digest])
    assert done.exit_code == 0 and json.loads(done.stdout)["verified"]
    listed = json.loads(run(cli.app, ["tickets", "changes", "list", "INC-41", "--kind", "initiated-by"]).stdout)
    assert [x["id"] for x in listed["changes"]] == [9]
    missing = run(cli.app, ["tickets", "problems", "link", "INC-41", "PRB-77", "--dry-run"])
    assert missing.exit_code == 2 and len(missing.output.strip().splitlines()) == 1
