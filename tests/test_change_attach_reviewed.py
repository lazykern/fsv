"""changes update ID --attach FILE (attach-only) on the reviewed contract:
plan hash over tenant, change, file names + sizes + sha256, limit and record
state; uploads then one PUT of the full attachment list, each journaled;
readback of the list by id and name; no replay. Synthetic tenant."""
import base64
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import change_attach, cli, config, journal

UPLOAD_BASE = 701


class FakeClient:
    def __init__(self, attachments=None):
        self.change = {"id": 123, "updated_at": "2026-01-01T00:00:00Z",
                       "attachments": attachments if attachments is not None else [{"id": 501, "name": "old.txt", "size": 9}]}
        self.uploads, self.writes, self.fail, self.drop_on_read = {}, [], None, False

    def int_get(self, path, params=None):
        if path == "changes/123":
            row = copy.deepcopy(self.change)
            if self.drop_on_read:
                row["attachments"] = row["attachments"][:-1]
            return {"change": row}
        raise AssertionError(path)

    def _send(self, method, url, **kw):
        assert method == "GET" and url.endswith("/a/dashboard")
        return httpx.Response(200, text="<html><script>var MAX_ATTACHMENT_LIMIT = 1;</script></html>")

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kw):
        path = url.removeprefix(config.API_INT + "/")
        body = json.loads(kw["content"])
        self.writes.append((method, path, body))
        step = "upload" if path == "attachments" else "attach"
        if self.fail and self.fail[0] == step:
            raise self.fail[1]
        if step == "upload":
            ident = UPLOAD_BASE + len(self.uploads)
            data = base64.b64decode(body["content"].partition(",")[2])
            self.uploads[ident] = {"id": ident, "name": body["content_file_name"], "size": len(data)}
            return dict(self.uploads[ident])
        assert (method, path) == ("PUT", "changes/123")
        known = {a["id"]: a for a in self.change["attachments"]} | self.uploads
        self.change["attachments"] = [copy.deepcopy(known[i]) for i in body["attachments"]]
        self.change["updated_at"] = "2026-01-01T00:05:00Z"
        return {"change": {"id": 123}}


@pytest.fixture
def env(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    c = FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: c)
    f = tmp_path / "plan.txt"
    f.write_bytes(b"Acme plan v2\n")
    return c, f


def run(*args):
    return CliRunner().invoke(cli.app, ["changes", "update", "CHN-123", "--no-input", *args])


def review(*args):
    r = run(*args, "--dry-run")
    assert r.exit_code == 0, r.output
    return json.loads(r.stdout)


def test_dry_run_prints_plan_hash_and_sends_nothing(env):
    c, f = env
    body = review("--attach", str(f))
    assert body["status"] == "dry_run" and len(body["plan_hash"]) == 64
    assert body["files"][0]["name"] == "plan.txt" and body["files"][0]["sha256"]
    assert body["kept_attachment_ids"] == [501] and body["attachment_size_limit"]["max_bytes"] == 1024 * 1024
    assert [s["step"] for s in body["steps"]] == ["upload", "attach"]
    assert not c.writes
    # the hash binds the limit and the file bytes
    assert review("--attach", str(f), "--max-bytes", "1000")["plan_hash"] != body["plan_hash"]
    f.write_bytes(b"Acme plan v3\n")
    assert review("--attach", str(f))["plan_hash"] != body["plan_hash"]


def test_yes_needs_full_expect_plan(env):
    c, f = env
    digest = review("--attach", str(f))["plan_hash"]
    for extra in ([], ["--expect-plan", digest[:12]]):
        r = run("--attach", str(f), "--yes", *extra)
        assert r.exit_code == 2, r.output
    assert not c.writes


def test_send_uploads_then_puts_full_list_journals_and_verifies(env):
    c, f = env
    digest = review("--attach", str(f))["plan_hash"]
    r = run("--attach", str(f), "--yes", "--expect-plan", digest)
    assert r.exit_code == 0, r.output
    out = json.loads(r.stdout)
    assert out["verified"] is True and out["uploaded_ids"] == [701]
    assert out["attachments"] == [{"id": 501, "name": "old.txt"}, {"id": 701, "name": "plan.txt"}]
    assert [(m, p) for m, p, _ in c.writes] == [("POST", "attachments"), ("PUT", "changes/123")]
    assert c.writes[1][2] == {"attachments": [501, 701]}
    kinds = [e.get("kind") for e in journal.read()]
    assert "plan:" + change_attach.ACTION + "_upload" in kinds and kinds.count("plan:" + change_attach.ACTION) >= 2
    replay = run("--attach", str(f), "--yes", "--expect-plan", digest)
    assert replay.exit_code == 2 and len(c.writes) == 2


def test_unverified_readback_exits_4(env, monkeypatch):
    c, f = env
    digest = review("--attach", str(f))["plan_hash"]
    original = change_attach.build_plan

    def build(*a, **kw):  # review and rebuild see the honest list; the readback does not
        c.drop_on_read = False
        try:
            return original(*a, **kw)
        finally:
            c.drop_on_read = True
    monkeypatch.setattr(change_attach, "build_plan", build)
    r = run("--attach", str(f), "--yes", "--expect-plan", digest)
    assert r.exit_code == 4 and json.loads(r.stdout)["verified"] is False


def test_failed_put_is_never_replayed(env):
    c, f = env
    digest = review("--attach", str(f))["plan_hash"]
    c.fail = ("attach", RuntimeError("synthetic disconnect"))
    r = run("--attach", str(f), "--yes", "--expect-plan", digest)
    assert r.exit_code == 2 and "uploaded IDs: [701]" in " ".join(r.output.split())
    c.fail = None
    again = run("--attach", str(f), "--yes", "--expect-plan", digest)
    assert again.exit_code == 2 and len(c.writes) == 2


@pytest.mark.parametrize("mode, kept, uploads", [("skip", [501], 0), ("replace", [], 1), ("append", [501], 1)])
def test_duplicate_rules_unchanged(env, tmp_path, mode, kept, uploads):
    c, _ = env
    same = tmp_path / "old.txt"
    same.write_bytes(b"Acme old v2\n")
    if not uploads:
        r = run("--attach", str(same), "--duplicate", mode)
        assert r.exit_code == 0 and "already attached" in r.output and not c.writes
        assert run("--attach", str(same), "--duplicate", mode, "--expect-plan", "0" * 64).exit_code == 2
        return
    body = review("--attach", str(same), "--duplicate", mode)
    assert body["kept_attachment_ids"] == kept and len(body["files"]) == uploads
    assert (body["replaced_attachments"] == [{"id": 501, "name": "old.txt"}]) is (mode == "replace")


def test_no_input_prompt_refuses_only_on_a_real_duplicate(env, tmp_path):
    c, f = env
    assert review("--attach", str(f))["files"]  # no same-name attachment: no --duplicate needed
    same = tmp_path / "old.txt"
    same.write_bytes(b"x")
    r = run("--attach", str(same), "--dry-run")
    assert r.exit_code == 2 and "--duplicate skip|replace|append" in " ".join(r.output.split())
    assert not c.writes


def test_backup_replaced_is_refused_on_the_reviewed_path(env):
    c, f = env
    r = run("--attach", str(f), "--duplicate", "replace", "--backup-replaced", "--dry-run")
    assert r.exit_code == 2 and "--backup-replaced is not on the reviewed" in " ".join(r.output.split())
    assert not c.writes
