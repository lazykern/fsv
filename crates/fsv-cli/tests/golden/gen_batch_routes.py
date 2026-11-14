"""Write the route fixtures for the reviewed commands of Python 36408b2
(integrated `assets`), 9070409 (`sr approvals`), be41032/9c8c91d (`changes
tasks-update`, `tasks-delete`), 0719e33 (`solutions create`, `problems
link-solution`), e116be9 (`tickets bulk`) and 1490984 (`changes
relationships`):

    python3 crates/fsv-cli/tests/golden/gen_batch_routes.py

Each target id is one scenario. A read that changes with the write serves
`before` for the reads up to the send (`once` routes), then `after`.
"""
import json

from gen_reviewed_routes import PAGE, ROUTES, fixed, reads, write
from gen_ticket_routes import CSRF


# --- change tasks: update reads the inventory for the plan and the rebuild;
# delete reads it for the plan, the send baseline and the rebuild.
UPDATE_BEFORE_READS = 2
DELETE_BEFORE_READS = 3

TASK_FIELDS = [
    {"name": "status", "label": "Status", "choices": [
        {"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"}, {"id": 3, "value": "Completed"}]},
    {"name": "notify_before", "label": "Notify before", "choices": [
        {"id": 0, "value": "Never"}, {"id": 900, "value": "15 Minutes"}]},
    {"name": "cf_system", "label": "System", "default": False, "field_type": "custom_dropdown", "choices": [
        {"id": 10, "value": "SAP"}, {"id": 11, "value": "Core Bank"}]},
    {"name": "cf_signoff", "label": "Sign-off", "default": False, "field_type": "custom_text",
     "required_for_closure": True},
]


def ctask(tid, title, status, **extra):
    row = {"id": tid, "title": title, "status": status, "owner_id": 131, "group_id": 4,
           "planned_start_date": "2030-01-01T09:00:00Z", "planned_end_date": "2030-01-01T10:00:00Z",
           "planned_effort": "1h", "description": "<div>Acme</div>", "custom_fields": {}}
    row.update(extra)
    return row


def change_tasks():
    def inv(*rows):
        return {"tasks": list(rows), "meta": PAGE}
    routes = []
    # 5: title/owner/group update readback matches
    routes += reads("/api/_/changes/5/tasks", inv(ctask(3, "Acme patch", 1), ctask(4, "Acme verify", 1)),
                    inv(ctask(3, "Acme patch v2", 1, owner_id=133, group_id=5), ctask(4, "Acme verify", 1)),
                    UPDATE_BEFORE_READS)
    # 7: completion accepted but the readback keeps the old status
    routes += reads("/api/_/changes/7/tasks", inv(ctask(8, "Acme close", 1, custom_fields={"signoff": "Acme ok"})),
                    inv(ctask(8, "Acme close", 1, custom_fields={"signoff": "Acme ok"})), UPDATE_BEFORE_READS)
    # 9: reminder set; the collection never returns notify_before
    routes += reads("/api/_/changes/9/tasks", inv(ctask(11, "Acme remind", 1)),
                    inv(ctask(11, "Acme remind", 1)), UPDATE_BEFORE_READS)
    # 6: delete removes task 5; 8: delete accepted, task 6 remains
    routes += reads("/api/_/changes/6/tasks", inv(ctask(5, "Acme drop", 1), ctask(7, "Acme keep", 1)),
                    inv(ctask(7, "Acme keep", 1)), DELETE_BEFORE_READS)
    routes += reads("/api/_/changes/8/tasks", inv(ctask(6, "Acme stuck", 1)),
                    inv(ctask(6, "Acme stuck", 1)), DELETE_BEFORE_READS)
    routes += [fixed("/api/_/change_task_fields", {"change_task_fields": TASK_FIELDS}),
               fixed("/search/autocomplete/agents", {"results": [
                   {"id": "ann@acme.com", "user_id": 131, "email": "ann@acme.com", "value": "Ann Lee"},
                   {"id": "bo@acme.com", "user_id": 133, "email": "bo@acme.com", "value": "Bo Kim"}]}),
               fixed("/api/_/bootstrap/agents_groups", {"groups": [{"id": 4, "name": "Ops"}, {"id": 5, "name": "DBA"}]}),
               write("PUT", "/api/_/changes/5/tasks/3", {"task": ctask(3, "Acme patch v2", 1)}),
               write("PUT", "/api/_/changes/7/tasks/8", {"task": ctask(8, "Acme close", 3)}),
               write("PUT", "/api/_/changes/9/tasks/11", {"task": ctask(11, "Acme remind", 1)}),
               write("PUT", "/api/_/changes/6/tasks/update", {}),
               write("PUT", "/api/_/changes/8/tasks/update", {}),
               CSRF]
    return routes


# --- integrated assets: the plan and the rebuild read the association
# inventory, then the readback.
ASSET_BEFORE_READS = 2


def cis(*rows, services=False):
    data = [{"item_id": i, "itam_cmdb_request_id": r, "name": n,
             "category": "Business service" if services else "Device"} for i, r, n in rows]
    return {"data": data, "meta": {"total_count": len(data), "total_pages": 1 if data else 0}}


def assoc(path, before, after, count=ASSET_BEFORE_READS, services=False):
    kind = "services" if services else "cis"
    once = {"method": "GET", "path": path, "query": {"association_type": kind}, "json": before, "once": True}
    return [once] * count + [{"method": "GET", "path": path, "query": {"association_type": kind}, "json": after}]


def integrated_assets():
    routes = []
    # tickets 123: add device 702 next to 701
    routes += assoc("/api/_/tickets/123/associated-cis", cis((701, 9001, "Acme laptop")),
                    cis((701, 9001, "Acme laptop"), (702, 9002, "Acme router")))
    routes += assoc("/api/_/tickets/123/associated-cis", cis((801, 9101, "Acme payroll"), services=True),
                    cis((801, 9101, "Acme payroll"), services=True), services=True)
    # problems 7: remove device 701
    routes += assoc("/api/_/problems/7/associated-cis", cis((701, 9001, "Acme laptop")), cis())
    # changes 5: listing, and an add whose readback misses the device
    routes += assoc("/api/_/changes/5/associated-cis", cis((703, 9003, "Acme db")), cis((703, 9003, "Acme db")))
    # sr 124: one device listed
    routes += assoc("/api/_/tickets/124/associated-cis", cis((704, 9004, "Acme printer")),
                    cis((704, 9004, "Acme printer")))
    # tickets 125: the device category is unproven
    routes += [{"method": "GET", "path": "/api/_/tickets/125/associated-cis",
                "json": {"data": [{"item_id": 705, "itam_cmdb_request_id": 9005, "category": "Hardware"}],
                         "meta": {"total_count": 1, "total_pages": 1}}}]
    routes += [{"method": "GET", "path": "/itam/core/device/api/", "query": {"q": "acme"},
                "json": {"data": [{"name": {"id": 702, "label": "Acme router"}, "type": "Router", "device_pk": 702}],
                         "total_rows": 1, "page": 1}},
               {"method": "GET", "path": "/itam/core/businessservice/api/",
                "json": {"data": [{"name": {"id": 801, "label": "Acme payroll"}, "businessservice_pk": 801}],
                         "total_rows": 4, "page": 2}},
               write("PUT", "/api/_/tickets/123/associate-ci", {}),
               write("PUT", "/api/_/problems/7/disassociate-ci", {}),
               write("PUT", "/api/_/changes/5/associate-ci", {}),
               CSRF]
    return routes


# --- sr approvals: the plan, the send baseline and the rebuild read the
# approval chains, then the readback.
APPROVAL_BEFORE_READS = 3


def chains(*groups):
    return {"approval_chains": [{"id": 1, "approval_groups": list(groups)}], "meta": {"has_next": False}}


def group(gid, name, status, *approvals, approval_type=1):
    return {"id": gid, "name": name, "approval_type": approval_type, "approval_status": status,
            "approvals": list(approvals)}


def approval(aid, member, status=(0, "requested"), **extra):
    return {"id": aid, "member_id": member, "status": {"id": status[0], "name": status[1]},
            "last_reminded_at": None, "remark": [], **extra}


def sr_approvals():
    base = group(40, "Acme CAB", 0, approval(401, 31), approval(402, 32), approval(403, 35, (1, "approved")))
    routes = []
    routes += reads("/api/_/tickets/130/approval-chains", chains(base), chains(base), APPROVAL_BEFORE_READS)
    # 131: request creates group 41
    routes += reads("/api/_/tickets/131/approval-chains", chains(base),
                    chains(base, group(41, "Acme release", 0, approval(411, 33), approval(412, 34))),
                    APPROVAL_BEFORE_READS)
    # 132: approve 401 with the remark
    approved = group(40, "Acme CAB", 0, approval(401, 31, (1, "approved"), remark=[{"status": 1, "data": "Acme ok"}]),
                     approval(402, 32), approval(403, 35, (1, "approved")))
    routes += reads("/api/_/tickets/132/approval-chains", chains(base), chains(approved), APPROVAL_BEFORE_READS)
    # 133: remind 402 accepted but last_reminded_at unchanged
    routes += reads("/api/_/tickets/133/approval-chains", chains(base), chains(base), APPROVAL_BEFORE_READS)
    routes += [fixed("/search/autocomplete/requesters", {"results": [
                   {"id": 34, "email": "bo@acme.com", "value": "Bo Kim"},
                   {"id": 36, "email": "old@acme.com", "value": "Old", "deactivated": True}]}),
               write("POST", "/api/_/tickets/131/approval-groups", {}),
               write("PUT", "/api/_/tickets/132/approvals/401", {}),
               write("PUT", "/api/_/tickets/133/approvals/402/resend", {}),
               CSRF]
    return routes


# --- solutions: create reads category/folder metadata for the plan and the
# rebuild, then the created article; links read the problem, its solutions
# and the article.
def problem(pid):
    return {"problem": {"id": pid, "human_display_id": f"PRB-{pid}", "subject": f"Acme problem {pid}", "status": 1}}


def article(aid, title, kind, folder=301):
    return {"article": {"id": aid, "title": title, "description": f"<div>{title}</div>", "status": 1,
                        "article_type": kind, "folder_id": folder}}


def solutions():
    routes = [fixed("/api/_/solutions/categories", {"categories": [
                  {"id": 201, "name": "Acme KB", "default_category": True},
                  {"id": 202, "name": "Globex KB", "default_category": False}], "meta": PAGE}),
              {"method": "GET", "path": "/api/_/solutions/folders", "query": {"category_id": "201"},
               "json": {"folders": [{"id": 301, "name": "Runbooks", "category_id": 201, "default_folder": True},
                                    {"id": 302, "name": "Archive", "category_id": 201, "default_folder": False}],
                        "meta": PAGE}},
              {"method": "GET", "path": "/api/_/solutions/folders", "query": {"category_id": "202"},
               "json": {"folders": [{"id": 303, "name": "Shared", "category_id": 202},
                                    {"id": 304, "name": "Shared 2", "category_id": 202}], "meta": PAGE}},
              write("POST", "/api/_/solutions/articles", {"article": {"id": 501}}),
              fixed("/api/_/solutions/articles/501", {"article": {
                  "id": 501, "title": "Acme VPN workaround", "description": "<div>Restart the Acme VPN client</div>",
                  "status": 1, "article_type": 2, "folder_id": 301}}),
              fixed("/api/_/solutions/articles/502", article(502, "Acme fix", 1)),
              fixed("/api/_/solutions/articles/503", article(503, "Acme FAQ", 3)),
              fixed("/api/_/solutions/articles/504", article(504, "Acme old fix", 2))]
    # problem 20: link 502 (readback has it); problem 21: unlink 504
    for pid in (20, 21):
        routes.append(fixed(f"/api/_/problems/{pid}", problem(pid)))
    routes += reads("/api/_/problems/20/solutions", {"articles": [], "meta": PAGE},
                    {"articles": [{"id": 502, "article_type": 1}], "meta": PAGE}, 2)
    routes += reads("/api/_/problems/21/solutions", {"articles": [{"id": 504, "article_type": 2}], "meta": PAGE},
                    {"articles": [{"id": 504, "article_type": 2}], "meta": PAGE}, 2)
    routes += [write("PUT", "/api/_/problems/20/solutions/502", {}),
               write("PUT", "/api/_/problems/21/solutions/504", {}),
               CSRF]
    return routes


# --- tickets bulk: the plan, the send-side rebuild and the execute rebuild
# read each ticket, then the readback.
BULK_BEFORE_READS = 3


def ticket(tid, **over):
    row = {"id": tid, "human_display_id": f"INC-{tid}", "subject": f"Acme ticket {tid}", "status": 2,
           "spam": False, "deleted": False}
    row.update(over)
    return {"ticket": row}


def ticket_bulk():
    routes = []
    for tid in (140, 141):
        routes.append(fixed(f"/api/_/tickets/{tid}", ticket(tid)))
    # 142/143 unspam: one readback still spam
    routes += reads("/api/_/tickets/142", ticket(142, spam=True), ticket(142, spam=False), BULK_BEFORE_READS)
    routes += reads("/api/_/tickets/143", ticket(143, spam=True), ticket(143, spam=True), BULK_BEFORE_READS)
    # 144 spam: readback spam
    routes += reads("/api/_/tickets/144", ticket(144), ticket(144, spam=True), BULK_BEFORE_READS)
    routes.append(fixed("/api/_/tickets/145", {"ticket": {"id": 145, "subject": "Acme no flags"}}))
    # fbc54d4 filtered selection: tickets 140/141 are Open (status 2)
    routes += [fixed("/api/_/ticket_form_fields", {"ticket_fields": [
                   {"name": "status", "label": "Status", "field_type": "default_status", "default_field": True,
                    "choices": [{"id": 2, "value": "Open"}, {"id": 3, "value": "Pending"}]}]}),
               fixed("/api/v2/ticket_form_fields", {}),
               fixed("/api/_/tickets", {"tickets": [{"id": 140, "subject": "Acme ticket 140", "status": 2},
                                                    {"id": 141, "subject": "Acme ticket 141", "status": 2}],
                                        "meta": PAGE})]
    routes += [write("POST", "/api/_/tickets/unspam_multiple", {}),
               write("POST", "/api/_/tickets/spam_multiple", {}),
               CSRF]
    return routes


# --- change relationships: the plan and the rebuild read the parent change
# and the inventory, then the readback.
REL_BEFORE_READS = 2


def change_relationships():
    def problems(*ids):
        return {"problems": [{"id": i, "subject": f"Acme problem {i}"} for i in ids], "meta": PAGE}
    def proxy(rows, key="projects"):
        return {"status": 200, "body": {key: rows, "meta": {"page": 1, "total_pages": 1, "total": len(rows)}}}
    routes = [fixed(f"/api/_/changes/{cid}", {"change": {"id": cid, "subject": f"Acme change {cid}"}})
              for cid in (50, 51, 52, 53)]
    # 50: add problem 71; 51: remove problem 72 but it stays
    routes += reads("/api/_/changes/50/problems", problems(70), problems(70, 71), REL_BEFORE_READS)
    routes += reads("/api/_/changes/51/problems", problems(72), problems(72), REL_BEFORE_READS)
    routes += [fixed("/api/_/problems/71", {"problem": {"id": 71}}),
               fixed("/api/_/problems/73", {"problem": {"id": 74}})]
    # 52: linked projects (proxy); 53: proxy refused
    routes += [{"method": "GET", "path": "/api/_/freshrelease/settings/fetch_linked_fr_projects",
                "query": {"placeholder[resourceId]": "52"},
                "json": proxy([{"id": 9, "key": "ACME", "name": "Acme rollout"}])},
               {"method": "GET", "path": "/api/_/freshrelease/settings/linked_issues",
                "query": {"placeholder[resourceId]": "52"},
                "json": proxy([{"id": 91, "key": "ACME-1", "name": "Acme step 1", "project_id": 9}], "issues")},
               {"method": "GET", "path": "/api/_/freshrelease/settings/fetch_linked_fr_projects",
                "query": {"placeholder[resourceId]": "53"}, "json": {"status": 500, "body": {}}}]
    routes += [write("PUT", "/api/_/changes/50/problems/associate", {}),
               write("DELETE", "/api/_/changes/51/problems/72", {}),
               CSRF]
    return routes


# --- 9070409 ticket task owner/group: same reads as the c46d3b0 tasks.
def ticket_tasks_assigned():
    from gen_reviewed_routes import TASK_BEFORE_READS, TASK_FIELDS as TICKET_TASK_FIELDS, task
    fields = TICKET_TASK_FIELDS + [{"name": "owner_id", "label": "Owner", "field_type": "default_owner"},
                                   {"name": "group_id", "label": "Group", "field_type": "default_group"}]
    def inv(*rows):
        return {"tasks": list(rows), "meta": PAGE}
    added = task(911, "Acme handover", 1, owner_id=133, group_id=5)
    routes = []
    routes += reads("/api/_/tickets/127/tasks", inv(task(910, "Acme check", 1)),
                    inv(task(910, "Acme check", 1), added), TASK_BEFORE_READS)
    routes += reads("/api/_/tickets/128/tasks", inv(task(912, "Acme old", 1)),
                    inv(task(912, "Acme new", 1, owner_id=133)), TASK_BEFORE_READS)
    routes += [fixed("/api/_/ticket_task_fields", {"ticket_task_fields": fields}),
               fixed("/search/autocomplete/agents", {"results": [
                   {"id": "bo@acme.com", "user_id": 133, "email": "bo@acme.com", "value": "Bo Kim"}]}),
               fixed("/api/_/bootstrap/agents_groups", {"groups": [{"id": 4, "name": "Ops"}, {"id": 5, "name": "DBA"}]}),
               write("POST", "/api/_/tickets/127/tasks", {"task": added}),
               write("PUT", "/api/_/tickets/128/tasks/912", {"task": task(912, "Acme new", 1, owner_id=133)}),
               CSRF]
    return routes


# --- 2b0c1a4/6ead7c9 attachments: files come from fixtures/files/attach
# (notes.txt is 11 bytes, log.txt 14). The agent page publishes 1 MiB.
# Problem notes: the plan and the rebuild read the notes, then the
# readback; deletes read the owner for the plan, the send baseline and the
# rebuild, then the readback.
ATTACH_NOTE_BEFORE_READS = 2
ATTACH_DELETE_BEFORE_READS = 3


def dashboard(limit):
    script = f"<script>window.MAX_ATTACHMENT_LIMIT = {limit};</script>" if limit else ""
    return {"method": "GET", "path": "/a/dashboard",
            "text": f'<html><head><meta name="csrf-token" content="tok">{script}</head></html>'}


def uploads(*ids):
    return [{"method": "POST", "path": "/api/_/attachments", "status": 200, "json": {"id": i}, "once": True}
            for i in ids]


def attachments(limit="1"):
    notes = [{"id": 11, "name": "notes.txt", "size": 11}]
    routes = [dashboard(limit),
              fixed("/api/_/ticket_form_fields", {"ticket_fields": [{"name": "subject", "label": "Subject"}]}),
              fixed("/api/_/problem_form_fields", {"problem_fields": [{"name": "subject", "label": "Subject"}]}),
              fixed("/api/_/tickets/5", {"ticket": {"id": 5, "subject": "Acme VPN", "attachments": [
                  {"id": 61, "name": "old.txt", "size": 3}]}}),
              fixed("/api/_/tickets/5/conversations", {"conversations": [
                  {"id": 950, "body": "<div>see attached</div>", "private": False, "from_email": "support@acme.com",
                   "attachments": notes},
                  {"id": 951, "body": "<div>noted</div>", "private": True, "attachments": notes}], "meta": PAGE}),
              fixed("/api/_/problems/7", {"problem": {"id": 7, "subject": "Acme outage"}}),
              fixed("/api/_/changes/5", {"change": {"id": 5, "subject": "Acme patch", "attachments": []}})]
    routes += reads("/api/_/problems/7/notes", {"notes": [], "meta": PAGE},
                    {"notes": [{"id": 77, "body": "<div>root cause</div>", "attachments": notes}], "meta": PAGE},
                    ATTACH_NOTE_BEFORE_READS)
    # ticket 6: attachment 62 deleted; ticket 8: 63 stays after the delete
    routes += reads("/api/_/tickets/6", {"ticket": {"id": 6, "attachments": [{"id": 62, "name": "a.txt", "size": 1}]}},
                    {"ticket": {"id": 6, "attachments": []}}, ATTACH_DELETE_BEFORE_READS)
    routes += reads("/api/_/tickets/8", {"ticket": {"id": 8, "attachments": [{"id": 63, "name": "b.txt", "size": 1}]}},
                    {"ticket": {"id": 8, "attachments": [{"id": 63, "name": "b.txt", "size": 1}]}},
                    ATTACH_DELETE_BEFORE_READS)
    routes += uploads(11, 12)
    routes += [write("POST", "/api/_/tickets/5/reply", {"conversation": {"id": 950}}),
               write("POST", "/api/_/tickets/5/notes", {"conversation": {"id": 951}}),
               write("POST", "/api/_/problems/7/notes", {"note": {"id": 77}}),
               write("DELETE", "/api/_/attachments/62", {}),
               write("DELETE", "/api/_/attachments/63", {})]
    return routes


def change_create_attached(limit="1"):
    base = [r for r in json.loads((ROUTES / "change_create.json").read_text())
            if r["path"] not in ("/a/dashboard", "/api/_/changes/900")]
    created = {"change": {"id": 900, "display_id": 900, "subject": "Hello", "requester_id": 51,
                          "attachments": [{"id": 11, "name": "notes.txt", "size": 11}]}}
    return base + [dashboard(limit), fixed("/api/_/changes/900", created),
                   {"method": "GET", "path": "/api/_/changes/900/associated-cis", "json": cis((702, 9002, "Acme router"))}
                   ] + uploads(11)


# --- 51cd544 change/problem time: the plan and the rebuild read the v2
# entries, then the readback.
TIME_BEFORE_READS = 2


def module_time():
    def entries(*rows):
        return {"time_entries": list(rows), "meta": PAGE}
    def entry(eid, spent="01:30:00", note="Acme work", agent=9, **extra):
        return {"id": eid, "agent_id": agent, "billable": True, "timer_running": False, "time_spent": spent,
                "note": note, "executed_at": "2030-01-01T09:00:00Z", **extra}
    created = entry(71, "00:45:00", "Acme deploy", executed_at="2030-01-02T02:00:00Z")
    routes = [fixed("/api/v2/agents/me", {"agent": {"id": 9, "first_name": "Ann", "last_name": "Lee"}})]
    routes += [fixed(f"/api/_/changes/{cid}", {"change": {"id": cid, "subject": f"Acme change {cid}"}})
               for cid in (60, 61, 62)]
    routes += [fixed(f"/api/_/problems/{pid}", {"problem": {"id": pid, "subject": f"Acme problem {pid}"}})
               for pid in (63, 64)]
    # 60: create 71; 61: update 72 (readback keeps the old note); 62: delete 73
    routes += reads("/api/v2/changes/60/time_entries", entries(entry(70)), entries(entry(70), created),
                    TIME_BEFORE_READS)
    routes += reads("/api/v2/changes/61/time_entries", entries(entry(72), entry(74, agent=10)),
                    entries(entry(72), entry(74, agent=10)), TIME_BEFORE_READS)
    routes += reads("/api/v2/changes/62/time_entries", entries(entry(73)), entries(), TIME_BEFORE_READS)
    # problems 63: list; 64: delete 76
    routes += reads("/api/v2/problems/63/time_entries", entries(entry(75)), entries(entry(75)), TIME_BEFORE_READS)
    routes += reads("/api/v2/problems/64/time_entries", entries(entry(76)), entries(), TIME_BEFORE_READS)
    routes += [write("POST", "/api/_/changes/60/time_entries", {"time_entry": {"id": 71}}),
               write("PUT", "/api/_/changes/61/time_entries/72", {"time_entry": {"id": 72}}),
               write("DELETE", "/api/_/changes/62/time_entries/73", {}),
               write("DELETE", "/api/_/problems/64/time_entries/76", {}),
               CSRF]
    return routes


# --- 51cd544 problem task writes: an update reads the exact task for the
# plan and the rebuild, then the readback; a delete reads the inventory for
# the plan and the rebuild, then the readback.
PTASK_BEFORE_READS = 2

PROBLEM_TASK_FIELDS = [
    {"name": "status", "label": "Status", "choices": [
        {"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"}, {"id": 3, "value": "Completed"}]},
    {"name": "notify_before", "label": "Notify before", "choices": [
        {"id": 0, "value": "Never"}, {"id": 900, "value": "15 Minutes"}]},
]


def problem_tasks():
    def ptask(tid, pid, title="Acme fix", status=1, **extra):
        return {"id": tid, "title": title, "status": status, "taskable_display_id": pid,
                "taskable_type": "Itil::Problem", "owner_id": 131, "group_id": 4, "notify_before": 0,
                "planned_start_date": "2030-01-01T09:00:00Z", "planned_end_date": "2030-01-01T10:00:00Z",
                "planned_effort": "1h", "description": "<div>Acme</div>", "custom_fields": {}, **extra}
    def inv(*rows):
        return {"tasks": list(rows), "meta": PAGE}
    routes = [fixed("/api/_/problem_task_fields", {"problem_task_fields": PROBLEM_TASK_FIELDS}),
              fixed("/search/autocomplete/agents", {"results": [
                  {"id": "bo@acme.com", "user_id": 133, "email": "bo@acme.com", "value": "Bo Kim"}]}),
              fixed("/api/_/bootstrap/agents_groups", {"groups": [{"id": 4, "name": "Ops"}, {"id": 5, "name": "DBA"}]})]
    routes += [fixed(f"/api/_/problems/{pid}", {"problem": {"id": pid, "subject": f"Acme problem {pid}"}})
               for pid in (80, 81, 82, 83)]
    # 80: update title/owner (verified); 81: complete (readback keeps status 1)
    routes += [fixed("/api/_/problems/80/tasks", inv(ptask(801, 80)))]
    routes += reads("/api/_/problems/80/tasks/801", {"task": ptask(801, 80)},
                    {"task": ptask(801, 80, "Acme fix v2", owner_id=133)}, PTASK_BEFORE_READS)
    routes += [fixed("/api/_/problems/81/tasks", inv(ptask(811, 81))),
               fixed("/api/_/problems/81/tasks/811", {"task": ptask(811, 81)})]
    # 82: delete 821; 83: task 831 belongs to another parent
    routes += reads("/api/_/problems/82/tasks", inv(ptask(821, 82), ptask(822, 82)), inv(ptask(822, 82)),
                    PTASK_BEFORE_READS)
    routes += [fixed("/api/_/problems/82/tasks/821", {"task": ptask(821, 82)}),
               fixed("/api/_/problems/83/tasks", inv(ptask(831, 83))),
               fixed("/api/_/problems/83/tasks/831", {"task": ptask(831, 99)})]
    routes += [write("PUT", "/api/_/problems/80/tasks/801", {"task": {"id": 801}}),
               write("PUT", "/api/_/problems/81/tasks/811", {"task": {"id": 811}}),
               write("PUT", "/api/_/problems/82/tasks/update", {}),
               CSRF]
    return routes


# --- 6c6b767 SR requested items: the plan and the rebuild read the item
# inventory, then the readback.
ITEM_BEFORE_READS = 2


def sr_items():
    def item(iid, tid, stage=1, parent=False):
        return {"id": iid, "ticket_id": tid, "is_parent": parent, "stage": {"id": stage, "name": f"Stage {stage}"},
                "item": {"id": 4400001, "display_id": 77, "name": "Feature Request"}, "quantity": 1}
    def inv(*rows):
        return {"requested_items": list(rows)}
    routes = [fixed(f"/api/_/tickets/{tid}", {"ticket": {"id": tid, "ticket_type": "Service Request",
                                                       "subject": f"Acme request {tid}"}})
              for tid in (150, 151, 152)]
    routes.append(fixed("/api/_/tickets/153", {"ticket": {"id": 153, "ticket_type": "Incident"}}))
    routes += [fixed("/api/_/tickets/150/requested_items", inv(item(1501, 150, parent=True), item(1502, 150)))]
    # 151: stage 1511 to Delivered; 152: remove 1522 but it stays
    routes += reads("/api/_/tickets/151/requested_items", inv(item(1511, 151)), inv(item(1511, 151, stage=2)),
                    ITEM_BEFORE_READS)
    routes += reads("/api/_/tickets/152/requested_items", inv(item(1521, 152, parent=True), item(1522, 152)),
                    inv(item(1521, 152, parent=True), item(1522, 152)), ITEM_BEFORE_READS)
    routes += [write("PATCH", "/api/_/tickets/151/requested_items/1511/update_stage", {}),
               write("DELETE", "/api/_/tickets/152/requested_items/1522", {}),
               CSRF]
    return routes


def ticket_create_attached():
    base = [r for r in json.loads((ROUTES / "ticket_create.json").read_text()) if r["path"] != "/a/dashboard"]
    created = {"ticket": {"id": 777, "display_id": 777, "subject": "Hello", "priority": 2, "requester_id": 51,
                          "attachments": [{"id": 11, "name": "notes.txt", "size": 11}]}}
    return base + [dashboard("1"), fixed("/api/_/tickets/777", created)] + uploads(11)


def main():
    for name, routes in [("attachments", attachments()),
                         ("ticket_create_attached", ticket_create_attached()),
                         ("module_time", module_time()),
                         ("problem_tasks", problem_tasks()),
                         ("sr_items", sr_items()),
                         ("attachments_unpublished", attachments(None)),
                         ("change_create_attached", change_create_attached()),
                         ("ticket_tasks_assigned", ticket_tasks_assigned()),
                         ("change_task_review", change_tasks()),
                         ("integrated_assets", integrated_assets()),
                         ("sr_approvals", sr_approvals()),
                         ("solutions", solutions()),
                         ("ticket_bulk", ticket_bulk()),
                         ("change_relationships", change_relationships())]:
        (ROUTES / f"{name}.json").write_text(json.dumps(routes, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
