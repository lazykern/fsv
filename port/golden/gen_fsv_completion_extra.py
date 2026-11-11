"""Golden values for fsv.completion helpers not covered by the dynamic oracle.

Usage (from the Python worktree):
    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/port/golden/gen_fsv_completion_extra.py \
        ../fsv-rust/crates/fsv-completion/tests/fixtures/dynamic_input.json \
        ../fsv-rust/crates/fsv-completion/tests/fixtures/extra_expected.json
"""
import json
import os
import sys
import tempfile
import time
from pathlib import Path

inp = json.loads(Path(sys.argv[1]).read_text())
OUT = Path(sys.argv[2])
tmp = Path(tempfile.mkdtemp())
os.environ["FSV_CONFIG_DIR"] = str(tmp / "cfg")
os.environ["FSV_CACHE_DIR"] = str(tmp / "cache")
os.environ.pop("FSV_COMPLETION_NETWORK", None)
(tmp / "cfg").mkdir()
(tmp / "cache" / "schema").mkdir(parents=True)


def put(path, doc):
    doc = dict(doc)
    doc.setdefault("saved_at", time.time())
    path.write_text(json.dumps(doc))


for name, doc in inp["schema"].items():
    put(tmp / "cache" / "schema" / f"default--{name}.json", doc)

from fsv import completion as c  # noqa: E402
from fsv.resources import REGISTRY  # noqa: E402

out = {}

tokens = ["", "o", "O", "1", "u", "l", "v", "9", "x"]
out["update_choice"] = {
    rname: [
        [field, tok, [list(p) for p in c.complete_update_choice(REGISTRY[rname], field)(tok)]]
        for field in ("status", "priority", "impact", "risk", "nosuch")
        for tok in tokens
    ]
    for rname in ("changes", "tickets", "problems")
}

topics = ["auth", "workflow", "fields", "scripting", "comments"]
out["help_topic"] = [
    [tok, [list(p) for p in c.complete_help_topic(topics)(tok)]]
    for tok in ("", "a", "W", "sc", "x")
]

fields = [
    {"name": "status", "label": "Status"},
    {"name": "cf_x", "label": "Région"},
    {"name": "", "label": "Only Label"},
    {"name": None, "label": None},
    {"name": 7, "label": ""},
]
out["field_matches"] = {
    "fields": fields,
    "rows": [
        [i, tok, c._field_matches(f, tok)]
        for i, f in enumerate(fields)
        for tok in ("", "st", "ST", "rég", "RÉG", "only", "7", "x")
    ],
}

cfg_file = tmp / "cfg" / "config.json"
config_texts = [None, "{}", '{"a": 1}', "not json", "[1, 2]", "", '{"completion": {"network": "on"}}']
rows = []
for text in config_texts:
    if text is None:
        cfg_file.unlink(missing_ok=True)
    else:
        cfg_file.write_text(text)
    rows.append([text, c._config_load()])
out["config_load"] = rows

network_cfgs = [
    {},
    {"completion": {}},
    {"completion": {"network": "on"}},
    {"completion": {"network": "off"}},
    {"completion": {"network": True}},
    {"completion": {"network": False}},
    {"completion": {"network": "true"}},
    {"completion": {"network": "True"}},
    {"completion": {"network": "1"}},
    {"completion": {"network": 1}},
    {"completion": {"network": 1.0}},
    {"completion": {"network": 0}},
    {"completion": {"network": "yes"}},
    {"completion": {"network": None}},
    {"completion": None},
    {"completion": "on"},
]
rows = []
for env in (None, "1", "0", "true"):
    if env is None:
        os.environ.pop("FSV_COMPLETION_NETWORK", None)
    else:
        os.environ["FSV_COMPLETION_NETWORK"] = env
    for cfg in network_cfgs:
        cfg_file.write_text(json.dumps(cfg))
        try:
            got = c._completion_network()
        except Exception as exc:
            got = type(exc).__name__
        rows.append([env, cfg, got])
os.environ.pop("FSV_COMPLETION_NETWORK", None)
out["completion_network"] = rows

OUT.write_text(json.dumps(out, indent=1, ensure_ascii=False) + "\n")
