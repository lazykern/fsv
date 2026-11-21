from common import Fake, rec, dump
from rich.table import Table
from fsv import state_flow as sf


class Rec:
    def __init__(self):
        self.out = []

    def print(self, *args, **kw):
        if not args:
            self.out.append({"blank": True})
        elif isinstance(args[0], Table):
            t = args[0]
            rows = [list(r) for r in zip(*[c._cells for c in t.columns])]
            self.out.append({"title": t.title, "rows": rows})
        else:
            self.out.append({"text": args[0]})


def run(fn, change, fake, **kw):
    sf.get_client = lambda: fake
    r = Rec()
    fn(change, console=r, **kw)
    return r.out


flow = {"state_flow": {"name": "Std", "state_list": [{"id": i, "value": v} for i, v in
        [(1, "Open"), (2, "Plan [x]"), (3, "Approve"), (4, "Build"), (5, "Test"), (6, "Done"), (7, "Closed")]]}}
f = Fake({"GET ws/3/change_management/9": flow, "GET change_management/9": {"name": "Plain", "state_list": []}})
rec("get_ws", lambda: sf.get_state_flow(9, f, 3)["name"])
rec("get_plain", lambda: sf.get_state_flow(9, f))
rec("get_calls", lambda: f.calls)
f2 = Fake({"GET change_management/9": flow})
rec("get_ws404", lambda: [sf.get_state_flow(9, f2, 4)["name"], f2.gets()])
rec("get_ws500", lambda: sf.get_state_flow(9, Fake({"GET ws/4/change_management/9": ("status", 500)}), 4))
rec("approvals", lambda: [sf._try_get_approvals(5, Fake({"GET changes/5/approvals": {"approvals": [{"id": 1}]}})),
                          sf._try_get_approvals(5, Fake({"GET changes/5/approvals": [[{"id": 1}]]})),
                          sf._try_get_approvals(5, Fake())])

change = {"id": 5, "human_display_id": "CHN-5", "state_flow_id": 9, "status": 4, "status_name": "Build",
          "state_traversal": [1, 2, 3, 2], "change_type": 1}
fa = Fake({"GET change_management/9": flow,
           "GET changes/5/approvals": {"approvals": [{"level_id": 1, "status": {"name": "approved"}, "member": {"name": "Ann"}},
                                                     {"level_id": 2, "status": {"name": "rejected"}, "member_id": 8},
                                                     {"level_id": 2, "member": {"name": ""}, "member_id": 9}]}})
rec("flow", lambda: run(sf.render_flow, change, fa))
rec("flow_no_id", lambda: run(sf.render_flow, {"id": 5}, fa))
rec("flow_empty", lambda: run(sf.render_flow, {"id": 6, "state_flow_id": 9}, Fake({"GET change_management/9": {"state_list": []}})))
rec("flow_bare", lambda: run(sf.render_flow, {"id": 6, "state_flow_id": 9, "status": 1}, Fake({"GET change_management/9": {"state_list": [{"id": 1, "value": "Open"}]}})))
rec("compact", lambda: run(sf.render_compact, change, fa))
rec("compact_ctx1", lambda: run(sf.render_compact, {**change, "status": 1}, fa, context=1))
rec("compact_none", lambda: [run(sf.render_compact, {**change, "status": 99}, fa), run(sf.render_compact, change, Fake()),
                             run(sf.render_compact, {"id": 1}, fa)])
rec("diagram", lambda: run(sf.render_state_diagram, change, fa))
rec("diagram_all", lambda: run(sf.render_state_diagram, {**change, "status": 7, "state_traversal": [1, 2, 3]}, fa, context=10))
rec("diagram_none", lambda: [run(sf.render_state_diagram, {**change, "status": 99}, fa), run(sf.render_state_diagram, change, Fake())])
dump()
