"""`--where type=NAME` comes from the tenant's ticket form metadata, and text
fields fail closed (the web has no list filter for them)."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, query
from fsv.resources import CHANGES, TICKETS
from test_views_cli import _world


def _schema(*choices):
    return {"fields": [
        {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": True},
        {"name": "status", "label": "Status", "field_type": "default_status", "default_field": True,
         "choices": [{"id": 2, "value": "Open"}]},
        {"name": "ticket_type", "label": "Type", "field_type": "default_ticket_type", "default_field": True,
         "choices": [dict(c) for c in choices]},
        {"name": "custom_notes", "label": "Notes", "field_type": "custom_paragraph"},
    ]}


ACME = _schema({"id": 4, "value": "Service Request"}, {"id": 3, "value": "Incident"})
# Another tenant: different ids, a renamed type, an extra type and a retired one.
GLOBEX = _schema({"id": 11, "value": "Service Request"}, {"id": 12, "value": "Change Request"},
                 {"id": 13, "value": "Outage"}, {"id": 14, "value": "Incident", "deleted": True})


def _built(sch, expr):
    return json.loads(query.build_query_hash_from_schema(None, TICKETS, sch, [expr]))


@pytest.mark.parametrize("sch,ident", [(ACME, "4"), (GLOBEX, "11")])
def test_type_name_and_id_resolve_from_each_tenants_metadata(sch, ident):
    for expr in ("type=Service Request", f"type={ident}"):
        assert _built(sch, expr) == [{"condition": "ticket_type", "operator": "is_in", "type": "default",
                                      "value": ["Service Request"]}]


def test_an_id_from_another_tenant_is_not_accepted():
    with pytest.raises(ValueError, match="no unique runtime choice '11'"):
        _built(ACME, "type=11")
    assert _built(GLOBEX, "type=Outage")[0]["value"] == ["Outage"]


@pytest.mark.parametrize("sch,expected,absent", [
    (ACME, "Service Request, Incident", "Outage"),
    (GLOBEX, "Service Request, Change Request, Outage", "Incident"),  # retired types are not offered
])
def test_unknown_type_lists_this_tenants_active_choices(sch, expected, absent):
    with pytest.raises(ValueError) as e:
        _built(sch, "type=Case")
    msg = str(e.value)
    assert f"this tenant's choices: {expected}" in msg and "nothing was sent" in msg
    assert absent not in msg.split("choices:")[1]


@pytest.mark.parametrize("expr", ["Subject=Acme", "subject!=Acme", "Notes=example", "custom_notes=example"])
def test_text_field_refusal_points_at_the_filterable_fields(expr):
    with pytest.raises(ValueError) as e:
        _built(ACME, expr)
    msg = str(e.value)
    assert msg.startswith("no list filter exists for text field") and "nothing was sent" in msg
    assert "filterable fields on this tenant: Status, Type" in msg
    assert "Subject" not in msg.split("filterable fields")[1] and "Notes" not in msg.split("filterable fields")[1]
    assert "`fsv tickets search TEXT`" in msg


def test_text_refusal_without_a_schema_names_the_field_kinds():
    msg = str(query.text_filter_refusal("changes", {"name": "subject", "label": "Subject"}))
    assert "choice, status, priority, group, agent, requester, ID, number or date field" in msg


def test_cli_unknown_type_exits_2_listing_choices_without_listing(monkeypatch):
    world = _world(monkeypatch, "acme")
    r = CliRunner().invoke(cli.app, ["tickets", "ls", "--where", "type=Case", "--json"])
    assert r.exit_code == 2, r.output
    assert "Incident" in r.output and "Service Request" in r.output
    assert not world.calls("/api/_/tickets") and not world.writes()


@pytest.mark.parametrize("tenant,module,expr", [
    ("acme", "tickets", "Subject=VPN"), ("acme", "changes", "Subject=Search"), ("globex", "changes", "Betreff=Drucker")])
def test_cli_text_filter_exits_2_and_names_supported_fields(monkeypatch, tenant, module, expr):
    world = _world(monkeypatch, tenant)
    r = CliRunner().invoke(cli.app, [module, "ls", "--where", expr, "--json"])
    assert r.exit_code == 2, r.output
    out = " ".join(r.output.split())
    assert "no list filter exists for text field" in out and "filterable fields on this tenant:" in out
    assert "Status" in out.split("filterable fields on this tenant:")[1]
    assert not world.calls(f"/api/_/{module}") and not world.writes()


def test_changes_type_uses_the_same_fail_closed_builder():
    with pytest.raises(ValueError, match="no list filter exists"):
        query.build_query_hash_from_schema(None, CHANGES, ACME, ["Subject=Acme"])
