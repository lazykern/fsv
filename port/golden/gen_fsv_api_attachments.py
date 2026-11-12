"""Golden `attachments` (Python 1c186e0, published page limit ba6aef9, signed
link churn 8f156c0) for
fsv-api `attachments`: hash-bound local files, the two-step upload/owner wires,
plan hashes, reviews, owner readback, ambiguity and drift refusals, metadata
and tenant-page size limits, `--max-bytes` overrides, and owner-verified
deletion. Mirrors `test_attachments_reviewed.py` (the non-CLI
tests, for both tenants, plus the deletes its CLI test drives). File paths
are not recorded (only name, size, type and sha256 are bound); the Rust test
recreates the same files. The Rust test replays the ops.

    cd /tmp/fsv-py-8f156c0 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=8f156c0... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_attachments.py
"""
from __future__ import annotations

import base64
import copy
import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import attachments as flow, config, formmeta, journal  # noqa: E402
from fsv.client import Client  # noqa: E402
from fsv.reviewed_write import WritePlan  # noqa: E402
from provenance import stamped  # noqa: E402

formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: 1
FILES = {"acme.txt": "Acme evidence 0\n", "globex.txt": "Acme evidence 1\n"}


def new_state(base):
    return {"uploads": {}, "notes": [], "conversations": [], "ticket": {"id": 123, "attachments": []},
            "problem": {"id": 123, "attachments": []}, "limit": 4096, "meta": None, "fail": None,
            "fail_step": "upload", "mismatch": False, "missing_id": False, "partial": False, "base": base,
            "planning": [], "requests": [], "page": None, "page_status": 200, "page_html": None}


def handler_for(s):
    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "/a/dashboard":
                script = "" if s["page"] is None else "".join(f"window.MAX_ATTACHMENT_LIMIT = {v};\n" for v in s["page"])
                html = f'<html><head><meta name="csrf-token" content="synthetic"></head><script>{script}</script></html>'
                headers = {"location": "/login"} if 300 <= s["page_status"] < 400 else {}
                return httpx.Response(s["page_status"], text=s["page_html"] or html,
                                      headers={"content-type": "text/html", **headers})
            if path in {"ticket_form_fields", "problem_form_fields"}:
                if s["meta"] is not None:
                    return httpx.Response(200, json=s["meta"])
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
            if path == "problems/123":
                return httpx.Response(200, json={"problem": copy.deepcopy(s["problem"])})
            if path == "problems/123/planning-fields":
                return httpx.Response(200, json={"problem_planning_fields": copy.deepcopy(s["planning"])})
            return httpx.Response(404, json={"error": "unexpected"})
        s["requests"].append({"method": req.method, "path": path, "raw": req.content.decode()})
        body = json.loads(req.content) if req.content else None
        step = "upload" if path == "attachments" else "owner"
        if s["fail"] and step == s["fail_step"]:
            if s["fail"] == "disconnect":
                raise httpx.ReadError("disconnect", request=req)
            return httpx.Response(s["fail"], json={"error": "failed"})
        if req.method == "DELETE":
            ident = int(path.rsplit("/", 1)[1])
            for row in [s["ticket"], s["problem"], *s["notes"], *s["conversations"], *s["planning"]]:
                row["attachments"] = [a for a in row.get("attachments", []) if a["id"] != ident]
            return httpx.Response(204)
        if path == "attachments":
            ident = s["base"] + len(s["uploads"])
            data = base64.b64decode(body["content"].partition(",")[2])
            s["uploads"][ident] = {"id": ident, "name": body["content_file_name"], "size": str(len(data)),
                                   "content_type": body["content_content_type"] + ";charset=utf-8"}
            return httpx.Response(200, json={} if s["missing_id"] else s["uploads"][ident])
        attachments = [copy.deepcopy(s["uploads"][i]) for i in body["attachments"]]
        if path == "tickets":
            s["ticket"] = {"id": 123, **body, "attachments": attachments}
            return httpx.Response(201, json={"ticket": {"id": 123}})
        row = {"id": 901, **body, "attachments": attachments, "body": "<div>Acme evidence</div>"}
        if path == "problems/123/notes":
            s["notes"].append(row)
            return httpx.Response(201, json={"note": {"id": 901}})
        if "notify_emails" in row:
            row["to_emails"] = row.pop("notify_emails")
        s["conversations"].append(row)
        return httpx.Response(201, json={"conversation": {"id": 901}})
    return handle


def client(handler):
    c = Client.__new__(Client)
    c._client = httpx.Client(transport=httpx.MockTransport(handler), follow_redirects=False)
    c._csrf = "synthetic"
    c._fw_domain = c._fw_session_id = None
    c._persist = False
    c._rl_rem = c._rl_tot = None
    c._probe_ok_until = 0
    c._probe_session = lambda: True
    return c


def builder(c, paths, kind, notify=(), max_bytes=None):
    if kind == "problem":
        return lambda: flow.problem_note_plan(c, "PRB-123", "Acme evidence", paths, max_bytes=max_bytes)
    if kind == "create":
        def build():
            raw = flow.metadata(c, "tickets")
            base = WritePlan("ticket_create_attachments", 0, config.DOMAIN, "POST", "tickets",
                             json.dumps({"subject": "Acme incident"}), journal.plan_hash(raw))
            return flow.wrap(base, paths, raw, client=c, max_bytes=max_bytes)
        return build
    return lambda: flow.conversation_plan(c, "INC-123", "Acme evidence", paths, kind=kind,
                                         sender="support@example.com" if kind == "reply" else None,
                                         notify=tuple(notify), max_bytes=max_bytes)


def plan_out(p):
    return {"base": dict(p.base.__dict__), "files": [f.review() for f in p.files], "limit": p.limit,
            "limit_source": p.limit_source, "published_limit": p.published_limit, "hash": p.hash, "review": p.review()}


def run(case):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    config.DOMAIN = case.get("tenant", "acme.example.com")
    config.API_INT = f"https://{config.DOMAIN}/api/_"
    s = new_state(case.get("base", 701))
    c = client(handler_for(s))
    tmp = Path(tempfile.mkdtemp(prefix="fsv-files-"))
    for name, text in FILES.items():
        (tmp / name).write_text(text)
    plan = build = delete_plan = None
    outputs = []
    for op in case["ops"]:
        try:
            if op["op"] == "build":
                paths = [str(tmp / n) for n in op.get("files", list(FILES))]
                build = builder(c, paths, op["kind"], op.get("notify", ()), op.get("max_bytes"))
                plan = build()
                out = {"plan": plan_out(plan)}
            elif op["op"] == "send":
                reviewed = {"full": plan.hash, "none": None, "prefix": plan.hash[:16]}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed, rebuild=build)}
            elif op["op"] == "files":
                out = {"files": [f.review() for f in flow.local_files([str(tmp / n) for n in op["files"]], op["limit"])]}
            elif op["op"] == "limit":
                out = {"limit": flow.size_limit(op["metadata"])}
            elif op["op"] == "resolve":
                out = {"resolved": dict(flow.resolve_limit(c, op["metadata"], op.get("max_bytes")).__dict__)}
            elif op["op"] == "write":
                (tmp / op["name"]).write_bytes(op["text"].encode())
                out = {}
            elif op["op"] == "delete_build":
                delete_plan = flow.delete_plan(c, op["ident"], op["owner"], child=op.get("child"), section=op.get("section"))
                out = {"plan": dict(delete_plan.__dict__), "hash": delete_plan.hash, "review": delete_plan.review()}
            elif op["op"] == "delete_send":
                reviewed = {"full": delete_plan.hash, "none": None}[op.get("hash", "full")]
                out = {"result": flow.delete(delete_plan, c, op["owner"], reviewed_hash=reviewed,
                                             child=op.get("child"), section=op.get("section"))}
            elif op["op"] == "set":
                for k, v in copy.deepcopy(op.get("state", {})).items():
                    s[k] = v
                for k, v in op.get("ticket", {}).items():
                    s["ticket"][k] = v
                out = {}
            else:
                raise AssertionError(op)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["writes"] = len(s["requests"])
        outputs.append(out)
    c.close()
    events = [[r["kind"], r["event"], r.get("created_id")] for r in journal.read()
              if str(r.get("kind", "")).startswith(("plan:ticket_", "plan:problem_", "plan:attachment"))]
    return {**case, "outputs": outputs, "events": events, "writes": s["requests"]}


S = {"op": "send"}
CASES = {}
for tenant, base in [("acme.example.com", 701), ("globex.example.com", 801)]:
    t = tenant.split(".")[0]
    for kind in ["note", "reply", "problem", "create"]:
        CASES[f"two_step_{kind}_{t}"] = {"tenant": tenant, "base": base, "ops": [{"op": "build", "kind": kind}, S, S]}
    for failure in [403, 429, 500, "disconnect", 400]:
        for step in ["upload", "owner"]:
            CASES[f"ambiguous_{failure}_{step}_{t}"] = {"tenant": tenant, "base": base, "ops": [
                {"op": "build", "kind": "note", "files": ["acme.txt"]},
                {"op": "set", "state": {"fail": failure, "fail_step": step}}, S, S]}
    CASES[f"delete_note_{t}"] = {"tenant": tenant, "base": base, "ops": [
        {"op": "build", "kind": "note"}, S,
        {"op": "delete_build", "ident": str(base), "owner": "tickets/123", "child": 901},
        {"op": "delete_send", "owner": "tickets/123", "child": 901},
        {"op": "delete_send", "owner": "tickets/123", "child": 901}]}
    CASES[f"delete_problem_{t}"] = {"tenant": tenant, "base": base, "ops": [
        {"op": "build", "kind": "problem"}, S,
        {"op": "delete_build", "ident": base + 1, "owner": "problems/123", "child": 901},
        {"op": "delete_send", "owner": "problems/123", "child": 901}]}
    CASES[f"delete_parent_{t}"] = {"tenant": tenant, "base": base, "ops": [
        {"op": "build", "kind": "create"}, S,
        {"op": "delete_build", "ident": base, "owner": "tickets/123"},
        {"op": "set", "state": {"mismatch": True}},
        {"op": "delete_send", "owner": "tickets/123", "hash": "none"},
        {"op": "delete_send", "owner": "tickets/123"}]}
    CASES[f"delete_signed_{t}"] = {"tenant": tenant, "base": base, "ops": [
        {"op": "set", "state": {"rotate": True, "conversations": [
            {"id": 901, "body": "Acme note", "attachments": [{"id": base, "name": "acme.txt", "size": 12}]}]}},
        {"op": "delete_build", "ident": str(base), "owner": "tickets/123", "child": 901},
        {"op": "delete_build", "ident": str(base), "owner": "tickets/123", "child": 901},
        {"op": "delete_send", "owner": "tickets/123", "child": 901}]}
for drift in ["file", "limit", "owner"]:
    ops = [{"op": "build", "kind": "note"}]
    if drift == "file":
        ops.append({"op": "write", "name": "acme.txt", "text": "changed"})
    elif drift == "limit":
        ops.append({"op": "set", "state": {"limit": 4097}})
    else:
        ops.append({"op": "set", "ticket": {"subject": "changed"}})
    CASES[f"drift_{drift}"] = {"ops": ops + [S]}
for i, limit in enumerate([None, 0, True, "4096", 1, 4096.0, -1]):  # None: metadata publishes none
    CASES[f"limit_{i}"] = {"ops": [{"op": "set", "state": {"limit": limit}}, {"op": "build", "kind": "note"}]}
CASES["missing_upload_id"] = {"ops": [{"op": "build", "kind": "note"}, {"op": "set", "state": {"missing_id": True}}, S]}
for mode in ["mismatch", "partial"]:
    CASES[f"readback_{mode}"] = {"ops": [{"op": "build", "kind": "note"}, {"op": "set", "state": {mode: True}}, S]}
CASES["note_notify"] = {"ops": [{"op": "build", "kind": "note", "notify": ["ops@example.com"]}, S]}
CASES["guards"] = {"ops": [
    {"op": "build", "kind": "reply_nosender"} if False else {"op": "build", "kind": "note", "files": ["acme.txt", "acme.txt"]},
    {"op": "build", "kind": "note", "files": ["missing.txt"]}, {"op": "build", "kind": "note", "files": []},
    {"op": "write", "name": "empty.txt", "text": ""}, {"op": "build", "kind": "note", "files": ["empty.txt"]},
    {"op": "write", "name": "a\u0001b.txt", "text": "x"}, {"op": "build", "kind": "note", "files": ["a\u0001b.txt"]},
    {"op": "write", "name": "notes.csv", "text": "a,b\n"}, {"op": "write", "name": "blob", "text": "x"},
    {"op": "write", "name": "photo.PNG", "text": "x"}, {"op": "write", "name": "caf\u00e9.json", "text": "{}"},
    {"op": "files", "files": ["notes.csv", "blob", "photo.PNG", "caf\u00e9.json"], "limit": 100},
    {"op": "files", "files": ["acme.txt"], "limit": 3},
    {"op": "build", "kind": "note"}, {**S, "hash": "none"}, {**S, "hash": "prefix"}]}
CASES["limits"] = {"ops": [{"op": "limit", "metadata": m} for m in [
    {"ticket_fields": [{"field_options": {"max_file_size_bytes": 10}}, {"settings": {"max_attachment_size_bytes": 10}}]},
    {"ticket_fields": [{"field_options": {"max_file_size_bytes": 10}}, {"settings": {"max_attachment_size_bytes": 11}}]},
    {"other": {"max_attachment_size_bytes": 10}}, [{"attachments": {"max_attachment_size_bytes": 5}}], {},
    {"fields": [{"options": {"max_file_size_bytes": False}}]}, {"max_attachment_size_bytes": 7, "settings": {"max_file_size_bytes": "x"}}]]}
CASES["delete_refusals"] = {"ops": [
    {"op": "build", "kind": "note"}, S,
    {"op": "delete_build", "ident": 999, "owner": "tickets/123", "child": 901},
    {"op": "delete_build", "ident": 701, "owner": "tickets/0"},
    {"op": "delete_build", "ident": 701, "owner": "tickets/123/"},
    {"op": "delete_build", "ident": 701, "owner": "assets/1"},
    {"op": "delete_build", "ident": 701, "owner": "tickets/123", "child": 901, "section": "problem_cause"},
    {"op": "delete_build", "ident": 701, "owner": "changes/123", "child": 1},
    {"op": "delete_build", "ident": 701, "owner": "tickets/123", "child": 902},
    {"op": "delete_build", "ident": "x", "owner": "tickets/123"},
    {"op": "delete_build", "ident": 701, "owner": "tickets/123"},
    {"op": "set", "state": {"planning": [{"id": 1, "name": "problem_cause", "attachments": [{"id": 55, "name": "x"}]}]}},
    {"op": "delete_build", "ident": 55, "owner": "problems/123", "section": "problem_cause"},
    {"op": "delete_send", "owner": "problems/123", "section": "problem_cause"},
    {"op": "delete_build", "ident": 55, "owner": "problems/123", "section": "problem_impact"},
    {"op": "set", "state": {"partial": True}},
    {"op": "delete_build", "ident": 701, "owner": "tickets/123", "child": 901}]}
for kind in ["note", "reply", "problem", "create"]:
    CASES[f"page_{kind}"] = {"ops": [{"op": "set", "state": {"limit": None, "page": ["40"]}},
                                     {"op": "build", "kind": kind}, S]}
CASES["page_total"] = {"ops": [{"op": "set", "state": {"limit": None, "page": ["0.00002"]}},
                               {"op": "build", "kind": "note", "files": ["acme.txt"]}, {"op": "build", "kind": "note"}]}
CASES["page_drift"] = {"ops": [{"op": "set", "state": {"limit": None, "page": ["40"]}}, {"op": "build", "kind": "note"},
                               {"op": "set", "state": {"page": ["20"]}}, S]}
page_ops = []
for page in [["40", "20"], ["0"], ["-1"], ["window.x"], ["1e3"], ["40", "40"], ["1.5"], ["0.5 "], ["00012.250"],
             ["1."], [".5"], ["0.0000001"], ["٣"]]:
    page_ops += [{"op": "set", "state": {"limit": None, "page": page}}, {"op": "build", "kind": "note"}]
for status in [302, 401, 403, 500, 404]:
    page_ops += [{"op": "set", "state": {"limit": None, "page": ["40"], "page_status": status}}, {"op": "build", "kind": "note"}]
for html in ['<html><form action="/login"><input type="password"></form>MAX_ATTACHMENT_LIMIT = 40;</html>',
             '<meta name="csrf-token" content="x">xMAX_ATTACHMENT_LIMIT = 40; MAX_ATTACHMENT_LIMIT=7 ;',
             '<meta name="csrf-token" content="x">MAX_ATTACHMENT_LIMIT = 40 ;MAX_ATTACHMENT_LIMIT\t=\n40;']:
    page_ops += [{"op": "set", "state": {"limit": None, "page_status": 200, "page_html": html}}, {"op": "build", "kind": "note"}]
CASES["page_refusals"] = {"ops": page_ops}
CASES["unpublished_override"] = {"ops": [
    {"op": "set", "state": {"limit": None}}, {"op": "build", "kind": "note"}, S,
    {"op": "build", "kind": "note", "max_bytes": 31}, {"op": "build", "kind": "note", "max_bytes": 32}, S]}
CASES["override_hash_bound"] = {"ops": [
    {"op": "set", "state": {"limit": None, "page": ["40"]}}, {"op": "build", "kind": "note", "max_bytes": 4096},
    {"op": "build", "kind": "note"}, {"op": "build", "kind": "note", "max_bytes": 0},
    {"op": "build", "kind": "note", "max_bytes": -1}, {"op": "build", "kind": "problem", "max_bytes": 40}]}
CASES["resolve"] = {"ops": [
    {"op": "resolve", "metadata": {"attachment_settings": {"max_attachment_size_bytes": 4096}, "fields": [{"max_file_size_bytes": 2048}]}},
    {"op": "resolve", "metadata": {"attachment_settings": {"max_attachment_size_bytes": 4096}}},
    {"op": "resolve", "metadata": {"attachment_settings": {"max_attachment_size_bytes": 4096}}, "max_bytes": 10},
    {"op": "resolve", "metadata": {}}, {"op": "set", "state": {"page": ["2"]}}, {"op": "resolve", "metadata": {}},
    {"op": "resolve", "metadata": {}, "max_bytes": 5}, {"op": "set", "state": {"page_status": 500}},
    {"op": "resolve", "metadata": {}, "max_bytes": 5}]}
json.dump(stamped({name: run(case) for name, case in CASES.items()}), sys.stdout, indent=1, ensure_ascii=False)
