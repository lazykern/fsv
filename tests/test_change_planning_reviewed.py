"""changes update --planning --file on the reviewed contract: plan hash over
section, description and file bytes; uploads then the planning save, each
journaled; readback of attachment IDs/names and description; no replay."""
import base64
import copy
import hashlib
import json
from urllib.parse import parse_qs, urlsplit

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, planning_files, planning_text
from fsv.errors import APIError

UPLOAD_BASE = 701


class FakeClient:
    """Synthetic tenant: one change, one planning field, attachment uploads."""

    def __init__(self, rows=None, limit=4096):
        self.rows = rows if rows is not None else [
            {"name": "acme_plan", "description": "<div>Acme rollout</div>",
             "attachments": [{"id": 501, "name": "old.txt", "size": 9}]}]
        self.limit = limit
        self.uploads = {}
        self.writes = []
        self.fail = None  # (step, exception)
        self.change = {"id": 123, "updated_at": "2026-01-01T00:00:00Z"}

    def int_get(self, path, params=None):
        if path == "change_form_fields":
            return {} if self.limit is None else {"attachment_settings": {"max_attachment_size_bytes": self.limit}}
        if path == "changes/123/planning-fields":
            return {"change_planning_fields": copy.deepcopy(self.rows)}
        if path == "changes/123":
            return {"change": dict(self.change)}
        raise AssertionError(path)

    def _send(self, method, url, **kw):
        assert method == "GET" and url.endswith("/a/dashboard")
        return httpx.Response(200, text="<html><script>var x = 1;</script></html>")

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kw):
        path = url.removeprefix(config.API_INT + "/")
        body = json.loads(kw["content"])
        self.writes.append((method, path, body))
        step = "upload" if path == "attachments" else "planning"
        if self.fail and self.fail[0] == step:
            raise self.fail[1]
        if step == "upload":
            ident = UPLOAD_BASE + len(self.uploads)
            data = base64.b64decode(body["content"].partition(",")[2])
            self.uploads[ident] = {"id": ident, "name": body["content_file_name"], "size": str(len(data))}
            return dict(self.uploads[ident])
        key = (parse_qs(urlsplit(path).query)["id"][0] if method == "POST" else urlsplit(path).path.rsplit("/", 1)[1])
        old = next((r for r in self.rows if r["name"] == key), None)
        known = {a["id"]: a for a in (old or {}).get("attachments", [])} | self.uploads
        row = {"name": key, "description": body.get("description", (old or {}).get("description")),
               "attachments": [copy.deepcopy(known[i]) for i in body.get("attachments", [a["id"] for a in (old or {}).get("attachments", [])])]}
        self.rows = [r for r in self.rows if r["name"] != key] + [row]
        self.change["updated_at"] = "2026-01-01T00:05:00Z"
        return {"planning_field": row}


@pytest.fixture
def env(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(planning_text.schema, "load", lambda *a, **kw: {"fields": [
        {"name": "acme_plan", "label": "Rollout Plan", "field_type": "planning_field", "id": 901},
        {"name": "acme_backout", "label": "Backout Plan", "field_type": "planning_field", "id": 902}]})
    c = FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: c)
    f = tmp_path / "plan.txt"
    f.write_bytes(b"Acme plan v2\n")
    return c, f


def run(*args):
    return CliRunner().invoke(cli.app, ["changes", "update", "CHN-123", "--planning", "Rollout Plan", *args])


def review(*args):
    r = run(*args, "--dry-run")
    assert r.exit_code == 0, r.output
    return json.loads(r.stdout)


def test_dry_run_prints_plan_hash_over_file_bytes_and_sends_nothing(env):
    c, f = env
    out = review("--file", str(f), "--description", "<div>Acme v2</div>")
    assert out["plan_hash"] and out["section"] == "acme_plan"
    assert out["files"] == [{"name": "plan.txt", "size": 13, "content_type": "text/plain",
                             "sha256": hashlib.sha256(b"Acme plan v2\n").hexdigest()}]
    assert [s["step"] for s in out["steps"]] == ["upload", "planning"]
    assert out["attachment_size_limit"]["max_bytes"] == 4096 and out["record_state"]["sends"] == 0
    assert c.writes == []
    f.write_bytes(b"Acme plan v3\n")
    assert review("--file", str(f), "--description", "<div>Acme v2</div>")["plan_hash"] != out["plan_hash"]


def test_yes_alone_refuses_and_sends_nothing(env):
    c, f = env
    r = run("--file", str(f), "--yes")
    assert r.exit_code == 2 and "--expect-plan" in r.output
    assert c.writes == []


def test_reviewed_send_uploads_then_saves_with_journal_and_readback(env):
    c, f = env
    h = review("--file", str(f), "--description", "<div>Acme v2</div>")["plan_hash"]
    r = run("--file", str(f), "--description", "<div>Acme v2</div>", "--yes", "--expect-plan", h)
    assert r.exit_code == 0, r.output
    out = json.loads(r.stdout)
    assert out["verified"] is True and out["description"] == "matched" and out["uploaded_ids"] == [UPLOAD_BASE]
    assert out["attachments"] == [{"id": 501, "name": "old.txt"}, {"id": UPLOAD_BASE, "name": "plan.txt"}]
    assert [w[:2] for w in c.writes] == [("POST", "attachments"), ("PUT", "changes/123/planning-fields/acme_plan")]
    assert c.writes[1][2] == {"description": "<div>Acme v2</div>", "attachments": [501, UPLOAD_BASE]}
    events = [(e.get("kind"), e.get("event")) for e in journal.read() if e.get("plan") == h]
    assert ("plan:change_planning_update_upload", "uploaded") in events
    assert events[-1] == ("plan:change_planning_update", "done")
    again = run("--file", str(f), "--description", "<div>Acme v2</div>", "--yes", "--expect-plan", h)
    assert again.exit_code != 0 and len(c.writes) == 2


def test_duplicate_skip_is_nothing_to_change_and_expect_plan_exits_2(env):
    c, f = env
    old = f.with_name("old.txt")
    old.write_bytes(b"Acme old\n")
    r = run("--file", str(old), "--duplicate", "skip", "--dry-run")
    assert r.exit_code == 0 and "nothing to change" in r.output and "plan_hash" not in r.output
    r = run("--file", str(old), "--duplicate", "skip", "--yes", "--expect-plan", "0" * 64)
    assert r.exit_code == 2 and c.writes == []


def test_duplicate_replace_drops_the_old_id_and_prompt_refuses_without_a_terminal(env):
    c, f = env
    old = f.with_name("old.txt")
    old.write_bytes(b"Acme old v2\n")
    for extra in ((), ("--no-input",)):  # not a terminal: never a silent choice
        r = run("--file", str(old), "--dry-run", *extra)
        assert r.exit_code == 2 and "skip|replace|append" in r.output.replace("\n", "") and c.writes == []
    out = review("--file", str(old), "--duplicate", "replace")
    assert out["replaced_attachments"] == [{"id": 501, "name": "old.txt"}] and out["kept_attachment_ids"] == []
    assert out["description"]["sent"] is False
    r = run("--file", str(old), "--duplicate", "replace", "--yes", "--expect-plan", out["plan_hash"])
    assert r.exit_code == 0, r.output
    assert c.writes[-1][2] == {"attachments": [UPLOAD_BASE]}
    assert run("--file", str(old), "--duplicate", "replace", "--backup-replaced", "--dry-run").exit_code == 2


@pytest.mark.parametrize("step,exc", [("upload", httpx.ReadError("disconnect")),
                                      ("planning", APIError(503, "{}"))])
def test_unknown_step_is_never_replayed(env, step, exc):
    c, f = env
    c.fail = (step, exc)
    h = review("--file", str(f))["plan_hash"]
    r = run("--file", str(f), "--yes", "--expect-plan", h)
    assert r.exit_code == 2 and "do not replay" in r.output and f"{step} step" in r.output
    n = len(c.writes)
    c.fail = None
    assert review("--file", str(f))["plan_hash"] == h
    again = run("--file", str(f), "--yes", "--expect-plan", h)
    assert again.exit_code != 0 and len(c.writes) == n
    assert sum(1 for w in c.writes if w[1] == "attachments") == 1


def test_size_limit_override_and_unpublished_limit(env):
    c, f = env
    r = run("--file", str(f), "--max-bytes", "5", "--dry-run")
    assert r.exit_code == 2 and "5-byte size limit" in r.output
    c.limit = None
    out = review("--file", str(f))
    assert out["attachment_size_limit"]["max_bytes"] is None and "--max-bytes" in out["attachment_size_limit"]["note"]
    r = run("--file", str(f), "--yes", "--expect-plan", out["plan_hash"])
    assert r.exit_code == 2 and "--max-bytes" in r.output and c.writes == []
    out = review("--file", str(f), "--max-bytes", "100")
    assert out["attachment_size_limit"]["source"] == "--max-bytes override"


def test_new_section_posts_description_and_files(env):
    c, f = env
    out = review("--file", str(f))
    assert out["steps"][-1]["method"] == "PUT"
    r = CliRunner().invoke(cli.app, ["changes", "update", "CHN-123", "--planning", "Backout Plan",
                                     "--file", str(f), "--dry-run"])
    out = json.loads(r.stdout)
    assert out["steps"][-1] == {"step": "planning", "method": "POST",
                                "path": "/api/_/changes/123/planning-fields?id=acme_backout"}
    assert out["description"]["after"] == "plan.txt"
    r = CliRunner().invoke(cli.app, ["changes", "update", "CHN-123", "--planning", "Backout Plan",
                                     "--file", str(f), "--yes", "--expect-plan", out["plan_hash"]])
    assert r.exit_code == 0, r.output
    assert c.writes[-1] == ("POST", "changes/123/planning-fields?id=acme_backout",
                            {"description": "plan.txt", "attachments": [UPLOAD_BASE]})
    assert json.loads(r.stdout)["verified"] is True
    assert planning_files.ACTION == "change_planning_update"
