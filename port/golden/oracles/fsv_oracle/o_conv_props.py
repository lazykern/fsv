from common import Fake, rec, dump, install_journal
from ticket_meta import ticket_fields
from fsv import ticket_conversation as tc, ticket_properties as tp

ROW = {"id": 123, "subject": "Test", "status": 2}
PERSON = [[{"id": 11, "value": "Acme User", "email": "person@example.com"}]]


def api(rows=None, **extra):
    routes = {"GET ticket_form_fields": ticket_fields(), "GET ws/1/business_rules": {},
              "GET tickets/123": rows or {"ticket": ROW}, "GET autocomplete/requesters": PERSON}
    routes.update(extra)
    return Fake(routes)


rec("recipients", lambda: tc.recipients(api(), ["person@example.com"]))
rec("recipients_plain", lambda: tc.recipients(api(), ["Ann <a@b.co>"]))
rec("recipients_nomatch", lambda: tc.recipients(api(), ["other@example.com"]))
rec("recipients_down", lambda: tc.recipients(api(**{"GET autocomplete/requesters": ("session",)}), ["person@example.com"]))
rec("status_ctx", lambda: tc.status_context(api(), 123, "pending"))
rec("status_ctx_other", lambda: tc.status_context(api(), 123, "Resolved"))
rec("status_ctx_noticket", lambda: tc.status_context(api(**{"GET tickets/123": {"ticket": {"id": 9}}}), 123, "Pending"))
rec("status_ctx_rules_down", lambda: tc.status_context(api(**{"GET ws/1/business_rules": ("status", 404)}), 123, "Pending"))

c = api()
cc = tc.recipients(c, ["person@example.com"])
p = tc.build_plan(c, "123", "Hello <team>", cc=cc, set_status="Pending")
rec("plan_reply", lambda: [p.payload_json, p.path, p.hash, p.review(), p.record_hash, p.status_label])
n = tc.build_plan(c, "INC-123", "Note", kind="note", notify=("b@y.co",))
rec("plan_note", lambda: [n.payload_json, n.path, n.hash, n.record_hash])
rec("plan_bad_kind", lambda: tc.build_plan(c, "123", "x", kind="forward"))
rec("plan_bad_cc", lambda: tc.build_plan(c, "123", "x", cc=("bad",)))

j = install_journal()
c = api(rows=[{"ticket": ROW}, {"ticket": {**ROW, "status": 3}}],
        **{"POST tickets/123/reply": {"conversation": {"id": 456}},
           "GET tickets/123/conversations": {"conversations": [{"id": 456, "body": p.payload["body"], "cc_emails": list(cc)}]}})
rec("send_reply", lambda: tc.send(p, c, reviewed_hash=p.hash, public_confirmed=True))
rec("send_reply_events", lambda: j.events())
rec("send_reply_writes", lambda: c.writes())
rec("send_again", lambda: tc.send(p, c, reviewed_hash=p.hash, public_confirmed=True))
rec("send_cancel", lambda: tc.send(p, c, reviewed_hash=None))
rec("send_hash", lambda: tc.send(p, c, reviewed_hash="x"))
rec("send_unconfirmed", lambda: tc.send(p, c, reviewed_hash=p.hash))
j = install_journal()
c = api(**{"POST tickets/123/notes": {"note": {"id": "x"}}})
rec("send_note_noid", lambda: tc.send(n, c, reviewed_hash=n.hash))
rec("send_note_noid_events", lambda: j.events())
j = install_journal()
c = api(**{"POST tickets/123/notes": ("status", 422)})
rec("send_note_422", lambda: tc.send(n, c, reviewed_hash=n.hash))
rec("send_note_422_events", lambda: j.events())
j = install_journal()
c = api(**{"POST tickets/123/notes": {"note": {"id": 7}}, "GET tickets/123/conversations": ("status", 500)})
rec("send_note_unverified", lambda: tc.send(n, c, reviewed_hash=n.hash))
j = install_journal()
c = api(rows={"ticket": {**ROW, "subject": "Changed"}})
rec("send_reply_changed", lambda: tc.send(p, c, reviewed_hash=p.hash, public_confirmed=True))

# properties
PROW = {"id": 123, "subject": "Test", "tags": ["old"], "planned_start_date": None, "planned_end_date": None, "status": 2}
rec("record", lambda: tp._record(api(rows={"ticket": PROW}), 123))
rec("record_bare", lambda: tp._record(api(rows={"id": "123", "x": 1}), 123))
rec("record_wrong", lambda: tp._record(api(rows={"ticket": {"id": 5}}), 123))
rec("record_list", lambda: tp._record(api(rows=[[1]]), 123))
c = api(rows={"ticket": PROW})
pp = tp.build_plan(c, "123", tags=["a", "b"], planned_start="2030-01-01T09:00:00+07:00")
rec("prop_plan", lambda: [pp.body_json, pp.before_json, pp.record_hash, pp.hash, pp.review()])
for name, kw in [("pe_tags", dict(tags=[" a"])), ("pe_empty_tags", dict(tags=[])), ("pe_none", dict()),
                 ("pe_date", dict(planned_end="2030-01-01")), ("pe_blank_date", dict(planned_start=" "))]:
    rec(name, lambda kw=kw: tp.build_plan(c, "123", **kw))
rec("pe_missing", lambda: tp.build_plan(api(rows={"ticket": {"id": 123}}), "123", tags=["a"]))
rec("pe_rules_down", lambda: tp.build_plan(api(rows={"ticket": PROW}, **{"GET ws/1/business_rules": ("status", 404)}), "123", tags=["a"]))
rec("pe_chn", lambda: tp.build_plan(c, "CHN-1", tags=["a"]))
j = install_journal()
after = {**PROW, "tags": ["a", "b"], "planned_start_date": "2030-01-01T02:00:00Z"}
c = api(rows=[{"ticket": PROW}, {"ticket": after}], **{"PUT tickets/123": {"ticket": after}})
rec("prop_send", lambda: tp.send(pp, c, reviewed_hash=pp.hash))
rec("prop_send_events", lambda: j.events())
rec("prop_send_writes", lambda: c.writes())
rec("prop_send_again", lambda: tp.send(pp, c, reviewed_hash=pp.hash))
rec("prop_cancel", lambda: tp.send(pp, c, reviewed_hash=None))
rec("prop_hash", lambda: tp.send(pp, c, reviewed_hash="x"))
j = install_journal()
c = api(rows=[{"ticket": PROW}, {"ticket": {**PROW, "tags": ["z"]}}], **{"PUT tickets/123": {}})
rec("prop_send_mismatch", lambda: tp.send(pp, c, reviewed_hash=pp.hash))
j = install_journal()
c = api(rows=[{"ticket": PROW}, ("status", 500)], **{"PUT tickets/123": {}})
rec("prop_send_readback_fail", lambda: tp.send(pp, c, reviewed_hash=pp.hash))
j = install_journal()
c = api(rows={"ticket": {**PROW, "subject": "Changed"}})
rec("prop_send_changed", lambda: tp.send(pp, c, reviewed_hash=pp.hash))
j = install_journal()
c = api(rows={"ticket": PROW}, **{"PUT tickets/123": ("status", 409)})
rec("prop_send_409", lambda: tp.send(pp, c, reviewed_hash=pp.hash))
rec("prop_send_409_events", lambda: j.events())
dump()
