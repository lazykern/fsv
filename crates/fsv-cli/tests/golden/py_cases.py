#!/usr/bin/env python3
"""Run a case file through Python only (`cmp.py --py-only`), for commands
the Rust binary does not have yet. Usage: py_cases.py CASEFILE.json [only-name...]

Honours FSV_PY_SRC / FSV_PY_REPO like cases.py.
"""
import json
import os
import subprocess
import sys

from cases import GOLDEN, ROUTES, py_hash


def main():
    case_file, only = sys.argv[1], set(sys.argv[2:])
    for c in json.load(open(case_file)):
        if only and c["name"] not in only:
            continue
        args = c["args"]
        if "hash_from" in c:
            hsh = py_hash(c["fixture"], c["hash_from"])
            args = [a.replace("{HASH}", hsh) for a in args]
        cmd = [sys.executable, os.path.join(GOLDEN, "cmp.py"), os.path.join(ROUTES, c["fixture"] + ".json"),
               *c.get("opts", []), "--py-only", "--", *args]
        p = subprocess.run(cmd, capture_output=True, text=True)
        print(f"===== {c['name']}\n{p.stdout}{p.stderr}")


if __name__ == "__main__":
    main()
