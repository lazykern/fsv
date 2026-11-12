#!/usr/bin/env python3
"""Expected values for the fsv-cli unit tests that call ported helpers by name
(`Requester`, `_run_form`, the asset helpers, `_flow_exit`, `_interactive`,
`_completion_shell`, `_refresh_cache`, `search_resource`).

    cd <python repo> && uv run python <this> [--check]
    FSV_PY_SRC=<git archive>/src FSV_PY_COMMIT=<sha> <venv python> <this>

Calls the Python functions in-process over the same canned routes the Rust
tests use (`tests/fixtures/routes/*.json`, httpx patched as in `run_py.py`) and
writes `tests/golden/units.json`.
"""
import contextlib
import io
import json
import os
import pathlib
import sys
import tempfile
import time
from types import SimpleNamespace
from urllib.parse import parse_qsl, urlsplit

GOLDEN = pathlib.Path(__file__).resolve().parent
ROUTES = GOLDEN.parent / "fixtures" / "routes"
OUT = GOLDEN / "units.json"
sys.path.insert(0, str(GOLDEN))
if os.environ.get("FSV_PY_SRC"):
    sys.path.insert(0, os.environ["FSV_PY_SRC"])

from pyrev import python_rev  # noqa: E402

HOME = pathlib.Path(tempfile.mkdtemp())
CFG = HOME / "config"
CFG.mkdir()
for k, v in {"HOME": HOME, "FSV_CONFIG_DIR": CFG, "FSV_CACHE_DIR": HOME / "cache", "XDG_STATE_HOME": HOME / "state",
             "FSV_NO_KEYCHAIN": "1", "NO_COLOR": "1", "COLUMNS": "80", "TZ": "UTC"}.items():
    os.environ[k] = str(v)
os.environ.pop("SHELL", None)


def write_config(extra=None):
    (CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com", **(extra or {})}))


write_config()
(CFG / "session.json").write_text(json.dumps({"saved_at": time.time(), "domain": "acme.freshservice.com",
                                              "cookies": {"_x_m": "m", "_x_d": "d", "_x_w": "w", "fw-session-id": "s"}}))

import httpx  # noqa: E402

routes, used, log = [], set(), []


def use(fixture):
    routes[:] = json.loads((ROUTES / f"{fixture}.json").read_text())
    used.clear()
    log.clear()


def matches(route, req):
    if route.get("method", "GET") != req.method or route.get("path") != req.url.path:
        return False
    pairs = parse_qsl(urlsplit(str(req.url)).query, keep_blank_values=True)
    return all((k, v if isinstance(v, str) else json.dumps(v)) in pairs for k, v in (route.get("query") or {}).items())


def handler(req):
    if req.url.path.startswith("/api/") or req.url.path.startswith("/search/") or req.url.path.startswith("/cmdb/"):
        log.append(f"{req.method} {req.url.path}")
    for i, r in enumerate(routes):
        if i not in used and matches(r, req):
            if r.get("once"):
                used.add(i)
            if "text" in r:
                return httpx.Response(r.get("status", 200), text=r["text"], headers={"content-type": "text/html"})
            return httpx.Response(r.get("status", 200), json=r.get("json", {}))
    return httpx.Response(404, json={"errors": "no route"})


_orig = httpx.Client.__init__


def _init(self, *a, **kw):
    kw["transport"] = httpx.MockTransport(handler)
    _orig(self, *a, **kw)


httpx.Client.__init__ = _init

import typer  # noqa: E402

from fsv import change_cli, cli, cli_sr, config  # noqa: E402
from fsv.change_flow import FlowExit  # noqa: E402
from fsv.client import Client  # noqa: E402
from fsv.drafts import Draft  # noqa: E402
from fsv.resources import REGISTRY  # noqa: E402


def run(fn):
    """`{"ok": value} | {"exit": code, "message": FlowExit message}` plus what was printed."""
    out, err = io.StringIO(), io.StringIO()
    res = {}
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            res["ok"] = fn()
        except typer.Exit as e:
            res["exit"] = e.exit_code
        except FlowExit as e:
            res["exit"], res["message"] = e.code, e.message
    res["out"], res["err"] = out.getvalue(), err.getvalue()
    return res


def lv(v):
    return None if v is None else {"id": v.id, "display": v.display, "email": v.email, "str": str(v)}


def requester():
    use("units")
    out = {"value": [], "ask": [], "resolve": []}
    for cls, key in ((change_cli._Requester, "requester"), (cli_sr._Requester, "sr_requester")):
        for flag, draft, prompted, cfg in [
            ("f@acme.com", "d@acme.com", "p@acme.com", "c@acme.com"),
            (None, "d@acme.com", "p@acme.com", "c@acme.com"),
            ("", "", "p@acme.com", "c@acme.com"),
            (None, None, None, "c@acme.com"),
            (None, None, None, 0),
            (None, None, None, 5),
            (None, None, None, ""),
            (None, None, None, None),
        ]:
            write_config({"defaults": {key: cfg}} if cfg is not None else {})
            r = cls(None, flag, False)
            r.prompted = prompted
            out["value"].append({"sr": key == "sr_requester", "flag": flag, "draft": draft, "prompted": prompted,
                                 "config": cfg, "want": r.value(Draft(requester=draft))})
    write_config()
    for answer in ["", "x@acme.com", "  "]:
        r = change_cli._Requester(None, None, False)
        asked = []
        orig = typer.prompt
        typer.prompt = lambda text, **kw: (asked.append([text, kw]), answer)[1]
        try:
            r.ask()
        finally:
            typer.prompt = orig
        out["ask"].append({"answer": answer, "asked": asked[0][0], "prompted": r.prompted})
    c = Client()
    for flag in ["ann@acme.com", "Ann", "Bo", "gone@acme.com", "nobody@acme.com", None]:
        r = change_cli._Requester(c, flag, False)
        log.clear()
        first = r.resolve(Draft())
        second = r.resolve(Draft())
        out["resolve"].append({"flag": flag, "want": [lv(first[0]), first[1]], "again": [lv(second[0]), second[1]],
                               "requests": list(log)})
    return out


def run_form():
    cases = []
    for action, saved, plan in [("save_draft", "/w/d.yaml", None), ("save_draft", None, None), ("cancel", None, None),
                                ("submit", None, None), ("submit", None, "PLAN")]:
        import fsv.formui.app as fa
        orig = fa.run_form
        fa.run_form = lambda session: SimpleNamespace(action=action, saved_path=saved, plan=plan)
        try:
            res = run(lambda: change_cli._run_form(None))
        finally:
            fa.run_form = orig
        cases.append({"action": action, "saved_path": saved, "plan": plan, "want": res})
    return cases


def assets():
    use("links")
    c = Client()

    def fresh(fn):
        cli._ASSET_CATEGORY_CACHE = None
        return run(fn)

    out = {"categories": fresh(lambda: cli._fetch_asset_categories(c))}
    out["resolve_category"] = [{"value": v, "want": fresh(lambda v=v: cli._resolve_asset_category(v, c))}
                               for v in ["all", "  All   Assets ", "application portfolio", "hardware", "application",
                                         "zzz", "  ", "Server"]]
    app_cat = cli._resolve_asset_category("Application Portfolio", c)
    out["resolve_id"] = []
    for value, associated, category in [("44", False, None), (" 46 ", True, None), ("AcmeApp", False, None),
                                        ("acme", False, None), ("zzz", False, None), ("acmeweb", False, None),
                                        ("AcmeApp", True, None), ("acme", True, None), ("nope", True, None),
                                        ("acme", False, app_cat), ("nope", True, app_cat), ("zzz", False, app_cat)]:
        out["resolve_id"].append({"value": value, "associated": associated, "category": category,
                                  "want": run(lambda: cli._resolve_change_asset_id(5, value, c, associated, category))})
    out["resolve_ids"] = [{"values": vs, "associated": a, "want": run(lambda: cli._resolve_change_asset_ids(5, vs, c, a))}
                          for vs, a in [(["44", "AcmeApp", " 44"], False), (["AcmeApp", "zzz"], False), (["AcmeApp"], True)]]

    def scripted(texts, picks=None):
        calls = []
        it = iter(texts)
        cli._prompt_text = lambda title, text, default="": (calls.append(["text", title, text, default]), next(it))[1]
        cli._prompt_multi_select = lambda title, text, values: (calls.append(["multi", title, text, [list(v) for v in values]]), picks)[1]
        return calls

    orig_text, orig_multi, orig_isatty, orig_no_input = cli._prompt_text, cli._prompt_multi_select, sys.stdin.isatty, cli.NO_INPUT
    out["prompt_category"], out["pick"] = [], []
    try:
        for answer in [None, "", "  hardware ", "zzz"]:
            calls = scripted([answer])
            out["prompt_category"].append({"answer": answer, "want": fresh(lambda: cli._prompt_asset_category(c)),
                                           "calls": calls})
        sys.stdin.isatty = lambda: True
        for name, texts, picks, category in [
            ("all", [None, "acme"], ["44", "46"], {"name": "All assets", "filter": "all_assets", "ci_type_id": ""}),
            ("category", ["acme"], ["44"], app_cat),
            ("prompted", ["", "acme"], ["46"], None),
            ("no_category", [None], None, None),
            ("no_query", [None], None, app_cat),
            ("no_pick", ["acme"], None, app_cat),
            ("no_match", ["zzz"], None, app_cat),
        ]:
            if name == "all":
                texts = texts[1:]
            calls = scripted(texts, picks)
            out["pick"].append({"name": name, "category": category, "texts": texts, "picks": picks,
                                "want": fresh(lambda: cli._pick_change_assets(5, c, category)), "calls": calls})
        sys.stdin.isatty = lambda: False
        out["pick"].append({"name": "no_tty", "category": None, "texts": [], "picks": None,
                            "want": fresh(lambda: cli._pick_change_assets(5, c, None)), "calls": scripted([])})
    finally:
        cli._prompt_text, cli._prompt_multi_select, sys.stdin.isatty, cli.NO_INPUT = orig_text, orig_multi, orig_isatty, orig_no_input

    def cmd(**kw):
        base = dict(id_="5", search=None, add=[], remove=[], page=1, per_page=30, dry_run=False, yes=False,
                    json_out=False, format_="json", pick=False, category_name=None, list_categories=False)
        return {"args": kw, "want": fresh(lambda: cli.assets_resource(**{**base, **kw}))}

    out["assets"] = [cmd(), cmd(search="acme"), cmd(search="acme", category_name="Application Portfolio"),
                     cmd(add=["AcmeApp", "44"], dry_run=True), cmd(remove=["AcmeApp"], dry_run=True),
                     cmd(list_categories=True), cmd(search="x", add=["1"]), cmd(category_name="zzz")]
    out["asset_categories"] = [{"format": f, "want": fresh(lambda f=f: cli.asset_categories_resource(f, False))}
                               for f in ["json", "csv"]]
    return out


def flow_exit():
    return [{"code": code, "message": msg, "want": run(lambda: cli._flow_exit(FlowExit(code, msg)))}
            for code, msg in [(2, "bad [x] <y>"), (0, "saved draft.yaml"), (3, ""), (0, ""), (1, "cancelled; nothing was sent")]]


def interactive():
    cases = []
    orig_isatty, orig = sys.stdin.isatty, cli.NO_INPUT
    try:
        for no_input, tty in [(False, True), (True, True), (False, False)]:
            cli.NO_INPUT = no_input
            sys.stdin.isatty = lambda tty=tty: tty
            cases.append({"no_input": no_input, "tty": tty, "want": cli._interactive()})
    finally:
        sys.stdin.isatty, cli.NO_INPUT = orig_isatty, orig
    return cases


def completion_shell():
    cases = []
    for shell, env in [("fish", "/bin/zsh"), ("tcsh", None), (None, "/bin/zsh"), (None, "/usr/local/bin/fish"),
                       ("", "/bin/bash"), (None, "/bin/sh"), (None, "/bin/ZSH"), (None, None), (None, "pwsh")]:
        if env is None:
            os.environ.pop("SHELL", None)
        else:
            os.environ["SHELL"] = env
        res = run(lambda: cli._completion_shell(shell))
        cases.append({"shell": shell, "env": env, "want": res})
    os.environ.pop("SHELL", None)
    return cases


def refresh_cache():
    use("auth")
    res = run(lambda: cli._refresh_cache(verbose=True, blocking=True))
    rows = sorted(line for line in res["out"].splitlines() if line.startswith("│"))
    files = sorted(str(p.relative_to(HOME)) for p in HOME.rglob("*.json") if "cache" in p.parts)
    return {"title": res["out"].splitlines()[0], "rows": rows, "files": files}


def search():
    use("units")
    res = REGISTRY["changes"]
    return [{"query": q, "json": j, "want": run(lambda q=q, j=j: cli.search_resource(res, q, 1, j, cli.SearchSort.relevance))}
            for q, j in [("acme", True), ("none", True)]]


def generate():
    return {"_python": python_rev(), "requester": requester(), "run_form": run_form(), "assets": assets(),
            "flow_exit": flow_exit(), "interactive": interactive(), "completion_shell": completion_shell(),
            "refresh_cache": refresh_cache(), "search_resource": search()}


if __name__ == "__main__":
    text = json.dumps(generate(), indent=1, ensure_ascii=False, sort_keys=False) + "\n"
    if "--check" in sys.argv[1:]:
        sys.exit(0 if OUT.read_text() == text else 1)
    OUT.write_text(text)
