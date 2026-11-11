from common import Fake, rec, dump, install_journal
import typer
from fsv import change_approvals as ca, change_approval_actions as aa

REQ = {"id": 0, "name": "requested"}
EVERY = {"id": 1, "name": "everyone"}

rec("recipient", lambda: [ca._recipient("7"), ca._recipient("a@b.co")])
rec("recipient_ctl", lambda: ca._recipient("a\x01"))
rec("recipient_me", lambda: ca._recipient("me"))
agents = [{"email": "Ann@x.io", "id": 21, "user_id": 99}, {"email": "old@x.io", "id": 22, "value": "Old (Deactivated)"}]
f = Fake({"GET autocomplete/agents": [agents]})
rec("approvers", lambda: ca.resolve_approvers(["5", "ann@X.io"], f))
rec("approvers_empty", lambda: ca.resolve_approvers([], f))
rec("approvers_dup", lambda: ca.resolve_approvers(["21", "ann@x.io"], f))
rec("approvers_deact", lambda: ca.resolve_approvers(["old@x.io"], f))
rec("approvers_bad", lambda: ca.resolve_approvers(["me"], f))

p = ca.build_plan("CHN-12", "CAB", "Please\r\nreview <now>", approver_ids=[7, 8], rule="anyone")
rec("plan", lambda: [p.path, p.payload, p.hash, p.review()])
for name, args, kw in [("pe_name", (12, " ", "m"), dict(approver_ids=[1])), ("pe_msg", (12, "n", "a\x07"), dict(approver_ids=[1])),
                       ("pe_ids", (12, "n", "m"), dict(approver_ids=[0])), ("pe_dup", (12, "n", "m"), dict(approver_ids=[1, 1])),
                       ("pe_rule", (12, "n", "m"), dict(approver_ids=[1], rule="all")), ("pe_zero", ("0", "n", "m"), dict(approver_ids=[1]))]:
    rec(name, lambda a=args, k=kw: ca.build_plan(*a, **k))

row = lambda i, m, st=REQ, ty=EVERY: {"id": i, "member_id": m, "status": st, "type": ty}
f = Fake({"GET changes/12/approvals": [[row(1, 7)]]})
rec("inventory_list", lambda: ca._inventory(f, p))
rec("inventory_env", lambda: ca._inventory(Fake({"GET changes/12/approvals": {"approvals": [row(1, 7)], "meta": {"total_count": 1}}}), p))
rec("inventory_total", lambda: ca._inventory(Fake({"GET changes/12/approvals": {"approvals": [row(1, 7)], "meta": {"total_count": 2}}}), p))
rec("inventory_dup", lambda: ca._inventory(Fake({"GET changes/12/approvals": [[row(1, 7), row(1, 8)]]}), p))
rec("inventory_partial", lambda: ca._inventory(Fake({"GET changes/12/approvals": {"approvals": [], "partial": True}}), p))
rec("inventory_404", lambda: ca._inventory(Fake(), p))

grp = lambda gid, rows, name="CAB", st=0: {"id": gid, "name": name, "approval_status": st, "approvals": rows}
chains = lambda *groups: {"approval_chains": [{"id": 1, "approval_groups": list(groups)}]}
f = Fake({"GET changes/12/approval-chains": chains(grp(50, [{"id": 1, "member_id": 7, "status": REQ}]))})
rec("groups", lambda: ca.get_change_approval_groups(12, f))
rec("groups_calls", lambda: f.calls)
rec("groups_bad_id", lambda: ca.get_change_approval_groups(0, f))
rec("groups_unreadable", lambda: ca.get_change_approval_groups(12, Fake({"GET changes/12/approval-chains": {"x": 1}})))
rec("groups_total", lambda: ca.get_change_approval_groups(12, Fake({"GET changes/12/approval-chains": {**chains(), "meta": {"total_count": 2}}})))
rec("groups_links", lambda: ca.get_change_approval_groups(12, Fake({"GET changes/12/approval-chains": {**chains(), "links": {"next": "x"}}})))
rec("groups_more_empty", lambda: ca.get_change_approval_groups(12, Fake({"GET changes/12/approval-chains": {"approval_chains": [], "has_more": True}})))
rec("groups_nested", lambda: ca.get_change_approval_groups(12, Fake({"GET changes/12/approval-chains": chains({**grp(50, []), "approval_group_meta": {"count": 3}})})))
rec("groups_status", lambda: ca.get_change_approval_groups(12, Fake({"GET changes/12/approval-chains": chains(grp(50, [{"id": 1, "member_id": 7, "status": "x"}]))})))
rec("groups_member", lambda: ca.get_change_approval_groups(12, Fake({"GET changes/12/approval-chains": chains(grp(50, [{"id": 1, "member_id": 7, "member": {"id": 8}, "status": 0}]))})))
rec("groups_change", lambda: ca.get_change_approval_groups(12, Fake({"GET changes/12/approval-chains": chains(grp(50, [{"id": 1, "member_id": 7, "change_id": 13, "status": 0}]))})))
two = Fake({"GET changes/12/approval-chains": [{"approval_chains": [{"id": 1, "approval_groups": [grp(50, [])]}], "meta": {"has_next": True, "total_count": 2}},
                                               {"approval_chains": [{"id": 2, "approval_groups": [grp(51, [])]}], "meta": {"has_next": False, "total_count": 2}}]})
rec("groups_two_pages", lambda: [g["id"] for g in ca.get_change_approval_groups(12, two)])

anyone = {"id": 2, "name": "anyone"}
rec("verify_ok", lambda: ca._verify_new([row(3, 7, ty=anyone), row(4, 8, ty=anyone)], p))
rec("verify_missing", lambda: ca._verify_new([row(3, 7, ty=anyone)], p))
rec("verify_rule", lambda: ca._verify_new([row(3, 7), row(4, 8)], p))
rec("verify_member", lambda: ca._verify_new([{**row(3, 7, ty=anyone), "member": {"id": 9}}, row(4, 8, ty=anyone)], p))

# send request
e = ca.build_plan(12, "CAB", "ok", approver_ids=[7])
j = install_journal()
f = Fake({"GET changes/12/approvals": [[row(1, 5)], [row(1, 5), row(2, 7)]],
          "GET changes/12/approval-chains": [chains(), chains(grp(60, [{"id": 2, "member_id": 7, "status": REQ}]))],
          "POST changes/12/approval-groups": {"ok": True}})
rec("send_ok", lambda: ca.send(e, f, reviewed_hash=e.hash))
rec("send_ok_events", lambda: j.events())
rec("send_ok_writes", lambda: f.writes())
rec("send_again", lambda: ca.send(e, f, reviewed_hash=e.hash))
j = install_journal()
f = Fake({"GET changes/12/approvals": [[row(1, 5)], [row(1, 5)]], "POST changes/12/approval-groups": {}})
rec("send_nothing_new", lambda: ca.send(e, f, reviewed_hash=e.hash))
rec("send_nothing_events", lambda: j.events())
j = install_journal()
f = Fake({"GET changes/12/approvals": [[]], "POST changes/12/approval-groups": ("status", 400)})
rec("send_400", lambda: ca.send(e, f, reviewed_hash=e.hash))
rec("send_400_events", lambda: j.events())
rec("send_guards", lambda: [ca.send(e, Fake(), reviewed_hash=None), ca.send(e, Fake(), reviewed_hash=None, dry_run=True)["status"]])
rec("send_ro", lambda: ca.send(e, Fake(), reviewed_hash=None, read_only=True))
rec("send_hash", lambda: ca.send(e, Fake(), reviewed_hash="x"))
j = install_journal()
rec("send_noinv", lambda: ca.send(e, Fake(), reviewed_hash=e.hash))

# actions
ap = aa.build_plan("CHN-12", "101", "approve", remark="fine")
rj = aa.build_plan(12, 101, "reject", remark="no")
cn = aa.build_plan(12, 50, "cancel")
rec("action_plans", lambda: [[x.action, x.path, x.payload, x.hash, x.review()] for x in (ap, rj, cn)])
for name, args in [("ae_target", (12, "x", "approve")), ("ae_op", (12, 1, "maybe")), ("ae_remark", (12, 1, "approve", "a\x07")),
                   ("ae_reject", (12, 1, "reject", " ")), ("ae_cancel", (12, 1, "cancel", "r")), ("ae_change", ("INC-1", 1, "approve"))]:
    rec(name, lambda a=args: aa.build_plan(*a))
rec("status", lambda: [aa._status({"status": 0}), aa._status({"status": {"id": 1, "name": "approved"}}), aa._status({"status": {"id": 3}})])
rec("status_bad", lambda: aa._status({"status": {"id": 1, "name": " "}}))
rec("status_bad2", lambda: aa._status({"status": -1}))
rec("requested", lambda: [aa._requested(v) for v in [0, 1, {"id": 0}, {"id": 2, "name": "requested"}, {"id": 1, "name": "approved"}]])
gch = lambda rows: Fake({"GET changes/12/approval-chains": chains(grp(50, rows))})
rec("cancel_group", lambda: aa._cancel_group(gch([{"id": 1, "member_id": 7, "status": REQ}]), cn))
rec("cancel_group_missing", lambda: aa._cancel_group(Fake({"GET changes/12/approval-chains": chains()}), cn))
rec("cancel_group_none_requested", lambda: aa._cancel_group(gch([{"id": 1, "member_id": 7, "status": 1}]), cn))
rec("cancel_group_unreadable", lambda: aa._cancel_group(Fake(), cn))
before = grp(50, [{"id": 1, "member_id": 7, "status": REQ}])
rec("verify_cancel_gone", lambda: aa._verify_cancel(Fake({"GET changes/12/approval-chains": chains()}), cn, before))
rec("verify_cancel_changed", lambda: aa._verify_cancel(gch([{"id": 1, "member_id": 7, "status": {"id": 3, "name": "cancelled"}}]), cn, before))
rec("verify_cancel_still", lambda: aa._verify_cancel(gch([{"id": 1, "member_id": 7, "status": REQ}]), cn, before))
rec("verify_cancel_keys", lambda: aa._verify_cancel(gch([{"id": 2, "member_id": 7, "status": 3}]), cn, before))

j = install_journal()
f = Fake({"GET changes/12/approvals": [[row(101, 7)], [{**row(101, 7), "status": {"id": 1, "name": "approved"}}]],
          "PUT changes/12/approvals/101": {}})
rec("act_approve", lambda: aa.send(ap, f, reviewed_hash=ap.hash))
rec("act_approve_events", lambda: j.events())
rec("act_approve_writes", lambda: f.writes())
j = install_journal()
f = Fake({"GET changes/12/approvals": [[row(101, 7)], [{**row(101, 7), "status": 2}]], "PUT changes/12/approvals/101": {}})
rec("act_reject_raw", lambda: aa.send(rj, f, reviewed_hash=rj.hash))
j = install_journal()
f = Fake({"GET changes/12/approvals": [[row(101, 7)], [{**row(101, 7), "status": {"id": 2, "name": "rejected"}}]], "PUT changes/12/approvals/101": {}})
rec("act_approve_wrong_name", lambda: aa.send(ap, f, reviewed_hash=ap.hash))
rec("act_wrong_events", lambda: j.events())
j = install_journal()
rec("act_not_requested", lambda: aa.send(ap, Fake({"GET changes/12/approvals": [[{**row(101, 7), "status": 1}]]}), reviewed_hash=ap.hash))
j = install_journal()
f = Fake({"GET changes/12/approvals": [[row(101, 7)]], "PUT changes/12/approvals/101": ("status", 409)})
rec("act_409", lambda: aa.send(ap, f, reviewed_hash=ap.hash))
rec("act_409_events", lambda: j.events())
j = install_journal()
f = Fake({"GET changes/12/approval-chains": [chains(before), chains()], "PUT changes/12/approval-groups/50/cancel": {}})
rec("act_cancel", lambda: aa.send(cn, f, reviewed_hash=cn.hash))
rec("act_cancel_writes", lambda: f.writes())
rec("act_cancel_events", lambda: j.events())
rec("act_cancel_dry", lambda: aa.send(cn, gch([{"id": 1, "member_id": 7, "status": REQ}]), reviewed_hash=None, dry_run=True))
rec("act_guards", lambda: [aa.send(ap, Fake(), reviewed_hash=None)])
rec("act_ro", lambda: aa.send(ap, Fake(), reviewed_hash=None, read_only=True))
rec("act_hash", lambda: aa.send(ap, Fake(), reviewed_hash="x"))
j = install_journal()
rec("act_cancel_unreadable", lambda: aa.send(cn, Fake(), reviewed_hash=cn.hash))


# _run
def run(plan, fake, confirm=False, **kw):
    out = []
    aa.get_client = lambda: fake
    import fsv.change_approval_actions as mod
    orig_send = mod.send
    mod.typer.echo = lambda msg="", err=False: out.append([err, msg])
    mod.typer.confirm = lambda *a, **k: (out.append(["confirm", a[0]]), confirm)[1]
    base = dict(dry_run=False, read_only=False, yes=False, expected=None, no_input=False, json_out=False)
    base.update(kw)
    def patched(p, client=None, **k):
        return orig_send(p, fake, **k)
    mod.send = patched
    try:
        aa._run(plan, **base)
    finally:
        mod.send = orig_send
    return out


j = install_journal()
rec("run_ro", lambda: run(ap, Fake(), read_only=True))
rec("run_expect_bad", lambda: run(ap, Fake(), expected="x"))
rec("run_yes_noexpect", lambda: run(ap, Fake(), yes=True))
rec("run_noinput", lambda: run(ap, Fake(), no_input=True))
rec("run_decline", lambda: run(rj, Fake(), confirm=False))
rec("run_dry", lambda: run(rj, Fake(), dry_run=True))
j = install_journal()
f = Fake({"GET changes/12/approvals": [[row(101, 7)], [{**row(101, 7), "status": 2}]], "PUT changes/12/approvals/101": {}})
rec("run_yes_text", lambda: run(rj, f, yes=True, expected=rj.hash))
j = install_journal()
f = Fake({"GET changes/12/approval-chains": [chains(before), chains(before), chains()], "PUT changes/12/approval-groups/50/cancel": {}})
rec("run_cancel_json", lambda: run(cn, f, confirm=True, json_out=True))
dump()
