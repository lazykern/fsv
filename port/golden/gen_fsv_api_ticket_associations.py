"""Golden `ticket_associations` for fsv-api `ticket_associations`: plans, sends,
listings, candidates, refusals and the exact writes, over the fake tenant of
`tests/test_ticket_associations.py`. The Rust test replays the same sequence.

    cd /tmp/fsv-py-<sha> && FSV_PY_SRC=$PWD FSV_PY_COMMIT=<sha> python \
        port/golden/gen_fsv_api_ticket_associations.py
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
os.environ["XDG_STATE_HOME"] = os.path.join(BOOT, "state")

import httpx  # noqa: E402

from fsv import config, ticket_associations as links  # noqa: E402
from fsv.client import Client  # noqa: E402
from fsv.reviewed_write import ReviewedWriteError  # noqa: E402
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.example.com"
config.API_INT = "https://acme.example.com/api/_"

s = {"problems": set(), "changes": {"change": set(), "change_cause": set()}, "children": set(),
     "writes": [], "fail": None, "effect": True,
     "tickets": {41: "Incident", 42: "Incident", 43: "Service Request", 44: "Incident"}}


def level(t):
    if t == 41 and s["children"]:
        return 2
    return 1 if t in s["children"] else 0


def handler(req):
    path = req.url.path.removeprefix("/api/_/")
    parts = path.split("/")
    q = dict(req.url.params)
    if req.method == "GET":
        if parts[0] == "tickets" and len(parts) == 2 and int(parts[1]) in s["tickets"]:
            t = int(parts[1])
            return httpx.Response(200, json={"ticket": {"id": t, "type": s["tickets"][t], "subject": f"Acme {t}",
                                                        "human_display_id": f"{'SR' if t == 43 else 'INC'}-{t}"}})
        if path == "problems/7":
            return httpx.Response(200, json={"problem": {"id": 7, "subject": "Acme outage"}})
        if path == "changes/9":
            return httpx.Response(200, json={"change": {"id": 9, "subject": "Globex patch"}})
        if path == "tickets/41/problems" and "filter" not in q:
            return httpx.Response(200, json={"problems": [{"id": p, "human_display_id": f"PRB-{p}"} for p in sorted(s["problems"])],
                                             "meta": {"has_next": False}, "requesters": []})
        if path == "tickets/41/changes":
            if q.get("change_type") not in ("change", "change_cause"):
                return httpx.Response(400, json={"errors": "not a valid change_type"})
            if q.get("filter") == "list":
                return httpx.Response(200, json={"changes": [{"id": 9, "subject": "Globex patch"}], "meta": {"has_next": False}})
            return httpx.Response(200, json={"changes": [{"id": c} for c in sorted(s["changes"][q["change_type"]])],
                                             "meta": {"has_next": False}})
        if parts[-1] == "tabs":
            t = int(parts[1])
            return httpx.Response(200, json=[{"name": "related_tickets", "count": 0, "hierarchy": level(t)},
                                             {"name": "associations", "count": 0}])
        if path == "tickets/41/child-tickets":
            if level(41) != 2:
                return httpx.Response(400, json={"errors": {"parent_ticket": "not_a_major_ticket"}})
            return httpx.Response(200, json={"child_tickets": [{"id": c, "deleted": False, "spam": False}
                                                               for c in sorted(s["children"])], "meta": {"has_next": False}})
        if path == "tickets/41/suggest_child_tickets":
            return httpx.Response(200, json={"tickets": [{"id": 42, "subject": "Acme printer"},
                                                         {"id": 44, "subject": "Globex laptop"}]})
        if path == "problems/search":
            return httpx.Response(200, json={"problems": [{"id": 7, "subject": "Acme outage", "status": 1}]})
        if path in {"problems/99", "problems/77"}:
            return httpx.Response(404, json={})
        raise AssertionError(path)
    body = json.loads(req.content) if req.content else None
    s["writes"].append([req.method, path, body, q])
    if s["fail"]:
        return httpx.Response(s["fail"], json={"errors": "synthetic"})
    if s["effect"]:
        if parts[2] == "problems":
            (s["problems"].add if body["type"] == "link" else s["problems"].discard)(int(parts[3]))
        elif parts[2] == "changes":
            if parts[3] == "associate":
                s["changes"][body["change_type"]].update(body["item_ids"])
            else:
                s["changes"][body["change_type"]].discard(int(parts[3]))
        elif parts[2] == "associate-child-tickets":
            s["children"].update(body["ticket_ids"])
            return httpx.Response(200, json={"success": True})
        elif parts[2] == "remove_child":
            s["children"].discard(int(q["ticket_id"]))
            return httpx.Response(204)
    return httpx.Response(200, json={})


c = Client.__new__(Client)
c._client = httpx.Client(transport=httpx.MockTransport(handler), follow_redirects=False)
c._csrf = "synthetic-token"
c._fw_domain = c._fw_session_id = None
c._persist = False
c._rl_rem = c._rl_tot = None
c._probe_ok_until = 0
c._probe_session = lambda: True

out = {}


def plan(name, *a, **k):
    p = links.build_plan(c, *a, **k)
    out[name] = {"review": p.review(), "hash": p.hash, "context": p.context_hash}
    return p


def sent(name, p):
    out[name + "_sent"] = links.send(p, c, reviewed_hash=p.hash)


out["list_problem_empty"] = links.listing(c, "INC-41", "problem")
p = plan("problem_link", "INC-41", "problem", "PRB-7")
sent("problem_link", p)
out["list_problem"] = links.listing(c, "INC-41", "problem")
u = plan("problem_unlink", "INC-41", "problem", "7", unlink=True)
sent("problem_unlink", u)
for kind in ["initiating", "initiated-by"]:
    p = plan(f"change_link_{kind}", "41", "change", "CHN-9", ctype=kind)
    sent(f"change_link_{kind}", p)
    out[f"list_change_{kind}"] = links.listing(c, "41", "change", kind)
    u = plan(f"change_unlink_{kind}", "41", "change", "9", unlink=True, ctype=kind)
    sent(f"change_unlink_{kind}", u)
out["list_child_empty"] = links.listing(c, "41", "child")
p = plan("child_link", "41", "child", "INC-42")
sent("child_link", p)
out["list_child"] = links.listing(c, "41", "child")
u = plan("child_unlink", "41", "child", "42", unlink=True)
sent("child_unlink", u)
out["cand_child_search"] = links.candidates(c, "INC-41", "child", "printer")
out["cand_child"] = links.candidates(c, "INC-41", "child")
out["cand_problem_search"] = links.candidates(c, "INC-41", "problem", "acme")
out["cand_change"] = links.candidates(c, "INC-41", "change", None, "initiating")
errors = {}
for name, args, kw in [
        ("sr_problem", ("SR-43", "problem", "7"), {}),
        ("missing_problem", ("INC-41", "problem", "99"), {}),
        ("self_child", ("41", "child", "41"), {}),
        ("bad_ctype", ("41", "change", "9"), {"ctype": "x"}),
        ("problem_not_linked", ("41", "problem", "7"), {"unlink": True}),
        ("change_not_linked", ("41", "change", "9"), {"unlink": True, "ctype": "initiating"}),
        ("child_not_linked", ("41", "child", "42"), {"unlink": True})]:
    try:
        links.build_plan(c, *args, **kw)
    except (ValueError, ReviewedWriteError) as exc:
        errors[name] = [type(exc).__name__, str(exc)]
out["errors"] = errors
out["writes"] = s["writes"]
json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False)
