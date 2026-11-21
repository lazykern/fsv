"""Key and answer sequences for the form UI session differential
(`gen_formui.py` plays them on Python's `FormSession`, `formui_diff.rs` on the
Rust one). An op is one of: set, suggest, help, effects, mark_invalid, ack,
plan, save, send. Values: JSON scalars and lists, or {"lookup": [id, name,
email]}, {"dt": iso}, {"date": iso}; "@TMP@" is the scenario's file directory.
"""
JANE = [9000007001, "Jane Doe", "jane.doe@example.com"]
SAM = [9000002, "Sam Lee", "sam.lee@example.com"]
CJANE = [9000001, "Jane Doe", "jane.doe@example.com"]
NOMAIL = [9000099, "No Mail", None]
REQ = "__requester__"
ATT = "__attachments__"


def S(key, value):
    return {"set": key, "value": value}


def L(*v):
    return {"lookup": list(v)}


def SUG(key, q):
    return {"suggest": key, "query": q}


def H(key):
    return {"help": key}


def E(key):
    return {"effects": key}


PLAN = {"plan": None}
PLAN_ACK = {"plan": True}

ACME_KEYS = ["requester", "subject", "change_type", "cloud_only", "stage", "staging_start", "staging_end", "services",
             "notify_ops", "reviewer", "downtime_window", "legacy_flag", "description", "priority", "impact", "risk",
             "group", "agent", "planned_start_date", "planned_end_date"]

SCENARIOS = [
    {"name": "acme_new_rules", "tenant": "acme", "kind": "change", "requester": JANE,
     "files": {"runbook.pdf": 20, "empty.txt": 0, "notes.txt": 5, "big.bin": 3_000_000},
     "ops": [
         S("subject", "Rotate keys"), S("subject", ""), S("subject", "Rotate keys"),
         S("change_type", "Normal"), S("change_type", "nope"), S("change_type", "Standard"),
         S("cloud_only", "Yes"), S("cloud_only", "No"), S("cloud_only", "Maybe"), S("cloud_only", None),
         S("cloud_only", "No"),
         S("staging_start", "2026-11-26 19:00"), S("staging_start", "garbage"), S("staging_end", "2026-11-25 19:00"),
         S("staging_end", {"dt": "2026-11-27T19:00:00+09:00"}), S("staging_start", {"dt": "2026-11-26T19:00:00+09:00"}),
         S("services", ["Billing", "Search"]), S("services", ["Nope"]), S("services", "Billing"), S("services", []),
         S("services", ["Billing", "Mail", "Chat"]),
         S("notify_ops", "Yes"), S("downtime_window", "22:00 - 22:30"), S("legacy_flag", True),
         S("priority", "Urgent"), S("priority", "4"), S("priority", "Low"), S("impact", "Low"), S("risk", "High"),
         S("description", "# Title\n\nSome **bold** text\n\n- a\n- b"), S("description", ""),
         S("description", "plain"),
         S("reviewer", "Jane"), S("reviewer", L(9000007001, "Jane Doe", None)),
         S("group", L(9000009001, "Platform Ops", None)), S("agent", L(9000008001, "Alex Kim", "alex.kim@example.com")),
         S("planned_start_date", "2026-12-01 10:00"), S("planned_end_date", "2026-12-01 12:00"),
         S("planned_end_date", "2026-11-30 12:00"),
         S(ATT, "@TMP@/notes.txt"), S(ATT, "@TMP@/missing.txt"), S(ATT, "@TMP@/empty.txt"),
         S(ATT, "@TMP@/notes.txt; @TMP@/runbook.pdf"),
         S("planning:Go-Live Checklist", "@TMP@/runbook.pdf"), S("planning:Go-Live Checklist", ""),
         S("planning:Go-Live Checklist", "@TMP@/runbook.pdf"),
         S("requester", L(*SAM)), S("requester", "jane"), S("requester", None), S("requester", L(*JANE)),
         PLAN, S("stage", "Staging"), S("reviewer", None),
         {"mark_invalid": "staging_start", "error": "not a date"}, {"mark_invalid": "staging_start", "error": ""},
         PLAN, PLAN_ACK, {"ack": True}, PLAN,
         {"save": "draft.yaml"}, {"send": True},
     ]},
    {"name": "acme_new_lookups", "tenant": "acme", "kind": "change", "requester": None,
     "ops": [SUG("requester", ""), SUG("requester", "jane"), SUG("requester", "JANE"), SUG("requester", "doerr"),
             SUG("requester", "nobody"), SUG("requester", "@example.com"),
             SUG("reviewer", "jane"), SUG("reviewer", ""), SUG("reviewer", "zz"),
             SUG("group", ""), SUG("group", "plat"), SUG("group", "xyz"), SUG("agent", ""), SUG("agent", "al"),
             SUG("agent", "zzz"),
             SUG("change_type", ""), SUG("change_type", "nor"), SUG("change_type", "em"), SUG("change_type", "zzz"),
             SUG("services", ""), SUG("services", "re"), SUG("stage", ""), SUG("stage", "sta"), SUG("priority", "u"),
             SUG("subject", "x"), SUG("legacy_flag", ""), SUG("description", ""),
             S("change_type", "Normal"), SUG("change_type", ""), S("services", ["Billing", "Mail"]),
             SUG("services", ""), SUG("services", "ma"),
             S("requester", L(*JANE)), PLAN]},
    {"name": "acme_new_help", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [H(k) for k in ACME_KEYS] + [E(k) for k in ACME_KEYS] + [
         S("cloud_only", "No")] + [H(k) for k in ACME_KEYS] + [E(k) for k in ACME_KEYS] + [
         S("priority", "Urgent"), H("priority"), E("priority"), H("__attachments__"), H("planning:Go-Live Checklist")]},
    {"name": "acme_edit", "tenant": "acme", "kind": "change", "mode": "edit", "target": 900000501,
     "requester": None,
     "ops": [PLAN, S("subject", "Billing: rotate keys v2"), S("stage", "Staging"), S("cloud_only", "No"),
             S("stage", "Staging"), S("staging_start", "2026-11-26 19:00"), S("staging_end", "2026-11-26 21:00"),
             S("downtime_window", ""), S("services", ["Billing", "Search"]), S("description", "Rotate keys."),
             S("description", "Rotate all the keys."), S("description", ""), S("reviewer", L(9000007002, "Jane Doerr", None)),
             S("reviewer", None), S("priority", "High"), S("group", None), S("legacy_flag", True),
             SUG("stage", ""), H("stage"), E("cloud_only"), PLAN, S("downtime_window", "22:00 - 22:30"),
             S("description", "Rotate all the keys."), PLAN, PLAN_ACK, {"save": "edit.yaml"}, {"send": True}]},
    {"name": "acme_edit_noop", "tenant": "acme", "kind": "change", "mode": "edit", "target": 900000501,
     "ops": [PLAN, S("subject", "Billing: rotate keys"), S("priority", "Medium"), PLAN, {"save": "noop.yaml"}]},
    {"name": "acme_default_requester", "tenant": "acme", "kind": "change",
     "config": {"defaults": {"requester": "jane.doe@example.com"}},
     "ops": [SUG("requester", "jan"), S("subject", "S"), PLAN]},
    {"name": "acme_default_requester_unresolved", "tenant": "acme", "kind": "change",
     "config": {"defaults": {"requester": "nobody@example.com"}}, "ops": [PLAN]},
    {"name": "acme_dry_run", "tenant": "acme", "kind": "change", "requester": JANE, "dry_run": True,
     "ops": [S("subject", "x"), PLAN, {"send": True}]},
    {"name": "acme_env_tz", "tenant": "acme", "kind": "change", "requester": JANE, "env": {"FSV_TZ": "Asia/Bangkok"},
     "ops": [S("planned_start_date", "2026-12-01 10:00"), S("planned_start_date", "2026-12-01 10:00+01:00"), PLAN]},
    {"name": "acme_max_file_mb", "tenant": "acme", "kind": "change", "requester": JANE,
     "config": {"forms": {"max_file_mb": 1}}, "files": {"big.bin": 3_000_000, "small.bin": 10},
     "ops": [S(ATT, "@TMP@/big.bin"), S(ATT, "@TMP@/small.bin"), PLAN]},
    {"name": "ticket_new", "tenant": "ticket", "kind": "ticket", "requester": JANE,
     "ops": [S("subject", "Service unavailable"), S("description", "Please help"), S("cloud_only", "Yes"),
             S("priority", "High"), S("impact", "Low"), S("requester", L(*NOMAIL)), S("requester", L(*JANE)),
             SUG("requester", "jane"), PLAN, PLAN_ACK, {"save": "t.yaml"}, {"send": True}]},
    {"name": "ticket_noreq", "tenant": "ticket", "kind": "ticket", "requester": None,
     "ops": [S("subject", "x"), PLAN, S("description", "d"), S("requester", L(*JANE)), PLAN]},
    {"name": "globex_new", "tenant": "globex", "kind": "change", "requester": None,
     "ops": [SUG("requester", ""), SUG("requester", "erika"), SUG("requester", "a@b.c"), SUG("requester", "x"),
             SUG("group", "x"), SUG("agent", "a"),
             SUG("agent", "a@b.c"), SUG("umgebung", ""), SUG("change_type", "not"),
             S("requester", L(9000012001, "Erika Muster", "erika.muster@example.com")),
             S("subject", "Wartung"), S("change_type", "Notfall"), S("umgebung", "Produktion"),
             S("umgebung", "Nope"), S("umgebung", "Test"), S("description", "Text"),
             S("priority", "Hoch"), S("planned_start_date", "2031-03-30 02:30"),
             S("planned_start_date", "2031-03-30 03:30"), S("planned_end_date", "2031-10-26 02:30"),
             S("planned_end_date", "2031-10-26 03:30"), S("planned_end_date", "2031-10-26 02:30+01:00"),
             S("group", L(5, "G", None)),
             H("planned_start_date"), H("umgebung"), H("change_type"), PLAN, PLAN_ACK, {"save": "g.yaml"},
             {"send": True}]},
    {"name": "globex_sr", "tenant": "globex", "kind": "sr", "item": 501, "requester": None,
     "files": {"offer.pdf": 10, "offer.txt": 10, "two.pdf": 10},
     "ops": [S("begruendung", "Neues Gerät"), S("modell", "Weder noch"), S("modell", "Standard 14 Zoll"),
             S("benoetigt_bis", {"date": "2031-05-01"}), S("benoetigt_bis", "2031-05-01"),
             S("benoetigt_bis", "2031-05-01 10:00"),
             S("angebot", "@TMP@/offer.txt"), S("angebot", "@TMP@/offer.pdf; @TMP@/two.pdf"),
             S("angebot", "@TMP@/offer.pdf"),
             S(REQ, L(*NOMAIL)), S(REQ, L(9000012001, "Erika Muster", "erika.muster@example.com")),
             S("benoetigt_bis", "2031-05-01 10:00"), S("benoetigt_bis", "2031-05-01"),
             SUG("modell", ""), SUG("modell", "ent"), H("benoetigt_bis"), H("angebot"), PLAN, PLAN_ACK,
             {"save": "sr.yaml"}, {"send": True}]},
    {"name": "catalog_sr", "tenant": "catalog", "kind": "sr", "item": 77, "requester": None,
     "files": {"deck.pptx": 10, "brief.pdf": 10, "notes.txt": 5},
     "ops": [S(REQ, L(*CJANE)), S("title_100", "Dark mode"), S("paragraph_101", "Please add dark mode."),
             S("paragraph_102", "x"), S("paragraph_102", ""),
             S("date_103", {"date": "2030-12-03"}), S("goal", "Lower cost"), E("goal"), H("goal"),
             S("goal_2_kpi", "-10% support tickets"), S("goal", "More revenue  (Total USD)"),
             S("goal_1_kpi", "up"), S("goal", "Other"), S("goal", "Lower cost"),
             S("priority", "Medium"), S("priority", "Medium - planned work"), SUG("priority", "crit"),
             S("overview_deck", "@TMP@/deck.pptx"), S("overview_deck", "@TMP@/brief.pdf"),
             S("overview_deck", "@TMP@/deck.pptx"), S(ATT, "@TMP@/notes.txt"), S(ATT, ""),
             S(ATT, "@TMP@/notes.txt"), SUG("goal", ""), SUG("goal", "co"), H("overview_deck"), PLAN, PLAN_ACK,
             {"save": "cat.yaml"}, {"send": True}]},
    {"name": "catalog_sr_default_requester", "tenant": "catalog", "kind": "sr", "item": 77,
     "config": {"defaults": {"sr_requester": "jane.doe@example.com"}},
     "ops": [SUG(REQ, "sam"), SUG(REQ, ""), PLAN]},
    {"name": "catalog_sr_other_item", "tenant": "catalog", "kind": "sr", "item": 78, "requester": CJANE,
     "ops": [PLAN]},
]


def K(*keys):
    return {"keys": list(keys)}


def T(text):
    return {"type": text}


def W(secs):
    return {"wait": secs}


def F(key):
    return {"focus": key}


def PRE(*pairs):
    return {"pre": [list(p) for p in pairs]}


CHANGE_FILLED = [("subject", "Billing: rotate keys"), ("change_type", "Normal"), ("cloud_only", "Yes"),
                 ("description", "Rotate the **billing** keys."),
                 ("priority", "High"), ("impact", "Low"), ("risk", "Low"),
                 ("planned_start_date", {"dt": "2030-11-26T19:00:00+09:00"}),
                 ("planned_end_date", {"dt": "2030-11-26T22:00:00+09:00"})]

UI_SCENARIOS = [
    {"name": "tab_walk", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [K("tab")] * 28 + [K("shift+tab")] * 6 + [K("f1"), K("f1")]},
    {"name": "text_and_choice", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [F("subject"), T("Rotate keys"), W(0.4), K("tab"), T("emer"), K("down"), K("enter"),
             K("tab"), T("n"), K("down"), K("down"), K("up"), K("enter"), K("tab"), T("zzz"), K("enter"),
             K("backspace", "backspace", "backspace"), T("y"), K("enter"), K("escape"), K("tab"),
             K("shift+tab"), K("shift+tab"), T("Standard"), K("enter"), K("enter"), K("backspace"),
             W(0.4), K("enter")]},
    {"name": "button_arrows", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [K("shift+tab"), K("shift+tab"), K("shift+tab"), K("shift+tab"), K("left"), K("left"), K("right"),
             K("down"), K("pagedown"), K("up"), K("home"), K("end"), K("tab"), K("down"), K("tab"), K("tab"),
             K("f1"), K("shift+tab"), K("shift+tab")]},
    {"name": "layer_arrows", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [PRE(*CHANGE_FILLED), K("ctrl+s"), K("left"), K("right"), K("right"), K("tab"), K("left"), K("f4"),
             K("left"), K("tab"), K("left"), K("right"), K("escape"), K("tab"), K("left"), K("escape"),
             F("subject"), T("x"), K("escape"), K("left"), K("right"), K("right"), K("left"), K("escape"),
             K("f6"), K("ctrl+s"), K("f6"), K("ctrl+a"), T("@TMP@/c.yaml"), K("enter"), K("left"), K("tab"),
             K("left"), K("right")]},
    {"name": "choice_rules", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [F("cloud_only"), K("down"), K("enter"), K("tab"), K("down"), K("enter"), F("cloud_only"), K("down"),
             K("down"), K("enter"), W(0.2), F("stage"), K("down"), K("enter"), F("cloud_only"), T("y"), K("enter"),
             F("stage"), K("enter"), K("escape"), F("cloud_only"), K("backspace", "backspace", "backspace"),
             K("enter")]},
    {"name": "multi_select", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [F("services"), T("s"), K("enter"), T("bill"), K("enter"), T("ch"), K("enter"), K("down"), K("space"),
             K("escape"), K("tab"), K("shift+tab"), K("down"), K("enter"), K("enter"), K("enter"), T("zzz"),
             K("enter"), K("backspace", "backspace", "backspace"), K("escape")]},
    {"name": "lookup_people", "tenant": "acme", "kind": "change", "requester": None,
     "ops": [F("requester"), T("jan"), K("enter"), W(0.4), K("down"), K("enter"), K("enter"), W(0.4),
             F("requester"), K("backspace"), T("zzz"), W(0.4), K("enter"), K("ctrl+a"), T("doe"), W(0.4), K("down"),
             K("down"), K("enter"), F("reviewer"), T("jane"), W(0.4), K("escape"),
             F("group"), K("enter"), W(0.1), K("down"), K("enter"), F("agent"), T("al"), W(0.1), K("enter"),
             K("tab"), K("tab")]},
    {"name": "lookup_denied", "tenant": "globex", "kind": "change", "requester": None,
     "ops": [F("requester"), T("erika"), W(0.4), K("enter"), K("enter"), F("requester"), K("ctrl+a"),
             T("x@y.z"), W(0.4), K("down"), K("enter"), F("group"), K("enter"), W(0.2), K("escape"),
             F("agent"), T("a@b.c"), W(0.4), K("down"), K("enter"), K("escape")]},
    {"name": "dates_typed", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [F("planned_start_date"), T("2030-11-26 19:00"), W(0.1), K("enter"), K("tab", "tab"), T("garbage"),
             K("enter"), K("tab"), K("shift+tab"), K("shift+tab"), K("ctrl+a"), T("tomorrow"), K("enter"),
             F("planned_end_date"), T("2030-11-26 25:00"), K("enter"), K("ctrl+a"), T("2030-11-26 22:00+01:00"),
             K("enter"), K("tab"), K("ctrl+a"), K("backspace"), K("enter"), K("tab"), K("tab")]},
    {"name": "date_calendar", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [PRE(("planned_start_date", {"dt": "2030-11-26T19:00:00+09:00"}),
                 ("cloud_only", "No"), ("staging_start", {"dt": "2030-12-02T09:30:00+09:00"})),
             F("planned_start_date"), K("f2"), K("right"), K("down"), K("pagedown"), K("pageup"), K("left"), K("up"),
             K("home"), K("end"), K("tab"), K("tab"), K("enter"), K("shift+tab"), K("shift+tab"), K("enter"),
             K("f2"), K("escape"), F("staging_start"), K("f2"), K("pagedown"), K("enter"), K("enter"),
             K("f2"), T("x"), K("tab"), K("tab"), K("tab"), K("escape"),
             F("planned_end_date"), K("tab"), K("enter"), K("escape")]},
    {"name": "description_editor", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [F("description"), T("# Title"), K("enter"), K("enter"), T("Some **bold** words"), W(0.4),
             K("enter"), T("- a"), K("enter"), T("- b"), K("tab"), K("shift+tab"),
             K("ctrl+a"), K("backspace"), W(0.4), K("tab"), K("tab")]},
    {"name": "file_editor", "tenant": "acme", "kind": "change", "requester": JANE,
     "files": {"alpha.pdf": 10, "alphabet.txt": 10, "beta.txt": 4, "empty.bin": 0},
     "ops": [F("__attachments__"), T("@TMP@/al"), K("right"), K("enter"), K("ctrl+a"), T("@TMP@/b"), K("right"),
             K("enter"), T("; "), T("@TMP@/alpha.pdf"), K("enter"), K("ctrl+a"), T("@TMP@/empty.bin"), K("enter"),
             K("ctrl+a"), T("@TMP@/missing"), K("enter"), K("tab"), K("shift+tab"), K("ctrl+a"), K("backspace"),
             K("enter"), F("planning:Go-Live Checklist"), T("@TMP@/beta.txt"), K("tab")]},
    {"name": "review_and_actions", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [K("ctrl+s"), K("tab"), K("shift+tab"), K("escape"), PRE(*CHANGE_FILLED), K("ctrl+s"), K("f7"), K("f7"),
             K("tab"), K("tab"), K("tab"), K("shift+tab"), K("escape"), K("ctrl+s"), K("f6"), T("/nonexistent/x.yaml"),
             K("enter"), K("ctrl+a"), T("@TMP@/cmd.yaml"), K("enter"), K("escape"), K("ctrl+s"), K("f4"),
             K("ctrl+a"), T("@TMP@/saved.yaml"), K("enter"), K("enter")]},
    {"name": "review_submit", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [PRE(*CHANGE_FILLED), K("ctrl+s"), K("enter")]},
    {"name": "review_blocked", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [PRE(*CHANGE_FILLED, ("priority", "Urgent")), K("ctrl+s"), K("tab"), K("space"), K("shift+tab"),
             K("enter")]},
    {"name": "save_draft_dialog", "tenant": "acme", "kind": "change", "requester": JANE,
     "files": {"exists.yaml": 3},
     "ops": [PRE(("subject", "Hello")), K("f4"), K("ctrl+a"), T("@TMP@"), K("enter"), K("ctrl+a"),
             T("@TMP@/nodir/x.yaml"), K("enter"), K("ctrl+a"), K("backspace"), K("enter"), K("ctrl+a"),
             T("@TMP@/exists.yaml"), K("enter"), K("enter"), ]},
    {"name": "save_draft_escape", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [K("f4"), K("escape"), K("f4"), K("tab"), K("tab"), K("tab"), K("enter")]},
    {"name": "cancel_untouched", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [K("escape")]},
    {"name": "cancel_touched_keep", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [F("subject"), T("x"), K("escape"), K("tab"), K("enter"), K("escape"), K("shift+tab"), K("enter"),
             K("escape"), K("y")]},
    {"name": "cancel_dirty_discard", "tenant": "acme", "kind": "change", "requester": JANE,
     "ops": [F("subject"), T("x"), K("escape"), K("enter")]},
    {"name": "sr_form_keys", "tenant": "catalog", "kind": "sr", "item": 77, "requester": CJANE,
     "files": {"deck.pptx": 10, "brief.pdf": 10},
     "ops": [K("tab")] * 3 + [F("title_100"), T("Dark mode"), K("enter"), T("Please"), K("tab"), T("x"),
            K("tab"), T("2030-12-03"), K("enter"), K("tab"), K("down"), K("enter"), K("tab"), T("up"), K("tab"),
            K("enter"), K("down"), K("enter"), K("tab"), T("@TMP@/brief.pdf"), K("enter"), K("ctrl+a"),
            T("@TMP@/deck.pptx"), K("enter"), K("ctrl+s"), K("tab"), K("shift+tab"), K("escape")]},
    {"name": "ticket_form_keys", "tenant": "ticket", "kind": "ticket", "requester": None,
     "ops": [F("requester"), T("jane"), W(0.4), K("down"), K("enter"), F("subject"), T("Outage"), K("enter"),
             F("description"), T("Help"), W(0.4), F("cloud_only"), K("down"), K("enter"), K("ctrl+s"),
             K("escape")]},
    {"name": "edit_form_keys", "tenant": "acme", "kind": "change", "mode": "edit", "target": 900000501,
     "ops": [K("tab")] * 4 + [F("subject"), K("ctrl+a"), T("Billing v2"), K("enter"), F("description"), K("ctrl+a"),
            T("New text"), W(0.4), F("stage"), K("down"), K("enter"), F("cloud_only"), K("down"), K("enter"),
            K("ctrl+s"), K("tab"), K("escape")]},
]
