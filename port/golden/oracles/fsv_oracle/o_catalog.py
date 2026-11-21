import os
from common import Fake, rec, dump, install_journal
from fsv import catalog as cat, config
from fsv.change_flow import FlowExit
from fsv.drafts import Draft
from fsv.forms import FormState, LookupValue, FormSpec

FILES = "/tmp/fsv_oracle/files"
os.makedirs(FILES, exist_ok=True)
open(f"{FILES}/deck.pptx", "wb").write(b"deck-bytes")
open(f"{FILES}/notes.txt", "wb").write(b"notes\n")


def ser(v):
    if isinstance(v, LookupValue):
        return {"lookup": [v.id, v.display, v.email]}
    if isinstance(v, (list, tuple)):
        return [ser(x) for x in v]
    if isinstance(v, dict):
        return {k: ser(x) for k, x in v.items()}
    return v


def item(**kw):
    out = {"id": 9000000001, "display_id": 9000000002, "name": "Acme request", "category_name": "Eng", "workspace_id": 3,
           "updated_at": "2030-01-15T10:00:00+09:00", "allow_attachments": True, "configs": {"attachment_mandatory": False},
           "custom_fields": [
               {"id": 9000000003, "name": "answer", "label": "Answer", "type": 1001, "required": False, "account_id": 9000000004},
               {"id": 9000000005, "name": "deck", "label": "Deck", "type": 1014, "required": False, "account_id": 9000000004,
                "field_options": {"allowed_file_types": '[".pptx"]', "max_attachment_limit": "1"}},
               {"id": 9000000008, "name": "size", "label": "Size", "type": 1003, "required": False, "account_id": 9000000004,
                "choices": [{"id": "s1", "value": "Small", "label": "Small", "position": 1},
                            {"id": "s2", "value": "Medium  Large", "label": "Medium  Large", "position": 2},
                            {"id": "s3", "value": "Mega", "label": "Mega", "position": 3}]},
           ]}
    out.update(kw)
    return out


def lookup_item():
    it = item()
    it["custom_fields"] = it["custom_fields"] + [{"id": 9000000009, "name": "who", "label": "Who", "type": 501, "required": False,
                                                  "account_id": 9000000004}]
    return it


rec("norm", lambda: [cat.norm("  A\u0301b  C "), cat.norm(5), cat.norm(None)])
rec("rows", lambda: [cat._rows([{"a": 1}, 2], "k"), cat._rows({"k": [{"b": 1}, "x"]}, "k"), cat._rows({"k": 1}, "k"), cat._rows("x", "k")])
cats = {"service_categories": [{"id": 1, "name": "Engineering", "sub_categories": [{"id": 2, "name": "Eng Tools"}, "x"]},
                               {"id": 3, "name": "Access", "children": [{"id": 4, "name": "VPN", "parent_id": 9}]}]}
f = Fake({"GET catalog/service-categories": cats})
rec("list_categories", lambda: [cat.list_categories(f), f.calls[0]["params"]])
summ = lambda i, did, name, c, **kw: {"id": i, "display_id": did, "name": name, "short_description": f"{name} desc", "category_id": c, **kw}
page1 = {"service_items": [summ(1, 11, "Laptop", 1), summ(2, 12, "VPN Access", 3), summ(1, 11, "Laptop", 1)], "meta": {"total_pages": 2}}
page2 = {"service_items": [summ(3, 13, "Old", 1, deleted=True), summ(4, 14, "Build Agent", 2)], "meta": {"total_pages": 2}}
f = Fake({"GET catalog/service-items": [page1, page2]})
rec("list_items", lambda: [[i["id"] for i in cat.list_items(f)], [c["params"] for c in f.calls]])
f = Fake({"GET catalog/service-items": {"service_items": [summ(5, 15, "Pop", 1)]}})
rec("list_popular", lambda: [cat.list_items(f, popular=True), f.calls[0]["params"]])
f = Fake({"GET catalog/service-items": [{"service_items": [summ(1, 11, "A", 1)] * 1}, {"service_items": []}]})
rec("list_short_page", lambda: [[i["id"] for i in cat.list_items(f)], len(f.calls)])
flat = cat.list_categories(Fake({"GET catalog/service-categories": cats}))
rec("resolve_category", lambda: [[c["id"] for c in cat.resolve_category(flat, "engineering")], [c["id"] for c in cat.resolve_category(flat, "3")],
                                 [c["id"] for c in cat.resolve_category(flat, "vp")]])
rec("resolve_category_many", lambda: cat.resolve_category(flat, "e"))
rec("resolve_category_none", lambda: cat.resolve_category(flat, "zzz"))
items = [summ(1, 11, "Laptop", 1), summ(2, 12, "VPN Access", 3, category_name="Access"), summ(4, 14, "Build Agent", 2)]
rec("filter_items", lambda: [[i["id"] for i in cat.filter_items(items, category_ids=[1, "2"])], [i["id"] for i in cat.filter_items(items, search="access")],
                             [i["id"] for i in cat.filter_items(items, search="eng tools", categories=flat)], [i["id"] for i in cat.filter_items(items, search="14")]])
f = Fake({"GET catalog/service-items/9000000002": {"service_item": item()}, "GET catalog/service-items/5": {"x": 1}})
rec("get_item", lambda: cat.get_item(f, 9000000002)["name"])
rec("get_item_bad", lambda: cat.get_item(f, 5))
rec("item_ref", lambda: [cat._item_ref(" #77 "), cat._item_ref("Laptop")])
f = Fake({"GET catalog/service-items": {"service_items": items}, "GET catalog/service-items/11": {"service_item": {"display_id": 11, "name": "Laptop"}},
          "GET catalog/service-items/77": {"service_item": {"display_id": 77}}})
rec("find_item", lambda: [cat.find_item(f, "#77")["display_id"], cat.find_item(f, "laptop")["display_id"]])
rec("find_item_empty", lambda: cat.find_item(f, " # "))
rec("find_item_none", lambda: cat.find_item(f, "printer"))
rec("find_item_many", lambda: cat.find_item(f, "a"))
f = Fake({"POST catalog/service-items/77/place-request": {"service_request": {"id": 5, "display_id": 6}}})
rec("place_request", lambda: [cat.place_request(f, 77, {"a": 1}), f.writes()])
rec("created_ref", lambda: [cat.created_ref(r) for r in [{"id": 1}, {"x": {"display_id": 6}}, {"x": {"id": 2, "human_display_id": "SR-9"}}, {"x": 1}, "s"]])

m = cat.ItemMeta(item(), {"business_rules": []}, 3, 0)
rec("item_meta", lambda: [m.display_id, m.name, [f.api_name for f in m.spec.fields], m.fingerprint.hash, m.as_of()])
f = Fake({"GET ws/3/business_rules": {"business_rules": [], "user": {"id": 1}}})
rec("load_item_meta", lambda: (lambda mm: [mm.rules, mm.workspace_id, mm.source, [c["path"] for c in f.calls], f.calls[0]["params"]["item_id"]])(
    cat.load_item_meta(f, None, item=item())))
spec = m.spec
rec("match_choice", lambda: [cat._match_choice(spec.by_name["size"], v) for v in ["Small", "medium   large", "me", "meg", "x", 5]])
rec("lenient", lambda: cat.lenient_answers(spec, {"Size": "meg", "answer": "hi", "unknown": 1}))
rec("file_answers", lambda: cat.file_answers(spec, {"Deck": "deck.pptx", "answer": "x", "deck2": "y"}, "/base"))
rec("file_answers_abs", lambda: cat.file_answers(spec, {"deck": ["/abs/a.pptx", "~/b.pptx"], "unknown": "z"}, None)["deck"][0])
st = FormState(spec, m.engine(), mode="new")
rec("rule_diag", lambda: cat.rule_diagnostics(m, st))
mu = cat.ItemMeta(item(), {"business_rules": [], "unavailable": "why"}, 3, 0)
rec("rule_diag_unavailable", lambda: cat.rule_diagnostics(mu, FormState(mu.spec, mu.engine(), mode="new")))
ml = cat.ItemMeta(lookup_item(), {"business_rules": []}, 3, 0)
stl = FormState(ml.spec, ml.engine(), mode="new")
rec("lookup_errors", lambda: [cat.unsupported_lookup_errors(stl, Draft(answers={"Who": "Ann"})), cat.unsupported_lookup_errors(stl, Draft(answers={"who": ""})),
                              cat.unsupported_lookup_errors(st, Draft(answers={"answer": "x"}))])
rec("digest", lambda: cat._digest(f"{FILES}/deck.pptx"))
rec("requester_line", lambda: [cat.requester_line(LookupValue(1, "Ann", "a@b.co")), cat.requester_line(LookupValue(2, "", "b@c.co")), cat.requester_line(LookupValue(3))])

ANN = LookupValue(id=9000000007, display="Acme", email="a@example.com")
draft = Draft(answers={"Answer": "ok", "Size": "meg", "Deck": f"{FILES}/deck.pptx"}, attachments=[f"{FILES}/notes.txt"])
p = cat.build_sr_plan(m, draft, resolver=None, requester=ANN)
j = p.to_json()
j["review"]["metadata"].pop("fetched_at")
rec("plan", lambda: [p.ok, p.target, p.files, p.hash, j])
bob = LookupValue(id=55, display="Bob", email="bob@example.com")
p2 = cat.build_sr_plan(m, Draft(answers={"Answer": "ok"}), resolver=None, requester=ANN, requested_for=bob, cc_emails=("c@example.com",), cc_ids=(66,))
rec("plan_recipients", lambda: [p2.ok, p2.payload, p2.identity, p2.hash, p2.review.rows])
p3 = cat.build_sr_plan(m, Draft(answers={"Nope": 1}, description_html="<p>x</p>", planning={"a": {}}, attachments=["/missing.txt"]), resolver=None, requester=None)
rec("plan_errors", lambda: [p3.ok, p3.errors, p3.payload, p3.hash])
p4 = cat.build_sr_plan(m, Draft(answers={}), resolver=None, requester=LookupValue(1, "NoMail"), cc_emails=("bad",))
rec("plan_errors2", lambda: [p4.errors, p4.payload])

rec("send_failure", lambda: [str(cat.SendFailure("upload", [{"id": 1, "name": "a.txt"}], RuntimeError("boom"))), str(cat.SendFailure("place-request", [], ValueError("x")))])
rec("substitute", lambda: cat._substitute({"a": ["<u1>", {"b": "<u1>"}], "c": "<u2>", "d": 1}, {"<u1>": 7}))

# execute
jn = install_journal()
ids = iter([101, 102])
f = Fake({"POST catalog/service-items/9000000002/place-request": {"service_request": {"id": 500, "display_id": 600}}})
rec("execute", lambda: cat.execute(p, f, upload_fn=lambda c, path: next(ids)))
rec("execute_events", lambda: jn.events())
rec("execute_writes", lambda: f.writes())
rec("execute_dup", lambda: (lambda: cat.execute(p, f, upload_fn=lambda c, path: 1))())
rec("execute_dup_allowed_started", lambda: None)
jn = install_journal()
f = Fake({"POST catalog/service-items/9000000002/place-request": ("status", 422)})
rec("execute_422", lambda: cat.execute(p, f, upload_fn=lambda c, path: 7))
rec("execute_422_events", lambda: jn.events())
jn = install_journal()


def boom(c, path):
    raise RuntimeError("disk")


rec("execute_upload_fail", lambda: cat.execute(p, Fake(), upload_fn=boom))
rec("execute_upload_fail_events", lambda: jn.events())
jn = install_journal()
rec("execute_bad_plan", lambda: cat.execute(p3, Fake()))
jn = install_journal()
f = Fake({"POST catalog/service-items/9000000002/place-request": {"ok": True}})
rec("execute_noid", lambda: cat.execute(p, f, upload_fn=lambda c, path: 3))
rec("execute_noid_events", lambda: jn.events())
jn = install_journal()
f = Fake({"POST catalog/service-items/9000000002/place-request": {"service_request": {"id": 500}},
          "GET tickets/500": {"ticket": {"id": 500, "email": "a@example.com", "requested_for": "bob@example.com", "cc_emails": ["x@example.com"]}}})
rec("execute_identity", lambda: cat.execute(p2, f, upload_fn=lambda c, path: 3))
jn = install_journal()
open(f"{FILES}/deck2.pptx", "wb").write(b"v1")
pd = cat.build_sr_plan(m, Draft(answers={"Answer": "ok", "Deck": f"{FILES}/deck2.pptx"}), resolver=None, requester=ANN)
open(f"{FILES}/deck2.pptx", "wb").write(b"v2")
rec("execute_changed_file", lambda: cat.execute(pd, Fake(), upload_fn=lambda c, path: 3))
jn = install_journal()
jn.record_plan("started", p.hash, action="create")
rec("dup_started", lambda: cat.duplicate_check(p, True))
jn = install_journal()
jn.record_plan("done", p.hash, action="create", created_id=5)
rec("dup_done", lambda: cat.duplicate_check(p, False))
rec("dup_allowed", lambda: cat.duplicate_check(p, True))

# freshen
f = Fake({"GET catalog/service-items/9000000002": {"service_item": item()}, "GET ws/3/business_rules": {"business_rules": []}})
lm = cat.load_item_meta(f, 9000000002)
pf = cat.build_sr_plan(lm, Draft(answers={"Answer": "ok"}), resolver=None, requester=ANN)
rec("freshen_same", lambda: (lambda r: [r[0] is pf, r[1]])(cat.freshen(pf, f, lambda mm: None)))
f = Fake({"GET catalog/service-items/9000000002": {"service_item": item(updated_at="2030-02-01T00:00:00+09:00")}, "GET ws/3/business_rules": {"business_rules": []}})
rec("freshen_same_hash", lambda: (lambda r: [r[0].hash == pf.hash, r[1]])(cat.freshen(pf, f, lambda mm: cat.build_sr_plan(mm, Draft(answers={"Answer": "ok"}), resolver=None, requester=ANN))))
rec("freshen_changed", lambda: (lambda r: [r[0].hash, r[1]])(cat.freshen(pf, f, lambda mm: cat.build_sr_plan(mm, Draft(answers={"Answer": "new"}), resolver=None, requester=ANN))))

# requested items
defs = [{"name": "a", "label": "A &amp; B"}, {"name": "n", "label": "Notice", "type": 1024}, {"name": "c", "label": "C", "field_type": "custom_static_rich_text"},
        {"name": "s", "label": "S", "fields": [{"name": "s1", "label": "S1"}], "nested_fields": [{"name": "s2", "label": "S2"}],
         "sections": [{"fields": [{"name": "s3", "label": "S3"}]}, "x"]}, {"name": "a", "label": "dup"}, "junk"]
rec("label_map", lambda: cat.label_map(defs))
f = Fake({"GET catalog/service-items/9000000002": {"service_item": item()}})
rec("item_labels", lambda: cat.item_labels(f, {"item": {"display_id": 9000000002, "custom_fields": [{"name": "answer", "label": "Ans"}]},
                                              "custom_fields": {"answer": "x", "size": "Small", "zzz": 1}}))
rec("item_labels_404", lambda: cat.item_labels(Fake(), {"service_item_display_id": 5, "custom_fields": {"q": 1}}))
rec("item_labels_complete", lambda: cat.item_labels(Fake(), {"item": {"custom_fields": [{"name": "q", "label": "Q"}]}, "custom_fields": {"q": 1}}))
rec("value_text", lambda: [cat.value_text(v) for v in [["a", {"name": "N"}], {"value": "V"}, {"x": 1}, True, False, "<p>a  <b>b</b></p>", 5, None, "a < b"]])
rec("is_file_value", lambda: [cat.is_file_value(v) for v in [[1, 2], [{"attachment_url": "u"}], [{"name": "n", "id": 1}], [{"name": "n"}], [], "x", [1, "a"]]])
os.makedirs("/tmp/fsv_oracle/dl", exist_ok=True)
rec("safe_target", lambda: str(cat.safe_target("/tmp/fsv_oracle/dl", "a.txt")))
for name, bad in [("st_empty", " "), ("st_dots", "a..b"), ("st_sep", "a/b"), ("st_bs", "a\\b"), ("st_drive", "C:x"), ("st_ctl", "a\x01")]:
    rec(name, lambda bad=bad: cat.safe_target("/tmp/fsv_oracle/dl", bad))
rec("domain_url", lambda: cat.domain_url("/a/b"))
dump()
