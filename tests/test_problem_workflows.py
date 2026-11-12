"""Problem form and child workflows: synthetic two-tenant HTTP contracts."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, formmeta, journal, problem_workflows as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=[("acme.example.com", 41), ("globex.example.com", 73)])
def wire(request, monkeypatch, tmp_path):
    domain, base = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "CACHE_DIR", tmp_path / "cache")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(formmeta, "configured_workspace", lambda: 17)
    fields = [{"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "required": True},
              {"id": 502, "name": "description", "label": "Description", "field_type": "default_description", "required": True},
              {"id": 503, "name": "requester", "label": "Requester", "field_type": "default_requester", "required": True}]
    for i, name in enumerate(["status", "priority", "impact"]):
        fields.append({"id": 510 + i, "name": name, "label": name.title(), "field_type": "default_" + name,
                       "required": True, "choices": [{"id": base + i, "value": "Open" if name == "status" else "Low"}]})
    for index, field in enumerate(fields):
        field["position"] = index + 1
    state = {"fields": fields, "requests": [], "writes": [], "rows": {123: {"id": 123, "subject": "Acme"}},
             "notes": [], "tasks": [], "failure": None, "rules_denied": False, "readback": "ok", "omit_id": False,
             "task_fields": [{"name": "status", "choices": [{"id": base + 10, "value": "Open"}]},
                             {"name": "notify_before", "choices": [{"id": base + 11, "value": "Never"}]}]}

    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        state["requests"].append(path)
        if req.method == "GET":
            if path == "problem_form_fields":
                return httpx.Response(200, json={"problem_fields": state["fields"]})
            if path == "problem_task_fields":
                return httpx.Response(200, json={"problem_task_fields": state["task_fields"]})
            if path.endswith("business_rules"):
                state["rules_requests"] = state.get("rules_requests", 0) + 1
                if state.get("rules_status"):
                    return httpx.Response(state["rules_status"], json={"error": "synthetic"})
                return httpx.Response(403, json={"error": "denied"}) if state["rules_denied"] else httpx.Response(200, json={"business_rules": []})
            if "/search/autocomplete/" in path:
                return httpx.Response(200, json={"results": [{"id": 901, "value": "Acme Operator", "email": "person@example.com"}]})
            if path in {"problems/123/tasks", "problems/123/notes"}:
                key = path.rsplit("/", 1)[1]
                rows = copy.deepcopy(state[key])
                if rows and state["readback"] == "wrong":
                    rows[0]["id"] = 999
                if rows and state["readback"] == "missing":
                    rows[0].pop("body" if key == "notes" else "title")
                return httpx.Response(200, json={key: rows, "has_more": False})
            if path.startswith("problems/"):
                row = copy.deepcopy(state["rows"][int(path.split("/")[1])])
                if state["writes"] and state.get("normalize"):
                    state["normalize"](row)
                if state["writes"] and state["readback"] == "missing":
                    row.pop("email", None)
                return httpx.Response(200, json={"problem": row})
            raise AssertionError(path)
        body = json.loads(req.content)
        state["writes"].append((path, body))
        if state["failure"] == "disconnect":
            raise httpx.ReadError("synthetic disconnect", request=req)
        if state["failure"]:
            return httpx.Response(state["failure"], json={"error": "synthetic"})
        if path == "problems":
            state["rows"][456] = {"id": 456, **body}
            key = "problem"
        else:
            key = "note" if path.endswith("/notes") else "task"
            state[key + "s"].append({"id": 456, **body})
        return httpx.Response(201, json={} if state["omit_id"] else {key: {"id": 456}})

    clients = []
    def make():
        c = Client(cookies={})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(handle))
        monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
        monkeypatch.setattr(c, "me", lambda: {"id": 901})
        clients.append(c)
        return c
    state.update(client=make(), make=make, base=base)
    yield state
    for c in clients:
        c.close()


def create(w):
    return flow.create_plan(w["client"], {"Subject": "Acme problem", "Description": "Acme details", "Status": "Open", "Priority": "Low", "Impact": "Low"}, "person@example.com")


@pytest.mark.parametrize("actual,matched", [
    ({"acme_count": "7", "default_flag": None}, True),
    ({"acme_count": 7, "default_flag": False}, True),
    ({"acme_count": "8"}, False),
    ({"default_flag": None}, False),
    ({"acme_count": True}, False),
    (None, False),
])
def test_create_verifies_only_submitted_custom_fields(wire, actual, matched):
    wire["fields"].append({"id": 601, "name": "acme_count", "label": "Acme count",
                           "field_type": "custom_number", "position": 20})
    answers = {"Subject": "Acme problem", "Description": "Acme details", "Status": "Open",
               "Priority": "Low", "Impact": "Low", "Acme count": 7}
    plan = flow.create_plan(wire["client"], answers, "person@example.com")
    assert plan.payload["custom_fields"] == {"acme_count": 7}
    def normalize(row):
        row["custom_fields"] = actual
        row["description"] = "Acme details"
    wire["normalize"] = normalize
    result = flow.send(plan, wire["client"], reviewed_hash=plan.hash)
    assert result["verified"] is matched
    assert result["fields"]["custom_fields"] == ("matched" if matched else "mismatched")
    assert result["fields"]["description"] == "matched"
    assert len(wire["writes"]) == 1


def test_create_allows_server_custom_defaults_when_none_submitted(wire):
    plan = create(wire)
    wire["normalize"] = lambda row: row.update(custom_fields={"acme_default": None})
    assert flow.send(plan, wire["client"], reviewed_hash=plan.hash)["verified"]


def test_form_engine_create_and_exact_readback(wire):
    p = create(wire)
    assert p.payload == {"subject": "Acme problem", "description": "<div>Acme details</div>", "status": wire["base"],
                         "priority": wire["base"] + 1, "impact": wire["base"] + 2,
                         "custom_fields": {}, "email": "person@example.com"}
    result = flow.send(p, wire["client"], reviewed_hash=p.hash)
    assert result["verified"] and result["problem_id"] == 456
    assert wire["writes"] == [("problems", p.payload)]


@pytest.mark.parametrize("kind", ["note", "task"])
def test_captured_child_wire_exact_identity_and_metadata(wire, kind):
    p = flow.child_plan(wire["client"], "PRB-123", kind=kind, text="Acme")
    expected = {"body": "Acme"} if kind == "note" else {"title": "Acme", "owner_id": 901,
                "status": wire["base"] + 10, "notify_before": wire["base"] + 11, "planned_effort": "1m"}
    assert p.payload == expected
    assert flow.send(p, wire["client"], reviewed_hash=p.hash)["verified"]
    assert wire["writes"][0][1] == expected


@pytest.mark.parametrize("kind", ["create", "note", "task"])
@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_ambiguity_guard_and_replacement_client(wire, kind, failure):
    p = create(wire) if kind == "create" else flow.child_plan(wire["client"], 123, kind=kind, text="Acme")
    wire["failure"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(p, wire["client"], reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, wire["make"](), reviewed_hash=p.hash)
    assert len(wire["writes"]) == 1


@pytest.mark.parametrize("kind", ["create", "note", "task"])
@pytest.mark.parametrize("readback", ["missing", "no-id"])
def test_missing_fields_or_response_id_never_guesses(wire, kind, readback):
    p = create(wire) if kind == "create" else flow.child_plan(wire["client"], 123, kind=kind, text="Acme")
    wire["readback"], wire["omit_id"] = readback, readback == "no-id"
    assert not flow.send(p, wire["client"], reviewed_hash=p.hash)["verified"]
    assert len(wire["writes"]) == 1


def test_rules_unavailable_not_empty_and_metadata_drift_block(wire):
    p = create(wire)
    wire["fields"][0]["label"] = "Changed subject"
    with pytest.raises(ReviewedWriteError):
        flow.send(p, wire["client"], reviewed_hash=p.hash)
    wire["rules_denied"] = True
    # unavailable is never "no rules": the review says so, and answers that
    # only a rule could make valid (hidden on the static form) still block
    p = create(wire)
    assert any("rules unavailable" in w for w in p.review()["warnings"])
    wire["fields"].append({"id": 602, "name": "acme_hidden", "label": "Acme hidden", "field_type": "custom_text",
                           "position": 21, "displayed_to_agents": False})
    with pytest.raises(ReviewedWriteError, match="rules are unavailable and this write depends on them"):
        flow.create_plan(wire["client"], {"Subject": "Acme problem", "Description": "Acme details", "Status": "Open",
                                          "Priority": "Low", "Impact": "Low", "Acme hidden": "x"}, "person@example.com")
    assert not wire["writes"]


@pytest.mark.parametrize("status", [500, 503])
def test_rules_server_error_degrades_with_warning_and_is_not_cached(wire, monkeypatch, status):
    wire["rules_status"] = status
    p = create(wire)
    (warning,) = p.review()["warnings"]
    assert "rules unavailable" in warning and f"HTTP {status}" in warning
    before = wire["rules_requests"]
    meta = formmeta.load_meta(wire["client"], "problem", "new")
    assert meta.rules[formmeta.RULES_UNAVAILABLE_KEY] and formmeta.rules_server_error(meta.rules)
    assert wire["rules_requests"] > before  # a 5xx is not cached as the form's answer
    monkeypatch.setattr(cli, "_client", lambda: wire["client"])
    out = CliRunner().invoke(cli.app, ["problems", "create", "--requester", "person@example.com", "--dry-run",
                                       "--set", "Subject=Acme problem", "--set", "Description=Acme details",
                                       "--set", "Status=Open", "--set", "Priority=Low", "--set", "Impact=Low"])
    assert out.exit_code == 0, out.output
    assert "rules unavailable" in json.loads(out.output)["warnings"][0]
    assert not wire["writes"]


def test_fetch_rules_5xx_never_falls_back_to_another_workspace(monkeypatch):
    calls = []

    class Fake:
        def int_get(self, path, params=None):
            calls.append(path)
            if path.startswith("ws/2/"):
                raise formmeta.APIError(500, "boom", "GET", path)
            return {"business_rules": []}

    rules = formmeta.fetch_rules(Fake(), "problem", [2, 1], fallback=lambda: [1, 3])
    assert rules[formmeta.RULES_UNAVAILABLE_KEY] == "workspace 2 answered HTTP 500 (server error)"
    assert rules["unavailable_status"] == 500 and calls == [calls[0]]
    assert formmeta.rules_warnings(rules) and formmeta.rules_unavailable_warning(rules)


def test_problem_form_dump_and_create_cli(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire["client"])
    runner = CliRunner()
    dump = runner.invoke(cli.app, ["form", "dump", "problems"])
    assert dump.exit_code == 0, dump.output
    assert json.loads(dump.output)["module"] == "problem"
    args = ["problems", "create", "--requester", "person@example.com", "--json"]
    for key, value in [("Subject", "Acme"), ("Description", "Acme details"), ("Status", "Open"), ("Priority", "Low"), ("Impact", "Low")]:
        args += ["--set", key + "=" + value]
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2 and not wire["writes"]
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.output)["verified"]


def _native_task_fields(base, *, extra_required=None, status_id=None):
    def field(name, required="false", choices=None):
        row = {"name": name, "label": name.title(), "required": False,
               "field_options": {"im_api_name": name, "required_for_create": required, "required_for_closure": required}}
        if choices is not None:
            row["choices"] = choices
        return row
    fields = [field("title", "true"), field("description"), field("workspace_id", "true"), field("group_id"),
              field("owner_id"),
              field("status", choices=[{"id": str(base + 10) if status_id is None else status_id, "value": "Open"},
                                       {"id": str(base + 12), "value": "Completed"}]),
              field("notify_before", choices=[{"id": "0", "value": "Never"}, {"id": "900", "value": "15 Minutes"}]),
              field("planned_effort")]
    if extra_required:
        next(f for f in fields if f["name"] == extra_required)["field_options"]["required_for_create"] = "true"
    return fields


def test_native_task_metadata_string_choice_ids_seed_integer_wire(wire):
    wire["task_fields"] = _native_task_fields(wire["base"])
    p = flow.child_plan(wire["client"], "PRB-123", kind="task", text="Acme")
    assert p.payload == {"title": "Acme", "owner_id": 901, "status": wire["base"] + 10,
                         "notify_before": 0, "planned_effort": "1m"}
    assert type(p.payload["status"]) is int and type(p.payload["notify_before"]) is int
    assert flow.send(p, wire["client"], reviewed_hash=p.hash)["verified"]
    assert wire["writes"] == [("problems/123/tasks", p.payload)]


@pytest.mark.parametrize("variant", [{"extra_required": "group_id"}, {"status_id": "1a"}, {"status_id": True},
                                     {"status_id": "-1"}, {"status_id": None, "extra_required": "description"}])
def test_native_task_metadata_fails_closed_without_write(wire, variant):
    wire["task_fields"] = _native_task_fields(wire["base"], **variant)
    with pytest.raises(ReviewedWriteError):
        flow.child_plan(wire["client"], "PRB-123", kind="task", text="Acme")
    assert not wire["writes"]
