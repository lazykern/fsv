"""Plan hashes (--expect-plan) and metadata fingerprints."""
from __future__ import annotations

import copy

import acme_forms
from fsv.forms import canonical_json, metadata_fingerprint, plan_hash, plan_matches


BODY = {"subject": "Rotate keys", "custom_fields": {"team": "Platform", "services": ["Billing"]},
        "email": "jane.doe@example.com", "br_validation_excludes": "eyJ.eyJ9"}


def test_plan_hash_is_stable_and_key_order_independent() -> None:
    h = plan_hash(BODY)
    assert len(h) == 64 and h == plan_hash(copy.deepcopy(BODY))
    reordered = {"email": BODY["email"], "br_validation_excludes": BODY["br_validation_excludes"],
                 "custom_fields": {"services": ["Billing"], "team": "Platform"}, "subject": "Rotate keys"}
    assert plan_hash(reordered) == h
    assert canonical_json({"b": 1, "a": "日本"}) == '{"a":"日本","b":1}'.encode()


def test_plan_hash_changes_with_any_sent_byte() -> None:
    h = plan_hash(BODY)
    for change in ({"subject": "Rotate keys "}, {"custom_fields": {"team": "Platform", "services": ["Search"]}},
                   {"br_validation_excludes": "eyJ.eyK9"}):
        assert plan_hash({**BODY, **change}) != h
    assert plan_hash({**BODY, "custom_fields": {**BODY["custom_fields"], "services": ("Billing",)}}) == h


def test_plan_hash_covers_file_content_and_name(tmp_path) -> None:
    deck = tmp_path / "deck.pptx"
    deck.write_bytes(b"v1")
    h1 = plan_hash(BODY, {"overview_deck": [deck]})
    assert h1 != plan_hash(BODY) and h1 == plan_hash(BODY, {"overview_deck": str(deck)})
    deck.write_bytes(b"v2")
    h2 = plan_hash(BODY, {"overview_deck": [deck]})
    assert h2 != h1
    other = tmp_path / "other.pptx"
    other.write_bytes(b"v2")
    assert plan_hash(BODY, {"overview_deck": [other]}) != h2  # the uploaded name is part of the plan
    assert plan_hash(BODY, [other]) == plan_hash(BODY, {"attachments": [other]})


def test_plan_matches_prefix() -> None:
    h = plan_hash(BODY)
    assert plan_matches(h, h) and plan_matches(h[:12].upper(), h) and plan_matches("sha256:" + h[:16], h)
    assert not plan_matches(h[:11], h) and not plan_matches("", h) and not plan_matches("0" * 64, h)


def test_metadata_fingerprint() -> None:
    fields, rules, item = acme_forms.ops_fields(), acme_forms.ops_rules(), acme_forms.service_item()
    rules["business_rules"][0]["updated_at"] = "2030-08-17T16:53:14+07:00"
    item["service_item"]["updated_at"] = "2030-08-17 10:00:00"  # naive = UTC: newer
    fp = metadata_fingerprint(fields, rules, item)
    assert fp.updated_at == "2030-08-17T10:00:00+00:00"
    assert dict(fp.counts) == {"form_fields": 13, "rules": 16, "item": 1}
    assert not fp.changed(metadata_fingerprint(copy.deepcopy(fields), copy.deepcopy(rules), copy.deepcopy(item)))
    edited = copy.deepcopy(rules)
    edited["business_rules"][3]["action_data"]["action"][0]["property"] = "unmandate"
    assert fp.changed(metadata_fingerprint(fields, edited, item))
    moved = copy.deepcopy(rules)
    moved["user"]["department_ids"] = [9000005001]  # rules can read the user record
    assert fp.changed(metadata_fingerprint(fields, moved, item))
    assert metadata_fingerprint().updated_at is None
