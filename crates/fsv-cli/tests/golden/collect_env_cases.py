#!/usr/bin/env python3
"""Rebuild ../fixtures/diff_env_cases.json from the integration tests' `Env` runs.

    python3 crates/fsv-cli/tests/golden/collect_env_cases.py

Runs `cargo test -p fsv-cli --offline` with FSV_CASE_DUMP set (see
`tests/common/mod.rs`), merges the cases each test recorded, drops runs whose
routes are a stock fixture (`diff_cases.json` has those) and duplicates, and
writes the rest for the differential test.
"""
import glob
import json
import os
import subprocess
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "../../../.."))
FIXTURES = os.path.join(HERE, "../fixtures")
SKIP_COMMANDS = {"tui", "completion", "--install-completion", "--show-completion"}


def main():
    dump = tempfile.mkdtemp()
    env = {**os.environ, "FSV_CASE_DUMP": dump}
    subprocess.run(["cargo", "test", "--offline", "-p", "fsv-cli", "--tests"], cwd=ROOT, env=env, check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    stock = []
    for f in glob.glob(os.path.join(FIXTURES, "routes", "*.json")):
        stock.append(json.load(open(f)))
    seen = set()
    out = []
    for f in sorted(glob.glob(os.path.join(dump, "*.jsonl"))):
        for line in open(f):
            case = json.loads(line)
            steps = case["steps"]
            last = steps[-1]
            if not last["args"] or last["args"][0] in SKIP_COMMANDS:
                continue
            if last["routes"] in stock and len(steps) == 1:
                continue
            key = json.dumps([[s["args"], s["stdin"], s["routes"], s["seed"]] for s in steps] + [last["session"], last["vars"]],
                             sort_keys=True)
            if key in seen:
                continue
            seen.add(key)
            out.append(case)
    out.sort(key=lambda c: (c["name"], json.dumps(c["steps"][-1]["args"])))
    with open(os.path.join(FIXTURES, "diff_env_cases.json"), "w") as fh:
        json.dump(out, fh, separators=(",", ":"))
        fh.write("\n")
    print(len(out), "cases")


main()
