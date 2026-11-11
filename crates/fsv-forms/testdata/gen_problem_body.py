"""problem_expected.json: what fsv.forms.payload._check / _module_body return
as fsv.problem_workflows.create_plan calls them (empty key map, the problem
body order, the evaluated status, requester skipped), over the frozen
payload states of gen_py_golden.py.

    cd <python repo> && uv run python <this file> <python repo>
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.argv = [sys.argv[0], *(sys.argv[1:2] or ["."])]
sys.path.insert(0, str(HERE))
import gen_py_golden as G  # noqa: E402
from fsv.forms import payload  # noqa: E402

PROBLEM_ORDER = ("subject", "description", "status", "priority", "impact")


def main() -> None:
    out: dict = {}
    for name in G.PAYLOAD_STATES:
        st = G.payload_state(name)
        status = st.evaluated.values.get("status")
        out[name] = {
            "check": [[inc, list(skip), G.run(lambda: payload._check(st, inc, skip=tuple(skip)))]
                      for inc in (False, True) for skip in ([], ["requester"], ["overview_deck", "team"])],
            "module_body": G.run(lambda: list(payload._module_body(st, {}, PROBLEM_ORDER, status))),
            "status": G.V(status),
        }
    (HERE / "problem_expected.json").write_text(
        json.dumps(out, ensure_ascii=False, indent=1, allow_nan=False) + "\n")


main()
