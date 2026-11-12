"""Golden `ticket_lifecycle` (Python 9ab0eaa) for fsv-api `ticket_lifecycle`:
reviewed resolve/close/reopen plans, hashes, reviews, PUT bodies, journal
events and refusals. Each case is data (row, metadata variant, fault flags,
ops) that the Rust test replays over `FnApi`; metadata loads through the real
`load_meta` from the `ticket_form_fields` / business-rules routes.

    cd /tmp/fsv-py-9ab0eaa && FSV_PY_SRC=$PWD FSV_PY_COMMIT=9ab0eaa... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_ticket_lifecycle.py

`--ack` (Python d699b8c or later) writes `ticket_lifecycle_ack_oracle.json`:
acknowledged blocking rule messages, workflow rewrites on send, and the
pure `readback` cases.
"""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import config, formmeta, journal, ticket_lifecycle as flow  # noqa: E402
from fsv.errors import APIError  # noqa: E402
from fsv.readback import resolution_text  # noqa: E402
from provenance import stamped  # noqa: E402

formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: None
DOMAIN = "acme.freshservice.com"


def form_field(id_, name, label, field_type, position, **extra):
    """Mirror of fsv-api `service::testing::form_field`."""
    default = field_type.startswith("default_")
    f = {"id": id_, "name": name, "label": label, "description": "", "field_type": field_type,
         "required": False, "required_for_closure": False, "position": position,
         "default_field": default, "choices": [], "nested_fields": [],
         "placeholder": "", "helptext": "", "hint": False,
         "content": None, "displayed_to_agents": True, "data_source": 0,
         "lookup_config": None, "workspace_id": 1 if default else 2,
         "updated_at": "2030-01-02T03:04:05Z"}
    f.update(extra)
    return f


def fields():
    out = [form_field(9000007101, "status", "Status", "default_status", 1, choices=[
        {"id": 31, "value": "Open"}, {"id": 47, "value": "Pending"},
        {"id": 62, "value": "Resolved"}, {"id": 89, "value": "Closed"}]),
        form_field(9000007102, "summary", "Resolution Summary", "custom_text", 2, required_for_closure=True),
        form_field(9000007103, "reason", "Pending Reason", "custom_text", 3)]
    return out


def rule(rid, name, cond, actions):
    return {"id": rid, "name": name, "workspace_id": 2, "module_type": "ticket", "item_id": None,
            "deleted": False, "user_type": "both", "form_type": "both", "state": "active",
            "rule_type": "client_side", "description": "", "condition_data": cond,
            "action_data": {"action": actions}, "is_reversible": False, "is_validation": False,
            "updated_at": "2030-01-03T00:00:00Z"}


PENDING = rule(9000007107, "Pending needs reason",
               {"all": [{"name": "status", "operator": "is", "value": "47", "evaluate_on": "ticket"}]},
               [{"name": ["customFields.reason"], "property": "mandate"}])
HIDE = rule(9000007108, "Hide summary", {"all": None}, [{"name": ["customFields.summary"], "property": "hide"}])


def variant(name):
    """`{"fields": [...], "rules": {...} | None}`; None rules: the route 404s."""
    fs = fields()
    rules = {"business_rules": []}
    if name == "label_changed":
        fs[1]["label"] = "Changed requirement"
    elif name == "pending_rule":
        rules = {"business_rules": [PENDING]}
    elif name == "pending_hide":
        rules = {"business_rules": [PENDING, HIDE]}
        fs[2]["lookup_config"] = {"link": "/api/_/lookup?field_name=reason_900000777"}
    elif name == "unknown_required":
        # Every raw entry parses into the spec, so `context`'s unknown-field
        # branch is unreachable through `load_meta`; this pins that behavior.
        fs.append({"name": "unknown_requirement", "required_for_closure": True})
    elif name == "rules_unavailable":
        rules = None
    elif name == "blocking_close":
        # A tenant is_validation rule: a message the web form blocks submit on.
        blocking = rule(9000007120, "Only requester closes",
                        {"all": [{"name": "status", "operator": "is", "value": "89", "evaluate_on": "ticket"}]},
                        [{"name": ["Only the requester closes Acme tickets"], "property": "message"}])
        blocking["is_validation"] = True
        rules = {"business_rules": [blocking]}
    elif name == "string_ids":
        for c in fs[0]["choices"]:
            c["id"] = str(c["id"])
    elif name.startswith("bad_id_"):
        fs[0]["choices"][1]["id"] = {"bad_id_sign": "-47", "bad_id_alpha": "47a", "bad_id_bool": True,
                                     "bad_id_zero": "0", "bad_id_float": 47.0, "bad_id_space": " 47",
                                     "bad_id_arabic": "\u0664\u0667"}[name]
    return {"fields": fs, "rules": rules}


def base_row():
    return {"id": 123, "status": 31, "group_id": None, "responder_id": 9000001234,
            "resolution_notes": None, "custom_fields": {"summary": "Acme fix", "reason": "", "untouched": "keep"}}


class Fake:
    """Python test `Client` plus metadata routes; PUTs mutate the row."""

    def __init__(self, row, meta, flags):
        self.row = row
        self.meta = meta
        self.flags = flags
        self.writes = []
        self.gets = []

    def int_get(self, path, params=None):
        self.gets.append(path)
        if path == "tickets/123":
            return {"ticket": copy.deepcopy(self.row)}
        if path == "ticket_form_fields":
            return {"ticket_fields": copy.deepcopy(self.meta["fields"])}
        if path.endswith("business_rules") and self.meta["rules"] is not None:
            return copy.deepcopy(self.meta["rules"])
        raise APIError(404, "missing", "GET", path)

    def me(self):
        return {"id": 901}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kwargs):
        assert method == "PUT" and url == config.API_INT + "/tickets/123", url
        body = json.loads(kwargs["content"])
        self.writes.append({"raw": kwargs["content"].decode(), "body": body})
        put_error = self.flags.get("put_error")
        if put_error == "disconnect":
            raise httpx.ReadTimeout("offline disconnect")
        if put_error is not None:
            raise APIError(put_error, "synthetic failure", "PUT", url)
        if len(self.writes) == self.flags.get("fail_at"):
            raise APIError(403, "synthetic rejection", "PUT", url)
        for key, value in body.items():
            if key == "resolution_notes":
                self.row[key] = {"notes_html": "wrong" if self.flags.get("bad_note") else value,
                                 "user": {"name": "Acme Operator", "email": "operator@example.com"}}
                if self.flags.get("change_after_note"):
                    self.row["group_id"] = 9000005678
            elif key == "custom_fields":
                self.row[key].update(value)
            elif key == "status" and "rewrite_status" in self.flags:
                self.row[key] = self.flags["rewrite_status"]
            elif key == "group_id" and "rewrite_group" in self.flags:
                self.row[key] = self.flags["rewrite_group"]
            elif key != "br_validation_excludes" and not (key == "status" and self.flags.get("bad_status")):
                self.row[key] = value
        return {"ticket": copy.deepcopy(self.row)}


def error(exc):
    return {"error": {"type": type(exc).__name__, "msg": str(exc)}}


def plan_out(p):
    return {"plan": dict(p.__dict__), "hash": p.hash, "review": p.review()}


def run(case):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    config.DOMAIN = case.get("tenant", DOMAIN)
    config.API_INT = f"https://{config.DOMAIN}/api/_"
    row = base_row()
    for k, v in case.get("row", {}).items():
        row[k] = v
    row["custom_fields"].update(case.get("custom", {}))
    meta = variant(case.get("meta", "base"))
    c = Fake(row, meta, case.get("flags", {}))
    real_record = journal.record_plan
    plan = None
    outputs = []
    for op in case["ops"]:
        kind = op["op"]
        try:
            if kind == "build":
                kwargs = {"ack": True} if op.get("ack") else {}
                plan = flow.build_plan(c, op.get("target", 123), op["status"], note=op.get("note"),
                                       answers=op.get("answers"), **kwargs)
                out = plan_out(plan)
            elif kind == "send":
                reviewed = {"full": plan.hash, "none": None, "prefix": plan.hash[:12]}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed)}
            elif kind == "row":
                for k, v in op.get("set", {}).items():
                    c.row[k] = v
                for k in op.get("pop", []):
                    c.row.pop(k, None)
                out = {}
            elif kind == "meta":
                c.meta = variant(op["variant"])
                out = {}
            elif kind == "deaf":
                journal.record_plan = lambda *a, **k: None
                out = {}
            else:
                raise AssertionError(kind)
        except Exception as exc:  # noqa: BLE001
            out = error(exc)
        out["writes"] = len(c.writes)
        outputs.append(out)
    journal.record_plan = real_record
    events = [r["event"] for r in journal.read() if r.get("kind") == "plan:ticket_transition"]
    return {**case, "outputs": outputs, "puts": c.writes, "final_row": c.row, "events": events}


B = {"op": "build"}
S = {"op": "send"}
def churn(n):
    """A re-read that only re-signs links and bumps server timestamps (8f156c0)."""
    return {"updated_at": f"2030-01-0{n}T00:00:00Z",
            "attachments": [{"id": 1, "name": "acme.txt", "size": 3, "updated_at": str(n),
                             "attachment_url": f"https://files.example.com/a?Expires={n}&Signature=s{n}"}],
            "description": f'<a href="https://files.example.com/b?v=1&amp;X-Amz-Signature=s{n}">b</a>'}


CASES = {}
for tenant, prefix in [(DOMAIN, "INC"), ("globex.example.com", "SR")]:
    CASES[f"resolve_then_transition_{prefix}"] = {"tenant": tenant, "ops": [
        {**B, "target": f"{prefix}-123", "status": "Resolved", "note": "Acme fix <safe>",
         "answers": {"summary": "Globex fix"}}, S, S]}
for status in ["Closed", "Open", "Pending"]:
    CASES[f"single_{status}"] = {"row": {"resolution_notes": {"notes_html": "<p>Earlier fix</p>"}}, "ops": [
        {**B, "status": status, "answers": {"reason": "Waiting for self"} if status == "Pending" else {}}, S]}
for label in ["Resolved", "Closed"]:
    CASES[f"required_closure_{label}"] = {"custom": {"summary": ""}, "ops": [
        {**B, "status": label, "note": "fix" if label == "Resolved" else None},
        {"op": "row", "set": {"custom_fields": {"summary": "fix", "reason": "", "untouched": "keep"}}},
        {**B, "status": label}]}
for i, value in enumerate([None, {"notes_html": None}, {"unexpected": "fix"}, ""]):
    CASES[f"missing_resolution_{i}"] = {"row": {"resolution_notes": value}, "ops": [{**B, "status": "Closed"}]}
CASES["resolution_alias_html"] = {"row": {"resolution_notes_html": "<p>alias</p>"}, "ops": [{**B, "status": "Closed"}, S]}
for bad in ["bad_note", "change_after_note", "bad_status", "fail_first", "fail_second"]:
    flags = {"fail_at": 1 if bad == "fail_first" else 2} if bad.startswith("fail") else {bad: True}
    CASES[f"partial_{bad}"] = {"flags": flags, "ops": [{**B, "status": "Resolved", "note": "fix"}, S, S]}
CASES["stale_and_consent"] = {"ops": [
    {**B, "status": "Open"}, {**S, "hash": "none"}, {**S, "hash": "prefix"},
    {"op": "row", "set": {"subject": "changed"}}, S, {"op": "row", "pop": ["subject"]},
    {"op": "meta", "variant": "label_changed"}, S, {"op": "meta", "variant": "base"},
    {"op": "deaf"}, S]}
for failure in [403, 429, 500, "disconnect"]:
    CASES[f"transport_{failure}"] = {"flags": {"put_error": failure}, "ops": [{**B, "status": "Open"}, S, S]}
CASES["pending_reason_rule"] = {"meta": "pending_rule", "ops": [
    {**B, "status": "Pending"}, {**B, "status": "Pending", "answers": {"reason": "Waiting"}}]}
CASES["hidden_closure_requirement"] = {"meta": "pending_hide", "custom": {"summary": None}, "ops": [
    {**B, "status": "Resolved", "note": "fix"}, {**B, "status": "Resolved", "note": "fix", "answers": {"summary": "x"}}]}
CASES["closure_required_extra_field_and_unavailable"] = {"meta": "unknown_required", "ops": [
    {**B, "status": "Resolved", "note": "fix"}, {"op": "meta", "variant": "rules_unavailable"}, {**B, "status": "Open"}]}
CASES["argument_errors"] = {"ops": [
    {**B, "target": "0", "status": "Open"}, {**B, "status": "Closed", "note": "fix"},
    {**B, "status": "Bogus"}, {**B, "status": "open"}, {**B, "status": "Open", "answers": {"status": "Closed"}},
    {**B, "status": "Open", "answers": {"reason": "a", "Pending Reason": "b"}},
    {**B, "status": "Open", "answers": {"nonexistent": "x"}}]}
CASES["read_churn"] = {"ops": [{"op": "row", "set": churn(1)}, {**B, "status": "Open"},
                               {"op": "row", "set": churn(2)}, S]}
CASES["string_status_ids"] = {"meta": "string_ids", "ops": [
    {**B, "status": "Pending", "answers": {"reason": "Waiting for self"}}, S]}
for bad in ["sign", "alpha", "bool", "zero", "float", "space", "arabic"]:
    CASES[f"bad_status_id_{bad}"] = {"meta": f"bad_id_{bad}", "ops": [
        {**B, "status": "Pending", "answers": {"reason": "Waiting for self"}}]}
CASES["routing_unavailable"] = {"ops": [{"op": "row", "pop": ["group_id"]}, {**B, "status": "Open"}]}

MATCHES = [
    ({"status": 31, "group_id": None}, {"status": 31, "group_id": None, "br_validation_excludes": "x"}),
    ({"status": True}, {"status": 1}),
    ({"status": 1.0}, {"status": 1}),
    ({}, {"status": 1}),
    ({"custom_fields": {"a": "1", "b": 2}}, {"custom_fields": {"a": 1}}),
    ({"custom_fields": None}, {"custom_fields": {"a": 1}}),
]
RESOLUTION = [
    {"resolution_notes": {"notes_html": "<p>Acme fix</p>", "user": {"name": "Operator"}}},
    {"resolution_notes": "Globex fix"},
    {"resolution_notes": {"unexpected": "not note text"}},
    {"resolution_notes": None, "resolution_notes_html": "<p>alias</p>"},
    {"resolution_notes": {"notes_html": 5}, "resolution_notes_html": "<p>alias</p>"},
    {"resolution_notes_html": 7},
    {},
]
ACK_CASES = {
    "blocking_message_needs_ack": {"meta": "blocking_close",
                                   "row": {"resolution_notes": {"notes_html": "Existing fix"}},
                                   "ops": [{**B, "status": "Closed"}, {**B, "status": "Closed", "ack": True}, S]},
    "ack_without_blocking_message": {"ops": [{**B, "status": "Pending", "answers": {"reason": "Waiting"},
                                              "ack": True}, S]},
    "workflow_status_rewrite": {"flags": {"rewrite_status": 62}, "ops": [
        {**B, "status": "Pending", "answers": {"reason": "Waiting"}}, S]},
    "workflow_routing_rewrite": {"flags": {"rewrite_group": 9000005678}, "ops": [{**B, "status": "Open"}, S]},
}
NOTE = "<div>Fixed by restarting the Acme service</div>"
PAYLOAD = {"status": 4, "group_id": 9000000201, "responder_id": None, "br_validation_excludes": "x"}


def _row(status, **kw):
    row = {"status": status, "group_id": 9000000201, "responder_id": None, "resolution_notes": NOTE,
           "custom_fields": {}}
    row.update(kw)
    return row


READBACK = [
    (_row(2), _row(4), PAYLOAD, NOTE),
    (_row(2), _row(4, group_id=9000000202), PAYLOAD, NOTE),
    (_row(2), _row(5), PAYLOAD, NOTE),
    (_row(2), _row(2), PAYLOAD, NOTE),
    (_row(2), _row(4, resolution_notes=""), PAYLOAD, NOTE),
    (None, _row(5), PAYLOAD, NOTE),
    (_row(2), _row(None), PAYLOAD, None),
    (_row(2), _row(True), PAYLOAD, None),
    (_row("2"), _row("4"), PAYLOAD, None),
    (_row(2), _row(4, custom_fields={"a": "1"}), {**PAYLOAD, "custom_fields": {"a": 1}}, None),
    (_row(2), _row(4, custom_fields={"a": "2"}), {**PAYLOAD, "custom_fields": {"a": 1}}, None),
]
if "--ack" in sys.argv:
    out = {"cases": {name: run(case) for name, case in ACK_CASES.items()},
           "readback": [{"before": b, "row": r, "payload": p, "note_html": n, "expected": list(flow.readback(b, r, p, n))}
                        for b, r, p, n in READBACK]}
    json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False)
    sys.exit(0)
out = {"cases": {name: run(case) for name, case in CASES.items()},
       "matches": [{"row": r, "payload": p, "expected": flow.matches(r, p)} for r, p in MATCHES],
       "resolution_text": [{"record": r, "expected": resolution_text(r)} for r in RESOLUTION]}
json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False)
