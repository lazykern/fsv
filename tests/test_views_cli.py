"""Site-like lists and detail pages on the CLI (``ls``, ``views``, ``get``)
against two invented tenants over a mocked httpx transport (no network)."""
from __future__ import annotations

import datetime as dt
import json

import pytest
from typer.testing import CliRunner

import acme_views as W
import fsv.cli as cli
from fsv import views as V

runner = CliRunner()
FIXED_NOW = dt.datetime(2030, 6, 15, 12, tzinfo=dt.timezone.utc)


def _world(monkeypatch, tenant: str) -> W.World:
    w = W.World(tenant)
    c = w.client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(V, "_now", lambda: FIXED_NOW)
    monkeypatch.setattr(cli.console, "width", 150)
    if tenant == "globex":
        monkeypatch.setenv("FSV_TZ", "Europe/Berlin")  # agents/me is 403 there
    else:
        monkeypatch.delenv("FSV_TZ", raising=False)  # use the profile time zone
    return w


@pytest.fixture
def acme(monkeypatch):
    return _world(monkeypatch, "acme")


@pytest.fixture
def globex(monkeypatch):
    return _world(monkeypatch, "globex")


def run(*args: str):
    r = runner.invoke(cli.app, list(args))
    if r.exception and not isinstance(r.exception, SystemExit):
        raise r.exception
    r.flat = " ".join(r.output.split())
    return r


# --------------------------------------------------------------- lists --
def test_changes_ls_uses_default_view_site_columns_and_params(acme):
    r = run("changes", "ls")
    assert r.exit_code == 0, r.output
    assert "New & My Open Changes · 1-2 of 2 · Sort by: Created Date" in r.flat
    header = r.output.split("\n")[2]
    for label in ("Subject", "Requester", "Status", "Priority", "Assigned to", "Department"):
        assert label in header
    assert header.index("Subject") < header.index("Requester") < header.index("Assigned to") < header.index("Department")
    assert "#CHN-501" in r.flat and "Platform Ops / Robin Agent (Me)" in r.flat and "Engineering" in r.flat
    assert "Search: reindex catalogue" in r.flat and " None " in r.flat
    q = acme.calls("/api/_/changes")[-1]
    assert q["filter"] == "my_open"
    assert q["include"] == "change_status,agent,requester,group,department"
    assert (q["order_by"], q["order_type"]) == ("created_at", "desc")
    assert not acme.writes()


def test_tickets_ls_state_column_from_list_headers(acme):
    r = run("tickets", "ls")
    assert r.exit_code == 0, r.output
    assert "New & My Open Tickets · 1-2 of 2" in r.flat
    header = r.output.split("\n")[2]
    assert header.index("Requester") < header.index("State") < header.index("Status") < header.index("Assigned to")
    assert "Response Due" in r.flat and " New " in r.flat
    assert "Service Desk / None" in r.flat and "Service Desk / Robin Agent (Me)" in r.flat
    q = acme.calls("/api/_/tickets")[-1]
    assert q["filter"] == "new_and_my_open"
    assert q["include"] == "stats,responder,requester,ticket_states,ticket_status,group"


def test_view_by_unique_prefix_takes_the_views_sort(acme):
    r = run("changes", "ls", "--view", "my team")
    assert r.exit_code == 0, r.output
    assert "My Team's Changes · 1-2 of 2 · Sort by: Last Modified" in r.flat
    q = acme.calls("/api/_/changes")[-1]
    assert (q["filter"], q["order_by"], q["order_type"]) == ("9001", "updated_at", "asc")


def test_view_by_exact_name_and_unknown_view(acme):
    assert run("changes", "ls", "--view", "All Changes").exit_code == 0
    assert acme.calls("/api/_/changes")[-1]["filter"] == "all"
    r = run("changes", "ls", "--view", "Nope")
    assert r.exit_code == 2
    assert "no view named 'Nope'" in r.flat and "'New & My Open Changes'" in r.flat


def test_views_lists_names_with_the_default_marked(acme):
    r = run("changes", "views")
    assert r.exit_code == 0
    assert "▸ │ New & My Open Changes" in r.output
    assert "My Team's Changes" in r.flat and "custom" in r.flat
    doc = json.loads(run("changes", "views", "-o", "json").output)
    assert [v["name"] for v in doc if v["opens_by_default"]] == ["New & My Open Changes"]


def test_where_accepts_filter_panel_labels_and_special_values(acme):
    r = run("changes", "ls", "--where", "Agents=Me", "--where", "Created=Last 6 months",
            "--where", "Cloud Only=(empty)", "--where", "Department=Finance", "--where", "Groups=Unassigned")
    assert r.exit_code == 0, r.output
    qh = json.loads(acme.calls("/api/_/changes")[-1]["query_hash"])
    got = {(x["condition"], x["operator"]): x["value"] for x in qh}
    assert got[("agent_id", "is_in")] == ["0"]
    assert got[("created_at", "is_greater_than")] == "six_months"
    assert got[("cloud_only", "is_in")] == ["-1"]
    assert got[("department_id", "is_in")] == [7202]
    assert got[("group_id", "is_in")] == ["-1"]
    assert "filter" not in acme.calls("/api/_/changes")[-1]  # filters replace the default view


def test_where_internal_names_and_choice_labels_still_work(acme):
    r = run("changes", "ls", "--where", "status=Open", "--where", "Stage=Live")
    assert r.exit_code == 0, r.output
    qh = json.loads(acme.calls("/api/_/changes")[-1]["query_hash"])
    assert {x["condition"]: x["value"] for x in qh} == {"status": [1], "stage": ["Live"]}
    bad = run("changes", "ls", "--where", "Colour=Blue")
    assert bad.exit_code == 2 and "field not found: Colour" in bad.flat and "Cloud Only" in bad.flat


def test_ls_csv_and_json(acme):
    rows = run("changes", "ls", "-o", "csv").output.splitlines()
    assert rows[0] == "id,subject,requester,status,priority,assigned_to,department"
    assert rows[1].startswith("CHN-501,Billing: rotate keys,Jane Doe,Planning,Medium,Platform Ops / Robin Agent (Me)")
    doc = json.loads(run("changes", "ls", "-o", "json").output)
    assert [x["human_display_id"] for x in doc] == ["CHN-501", "CHN-502"]


# -------------------------------------------------------- change detail --
def test_change_get_reads_like_the_web(acme):
    r = run("changes", "get", "CHN-501")
    assert r.exit_code == 0, r.output
    out = r.output
    assert out.startswith("#CHN-501  Billing: rotate keys\nPlanning, due Sun, 30 Jun 2030 18:00")
    assert "✓ Open ›  Planning  › Awaiting Approval › Closed" in out
    assert "Jane Doe reported 2 days ago (Thu, 13 Jun 18:00)" in out  # profile time zone (Tokyo)
    tabs = "Details   Devops  Risk  Notes (1)  Tasks (1)  Assets  Impacted services  Associations (1)  Approvals (1)  Activity"
    assert tabs in out
    assert "Rotate the billing keys." in r.flat and "• staging first" in r.flat
    assert "Go-Live Checklist" in r.flat and "📎 runbook.pdf" in r.flat and "Reason for change Why now" in r.flat
    assert "Not requested" in out and "Resolution due by" in out and "(in 2w 1d)" in r.flat
    # Properties: form order, names not ids, edit-form rules decide visibility
    props = out[out.index("Properties"):]
    order = ["Requester", "Department", "Change Type", "Cloud Only", "Stage", "Services", "Downtime Window",
             "Priority", "Group", "Agent", "Status"]
    pos = [props.index(x) for x in order]
    assert pos == sorted(pos)
    assert "Staging Start" not in out and "Legacy Flag" not in out  # hidden by rules
    assert "Downtime Window*" in out  # required on the edit form when Stage is Live
    assert "Platform Ops" in props and "7101" not in out and "7201" not in out
    assert "Subject" not in props and "Description" not in props
    assert not acme.writes()


def test_change_get_json_keeps_record_and_adds_sections(acme):
    doc = json.loads(run("changes", "get", "CHN-501", "-o", "json").output)
    assert doc["status"] == 2 and doc["human_display_id"] == "CHN-501"
    view = doc["view"]
    assert view["status"] == "Planning"
    assert [t["name"] for t in view["tabs"]][:3] == ["details", "devops", "risk"]
    assert [s["name"] for s in view["lifecycle"]["states"] if s["mark"] == "current"] == ["Planning"]
    assert view["details"]["planning"][0] == {"name": "go_live_checklist", "label": "Go-Live Checklist", "help": "",
                                              "content": "Smoke test billing\n\n• login", "files": ["runbook.pdf"],
                                              "empty": False}
    assert {"label": "Approval", "value": "Not requested"} in view["summary"]
    assert {"label": "Resolution due by", "value": "Sun, 30 Jun 2030 18:00  (in 2w 1d)"} in view["summary"]
    props = {p["label"]: p["value"] for p in view["properties"]}
    assert props["Department"] == "Engineering" and props["Requester"] == "Jane Doe"
    assert "Staging Start" not in props


@pytest.mark.parametrize("tab,expect", [
    ("notes", "Robin Agent added a note · 🔒 private · a day ago"),
    ("tasks", "Take snapshot #3"),
    ("approvals", "✓ Kim Lee Approved"),
    ("assets", "billing-db Database · #42"),
    ("impacted", "none associated"),
    ("associations", "Tickets (1) #INC-601 VPN drops"),
    ("activity", "10:30 Robin Agent set Status as Planning"),
    ("risk", "2 factors considered"),
    ("devops", "Pull requests (0)"),
])
def test_change_tabs(acme, tab, expect):
    r = run("changes", "get", "CHN-501", "--tab", tab)
    assert r.exit_code == 0, r.output
    assert expect in r.flat
    assert "Properties" not in r.output  # a tab replaces the Details tab


def test_unknown_tab_lists_the_servers_tabs(acme):
    r = run("changes", "get", "CHN-501", "--tab", "resolution")
    assert r.exit_code == 2
    assert "tab 'resolution' is not shown for CHN-501" in r.flat and "impacted_services" in r.flat


def test_change_all_tabs(acme):
    r = run("changes", "get", "CHN-501", "--all-tabs")
    assert r.exit_code == 0
    for label in ("Devops ─", "Risk ─", "Notes ─", "Tasks ─", "Assets ─", "Impacted services ─", "Associations ─",
                  "Approvals ─", "Activity ─"):
        assert label in r.output
    assert "Properties" in r.output


# -------------------------------------------------------- ticket detail --
def test_service_request_page(acme):
    r = run("tickets", "get", "SR-602")
    assert r.exit_code == 0, r.output
    assert r.output.startswith("#SR-602  Access to analytics\nClosed (late)\n")
    assert "Jane Doe reported a month ago (Tue, 14 May 18:00) via Phone Request for: Self" in r.flat
    assert "Analytics workspace access Stage: Requested" in r.flat
    assert "Request type Add permission" in r.flat and "Workspace Sales" in r.flat and "Reason Quarterly report" in r.flat
    assert "Unused" not in r.flat
    assert "📎 screenshot.png (2.0 KB)" in r.flat
    assert "Conversations (5)" in r.flat and "… 2 earlier (use --all-conversations)" in r.flat
    assert "Robin Agent private note · 🔒 private" in r.flat and "Message 4" in r.flat and "Message 0" not in r.flat
    assert "Details Related tickets Tasks Assets Associations Approvals (1) Activity Resolution" in r.flat
    assert "Affected App Billing" in r.flat and "Time taken 1w 4d" in r.flat
    assert acme.calls("/api/_/tickets/602/conversations")[0]["per_page"] == "3"


def test_all_conversations_pages_through(acme):
    r = run("tickets", "get", "SR-602", "--all-conversations")
    assert r.exit_code == 0
    assert all(f"Message {i}" in r.flat for i in range(5)) and "earlier" not in r.flat
    assert r.flat.index("Message 0") < r.flat.index("Message 4")  # oldest first


def test_ticket_tabs(acme):
    assert "Resolved Sat, 25 May 2030 18:00" in run("tickets", "get", "SR-602", "--tab", "resolution").flat
    assert "Parent (0)" in run("tickets", "get", "SR-602", "--tab", "related").flat


# --------------------------------------------------------------- globex --
def test_globex_list_without_views_in_its_own_language(globex):
    r = run("changes", "ls")
    assert r.exit_code == 0, r.output
    assert "All changes · 1-1 of 1 · Sort by: Created Date" in r.flat
    header = r.output.split("\n")[2]
    for label in ("Betreff", "Anforderer", "Priorität", "Abteilung"):
        assert label in header
    assert "Drucker-Treiber aktualisieren" in r.flat and "Hoch" in r.flat and "Max Weber" in r.flat
    assert "filter" not in globex.calls("/api/_/changes")[-1]
    t = run("tickets", "ls")
    assert t.exit_code == 0 and "Overdue" in t.flat and "Niedrig" in t.flat
    assert run("changes", "views").exit_code == 0


def test_globex_change_page_degrades_gracefully(globex):
    r = run("changes", "get", "CHN-77")
    assert r.exit_code == 0, r.output
    assert r.output.startswith("#CHN-77  Drucker-Treiber aktualisieren\nOffen, due Thu, 20 Jun 2030 10:00\n")
    assert "›" not in r.output  # no lifecycle on this account
    assert "Lena Vogel reported a day ago (Fri, 14 Jun 10:00)" in r.flat  # FSV_TZ
    assert "Details Notes Tasks Approvals Activity" in r.flat  # fsv's fallback tabs
    assert "Planning" not in r.output
    assert "Umgebung Test" in r.flat and "Abteilung Vertrieb" in r.flat
    assert "· tabs: not permitted for your role (403)" in r.flat
    assert "static field visibility (business rules: not available on this account (404))" in r.flat
    assert "/api/_/business_rules" in globex.paths()
    tasks = run("changes", "get", "CHN-77", "--tab", "tasks")
    assert tasks.exit_code == 0 and "Tasks: not permitted for your role (403)" in tasks.flat
    doc = json.loads(run("changes", "get", "CHN-77", "--json").output)
    assert doc["view"]["lifecycle"] is None and doc["view"]["status"] == "Offen"


def test_globex_ticket_page(globex):
    r = run("tickets", "get", "SR-88")
    assert r.exit_code == 0, r.output
    assert "via Portal" not in r.flat or "Request for: #800003" in r.flat
    assert "requested items: not permitted for your role (403)" in r.flat
    assert "Lena Vogel replied · public" in r.flat and "Erledigt?" in r.flat
    assert "Bitte bis Freitag." in r.flat
