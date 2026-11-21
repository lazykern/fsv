"""Golden values for render.strip_html, render._requested_item_field_defs and
views.ticket_headers.

Usage (from the Python worktree):
    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/port/golden/gen_fsv_views_extra.py \
        ../fsv-rust/crates/fsv-views/tests/fixtures/views_extra.json \
        ../fsv-rust/crates/fsv-core/tests/fixtures/strip_html.json
"""
import json
import sys
from pathlib import Path

from fsv import render, views
from fsv.errors import APIError, SessionError
from provenance import stamped, write_sidecar

out = {}

out["strip_html"] = [
    [s, render.strip_html(s)]
    for s in [None, "", "plain", "<p>Hi&nbsp;there</p>", "  <b>a</b><i>b</i>  ", "a &lt;b&gt; &amp; c",
              "<br/>line<br>two", "x < y > z", "&#39;q&#39; &copy; &bogus;", "<p>\n multi\nline </p>",
              "<a href='x'>link</a>&amp;amp;"]
]

field = {
    "name": "a", "label": "A",
    "nested_fields": [
        {"name": "b", "label": "B", "nested_fields": [{"name": "c", "label": "C"}]},
        {"name": "d", "label": "D", "nested_fields": []},
        {"name": "e", "label": "E", "nested_fields": None},
    ],
}
out["field_defs"] = [
    [f, [d["name"] for d in render._requested_item_field_defs(f)]]
    for f in [field, {"name": "solo"}, {"name": "x", "nested_fields": [{"name": "y"}, {"name": "z"}]}]
]


class Fake:
    def __init__(self, result):
        self.result = result
        self.calls = []

    def int_get(self, path, params=None):
        self.calls.append(path)
        if isinstance(self.result, Exception):
            raise self.result
        return self.result


cases = {
    "rows": {"headers": [{"name": "subject"}, "x", 3, {"name": "status", "label": "Status"}]},
    "not_list": {"headers": "x"},
    "no_key": {"other": []},
    "top_list": [{"name": "subject"}],
    "null": None,
    "forbidden": APIError(403, {"message": "no"}),
    "other_error": RuntimeError("boom"),
    "session": SessionError("expired"),
}
rows = {}
for name, result in cases.items():
    c = Fake(result)
    try:
        got = views.ticket_headers(c)
    except SessionError as exc:
        got = {"session_error": str(exc)}
    rows[name] = {"calls": c.calls, "out": got}
out["ticket_headers"] = rows

Path(sys.argv[1]).parent.mkdir(parents=True, exist_ok=True)
Path(sys.argv[1]).write_text(json.dumps(stamped(out), indent=1, ensure_ascii=False) + "\n")
# strip_html is ported in fsv-core (fsv_core::output::strip_html).
Path(sys.argv[2]).write_text(json.dumps(out["strip_html"], indent=1, ensure_ascii=False) + "\n")
write_sidecar(sys.argv[2])
