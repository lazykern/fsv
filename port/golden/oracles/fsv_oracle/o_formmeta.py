from common import Fake, rec, dump, OUT
from ticket_meta import change_fields, ticket_fields
from fsv import config, formmeta as fm
from fsv.forms import FormSpec, FormState, LookupValue


def ser(v):
    if isinstance(v, LookupValue):
        return {"lookup": [v.id, v.display, v.email]}
    if isinstance(v, (list, tuple)):
        return [ser(x) for x in v]
    if isinstance(v, dict):
        return {k: ser(x) for k, x in v.items()}
    return v


rec("module_name", lambda: [fm.module_name("Changes"), fm.module_name(" tickets ")])
rec("module_name_bad", lambda: fm.module_name("problems"))
rec("cache_path", lambda: [str(fm._cache_path("x").relative_to(config.CACHE_DIR)), config.cache_domain_key()])
rec("non_empty", lambda: [fm._non_empty(v) for v in [{"a": [1]}, {"a": []}, {"a": 1}, [1], [], "", "x", None, {}]])
fm._save("empty", "schema", {"a": []})
fm._save("full", "schema", {"a": [1]})
rec("cached", lambda: [fm._cached("empty"), fm._cached("full"), fm._cached("missing")])
RULE = {"id": 1, "name": "r", "condition_data": {}, "action_data": []}
rec("normalize", lambda: [fm.normalize_rules({"business_rules": [RULE], "user": {"id": 1}}), fm.normalize_rules([RULE]),
                          fm.normalize_rules({"business_rules": []})])
rec("normalize_bad", lambda: fm.normalize_rules({"x": 1}))
rec("normalize_listing", lambda: fm.normalize_rules({"business_rule": [{"id": 1}]}))
rec("ws_id", lambda: [fm._ws_id(v) for v in [5, "7", " 8 ", True, "x", 1.5, None]])
doc = {"user": {"current_workspace": {"id": 3}, "workspaces": [{"id": 4, "primary": True}, {"id": 5}, 6], "workspace_ids": ["6", 5]}, "workspace_id": 9}
rec("workspaces_in", lambda: [fm.workspaces_in(doc), fm.workspaces_in([1]), fm.workspaces_in({"a": {"b": {"c": {"d": {"workspace_id": 1}}}}})])
rec("cookie_ws", lambda: [fm._cookie_workspace(Fake(cookies=["12", "x"])), fm._cookie_workspace(Fake())])
rec("account_cookie", lambda: fm.account_workspaces(Fake(cookies=["12"])))
rec("account_me", lambda: fm.account_workspaces(Fake({"GET bootstrap/me": {"workspace_id": 4}})))
rec("account_ws", lambda: fm.account_workspaces(Fake({"GET workspaces": {"workspaces": [{"id": 7}, {"id": 8, "default": True}]}})))
rec("account_none", lambda: fm.account_workspaces(Fake()))
rec("account_session", lambda: fm.account_workspaces(Fake({"GET bootstrap/me": ("session",)})))


def calls_of(f):
    return [[c["path"], (c["params"] or {}).get("form_type")] for c in f.calls]


f = Fake({"GET ws/3/business_rules": {"business_rules": [RULE], "user": {"id": 1}}})
rec("fetch_second", lambda: [fm.fetch_rules(f, "change", [2, 3, 2, None], form_type="new_form"), calls_of(f)])
f = Fake({"GET ws/2/business_rules": {}, "GET ws/3/business_rules": {"business_rules": [RULE]}})
rec("fetch_empty_ws", lambda: [fm.fetch_rules(f, "change", [2, 3]), calls_of(f)])
f = Fake({"GET ws/5/business_rules": {"business_rules": [RULE]}})
rec("fetch_fallback", lambda: [fm.fetch_rules(f, "ticket", 2, fallback=lambda: [2, 5]), calls_of(f)])
f = Fake({"GET business_rules": {"business_rules": [RULE]}})
rec("fetch_plain", lambda: [fm.fetch_rules(f, "change", None), calls_of(f)])
rec("fetch_unavailable", lambda: fm.fetch_rules(Fake(), "change", [2]))
rec("fetch_unavailable_none", lambda: fm.fetch_rules(Fake({"GET business_rules": ("status", 403)}), "change", None))
rec("fetch_500", lambda: fm.fetch_rules(Fake({"GET ws/2/business_rules": ("status", 500)}), "change", 2))
rec("warnings", lambda: [fm.rules_warnings({"unavailable": "x"}), fm.rules_warnings({}), fm.rules_warnings(None)])
rec("note", lambda: [fm.rules_note({"unavailable": "x"}), fm.rules_note({"business_rules": []})])
rec("ws_from_fields", lambda: [fm.workspace_from_fields(change_fields()["change_fields"]), fm.workspace_from_fields(ticket_fields()["ticket_fields"]),
                               fm.workspace_from_fields([]), fm.workspace_from_fields([{"workspace_id": 4, "default_field": True}, {"workspace_id": 5}, {"workspace_id": 5}])])
rec("configured", lambda: fm.configured_workspace())
rec("candidates", lambda: [fm.workspace_candidates(1, None, "", 1, 2), fm.workspace_candidates()])
orig = config.load_config
config.load_config = lambda: {"forms": {"workspace_id": "9"}}
rec("configured_set", lambda: [fm.configured_workspace(), fm.workspace_candidates(1, 2)])
f = Fake({"GET ws/9/business_rules": {"business_rules": [RULE]}})
rec("load_rules_configured", lambda: [fm.load_rules(f, "change", 1), calls_of(f)])
config.load_config = orig
f = Fake({"GET bootstrap/me": {"workspace_id": 4}, "GET ws/4/business_rules": {"business_rules": [RULE]}})
rec("load_rules_account", lambda: [fm.load_rules(f, "change", None), calls_of(f)])

# FormMeta / load_meta
m = fm.FormMeta("change", "new_form", change_fields(), {"business_rules": [], "user": {"id": 1}}, 1, 0, "server")
rec("meta_props", lambda: [m.fields_key, len(m.raw_fields), m.user, m.as_of(), m.fingerprint.hash, [x.api_name for x in m.spec.fields]])
f = Fake({"GET ticket_form_fields": ticket_fields(), "GET ws/1/business_rules": {}})
rec("load_meta", lambda: (lambda mm: [mm.module, mm.form_type, mm.rules, mm.workspace_id, mm.source, mm.module_id, calls_of(f)])(
    fm.load_meta(f, "tickets", "edit", fresh=True, module_id=123)))
rec("load_meta_cached", lambda: (lambda mm: [mm.source, mm.rules, calls_of(f)])(fm.load_meta(f, "ticket", "edit", module_id=123)))
rec("load_meta_nofields", lambda: fm.load_meta(Fake({"GET change_form_fields": {"change_fields": []}}), "change", fresh=True))

# lookups
boot = {"groups": [{"id": 4, "name": "Ops"}, {"id": 5, "name": "Ops East"}], "agents": [{"id": 1, "user_id": 21, "name": "Ann"}, {"id": 2, "value": "Bob"}]}
cr = fm.ClientResolver(Fake({"GET bootstrap/agents_groups": boot}))
rec("name_for", lambda: [cr.name_for("group", 4), cr.name_for("agent", "21"), cr.name_for("agent", 2), cr.name_for("user", 1), cr.name_for("group", 9)])
spec = FormSpec.from_form_fields(change_fields(), module="change")
f = Fake({"GET bootstrap/agents_groups": boot,
          "GET autocomplete/requesters": [[{"id": 1, "value": "Ann", "email": "ann@x.io"}, {"id": 2, "value": "Gone", "deleted": True}, {"name": "noid"}]],
          "GET autocomplete/agents": [[{"user_id": 21, "value": "Ann", "details": "ann@x.io"}, {"user_id": 22, "value": "Old (Deactivated)"}]]})
cr = fm.ClientResolver(f)
rec("lookup", lambda: [ser(list(cr.lookup("user", "a", spec.by_name["requester"]))), ser(list(cr.lookup("agent", "a", spec.by_name["agent"]))),
                       ser(list(cr.lookup("group", "ops e", spec.by_name["group"]))), ser(list(cr.lookup("other", "x", spec.by_name["subject"])))])
rec("email", lambda: [fm._email(r) for r in [{"email": "a@b"}, {"email": "x", "details": "d@e"}, {"primary_email": "p@q"}, {"email": 5}]])
rec("person", lambda: [ser(fm._person({"value": "Ann", "email": "a@b"}, 1)), ser(fm._person({"name": "N"}, 2)), ser(fm._person({}, 3))])
f = Fake({"GET autocomplete/requesters": [[{"id": 1, "value": "Ann Lee", "email": "ann@x.io"}, {"id": 2, "value": "Ann Lee", "email": "ann2@x.io"},
                                           {"id": 3, "value": "Bo", "email": None}]]})
rec("req_email", lambda: ser(fm.resolve_requester(f, "ANN@x.io")))
rec("req_many", lambda: fm.resolve_requester(f, "Ann Lee"))
rec("req_pick", lambda: ser(fm.resolve_requester(f, "Ann Lee", pick=lambda c: c[-1])))
rec("req_noemail", lambda: fm.resolve_requester(f, "Bo"))
rec("req_none", lambda: fm.resolve_requester(Fake({"GET autocomplete/requesters": [[]]}), "zed"))
rec("req_empty", lambda: fm.resolve_requester(f, "  "))

rec("find_field", lambda: [fm.find_field(spec, "cf_env").api_name, fm.find_field(spec, " environment ").api_name, fm.find_field(spec, "Subject").api_name])
rec("find_field_dup", lambda: fm.find_field(spec, "dup"))
rec("find_field_none", lambda: fm.find_field(spec, "nope"))
rec("is_x", lambda: [[f.api_name for f in spec.fields if fm.is_status(f)], [f.api_name for f in spec.fields if fm.is_requester(f)],
                     [f.api_name for f in spec.fields if fm.is_description(f)]])
record = {"requester_id": 7, "requester": {"id": 7, "name": "Ann", "email": "ann@x.io"}, "group_id": {"id": 4, "name": "Ops"},
          "agent_id": 21, "status": 2, "description": "", "subject": "Hi", "custom_fields": {"cf_env": "Prod", "cf_note": None}}
ex = fm.existing_from_record(spec, record)
rec("existing", lambda: ser(ex))
rec("hint", lambda: [fm._hint_name("requester", 7, record), fm._hint_name("agent", 21, {"owner_name": "Own"}), fm._hint_name("group", 4, {}),
                     fm._hint_name("agent", 21, {"agent": {"user_id": 21, "full_name": "Full"}})])
cr = fm.ClientResolver(Fake({"GET bootstrap/agents_groups": boot}))
rec("name_existing", lambda: ser(fm.name_existing(dict(ex), spec, record, cr)))
rec("name_existing_nores", lambda: ser(fm.name_existing(dict(ex), spec, {}, None)))
st = FormState(spec, None, mode="new")
failed = set()
rec("apply", lambda: [fm.apply_answers(st, {"Environment": "Prod", "subject": "S", "dup": "x", "cf_env": "Nope", "Status": "Planning"}, failed), sorted(failed),
                      ser(st.values.get("subject")), ser(st.values.get("status"))])
dump()
