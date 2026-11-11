"""Golden outputs for fsv-app's form send paths (`crates/fsv-app/src/form/tests.rs`).

Runs the Python CLI send helpers (`change_cli._send`, `cli_sr._send`,
`change_cli._report_partial`) and `FormSession.send` over the same mocked
tenant the Rust tests route, with the form metadata changed between the
review and the send. Prints one JSON object: scenario -> stderr text,
confirm questions, exit code/message, writes.

    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_app_send.py
"""
from __future__ import annotations

import io
import json
import os
import sys
import tempfile
from contextlib import redirect_stderr
from urllib.parse import parse_qs

ROOT = tempfile.mkdtemp(prefix="fsv-golden-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(ROOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(ROOT, "cache")
os.environ["XDG_STATE_HOME"] = os.path.join(ROOT, "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"
os.environ["COLUMNS"] = "80"
os.environ.pop("FSV_TZ", None)
os.makedirs(os.environ["FSV_CONFIG_DIR"])
with open(os.path.join(os.environ["FSV_CONFIG_DIR"], "config.json"), "w") as f:
    f.write('{"domain": "acme.freshservice.com"}')

import httpx  # noqa: E402
import typer  # noqa: E402

from fsv import change_cli, cli_sr, config  # noqa: E402
from fsv import client as client_mod  # noqa: E402
from fsv.errors import exit_code_for  # noqa: E402
from fsv.formui.session import change_session, sr_session  # noqa: E402
from fsv.forms import LookupValue  # noqa: E402
from fsv.tui import actions  # noqa: E402

config.reload()

DASHBOARD = '<html><head><meta name="csrf-token" content="tok"></head><body>app</body></html>'


def field(i, name, label, ft, required, default):
    return {"id": i, "name": name, "label": label, "field_type": ft, "required": required, "position": i,
            "default_field": default, "choices": [], "nested_fields": [], "workspace_id": 2}


def item(extra=False, files=False):
    cf = [{"id": 21, "name": "reason", "label": "Reason", "type": 1001, "required": False, "account_id": 1}]
    if extra:
        cf.append({"id": 22, "name": "urgency", "label": "Urgency", "type": 1001, "required": False,
                   "account_id": 1})
    return {"id": 11, "display_id": 5, "name": "Laptop", "allow_attachments": files,
            "configs": {"attachment_mandatory": False}, "custom_fields": cf}


# Hides the new `urgency` field: the item's br_validation_excludes changes.
SR_RULE = {"id": 1, "name": "hide", "workspace_id": 2, "module_type": "service_request", "item_id": 11,
           "deleted": False, "user_type": "both", "form_type": "both", "state": "active",
           "rule_type": "client_side", "description": "",
           "condition_data": {"all": [{"name": "customField.reason", "operator": "is", "value": ["broken é"],
                                       "evaluate_on": "change"}]},
           "action_data": {"action": [{"name": ["customField.urgency"], "property": "hide"}]},
           "is_reversible": True, "is_validation": False}


class Tenant:
    """`route` of form/tests.rs; `phase` switches the metadata after review."""

    def __init__(self) -> None:
        self.phase = "review"
        self.requests: list[httpx.Request] = []
        self.fail_put = False
        self.place_status = 200
        self.upload_status = 200
        self.files = False

    def change_fields(self):
        f = [field(1, "subject", "Subject", "default_subject", True, True),
             field(2, "requester", "Requester", "default_requester", True, True),
             field(3, "cf_note", "Note", "custom_text", False, False)]
        if self.phase == "promoted":
            f[2]["default_field"] = True
        if self.phase == "required":
            f.append(field(4, "cf_reason", "Reason", "custom_text", True, False))
        return {"change_fields": f}

    def __call__(self, req: httpx.Request) -> httpx.Response:
        self.requests.append(req)
        p = req.url.path
        if req.method == "GET":
            if p == "/a/dashboard":
                return httpx.Response(200, text=DASHBOARD, headers={"content-type": "text/html"})
            p = p.removeprefix("/api/_/")
            if p == "change_form_fields":
                return httpx.Response(200, json=self.change_fields())
            if p.endswith("business_rules"):
                rules = [SR_RULE] if self.phase == "item" else []
                return httpx.Response(200, json={"business_rules": rules, "user": {"time_zone": "Tokyo"}})
            if p == "catalog/service-items/5":
                return httpx.Response(200, json={"service_item": item(self.phase == "item", self.files)})
            if p == "bootstrap/agents_groups":
                return httpx.Response(200, json={"groups": [{"id": 4, "name": "Ops"}]})
            return httpx.Response(200, json={})
        p = p.removeprefix("/api/_/")
        if req.method == "POST" and p == "changes":
            return httpx.Response(200, json={"change": {"id": 78, "display_id": 78}})
        if req.method == "POST" and p == "attachments":
            if self.upload_status != 200:
                return httpx.Response(self.upload_status, json={"description": "upload boom"})
            body = json.loads(req.content or b"{}")
            return httpx.Response(200, json={"id": 9000005001, "content_file_name": body.get("content_file_name"),
                                             "content_file_size": body.get("content_file_size")})
        if req.method == "PUT" and p == "changes/78":
            return httpx.Response(500, json={"description": "boom"})
        if req.method == "POST" and p == "catalog/service-items/5/place-request":
            if self.place_status != 200:
                return httpx.Response(self.place_status, json={"description": "place boom"})
            return httpx.Response(200, json={"service_request": {"id": 91, "display_id": 91}})
        return httpx.Response(404, json={"description": "not found"})

    def client(self):
        c = client_mod.Client(cookies={"_x_w": "w", "fw-session-id": "fw1"})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(self), follow_redirects=False)
        c._csrf = "tok"
        return c

    def writes(self):
        return [[r.method, r.url.path, r.content.decode() if r.content else None] for r in self.requests
                if r.method in ("POST", "PUT")]


JANE = LookupValue(9, "Jane Doe", "jane@example.com")


def run(fn, answer=None):
    """-> stderr text, confirm questions, exit code, exit message, result."""
    asked: list[str] = []

    def confirm(text, default=False, **_):
        asked.append(text)
        return answer

    real = typer.confirm
    typer.confirm = confirm
    buf = io.StringIO()
    code, msg, result = 0, None, None
    try:
        with redirect_stderr(buf):
            result = fn()
    except change_cli.FlowExit as e:
        code, msg = e.code, e.message if hasattr(e, "message") else str(e)
    finally:
        typer.confirm = real
    return {"stderr": buf.getvalue(), "asked": asked, "code": code, "message": msg, "result": result}


def reset_journal():
    import shutil
    shutil.rmtree(os.environ["XDG_STATE_HOME"], ignore_errors=True)
    shutil.rmtree(os.environ["FSV_CACHE_DIR"], ignore_errors=True)


def new_change(t, attach=None):
    c = t.client()
    s = change_session(c, requester=JANE)
    s.set("subject", "Deploy é")
    s.set("cf_note", "hi")
    if attach:
        s.draft.attachments = [attach]
    return c, s, s.build_plan()


def change_send(c, s, plan, tty, yes=False, expect=None):
    return lambda: change_cli._send(plan, c, rebuild=lambda m: s.build_plan(s.ack, m), tty=tty, yes=yes,
                                    expect_plan=expect, allow_duplicate=False, attach_flow="after-create")


def session_send(s, plan):
    def go():
        try:
            return s.send(plan)
        except change_cli.FlowExit:
            raise
        except Exception as e:  # noqa: BLE001
            return {"error": str(e), "detail": actions.failure_detail(e), "code": exit_code_for(e)}
    return go


out = {}


def scenario(name, phase, fn_of, answer=None, attach=None):
    reset_journal()
    t = Tenant()
    c, s, plan = new_change(t, attach)
    t.phase = phase
    n = len(t.writes())
    r = run(fn_of(c, s, plan), answer)
    r["writes"] = t.writes()[n:]
    r["old_hash"] = plan.hash
    out[name] = r


scenario("change_session_send", "promoted", lambda c, s, p: session_send(s, p))
scenario("change_cli_no_tty", "promoted", lambda c, s, p: change_send(c, s, p, tty=False))
scenario("change_cli_yes", "promoted", lambda c, s, p: change_send(c, s, p, tty=True, yes=True))
scenario("change_cli_confirm_yes", "promoted", lambda c, s, p: change_send(c, s, p, tty=True), answer=True)
scenario("change_cli_confirm_no", "promoted", lambda c, s, p: change_send(c, s, p, tty=True), answer=False)
scenario("change_cli_incomplete", "required", lambda c, s, p: change_send(c, s, p, tty=True), answer=True)

attach = os.path.join(ROOT, "a.txt")
with open(attach, "w") as f:
    f.write("hello")


def partial(c, s, p):
    def go():
        try:
            return change_send(c, s, p, tty=False)()
        except change_cli.PartialFailure as e:
            raise change_cli._report_partial(e, "change") from None
    return go


scenario("change_partial_cli", "review", partial, attach=attach)
scenario("change_partial_session", "review", lambda c, s, p: session_send(s, p), attach=attach)


def sr_scenario(name, fn_of, answer=None):
    reset_journal()
    t = Tenant()
    c = t.client()
    s = sr_session(c, "5", requester=JANE)
    s.set("reason", "broken é")
    plan = s.build_plan()
    t.phase = "item"
    n = len(t.writes())
    r = run(fn_of(c, s, plan), answer)
    r["writes"] = t.writes()[n:]
    r["old_hash"] = plan.hash
    out[name] = r


def sr_send(c, s, plan, tty):
    return lambda: cli_sr._send(plan, c, rebuild=lambda m: s.build_plan(s.ack, m), tty=tty, yes=False,
                                expect_plan=None, allow_duplicate=False)


sr_scenario("sr_session_send", lambda c, s, p: session_send(s, p))
sr_scenario("sr_cli_no_tty", lambda c, s, p: sr_send(c, s, p, tty=False))
sr_scenario("sr_cli_confirm_yes", lambda c, s, p: sr_send(c, s, p, tty=True), answer=True)


def sr_failure(name, place, upload=200, files=False, session=False):
    """An unchanged SR plan whose upload or place-request fails."""
    reset_journal()
    t = Tenant()
    t.place_status, t.upload_status, t.files = place, upload, files
    c = t.client()
    s = sr_session(c, "5", requester=JANE)
    s.set("reason", "broken é")
    if files:
        s.draft.attachments = [attach]
    plan = s.build_plan()
    n = len(t.writes())
    r = run(session_send(s, plan) if session else sr_send(c, s, plan, tty=False))
    r["writes"] = [w[:2] for w in t.writes()[n:]]
    r["hash"] = plan.hash
    out[name] = r


sr_failure("sr_place_500_cli", 500)
sr_failure("sr_place_422_files_cli", 422, files=True)
sr_failure("sr_upload_500_cli", 200, upload=500, files=True)
sr_failure("sr_place_500_files_session", 500, files=True, session=True)

json.dump(out, sys.stdout, indent=1, ensure_ascii=False, default=str)
print()
