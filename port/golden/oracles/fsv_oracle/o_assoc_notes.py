from common import Fake, rec, dump, install_journal
from fsv import change_associations as ca, ticket_notes as tn

rec("id", lambda: [ca._id(v) for v in [7, "007", "0", -1, True, 1.0, "١", "x", "12"]])
f = Fake({"GET changes/7/tickets": {"tickets": [{"id": 3}, {"id": "4"}]},
          "GET changes/8/tickets": {"tickets": [{"id": 0}]},
          "GET changes/9/tickets": ["x"],
          "GET changes/10/tickets": ("session",)})
r = ca.AssociationReader(f)
rec("read_page_ok", lambda: r.int_get("changes/7/tickets", {"page": 1}))
rec("read_page_bad_id", lambda: r.int_get("changes/8/tickets"))
rec("read_page_not_dict", lambda: r.int_get("changes/9/tickets"))
rec("read_page_session", lambda: r.int_get("changes/10/tickets"))
rec("read_page_404", lambda: r.int_get("changes/11/tickets"))
f = Fake({"GET changes/7/tickets": {"tickets": [{"id": 3}, {"id": 5}]},
          "GET changes/7/problems": {"problems": [{"id": 1}]},
          "GET changes/7/releases": {"releases": []}})
rec("inventory_ok", lambda: ca.inventory(7, f))
rec("ticket_ids", lambda: sorted(ca.ticket_ids(7, f)))
f = Fake({"GET changes/7/tickets": {"tickets": [{"id": "x"}]}})
rec("ticket_ids_bad", lambda: sorted(ca.ticket_ids(7, f)))
p = ca.build_plan("CHN-7", [9, "3", 9])
rec("plan", lambda: [p.method, p.path, p.payload, p.params, p.hash, p.review()])
u = ca.build_plan("7", [4], unlink=True)
rec("plan_unlink", lambda: [u.method, u.path, u.payload, u.params, u.hash])
rec("plan_errs", lambda: [str(e) for e in [
    *(lambda fs: [x for x in fs])([])]])
for name, fn in [("e_two", lambda: ca.build_plan("7", [4, 5], unlink=True)),
                 ("e_bool", lambda: ca.build_plan("7", [True])),
                 ("e_inc", lambda: ca.build_plan("INC-7", [1])),
                 ("e_empty", lambda: ca.build_plan("7", []))]:
    rec(name, fn)
# send: verified add
j = install_journal()
f = Fake({"GET changes/7/tickets": [{"tickets": []}, {"tickets": [{"id": 3}]}], "PUT changes/7/tickets/associate": {}})
p3 = ca.build_plan("7", [3])
rec("send_ok", lambda: ca.send(p3, f, reviewed_hash=p3.hash))
rec("send_ok_events", lambda: j.events())
rec("send_ok_body", lambda: [[c["method"], c["path"], c["params"], c["body"]] for c in f.writes()])
rec("send_again", lambda: ca.send(p3, f, reviewed_hash=p3.hash))
rec("send_dry", lambda: ca.send(p3, f, reviewed_hash=None, dry_run=True)["status"])
rec("send_ro", lambda: ca.send(p3, f, reviewed_hash=None, read_only=True))
rec("send_bad_hash", lambda: ca.send(p3, f, reviewed_hash="x"))
j = install_journal()
f = Fake({"GET changes/7/tickets": {"tickets": [{"id": 1}]}, "DELETE changes/7/tickets/1": ("status", 422)})
u1 = ca.build_plan("7", [1], unlink=True)
rec("send_cancel", lambda: ca.send(u1, f, reviewed_hash=None))
rec("send_422", lambda: ca.send(u1, f, reviewed_hash=u1.hash))
rec("send_422_events", lambda: j.events())

# ticket notes
rec("valid_recipient", lambda: [tn._valid_recipient(v) for v in ["a@b.co", "Ann Lee <ann@x.io>", "bad", "a@b", "x\n@y.co", "Ann <a@b.co> ", "<a@b.co>"]])
np_ = tn.build_plan("INC-5", "a < b\r\nline2\n\n\n\nnext \"q\"", public=False)
rec("note_plan", lambda: [np_.body_html, np_.payload, np_.hash, np_.path])
rec("note_html", lambda: tn.build_plan(5, "<p>x</p>", public=True, notify_emails=["b@y.co"], body_format="html").review())
for name, fn in [("ne_blank", lambda: tn.build_plan("5", "  ")), ("ne_html", lambda: tn.build_plan("5", "<p> </p>", body_format="html")),
                 ("ne_notify", lambda: tn.build_plan("5", "x", notify_emails=["bad"])), ("ne_chn", lambda: tn.build_plan("CHN-5", "x")),
                 ("ne_zero", lambda: tn.build_plan("0", "x"))]:
    rec(name, fn)
from dataclasses import replace
good = tn.build_plan("5", "hi")
rec("check_ok", lambda: tn._check_plan(good))
rec("check_bad", lambda: tn._check_plan(replace(good, body_html="<div>x</div><script>")))
rec("check_tenant", lambda: tn._check_plan(replace(good, tenant="other.freshservice.com")))
j = install_journal()
rec("dup_none", lambda: tn._duplicate(good))
j.record_plan("started", good.hash, action="ticket_note")
rec("dup_started", lambda: tn._duplicate(good))
j.record_plan("failed", good.hash, action="ticket_note")
rec("dup_failed", lambda: tn._duplicate(good))
rec("returned", lambda: [tn._returned_note(x) for x in [{"note": {"id": 1}}, {"conversation": {"id": 2}, "note": "x"}, {"id": 3}, {"x": 1}, [1]]])
rec("note_id", lambda: [tn._note_id(x) for x in [{"id": 1}, {"id": "77"}, {"id": "0"}, {"id": 0}, {"id": True}, {"id": "7a"}, None, {"id": 1.0}]])
j = install_journal()
f = Fake({"POST tickets/5/notes": {"conversation": {"id": "77"}},
          "GET tickets/5/conversations": {"conversations": [{"id": 77, "body": "<div>hi</div>", "private": True, "notify_emails": []}]}})
rec("note_send", lambda: tn.send(good, f, reviewed_hash=good.hash))
j = install_journal()
f = Fake({"POST tickets/5/notes": {"note": {"id": 9}}, "GET tickets/5/conversations": ("status", 500)})
rec("note_send_readback_fail", lambda: tn.send(good, f, reviewed_hash=good.hash))
dump()
