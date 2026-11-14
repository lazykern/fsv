"""Read baselines ignore delivery churn, not business content or wire payloads."""
import copy

import pytest

from fsv import journal
from fsv.reviewed_snapshot import stable_snapshot


def attachment(signature="first"):
    return {"id": 701, "name": "acme.txt", "size": 12, "content_type": "text/plain",
            "attachment_url": f"https://files.example.com/acme.txt?Expires=100&Signature={signature}&Key-Pair-Id=key",
            "canonical_url": f"https://files.example.com/delivery/{signature}",
            "created_at": signature, "updated_at": signature, "has_access": True}


def test_nested_snapshots_stable_and_source_untouched():
    before = {"parent": {"id": 123, "attachments": [attachment()]},
              "before": [{"id": 901, "body": "Acme note", "private": True, "attachments": [attachment()]}]}
    original = copy.deepcopy(before)
    fresh = copy.deepcopy(before)
    fresh["parent"]["attachments"] = [attachment("second")]
    fresh["before"][0]["attachments"] = [attachment("second")]
    assert journal.snapshot_hash(before) == journal.snapshot_hash(fresh)
    assert journal.plan_hash(before) != journal.plan_hash(fresh)
    assert before == original


@pytest.mark.parametrize("field,value", [("id", 702), ("name", "globex.txt"), ("size", 13),
                                        ("content_type", "image/png"), ("has_access", False)])
def test_attachment_identity_content_and_flags_remain_bound(field, value):
    before = {"attachments": [attachment()]}
    fresh = copy.deepcopy(before)
    fresh["attachments"][0][field] = value
    assert journal.snapshot_hash(before) != journal.snapshot_hash(fresh)


@pytest.mark.parametrize("field,value", [("body", "Changed"), ("private", False), ("source", 0),
                                        ("planned_start_date", "2030-01-03T10:00:00Z"),
                                        ("custom_fields", {"updated_at": "changed"})])
def test_business_fields_remain_bound(field, value):
    before = {"body": "Acme", "private": True, "source": 2,
              "planned_start_date": "2030-01-02T10:00:00Z", "custom_fields": {"updated_at": "original"}}
    assert journal.snapshot_hash(before) != journal.snapshot_hash({**before, field: value})


@pytest.mark.parametrize("query1,query2", [
    ("Expires=100&Signature=first&Key-Pair-Id=key", "Expires=200&Signature=second&Key-Pair-Id=other"),
    ("X-Amz-Date=first&X-Amz-Expires=100&X-Amz-Signature=first", "X-Amz-Date=second&X-Amz-Expires=200&X-Amz-Signature=second"),
])
def test_signed_rich_text_urls_only_strip_delivery_query(query1, query2):
    body = lambda q: '<div><a href="https://files.example.com/acme?version=1&amp;' + q.replace('&', '&amp;') + '">Acme</a></div>'
    first, second = body(query1), body(query2)
    assert journal.snapshot_hash({"body": first}) == journal.snapshot_hash({"body": second})
    assert journal.plan_hash({"body": first}) != journal.plan_hash({"body": second})
    assert journal.snapshot_hash({"body": first}) != journal.snapshot_hash({"body": second.replace("version=1", "version=2")})
    assert stable_snapshot("https://example.com/?Expires=100") == "https://example.com/?Expires=100"


def test_unchanged_legacy_payload_and_context_hashes_preserved():
    plain = {"id": 123, "body": "Acme", "private": True, "attachments": [{"id": 701, "name": "acme.txt", "size": 12}]}
    # Plan hashes bind the tenant on top of the unchanged canonical hash.
    from fsv import tenant
    assert journal.plan_hash(plain) == tenant.bind_plan(journal.snapshot_hash(plain))
    assert journal.plan_hash(None) is None
