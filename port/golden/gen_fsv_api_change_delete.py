"""Golden `change_delete` for fsv-api `change_delete`: the bodyless soft-delete
DELETE, its review, bounded readback sleeps, refusals and failures. Each case
is replayed by the Rust test over the same one-row world.

    cd /tmp/fsv-py-<sha> && FSV_PY_SRC=$PWD FSV_PY_COMMIT=<sha> python \
        port/golden/gen_fsv_api_change_delete.py
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

from fsv import change_delete as flow, config, readback  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

ROW = {"id": 123, "subject": "Acme rollout", "deleted": False}


def client(row, lag, fail, writes):
    s = {"row": copy.deepcopy(row), "lag": lag, "wrote": False, "reads_after": 0}

    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            out = dict(s["row"])
            if s["wrote"]:
                s["reads_after"] += 1
                if s["lag"] > 0:
                    s["lag"] -= 1
                    out["deleted"] = False
            return httpx.Response(200, json={"change": out})
        writes.append({"method": req.method, "path": path, "body": req.content.decode()})
        if fail == "disconnect":
            raise httpx.ReadError("disconnect", request=req)
        if fail:
            return httpx.Response(fail, json={"errors": "synthetic"})
        s["row"]["deleted"] = True
        s["wrote"] = True
        return httpx.Response(204)

    c = Client.__new__(Client)
    c._client = httpx.Client(transport=httpx.MockTransport(handle), follow_redirects=False)
    c._csrf = "synthetic"
    c._fw_domain = c._fw_session_id = None
    c._persist = False
    c._rl_rem = c._rl_tot = None
    c._probe_ok_until = 0
    c._probe_session = lambda: True
    return c, s


def run(name, row=ROW, lag=0, fail=None):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    config.DOMAIN = "acme.freshservice.com"
    config.API_INT = "https://acme.freshservice.com/api/_"
    sleeps = []
    readback._sleep = sleeps.append
    writes = []
    c, s = client(row, lag, fail, writes)
    case = {"name": name}
    try:
        plan = flow.build_plan(c, "CHN-123")
    except Exception as exc:
        case["build_err"] = str(exc)
        case["writes"] = len(writes)
        return case
    shown = dict(hash=plan.hash, review=flow.review(plan, c), payload_json=plan.payload_json,
                 context=plan.context_hash)
    if name == "no_confirm":
        case.update(shown)
        try:
            flow.send(plan, c, reviewed_hash=plan.hash)
        except Exception as exc:
            case["send_err"] = str(exc)
        case["writes"] = len(writes)
        return case
    if name == "drift":
        case["hash"] = plan.hash
        s["row"]["subject"] = "Globex changed"
        try:
            flow.send(plan, c, reviewed_hash=plan.hash, confirm_destructive=True)
        except Exception as exc:
            case["err"] = str(exc)
        case["writes"] = len(writes)
        return case
    if fail is None:
        case.update(shown)
        case["result"] = flow.send(plan, c, reviewed_hash=plan.hash, confirm_destructive=True)
        case["sleeps"] = sleeps
        case["reads_after"] = s["reads_after"]
        replay = True
    else:
        try:
            flow.send(plan, c, reviewed_hash=plan.hash, confirm_destructive=True)
        except Exception as exc:
            case["err"] = str(exc)
        replay = name != "422"
    if replay:
        try:
            flow.send(plan, c, reviewed_hash=plan.hash, confirm_destructive=True)
        except Exception as exc:
            case["replay"] = str(exc)
    case["writes"] = writes if fail is None else (len(writes) if replay else len(writes))
    return case


CASES = [("ok", {}), ("lag2", {"lag": 2}), ("lag100", {"lag": 100}), ("no_confirm", {}),
         ("deleted_true", {"row": {**ROW, "deleted": True}}),
         ("deleted_missing", {"row": {"id": 123, "subject": "Acme rollout"}}),
         ("deleted_str", {"row": {**ROW, "deleted": "false"}}),
         ("subject_null", {"row": {**ROW, "subject": None}}), ("drift", {}),
         ("403", {"fail": 403}), ("429", {"fail": 429}), ("422", {"fail": 422}),
         ("disconnect", {"fail": "disconnect"})]
json.dump(stamped({"cases": [run(n, **kw) for n, kw in CASES]}), sys.stdout, indent=1, ensure_ascii=False)
