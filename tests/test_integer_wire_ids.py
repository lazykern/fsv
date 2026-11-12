"""Numeric identities stay JSON integers in the reviewed request, not just reads."""
import pytest
from fsv import change_flow, ticket_flow, integrated_assets
from fsv.drafts import Draft
from fsv.formmeta import FormMeta
from test_ticket_create import meta, draft, WHO
import acme_change_form as acme


@pytest.mark.parametrize("builder,metadata,answers", [
    (ticket_flow.build_create_plan, meta, draft),
    (change_flow.build_create_plan,
     lambda: FormMeta("change", "new_form", acme.fields(), acme.rules(), 2, 0, "fixture"),
     lambda: Draft(answers={"Subject": "Acme change", "Description": "Synthetic maintenance"})),
])
def test_create_device_wire_ids(builder, metadata, answers):
    plan = builder(metadata(), answers(), resolver=None, requester=WHO, devices=(81, 82))
    assert plan.payload is not None
    assert plan.payload["device_ids"] == [81, 82]
    assert all(type(i) is int for i in plan.payload["device_ids"])
    without = builder(metadata(), answers(), resolver=None, requester=WHO)
    if plan.ok:
        assert plan.hash != without.hash


@pytest.mark.parametrize("module", ["tickets", "sr", "changes", "problems"])
@pytest.mark.parametrize("kind,key", [("devices", "device_ids"), ("services", "businessapp_ids")])
@pytest.mark.parametrize("ids", [["81"], ["81", "82"]])
def test_association_single_bulk_numeric_ids(module, kind, key, ids):
    from types import SimpleNamespace
    client = SimpleNamespace(int_get=lambda *a, **k: {"data": [], "meta": {"total_count": 0, "total_pages": 0}})
    plan = integrated_assets.build_plan(client, module, 23, ids, kind=kind)
    assert plan.payload[key] == [int(i) for i in ids]
    assert all(type(i) is int for i in plan.payload[key])
