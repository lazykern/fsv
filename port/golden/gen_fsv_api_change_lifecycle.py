"""Golden `change_lifecycle` for fsv-api `change_lifecycle`: `precheck`,
`outgoing` and `locks` over the state-flow and the inputs listed in the
Rust fixture. The inputs (`flow`, case records, bodies and states) are read
from the committed fixture (or argv[1]); the expected values are recomputed by the
Python functions.

    cd /tmp/fsv-py-<sha> && FSV_PY_SRC=$PWD FSV_PY_COMMIT=<sha> python \
        port/golden/gen_fsv_api_change_lifecycle.py
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

from fsv import change_lifecycle as cl  # noqa: E402
from provenance import stamped  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "..", "..", "crates", "fsv-api", "src", "testdata", "change_lifecycle_oracle.json")
inputs = json.load(open(sys.argv[1] if len(sys.argv) > 1 else FIXTURE))
flow = inputs["flow"]
doc = {
    "flow": flow,
    "precheck": [{**{k: c[k] for k in ("name", "record", "body")},
                  "expected": [list(x) for x in cl.precheck(flow, c["record"], c["body"])]}
                 for c in inputs["precheck"]],
    "outgoing": [{"state": c["state"], "expected": [[n, crit] for n, crit in cl.outgoing(flow, c["state"])]}
                 for c in inputs["outgoing"]],
    "locks": [{"state": c["state"], "expected": cl.locks(flow, c["state"])} for c in inputs["locks"]],
}
json.dump(stamped(doc), sys.stdout, indent=1, ensure_ascii=False)
