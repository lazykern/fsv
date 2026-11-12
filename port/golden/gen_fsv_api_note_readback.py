"""Golden `ticket_notes.send` normalized readback (Python fdbb3e9) for fsv-api
`notes::send`: server-normalized body, `to_emails` recipient fallback.

    cd /tmp/fsv-py-fdbb3e9 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=fdbb3e9... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_note_readback.py
"""
from __future__ import annotations

import json
import os
import sys
import tempfile

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import config, ticket_notes as notes  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

DOMAIN = "acme.freshservice.com"
config.DOMAIN = DOMAIN
config.API_INT = f"https://{DOMAIN}/api/_"

CASES = {
    "normalized_body_empty_to_emails": ('<div class="server"><div>Acme &amp; Globex</div></div>', []),
    "wrong_body": ("<div>Acme &amp; Wrong</div>", []),
    "no_recipients": ("<div>Acme &amp; Globex</div>", None),
    "to_emails_mismatch": ("<div>Acme &amp; Globex</div>", ["x@example.com"]),
    "notify_emails_preferred": ("<div>Acme &amp; Globex</div>", "both"),
    "body_not_string": (None, []),
}


def client(handler):
    c = Client.__new__(Client)
    c._client = httpx.Client(transport=httpx.MockTransport(handler), follow_redirects=False)
    c._csrf = "tok"
    c._fw_domain = c._fw_session_id = None
    c._persist = False
    c._rl_rem = c._rl_tot = None
    c._probe_ok_until = 0
    c._probe_session = lambda: True
    return c


out = {}
for name, (body, recipients) in CASES.items():
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    plan = notes.build_plan(123, "Acme & Globex")
    seen = []

    def handler(req, body=body, recipients=recipients):
        seen.append({"method": req.method, "path": req.url.path,
                     "body": json.loads(req.content) if req.content else None})
        if req.method == "POST":
            return httpx.Response(201, json={"note": {"id": 99001}})
        row = {"id": 99001, "body": body, "private": True}
        if recipients == "both":
            row["notify_emails"] = []
            row["to_emails"] = ["x@example.com"]
        elif recipients is not None:
            row["to_emails"] = recipients
        return httpx.Response(200, json={"conversations": [row], "meta": {"count": 1}})

    c = client(handler)
    result = notes.send(plan, c, reviewed_hash=plan.hash)
    c.close()
    out[name] = {"row_body": body, "recipients": recipients, "plan_hash": plan.hash,
                 "requests": seen, "result": result}
json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False)
