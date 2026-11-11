"""Golden output for fsv-views source helpers.

Usage: uv run python /tmp/fsv_port/views_oracle.py <out.json>
"""
import json
import sys
from pathlib import Path

from fsv import render, views
from fsv.errors import APIError, SessionError
from fsv.resources import REGISTRY

OUT = Path(sys.argv[1])


class Scripted:
    def __init__(self, responses, key):
        self.responses = list(responses)
        self.key = key
        self.calls = []

    def int_get(self, path, params=None):
        self.calls.append([path, params])
        r = self.responses.pop(0) if self.responses else {"ok": {self.key: []}}
        if "api" in r:
            raise APIError(r["api"], {})
        if "session" in r:
            raise SessionError(r["session"])
        if "exc" in r:
            raise RuntimeError("boom")
        return r["ok"]


def miss_json(m):
    return None if m is None else {"what": m.what, "status": m.status}


P = "tickets/5/conversations"
COLLECT = [
    {"name": "page_numbers", "key": "conversations", "params": {"per_page": 2}, "responses": [
        {"ok": {"conversations": [{"id": 1}, {"id": 2}]}}, {"ok": {"conversations": [{"id": 3}]}}]},
    {"name": "has_more_dedupe", "key": "items", "params": {}, "responses": [
        {"ok": {"items": [{"id": 1}], "meta": {"has_more": True, "x": 1}}},
        {"ok": {"items": [{"id": 1}, {"id": 2}, {"name": "no id"}], "meta": {"has_more": False, "count": 2}}}]},
    {"name": "next_page_url", "key": "conversations", "params": {"include": "user", "page": 1}, "responses": [
        {"ok": {"conversations": [{"id": 1}], "next_page_url": "/api/_/tickets/5/conversations?page=2&per_page=1"}},
        {"ok": {"conversations": [{"id": 2}], "next_page_url": None}}]},
    {"name": "foreign_endpoint", "key": "conversations", "params": {}, "responses": [
        {"ok": {"conversations": [{"id": 1}], "next_page_url": "/api/v2/other?page=2"}}]},
    {"name": "unreadable_link", "key": "conversations", "params": {}, "responses": [
        {"ok": {"conversations": [{"id": 1}], "next_page_url": "/api/_/tickets/5/conversations"}}]},
    {"name": "start_token", "key": "conversations", "params": {"page": 1, "per_page": 1}, "responses": [
        {"ok": {"conversations": [{"id": 1}], "meta": {"next_start_token": "tok2"}}},
        {"ok": {"conversations": [{"id": 2}], "next_cursor": "c3"}},
        {"ok": {"conversations": [{"id": 3}], "meta": {"has_next": False}}}]},
    {"name": "api_error_second_page", "key": "conversations", "params": {"per_page": 1}, "responses": [
        {"ok": {"conversations": [{"id": 1}]}}, {"api": 500}]},
    {"name": "generic_error", "key": "conversations", "params": {}, "what": "Conversations", "responses": [{"exc": True}]},
    {"name": "session_error", "key": "conversations", "params": {}, "responses": [{"session": "expired"}]},
    {"name": "session_error_later", "key": "conversations", "params": {"per_page": 1}, "responses": [
        {"ok": {"conversations": [{"id": 1}]}}, {"session": "expired later"}]},
    {"name": "unreadable_page", "key": "conversations", "params": {}, "responses": [{"ok": "garbage"}]},
    {"name": "key_not_list", "key": "conversations", "params": {}, "responses": [{"ok": {"conversations": {"id": 1}}}]},
    {"name": "bare_list", "key": "conversations", "params": {}, "responses": [{"ok": [{"id": 1}, {"id": 2}]}]},
    {"name": "page_limit", "key": "items", "params": {}, "max_pages": 2, "responses": [
        {"ok": {"items": [{"id": 1}], "has_more": True}}, {"ok": {"items": [{"id": 2}], "has_more": True}}]},
    {"name": "cycle", "key": "items", "params": {}, "responses": [
        {"ok": {"items": [{"id": 1}], "next_page_url": "/api/_/tickets/5/conversations?page=2"}},
        {"ok": {"items": [{"id": 2}], "next_page_url": "/api/_/tickets/5/conversations?page=2"}}]},
    {"name": "total_count_empty_page", "key": "items", "params": {"page": 1, "per_page": 2}, "responses": [
        {"ok": {"items": [{"id": 1}, {"id": 2}], "meta": {"total_count": 5}}}, {"ok": {"items": []}}]},
    {"name": "total_count_done", "key": "items", "params": {"page": 2, "per_page": 2}, "responses": [
        {"ok": {"items": [{"id": 3}], "meta": {"count": "3"}}}]},
    {"name": "cursor_missing", "key": "items", "params": {"cursor": "a", "per_page": 1}, "responses": [
        {"ok": {"items": [{"id": 1}]}}]},
]

collect = []
for case in COLLECT:
    c = Scripted(case["responses"], case["key"])
    try:
        rows, meta, miss = views.collect_pages(c, P, case["key"], case["params"], what=case.get("what", ""),
                                               max_pages=case.get("max_pages", views.MAX_DETAIL_PAGES))
        result = {"rows": rows, "meta": meta, "failure": miss_json(miss)}
    except SessionError as e:
        result = {"session": str(e)}
    collect.append({**case, "result": result, "calls": c.calls})

VIEW_DATA = [
    {"ok": {"change_filters": [
        {"id": 1, "name": "All", "default": True},
        {"id": "mine", "name": "", "default": True, "user_id": 9},
        {"id": "", "name": "blank"},
        {"name": "no id"},
        {"id": 0, "name": "zero", "default": 0},
        "junk",
    ]}},
    {"ok": [{"id": "a", "name": "A"}]},
    {"ok": {"other": [{"id": 1}]}},
    {"api": 403},
    {"exc": True},
    {"session": "expired"},
]
load_views = []
for name in ("changes", "tickets"):
    for data in VIEW_DATA:
        c = Scripted([data], "x")
        try:
            got, miss = views.load_views(c, REGISTRY[name])
            result = {"views": [dict(v.to_json(), raw=v.raw) for v in got], "missing": miss_json(miss)}
        except SessionError as e:
            result = {"session": str(e)}
        load_views.append({"resource": name, "response": data, "result": result, "calls": c.calls})

RECORDS = [
    {"id": 1, "responder_id": 5, "type": "Incident"},
    {"id": 1, "agent_id": None, "responder_id": 5, "ticket_type": None, "type": ""},
    {"id": 1, "agent_id": 7, "ticket_type": "Service Request", "type": "Incident"},
    {"id": 1},
]
record_for_form = []
for name in ("tickets", "changes"):
    for r in RECORDS:
        record_for_form.append({"resource": name, "record": r, "result": views._record_for_form(REGISTRY[name], r)})

HTML = [None, "", "<p>a &amp; b</p><br>c ", "<div>x<br/>y</div>&lt;tag&gt; &nbsp;z", "plain", "<b>Tom</b> &#39;s\n\n<i>x</i>"]
strip_html = [[h, render.strip_html(h)] for h in HTML]

FIELD = {"name": "a", "nested_fields": [{"name": "b", "nested_fields": [{"name": "c"}]}, {"name": "d", "nested_fields": None}]}
field_defs = [d.get("name") for d in render._requested_item_field_defs(FIELD)]
field_defs_plain = [d.get("name") for d in render._requested_item_field_defs({"name": "solo"})]

OUT.write_text(json.dumps({
    "collect_pages": collect, "load_views": load_views, "record_for_form": record_for_form,
    "strip_html": strip_html, "field_defs": {"input": FIELD, "names": field_defs, "solo": field_defs_plain},
}, indent=1, ensure_ascii=False) + "\n")
print("ok")
