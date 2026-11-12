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
         "change": {"id": 123, "attachments": []}, "problem": {"id": 123, "attachments": []}, "limit": 4096, "fail": None, "fail_step": "upload", "mismatch": False,
         "missing_id": False, "partial": False, "page": None, "page_status": 200}
    files = [tmp_path / "acme.txt", tmp_path / "globex.txt"]
    for i, file in enumerate(files):
        file.write_bytes(f"Acme evidence {i}\n".encode())
    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "/a/dashboard":
                script = "" if s["page"] is None else "".join(f"window.MAX_ATTACHMENT_LIMIT = {v};\n" for v in s["page"])
                html = f'<html><head><meta name="csrf-token" content="synthetic"></head><script>{script}</script></html>'
                headers = {"location": "/login"} if 300 <= s["page_status"] < 400 else {}
                return httpx.Response(s["page_status"], text=html, headers={"content-type": "text/html", **headers})
            if path in {"ticket_form_fields", "problem_form_fields"}:
                if s["limit"] is None:
                    return httpx.Response(200, json={"ticket_fields": [{"name": "subject"}]})
                return httpx.Response(200, json={"attachment_settings": {"max_attachment_size_bytes": s["limit"]}})
            if path == "tickets/123/conversations":
                rows = copy.deepcopy(s["conversations"])
                if s.get("rotate"):
                    s["reads"] = s.get("reads", 0) + 1
                    for row in rows:
                        for file in row.get("attachments", []):
                            file.update(attachment_url=f"https://files.example.com/acme?Expires={s['reads']}&Signature=synthetic&Key-Pair-Id=key",
                                        updated_at=str(s["reads"]))
                if s["mismatch"] and rows:
                    rows[0]["attachments"] = []
                return httpx.Response(200, json={"conversations": rows, "has_more": s["partial"]})
            if path == "problems/123/notes":
                return httpx.Response(200, json={"notes": copy.deepcopy(s["notes"]), "has_more": s["partial"]})
            if path == "tickets/123":
                return httpx.Response(200, json={"ticket": copy.deepcopy(s["ticket"])})
            if path == "changes/123":
                return httpx.Response(200, json={"change": copy.deepcopy(s["change"])})
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
        if path == "changes":
            s["change"] = {"id": 123, **body, "attachments": attachments}
            return httpx.Response(201, json={"change": {"id": 123}})
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
    if kind in {"create", "change"}:
        def build():
            raw = flow.metadata(c, "tickets")
            base = WritePlan("ticket_create_attachments", 0, config.DOMAIN, "POST", "changes" if kind == "change" else "tickets", json.dumps({"subject": "Acme incident"}), journal.plan_hash(raw))
            return flow.wrap(base, paths, raw, client=c)
        return build
    return lambda: flow.conversation_plan(c, "INC-123", "Acme evidence", paths, kind=kind,
                                         sender="support@example.com" if kind == "reply" else None)


@pytest.mark.parametrize("kind", ["note", "reply", "problem", "create", "change"])
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


@pytest.mark.parametrize("limit", [0, True, "4096", 1])
def test_metadata_limit_fail_closed(api, limit):
    c, s, paths, _ = api
    s["limit"] = limit
    with pytest.raises(ReviewedWriteError):
        builder(c, paths, "note")()
    assert not s["writes"]


def test_conflicting_metadata_byte_limits_block(api):
    c, s, paths, _ = api
    s["limit"] = 4096
    raw = {"attachment_settings": {"max_attachment_size_bytes": 4096}, "fields": [{"max_file_size_bytes": 2048}]}
    with pytest.raises(ReviewedWriteError, match="conflicting"):
        flow.resolve_limit(c, raw)


@pytest.mark.parametrize("kind", ["note", "reply", "problem", "create", "change"])
def test_published_page_limit_is_mib_per_submit(api, kind):
    c, s, paths, _ = api
    s.update(limit=None, page=["40"])
    p = builder(c, paths, kind)()
    review = p.review()
    assert p.limit == review["max_attachment_size_bytes"] == 40 * 1024 * 1024
    assert review["attachment_size_limit"]["source"] == flow.PAGE_SOURCE
    assert review["attachment_size_limit"]["total_bytes"] == sum(f.size for f in p.files)
    assert flow.send(p, c, reviewed_hash=p.hash, rebuild=builder(c, paths, kind))["verified"]


def test_total_over_published_limit_refused_before_upload(api):
    c, s, paths, _ = api
    s.update(limit=None, page=["0.00002"])  # 20.97 bytes -> 20; each file is 16 bytes
    assert builder(c, paths[:1], "note")().limit == 20
    with pytest.raises(ReviewedWriteError, match="exceed"):
        builder(c, paths, "note")()  # each file fits, the total does not
    assert not s["writes"]


def test_page_limit_drift_invalidates_review(api):
    c, s, paths, _ = api
    s.update(limit=None, page=["40"])
    build = builder(c, paths, "note")
    p = build()
    s["page"] = ["20"]
    with pytest.raises(ReviewedWriteError, match="changed"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert not s["writes"]


@pytest.mark.parametrize("page", [["40", "20"], ["0"], ["-1"], ["window.x"], ["1e3"]])
def test_ambiguous_or_invalid_page_limit_blocks(api, page):
    c, s, paths, _ = api
    s.update(limit=None, page=page)
    with pytest.raises(ReviewedWriteError, match="limit"):
        builder(c, paths, "note")()
    assert not s["writes"]


@pytest.mark.parametrize("status", [302, 500])
def test_unreadable_page_blocks(api, status):
    from fsv.session import SessionError
    c, s, paths, _ = api
    s.update(limit=None, page=["40"], page_status=status)
    with pytest.raises(SessionError if status == 302 else ReviewedWriteError):
        builder(c, paths, "note")()
    assert not s["writes"]


def test_unpublished_limit_is_reviewed_and_needs_explicit_override(api):
    c, s, paths, base = api
    s.update(limit=None, page=None)
    build = builder(c, paths, "note")
    p = build()
    review = p.review()
    assert p.limit is None and review["attachment_size_limit"]["source"] == flow.NOT_PUBLISHED
    assert review["attachment_size_limit"]["published_bytes"] == flow.NOT_PUBLISHED
    assert "--max-bytes" in review["attachment_size_limit"]["note"]
    with pytest.raises(ReviewedWriteError, match="--max-bytes"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=build)
    assert not s["writes"] and not journal.read()
    total = sum(f.size for f in p.files)
    with pytest.raises(ReviewedWriteError, match="exceed"):
        flow.conversation_plan(c, "INC-123", "Acme evidence", paths, kind="note", max_bytes=total - 1)
    build = lambda: flow.conversation_plan(c, "INC-123", "Acme evidence", paths, kind="note", max_bytes=total)
    p = build()
    assert p.review()["attachment_size_limit"]["source"] == flow.OVERRIDE_SOURCE
    assert flow.send(p, c, reviewed_hash=p.hash, rebuild=build)["verified"]
    assert s["writes"][-1][2]["attachments"] == [base, base+1]


def test_override_replaces_published_limit_and_is_hash_bound(api):
    c, s, paths, _ = api
    s.update(limit=None, page=["40"])
    p = flow.conversation_plan(c, "INC-123", "Acme evidence", paths, kind="note", max_bytes=4096)
    limit = p.review()["attachment_size_limit"]
    assert (limit["max_bytes"], limit["published_bytes"]) == (4096, 40 * 1024 * 1024)
    assert p.hash != builder(c, paths, "note")().hash
    for bad in (0, -1, True):
        with pytest.raises(ReviewedWriteError, match="--max-bytes"):
            flow.conversation_plan(c, "INC-123", "Acme evidence", paths, kind="note", max_bytes=bad)


@pytest.mark.parametrize("kind", ["note", "reply", "problem"])
def test_cli_max_bytes_option(api, kind):
    _, s, paths, _ = api
    s.update(limit=None, page=None)
    runner = CliRunner()
    args = {"note": ["tickets", "add-note", "INC-123", "--body", "Acme evidence"],
            "reply": ["tickets", "reply", "INC-123", "Acme evidence", "--sender", "support@example.com"],
            "problem": ["problems", "note", "PRB-123", "--body", "Acme evidence"]}[kind] + ["--attach", paths[0]]
    r = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    review = json.loads(r.output)
    assert review["attachment_size_limit"]["source"] == flow.NOT_PUBLISHED
    r = runner.invoke(cli.app, args + ["--yes", "--expect-plan", review["plan_hash"], "--json"])
    assert r.exit_code == 2 and "--max-bytes" in r.output
    assert runner.invoke(cli.app, args + ["--max-bytes", "0", "--dry-run"]).exit_code == 2
    r = runner.invoke(cli.app, args + ["--max-bytes", "1", "--dry-run", "--json"])
    assert r.exit_code == 2 and "exceed" in r.output
    r = runner.invoke(cli.app, args + ["--max-bytes", "4096", "--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["attachment_size_limit"]["max_bytes"] == 4096
    assert not s["writes"]


def test_cli_delete_signed_attachment_uses_same_review_and_send_hash(api):
    _, s, _, base = api
    s["conversations"] = [{"id": 901, "body": "Acme note", "attachments": [{"id": base, "name": "acme.txt", "size": 12}]}]
    s["rotate"] = True
    runner = CliRunner()
    args = ["attachments", "delete", str(base), "--owner", "tickets/123", "--child", "901"]
    first = runner.invoke(cli.app, args + ["--dry-run"])
    second = runner.invoke(cli.app, args + ["--dry-run"])
    assert first.exit_code == second.exit_code == 0
    hash_ = json.loads(first.output)["plan_hash"]
    assert json.loads(second.output)["plan_hash"] == hash_
    assert not s["writes"]
    assert runner.invoke(cli.app, args + ["--yes", "--expect-plan", hash_[:16]]).exit_code == 2
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", hash_])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.output)["verified"]
    assert s["writes"] == [("DELETE", f"attachments/{base}", None)]


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
