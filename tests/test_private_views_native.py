"""Private views from native read shapes: numeric Myself code, no top-level
workspace on the exact read, read/write workspace-condition forms, change views."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, private_views as views
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError

USER, WS = 905, 7


def _ticket_read(vid, name="Acme queue"):
    # Native exact read: number only, group_ids list, workspace only in meta
    # and in the query (string form), no top-level workspace_id.
    return {"id": vid, "name": name, "default": False, "visibility": 3, "user_id": USER, "group_id": None,
            "group_ids": [], "advanced_query_hash": None, "advanced": False, "order_by": "created_at",
            "order_type": "desc", "per_page": 30,
            "query_hash": [{"condition": "priority", "operator": "is_in", "value": ["3"], "type": "default"},
                           {"condition": "workspace_id", "operator": "is", "value": str(WS), "type": "default"}],
            "meta": {"workspace_id": [{"id": WS, "name": "Acme IT"}]}}


def _change_read(vid, name="Globex changes"):
    return {"id": vid, "name": name, "default": False, "visibility": 3, "user_id": USER, "group_ids": [],
            "query_hash": None, "order_by": "created_at", "order_type": "desc",
            "advanced_query_hash": {"all": [
                {"condition": "planned_start_date", "operator": "in_the_last", "value": "one_week", "type": "default"},
                {"condition": "workspace_id", "operator": "is", "value": WS, "type": "default"}]}}


@pytest.fixture
def wire(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    state = {"ticket_filters": {801: _ticket_read(801)}, "change-filters": {811: _change_read(811)},
             "writes": [], "next": 900}

    def handle(req):
        parts = req.url.path.split("/api/_/", 1)[1].split("/")
        store = state[parts[0]]
        if req.method == "GET":
            if len(parts) == 1:
                key = parts[0].replace("-", "_")
                return httpx.Response(200, json={key: [{"id": k, "name": v["name"], "user_id": v["user_id"],
                                                        "default": v["default"], "advanced": False}
                                                       for k, v in store.items()] +
                                                 [{"id": "my_open", "name": "System", "default": True}],
                                                 "meta": {"editable_filter_ids": list(store)}})
            row = store.get(int(parts[1]))
            if row is None:
                return httpx.Response(404, json={})
            env = "ticket_filter" if parts[0] == "ticket_filters" else "change_filter"
            return httpx.Response(200, json={env: copy.deepcopy(row)})
        body = json.loads(req.content) if req.content else None
        state["writes"].append((req.method, req.url.path, body))
        if req.method == "DELETE":
            del store[int(parts[1])]
            return httpx.Response(204)
        if parts[0] == "ticket_filters":
            vid = state["next"] if req.method == "POST" else int(parts[1])
            state["next"] += 1
            stored = _ticket_read(vid, body["name"])
            # The server stores the write form and reads it back in its own form.
            stored["query_hash"] = [c if c["condition"] != "workspace_id" else
                                    {**c, "value": str(c["value"][0]["id"])} for c in body["query_hash"]]
            store[vid] = stored
            return httpx.Response(201 if req.method == "POST" else 200, json={"ticket_filter": {"id": vid}})
        if req.method == "PUT":  # captured: the whole filter is re-sent; the id is kept
            vid = int(parts[1])
        else:
            vid = state["next"]
            state["next"] += 1
        store[vid] = {**_change_read(vid, body["name"]), "advanced_query_hash": body["advanced_query_hash"]}
        return httpx.Response(200, json={"change_filter": {"id": vid}})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    monkeypatch.setattr(c, "me", lambda: {"id": USER})
    monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
    state["client"] = c
    yield state
    c.close()


def test_numeric_myself_view_clone_rename_delete_with_meta_workspace(wire):
    c = wire["client"]
    p = views.build_plan(c, 801, action="create", name="Acme clone")
    assert p.payload["workspace_id"] == WS and p.payload["visibility"] == 3
    ws_cond = [q for q in p.payload["query_hash"] if q["condition"] == "workspace_id"]
    assert ws_cond == [{"condition": "workspace_id", "operator": "is", "value": [{"id": WS}], "type": "default"}]
    r = views.send(p, c, reviewed_hash=p.hash)
    assert r["verified"], r
    new = r["view_id"]
    q = views.build_plan(c, new, action="rename", name="Acme renamed")
    assert q.method == "PUT" and q.action == "private_view_edit"
    assert views.send(q, c, reviewed_hash=q.hash)["verified"]
    d = views.build_plan(c, new, action="delete")
    assert views.send(d, c, reviewed_hash=d.hash)["verified"]
    assert [w[0] for w in wire["writes"]] == ["POST", "PUT", "DELETE"]


@pytest.mark.parametrize("change", [{"visibility": 1}, {"visibility": 2}, {"group_ids": [44]},
                                      {"group_ids": None}, {"user_id": 906}, {"default": True}])
def test_shared_or_foreign_views_refused(wire, change):
    wire["ticket_filters"][801].update(change)
    if change.get("group_ids") is None and "group_ids" in change:
        del wire["ticket_filters"][801]["group_ids"]
    with pytest.raises(ReviewedWriteError):
        views.build_plan(wire["client"], 801, action="create", name="Acme")
    assert not wire["writes"]


def test_conflicting_or_missing_workspace_refused(wire):
    row = wire["ticket_filters"][801]
    row["meta"]["workspace_id"] = [{"id": WS + 1}]
    with pytest.raises(ReviewedWriteError, match="workspace"):
        views.build_plan(wire["client"], 801, action="create", name="Acme")
    del row["meta"]
    row["query_hash"] = row["query_hash"][:1]
    with pytest.raises(ReviewedWriteError, match="workspace"):
        views.build_plan(wire["client"], 801, action="create", name="Acme")
    assert not wire["writes"]


def test_change_view_create_rename_delete_verified(wire):
    c = wire["client"]
    p = views.build_plan(c, 811, action="create", name="Globex clone", kind="changes")
    assert (p.method, p.path) == ("POST", "change-filters")
    assert p.payload["query_hash"] is None and p.payload["workspace_id"] == WS
    assert p.payload["advanced_query_hash"]["all"][-1]["value"] == WS
    r = views.send(p, c, reviewed_hash=p.hash)
    assert r["verified"], r
    new = r["view_id"]
    q = views.build_plan(c, new, action="rename", name="Globex renamed", kind="changes")
    assert (q.method, q.path, q.action) == ("PUT", f"change-filters/{new}", "change_private_view_edit")
    # the whole filter is re-sent unchanged except the name
    assert q.payload == {**p.payload, "name": "Globex renamed"}
    renamed = views.send(q, c, reviewed_hash=q.hash)
    assert renamed["verified"] and renamed["fields"]["name"] == "matched"
    assert wire["change-filters"][new]["name"] == "Globex renamed"
    d = views.build_plan(c, new, action="delete", kind="changes")
    assert (d.method, d.path, d.payload) == ("DELETE", f"change-filters/{new}", None)
    gone = views.send(d, c, reviewed_hash=d.hash)
    assert gone["verified"] and new not in wire["change-filters"]
    assert [w[0] for w in wire["writes"]] == ["POST", "PUT", "DELETE"]


def test_change_view_rename_to_same_name_and_shared_views_refused(wire):
    c = wire["client"]
    with pytest.raises(ValueError, match="nothing to change"):
        views.build_plan(c, 811, action="rename", name="Globex changes", kind="changes")
    for change in ({"visibility": 1}, {"group_ids": [44]}, {"user_id": 906}, {"default": True}):
        wire["change-filters"][811] = {**_change_read(811), **change}
        for action, name in (("rename", "x"), ("delete", None)):
            with pytest.raises(ReviewedWriteError):
                views.build_plan(c, 811, action=action, name=name, kind="changes")
    assert not wire["writes"]


def test_change_view_delete_unverified_when_still_readable(wire, monkeypatch):
    c = wire["client"]
    d = views.build_plan(c, 811, action="delete", kind="changes")
    real = c._client._transport.handler

    def keep(req):
        resp = real(req)
        if req.method == "DELETE":
            wire["change-filters"][811] = _change_read(811)  # server kept it
        return resp

    c._client = httpx.Client(transport=httpx.MockTransport(keep))
    assert views.send(d, c, reviewed_hash=d.hash)["verified"] is False


def test_change_cli_list_show_and_full_hash_gate(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire["client"])
    runner = CliRunner()
    listed = runner.invoke(cli.app, ["changes", "private-views", "list"])
    assert listed.exit_code == 0, listed.output
    assert [v["id"] for v in json.loads(listed.output)["views"]] == [811]
    shown = json.loads(runner.invoke(cli.app, ["changes", "private-views", "show", "811"]).output)
    assert shown["private"] is True and shown["visibility"] == "Myself" and shown["workspace_id"] == WS
    args = ["changes", "private-views", "write", "811", "create", "--name", "Globex mine"]
    review = runner.invoke(cli.app, args + ["--dry-run"])
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2 and not wire["writes"]
    done = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert done.exit_code == 0 and json.loads(done.output)["verified"]
    rename = ["changes", "private-views", "write", "811", "rename", "--name", "Globex renamed"]
    digest = json.loads(runner.invoke(cli.app, rename + ["--dry-run"]).output)["plan_hash"]
    done = runner.invoke(cli.app, rename + ["--yes", "--expect-plan", digest])
    assert done.exit_code == 0 and json.loads(done.output)["verified"]
    delete = ["changes", "private-views", "write", "811", "delete"]
    review = runner.invoke(cli.app, delete + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    writes = len(wire["writes"])
    no_confirm = runner.invoke(cli.app, delete + ["--yes", "--expect-plan", digest])
    assert no_confirm.exit_code == 2 and "--confirm-destructive" in no_confirm.output
    assert len(wire["writes"]) == writes
    gone = runner.invoke(cli.app, delete + ["--yes", "--expect-plan", digest, "--confirm-destructive"])
    assert gone.exit_code == 0 and json.loads(gone.output)["verified"]
    replay = runner.invoke(cli.app, delete + ["--yes", "--expect-plan", digest, "--confirm-destructive"])
    assert replay.exit_code == 2 and len(wire["writes"]) == writes + 1
