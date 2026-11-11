"""Write journal and editor round-trips. Hermetic: journal goes to the tmp
XDG_STATE_HOME set by conftest."""
from __future__ import annotations

import json
import os
import stat
import sys

import httpx
import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import client as client_mod
from fsv import editor, journal
from fsv.errors import APIError

runner = CliRunner()


def _client_with(handler):
    c = client_mod.Client(cookies={"_x_m": "SECRETCOOKIE", "fw-session-id": "SECRETSESSION"})
    c._client = httpx.Client(transport=httpx.MockTransport(handler), follow_redirects=False,
                             cookies=c._client.cookies)
    c._csrf = "tok"
    return c


def test_journal_path_respects_xdg_state_home(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path / "st"))
    assert journal.journal_path() == tmp_path / "st" / "fsv" / "journal.jsonl"
    assert str(journal.journal_path()).startswith(str(tmp_path))


def test_plan_hash_is_order_independent_and_none_for_no_body():
    assert journal.plan_hash({"a": 1, "b": [2]}) == journal.plan_hash({"b": [2], "a": 1})
    assert journal.plan_hash({"a": 1}) != journal.plan_hash({"a": 2})
    assert journal.plan_hash(None) is None


def test_successful_write_is_journaled_without_secrets():
    c = _client_with(lambda req: httpx.Response(200, json={"change": {"id": 4321}}))
    c.int_post("changes", {"subject": "TOPSECRETBODY"})
    rows = journal.read()
    assert len(rows) == 1
    r = rows[0]
    assert r["method"] == "POST" and r["path"].endswith("/changes") and r["ok"] is True
    assert r["status"] == 200 and r["created_id"] == 4321 and r["kind"] == "create:changes"
    assert r["plan_hash"] == journal.plan_hash({"subject": "TOPSECRETBODY"})
    raw = journal.journal_path().read_text()
    for secret in ("SECRETCOOKIE", "SECRETSESSION", "TOPSECRETBODY", "tok"):
        assert secret not in raw
    assert stat.S_IMODE(os.stat(journal.journal_path()).st_mode) == 0o600


def test_failed_write_is_journaled_and_reads_are_not():
    def handler(req):
        if req.method == "GET":
            return httpx.Response(200, json={})
        return httpx.Response(422, json={"errors": "nope"})
    c = _client_with(handler)
    c.int_get("changes/1")
    with pytest.raises(APIError):
        c.int_put("changes/1", {"status": 2})
    rows = journal.read()
    assert len(rows) == 1
    assert rows[0]["ok"] is False and rows[0]["status"] == 422 and rows[0]["target"] == "changes/1"
    assert rows[0]["created_id"] is None


def test_journal_failure_never_breaks_the_command(monkeypatch, tmp_path):
    blocker = tmp_path / "file"
    blocker.write_text("x")
    monkeypatch.setenv("XDG_STATE_HOME", str(blocker))  # a file where a dir is needed
    c = _client_with(lambda req: httpx.Response(200, json={"id": 1}))
    assert c.int_post("changes", {"a": 1}) == {"id": 1}


def test_journal_cli_ls_and_tail():
    c = _client_with(lambda req: httpx.Response(200, json={"id": 9}))
    for i in range(3):
        c.int_post("changes", {"n": i})
    out = runner.invoke(cli.app, ["journal", "tail", "-n", "2", "--json"]).output.strip().splitlines()
    assert len(out) == 2 and json.loads(out[-1])["plan_hash"] == journal.plan_hash({"n": 2})
    assert len(runner.invoke(cli.app, ["journal", "ls", "--json"]).output.strip().splitlines()) == 3
    assert runner.invoke(cli.app, ["journal", "path"]).output.strip() == str(journal.journal_path())


def test_command_name_recorded_without_arguments(monkeypatch):
    c = _client_with(lambda req: httpx.Response(200, json={"note": {"id": 1}}))
    monkeypatch.setattr(cli, "_client", lambda: c)
    r = runner.invoke(cli.app, ["changes", "add-note", "CHN-1", "private words"])
    assert r.exit_code == 0, r.output
    row = journal.read()[-1]
    assert row["kind"] == "note" and "changes add-note" in row["command"]
    assert "private words" not in journal.journal_path().read_text()


# ── editor ───────────────────────────────────────────────────────────────────
def _fake_editor(monkeypatch, edits):
    """edits: list of callables(text)->text, applied one per editor launch."""
    seen = []

    def run(argv, check=False):
        path = argv[-1]
        text = open(path, encoding="utf-8").read()
        seen.append(text)
        fn = edits[min(len(seen) - 1, len(edits) - 1)]
        new = fn(text)
        if new is not None:
            open(path, "w", encoding="utf-8").write(new)

    monkeypatch.setattr(editor.subprocess, "run", run)
    return seen


def test_json_error_reopens_editor_with_error_and_keeps_edits(monkeypatch):
    seen = _fake_editor(monkeypatch, [
        lambda t: t.replace('"a": 1', '"a": 1,, "keep": 7'),   # invalid JSON
        lambda t: t.replace(",,", ","),                         # user fixes it
    ])
    out = editor.edit_json({"a": 1})
    assert out == {"a": 1, "keep": 7}
    assert len(seen) == 2
    assert seen[1].lstrip().startswith("// ERROR:") and '"keep": 7' in seen[1]


def test_unchanged_by_content_not_mtime(monkeypatch):
    def touch_only(text):
        os.utime  # no-op marker
        return text  # rewrite identical content: mtime changes, content does not
    _fake_editor(monkeypatch, [touch_only])
    with pytest.raises(editor.EditorAbort):
        editor.edit_json({"a": 1})


def test_changed_content_with_same_mtime_is_accepted(monkeypatch):
    def edit(text):
        return text.replace('"a": 1', '"a": 2')
    _fake_editor(monkeypatch, [edit])
    monkeypatch.setattr(os.path, "getmtime", lambda p: 1.0)
    assert editor.edit_json({"a": 1}) == {"a": 2}


def test_unchanged_after_error_aborts(monkeypatch):
    _fake_editor(monkeypatch, [lambda t: t + "{{", lambda t: t])
    with pytest.raises(editor.EditorAbort):
        editor.edit_json({"a": 1})


def test_visual_beats_editor(monkeypatch):
    monkeypatch.setenv("EDITOR", "ed-editor")
    monkeypatch.setenv("VISUAL", "vis-editor --wait")
    assert editor._editor() == ["vis-editor", "--wait"]
    monkeypatch.delenv("VISUAL")
    assert editor._editor() == ["ed-editor"]


def test_edit_text_generic_yaml_style_parse_error(monkeypatch):
    def parse(text):
        if "bad" in text:
            raise ValueError("line 1: bad value")
        return text.strip()
    seen = _fake_editor(monkeypatch, [lambda t: "bad\n", lambda t: "good\n"])
    assert editor.edit_text("start\n", parse, suffix=".yaml") == "good"
    assert seen[1].startswith("# ERROR: line 1: bad value")
