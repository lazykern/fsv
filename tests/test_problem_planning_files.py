"""Problem analysis-section files on the captured change wire: uploads,
then POST .../planning-fields?id=<section> (first write) or PUT
.../planning-fields/<section>; download of section files; attachment
delete keeps the section text. Synthetic tenant only."""
import base64
import copy
import json
from urllib.parse import parse_qs, urlsplit

import pytest
from typer.testing import CliRunner

from fsv import cli, config, create, journal

PID = 47
UPLOAD_BASE = 801


class FakeProblem:
    def __init__(self, rows=None):
        self.rows = rows if rows is not None else [
            {"id": 3, "name": "problem_impact", "description": "<div>Acme impact</div>",
             "attachments": [{"id": 611, "name": "impact.txt", "size": 5}]}]
        self.uploads, self.writes, self.deleted = {}, [], []
        self.problem = {"id": PID, "updated_at": "2026-01-01T00:00:00Z"}
        self.keep_text = True

    def int_get(self, path, params=None):
        if path == "problem_form_fields":
            return {"attachment_settings": {"max_attachment_size_bytes": 4096}}
        if path == f"problems/{PID}/planning-fields":
            return {"problem_planning_fields": copy.deepcopy(self.rows)}
        if path == f"problems/{PID}":
            return {"problem": dict(self.problem)}
        raise AssertionError(path)

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kw):
        path = url.removeprefix(config.API_INT + "/")
        body = json.loads(kw["content"]) if kw.get("content") else None
        self.writes.append((method, path, body))
        if method == "DELETE":  # captured: 204, the section keeps its text
            ident = int(path.rsplit("/", 1)[1])
            for r in self.rows:
                r["attachments"] = [a for a in r.get("attachments") or [] if a["id"] != ident]
                if not self.keep_text:
                    r["description"] = ""
            self.deleted.append(ident)
            return {}
        if path == "attachments":
            ident = UPLOAD_BASE + len(self.uploads)
            data = base64.b64decode(body["content"].partition(",")[2])
            self.uploads[ident] = {"id": ident, "name": body["content_file_name"], "size": len(data)}
            return dict(self.uploads[ident])
        key = parse_qs(urlsplit(path).query)["id"][0] if method == "POST" else urlsplit(path).path.rsplit("/", 1)[1]
        old = next((r for r in self.rows if r["name"] == key), None)
        known = {a["id"]: a for a in (old or {}).get("attachments", [])} | self.uploads
        row = {"id": 9, "name": key, "description": body.get("description", (old or {}).get("description")),
               "description_text": "", "problem_id": PID,
               "attachments": [copy.deepcopy(known[i]) for i in body.get("attachments", [])]}
        self.rows = [r for r in self.rows if r["name"] != key] + [row]
        self.problem["updated_at"] = "2026-01-01T00:05:00Z"
        return {"problem_planning_field": row}


@pytest.fixture
def env(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    c = FakeProblem()
    monkeypatch.setattr(cli, "_client", lambda: c)
    f = tmp_path / "cause.txt"
    f.write_bytes(b"Acme cause\n")
    return c, f


def run(*args):
    return CliRunner().invoke(cli.app, ["problems", "update", f"PRB-{PID}", *args])


def reviewed_send(*args):
    review = run(*args, "--dry-run")
    assert review.exit_code == 0, review.output
    out = json.loads(review.stdout)
    sent = run(*args, "--yes", "--expect-plan", out["plan_hash"])
    return out, sent


def test_first_write_posts_section_with_uploaded_file_and_reads_back(env):
    c, f = env
    out, sent = reviewed_send("--planning", "Root Cause", "--description", "<div>Acme cause</div>",
                              "--file", str(f))
    assert out["action"] == "problem_planning_update" and out["section"] == "problem_cause"
    assert [s["path"] for s in out["steps"]] == ["/api/_/attachments",
                                                  f"/api/_/problems/{PID}/planning-fields?id=problem_cause"]
    assert sent.exit_code == 0, sent.output
    result = json.loads(sent.stdout)
    assert result["verified"] is True and result["uploaded_ids"] == [UPLOAD_BASE]
    upload, save = c.writes
    assert upload[2]["content_file_name"] == "cause.txt" and upload[2]["content_file_size"] == 11
    assert save == ("POST", f"problems/{PID}/planning-fields?id=problem_cause",
                    {"description": "<div>Acme cause</div>", "attachments": [UPLOAD_BASE]})
    replay = run("--planning", "Root Cause", "--description", "<div>Acme cause</div>", "--file", str(f),
                 "--yes", "--expect-plan", out["plan_hash"])
    assert replay.exit_code == 2 and len(c.writes) == 2


def test_existing_section_is_put_and_keeps_its_files(env):
    c, f = env
    out, sent = reviewed_send("--planning", "problem_impact", "--file", str(f), "--duplicate", "append")
    assert sent.exit_code == 0, sent.output
    method, path, body = c.writes[-1]
    assert (method, path) == ("PUT", f"problems/{PID}/planning-fields/problem_impact")
    assert body == {"attachments": [611, UPLOAD_BASE]} and json.loads(sent.stdout)["verified"]


def test_size_limit_and_unknown_section_refuse_before_upload(env, tmp_path):
    c, f = env
    big = tmp_path / "big.bin"
    big.write_bytes(b"x" * 5000)
    assert run("--planning", "Root Cause", "--description", "x", "--file", str(big), "--dry-run").exit_code == 2
    assert run("--planning", "Workaround", "--description", "x", "--dry-run").exit_code == 2
    assert run("--planning", "Root Cause", "--description", "x", "--priority", "High", "--dry-run").exit_code == 2
    assert not c.writes


def test_planning_options_refused_on_tickets(env):
    r = CliRunner().invoke(cli.app, ["tickets", "update", "INC-5", "--planning", "Root Cause", "--description", "x"])
    assert r.exit_code == 2


def test_download_all_planning(env, monkeypatch, tmp_path):
    c, _ = env
    got = []
    monkeypatch.setattr(create, "download_attachment",
                        lambda att, out_dir, **kw: got.append(att["id"]) or {"name": att["name"], "size": 5})
    r = CliRunner().invoke(cli.app, ["problems", "download", f"PRB-{PID}", "--all-planning", "--json",
                                     "--max-bytes", "4096", "--dir", str(tmp_path / "out")])
    assert r.exit_code == 0, r.output
    rows = json.loads(r.stdout)
    assert got == [611]
    assert {x["source"] for x in rows} == {"planning:problem_cause", "planning:problem_impact",
                                           "planning:problem_symptom"}


def _delete(*extra):
    args = ["attachments", "delete", "611", "--owner", f"problems/{PID}", "--section", "problem_impact"]
    review = CliRunner().invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.stdout)["plan_hash"]
    return CliRunner().invoke(cli.app, args + ["--yes", "--expect-plan", digest, *extra])


def test_attachment_delete_on_problem_section_keeps_text(env):
    c, _ = env
    assert _delete().exit_code == 2 and not c.writes  # --confirm-destructive missing
    done = _delete("--confirm-destructive")
    assert done.exit_code == 0, done.output
    result = json.loads(done.stdout)
    assert result["verified"] is True and result["section_text"] == "kept"
    assert c.writes == [("DELETE", "attachments/611", None)]
    assert c.rows[0]["description"] == "<div>Acme impact</div>" and c.rows[0]["attachments"] == []


def test_attachment_delete_unverified_when_section_text_lost(env):
    c, _ = env
    c.keep_text = False
    done = _delete("--confirm-destructive")
    assert done.exit_code == 4 and json.loads(done.stdout)["section_text"] == "changed"
