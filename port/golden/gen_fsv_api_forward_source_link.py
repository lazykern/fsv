"""Golden exact-source forward results: the source link readback is unavailable.

Runs upstream's `wire` pytest fixture (tests/test_ticket_properties_http.py)
and records a projection of each send result: verification, fields, source
keys, warning presence and POST count.

    mkdir -p /tmp/fsv-py-915df4c
    git -C /Users/phusit.somb/lab/work/fsv archive 915df4c | tar -x -C /tmp/fsv-py-915df4c
    cd /tmp/fsv-py-915df4c && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-915df4c FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 915df4c) \\
        uv run --offline --project /tmp/fsv-py-915df4c \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_forward_source_link.py \\
        > /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-api/src/testdata/forward_source_link_oracle.json
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

import pytest

from provenance import stamped

SRC = Path(os.environ["FSV_PY_SRC"])
OUT = Path(tempfile.mkdtemp(prefix="fsv-fwd-")) / "cases.json"

HARNESS = '''
import json
import pytest
from fsv import ticket_conversation as conv
from test_ticket_properties_http import wire  # noqa: F401

VARIANTS = {
    "plain": {},
    "quoted_source": {"quoted_text_html": "<div>Original Acme message</div>", "source": 6},
    "formatted_sender": {"from_email": '"Acme Support" <SUPPORT@example.com>',
                         "to_emails": ["Acme Person <PERSON@example.com>"]},
    "delivery_mismatch": {"to_emails": ["other@example.com"]},
}
KEYS = ("verified", "fields", "source_conversation_id", "source_link_readback", "source_link_verified",
        "verification_scope")


@pytest.mark.parametrize("name", list(VARIANTS))
def test_record(wire, name):
    wire.conversation = {"id": 901, "body": "<div>Original Acme message</div>"}
    p = conv.build_plan(wire.client, wire.cid, "Acme update", kind="forward", conversation_id=901,
                        sender="support@example.com", notify=("person@example.com",))
    extra = VARIANTS[name]
    wire.conv_transform = lambda row: row.update(extra) if row.get("id") == wire.conv_id else None
    result = conv.send(p, wire.client, reviewed_hash=p.hash, public_confirmed=True)
    out = {k: result.get(k) for k in KEYS}
    out.update(name=name, extra=extra, has_warning="warning" in result,
               posts=len([r for r in wire.requests if r.method == "POST"]))
    with open(OUT, "a") as fh:
        fh.write(json.dumps(out) + "\\n")
'''


def main():
    test = SRC / "tests" / "test_gen_forward_source_link_tmp.py"
    test.write_text(HARNESS.replace("open(OUT", f"open({str(OUT)!r}"))
    stdout, sys.stdout = sys.stdout, sys.stderr
    try:
        code = pytest.main(["-q", "-p", "no:cacheprovider", str(test)], plugins=[])
    finally:
        sys.stdout = stdout
        test.unlink()
    if code != 0:
        raise SystemExit(f"harness failed: {code}")
    cases = [json.loads(line) for line in OUT.read_text().splitlines()]
    json.dump(stamped({"cases": cases}), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
