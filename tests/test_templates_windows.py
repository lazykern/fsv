"""Ticket templates, maintenance windows and agent notify entries (offline)."""
import datetime as dt
import json
from zoneinfo import ZoneInfo

import httpx
import pytest
from typer.testing import CliRunner

import fsv.change_flow as change_flow
import fsv.cli as cli
from fsv import change_windows as W, config, journal, schema, templates as T
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_create import Client as CreateClient, meta

TEMPLATES = {"ticket_templates": [{"id": 501, "name": "Acme outage", "type": "shared", "workspace_id": 3},
                                  {"id": 502, "name": "Globex access", "type": "shared", "workspace_id": 3}],
             "meta": {"max_template_count": 2}}
TEMPLATE = {"ticket_template": {"id": 501, "name": "Acme outage", "template_data": {
    "subject": "Acme outage", "description": "<p>Please help</p>", "status": None, "tags": ["x"],
    "custom_fields": {"stage": "Live", "nested": {"a": 1}}}}}


class TemplateClient(CreateClient):
    def int_get(self, path, params=None):
        return {"ticket-templates": TEMPLATES, "ticket-templates/501": TEMPLATE}[path]


@pytest.fixture(autouse=True)
def isolated_journal(monkeypatch, tmp_path):
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal.jsonl")


def test_template_list_show_and_prefill_mapping():
    c = TemplateClient()
    assert [r["name"] for r in T.list_templates(c)] == ["Acme outage", "Globex access"]
    row = T.show(c, "acme OUTAGE")
    draft, applied, ignored = T.to_draft(row)
    assert draft.answers == {"subject": "Acme outage", "stage": "Live"}
    assert draft.description_html == "<p>Please help</p>"
    assert applied == ["subject", "stage", "description"] and ignored == ["nested", "tags"]
    with pytest.raises(T.TemplateError, match="no unique ticket template"):
        T.find(c, "Initech")


def test_create_with_template_goes_through_the_form_review(monkeypatch):
    c = TemplateClient()
    m = meta()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(change_flow, "interactive", lambda: False)
    monkeypatch.setattr("fsv.ticket_cli.load_meta", lambda *a, **kw: m)
    args = ["tickets", "create", "--requester", "jane.doe@example.com", "--template", "Acme outage",
            "--set", "Cloud Only=Yes"]
    res = CliRunner().invoke(cli.app, args + ["--dry-run", "--json"])
    assert res.exit_code == 0, res.output
    doc = json.loads(res.output[res.output.index("{"):])
    assert doc["payload"]["subject"] == "Acme outage" and "Please help" in doc["payload"]["description"]
    assert not c.writes
    res = CliRunner().invoke(cli.app, ["tickets", "create", "--requester", "jane.doe@example.com",
                                       "--template", "Initech", "--dry-run"])
    assert res.exit_code == 2 and not c.writes


class ReadbackTemplateClient(TemplateClient):
    stored = None

    def int_get(self, path, params=None):
        if path == "tickets/123":
            if self.stored is None:
                raise RuntimeError("synthetic read failure")
            return {"ticket": self.stored}
        return super().int_get(path, params)


@pytest.mark.parametrize("server_subject,expect", [("Acme outage (P2)", "server changed subject from Acme outage to Acme outage (P2)"),
                                                   ("Acme outage", None), (None, "could not read the ticket back")])
def test_template_create_reports_server_changed_prefill(monkeypatch, server_subject, expect):
    c = ReadbackTemplateClient()
    if server_subject is not None:
        c.stored = {"id": 123, "subject": server_subject, "custom_fields": {}}
    m = meta()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(change_flow, "interactive", lambda: False)
    monkeypatch.setattr("fsv.ticket_cli.load_meta", lambda *a, **kw: m)
    monkeypatch.setattr(change_flow, "load_meta", lambda *a, **kw: m)
    args = ["tickets", "create", "--requester", "jane.doe@example.com", "--template", "Acme outage",
            "--set", "Cloud Only=Yes", "--json"]
    review = CliRunner().invoke(cli.app, args + ["--dry-run"])
    digest = json.loads(review.stdout[review.stdout.index("{"):])["plan_hash"]
    res = CliRunner().invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert res.exit_code == 0, res.output
    doc = json.loads(res.stdout[res.stdout.index("{"):])
    check = doc["template_readback"]
    if expect is None:
        assert check["checked"] and check["changes"] == [] and "server changed" not in res.stderr
    else:
        assert expect in res.stderr
        if server_subject is not None:
            assert check["changes"] == [{"field": "subject", "sent": "Acme outage", "server": "Acme outage (P2)"}]
    assert len(c.writes) == 1


class AgentClient:
    def __init__(self, rows):
        self.rows, self.calls = rows, []

    def autocomplete(self, kind, query, params=None):
        self.calls.append((kind, query, params))
        return self.rows


def test_notify_agent_entry_is_name_and_email_from_agent_lookup():
    c = AgentClient([{"id": "robin@example.com", "value": "Robin Agent", "user_id": 77},
                     {"id": 8, "value": "Robin Agentson", "email": "ra@example.com"}])
    assert T.notify_entry(c, "Robin Agent") == "Robin Agent <robin@example.com>"
    assert c.calls[0] == ("agents", "Robin Agent", {"qf": "name"})
    assert T.notify_entry(c, "ra@example.com") == "Robin Agentson <ra@example.com>"
    for bad in ("Robin", "", "nobody@example.com"):
        with pytest.raises(T.TemplateError):
            T.notify_entry(c, bad)
    with pytest.raises(T.TemplateError):
        T.notify_entry(AgentClient([{"id": "x@example.com", "value": "Robin Agent", "active": False}]), "Robin Agent")


@pytest.fixture(params=["acme.example.com", "globex.example.com"])
def wapi(request, monkeypatch):
    domain = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr("fsv.formmeta.configured_workspace", lambda: None)
    sch = {"fields": [{"name": "change_type", "label": "Change Type", "workspace_id": 3,
                       "choices": [{"id": 1, "value": "Normal"}, {"id": 4, "value": "Emergency"}]}]}
    monkeypatch.setattr(schema, "load", lambda res, client=None, force=False: sch)
    monkeypatch.delenv("FSV_TZ", raising=False)
    s = {"windows": [{"id": 61, "name": "Globex freeze", "start_time": "2030-11-20T09:00:00Z",
                      "end_time": "2030-11-20T10:00:00Z"}], "writes": [], "overlaps": [],
         "agent": {"id": 7, "time_zone": "Eastern Time (US & Canada)"}}

    s["tenant_tz"] = "America/New_York"

    def stored(day, clock):
        when = dt.datetime.strptime(f"{day} {clock}", "%d-%m-%Y %I:%M%p")
        return when.replace(tzinfo=ZoneInfo(s["tenant_tz"])).isoformat()

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET" and path.endswith("agents/me"):
            return httpx.Response(200, json={"agent": s["agent"]}) if s["agent"] else httpx.Response(404, json={})
        if req.method == "GET":
            assert path == "maintenance_windows"
            return httpx.Response(200, json={"maintenance_windows": s["windows"]})
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, path, body))
        if path == W.OVERLAP_PATH:
            if "overlap_answer" in s:
                answer = s["overlap_answer"]
                if isinstance(answer, httpx.Response):
                    return answer
                return httpx.Response(200, json=answer)
            return httpx.Response(200, json={"overlapping_windows": s["overlaps"]})
        if req.method == "POST":
            # The tenant stores the wall clock in ITS zone (VERIFIED LIVE).
            cw = body["change_window"]
            row = {"id": 62, "name": cw["name"], "start_time": stored(cw["start_date"], cw["start_time"]),
                   "end_time": stored(cw["end_date"], cw["end_time"])}
            s["windows"].append(row)
            return httpx.Response(201, json={"maintenance_window": row})
        wid = int(path.rsplit("/", 1)[1])
        s["windows"] = [w for w in s["windows"] if w["id"] != wid]
        return httpx.Response(200, json={"success": True, "message": "delete_success"})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s
    c.close()


def _plan(c, **kw):
    return W.create_plan(c, "Acme release", "Quarterly", "2030-11-21 10:00", "2030-11-21 13:30", ["change_type=Normal"],
                         **kw)


def test_window_create_sends_overlap_check_then_captured_body(wapi):
    c, s = wapi
    p = _plan(c)
    assert p.write.payload == {"change_window": {
        "name": "Acme release", "description": "Quarterly", "start_date": "21-11-2030", "start_time": "10:00am",
        "end_date": "21-11-2030", "end_time": "1:30pm",
        "change_event": {"additional_info": {}, "criteria": [{"name": "change_type", "operator": "is", "value": "1"}]},
        "workspace_id": 3, "alert_suppression": True}}
    assert not s["writes"]
    r = W.send_create(p, c, reviewed_hash=p.hash, rebuild=lambda: _plan(c))
    assert r["verified"] and r["window_id"] == 62
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH, "maintenance_windows"]
    assert s["writes"][0][2]["type"] == "blackout_events"
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        W.send_create(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 2


def test_window_overlap_and_invalid_input_refuse(wapi):
    c, s = wapi
    s["overlaps"] = [{"id": 61}]
    p = _plan(c)
    with pytest.raises(ReviewedWriteError, match="overlapping"):
        W.send_create(p, c, reviewed_hash=p.hash)
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH]
    for kw in ({"criteria": []}, {"criteria": ["priority=High"]}, {"criteria": ["change_type=Minor"]},
               {"end": "2030-11-21 09:00"}, {"start": "21-11-2030 10:00"}):
        args = {"name": "x", "description": "", "start": "2030-11-21 10:00", "end": "2030-11-21 11:00",
                "criteria": ["change_type=Normal"], **kw}
        with pytest.raises(ReviewedWriteError):
            W.create_plan(c, args["name"], args["description"], args["start"], args["end"], args["criteria"])


def test_window_delete_and_cli(wapi):
    c, s = wapi
    runner = CliRunner()
    r = runner.invoke(cli.app, ["changes", "windows", "list", "--json"])
    assert r.exit_code == 0 and json.loads(r.output)[0]["id"] == 61
    r = runner.invoke(cli.app, ["changes", "windows", "delete", "61", "--dry-run"])
    assert r.exit_code == 0, r.output
    h = json.loads(r.output)["plan_hash"]
    r = runner.invoke(cli.app, ["changes", "windows", "delete", "61", "--yes", "--expect-plan", h])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["fields"] == {"absent": "matched"}
    assert s["writes"] == [("DELETE", "maintenance_windows/61", None)]
    r = runner.invoke(cli.app, ["changes", "windows", "create", "--name", "Acme", "--start", "2030-11-21 10:00",
                                "--end", "2030-11-21 11:00", "--criteria", "change_type=Emergency", "--dry-run"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["overlap_check"]["payload"]["criteria"][0]["value"] == "4"
    assert len(s["writes"]) == 1


@pytest.mark.parametrize("answer", [
    [], {}, False, {"overlapping_windows": []}, {"overlaps": []}, {"conflicts": [], "count": 0},
    {"blackout_windows": [], "maintenance_windows": []}, {"overlap": False}, {"is_overlapping": False},
    {"overlapping_windows": {"blackout_windows": [], "maintenance_windows": []}},
    {"match_found": False},  # native tenant answer (no windows to overlap)
    httpx.Response(200, text="false", headers={"content-type": "text/plain"}),
])
def test_window_overlap_answer_shapes_without_conflicts_create(wapi, answer):
    c, s = wapi
    s["overlap_answer"] = answer
    p = _plan(c)
    r = W.send_create(p, c, reviewed_hash=p.hash, rebuild=lambda: _plan(c))
    assert r["verified"] and r["overlaps"] == 0
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH, "maintenance_windows"]


@pytest.mark.parametrize("answer", [
    [{"id": 61}], True, {"has_overlap": True}, {"match_found": True}, {"overlap_count": 2}, {"conflicts": [{"id": 61}]},
    {"blackout_windows": [], "maintenance_windows": [{"id": 61}]},
    {"overlapping_windows": {"maintenance_windows": [{"id": 61}]}},
])
def test_window_overlap_answer_shapes_with_conflicts_refuse(wapi, answer):
    c, s = wapi
    s["overlap_answer"] = answer
    p = _plan(c)
    with pytest.raises(ReviewedWriteError, match="overlapping window"):
        W.send_create(p, c, reviewed_hash=p.hash)
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH]


@pytest.mark.parametrize("answer", [
    {"message": "Acme"}, {"status": "ok"}, {"overlap": False, "windows": [{"id": 61}]},
    {"match_found": "no"}, {"match_found": False, "windows": [{"id": 61}]}, {"matches": True}, {"a": {"b": {"c": []}}},
    httpx.Response(200), httpx.Response(200, text="Globex", headers={"content-type": "text/plain"}),
])
def test_window_overlap_unknown_answer_refuses_unless_allowed(wapi, answer):
    c, s = wapi
    s["overlap_answer"] = answer
    p = _plan(c)
    with pytest.raises(ReviewedWriteError, match="nothing was created") as exc:
        W.send_create(p, c, reviewed_hash=p.hash)
    assert "Acme" not in str(exc.value) and "Globex" not in str(exc.value)
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH]
    if "contradictory" in str(exc.value):
        return
    pa = _plan(c, allow_overlap=True)
    assert pa.hash != p.hash
    r = W.send_create(pa, c, reviewed_hash=pa.hash, rebuild=lambda: _plan(c, allow_overlap=True))
    assert r["verified"] and r["overlaps"] == "unknown"
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH, W.OVERLAP_PATH, "maintenance_windows"]


# The captured UI overlap check asks about blackout windows only; an existing
# maintenance window over the same hours must still refuse (live regression:
# window B overlapping window A was created while the check said no match).
@pytest.mark.parametrize("answer", [{"match_found": False}, [], {"overlapping_windows": []}])
@pytest.mark.parametrize("start,end", [
    ("2030-11-21T13:00:00-05:00", "2030-11-21T14:00:00-05:00"),   # same hours, offset form
    ("2030-11-21T18:00:00Z", "2030-11-21T19:00:00Z"),             # tail overlap, UTC form
    ("2030-11-21T15:30:00", "2030-11-21T16:00:00"),               # offset-less: overlaps when read as UTC
])
def test_window_existing_maintenance_overlap_refuses_whatever_the_check_says(wapi, answer, start, end):
    c, s = wapi
    s["windows"].append({"id": 63, "name": "Acme window A", "start_time": start, "end_time": end})
    s["overlap_answer"] = answer
    p = _plan(c)
    with pytest.raises(ReviewedWriteError, match=r"overlapping window\(s\) already listed \(63\)"):
        W.send_create(p, c, reviewed_hash=p.hash, rebuild=lambda: _plan(c))
    assert s["writes"] == []
    pa = _plan(c, allow_overlap=True)
    assert pa.hash != p.hash
    r = W.send_create(pa, c, reviewed_hash=pa.hash, rebuild=lambda: _plan(c, allow_overlap=True))
    assert r["verified"] and r["overlapping_window_ids"] == [63] and r["overlaps"] == 1
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH, "maintenance_windows"]


@pytest.mark.parametrize("answer", [{"match_found": False}, []])
def test_window_empty_list_and_back_to_back_windows_create(wapi, answer):
    c, s = wapi
    s["overlap_answer"] = answer
    s["windows"] = []
    p = _plan(c)
    r = W.send_create(p, c, reviewed_hash=p.hash, rebuild=lambda: _plan(c))
    assert r["verified"] and r["overlaps"] == 0 and r["overlapping_window_ids"] == []
    s["windows"] = [{"id": 64, "name": "Globex after", "start_time": "2030-11-21T18:30:00Z",
                     "end_time": "2030-11-21T20:00:00Z"}]
    s["writes"].clear()
    p = W.create_plan(c, "Acme second", "", "2030-11-21 10:00", "2030-11-21 13:30", ["change_type=Normal"])
    r = W.send_create(p, c, reviewed_hash=p.hash)
    assert r["verified"] and r["overlaps"] == 0
    assert p.review()["existing_window_check"]["rule"].startswith("any listed window")
    assert "America/New_York" in p.review()["existing_window_check"]["rule"]


@pytest.mark.parametrize("bad", [{"start_time": "a", "end_time": "b"}, {"start_time": None},
                                 {"start_time": "2030-11-21T10:00:00Z", "end_time": "2030-11-21T11:00:00"},
                                 {"start_time": "10:00am", "end_time": "11:00am"}])
def test_window_unreadable_existing_times_fail_closed(wapi, bad):
    c, s = wapi
    s["windows"] = [{"id": 65, "name": "Acme odd", **bad}]
    p = _plan(c)
    with pytest.raises(ReviewedWriteError, match="unreadable start/end") as exc:
        W.send_create(p, c, reviewed_hash=p.hash)
    assert "10:00" not in str(exc.value)
    assert s["writes"] == []
    pa = _plan(c, allow_overlap=True)
    r = W.send_create(pa, c, reviewed_hash=pa.hash)
    assert r["verified"] and r["overlaps"] == "unknown" and r["overlapping_window_ids"] == "unknown"


# --- time zones: the tenant reads the sent wall clock in the agent profile
# zone, whatever zone the user typed in (live regression: FSV_TZ moved the
# guard's zone but not the sent wall clock, giving a false 'no overlap' and a
# false refusal).
NY_A = {"id": 71, "name": "Acme A", "start_time": "2030-11-14T02:00:00-05:00", "end_time": "2030-11-14T04:00:00-05:00"}
NY_C = {"id": 72, "name": "Acme C", "start_time": "2030-11-14T04:00:00-05:00", "end_time": "2030-11-14T06:00:00-05:00"}


def _zplan(c, start, end, **kw):
    return W.create_plan(c, "Acme zone", "", start, end, ["change_type=Normal"], **kw)


@pytest.mark.parametrize("via", ["option", "env"])
def test_window_other_input_zone_converts_to_tenant_zone_before_check_and_send(wapi, monkeypatch, via):
    c, s = wapi
    s["windows"] = [dict(NY_A), dict(NY_C)]
    s["overlap_answer"] = {"match_found": False}
    kw = {"timezone": "Asia/Bangkok"} if via == "option" else {}
    if via == "env":
        monkeypatch.setenv("FSV_TZ", "Asia/Bangkok")
    # 15:00-17:00 +07:00 is 03:00-05:00 -05:00: overlaps A and C (was a false 'no overlap' live).
    p = _zplan(c, "2030-11-14 15:00", "2030-11-14 17:00", **kw)
    assert p.write.payload["change_window"]["start_time"] == "3:00am"
    assert p.write.payload["change_window"]["end_time"] == "5:00am"
    rv = p.review()
    assert rv["start"] == {"input": "2030-11-14 15:00 Asia/Bangkok (+07:00)",
                           "tenant": "2030-11-14 03:00 America/New_York (-05:00)"}
    assert rv["time_zone"]["input"].startswith("Asia/Bangkok (")
    assert rv["time_zone"]["tenant"].startswith("America/New_York")
    with pytest.raises(ReviewedWriteError, match=r"already listed \(71, 72\)"):
        W.send_create(p, c, reviewed_hash=p.hash)
    assert s["writes"] == []
    # 03:00-05:00 +07:00 is the previous day 15:00-17:00 -05:00: no overlap (was a false refusal live).
    p = _zplan(c, "2030-11-14 03:00", "2030-11-14 05:00", **kw)
    assert {k: p.write.payload["change_window"][k] for k in ("start_date", "start_time", "end_date", "end_time")} == {
        "start_date": "13-11-2030", "start_time": "3:00pm", "end_date": "13-11-2030", "end_time": "5:00pm"}
    r = W.send_create(p, c, reviewed_hash=p.hash, rebuild=lambda: _zplan(c, "2030-11-14 03:00", "2030-11-14 05:00", **kw))
    assert r["verified"] and r["overlapping_window_ids"] == [] and r["fields"] == {
        "name": "matched", "start": "matched", "end": "matched"}


def test_window_timezone_option_beats_env_and_offsets_are_accepted(wapi, monkeypatch):
    c, s = wapi
    monkeypatch.setenv("FSV_TZ", "Asia/Bangkok")
    p = _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00", timezone="Europe/London")
    assert p.write.payload["change_window"]["start_time"] == "3:00am"
    assert p.review()["time_zone"]["input"] == "Europe/London (--timezone)"
    q = _zplan(c, "2030-11-14 15:00+07:00", "2030-11-14T16:00+07:00", timezone="Europe/London")
    assert q.write.payload["change_window"]["start_time"] == "3:00am"
    monkeypatch.delenv("FSV_TZ")
    d = _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00")
    assert d.write.payload["change_window"]["start_time"] == "8:00am"
    assert d.review()["time_zone"]["input"] == "America/New_York (tenant time zone)"
    assert d.hash != _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00", timezone="America/New_York").hash


@pytest.mark.parametrize("agent", [None, {"id": 7}, {"id": 7, "time_zone": "Atlantis/Nowhere"}, {"id": 7, "time_zone": 5}])
def test_window_unknown_tenant_zone_refuses_even_with_fsv_tz(wapi, monkeypatch, agent):
    c, s = wapi
    s["agent"] = agent
    monkeypatch.setenv("FSV_TZ", "Asia/Bangkok")
    with pytest.raises(ReviewedWriteError, match="tenant time zone unknown"):
        _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00")
    r = CliRunner().invoke(cli.app, ["changes", "windows", "create", "--name", "Acme", "--start", "2030-11-14 08:00",
                                     "--end", "2030-11-14 09:00", "--criteria", "change_type=Normal", "--dry-run"])
    assert r.exit_code != 0 and "tenant time zone unknown" in r.output
    assert s["writes"] == []


@pytest.mark.parametrize("start,end,tz,match", [
    ("2030-03-10 02:30", "2030-03-10 04:00", None, "does not exist"),        # spring gap in the tenant zone
    ("2030-11-03 01:30", "2030-11-03 03:00", None, "happens twice"),         # fall-back hour typed in the tenant zone
    ("2030-11-03 12:30", "2030-11-03 14:00", "Asia/Bangkok", "happens twice there"),  # lands on the tenant's repeated hour
    ("2030-11-14 08:00", "2030-11-14 09:00", "Mars/Olympus", "unknown time zone"),
    ("2030-11-14 08:00", "2030-11-14 08:00", None, "after its start"),
    ("2030-11-14 09:00+00:00", "2030-11-14 15:30", "Asia/Bangkok", "after its start"),  # 09:00Z vs 08:30Z
    ("2030-11-14 8am", "2030-11-14 09:00", None, "must be 'YYYY-MM-DD HH:MM'"),
])
def test_window_dst_and_zone_edges_refuse(wapi, start, end, tz, match):
    c, s = wapi
    with pytest.raises(ReviewedWriteError, match=match):
        _zplan(c, start, end, timezone=tz)
    assert s["writes"] == []


def test_window_dst_offsets_follow_the_date(wapi):
    c, s = wapi
    summer = _zplan(c, "2030-07-01 20:00", "2030-07-01 21:00", timezone="Asia/Bangkok")
    winter = _zplan(c, "2030-12-01 20:00", "2030-12-01 21:00", timezone="Asia/Bangkok")
    assert summer.write.payload["change_window"]["start_time"] == "9:00am"   # -04:00
    assert winter.write.payload["change_window"]["start_time"] == "8:00am"   # -05:00
    assert summer.review()["start"]["tenant"].endswith("(-04:00)")


def test_window_readback_in_another_zone_is_not_verified(wapi):
    c, s = wapi
    s["tenant_tz"] = "UTC"  # the tenant stored it in a zone other than the agent profile says
    s["overlap_answer"] = {"match_found": False}
    p = _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00")
    r = W.send_create(p, c, reviewed_hash=p.hash)
    assert not r["verified"] and r["fields"]["start"] == "mismatched" and r["fields"]["name"] == "matched"


def test_window_legacy_hash_journal_still_guards(wapi):
    c, s = wapi
    p = _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00")
    assert p.legacy_hash and p.legacy_hash != p.hash
    journal.record_plan("started", p.legacy_hash, action="change_window_create", target=0)
    with pytest.raises(ReviewedWriteError, match="do not replay"):
        W.send_create(p, c, reviewed_hash=p.hash)
    assert s["writes"] == []


def test_window_cli_timezone_option_reviews_both_renderings(wapi):
    c, s = wapi
    r = CliRunner().invoke(cli.app, ["changes", "windows", "create", "--name", "Acme", "--start", "2030-11-14 15:00",
                                     "--end", "2030-11-14 17:00", "--criteria", "change_type=Normal",
                                     "--timezone", "Asia/Bangkok", "--dry-run"])
    assert r.exit_code == 0, r.output
    doc = json.loads(r.output)
    assert doc["end"] == {"input": "2030-11-14 17:00 Asia/Bangkok (+07:00)",
                          "tenant": "2030-11-14 05:00 America/New_York (-05:00)"}
    assert doc["payload"]["change_window"]["start_time"] == "3:00am"
    assert s["writes"] == []


# --- the plain dry-run evaluates the existing-window overlap (read-only GET)
@pytest.mark.parametrize("windows,ids", [
    ([], []),
    ([NY_A, NY_C], [71, 72]),                                                # both intersect 03:00-05:00
    ([NY_A], [71]),
    ([{**NY_C, "id": 73, "start_time": "2030-11-14T05:00:00-05:00", "end_time": "2030-11-14T06:00:00-05:00"}], []),  # touching
    ([{**NY_A, "id": 74, "start_time": "2030-11-14T01:00:00-05:00", "end_time": "2030-11-14T03:00:00-05:00"}], []),  # touching
    ([{**NY_A, "id": 75, "start_time": "2030-11-14T07:59:00Z", "end_time": "2030-11-14T08:01:00Z"}], [75]),  # other offset
])
def test_window_dry_run_reports_overlapping_ids(wapi, windows, ids):
    c, s = wapi
    s["windows"] = [dict(w) for w in windows]
    r = CliRunner().invoke(cli.app, ["changes", "windows", "create", "--name", "Acme", "--start", "2030-11-14 15:00",
                                     "--end", "2030-11-14 17:00", "--criteria", "change_type=Normal",
                                     "--timezone", "Asia/Bangkok", "--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    doc = json.loads(r.output)
    assert doc["status"] == "dry_run" and doc["overlapping_window_ids"] == ids
    if ids:
        assert doc["overlap_warning"] == (f"overlapping windows exist: {', '.join(map(str, ids))}; "
                                          "sending refuses unless --allow-overlap")
    else:
        assert "overlap_warning" not in doc
    assert s["writes"] == []  # neither the overlap POST nor the create ran
    if ids:
        r = CliRunner().invoke(cli.app, ["changes", "windows", "create", "--name", "Acme", "--start",
                                         "2030-11-14 15:00", "--end", "2030-11-14 17:00", "--criteria",
                                         "change_type=Normal", "--timezone", "Asia/Bangkok", "--yes",
                                         "--expect-plan", doc["plan_hash"]])
        assert r.exit_code != 0 and "already listed" in r.output and s["writes"] == []


def test_window_dry_run_unreadable_times_warn_unknown(wapi):
    c, s = wapi
    s["windows"] = [{"id": 76, "name": "Acme odd", "start_time": "soon", "end_time": "later"}]
    rv = _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00").review()
    assert rv["overlapping_window_ids"] == "unknown" and rv["overlap_warning"].startswith("overlap unknown")
    assert s["writes"] == []


# --- --allow-overlap is part of the reviewed plan (hash and warning)
def _cli(*extra):
    return CliRunner().invoke(cli.app, ["changes", "windows", "create", "--name", "Acme", "--start", "2030-11-14 15:00",
                                        "--end", "2030-11-14 17:00", "--criteria", "change_type=Normal",
                                        "--timezone", "Asia/Bangkok", *extra])


def test_window_allow_overlap_changes_hash_and_warns(wapi):
    c, s = wapi
    s["windows"] = [dict(NY_A), dict(NY_C)]
    s["overlap_answer"] = {"match_found": False}
    plain = json.loads(_cli("--dry-run").output)
    allowed = json.loads(_cli("--dry-run", "--allow-overlap").output)
    assert plain["plan_hash"] != allowed["plan_hash"]
    assert plain["allow_overlap"] is False and allowed["allow_overlap"] is True
    assert allowed["overlapping_window_ids"] == [71, 72]
    assert allowed["overlap_warning"] == ("overlapping windows exist: 71, 72; "
                                          "you chose to create it anyway (--allow-overlap)")
    # The flag cannot be added (or dropped) after review.
    r = _cli("--allow-overlap", "--yes", "--expect-plan", plain["plan_hash"])
    assert r.exit_code != 0 and s["writes"] == []
    r = _cli("--yes", "--expect-plan", allowed["plan_hash"])
    assert r.exit_code != 0 and s["writes"] == []
    r = _cli("--allow-overlap", "--yes", "--expect-plan", allowed["plan_hash"])
    assert r.exit_code == 0, r.output
    out = json.loads(r.output)
    assert out["verified"] and out["overlapping_window_ids"] == [71, 72]
    assert [w[1] for w in s["writes"]] == [W.OVERLAP_PATH, "maintenance_windows"]


def test_window_allow_overlap_without_listed_overlap_still_says_so(wapi):
    c, s = wapi
    s["windows"] = []
    rv = _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00", allow_overlap=True).review()
    assert rv["overlapping_window_ids"] == [] and rv["overlap_warning"].startswith("--allow-overlap:")
    s["windows"] = [{"id": 76, "name": "Acme odd", "start_time": "soon", "end_time": "later"}]
    rv = _zplan(c, "2030-11-14 08:00", "2030-11-14 09:00", allow_overlap=True).review()
    assert rv["overlap_warning"] == ("overlap unknown: a listed window has unreadable start/end times; "
                                     "you chose to create it anyway (--allow-overlap)")
