#!/usr/bin/env python3
"""Run golden cases through cmp.py. Usage: cases.py CASEFILE.json EMIT_TO [only-name...]

Each case: {"name", "fixture", "args", "opts"?: [...], "hash_from"?: [dry-run args]}
(an "opts" `--files DIR` is relative to this directory)
"{HASH}" in args is replaced with the Python dry run's plan_hash. Cases whose
outputs differ are reported and not emitted. FSV_RS_BIN, FSV_PY_REPO and
FSV_PY_SRC are passed on to cmp.py.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

GOLDEN = os.path.dirname(os.path.abspath(__file__))
ROUTES = os.path.join(GOLDEN, "../fixtures/routes")
PY = os.environ.get("FSV_PY_REPO", "/Users/phusit.somb/lab/work/fsv")


def py_hash(fixture, args, files=None):
    h = tempfile.mkdtemp()
    work = os.path.join(h, "work")
    if files:
        shutil.copytree(files, work)
    else:
        os.makedirs(work)
    cfg = os.path.join(h, "config")
    os.makedirs(cfg)
    json.dump({"domain": "acme.freshservice.com"}, open(os.path.join(cfg, "config.json"), "w"))
    json.dump({"saved_at": time.time(), "domain": "acme.freshservice.com",
               "cookies": {"_x_m": "m", "_x_d": "d", "_x_w": "w", "fw-session-id": "s"}},
              open(os.path.join(cfg, "session.json"), "w"))
    env = {"HOME": h, "FSV_CONFIG_DIR": cfg, "FSV_CACHE_DIR": os.path.join(h, "cache"),
           "XDG_STATE_HOME": os.path.join(h, "state"), "FSV_NO_KEYCHAIN": "1", "NO_COLOR": "1",
           "COLUMNS": "200", "TZ": "UTC", "PATH": "/usr/bin:/bin"}
    if os.environ.get("FSV_PY_SRC"):
        env["PYTHONPATH"] = os.environ["FSV_PY_SRC"]
    p = subprocess.run([os.path.join(PY, ".venv/bin/python"), os.path.join(GOLDEN, "run_py.py"),
                        os.path.join(ROUTES, fixture + ".json"), os.path.join(h, "log"), "--", *args],
                       cwd=work, env=env, capture_output=True, text=True)
    try:
        return json.loads(p.stdout)["plan_hash"]
    except Exception:
        sys.exit(f"no hash for {args}: {p.stdout}\n{p.stderr}")


def run(case_file, emit_to, only=()):
    """Emits the identical cases; returns the names that were not emitted."""
    skipped = []
    for c in json.load(open(case_file)):
        if only and c["name"] not in only:
            continue
        args = c["args"]
        opts = list(c.get("opts", []))
        files = None
        if "--files" in opts:
            i = opts.index("--files") + 1
            files = opts[i] = os.path.join(GOLDEN, opts[i])
        if "hash_from" in c:
            hsh = py_hash(c["fixture"], c["hash_from"], files)
            args = [a.replace("{HASH}", hsh) for a in args]
        cmd = [sys.executable, os.path.join(GOLDEN, "cmp.py"), os.path.join(ROUTES, c["fixture"] + ".json"),
               *opts, "--emit", c["name"], "--", *args]
        p = subprocess.run(cmd, env={**os.environ, "EMIT_TO": emit_to}, capture_output=True, text=True)
        first = p.stdout.splitlines()[0] if p.stdout else p.stderr
        print(f"{c['name']}: {first}")
        if not p.stdout.startswith("IDENTICAL") or "NOT EMITTED" in p.stdout or "WARNING" in p.stdout:
            print(p.stdout + p.stderr)
        if "NOT EMITTED" in p.stdout or p.returncode:
            skipped.append(c["name"])
    return skipped


if __name__ == "__main__":
    run(sys.argv[1], sys.argv[2], set(sys.argv[3:]))
