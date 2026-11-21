"""Expected values for the fsv-api attachment and clone failure matrices
(Python `3432bea`: `tests/test_attachment_failure_matrix.py`,
`tests/test_clone_failure_matrix.py`).

Each case drives the library functions through a real `fsv.client.Client`
over an httpx MockTransport with the same handler as the Python test. The
clone cases run the steps the way `changes clone` does (base create, then
tasks, assets and planning, each attempted once); the CLI's own output and
exit handling are fsv-cli's. Writes record the path with its query string.

    cd /Users/phusit.somb/lab/work/fsv && uv run python \\
        ../fsv-rust/port/golden/gen_fsv_api_failure_matrix.py \\
        > ../fsv-rust/crates/fsv-api/src/testdata/failure_matrix_oracle.json
"""
from __future__ import annotations

import base64
import io
import json
import os
import sys
import tempfile
from contextlib import redirect_stderr
from pathlib import Path

TMP = Path(tempfile.mkdtemp(prefix="fsv-failure-matrix-"))
os.environ["FSV_CONFIG_DIR"] = str(TMP / "config")
os.environ["FSV_CACHE_DIR"] = str(TMP / "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"
os.environ["XDG_STATE_HOME"] = str(TMP / "state")
(TMP / "config").mkdir()
(TMP / "config" / "config.json").write_text('{"domain": "acme.freshservice.com"}')

import httpx  # noqa: E402

from fsv import create  # noqa: E402
from fsv.client import Client  # noqa: E402
from fsv.errors import APIError, SessionError, exit_code_for  # noqa: E402
from provenance import stamped  # noqa: E402


def client(handle):
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    c._get_csrf = lambda: "synthetic-token"
    c._probe_session = lambda: True
    return c


def error(e):
    out = {"error": type(e).__name__, "msg": str(e), "exit": exit_code_for(e)}
    if isinstance(e, APIError):
        out["status"] = e.status
    return out


def attempt(fn):
    try:
        return {"ok": fn()}
    except Exception as e:  # noqa: BLE001
        return error(e)


def rel(req):
    return req.url.raw_path.decode().removeprefix("/api/_/")


# ------------------------------------------------------------ attachments

def attach_wire():
    state = {"writes": [], "downloads": [], "fail": None, "upload_result": None}

    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "changes/123":
                return httpx.Response(200, json={"change": {"attachments": [
                    {"id": 71, "name": "plan.txt", "attachment_url": "https://example.com/old"},
                    {"id": 72, "name": "keep.txt"}]}})
            assert str(req.url) == "https://example.com/old"
            state["downloads"].append(str(req.url))
            return httpx.Response(503 if state["fail"] == "download" else 200, content=b"Acme old bytes")
        body = json.loads(req.content)
        state["writes"].append([req.method, rel(req), body])
        if state["fail"] == path:
            return httpx.Response(503, json={"description": "synthetic outage"})
        if path == "attachments":
            default = {"id": 90 + len(state["writes"]), "content_file_name": body["content_file_name"],
                       "content_file_size": body["content_file_size"]}
            return httpx.Response(200, json=state["upload_result"] if state["upload_result"] is not None else default)
        assert path == "changes/123"
        return httpx.Response(200, json={"change": {"id": 123}})

    c = client(handle)
    create.get_client = lambda: c
    return state, c


def plan_file(data):
    d = Path(tempfile.mkdtemp(dir=TMP))
    p = d / "plan.txt"
    p.write_bytes(data)
    return p


def strip_volatile(writes):
    """Upload bodies minus `content_updated_at` (a clock reading)."""
    return [[m, p, {k: v for k, v in b.items() if k != "content_updated_at"} if isinstance(b, dict) else b]
            for m, p, b in writes]


def gen_attachments():
    out = {"policies": [], "replace_failure": [], "missing_id": [], "receipt": []}
    for mode, backup in [("skip", False), ("append", False), ("replace", False), ("replace", True)]:
        state, c = attach_wire()
        path = plan_file(b"Globex new bytes")
        r = attempt(lambda: create.attach_files_to_change(123, [str(path)], duplicate=mode, backup_replaced=backup,
                                                          backup_name="plan.backup.txt"))
        uploads = [b for _, p, b in state["writes"] if p == "attachments"]
        out["policies"].append({"mode": mode, "backup": backup, **r, "writes": strip_volatile(state["writes"]),
                                "upload_bytes": [base64.b64decode(b["content"].split(",", 1)[1]).decode()
                                                 for b in uploads],
                                "downloads": state["downloads"]})
    for failure in ["download", "attachments", "changes/123"]:
        state, c = attach_wire()
        state["fail"] = failure
        path = plan_file(b"Globex new bytes")
        r = attempt(lambda: create.attach_files_to_change(123, [str(path)], duplicate="replace", backup_replaced=True,
                                                          backup_name="plan.backup.txt"))
        out["replace_failure"].append({"failure": failure, **r, "writes": strip_volatile(state["writes"]),
                                       "downloads": state["downloads"]})
    for response in [{}, [], {"name": "plan.txt"}]:
        state, c = attach_wire()
        state["upload_result"] = response
        path = plan_file(b"new")
        r = attempt(lambda: create.attach_files_to_change(123, [str(path)], duplicate="append"))
        out["missing_id"].append({"response": response, **r, "writes": strip_volatile(state["writes"])})
    for response in [{"id": "91", "content_file_name": "plan.txt", "content_file_size": 3},
                     {"id": 91, "name": "renamed.txt", "size": 3},
                     {"id": 91, "name": "plan.txt", "size": 99}]:
        state, c = attach_wire()
        state["upload_result"] = response
        buf = io.StringIO()
        with redirect_stderr(buf):
            r = attempt(lambda: create._upload_bytes("plan.txt", b"new", "text/plain", c))
        out["receipt"].append({"response": response, **r, "stderr": buf.getvalue(), "writes": state["writes"]})
    state, c = attach_wire()
    out["invalid_mode"] = {**attempt(lambda: create.attach_files_to_change(123, ["missing.txt"], duplicate="typo")),
                           "writes": state["writes"]}
    state, c = attach_wire()
    state["fail"] = "download"
    d = Path(tempfile.mkdtemp(dir=TMP))
    (d / "plan.txt").write_bytes(b"local bytes")
    out["download_error"] = {
        **attempt(lambda: create.download_attachment({"name": "plan.txt", "attachment_url": "https://example.com/old"},
                                                     d, force=True)),
        "kept": (d / "plan.txt").read_bytes().decode(), "downloads": state["downloads"]}
    state, c = attach_wire()
    absent = TMP / "absent"
    out["missing_url"] = {**attempt(lambda: create.download_attachment({"name": "plan.txt"}, absent)),
                          "created_dir": absent.exists(), "downloads": state["downloads"]}
    return out


# ------------------------------------------------------------ clone

CLONE_ROWS = {
    "changes/123": {"change": {"id": 123, "subject": "Acme maintenance", "status": 47,
                               "attachments": [{"id": 11}], "state_flow_id": 27}},
    "changes/123/tasks": {"tasks": [{"id": 501, "title": "Verify Acme", "owner_id": 71, "status": 3}]},
    "changes/123/assets": {"associated_assets": [{"id": 801, "config_item": {"display_id": 701}}]},
    "changes/123/planning-fields": {"change_planning_fields": [
        {"name": "test_plan", "description": "Acme checks", "attachments": []}]},
}


def clone_wire():
    state = {"writes": [], "reads": [], "fail": None, "status": 500, "missing_id": False}

    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            state["reads"].append(path)
            assert path in CLONE_ROWS, path
            return httpx.Response(200, json=CLONE_ROWS[path])
        body = json.loads(req.content)
        state["writes"].append([req.method, rel(req), body])
        kind = ("base" if path == "changes" else "tasks" if path.endswith("/tasks") else
                "assets" if path.endswith("/associate") else "planning")
        if kind == state["fail"]:
            return httpx.Response(state["status"], json={"description": "synthetic denial"})
        if kind == "base":
            return httpx.Response(200, json={"change": {} if state["missing_id"] else
                                             {"id": 456, "human_display_id": "CHN-456"}})
        return httpx.Response(200, json={"task": {"id": 901}})

    c = client(handle)
    create.get_client = lambda: c
    create._get_change_schema_fields = lambda: []
    return state, c


def run_clone(state, c):
    """`changes clone ID --no-edit --with-tasks --with-assets --with-planning`'s library calls."""
    data = create.change_clone_data(123, c)
    created = create.submit_change(data, c)
    out = {"data": data, "created": created, "steps": {}}
    new = created.get("id")
    failures = []
    if new is not None:
        for name, fn in [("tasks", lambda: create.clone_tasks(123, new, c, dropped=[])),
                         ("assets", lambda: create.clone_assets(123, new, c)),
                         ("planning", lambda: create.clone_planning_fields(123, new, c))]:
            try:
                out["steps"][name] = {"ok": fn()}
            except (SessionError, APIError, ValueError, RuntimeError) as e:
                out["steps"][name] = error(e)
                failures.append(e)
    out["exit"] = exit_code_for(failures[0]) if failures else 0
    out["writes"] = state["writes"]
    out["reads"] = state["reads"]
    return out


def gen_clone():
    out = {"partial": []}
    for failed in ["tasks", "assets", "planning"]:
        for status in [403, 409, 429, 500]:
            state, c = clone_wire()
            state.update(fail=failed, status=status)
            out["partial"].append({"failed": failed, "status": status, **run_clone(state, c)})
    state, c = clone_wire()
    out["side_effects"] = {"data": create.change_clone_data(123, c), "effects": create.clone_side_effects(123, c),
                           "writes": state["writes"], "reads": state["reads"]}
    state, c = clone_wire()
    state["missing_id"] = True
    out["missing_id"] = run_clone(state, c)
    return out


print(json.dumps(stamped({"attachments": gen_attachments(), "clone": gen_clone()}), indent=1, ensure_ascii=False))
