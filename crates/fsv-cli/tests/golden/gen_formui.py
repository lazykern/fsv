#!/usr/bin/env python3
"""Expected values for the form UI differential (`fsv-cli/tests/formui_diff.rs`).

    cd <python repo> && uv run python <this> [--check]
    FSV_PY_SRC=<git archive>/src FSV_PY_TESTS=<git archive>/tests FSV_PY_COMMIT=<sha> <venv python> <this>

Runs the Python form UI logic (`fuzzy`, `when`, file checks, `FormSession`)
in-process and writes `tests/golden/formui.json`. Session scenarios run over
the invented Acme and Globex tenants of the Python tests
(`formui_tenants.py`); every request they make is recorded into
`tests/fixtures/routes/formui_*.json`, so the Rust side answers from the same
bytes and must make the same requests.
"""
import datetime as dt
import json
import os
import pathlib
import shlex
import sys
import tempfile
from zoneinfo import ZoneInfo

GOLDEN = pathlib.Path(__file__).resolve().parent
ROUTES = GOLDEN.parent / "fixtures" / "routes"
OUT = GOLDEN / "formui.json"
sys.path.insert(0, str(GOLDEN))
if os.environ.get("FSV_PY_SRC"):
    sys.path.insert(0, os.environ["FSV_PY_SRC"])
PY_TESTS = pathlib.Path(os.environ.get("FSV_PY_TESTS") or pathlib.Path.cwd() / "tests")
sys.path.insert(0, str(PY_TESTS))

from pyrev import python_rev  # noqa: E402

ROOT = pathlib.Path(tempfile.mkdtemp()).resolve()
HOME = ROOT / "home"
CFG = HOME / "config"
CFG.mkdir(parents=True)
for k, v in {"HOME": HOME, "FSV_CONFIG_DIR": CFG, "FSV_CACHE_DIR": HOME / "cache", "XDG_STATE_HOME": HOME / "state",
             "FSV_NO_KEYCHAIN": "1", "NO_COLOR": "1", "COLUMNS": "80", "TZ": "UTC"}.items():
    os.environ[k] = str(v)
for k in ("FSV_TZ", "SHELL", "FSV_MAX_FILE_MB"):
    os.environ.pop(k, None)

from fsv.formui import session as S  # noqa: E402
from fsv.formui.fuzzy import fold, fuzzy_filter, score  # noqa: E402
from fsv.formui.when import add_months, month_weeks, offset_text, parse_when, preview, to_text  # noqa: E402
from fsv.formui.widgets import complete_path  # noqa: E402
from fsv.forms import LookupValue  # noqa: E402


# ---------------------------------------------------------------- pure logic
FUZZY_ITEMS = [
    ("Staging", ""), ("Live", "production traffic"), ("Pre-stage", ""), ("Mostly stable", ""), ("Sandbox", ""),
    ("Änderungstyp", ""), ("Priorität", "high is urgent"), ("Normal change", "standard"), ("Emergency", "fast"),
    ("Standard", "pre-approved"), ("Platform Ops", ""), ("Network  Team", "  spaced  "), ("Ñandú", ""),
    ("a-b_c.d", ""), ("CAFÉ", "Crème brûlée"), ("", "only a description"), ("Zeta", ""), ("zebra crossing", "z"),
    ("Release 2026-11", "q4"), ("Server (prod)", "eu-west"), ("straße", ""), ("İstanbul", ""), ("ǅ", ""),
]
FUZZY_QUERIES = ["", " ", "sta", "STA", "st", "s", "traffic", "sbx", "anderung", "PRIO", "norm", "std", "ops plat",
                 "plat ops", "network", "net team", "nandu", "abcd", "a-b", "cafe", "creme", "brulee", "zz", "z",
                 "2026", "11", "prod", "eu", "strasse", "straße", "istanbul", "xyz", "  sta  ", "pre", "approved",
                 "only", "dj", "ne", "e", "ä", "ǆ"]


def fuzzy():
    return {
        "items": FUZZY_ITEMS,
        "queries": FUZZY_QUERIES,
        "filter": {q: fuzzy_filter(q, FUZZY_ITEMS) for q in FUZZY_QUERIES},
        "score": {q: [score(q, label, desc) for label, desc in FUZZY_ITEMS] for q in FUZZY_QUERIES},
        "fold": {s: fold(s) for s in ["  ÄNDERUNG  Typ ", "ǅ", "İ", "ß", "ﬁ", "Ñandú", "á", "x\ty\nz", ""]},
    }


ZONES = ["Asia/Bangkok", "UTC", "Europe/Berlin", "America/New_York", "Pacific/Auckland", "Asia/Kolkata"]
TODAY = dt.date(2026, 11, 11)
WHEN_TEXTS = [
    "", "  ", "2026-11-26 19:00", "2026-11-26T19:00", "26 Nov 2026 19:00", "Nov 26 2026 7pm",
    "thu 26 nov 2026 7:00pm", "2026/11/26 19:00", "2026-11-26", "2026-02-30 10:00", "2026-11-26 25:00",
    "2026-11-26 19:00+09:00", "2026-11-26 19:00 +0900", "2026-11-26 19:00Z", "2026-11-26 19:00 utc",
    "2026-11-26 19:00:30", "2026-11-26 7:05 am", "2026-11-26 12am", "2026-11-26 12pm", "2026-11-26 13pm",
    "2026-11-26 0:00", "2026-11-26 24:00", "tomorrow 9am", "today 18:30", "yesterday 8:00", "+2d 10:00",
    "-1d 10:00", "+1w 9:00", "fri 22:00", "friday 22:00", "next fri 22:00", "mon 9am", "wed 9am", "9am", "19:00",
    "7pm", "now", "nov 26", "26 nov", "26 november 2026 7pm", "Nov 26, 2026 7pm", "2026-13-01 10:00",
    "2026-00-10 10:00", "0000-01-01 10:00", "10000-01-01 10:00", "2026-3-4 5:06", "2026-03-29 02:30",
    "2026-03-29 03:30", "2026-10-25 02:30", "2026-10-25 01:30", "2026-03-08 02:30", "2026-11-01 01:30",
    "2026-09-27 02:30", "2026-04-05 02:30", "garbage", "2026-11-26 19:00 garbage", "19:00 2026-11-26",
    "2026-11-26 19:00+25:00", "2026-11-26 19:00-0530", "2026-11-26 19:00+05:30", "26/11/2026 19:00",
    "11/26/2026 19:00", "2026-11-26  19:00", "2026-11-26\t19:00", "Thu 26 Nov 2026", "THU 26 NOV 2026 7PM",
    "2026-11-26 7:00 PM", "2026-11-26 7 pm", "2026-11-26 7.30pm", "2028-02-29 10:00", "2027-02-29 10:00",
    "+0d 10:00", "+400d 10:00", "+d 10:00", "3d 10:00", "in 2d 10:00", "tomorrow", "tomorrow 25:00",
    "sat 10:00", "sun 10:00", "tue 10:00", "thu 10:00", "2026-11-11 10:00", "2026-11-12", "1 jan 2027 00:00",
]


def when_row(text, tz, date_only):
    p = parse_when(text, tz=ZoneInfo(tz), tz_source="Freshservice user time zone (X)", date_only=date_only,
                   today=TODAY)
    v = p.value
    row = {"value": None if v is None else v.isoformat(), "kind": type(v).__name__, "preview": p.preview,
           "error": p.error}
    if isinstance(v, dt.datetime):
        row["to_text"] = to_text(v, ZoneInfo(tz))
        row["to_text_no_tz"] = to_text(v, None)
    return row


def when():
    cases = []
    for text in WHEN_TEXTS:
        for tz in ZONES:
            for date_only in (False, True):
                cases.append({"text": text, "tz": tz, "date_only": date_only, "want": when_row(text, tz, date_only)})
    sources = ["", "FSV_TZ", "tz argument", "Freshservice user time zone (Tokyo)", "system time zone"]
    previews = []
    for src in sources:
        for typed in (False, True):
            for v in (dt.datetime(2026, 1, 2, 3, 4, tzinfo=dt.timezone.utc),
                      dt.datetime(2026, 1, 2, 3, 4, tzinfo=dt.timezone(dt.timedelta(hours=-5, minutes=-30))),
                      dt.date(2026, 1, 2), None):
                previews.append({"source": src, "typed": typed, "value": None if v is None else v.isoformat(),
                                 "want": preview(v, src, typed)})
    offsets = {str(h): offset_text(dt.datetime(2026, 1, 1, tzinfo=dt.timezone(dt.timedelta(minutes=h))))
               for h in (0, 30, -30, 330, -330, 540, 765, -720)}
    weeks = {f"{y}-{m}-{fw}": [[d.isoformat() for d in w] for w in month_weeks(y, m, fw)]
             for y, m in [(2026, 2), (2026, 11), (2024, 2), (2026, 12), (2027, 1)] for fw in (0, 6)}
    months = {f"{d}+{n}": add_months(dt.date.fromisoformat(d), n).isoformat()
              for d in ["2026-01-31", "2026-03-31", "2024-02-29", "2026-12-15", "2026-11-30"]
              for n in (-14, -12, -1, 0, 1, 2, 12, 13)}
    return {"today": TODAY.isoformat(), "cases": cases, "previews": previews, "offsets": offsets, "weeks": weeks,
            "months": months}


def files():
    tree = ROOT.resolve() / "files"
    (tree / "docs" / "sub").mkdir(parents=True)
    (tree / "docs" / ".hidden").mkdir()
    for rel, data in {"a.pdf": b"pdf", "b.PNG": b"png", "empty.pdf": b"", "doc one.txt": b"x", "docs/readme.md": b"r",
                      "docs/readme.txt": b"r", "docs/.env": b"e", "docs/sub/z.bin": b"zz",
                      "big.dat": b"x" * 2_500_000, "ünï.pdf": b"u"}.items():
        (tree / rel).write_bytes(data)
    cwd, home = tree, tree / "docs"
    os.environ["HOME"] = str(home)
    saved = os.getcwd()
    os.chdir(cwd)

    def rel(x):
        return x.replace(str(tree), "@ROOT@")

    check = []
    raws = ["", "a.pdf", "a.pdf; b.PNG", "a.pdf;b.PNG;", " a.pdf ;; b.PNG ", "a.pdf\nb.PNG", "missing.pdf", "docs",
            "empty.pdf", "~/readme.md", "~", "./docs/../a.pdf", "docs/sub/../readme.txt", "doc one.txt", "big.dat",
            "ünï.pdf", "/", "a.pdf; docs", "a.pdf; missing.pdf; empty.pdf", "docs/sub/z.bin", ["a.pdf", "b.PNG"],
            ["a.pdf", " ", "b.PNG"], ["a.pdf", 5], None, 0]
    rules = [{}, {"allowed": ["pdf"]}, {"allowed": [".PDF", "png"]}, {"allowed": ["txt"]}, {"max_files": 1},
             {"max_files": 0}, {"max_bytes": 1_000_000}, {"max_bytes": 2_500_000}, {"max_bytes": 3},
             {"allowed": [".pdf"], "max_files": 2, "max_bytes": 1_000_000}]
    for raw in raws:
        for r in rules:
            paths, err = S.check_files(raw, allowed=tuple(r.get("allowed", ())), max_files=r.get("max_files"),
                                       max_bytes=r.get("max_bytes"))
            check.append({"raw": raw, "rules": r, "want": [[rel(p) for p in paths], rel(err)]})
    complete = {}
    for v in ["", "a", "a.", "b", "B", "d", "do", "doc", "docs", "docs/", "docs/r", "docs/re", "docs/readme.",
              "docs/s", "docs/sub", "docs/.", "docs/.e", "./d", "~/", "~/r", "~/s", "a.pdf; d", "a.pdf; docs/r",
              "a.pdf;", "a.pdf; ", "nope/x", "ü", "e", "em", "big", "/nonexistent/x", "x; ; do", "  do", "docs/sub/"]:
        got = complete_path(v)
        complete[v] = None if got is None else rel(got)
    os.chdir(saved)
    os.environ["HOME"] = str(HOME)
    sizes = {str(n): S.human_size(n) for n in [0, 1, 1023, 1024, 1025, 1536, 10_000, 1048575, 1048576, 5_000_000,
                                                1073741823, 1073741824, 5 * 1024 ** 3, 5000 * 1024 ** 3]}
    quote = {s: shlex.quote(s) for s in ["", "a", "a b", "it's", "x/y.z", "a=b", "~", "a$b", "\"q\"", "é", "a\nb",
                                           "-x", "a,b", "a:b", "@x", "%x", "+x"]}
    return {"check": check, "complete": complete, "sizes": sizes, "quote": quote}



# ------------------------------------------------------------------ sessions
import copy  # noqa: E402
import io  # noqa: E402
import re  # noqa: E402
from urllib.parse import parse_qs, urlsplit  # noqa: E402

import httpx  # noqa: E402

import acme_catalog as AC  # noqa: E402
import acme_change_form as A  # noqa: E402
import formui_tenants as T  # noqa: E402
from fsv import client as client_mod  # noqa: E402
from fsv import config as fsv_config  # noqa: E402
from fsv.change_flow import FlowExit, PartialFailure  # noqa: E402
from rich.console import Console  # noqa: E402

JANE = [9000007001, "Jane Doe", "jane.doe@example.com"]
SAM = [9000002, "Sam Lee", "sam.lee@example.com"]
NOMAIL = [9000099, "No Mail", None]
CATALOG_JANE = [9000001, "Jane Doe", AC.JANE]
HOST = "acme.freshservice.com"
fsv_config.DOMAIN = HOST
fsv_config.API_V2, fsv_config.API_INT, fsv_config.LOGIN_URL = fsv_config._urls(HOST)


class TicketTenant(T.AcmeChanges):
    """The Acme fields as an incident form (`test_ticket_create.meta`)."""

    def get(self, p, q):
        if p == "/api/_/ticket_form_fields":
            keep = {"requester", "subject", "description", "priority", "impact", "status", "cloud_only", "stage",
                    "staging_start", "staging_end", "legacy_flag", "reviewer"}
            fields = [f for f in copy.deepcopy(self.fields)["change_fields"] if f["name"] in keep]
            return httpx.Response(200, json={"ticket_fields": fields})
        if p == "/api/_/ws/2/business_rules":
            rules = copy.deepcopy(self.rules)
            rules["business_rules"] = rules["business_rules"][:3]
            for r in rules["business_rules"]:
                r["module_type"] = "ticket"
            return httpx.Response(200, json=rules)
        return super().get(p, q)

    def write(self, method, p, body):
        if method == "POST" and p == "/api/_/tickets":
            return httpx.Response(200, json={"ticket": {"id": 123}})
        return super().write(method, p, body)


class Acme(T.AcmeChanges):
    def get(self, p, q):
        if p == "/lookup_choices":
            needle = q.get("q", "").casefold()
            return httpx.Response(200, json={"results": [{"id": x["id"], "name": x["value"]} for x in A.PEOPLE
                                                          if needle in x["value"].casefold()]})
        return super().get(p, q)


TENANTS = {"acme": Acme, "globex": T.Globex, "catalog": AC.AcmeCatalog, "ticket": TicketTenant}


def enc(v):
    if isinstance(v, LookupValue):
        return {"lookup": [v.id, v.display, v.email]}
    if isinstance(v, dt.datetime):
        return {"dt": v.isoformat()}
    if isinstance(v, dt.date):
        return {"date": v.isoformat()}
    if isinstance(v, (list, tuple)):
        return [enc(x) for x in v]
    if isinstance(v, dict):
        return {str(k): enc(x) for k, x in v.items()}
    return v


def dec(v):
    if isinstance(v, dict) and "lookup" in v:
        return LookupValue(*v["lookup"])
    if isinstance(v, dict) and "dt" in v:
        return dt.datetime.fromisoformat(v["dt"])
    if isinstance(v, dict) and "date" in v:
        return dt.date.fromisoformat(v["date"])
    if isinstance(v, list):
        return [dec(x) for x in v]
    return v


def request_row(req):
    parts = urlsplit(str(req.url))
    row = {"method": req.method, "path": parts.path,
           "query": sorted([k, v] for k, vs in parse_qs(parts.query, keep_blank_values=True).items() for v in vs)}
    if req.method != "GET":
        row["body"] = json.loads(req.content or b"null")
    return row


class Recorder:
    """The tenant's transport that remembers each exchange (routes for Rust)."""

    def __init__(self, tenant):
        self.tenant, self.rows = tenant, []

    def __call__(self, req):
        resp = self.tenant(req)
        row = request_row(req)
        if "text" in resp.headers.get("content-type", "") or resp.headers.get("content-type", "").startswith("text/html"):
            body = {"text": resp.text}
        else:
            body = {"json": json.loads(resp.content or b"null")}
        self.rows.append((row, resp.status_code, body))
        return resp

    def routes(self):
        out, seen = [], {}
        for row, status, body in self.rows:
            key = (row["method"], row["path"], json.dumps(row["query"]))
            seen.setdefault(key, []).append((row, status, body))
        for key, items in seen.items():
            same = all((s, b) == (items[0][1], items[0][2]) for _r, s, b in items)
            for row, status, body in items[:1] if same else items:
                route = {"method": row["method"], "path": row["path"]}
                if row["query"]:
                    route["query"] = {k: v for k, v in row["query"]}
                if status != 200:
                    route["status"] = status
                route.update(body)
                if not same:
                    route["once"] = True
                out.append(route)
        return out

    def log(self):
        return [row for row, _s, _b in self.rows]


def client_for(rec):
    c = client_mod.Client(cookies={"_x_m": "cookie", "fw-session-id": "session"})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(rec), follow_redirects=False)
    c._csrf = "tok"
    return c


def review_plain(review, width=100):
    buf = io.StringIO()
    con = Console(file=buf, width=width, force_terminal=False, color_system=None, legacy_windows=False,
                  highlight=False)
    review.render(con)
    return [line.rstrip() for line in buf.getvalue().rstrip("\n").split("\n")]


def row_of(s, it):
    v = s.view(it.key)
    return [v.visible, v.required, v.disabled, v.disabled_reason, enc(v.value), v.text, v.error, v.note]


def snap(s):
    return {"items": {it.key: row_of(s, it) for it in s.items}, "problems": s.problems(),
            "missing": s.missing_keys(), "input_errors": s.input_errors(), "visible": s.visible_keys()}


def delta(old, new):
    out = {k: v for k, v in new.items() if k != "items"}
    out["items"] = {k: v for k, v in new["items"].items() if old["items"].get(k) != v}
    return out


def draft_json(s):
    d = s.draft
    return {"answers": {k: enc(v) for k, v in d.answers.items()}, "requester": d.requester,
            "description_html": d.description_html, "attachments": list(d.attachments or []),
            "planning": enc(d.planning), "ack": sorted(getattr(d, "ack", []) or []), "s_ack": s.ack,
            "requester_value": enc(s.requester)}


def session_head(s):
    return {"title": s.title, "tz_source": s.tz_source, "load_notes": list(s.load_notes),
            "items": [[it.key, it.label, it.widget, it.entity, it.help, it.placeholder, list(it.allowed_types),
                       it.max_files, it.field is None] for it in s.items]}


def tmp_sub(x, tmp):
    text = json.dumps(x).replace(json.dumps(str(tmp))[1:-1], "@TMP@")
    return json.loads(re.sub(r"fetched [0-9TZ:+.-]+", "fetched @T@", text))


def plan_json(p, tmp):
    return tmp_sub({"errors": list(p.errors), "hash": p.hash, "review_hash": p.review.plan_hash,
                    "ok": p.ok, "blocking": list(p.review.blocking_unacked),
                    "review": review_plain(p.review), "payload": enc(p.payload)}, tmp)


def run_op(s, op, tmp, state):
    kind = next(k for k in op if k not in ("note",))
    if kind == "set":
        raw = dec(op["value"])
        if isinstance(raw, str):
            raw = raw.replace("@TMP@", str(tmp))
        elif isinstance(raw, list):
            raw = [x.replace("@TMP@", str(tmp)) if isinstance(x, str) else x for x in raw]
        out = s.set(op["set"], raw)
        res = {"notes": out.notes, "error": out.error}
    elif kind == "suggest":
        try:
            sug, note = s.suggestions(op["suggest"], op["query"])
            res = {"suggestions": [[x.label, x.detail, enc(x.value), x.selected] for x in sug], "note": note}
        except Exception as e:  # noqa: BLE001
            res = {"raised": f"{type(e).__name__}: {e}"}
    elif kind == "help":
        h = s.help_for(op["help"])
        res = {"help": {**h, "choices": [list(c) for c in h["choices"]]}}
    elif kind == "effects":
        res = {"effects": s.effects(op["effects"])}
    elif kind == "mark_invalid":
        s.mark_invalid(op["mark_invalid"], op["error"])
        res = {}
    elif kind == "ack":
        s.ack = op["ack"]
        res = {}
    elif kind == "plan":
        state["plan"] = s.build_plan(op["plan"])
        res = {"plan": plan_json(state["plan"], tmp)}
    elif kind == "save":
        path = str(tmp / op["save"])
        out = s.save_draft(path, state.get("plan"))
        res = {"path": tmp_sub(out, tmp), "text": tmp_sub((tmp / op["save"]).read_text(), tmp),
               "command": tmp_sub(s.command_for(path, state["plan"]), tmp) if state.get("plan") else None}
    elif kind == "send":
        try:
            res = {"sent": enc(s.send(state["plan"], allow_duplicate=op.get("allow_duplicate", False)))}
        except FlowExit as e:
            res = {"exit": e.code, "message": e.message}
        except PartialFailure as e:
            res = {"partial": str(e)}
    else:
        raise SystemExit(f"unknown op {op}")
    return kind, res


def run_scenario(sc):
    global HOME
    tmp = ROOT / f"sc-{sc['name']}"
    (tmp / "cfg").mkdir(parents=True)
    (tmp / "files").mkdir()
    for name, size in sc.get("files", {}).items():
        (tmp / "files" / name).write_bytes(b"x" * size)
    tmp_files = tmp / "files"
    os.environ["FSV_CONFIG_DIR"] = str(tmp / "cfg")
    os.environ["FSV_CACHE_DIR"] = str(tmp / "cache")
    os.environ["XDG_STATE_HOME"] = str(tmp / "state")
    for k, v in sc.get("env", {}).items():
        os.environ[k] = v
    (tmp / "cfg" / "config.json").write_text(json.dumps({"domain": HOST, **sc.get("config", {})}))
    fsv_config.refresh_paths()
    try:
        tenant = TENANTS[sc["tenant"]]()
        rec = Recorder(tenant)
        c = client_for(rec)
        who = LookupValue(*sc["requester"]) if sc.get("requester") else None
        kind = sc["kind"]
        if kind == "change":
            s = S.change_session(c, mode=sc.get("mode", "new"), target=sc.get("target"), requester=who,
                                 dry_run=sc.get("dry_run", False))
        elif kind == "sr":
            s = S.sr_session(c, sc["item"], requester=who, dry_run=sc.get("dry_run", False))
        else:
            s = S.ticket_session(c, requester=who, dry_run=sc.get("dry_run", False))
        prev = snap(s)
        out = {"head": session_head(s), "initial": prev, "draft": draft_json(s), "steps": []}
        state = {}
        for op in sc["ops"]:
            try:
                kind, res = run_op(s, op, tmp_files, state)
            except Exception as e:  # noqa: BLE001
                res = {"raised": f"{type(e).__name__}: {e}"}
            cur = snap(s)
            res["snap"] = delta(prev, cur)
            res["draft"] = draft_json(s)
            prev = cur
            out["steps"].append(tmp_sub(res, tmp_files))
        out["requests"] = [r for r in rec.log() if not r["path"].startswith("/a/")]
        routes = rec.routes()
        meta = '<meta name="csrf-token" content="tok">'
        page = next((r for r in routes if r["path"] == "/a/dashboard"), None)
        if page is None:
            routes.append({"method": "GET", "path": "/a/dashboard", "text": f"<html><head>{meta}</head></html>"})
        elif meta not in page["text"]:
            page["text"] = page["text"].replace("<html>", f"<html><head>{meta}</head>", 1)
        return out, routes
    finally:
        fsv_config.refresh_paths()
        for k in sc.get("env", {}):
            os.environ.pop(k, None)


def L(*a):
    return {"lookup": list(a)}


def sc_set(key, value):
    return {"set": key, "value": value}


SCENARIOS = []
from formui_scenarios import SCENARIOS  # noqa: E402,F811


def sessions():
    out = {}
    (ROUTES / "formui").mkdir(exist_ok=True)
    for sc in SCENARIOS:
        res, routes = run_scenario(sc)
        out[sc["name"]] = {"spec": sc, "want": res}
        (ROUTES / "formui" / f"{sc['name']}.json").write_text(json.dumps(routes, indent=1, ensure_ascii=False) + "\n")
    return out


def generate():
    out = {"_python": python_rev(), "fuzzy": fuzzy(), "when": when(), "files": files(), "sessions": sessions()}
    return out


if __name__ == "__main__":
    text = json.dumps(generate(), indent=1, ensure_ascii=False) + "\n"
    if "--check" in sys.argv[1:]:
        sys.exit(0 if OUT.read_text() == text else 1)
    OUT.write_text(text)
