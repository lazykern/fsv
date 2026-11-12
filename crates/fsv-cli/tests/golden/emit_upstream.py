#!/usr/bin/env python3
"""Rebuild tests/upstream_cmds.rs from the cases in tests/golden/upstream/.

    FSV_RS_BIN=target/debug/fsv python3 crates/fsv-cli/tests/golden/emit_upstream.py [--check]

Each `<part>.json` runs through cases.py (cmp.py --emit); `<part>_hand.rs`
holds the hand-written tests for the cases where Python and Rust differ. Fails
if the cases that are not emitted change, so a new divergence is not dropped
silently. `--check` exits 1 if the file is stale instead of writing it.
"""
import os
import pathlib
import subprocess
import sys
import tempfile

import cases
from pyrev import python_rev, stamp

GOLDEN = pathlib.Path(__file__).resolve().parent
OUT = GOLDEN.parent / "upstream_cmds.rs"
PARTS = ["watch", "time", "plan", "mail", "rev", "srf", "asset", "asset_hand.rs", "canned", "pv", "lp",
         "pw", "form", "tail_hand.rs", "missing", "clone", "lifecycle", "merge", "assign", "set_status", "note_send"]
# Not emitted: each is covered by a hand test in `*_hand.rs`.
DIVERGENT = {"asset_picker_api_error", "asset_picker_page_zero"}

HEADER = """//! Upstream commands ported from Python: watchers, ticket time,
//! planning-text, mark-known-error, bulk-priority, reply --bcc/--sender,
//! forward, update --reviewed with --set answers, the SR send-failure report,
//! the change asset picker, canned replies and scenarios, private ticket
//! views, list columns and exports, problem create and child writes,
//! Click's missing-parameter errors, change clones that fail part-way, and
//! ticket status transitions, merges, assignment and pick-up, closing
//! replies and notes (`--set-status`), and the add-note send report.
//! Expected output was captured from
//! Python on the same routes (`tests/golden/emit_upstream.py`); one-shot
//! routes serve the record before the write and the changed record after it.
//! {stamp}

mod common;
use common::run_fixture;
"""


def generate():
    rev = python_rev(os.environ.get("FSV_PY_REPO", cases.PY))
    body, skipped = [], set()
    with tempfile.TemporaryDirectory() as tmp:
        for part in PARTS:
            if part.endswith(".rs"):
                body.append((GOLDEN / "upstream" / part).read_text())
                continue
            emit_to = os.path.join(tmp, part + ".rs")
            skipped |= set(cases.run(str(GOLDEN / "upstream" / (part + ".json")), emit_to))
            lines = pathlib.Path(emit_to).read_text().split("\n")
            assert lines[0] == f"// {stamp(rev)}" and lines[1] == "", lines[:2]
            body.append("\n".join(lines[2:]))
    if skipped != DIVERGENT:
        sys.exit(f"not emitted changed: new {sorted(skipped - DIVERGENT)}, now emitted {sorted(DIVERGENT - skipped)}")
    raw = HEADER.replace("{stamp}", stamp(rev)) + "\n" + "\n\n".join(b.strip("\n") for b in body) + "\n"
    return subprocess.run(["rustfmt", "--edition", "2024", "--emit", "stdout"], input=raw,
                          capture_output=True, text=True, check=True).stdout


if __name__ == "__main__":
    text = generate()
    if "--check" in sys.argv[1:]:
        sys.exit(0 if OUT.read_text() == text else 1)
    OUT.write_text(text)
