#!/usr/bin/env python3
"""Run Python and Rust fsv with the same routes; diff stdout, stderr, exit
code and the requests sent. Dev aid only; the Rust tests never run Python.

Usage: cmp.py ROUTES.json [--stdin TEXT] [--files DIR] [--show] [--emit TEST_NAME] -- ARGS...
With --emit, a Rust test asserting Python's output is appended to $EMIT_TO (or printed).
"""

import difflib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "../../../.."))
PY = os.environ.get("FSV_PY_REPO", "/Users/phusit.somb/lab/work/fsv")

argv = sys.argv[1:]
routes = os.path.abspath(argv.pop(0))
stdin = ""
files = None
show = False
emit = None
while argv and argv[0] != "--":
    opt = argv.pop(0)
    if opt == "--stdin":
        stdin = argv.pop(0)
    elif opt == "--files":
        files = os.path.abspath(argv.pop(0))
    elif opt == "--show":
        show = True
    elif opt == "--emit":
        emit = argv.pop(0)
args = argv[1:]


raw = {}


def run(impl):
    h = tempfile.mkdtemp()
    cfg = os.path.join(h, "config")
    os.makedirs(cfg)
    json.dump({"domain": "acme.freshservice.com"}, open(os.path.join(cfg, "config.json"), "w"))
    json.dump({"saved_at": time.time(), "domain": "acme.freshservice.com",
               "cookies": {"_x_m": "m", "_x_d": "d", "_x_w": "w", "fw-session-id": "s"}},
              open(os.path.join(cfg, "session.json"), "w"))
    work = os.path.join(h, "work")
    if files:
        shutil.copytree(files, work)
    else:
        os.makedirs(work)
    log = os.path.join(h, "log")
    env = {"HOME": h, "FSV_CONFIG_DIR": cfg, "FSV_CACHE_DIR": os.path.join(h, "cache"),
           "XDG_STATE_HOME": os.path.join(h, "state"), "FSV_NO_KEYCHAIN": "1", "NO_COLOR": "1",
           "COLUMNS": "200", "TZ": "UTC", "PATH": "/usr/bin:/bin"}
    if impl == "py":
        cmd = [os.path.join(PY, ".venv/bin/python"), os.path.join(HERE, "run_py.py"), routes, log, "--", *args]
    else:
        env["FSV_TEST_ROUTES"] = routes
        env["FSV_TEST_LOG"] = log
        cmd = [os.path.join(ROOT, "target/debug/fsv"), *args]
    p = subprocess.run(cmd, cwd=work, env=env, input=stdin.encode(), capture_output=True)
    p.stdout = p.stdout.decode("utf-8", "replace")
    p.stderr = p.stderr.decode("utf-8", "replace")
    reqs = []
    if os.path.exists(log):
        for line in open(log):
            r = json.loads(line)
            reqs.append(f"{r['method']} {r['url']} {json.dumps(r['body'], sort_keys=True)}")
    listing = []
    for d, _, fs in os.walk(work):
        for f in fs:
            p2 = os.path.join(d, f)
            listing.append(f"{os.path.relpath(p2, work)} {os.path.getsize(p2)}")
    shutil.rmtree(h)
    raw[impl] = {"stdout": p.stdout, "stderr": p.stderr}
    return {"exit": [str(p.returncode)], "stdout": p.stdout.split("\n"), "stderr": p.stderr.split("\n"),
            "requests": reqs, "files": sorted(listing)}


py, rs = run("py"), run("rs")
py_raw = raw["py"]
same = True
for k in ("exit", "stdout", "stderr", "requests", "files"):
    if py[k] != rs[k]:
        same = False
        print(f"### {k} differs")
        sys.stdout.writelines(l + "\n" for l in difflib.unified_diff(py[k], rs[k], "py", "rs", lineterm=""))
if same:
    print("IDENTICAL (exit %s)" % py["exit"][0])
same_out = all(py[k] == rs[k] for k in ("exit", "stdout", "stderr"))
if emit and not same_out:
    print("NOT EMITTED: outputs differ")
if emit and same_out:
    def lit(lines, raw):
        text = raw
        if "\r" not in text and '"#' not in text:
            return 'r#"' + text + '"#'
        out = []
        for ch in text:
            if ch == "\\":
                out.append("\\\\")
            elif ch == '"':
                out.append('\\"')
            elif ch == "\n":
                out.append("\\n")
            elif ch == "\r":
                out.append("\\r")
            elif ch == "\t":
                out.append("\\t")
            else:
                out.append(ch)
        return '"' + "".join(out) + '"'
    fixture = os.path.splitext(os.path.basename(routes))[0]
    argl = ", ".join(json.dumps(a) for a in args)
    writes = []
    for r in py["requests"]:
        m, url, _ = r.split(" ", 2)
        if m not in ("GET", "HEAD"):
            writes.append(f'"{m} {url.split("acme.freshservice.com", 1)[1].split("?")[0]}"')
    stdin_arg = f"Some({json.dumps(stdin)})" if stdin else "None"
    rust = [f"#[test]", f"fn {emit}() {{",
            f'    let r = run_fixture("{fixture}", &[{argl}], {stdin_arg});',
            f'    assert_eq!(r.code, {py["exit"][0]}, "{{}}", r.err);',
            f'    assert_eq!(r.out, {lit(py["stdout"], py_raw["stdout"])});',
            f'    assert_eq!(r.err, {lit(py["stderr"], py_raw["stderr"])});']
    if writes:
        rust.append(f'    assert_eq!(r.write_paths(), [{", ".join(writes)}]);')
    rust.append("}")
    out_file = os.environ.get("EMIT_TO")
    text = "\n".join(rust) + "\n\n"
    if out_file:
        open(out_file, "a").write(text)
    else:
        print(text)
if show:
    for k in ("exit", "stdout", "stderr", "requests"):
        print(f"--- py {k}")
        print("\n".join(py[k]))
