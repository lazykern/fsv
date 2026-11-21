from common import Fake, rec, dump, OUT
from fsv import service
from fsv.resources import CHANGES, PROBLEMS, TICKETS, Resource


def clean(x):
    if isinstance(x, Resource):
        return x.name
    if isinstance(x, dict):
        return {k: clean(v) for k, v in x.items()}
    if isinstance(x, (list, tuple)):
        return [clean(v) for v in x]
    return x


def calls(f):
    return [[c["method"], c["path"], c["params"]] for c in f.calls]


# list_page
f = Fake({"GET changes": {"changes": [{"id": 1}], "meta": {"total_count": 1}}})
rec("list_page", lambda: clean([*service.list_page(CHANGES, client=f, filter_name="mine", order_by="created_at", order_type="")]) + [calls(f)])
f = Fake({"GET tickets": ["not a dict"]})
rec("list_page_nondict", lambda: clean([*service.list_page(TICKETS, client=f)]))
# _list_params
rec("list_params", lambda: service._list_params(TICKETS, 2, 10, "f", "updated_at", None, "[q]", False))
# list_items
f = Fake({"GET problems": {"problems": [{"id": 1}, {"id": 2}], "meta": {"total_count": 0}, "total": 7}})
rec("list_items", lambda: clean([*service.list_items(PROBLEMS, client=f, query_hash="[x]", or_grouping=True)]) + [calls(f)])
f = Fake({"GET changes": {"changes": [{"id": 1}]}})
rec("list_items_len", lambda: clean([*service.list_items(CHANGES, client=f, order_by="id", order_type="asc")]) + [calls(f)])
# _normalize_search_result
rec("normalize", lambda: clean(service._normalize_search_result(
    {"ticket_path": "/a/tickets/42", "ticket_display_id": "INC-42", "subject": "<b>Disk</b> &amp; CPU",
     "ticket_status": "Open", "responder_name": "Ann", "ticket_group": "Ops"}, "tickets")))
rec("normalize_change", lambda: clean(service._normalize_search_result(
    {"itil_module_path": "/a/changes/7", "itil_module_display_id": "CHN-7", "subject": "Patch",
     "itil_module_priority": "High", "owner_name": "Bo", "itil_module_group": "Net", "created_at": "2026-01-01"}, "changes")))
rec("normalize_odd", lambda: clean(service._normalize_search_result({"itil_module_path": "/x"}, "changes")))
rec("normalize_unknown", lambda: service._normalize_search_result({}, "assets"))
# search_items
f = Fake({"GET search/changes": {"results": [{"itil_module_path": "/a/changes/7", "subject": "x"}], "total_entries": 9}})
rec("search_one", lambda: clean([*service.search_items("x", entity="changes", client=f)]))
f = Fake({"GET search/tickets": {"results": [{"ticket_path": "/a/tickets/1"}]},
          "GET search/problems": ("status", 500),
          "GET search/changes": {"results": []}})
rec("search_all", lambda: clean([*service.search_items("x", client=f)]))
# list_work_items
f = Fake({"GET tickets": {"tickets": [{"id": 1, "updated_at": "2026-01-02"}]},
          "GET changes": {"changes": [{"id": 2, "updated_at": "2026-01-03"}, {"id": 3}]},
          "GET problems": ("status", 500)})
rec("work_items", lambda: clean([*service.list_work_items(client=f)]))
# get_item
f = Fake({"GET tickets/5": {"ticket": {"id": 5}}, "GET changes/6": {"id": 6}, "GET problems/7": {"problem": {"id": 7}}})
rec("get_item", lambda: clean([service.get_item(TICKETS, 5, client=f), service.get_item(CHANGES, 6, client=f),
                               service.get_item(PROBLEMS, 7, client=f, includes="")]) + [calls(f)])
# activities/notes/tasks/approvals
f = Fake({"GET changes/3/activities": {"activities": [{"id": 1}]},
          "GET tickets/3/conversations": {"conversations": [{"id": 2}]},
          "GET changes/3/notes": {"notes": [{"id": 3}]},
          "GET changes/3/tasks": {"tasks": [{"id": 4}]},
          "GET changes/3/approvals": {"approvals": [{"id": 5}]},
          "GET tickets/3/approvals": {"approvals": [{"id": 6}]}})
rec("reads", lambda: [service.get_activities(CHANGES, 3, client=f, page=2), service.get_notes(TICKETS, 3, client=f),
                      service.get_notes(CHANGES, 3, client=f, page=2, per_page=5), service.get_tasks(CHANGES, 3, client=f),
                      service.get_approvals(CHANGES, 3, client=f), service.get_ticket_approvals(TICKETS, 3, client=f),
                      calls(f)])
# get_assets
f = Fake({"GET tickets/3/associated_assets": {"associated_assets": [{"id": 1}]}, "GET problems/3/assets": {"assets": [{"id": 2}]},
          "GET changes/3/assets": {"assets": [{"display_id": 9}]}})
rec("assets", lambda: [service.get_assets(TICKETS, 3, client=f), service.get_assets(PROBLEMS, 3, client=f),
                       service.get_assets(CHANGES, 3, client=f), calls(f)])
# get_associations
f = Fake({"GET tickets/3/changes": {"changes": [{"id": 1}]}, "GET tickets/3/problems": ("status", 403),
          "GET problems/3/changes": {"changes": [{"id": 2}]}, "GET problems/3/tickets": {"tickets": [{"id": 3}]}})
rec("assoc", lambda: [service.get_associations(TICKETS, 3, client=f), service.get_associations(PROBLEMS, 3, client=f),
                      service.get_associations(Resource("assets", "assets", "assets", "asset", ""), 3, client=f) if False else {}])
f = Fake({"GET changes/3/tickets": {"tickets": [{"display_id": 11}]}, "GET changes/3/problems": {"problems": []},
          "GET changes/3/releases": ("status", 404)})
rec("assoc_change", lambda: service.get_associations(CHANGES, 3, client=f))
# requested items
f = Fake({"GET tickets/5/requested_items": {"requested_items": [{"id": 9, "item": {"a": 1}}, {"name": "x"}, {"id": 8}]},
          "GET tickets/5/requested_items/9": {"requested_item": {"quantity": 2, "item": {"b": 2}}},
          "GET tickets/5/requested_items/8": ("status", 500)})
rec("requested", lambda: service.get_requested_items(5, client=f))
# planning
rec("planning_defs", lambda: service.get_change_planning_field_definitions(
    {"fields": [{"name": "a", "field_type": "planning_field"}, {"name": "b", "field_type": "custom_text"},
                {"name": "c", "field_type": "default_backout_plan"}, {"name": "d"}]}))
f = Fake({"GET changes/4/planning-fields": {"change_planning_fields": [{"name": "rollout"}, {"id": 2}]}})
rec("planning_fields", lambda: service.get_change_planning_fields(4, client=f))
rec("by_name", lambda: service.planning_fields_by_name([{"name": "a", "x": 1}, {"name": ""}, {"name": 5}]))
rec("urls", lambda: service.extract_description_attachment_urls(
    '<a href="/helpdesk/attachments/12?download=true">a</a> <img src="https://x.io/helpdesk/attachments/13"> /helpdesk/attachments/12?download=true &lt;/helpdesk/attachments/14&gt;'))
f = Fake({"GET changes/4": {"change": {"id": 4, "description": '<a href="/helpdesk/attachments/1">x</a>', "attachments": [{"id": 1}]}},
          "GET changes/4/planning-fields": {"change_planning_fields": [{"name": "rollout", "id": 2}]}})
rec("evidence", lambda: clean(service.get_change_evidence(4, client=f)))
# writes
f = Fake({"PUT changes/3": {"change": {"id": 3, "subject": "s"}}, "POST tickets/3/reply": {"conversation": {"id": 8}},
          "POST problems/3/notes": {"id": 1}})
rec("writes", lambda: [service.update_item(CHANGES, 3, {"subject": "s"}, client=f), service.add_reply(TICKETS, 3, "hi", client=f),
                       service.add_note(PROBLEMS, 3, "n", public=True, client=f), [c["body"] for c in f.writes()]])
rec("status", lambda: [service.resolve_status({"status": 3, "status_name": "Open"}, CHANGES, {}),
                       service.resolve_status({"status": 3, "ticket_status": {"name": "Pending"}}, TICKETS, {}),
                       service.resolve_status({"status": 2}, CHANGES, {"fields": [{"name": "status", "choices": [{"id": 2, "value": "Planning"}]}]}),
                       service.resolve_status({"status": None}, CHANGES, {"fields": []})])
rec("priority", lambda: [service.resolve_priority({"priority": 4}), service.resolve_priority({"priority": 9}), service.resolve_priority({})])
rec("url", lambda: service.item_url(PROBLEMS, 12))
dump()
