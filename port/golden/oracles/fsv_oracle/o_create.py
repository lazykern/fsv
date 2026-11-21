import builtins
import os
import shutil
import sys
from datetime import datetime, timezone

from common import Fake, rec, dump, install_journal
from fsv import create as cr
from fsv.errors import APIError
import common
common.Resp.text = property(lambda r: r.content.decode("utf-8", "replace"))

FILES = "/tmp/fsv_oracle/cfiles"
shutil.rmtree(FILES, ignore_errors=True)
os.makedirs(FILES)
for n, b in [("a.txt", b"hello"), ("b.txt", b"world!"), ("rollout.txt", b"steps")]:
    open(f"{FILES}/{n}", "wb").write(b)

SCHEMA = [
    {"name": "subject", "field_type": "default_subject", "required": True, "default_field": True},
    {"name": "description", "field_type": "default_description", "required": True, "default_field": True},
    {"name": "requester", "field_type": "default_requester", "required": True, "default_field": True},
    {"name": "agent", "field_type": "default_agent", "required": False, "default_field": True},
    {"name": "status", "field_type": "default_status", "required": True, "default_field": True,
     "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}]},
    {"name": "priority", "field_type": "default_priority", "required": False, "default_field": True,
     "choices": [{"id": i, "value": f"P{i}"} for i in range(1, 7)]},
    {"name": "cf_flag", "field_type": "custom_checkbox"},
    {"name": "cf_yes", "field_type": "custom_dropdown", "choices": [{"id": "y", "value": "true"}, {"id": "n", "value": "false"}]},
    {"name": "cf_named", "field_type": "custom_dropdown", "choices": [{"name": "N1"}, {"id": 7}]},
    {"name": "planned_start_date", "field_type": "default_date", "default_field": True},
    {"name": "cf_num", "field_type": "custom_number"},
    {"name": "cf_dec", "field_type": "custom_decimal"},
    {"name": "group", "field_type": "default_group", "required": True},
    {"name": "department", "field_type": "default_department"},
    {"name": "cf_multi", "field_type": "custom_multi_select_dropdown"},
    {"name": "cf_ref_id", "field_type": "custom_text"},
    {"name": "cf_text", "field_type": "custom_text", "required": True},
    {"name": "id", "field_type": "default_id", "required": True},
    {"name": "cf_note", "field_type": "custom_content"},
    {"name": "cf_static", "field_type": "custom_static_rich_text"},
    {"name": "", "field_type": "custom_text"},
    {"name": "reason_for_change", "field_type": "planning_field", "label": "Reason for change", "id": 11},
    {"name": "rollout_plan", "field_type": "planning_field", "label": "Rollout  Plan", "id": 12, "export_label": "Rollout plans"},
    {"name": "backout", "field_type": "default_backout_plan", "label": "Backout &amp; Recovery", "id": 13, "helptext": "Back out steps"},
]
cr.load_schema = lambda res: {"fields": SCHEMA}


def tty(on, answers=()):
    q = list(answers)
    sys.stdin.isatty = lambda: on
    builtins.input = lambda *a: q.pop(0) if q else ""


tty(False)

rec("build_template", lambda: [cr._build_template(SCHEMA, lvl) for lvl in ("required", "optional", "all")])
rec("field_default", lambda: [cr._field_default(f) for f in SCHEMA] + [cr._field_default({"field_type": "custom_text", "choices": [{"id": True}, {"id": False}]})])
rec("change_template", lambda: [cr.change_template(), cr.change_template("all") == cr._build_template(SCHEMA, "all")])
rec("schema_fields", lambda: [cr._get_change_schema_fields() == SCHEMA])
cr.load_schema = lambda res: {"data": {"change_fields": [{"name": "x"}]}}
rec("schema_fields_data", lambda: cr._get_change_schema_fields())
cr.load_schema = lambda res: {"fields": [], "change_fields": [{"name": "y"}]}
rec("schema_fields_flat", lambda: cr._get_change_schema_fields())
cr.load_schema = lambda res: {"fields": SCHEMA}

CHANGE = {"change": {"id": 5, "display_id": 5, "subject": "S", "status": 3, "attachments": [1], "stats": {}, "updated_at": "x",
                     "attached_risk_survey_response_status": "completed",
                     "reason_planning_fields": [], "requester_id": 9, "planning_field_definitions": [],
                     "custom_fields": {"cf_lookup": {"id": 4, "value": "Four"}, "cf_text": "t", "cf_obj": {"id": 1, "value": 2, "x": 3}}}}
import copy
rec("clone_data", lambda: cr.change_clone_data(5, Fake({"GET changes/5": copy.deepcopy(CHANGE)})))
rec("for_edit", lambda: cr.get_change_for_edit(5, Fake({"GET changes/5": copy.deepcopy(CHANGE)})))
rec("parse_when", lambda: [str(cr._parse_when(v)) for v in ["2030-01-01T10:00:00Z", "2030-01-01T10:00:00+07:00", "2030-01-01 10:00:00", "", "junk", 5, None]])
NOW = datetime(2030, 1, 1, tzinfo=timezone.utc)
TASK = {"id": 70, "title": "Deploy", "status": 3, "owner_id": 4, "agent_id": None, "due_date": "2029-12-01T00:00:00Z", "planned_end_date": "2031-01-01T00:00:00Z",
        "start_date": "junk", "created_at": "x", "stack_rank": 1, "description": "d", "custom_fields": {"a": None, "b": 2}}
TASK2 = {"id": 71, "title": "", "owner": {"id": 1}, "custom_fields": {"a": None}}


def tcb():
    d1, d2 = [], []
    return [cr._task_clone_body(TASK, now=NOW, dropped=d1), d1, cr._task_clone_body(TASK, keep_owners=True, now=NOW), cr._task_clone_body(TASK2, now=NOW, dropped=d2), d2]


rec("task_clone_body", tcb)
f = Fake({"GET changes/5/tasks": {"tasks": [TASK, TASK2]}, "POST changes/6/tasks": [{"task": {"id": 1}}, {"id": 2}]})


def ct():
    d = []
    out = cr.clone_tasks(5, 6, f, now=NOW, dropped=d)
    return [out, d, f.writes()]


rec("clone_tasks", ct)
PLAN_ROWS = {"change_planning_fields": [
    {"name": "reason_for_change", "label": "Reason", "description": " why ", "attachments": [
        {"id": 1, "name": "r.txt", "attachment_url": "https://acme.freshservice.com/helpdesk/attachments/1", "content_type": "text/plain; charset=utf-8"},
        {"id": 2, "name": "gone.txt", "canonical_url": "https://cdn/x?sig=1"},
        {"id": 3, "name": "nourl.txt"}]},
    {"name": "rollout_plan", "label": "Rollout plan", "description": "", "attachments": []},
    {"name": "backout", "label": "Backout", "description": "back", "attachments": []},
    {"name": "", "description": "orphan"}]}
ASSETS = {"associated_assets": [{"id": 100, "display_id": 7, "config_item": {"display_id": 7}}, {"id": 101, "config_item": {"display_id": 8}}, {"id": 102}]}
f = Fake({"GET changes/5/tasks": {"tasks": [TASK, TASK2]}, "GET changes/5/planning-fields": PLAN_ROWS, "GET changes/5/assets": ASSETS})
rec("side_effects", lambda: cr.clone_side_effects(5, f))
f = Fake({"GET changes/5/assets": ASSETS, "PUT changes/6/assets/associate": {}})
rec("clone_assets", lambda: [cr.clone_assets(5, 6, f), f.writes()])
f = Fake({"GET changes/5/assets": {"associated_assets": []}})
rec("clone_assets_none", lambda: [cr.clone_assets(5, 6, f), f.writes()])
f = Fake({"GET changes/5/planning-fields": PLAN_ROWS, "FETCH https://acme.freshservice.com/helpdesk/attachments/1?download=true": ("raw", 200, b"R!"),
          "FETCH https://cdn/x?sig=1": ("status", 403), "POST attachments": {"attachment": {"id": 900}}, "POST changes/6/planning-fields?id=reason_for_change": {},
          "POST changes/6/planning-fields?id=backout": {}})


def cpf():
    out = cr.clone_planning_fields(5, 6, f)
    calls = [dict(c) for c in f.calls]
    for c in calls:
        if isinstance(c.get("body"), dict):
            c["body"].pop("content_updated_at", None)
    return [out, calls]


rec("clone_planning", cpf)
rec("clone_planning_none_atts", lambda: cr.clone_planning_fields(5, 6, Fake({"GET changes/5/planning-fields": {"change_planning_fields": [{"name": "x", "description": "d", "attachments": None}]}})))
rec("attachment_url", lambda: [cr._attachment_url(a) for a in [{"attachment_url": "https://x/helpdesk/attachments/1"}, {"attachment_url": "https://x/helpdesk/attachments/1?a=1"},
                                                              {"canonical_url": "https://y/z"}, {"canonical_path": "/helpdesk/attachments/9"}, {}]])
rec("safe_filename", lambda: [cr._safe_filename(n) for n in [" a/b\x00c ", "  ", "ok.txt"]])

DL = "/tmp/fsv_oracle/cdl"
shutil.rmtree(DL, ignore_errors=True)


def dl():
    f = Fake({"FETCH https://x/f1": ("raw", 200, b"one"), "FETCH https://x/bad": ("raw", 500, b"err body")})
    out = [cr.download_attachment({"name": "one.txt", "attachment_url": "https://x/f1"}, DL, c=f)]
    out.append(cr.download_attachment({"name": "one.txt", "attachment_url": "https://x/f1"}, DL, c=f))
    out.append(cr.download_attachment({"name": "one.txt", "attachment_url": "https://x/f1"}, DL, force=True, c=f))
    out.append(cr.download_attachment({"id": 4, "attachment_url": "https://x/f1"}, DL, c=f))
    for o in out:
        o["path"] = os.path.basename(o["path"])
    return [out, [c["path"] for c in f.calls]]


rec("download", dl)
rec("download_bad", lambda: cr.download_attachment({"name": "z.txt", "attachment_url": "https://x/bad"}, DL, c=Fake({"FETCH https://x/bad": ("raw", 500, b"err body")})))
rec("download_nourl", lambda: cr.download_attachment({"id": 1}, DL, c=Fake()))


def reup():
    f = Fake({"FETCH https://x/a": ("raw", 200, b"abc"), "POST attachments": [{"id": 5}, "notadict"]})
    out = [cr._reupload_attachment({"attachment_url": "https://x/a", "name": "a.bin"}, f), cr._reupload_attachment({"attachment_url": "https://x/a"}, f),
           cr._reupload_attachment({}, f), cr._reupload_attachment({"attachment_url": "https://x/none"}, f)]
    bodies = [dict(c["body"]) for c in f.writes() if c["method"] == "POST"]
    for b in bodies:
        b.pop("content_updated_at")
    return [out, bodies]


rec("reupload", reup)
rec("upload_bytes", lambda: (lambda f: [cr._upload_bytes("n.txt", b"xyz", "text/plain", f), f.writes()])(Fake({"POST attachments": {"id": "12", "content_file_size": 3, "content_file_name": "n.txt"}})))
rec("upload_bytes_noid", lambda: cr._upload_bytes("n.txt", b"xyz", "text/plain", Fake({"POST attachments": {"name": "n.txt"}})))
rec("upload_file", lambda: (lambda f: [cr.upload_file(f"{FILES}/a.txt", f), f.writes()[0]["body"]])(Fake({"POST attachments": {"id": 33}})))

EXIST = {"change": {"id": 5, "attachments": [{"id": 1, "name": "a.txt", "attachment_url": "https://x/old-a", "content_type": "text/plain"}, {"id": 2, "content_file_name": "c.txt"}, {"name": "noid.txt"}]}}


def attach(paths, tty_on=False, answers=(), **kw):
    tty(tty_on, answers)
    f = Fake({"GET changes/5": EXIST, "POST attachments": [{"id": 50}, {"id": 51}, {"id": 52}], "PUT changes/5": {}, "FETCH https://x/old-a": ("raw", 200, b"old")})
    try:
        out = cr.attach_files_to_change(5, paths, f, **kw)
    finally:
        tty(False)
    return [out, [(c["method"], c["path"], (c["body"] or {}).get("content_file_name") if c["method"] == "POST" else c.get("body")) for c in f.calls]]


A, B = f"{FILES}/a.txt", f"{FILES}/b.txt"
rec("attach_append", lambda: attach([A, B], duplicate="append"))
rec("attach_skip", lambda: attach([A], duplicate="SKIP"))
rec("attach_replace", lambda: attach([A], duplicate="replace", backup_replaced=False))
rec("attach_replace_backup", lambda: attach([A], duplicate="replace", backup_replaced=True, backup_name="old.txt"))
rec("attach_prompt_notty", lambda: attach([A]))
rec("attach_prompt_tty", lambda: attach([A], tty_on=True, answers=["r", "n"]))
rec("attach_prompt_tty_backup", lambda: attach([A], tty_on=True, answers=["overwrite", "", "mine.txt"]))
rec("attach_prompt_tty_append", lambda: attach([A], tty_on=True, answers=["a"]))
rec("attach_prompt_tty_skip", lambda: attach([A], tty_on=True, answers=["whatever"]))
rec("attach_bad_mode", lambda: attach([A], duplicate="nope"))
rec("set_due_by", lambda: (lambda f: [cr.set_due_by(5, "2030-01-01", f), f.writes()])(Fake({"PATCH changes/5": {"ok": 1}})))

BODY = {"subject": "S", "cf_text": "x", "group_id": {"id": 3, "value": "G"}, "planned_start_date": "d", "attachments": [1], "unknown": 1}
rec("update_change", lambda: (lambda f: [cr.update_change(5, BODY, f), f.writes()])(Fake({"PUT changes/5": {"change": {"id": 5}}})))
rec("update_change_errs", lambda: cr.update_change(5, BODY, Fake({"PUT changes/5": ("fail", 400, {"errors": [{"field": "subject", "message": "bad"}, {}]})})))
rec("update_change_desc", lambda: cr.update_change(5, BODY, Fake({"PUT changes/5": ("fail", 400, {"description": "Validation failed"})})))
rec("update_change_dict", lambda: cr.update_change(5, BODY, Fake({"PUT changes/5": ("fail", 400, {"code": "x"})})))
rec("update_change_raw", lambda: cr.update_change(5, BODY, Fake({"PUT changes/5": ("fail", 500, "boom")})))
rec("submit_change", lambda: (lambda f: [cr.submit_change(BODY, f), f.writes()])(Fake({"POST changes": {"change": {"id": 9}}})))
rec("submit_change_br", lambda: cr.submit_change({"br_validation_excludes": "x"}, Fake()))
rec("submit_change_errs", lambda: cr.submit_change(BODY, Fake({"POST changes": ("fail", 400, {"errors": [{"field": "f", "message": "m"}]})})))
rec("submit_change_desc", lambda: cr.submit_change(BODY, Fake({"POST changes": ("fail", 400, {"description": "D"})})))


def af():
    out = []
    for body in [{"errors": [{"field": "a", "message": "b"}]}, {"description": "D"}, {"x": 1}, "text"]:
        e = cr._api_failure("create", APIError(400, body))
        out.append([type(e).__name__, str(e)])
    return out


rec("api_failure", af)
rec("post_change_body", lambda: (lambda f: [cr.post_change_body({"a": 1}, f), f.writes()])(Fake({"POST changes": {"change": {"id": 9}}})))
rec("post_change_body_list", lambda: cr.post_change_body({"a": 1}, Fake({"POST changes": [[1, 2]]})))
rec("post_change_body_err", lambda: cr.post_change_body({"a": 1}, Fake({"POST changes": ("fail", 400, {"description": "D"})})))
rec("put_change_body", lambda: (lambda f: [cr.put_change_body(5, {"a": 1}, f), f.writes()])(Fake({"PUT changes/5": {"id": 5}})))
rec("put_change_body_err", lambda: cr.put_change_body(5, {"a": 1}, Fake({"PUT changes/5": ("fail", 422, {"errors": [{"field": "a"}]})})))

KEYS = ["Planning-Field-cf_Rollout_Plans", "cf_reason", "Backout & Recovery", "", None, 12]
rec("planning_key", lambda: [cr._planning_key(v) for v in KEYS])
rec("singular_key", lambda: [cr._singular_key(v) for v in KEYS + ["plans-items s"]])
rec("is_planning_field", lambda: [cr._is_planning_field(f) for f in SCHEMA[-4:]])
rec("norm_label", lambda: [cr._norm_label(v) for v in ["  Backout &amp;  Recovery ", None, 5]])
rec("planning_definitions", lambda: [f["name"] for f in cr.planning_definitions(SCHEMA + ["junk"])])
rec("planning_server_key", lambda: [cr.planning_server_key(d) for d in [{"name": "a", "id": 1}, {"id": 2}, {"name": "", "id": 3}]])
RESOLVE = ["reason_for_change", "12", "rollout plan", "backout &amp; recovery", "planning-field-cf_rollout_plans", "cf_reason_for_change", "Back out step", "  "]
rec("resolve_def", lambda: [cr.resolve_planning_definition(v, SCHEMA)["name"] for v in RESOLVE[:-1]])
rec("resolve_def_missing", lambda: cr.resolve_planning_definition("zzz", SCHEMA))
rec("resolve_def_blank", lambda: cr.resolve_planning_definition("  ", SCHEMA))
AMB = [{"name": "a", "field_type": "planning_field", "label": "Same"}, {"name": "b", "field_type": "planning_field", "label": "same"},
       {"name": "plan_x", "field_type": "planning_field", "label": "X"}, {"name": "plans_x", "field_type": "planning_field", "label": "Y"}]
rec("resolve_def_amb_label", lambda: cr.resolve_planning_definition("SAME", AMB))
rec("resolve_def_amb_alias", lambda: cr.resolve_planning_definition("planx", AMB))
rec("resolve_field", lambda: [cr.resolve_planning_field("Rollout plan", SCHEMA), cr.resolve_planning_field("13", [{"id": 13, "field_type": "planning_field"}])])
ROWS = [{"name": "r1", "label": "Reason", "field_id": 11}, {"name": "r2", "label": "Plan", "planning_field_id": 12}, {"id": 13, "label": "Backout"},
        {"label": "dup"}, {"label": "Dup"}, "junk"]
rec("match_row", lambda: [cr.match_planning_row(d, ROWS) for d in [{"name": "r1"}, {"id": 12}, {"id": "13"}, {"label": " reason "}, {"label": "Plan", "id": 99},
                                                                  {"name": "zz", "label": "nothing"}, {}]])
rec("match_row_amb", lambda: cr.match_planning_row({"label": "DUP"}, ROWS))
rec("match_row_conflict", lambda: cr.match_planning_row({"name": "r1", "id": 99}, ROWS))
rec("planning_description", lambda: [cr._planning_description(r) for r in [None, {}, {"description": "  "}, {"description": 5, "body": " b "}, {"content": "c"}, {"description_text": "dt", "content": "c"}]])
rec("default_planning_description", lambda: [cr._default_planning_description([A, "/x/y/z.pdf"]), cr._default_planning_description([])])
rec("attachment_name", lambda: [cr._attachment_name(a) for a in [{"name": "n"}, {"content_file_name": "c"}, {"name": "", "content_file_name": "c"}, {}]])
rec("attachment_id", lambda: [cr._attachment_id(a) for a in [{"id": 3}, {"id": "4"}, {}, {"id": None}]])


def pda():
    out = []
    tty(False)
    try:
        cr._prompt_duplicate_action("a.txt", 2)
    except APIError as e:
        out.append(str(e))
    for ans in ["r", "Replace", "o", "overwrite", "a", "APPEND", "", "s", "x"]:
        tty(True, [f" {ans} "])
        out.append(cr._prompt_duplicate_action("a.txt", 2))
    tty(False)
    return out


rec("prompt_duplicate", pda)
rec("backup_name_given", lambda: [cr._backup_name("a.txt", "b.tar.gz", 0), cr._backup_name("a.txt", "b.tar.gz", 2), cr._backup_name("a.txt", "plain", 1)])
rec("backup_name_stamp", lambda: [cr._backup_name("dir/a.txt"), cr._backup_name("a.txt", None, 3), cr._backup_name("noext")])


def cbn():
    out = []
    tty(True, ["  mine.txt "])
    out.append(cr._choose_backup_name("a.txt", None, 0, True))
    tty(True, [""])
    out.append(cr._choose_backup_name("a.txt", "given.txt", 0, True))
    out.append(cr._choose_backup_name("a.txt", "given.txt", 1, True))
    tty(False)
    out.append(cr._choose_backup_name("a.txt", "given.txt", 0, True))
    return out


rec("choose_backup_name", cbn)


def sbr():
    out = [cr._should_backup_replaced("a", True), cr._should_backup_replaced("a", False)]
    tty(False)
    out.append(cr._should_backup_replaced("a", None))
    for ans in ["", "y", " N ", "no", "nope"]:
        tty(True, [ans])
        out.append(cr._should_backup_replaced("a", None))
    tty(False)
    return out


rec("should_backup", sbr)
rec("backup_attachment", lambda: (lambda f: [cr._backup_attachment({"attachment_url": "https://x/o"}, f, "old.txt"), f.writes()])(
    Fake({"FETCH https://x/o": ("raw", 200, b"old"), "POST attachments": {"id": 61}})))
rec("backup_attachment_ct", lambda: (lambda f: [cr._backup_attachment({"attachment_url": "https://x/o", "content_type": "image/png"}, f, "old.zzz"), [c for c in f.writes() if c["method"] == "POST"][0]["body"]["content_content_type"]])(
    Fake({"FETCH https://x/o": ("raw", 200, b"old"), "POST attachments": {"id": 61}})))
rec("backup_attachment_nourl", lambda: cr._backup_attachment({"id": 1}, Fake(), "x"))
rec("backup_attachment_fail", lambda: cr._backup_attachment({"attachment_url": "https://x/o"}, Fake({"FETCH https://x/o": ("raw", 404, b"missing")}), "x"))

UPF_ROWS = {"change_planning_fields": [{"name": "rollout_plan", "label": "Rollout plan", "description": "", "attachments": [{"id": 1, "name": "a.txt", "attachment_url": "https://x/old-a"}]},
                                       {"name": "reason_for_change", "label": "Reason", "description": "why", "attachments": []}]}


def upf(field, rows=UPF_ROWS, **kw):
    f = Fake({"GET changes/5/planning-fields": rows, "POST attachments": [{"id": 70}, {"id": 71}], "FETCH https://x/old-a": ("raw", 200, b"old"),
              "PUT changes/5/planning-fields/rollout_plan": {"ok": "put"}, "PUT changes/5/planning-fields/reason_for_change": {"ok": "put"},
              "POST changes/5/planning-fields?id=backout": {"ok": "post"}})
    out = cr.update_planning_field(5, field, c=f, **kw)
    return [out, [(c["method"], c["path"], (c["body"] or {}).get("content_file_name") if c["path"] == "attachments" else c.get("body")) for c in f.calls]]


rec("upf_append", lambda: upf("rollout_plan", file_paths=[A, B], duplicate="append"))
rec("upf_replace", lambda: upf("Rollout plan", file_paths=[A], duplicate="replace", backup_replaced=True, backup_name="bk.txt", label="Rollout plan"))
rec("upf_skip_noop", lambda: upf("rollout_plan", file_paths=[A], duplicate="skip"))
rec("upf_desc_only", lambda: upf("reason_for_change", description="new why"))
rec("upf_desc_kept", lambda: upf("reason_for_change", file_paths=[B], duplicate="append"))
rec("upf_new", lambda: upf("backout", file_paths=[f"{FILES}/rollout.txt"], duplicate="append"))
rec("upf_new_desc", lambda: upf("backout", description="text"))
rec("upf_new_empty", lambda: upf("backout"))
rec("upf_bad_mode", lambda: upf("rollout_plan", file_paths=[A], duplicate="x"))
rec("upf_unavailable", lambda: upf("x", rows={"nope": 1}))
rec("upf_unavailable_list", lambda: upf("x", rows=[[1]]))
rec("upf_invalid_row", lambda: upf("x", rows={"change_planning_fields": [{"name": "a"}, 5]}))
rec("upf_unidentified", lambda: upf("x", rows={"change_planning_fields": [{"name": "a"}, {"description": "?"}]}))

rec("approvals", lambda: [cr.get_change_approvals(5, Fake({"GET changes/5/approvals": [[{"id": 1}]]})), cr.get_change_approvals(5, Fake({"GET changes/5/approvals": {"approvals": [{"id": 2}]}})),
                          cr.get_change_approvals(5, Fake({"GET changes/5/approvals": {}}))])
rec("approval_groups", lambda: cr.get_change_approval_groups(5, Fake({"GET changes/5/approval-chains": {"approval_chains": []}})))
rec("approval_groups_bad", lambda: cr.get_change_approval_groups(5, Fake({"GET changes/5/approval-chains": {"x": 1}})))
rec("assets_direct", lambda: cr.get_change_assets(5, Fake({"GET changes/5/assets": ASSETS})))
rec("assets_null", lambda: cr.get_change_assets(5, Fake({"GET changes/5/assets": {"associated_assets": None}})))


def assets_fallback():
    f = Fake({"GET changes/5/associated-cis": [{"data": [{"id": 1}]}, ("status", 404), {"data": [{"id": 3}]}]})
    return [cr.get_change_assets(5, f), [[c["path"], c["params"]] for c in f.calls]]


rec("assets_fallback", assets_fallback)
rec("assets_other_shape", lambda: (lambda f: [cr.get_change_assets(5, f), len(f.calls)])(Fake({"GET changes/5/assets": {"x": 1}, "GET changes/5/associated-cis": {"data": [{"id": 9}]}})))
rec("assets_500", lambda: cr.get_change_assets(5, Fake({"GET changes/5/assets": ("status", 500)})))
rec("assets_cis_500", lambda: cr.get_change_assets(5, Fake({"GET changes/5/associated-cis": ("status", 500)})))
rec("search_assets", lambda: (lambda f: [cr.search_assets_for_change(5, "srv", 2, 10, f), f.calls[0]["params"], cr.search_assets_for_change(5, "", c=f), f.calls[1]["params"]])(
    Fake({"GET assets-to-associate": {"assets": []}})))
rec("associate_assets", lambda: (lambda f: [cr.associate_assets(5, [7, 8], f), f.writes()])(Fake({"PUT changes/5/assets/associate": {}})))
rec("dissociate_assets", lambda: (lambda f: [cr.dissociate_assets(5, [8, 7], f), f.writes()])(Fake({"GET changes/5/assets": ASSETS, "PUT changes/5/assets/detach": {}})))
rec("dissociate_missing", lambda: cr.dissociate_assets(5, [99], Fake({"GET changes/5/assets": ASSETS})))
rec("associations", lambda: (lambda r: [r["complete"], sorted(r.keys())])(cr.get_change_associations(5, Fake({}))))
rec("search_tickets", lambda: (lambda f: [cr.search_change_tickets("vpn", f), f.calls[0]["params"]])(Fake({"GET changes/tickets/search": {"tickets": [{"id": 1}]}})))


def acts():
    f = Fake({"GET changes/5/activities": [{"activities": [{"id": 1}], "next_page_url": "/x?start_token=abc&y"}, {"activities": [{"id": 2}], "next_page_url": "/x?nothing"}]})
    a = cr.get_change_activities(5, f)
    g = Fake({"GET changes/5/activities": [{"activities": [{"id": 1}], "next_page_url": "/x?start_token=T"}, {"activities": [], "next_page_url": "/x?start_token=U"}]})
    b = cr.get_change_activities(5, g)
    return [a, [c["params"] for c in f.calls], b, [c["params"] for c in g.calls]]


rec("activities", acts)
jn = install_journal()
rec("associate_ticket_dry", lambda: cr.associate_ticket(5, [7], Fake(), dry_run=True))
rec("associate_ticket_ro", lambda: cr.associate_ticket(5, [7], Fake(), read_only=True))
rec("dissociate_ticket_dry", lambda: cr.dissociate_ticket(5, 7, Fake(), dry_run=True))
rec("task_for_edit", lambda: cr.get_task_for_edit(5, 70, Fake({"GET changes/5/tasks/70": {"task": TASK}})))
rec("task_put", lambda: (lambda f: [cr._task_put(f, "changes/5/tasks/70", {"a": 1}), f.writes()])(Fake({"PUT changes/5/tasks/70": {"ok": 1}})))
rec("delete_task", lambda: (lambda f: [cr.delete_task(5, 70, f), f.writes()])(Fake({"PUT changes/5/tasks/update": {}})))
rec("update_task", lambda: (lambda f: [cr.update_task(5, 70, {"title": "T", "custom_fields": {"team_name": "x", "a": None, "b": 1}}, f), f.writes()])(
    Fake({"PUT changes/5/tasks/70": {"task": {"id": 70}}})))
rec("update_task_nocf", lambda: (lambda f: [cr.update_task(5, 70, {"title": "T", "custom_fields": {}}, f), f.writes()[0]["body"]])(Fake({"PUT changes/5/tasks/70": {"id": 70}})))

PDL = "/tmp/fsv_oracle/pdl"
shutil.rmtree(PDL, ignore_errors=True)
DL_ROWS = {"change_planning_fields": [
    {"name": "reason_for_change", "label": "Reason for change!", "description": " why ", "attachments": [
        {"name": "a.txt", "attachment_url": "https://x/1"}, {"name": "a.txt", "canonical_url": "https://x/2"}, {"name": "skip.txt"}, {"attachment_url": "https://x/404"}]},
    {"name": "rollout_plan", "label": "", "description": "", "attachments": [{"name": "r.txt", "attachment_url": "https://x/3"}]},
    {"name": "backout", "label": "???", "description": "back", "attachments": []},
    {"name": "empty", "label": "Empty", "description": "", "attachments": []},
    {"label": "nameless", "description": "x"}]}


def pdl(names=None, desc=False, sub="all"):
    out = f"{PDL}/{sub}"
    f = Fake({"GET changes/5/planning-fields": DL_ROWS, "FETCH https://x/1": ("raw", 200, b"one"), "FETCH https://x/2": ("raw", 200, b"two"),
              "FETCH https://x/3": ("raw", 200, b"three"), "FETCH https://x/404": ("raw", 404, b"")})
    paths = cr.download_planning_attachments(5, names, out, desc, f)
    return [[[p.name, p.read_bytes().decode()] for p in paths], [c["path"] for c in f.calls]]


rec("planning_download", lambda: pdl(desc=True))
rec("planning_download_names", lambda: pdl(names=["rollout_plan", "backout"], sub="names"))
dump()
