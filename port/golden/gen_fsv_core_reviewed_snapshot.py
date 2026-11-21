"""Golden values for fsv.reviewed_snapshot.stable_snapshot, journal.snapshot_hash
and the urllib.parse.urlsplit/urlunsplit behaviour they depend on.

Usage (from a clean export of the Python repo):
    cd /tmp/fsv-py-<sha> && FSV_PY_SRC=$PWD FSV_PY_COMMIT=<full sha> uv run --offline python \
        <fsv-rust>/port/golden/gen_fsv_core_reviewed_snapshot.py \
        <fsv-rust>/crates/fsv-core/tests/fixtures/reviewed_snapshot.json
"""
import json
import sys
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

from fsv import journal
from fsv.reviewed_snapshot import stable_snapshot
from provenance import stamped

OUT = Path(sys.argv[1])


def attachment(signature="first"):
    return {"id": 701, "name": "acme.txt", "size": 12, "content_type": "text/plain",
            "attachment_url": f"https://files.example.com/acme.txt?Expires=100&Signature={signature}&Key-Pair-Id=key",
            "canonical_url": f"https://files.example.com/delivery/{signature}",
            "created_at": signature, "updated_at": signature, "has_access": True}


def rich(query):
    return ('<div><a href="https://files.example.com/acme?version=1&amp;' + query.replace("&", "&amp;")
            + '">Acme</a></div>')


values = [
    None, True, 3, 2.5, "", "plain text", [], {},
    {"parent": {"id": 123, "attachments": [attachment()]},
     "before": [{"id": 901, "body": "Acme note", "private": True, "attachments": [attachment()]}]},
    {"parent": {"id": 123, "attachments": [attachment("second")]},
     "before": [{"id": 901, "body": "Acme note", "private": True, "attachments": [attachment("second")]}]},
    {"attachments": [attachment()]},
    {"files": [attachment(), {"id": 2, "inline": True, "is_inline": False, "deleted": False, "is_deleted": None,
                              "parent_id": 9, "attachable_type": "Ticket", "uploaded_by": 4, "url": "x",
                              "attachments": [{"id": 5}]}]},
    {"attachments": {"id": 1, "nested": {"id": 2, "created_at": "x"}, "updated_at": "y"}},
    {"attachments": [[{"id": 1, "name": "n", "url": "u"}]]},
    {"body": "Acme", "private": True, "source": 2, "planned_start_date": "2030-01-02T10:00:00Z",
     "custom_fields": {"updated_at": "original", "link": "https://x.example/?Signature=a"}},
    {"created_at": 1, "updated_at": 2, "last_viewed_at": 3, "fetched_at": 4, "expires_at": 5,
     "url_expires_at": 6, "due_by": 7, "rows": [{"updated_at": 1, "id": 2}]},
    {"plain": {"id": 123, "body": "Acme", "private": True, "attachments": [{"id": 701, "name": "acme.txt", "size": 12}]}},
    rich("Expires=100&Signature=first&Key-Pair-Id=key"),
    rich("Expires=200&Signature=second&Key-Pair-Id=other"),
    rich("X-Amz-Date=first&X-Amz-Expires=100&X-Amz-Signature=first"),
    rich("X-Amz-Date=second&X-Amz-Expires=200&X-Amz-Signature=second").replace("version=1", "version=2"),
    "https://example.com/?Expires=100",
    "https://example.com/?expires=1&policy=p",
    "see https://a.example/x?SIGNATURE=s&a=1#frag and http://b.example/y?x-AMZ-credential=c&b= done",
    "https://a.example/p?AWSAccessKeyId=k&keep=1&keep=2&blank=&flag",
    "https://a.example/p?Signature=s&q=a+b%20c&u=%C3%A9&amp=1",
    "https://a.example?Signature=s",
    "https://a.example/?Signature=s&Expires=1",
    "HTTPS://A.EXAMPLE/P?Signature=s&k=v",
    "httphttps://a.example/?Signature=s",
    "http://",
    "https://\x1c",
    "x\u00a0https://a.example/?Signature=s\u2028next https://b/?a=1",
    "<a href='https://a.example/?Signature=s&amp;v=1'>",
    "https://a.example/?Signature=s&amp;v=1&amp;w=&#38;",
    "https://u:p@a.example:8443/p;params?Signature=s&k=v",
    "https://[::1]:80/?Signature=s",
    "https://[::1%eth0]/?Signature=s",
    "https://[::1%]/?Signature=s",
    "https://[v1.fe]/?Signature=s",
    "https://[v1]/?Signature=s",
    "https://[1.2.3.4]/?Signature=s",
    "https://[nothost]/?Signature=s",
    "https://x[::1]/?Signature=s",
    "https://[::1]x/?Signature=s",
    "https://[::1/?Signature=s",
    "https://[::1]/?a=1",
    "https://a\uff03b.example/?Signature=s",
    "https://a\uff03b.example/?a=1",
    "https://caf\u00e9.example/?Signature=s",
    "https://a.example/\u00e9?Signature=s&v=\u00e9",
    "https://a.example/?Signature=s;x=1",
    "https://a.example/?%53ignature=s",
    {"body": "https://[1.2.3.4]/?a=1", "other": "https://[bad/?Signature=s"},
]

snapshots = []
for value in values:
    row = {"value": value, "plan_hash": journal.plan_hash(value)}
    try:
        row["snapshot"] = stable_snapshot(value)
        row["snapshot_hash"] = journal.snapshot_hash(value)
    except ValueError as exc:
        row["error"] = str(exc)
    snapshots.append(row)

urls = [
    "", "http://a/b?c#d", "HTTP://A/B", "  \x00\x1fhttp://a/b", "ht\ttp://a\r\n/b", "1http://a", "mailto:x@y",
    "//host/path", "path?q", "#frag", "?q#f?g", "http:path", "http:///p", "http://a?b#c?d", "svn+ssh://h/p",
    "custom://h", "custom:/p", "custom:p", "file:///etc", "http://[::1]", "http://[::1]:x", "http://u@[::1]:8",
    "http://[::ffff:1.2.3.4]", "http://[v7.x]", "http://[vg.x]", "http://[v1.]", "http://[01.2.3.4]",
    "http://[1.2.3.04]", "http://[::1%a%b]", "http://[]", "http://a]@[::1]", "http://[::1]@b",
    "http://\uff1fa", "http://\u2100", "http://\u00e9", "http://\ufe6b/", "http://a:b@c:1/",
    "http://h/\uff03", "http://1:2:3:4:5:6:7::/",
]
splits = []
for url in urls:
    try:
        parts = urlsplit(url)
        splits.append({"url": url, "split": list(parts), "unsplit": urlunsplit(parts)})
    except ValueError as exc:
        splits.append({"url": url, "error": str(exc)})

unsplits = [[parts, urlunsplit(parts)] for parts in [
    ["", "", "", "", ""], ["http", "", "", "", ""], ["http", "", "p", "", ""], ["http", "", "/p", "q", "f"],
    ["custom", "", "/p", "", ""], ["custom", "", "p", "", ""], ["", "", "//p", "", ""], ["", "h", "p", "", ""],
    ["https", "h", "", "a=1", ""], ["", "", "", "", "f"], ["mailto", "", "x@y", "", ""],
]]

OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(stamped({"snapshots": snapshots, "urlsplit": splits, "urlunsplit": unsplits}),
                          indent=1, ensure_ascii=False) + "\n")
