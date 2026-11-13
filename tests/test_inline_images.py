"""Offline inline-image uploads embedded in ticket replies and notes."""
import copy
import json
import re

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, inline_images as flow, journal
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError

PNG = b"\x89PNG\r\n\x1a\n" + b"synthetic-image-bytes"


@pytest.fixture(params=[("acme.example.com", 7001), ("globex.example.com", 8001)])
def api(request, monkeypatch, tmp_path):
    domain, base = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"writes": [], "conversations": [], "fail": None, "strip": False, "next": base}
    files = [tmp_path / "acme.png", tmp_path / "globex.gif"]
    for f in files:
        f.write_bytes(PNG)

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "ticket_form_fields":
                return httpx.Response(200, json={"attachment_settings": {"max_attachment_size_bytes": 4096}})
            if path == "tickets/123":
                return httpx.Response(200, json={"ticket": {"id": 123, "attachments": []}})
            if path == "tickets/123/conversations":
                return httpx.Response(200, json={"conversations": copy.deepcopy(s["conversations"]), "has_more": False})
            raise AssertionError(path)
        s["writes"].append((req.method, path, req.headers.get("content-type"), req.content))
        if path == flow.UPLOAD_PATH:
            assert req.headers["content-type"].startswith("multipart/form-data; boundary=")
            assert b'name="file"; filename="' in req.content and PNG in req.content
            if s["fail"] == "upload":
                return httpx.Response(500, json={"error": "failed"})
            s["next"] += 1
            return httpx.Response(201, json={"link": f"https://files.example.com/inline/attachment?token=t{s['next']}&v=1",
                                             "fileid": s["next"]})
        body = json.loads(req.content)
        assert path in {"tickets/123/notes", "tickets/123/reply"}
        if s["fail"] == "owner":
            raise httpx.ReadError("disconnect", request=req)
        stored = re.sub(r' fileid="[0-9]+"', "", body["body"])  # the server drops fileid, keeps data-id
        if s["strip"]:
            stored = "<div>Acme evidence</div>"
        row = {"id": 901, **body, "body": stored, "attachments": []}
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


def build(c, paths, kind, **kw):
    return lambda: flow.plan(c, "INC-123", "Acme evidence", paths, kind=kind,
                             sender="support@example.com" if kind == "reply" else None, **kw)


@pytest.mark.parametrize("kind", ["note", "reply"])
def test_upload_embed_and_verify_link_in_body(api, kind):
    c, s, paths, base = api
    b = build(c, paths, kind)
    p = b()
    review = p.review()
    assert [i["name"] for i in review["inline_images"]] == ["acme.png", "globex.gif"]
    assert review["inline_images"][0]["size"] == len(PNG) and "no delete route" in review["warning"]
    assert not s["writes"]
    r = flow.send(p, c, reviewed_hash=p.hash, rebuild=b)
    assert r["verified"], r
    assert r["inline_image_ids"] == [base + 1, base + 2]
    assert [w[1] for w in s["writes"]] == [flow.UPLOAD_PATH, flow.UPLOAD_PATH, f"tickets/123/{kind}s"
                                           if kind == "note" else "tickets/123/reply"]
    sent = json.loads(s["writes"][-1][3])
    assert "attachments" not in sent
    assert f'data-id="{base + 1}"' in sent["body"] and 'class="fr-fil fr-dib"' in sent["body"]
    assert sent["body"].startswith(json.loads(p.base.payload_json)["body"])
    if kind == "reply":
        assert sent["full_text"] == sent["body"]
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=b)
    assert len(s["writes"]) == 3


def test_readback_without_image_is_not_verified(api):
    c, s, paths, _ = api
    s["strip"] = True
    p = build(c, paths, "note")()
    r = flow.send(p, c, reviewed_hash=p.hash, rebuild=build(c, paths, "note"))
    assert not r["verified"] and set(r["inline_images"].values()) == {"mismatched"}


@pytest.mark.parametrize("step", ["upload", "owner"])
def test_failure_is_single_attempt_and_reports_orphans(api, step):
    c, s, paths, base = api
    s["fail"] = step
    b = build(c, paths, "note")
    p = b()
    with pytest.raises(ReviewedWriteError, match="no delete route exists"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=b)
    count = len(s["writes"])
    assert count == (1 if step == "upload" else 3)
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=b)
    assert len(s["writes"]) == count


def test_non_image_and_oversize_are_refused_before_upload(api, tmp_path):
    c, s, paths, _ = api
    doc = tmp_path / "acme.txt"
    doc.write_bytes(b"Acme")
    with pytest.raises(ReviewedWriteError, match="JPEG, PNG, GIF or WebP"):
        build(c, [str(doc)], "note")()
    with pytest.raises(ReviewedWriteError, match="size limit"):
        build(c, paths, "note", max_bytes=len(PNG))()
    assert not s["writes"]


def test_drifted_bytes_block(api):
    c, s, paths, _ = api
    b = build(c, paths, "note")
    p = b()
    open(paths[0], "ab").write(b"x")
    with pytest.raises(ReviewedWriteError):
        flow.send(p, c, reviewed_hash=p.hash, rebuild=b)
    assert not s["writes"]


def test_cli_note_and_reply_review_then_send(api):
    c, s, paths, _ = api
    runner = CliRunner()
    r = runner.invoke(cli.app, ["tickets", "add-note", "INC-123", "--body", "Acme evidence",
                                "--inline-image", paths[0], "--dry-run"])
    assert r.exit_code == 0, r.output
    plan_hash = json.loads(r.output)["plan_hash"]
    r = runner.invoke(cli.app, ["tickets", "add-note", "INC-123", "--body", "Acme evidence",
                                "--inline-image", paths[0], "--yes", "--expect-plan", plan_hash])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["verified"]
    r = runner.invoke(cli.app, ["tickets", "reply", "INC-123", "Acme evidence", "--inline-image", paths[0],
                                "--attach", paths[1], "--sender", "support@example.com", "--dry-run"])
    assert r.exit_code == 2
    r = runner.invoke(cli.app, ["tickets", "reply", "INC-123", "Acme evidence", "--inline-image", paths[0],
                                "--sender", "support@example.com", "--dry-run"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["inline_images"][0]["name"] == "acme.png"
    assert len(s["writes"]) == 2
