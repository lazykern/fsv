from common import Fake, rec, dump, install_journal
from fsv import change_tasks as t

rec("id", lambda: [t._id(v) for v in [5, "05", "0", True, "x", -2, 1.5]])
agents = [{"email": "Ann@x.io", "user_id": 11}, {"email": "bob@x.io", "user_id": 12, "deleted": True},
          {"email": "cy@x.io", "user_id": 13, "value": "Cy (Deactivated)"}, {"email": "dee@x.io", "user_id": "x"}]
f = Fake({"GET autocomplete/agents": [agents]})
rec("owner_none", lambda: [t.resolve_owner(None, f), t.resolve_owner("", f), t.resolve_owner("42", f)])
rec("owner_email", lambda: t.resolve_owner("ann@X.io", f))
rec("owner_me", lambda: t.resolve_owner("me", f))
rec("owner_deleted", lambda: t.resolve_owner("bob@x.io", f))
rec("owner_deact", lambda: t.resolve_owner("cy@x.io", f))
rec("owner_badid", lambda: t.resolve_owner("dee@x.io", f))
rec("owner_fail", lambda: t.resolve_owner("ann@x.io", Fake({"GET autocomplete/agents": ("session",)})))
f = Fake({"GET bootstrap/agents_groups": {"groups": [{"name": "Ops", "id": 4}, {"name": "dup", "id": 5}, {"name": "DUP", "id": 6}]}})
rec("group", lambda: [t.resolve_group(None, f), t.resolve_group("  ", f), t.resolve_group("9", f), t.resolve_group("ops", f)])
rec("group_dup", lambda: t.resolve_group("dup", f))
rec("group_missing", lambda: t.resolve_group("x", Fake()))
rec("status_label", lambda: t.status_label("  In Progress "))
rec("status_label_bad", lambda: t.status_label("a\tb"))
rec("status_label_blank", lambda: t.status_label("  "))
fields = {"change_task_fields": [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"},
                                                               {"id": 9, "value": "Old", "deleted": True}]}]}
f = Fake({"GET change_task_fields": fields})
rec("resolve_status", lambda: [t.resolve_status("open", f), t.resolve_status("2", f)])
rec("resolve_status_deleted", lambda: t.resolve_status("Old", f))
rec("resolve_status_nometa", lambda: t.resolve_status("Open", Fake()))
rec("date", lambda: [t._date(v) for v in [None, " ", "2026-01-02T03:04:05Z", "2026-01-02T03:04+07:00", "2026-01-02T03:04:05.000+00:00"]])
for i, v in enumerate(["2026-01-02", "2026-01-02T03:04:05", "2026-01-02T03:04:05.5Z", "2026-01-02T03:04:05+05:30:30", "nope T"]):
    rec(f"date_bad{i}", lambda v=v: t._date(v))
rec("effort", lambda: [t._effort(v) for v in [None, "", "1h30m", "90m", "01:30", "2h", " 3h 75m ", "0:05"]])
rec("effort_bad", lambda: t._effort("1:75"))
rec("effort_bad2", lambda: t._effort("soon"))
rec("note", lambda: [t._note(v) for v in [None, " ", "a\r\nb\tc"]])
rec("note_bad", lambda: t._note("a\x07"))
p = t.build_plan("CHN-12", "Review", owner_id=7, group_id=4, status="Done", status_id=3, note="x < y\r\nz",
                 notify_before="1 hour", planned_start="2026-01-02T03:00:00Z", planned_end="2026-01-02T05:00:00+01:00",
                 planned_effort="90m")
rec("plan_full", lambda: [p.path, p.payload, p.hash, p.review()])
d = t.build_plan(12, "Review", status_id=1)
rec("plan_default", lambda: [d.payload, d.hash])
for name, kw in [("pe_title", dict(change_id=12, title=" ", status_id=1)), ("pe_owner", dict(change_id=12, title="x", status_id=1, owner_id=0)),
                 ("pe_group", dict(change_id=12, title="x", status_id=1, group_id=-1)), ("pe_status", dict(change_id=12, title="x", status_id=None)),
                 ("pe_notify", dict(change_id=12, title="x", status_id=1, notify_before="soon")),
                 ("pe_order", dict(change_id=12, title="x", status_id=1, planned_start="2026-01-02T05:00:00Z", planned_end="2026-01-02T04:00:00Z")),
                 ("pe_inc", dict(change_id="INC-1", title="x", status_id=1))]:
    rec(name, lambda kw=kw: t.build_plan(kw.pop("change_id"), kw.pop("title"), **kw))
f = Fake({"GET changes/12/tasks": {"tasks": [{"id": 1, "title": "a"}]}})
rec("inventory", lambda: t._inventory(f, d))
rec("inventory_badid", lambda: t._inventory(Fake({"GET changes/12/tasks": {"tasks": [{"id": "x"}]}}), d))
rec("inventory_404", lambda: t._inventory(Fake(), d))
rec("inventory_calls", lambda: f.calls)
rec("matches", lambda: [t._matches(r, pl) for r, pl in [
    ({"title": "Review", "owner_id": None}, d), ({"title": "Review"}, d), ({"title": "Review", "owner_id": 7}, d),
    ({"title": "Review", "owner_id": "7"}, p), ({"title": "Review", "owner_id": 8}, p)]])
rec("compare", lambda: [t._compare_fields(None, d), t._compare_fields({"title": "Review", "status": 1, "notify_before": "0"}, d),
                        t._compare_fields({"title": "Review", "status": 3, "notify_before": 60, "group_id": "4", "owner_id": 7,
                                           "planned_start_date": "2026-01-02T04:00:00+01:00", "planned_end_date": "bad",
                                           "description": "<div><p>x &lt; y<br>z</p></div>"}, p)])
rec("returned_id", lambda: [t._returned_id(r) for r in [{"task": {"id": 5}}, {"task": {"id": "x"}, "id": 6}, {"id": "7"}, [1], {"id": True}]])
# send
j = install_journal()
f = Fake({"GET changes/12/tasks": [{"tasks": [{"id": 1, "title": "a"}]}, {"tasks": [{"id": 1, "title": "a"}, {"id": 2, "title": "Review", "owner_id": None, "status": 1, "notify_before": 0}]}],
          "POST changes/12/tasks": {"created": True}})
rec("send_title_match", lambda: t.send(d, f, reviewed_hash=d.hash))
rec("send_events", lambda: j.events())
rec("send_again", lambda: t.send(d, f, reviewed_hash=d.hash))
j = install_journal()
f = Fake({"GET changes/12/tasks": [{"tasks": []}, {"tasks": []}], "POST changes/12/tasks": {"task": {"id": 9}}})
rec("send_unverified", lambda: t.send(d, f, reviewed_hash=d.hash))
j = install_journal()
f = Fake({"GET changes/12/tasks": [{"tasks": []}, {"tasks": [{"id": 9, "title": "Review", "owner_id": None}]}], "POST changes/12/tasks": {"task": {"id": 9}}})
rec("send_partial_fields", lambda: t.send(d, f, reviewed_hash=d.hash))
j = install_journal()
f = Fake({"GET changes/12/tasks": {"tasks": []}, "POST changes/12/tasks": ("status", 422)})
rec("send_422", lambda: t.send(d, f, reviewed_hash=d.hash))
rec("send_422_events", lambda: j.events())
j = install_journal()
f = Fake({"GET changes/12/tasks": {"tasks": []}, "POST changes/12/tasks": ("status", 500)})
rec("send_500", lambda: t.send(d, f, reviewed_hash=d.hash))
rec("send_500_events", lambda: j.events())
rec("send_guards", lambda: [t.send(d, Fake(), reviewed_hash=None), t.send(d, Fake(), reviewed_hash=None, dry_run=True)["status"]])
rec("send_ro", lambda: t.send(d, Fake(), reviewed_hash=None, read_only=True))
rec("send_hash", lambda: t.send(d, Fake(), reviewed_hash="x"))
j = install_journal()
rec("send_noinv", lambda: t.send(d, Fake(), reviewed_hash=d.hash))
dump()
