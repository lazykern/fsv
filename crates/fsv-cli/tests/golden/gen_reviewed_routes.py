"""Write the route fixtures for the reviewed commands of Python 9910ddc
(`tickets note-edit/note-delete`), 17ada1b (`problems link-incident/
unlink-incident`), c46d3b0 (`tickets tasks list/add/status/delete`) and
5d7d777 (`problems status`):

    python3 crates/fsv-cli/tests/golden/gen_reviewed_routes.py

Each target id is one scenario. A read that changes with the write serves
`before` for the reads up to the send (`once` routes), then `after`.
"""
import json
import pathlib

from gen_ticket_routes import CSRF, field

ROUTES = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "routes"
PAGE = {"has_next": False}


def reads(path, before, after, count):
    return ([{"method": "GET", "path": path, "json": before, "once": True}] * count
            + [{"method": "GET", "path": path, "json": after}])


def fixed(path, body):
    return {"method": "GET", "path": path, "json": body}


def write(method, path, body=None, status=200):
    return {"method": method, "path": path, "status": status, "json": {} if body is None else body}


# --- 9910ddc note edit/delete: dry run reads history twice (plan, review);
# send reads it for the plan, the rebuild, then the readback.
NOTE_BEFORE_READS = 2


def note(nid, body, *, source=2, private=True):
    return {"id": nid, "body": body, "body_text": body, "source": source, "private": private,
            "user_id": 9000001234, "created_at": "2030-01-01T00:00:00Z"}


def notes():
    def history(rows):
        return {"conversations": rows, "meta": PAGE}
    first = note(500, "<div>Acme opening</div>", source=0, private=False)
    routes = []
    for tid, before, after in [
        (123, [first, note(501, "<div>Acme draft</div>")], [first, note(501, "<div>Globex final</div>")]),
        (124, [first, note(502, "<div>Acme stale</div>")], [first]),
        (125, [first], [first]),
        (126, [first, note(503, "<div>Acme draft</div>")], [first, note(503, "<div>Acme draft</div>")]),
    ]:
        routes += reads(f"/api/_/tickets/{tid}/conversations", history(before), history(after), NOTE_BEFORE_READS)
    routes += [write("PUT", "/api/_/conversations/501", {"conversation": note(501, "<div>Globex final</div>")}),
               write("DELETE", "/api/_/conversations/502"),
               write("PUT", "/api/_/conversations/503", {"conversation": note(503, "<div>Acme draft</div>")}),
               CSRF]
    return routes


# --- 17ada1b problem incidents: the inventory is read for the plan, the
# rebuild, then the readback.
INCIDENT_BEFORE_READS = 2


def incidents():
    def inc(tid, kind="Incident"):
        prefix = "INC" if kind == "Incident" else "SR"
        return {"ticket": {"id": tid, "type": kind, "human_display_id": f"{prefix}-{tid}", "subject": f"Acme {tid}"}}
    def problem(pid):
        return {"problem": {"id": pid, "human_display_id": f"PRB-{pid}", "subject": f"Acme problem {pid}"}}
    def tickets(*ids):
        return {"tickets": [{"id": i, "subject": f"Acme {i}"} for i in ids], "meta": PAGE}
    routes = [fixed(f"/api/_/problems/{p}", problem(p)) for p in (7, 8, 9)]
    routes += [fixed(f"/api/_/tickets/{t}", inc(t)) for t in (123, 124)]
    routes.append(fixed("/api/_/tickets/125", inc(125, "Service Request")))
    routes += reads("/api/_/problems/7/tickets", tickets(), tickets(123), INCIDENT_BEFORE_READS)
    routes += reads("/api/_/problems/8/tickets", tickets(124), tickets(), INCIDENT_BEFORE_READS)
    # 9: the association is accepted but the readback still lacks it
    routes += reads("/api/_/problems/9/tickets", tickets(), tickets(), INCIDENT_BEFORE_READS)
    routes += [write("PUT", "/api/_/problems/7/tickets/associate"),
               write("DELETE", "/api/_/problems/8/tickets/124"),
               write("PUT", "/api/_/problems/9/tickets/associate"),
               CSRF]
    return routes


# --- c46d3b0 ticket tasks: send reads the inventory for the plan, the
# pre-send snapshot, the rebuild, then the readback.
TASK_BEFORE_READS = 3

TASK_FIELDS = [
    {"name": "title", "label": "Title", "field_type": "default_title", "required": True},
    {"name": "status", "label": "Status", "field_type": "default_status", "choices": [
        {"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"}, {"id": 3, "value": "Completed"}]},
    {"name": "notify_before", "label": "Notify Before", "field_type": "default_notify_before", "choices": [
        {"id": 0, "value": "Never"}, {"id": 900, "value": "15 minutes"}]},
    {"name": "cf_area", "label": "Area", "field_type": "custom_text", "custom": True},
    {"name": "cf_tier", "label": "Tier", "field_type": "custom_dropdown", "custom": True, "choices": [
        {"id": 1, "value": "Gold"}, {"id": 2, "value": "Silver"}]},
]


def task(tid, title, status, **extra):
    return {"id": tid, "title": title, "status": status, "notify_before": 0, "custom_fields": {}, **extra}


def tasks():
    def inv(*rows):
        return {"tasks": list(rows), "meta": PAGE}
    added = task(902, "Acme rollback check", 1, custom_fields={"cf_area": "Network"})
    routes = []
    routes += reads("/api/_/tickets/123/tasks", inv(task(901, "Acme check", 1)),
                    inv(task(901, "Acme check", 1), added), TASK_BEFORE_READS)
    routes += reads("/api/_/tickets/124/tasks", inv(task(903, "Acme verify", 1)),
                    inv(task(903, "Acme verify", 3)), TASK_BEFORE_READS)
    routes += reads("/api/_/tickets/125/tasks", inv(task(904, "Acme cleanup", 1), task(905, "Acme notify", 1)),
                    inv(task(905, "Acme notify", 1)), TASK_BEFORE_READS)
    # 126: status change accepted but the readback keeps the old status
    routes += reads("/api/_/tickets/126/tasks", inv(task(906, "Acme verify", 1)),
                    inv(task(906, "Acme verify", 1)), TASK_BEFORE_READS)
    routes += [fixed("/api/_/ticket_task_fields", {"ticket_task_fields": TASK_FIELDS}),
               write("POST", "/api/_/tickets/123/tasks", {"task": added}),
               write("PUT", "/api/_/tickets/124/tasks/903", {"task": task(903, "Acme verify", 3)}),
               write("PUT", "/api/_/tickets/125/tasks/update", {}),
               write("PUT", "/api/_/tickets/126/tasks/906", {"task": task(906, "Acme verify", 1)}),
               CSRF]
    return routes


# --- 5d7d777 problem status: send reads the record for the plan, the
# rebuild, then the readback.
PROBLEM_BEFORE_READS = 2

PROBLEM_FIELDS = [
    field(9000009001, "subject", "Subject", "default_subject", 1),
    field(9000009002, "status", "Status", "default_status", 2, choices=[
        {"id": 1, "value": "Open"}, {"id": 2, "value": "Change Requested"}, {"id": 3, "value": "Closed"}]),
    field(9000009003, "cf_owner_note", "Owner Note", "custom_text", 3, closure=True),
]


def problems(rejected=False):
    def prb(pid, status, **extra):
        row = {"id": pid, "human_display_id": f"PRB-{pid}", "subject": f"Acme problem {pid}", "status": status,
               "planned_start_date": None, "planned_end_date": None,
               "custom_fields": {"cf_owner_note": "Acme owner"}}
        row.update(extra)
        return {"problem": row}
    def analysis(filled):
        rows = [{"name": key, "description": f"<p>Acme {key}</p>" if filled else ""}
                for key in ("problem_cause", "problem_impact", "problem_symptom")]
        return {"problem_planning_fields": rows}
    routes = []
    # 7 open -> change requested; server fills planned dates
    routes += reads("/api/_/problems/7", prb(7, 1),
                    prb(7, 2, planned_start_date="2030-01-02T00:00:00Z", planned_end_date="2030-01-03T00:00:00Z"),
                    PROBLEM_BEFORE_READS)
    # 8 closable: analysis filled, one solution
    routes += reads("/api/_/problems/8", prb(8, 2), prb(8, 3), PROBLEM_BEFORE_READS)
    routes += [fixed("/api/_/problems/8/planning-fields", analysis(True)),
               fixed("/api/_/problems/8/solutions", {"solutions": [{"id": 61, "title": "Acme workaround"}], "meta": PAGE})]
    # 9 analysis empty, no solution
    routes += [fixed("/api/_/problems/9", prb(9, 2)),
               fixed("/api/_/problems/9/planning-fields", analysis(False)),
               fixed("/api/_/problems/9/solutions", {"solutions": [], "meta": PAGE})]
    # 10 closure-required field empty
    routes += [fixed("/api/_/problems/10", prb(10, 2, custom_fields={"cf_owner_note": ""})),
               fixed("/api/_/problems/10/planning-fields", analysis(True)),
               fixed("/api/_/problems/10/solutions", {"solutions": [{"id": 62, "title": "Acme fix"}], "meta": PAGE})]
    # 11 transition accepted but the readback keeps the old status
    routes += reads("/api/_/problems/11", prb(11, 1), prb(11, 1), PROBLEM_BEFORE_READS)
    closure_rejection = {"errors": [{"code": "violated_closure_rules", "description": {
        "check_analysis_completed": True, "check_workaround_or_solution_added": True}}]}
    routes += [write("PUT", "/api/_/problems/7", prb(7, 2)),
               write("PUT", "/api/_/problems/8", closure_rejection, 422) if rejected else write("PUT", "/api/_/problems/8", prb(8, 3)),
               write("PUT", "/api/_/problems/11", prb(11, 1)),
               fixed("/api/_/problem_form_fields", {"problem_fields": PROBLEM_FIELDS}),
               fixed("/api/_/business_rules", {"business_rules": []}),
               CSRF]
    return routes


def main():
    for name, routes in [("ticket_note_mutation", notes()),
                         ("problem_incidents", incidents()),
                         ("ticket_tasks_reviewed", tasks()),
                         ("problem_status", problems()),
                         ("problem_status_rejected", problems(rejected=True))]:
        (ROUTES / f"{name}.json").write_text(json.dumps(routes, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
