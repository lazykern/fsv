"""Write the route fixtures for the reviewed ticket commands of Python
9ab0eaa (`tickets status/resolve/close/reopen`), f402e61 (`tickets merge`)
and cc41912 (`tickets assign/pick-up`):

    python3 crates/fsv-cli/tests/golden/gen_ticket_routes.py

Each ticket id is one scenario. Reads before a write are `once` routes in
call order; the last read is the record after the write.
"""
import copy
import json
import pathlib

ROUTES = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "routes"


def field(fid, name, label, field_type, position, *, choices=(), default=None, closure=False):
    return {"id": fid, "name": name, "label": label, "description": "", "field_type": field_type,
            "required": False, "required_for_closure": closure, "position": position,
            "default_field": field_type.startswith("default_") if default is None else default,
            "choices": list(choices), "nested_fields": [], "placeholder": "", "helptext": "", "hint": False,
            "content": None, "displayed_to_agents": True, "data_source": 0, "lookup_config": None,
            "workspace_id": 1, "updated_at": "2030-01-02T03:04:05Z"}


TICKET_FIELDS = [
    field(9000007101, "status", "Status", "default_status", 1, choices=[
        {"id": 31, "value": "Open"}, {"id": 47, "value": "Pending"},
        {"id": 62, "value": "Resolved"}, {"id": 89, "value": "Closed"}]),
    field(9000007102, "summary", "Resolution Summary", "custom_text", 2, closure=True),
    field(9000007103, "reason", "Pending Reason", "custom_text", 3),
]

CSRF = {"method": "GET", "path": "/a/dashboard",
        "text": '<html><head><meta name="csrf-token" content="synthetic-token"></head></html>'}


def ticket(tid, **over):
    row = {"id": tid, "human_display_id": f"INC-{tid}", "subject": "Acme VPN", "status": 31,
           "group_id": None, "responder_id": 9000001234, "resolution_notes": None,
           "custom_fields": {"summary": "Acme fix", "reason": "", "untouched": "keep"}}
    row.update(copy.deepcopy(over))
    return row


def get(tid, row, once=True):
    r = {"method": "GET", "path": f"/api/_/tickets/{tid}", "json": {"ticket": row}}
    if once:
        r["once"] = True
    return r


def put(tid, row):
    return {"method": "PUT", "path": f"/api/_/tickets/{tid}", "json": {"ticket": row}}


def note(html):
    return {"notes_html": html, "user": {"name": "Acme Operator", "email": "operator@example.com"}}


def lifecycle():
    noted = "<div>Acme fix &lt;safe&gt;</div>"
    open_row = ticket(123)
    with_note = ticket(123, resolution_notes=note(noted))
    resolved = ticket(123, status=62, resolution_notes=note(noted))
    closed_from = ticket(124, status=62, resolution_notes=note("<div>Acme fix</div>"))
    closed = ticket(124, status=89, resolution_notes=note("<div>Acme fix</div>"))
    reopen_from = ticket(127, status=62, resolution_notes=note("<div>Acme fix</div>"))
    reopened = ticket(127, status=31, resolution_notes=note("<div>Acme fix</div>"))
    routes = [
        # resolve: plan, send context, baseline, after note, fresh context, after status
        get(123, open_row), get(123, open_row), get(123, open_row),
        get(123, with_note), get(123, with_note), get(123, resolved, once=False),
        put(123, resolved),
        # close: plan, send context, after status
        get(124, closed_from), get(124, closed_from), get(124, closed, once=False), put(124, closed),
        # no persisted resolution note
        get(125, ticket(125, status=62), once=False),
        # closure-required field empty
        get(126, ticket(126, status=62, resolution_notes=note("<div>Acme fix</div>"),
                        custom_fields={"summary": "", "reason": "", "untouched": "keep"}), once=False),
        # reopen: plan, send context, after status
        get(127, reopen_from), get(127, reopen_from), get(127, reopened, once=False), put(127, reopened),
        # transition write accepted but readback keeps the old status
        get(128, ticket(128, status=62, resolution_notes=note("<div>Acme fix</div>")), once=False),
        put(128, ticket(128)),
        {"method": "GET", "path": "/api/_/ticket_form_fields", "json": {"ticket_fields": TICKET_FIELDS}},
        {"method": "GET", "path": "/api/_/business_rules", "json": {"business_rules": []}},
        CSRF,
    ]
    return routes


def reads(tid, before, after, count, key="ticket", suffix=""):
    """`count` once-reads of `before`, then `after` for good."""
    path = f"/api/_/tickets/{tid}{suffix}"
    return ([{"method": "GET", "path": path, "json": {key: before} if key else before, "once": True}] * count
            + [{"method": "GET", "path": path, "json": {key: after} if key else after}])


def history(rows):
    return {"conversations": rows, "meta": {"has_next": False}}


MERGE_FIELDS = [field(9000007101, "status", "Status", "default_status", 1, choices=[
    {"id": 31, "value": "Open"}, {"id": 89, "value": "Closed"}])]


def merge(post):
    """123 survives; 124 and 125 merge into it. `post` is the merge route;
    `primary_note` false leaves the primary note out of the readback."""
    def rec(tid, status=31):
        return {"id": tid, "human_display_id": f"INC-{tid}", "subject": f"Acme {tid}", "status": status,
                "group_id": None, "responder_id": None}
    hist = {123: [{"id": 10, "body": "earlier"}], 124: [{"id": 11, "body": "source A"}],
            125: [{"id": 12, "body": "source B"}]}
    moved = hist[123] + hist[124] + hist[125]
    primary_after = moved + ([{"id": 20, "body": "<p>Primary note</p>", "private": True}] if post.get("_note", True) else [])
    routes = []
    # build_plan reads every record and history once; send rebuilds, then verifies.
    for tid in (123, 124, 125):
        after = rec(tid) if tid == 123 else rec(tid, 89)
        routes += reads(tid, rec(tid), after, 2)
        routes += reads(tid, history(hist[tid]),
                        history(primary_after if tid == 123 else
                                [{"id": 100 + tid, "body": "<div>Secondary note</div>", "private": True}]),
                        2, key=None, suffix="/conversations")
    routes += reads(126, rec(126), rec(126), 0)
    routes += [{k: v for k, v in post.items() if not k.startswith("_")},
               {"method": "GET", "path": "/api/_/ticket_form_fields", "json": {"ticket_fields": MERGE_FIELDS}},
               {"method": "GET", "path": "/api/_/business_rules", "json": {"business_rules": []}},
               CSRF]
    return routes


# The plan reads each record twice (routing check, then `ticket_properties`);
# send builds once more, then reads the record back.
ASSIGN_BEFORE_READS = 4

MERGED = {"method": "POST", "path": "/api/_/merge_tickets/complete_merge",
          "json": {"count": 2, "target": {"ticket_id": 123}, "source": {"ticket_ids": [124, 125]}}}

ASSIGN_FIELDS = [field(9000007100, "group", "Group", "default_group", 1),
                 field(9000007101, "agent", "Agent", "default_agent", 2)]
BOOT = {"agents": [{"id": 71, "name": "Acme Agent", "email": "agent@example.com", "group_ids": [81], "deleted": False},
                   {"id": 72, "name": "Globex Agent", "group_ids": [], "deleted": False}],
        "groups": [{"id": 81, "name": "Acme Desk", "agent_ids": [71]},
                   {"id": 82, "name": "Globex Desk", "agent_ids": []}]}


def assignment(before_reads):
    def rec(tid, responder, group):
        return {"id": tid, "human_display_id": f"INC-{tid}", "subject": "Acme VPN", "status": 2,
                "responder_id": responder, "group_id": group, "custom_fields": {}}
    routes = []
    routes += reads(123, rec(123, 71, 81), rec(123, None, 81), before_reads)
    routes += reads(124, rec(124, None, 81), rec(124, 71, 81), before_reads)
    routes += reads(125, rec(125, None, None), rec(125, 71, None), before_reads)
    routes += reads(126, rec(126, None, 81), rec(126, None, 81), before_reads)
    routes += [put(123, rec(123, None, 81)), put(124, rec(124, 71, 81)), put(126, rec(126, None, 81)),
               {"method": "PUT", "path": "/api/_/tickets/pick_up", "status": 204},
               {"method": "GET", "path": "/api/_/bootstrap/agents_groups", "json": BOOT},
               {"method": "GET", "path": "/api/v2/agents/me", "json": {"agent": {"id": 71}}},
               {"method": "GET", "path": "/api/_/ticket_form_fields", "json": {"ticket_fields": ASSIGN_FIELDS}},
               {"method": "GET", "path": "/api/_/business_rules", "json": {"business_rules": []}},
               CSRF]
    return routes


def main():
    for name, routes in [("ticket_lifecycle", lifecycle()),
                         ("ticket_merge", merge(MERGED)),
                         ("ticket_merge_unverified", merge({**MERGED, "_note": False})),
                         ("ticket_merge_500", merge({**MERGED, "status": 500,
                                                     "json": {"message": "synthetic failure"}})),
                         ("ticket_assign", assignment(ASSIGN_BEFORE_READS))]:
        (ROUTES / f"{name}.json").write_text(json.dumps(routes, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
