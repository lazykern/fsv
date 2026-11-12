"""Golden output for fsv-completion dynamic completers.

Usage: uv run python port/golden/oracles/fsv_port/completion_oracle.py <input.json> <out.json>
"""
import contextlib
import io
import json
import os
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from provenance import stamped  # noqa: E402

inp = json.loads(Path(sys.argv[1]).read_text())
OUT = Path(sys.argv[2])
tmp = Path(tempfile.mkdtemp())
os.environ["FSV_CONFIG_DIR"] = str(tmp / "cfg")
os.environ["FSV_CACHE_DIR"] = str(tmp / "cache")
os.environ.pop("FSV_COMPLETION_NETWORK", None)
(tmp / "cfg").mkdir()
(tmp / "cache" / "schema").mkdir(parents=True)
(tmp / "cache" / "filters").mkdir(parents=True)


def put(path: Path, doc: dict) -> None:
    doc = dict(doc)
    doc.setdefault("saved_at", time.time())
    path.write_text(json.dumps(doc))


for name, doc in inp["schema"].items():
    put(tmp / "cache" / "schema" / f"default--{name}.json", doc)
for name, doc in inp["filters"].items():
    put(tmp / "cache" / "filters" / f"default--{name}.json", doc)
put(tmp / "cache" / "groups--default.json", inp["groups"])

from fsv import _complete, completion  # noqa: E402
import fsv.client  # noqa: E402

calls: list = []


class Fake:
    def autocomplete(self, kind, query, params=None):
        calls.append(["autocomplete", kind, query, params])
        return [dict(r) for r in inp["remote"][kind]]

    def lookup_choices(self, link, query):
        calls.append(["lookup_choices", link, query, None])
        return [dict(r) for r in inp["remote"]["lookup"]]


fsv.client.get_client = lambda: Fake()

out = {}
for mode in ("offline", "network"):
    completion._completion_network = (lambda m=mode: m == "network")
    cases = []
    for completer, res, token, extra in inp["cases"]:
        calls.clear()
        try:
            pairs = [list(p) for p in _complete._completions(completer, res, token, extra)]
        except Exception as e:  # noqa: BLE001
            pairs = {"error": repr(e)}
        cases.append({"case": [completer, res, token, extra], "pairs": pairs, "calls": list(calls)})
    runs = []
    for argv in inp["runs"]:
        buf = io.StringIO()
        sys.argv = ["_complete", *argv]
        with contextlib.redirect_stdout(buf):
            _complete.main()
        runs.append({"argv": argv, "stdout": buf.getvalue()})
    out[mode] = {"cases": cases, "runs": runs}

OUT.write_text(json.dumps(stamped(out), indent=1, ensure_ascii=False) + "\n")
print("ok", sum(len(v["cases"]) for v in out.values()))
