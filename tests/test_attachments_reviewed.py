"""Offline two-step native attachments, owner readback, failure and drift guards."""
import base64
import copy
import hashlib
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import attachments as flow, cli, config, journal
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError, WritePlan


@pytest.fixture(params=[("acme.example.com", 701), ("globex.example.com", 801)])
def api(request, monkeypatch, tmp_path):
    domain, base = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"writes": [], "uploads": {}, "notes": [], "conversations": [], "ticket": {"id": 123, "attachments": []},
         "problem": {"id": 123, "attachments": []}, "limit": 4096, "fail": None, "fail_step": "upload", "mismatch": False,
         "missing_id": False, "partial": False}
    files = [tmp_path / "acme.txt", tmp_path / "globex.txt"]
    for i, file in enumerate(files):
        file.write_bytes(f"Acme evidence {i}\n".encode())
    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path in {"ticket_form_fields", "problem_form_fields"}:
                return httpx.Response(200, json={"attachment_settings": {"max_attachment_size_bytes": s["limit"]}})
            if path == "tickets/123/conversations":
                rows = copy.deepcopy(s["conversations"])
                if s["mismatch"] and rows:
                    rows[0]["attachments"] = []
                return httpx.Response(200, json={"conversations": rows, "has_more": s["partial"]})
            if path == "problems/123/notes":
                return httpx.Response(200, json={"notes": copy.deepcopy(s["notes"]), "has_more": s["partial"]})
            if path == "tickets/123":
                return httpx.Response(200, json={"ticket": copy.deepcopy(s["ticket"])})
            if path == "problems/123":
                return httpx.Response(200, json={"problem": copy.deepcopy(s["problem"])})
            raise AssertionError(path)
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, path, body))
        step = "upload" if path == "attachments" else "owner"
        if s["fail"] and step == s["fail_step"]:
            if s["fail"] == "disconnect":
                raise httpx.ReadError("disconnect", request=req)
            return httpx.Response(s["fail"], json={"error": "failed"})
        if req.method == "DELETE":
            ident = int(path.rsplit("/", 1)[1])
            for row in [s["ticket"], s["problem"], *s["notes"], *s["conversations"]]:
                row["attachments"] = [a for a in row["attachments"] if a["id"] != ident]
            return httpx.Response(204)
        if path == "attachments":
            ident = base+len(s["uploads"])
            data = base64.b64decode(body["content"].partition(",")[2])
            assert len(data) == body["content_file_size"]
            s["uploads"][ident] = {"id": ident, "name": body["content_file_name"], "size": str(len(data)), "content_type": body["content_content_type"]+";charset=utf-8"}
            return httpx.Response(200, json={} if s["missing_id"] else s["uploads"][ident])
        attachments = [copy.deepcopy(s["uploads"][i]) for i in body["attachments"]]
        if path == "tickets":
            s["ticket"] = {"id": 123, **body, "attachments": attachments}
            return httpx.Response(201, json={"ticket": {"id": 123}})
        row = {"id": 901, **body, "attachments": attachments, "body": "<div>Acme evidence</div>"}
        if path == "problems/123/notes":
            s["notes"].append(row)
            return httpx.Response(201, json={"note": {"id": 901}})
        assert path in {"tickets/123/notes", "tickets/123/reply"}
        if "notify_emails" in row:
            row["to_emails"] = row.pop("notify_emails")
        s["conversations"].append(row)
        return httpx.Response(201, json={"conversation": {"id": 901}})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr("fsv.ticket_notes.get_client", lambda: c)
    yield c, s, [str(f) for f in files], base
    c.close()


def builder(c, paths, kind):
    if kind == "problem":
        return lambda: flow.problem_note_plan(c, "PRB-123", "Acme evidence", paths)
    if kind == "create":
        def build():
            raw = flow.metadata(c, "tickets")
            base = WritePlan("ticket_create_attachments", 0, config.DOMAIN, "POST", "tickets", json.dumps({"subject": "Acme incident"}), journal.plan_hash(raw))
            return flow.wrap(base, paths, raw)
        return build
    return lambda: flow.conversation_plan(c, "INC-123", "Acme evidence", paths, kind=kind,
                                         sender="support@example.com" if kind == "reply" else None)


@pytest.mark.parametrize("kind", ["note", "reply", "problem", "create"])
def test_native_two_step_and_normalized_readback(api, kind):
    c, s, paths, base = api
    build = builder(c, paths, kind)
    p = build()
    assert p.review()["files"][0]["sha256"] == hashlib.sha256(open(paths[0], "rb").read()).hexdigest()
    assert not s["writes"]
    r = flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert r["verified"] and r["attachments"] == [base, base+1]
    assert len(s["writes"]) == 3
    assert s["writes"][0][:2] == ("POST", "attachments")
    assert s["writes"][0][2]["content"].startswith("data:text/plain;base64,")
    assert s["writes"][-1][2]["attachments"] == [base, base+1]
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert len(s["writes"]) == 3


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
@pytest.mark.parametrize("step", ["upload", "owner"])
def test_ambiguous_never_replayed(api, failure, step):
    c, s, paths, _ = api
    build = builder(c, paths[:1], "note")
    p = build()
    s.update(fail=failure, fail_step=step)
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert len(s["writes"]) == (1 if step == "upload" else 2)
    assert [r["event"] for r in journal.read() if r.get("kind") == "plan:"+p.base.action] == ["started"]


@pytest.mark.parametrize("drift", ["file", "limit", "owner"])
def test_preupload_drift_blocks(api, drift):
    c, s, paths, _ = api
    build = builder(c, paths, "note")
    p = build()
    if drift == "file":
        from pathlib import Path
        Path(paths[0]).write_text("changed")
    elif drift == "limit":
        s["limit"] += 1
    else:
        s["ticket"]["subject"] = "changed"
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert not s["writes"]


@pytest.mark.parametrize("limit", [None, 0, True, "4096", 1])
def test_metadata_limit_fail_closed(api, limit):
    c, s, paths, _ = api
    s["limit"] = limit
    with pytest.raises(ReviewedWriteError):
        builder(c, paths, "note")()
    assert not s["writes"]


def test_unknown_upload_id_stops_owner(api):
    c, s, paths, _ = api
    build = builder(c, paths, "note")
    p = build()
    s["missing_id"] = True
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert len(s["writes"]) == 1


@pytest.mark.parametrize("mode", ["mismatch", "partial"])
def test_readback_not_echo(api, mode):
    c, s, paths, _ = api
    build = builder(c, paths, "note")
    p = build()
    s[mode] = True
    assert not flow.send(p, c, reviewed_hash=p.hash, rebuild=build)["verified"]
    assert len(s["writes"]) == 3


@pytest.mark.parametrize("kind", ["note", "reply", "problem"])
def test_cli_review_full_hash_send_and_delete(api, kind):
    _, s, paths, base = api
    runner = CliRunner()
    if kind == "note":
        args = ["tickets", "add-note", "INC-123", "--body", "Acme evidence"]
    elif kind == "reply":
        args = ["tickets", "reply", "INC-123", "Acme evidence", "--sender", "support@example.com"]
    else:
        args = ["problems", "note", "PRB-123", "--body", "Acme evidence"]
    args += ["--attach", paths[0], "--attach", paths[1]]
    r = runner.invoke(cli.app, args+["--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    hash_ = json.loads(r.output)["plan_hash"]
    assert not s["writes"]
    assert runner.invoke(cli.app, args+["--yes", "--expect-plan", hash_[:16]]).exit_code == 2
    r = runner.invoke(cli.app, args+["--yes", "--expect-plan", hash_, "--json"])
    assert r.exit_code == 0, r.output
    owner = "problems/123" if kind == "problem" else "tickets/123"
    args = ["attachments", "delete", str(base), "--owner", owner, "--child", "901"]
    r = runner.invoke(cli.app, args+["--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    hash_ = json.loads(r.output)["plan_hash"]
    r = runner.invoke(cli.app, args+["--yes", "--expect-plan", hash_, "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["verified"]
    assert s["writes"][-1] == ("DELETE", "attachments/"+str(base), None)


def test_cli_incident_create_real_owner_transport(api, monkeypatch):
    from test_ticket_create import meta
    from fsv import change_flow
    _, s, paths, _ = api
    monkeypatch.setattr(change_flow, "interactive", lambda: False)
    m = meta()
    monkeypatch.setattr("fsv.ticket_cli.load_meta", lambda *a, **kw: m)
    from test_ticket_create import WHO
    monkeypatch.setattr("fsv.change_cli.resolve_requester", lambda *a, **kw: WHO)
    args = ["tickets", "create", "--requester", "jane.doe@example.com", "--set", "Subject=Acme", "--set", "Description=Help",
            "--set", "Cloud Only=Yes", "--set", "Stage=Live", "--attach", paths[0]]
    runner = CliRunner()
    r = runner.invoke(cli.app, args+["--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    hash_ = json.loads(r.output)["plan_hash"]
    assert not s["writes"]
    r2 = runner.invoke(cli.app, args+["--dry-run", "--json"])
    assert json.loads(r2.output) == json.loads(r.output), (r.output, r2.output)
    r = runner.invoke(cli.app, args+["--yes", "--expect-plan", hash_, "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["verified"]
    assert len(s["writes"]) == 2 and s["writes"][-1][1] == "tickets"
