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

ROOT = pathlib.Path(tempfile.mkdtemp())
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


def generate():
    out = {"_python": python_rev(), "fuzzy": fuzzy(), "when": when(), "files": files()}
    return out


if __name__ == "__main__":
    text = json.dumps(generate(), indent=1, ensure_ascii=False) + "\n"
    if "--check" in sys.argv[1:]:
        sys.exit(0 if OUT.read_text() == text else 1)
    OUT.write_text(text)
