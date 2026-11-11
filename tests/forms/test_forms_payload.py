from __future__ import annotations

import pytest

from fsv.forms import (
    BR_VALIDATION_EXCLUDES,
    FormSpec,
    FormState,
    LookupValue,
    PayloadError,
    RulesEngine,
    build_br_validation_excludes,
    build_change_payload,
    build_sr_payload,
    decode_br_validation_excludes,
    build_ticket_payload,
    format_requester,
    text_to_html,
)


def _filled_change(state: FormState) -> FormState:
    s = state
    s.set("requester", "jane.doe@acme.example")
    s.set("department", "Engineering")
    s.set("team", "Platform")
    s.set("group", 602)
    s.set("agent", "alex")
    s.set("services", ["Billing"])
    s.set("cloud_only", "No")  # populates Stage = Staging
    s.set("rollout_tool", "Manual")
    s.set("reviewers", [LookupValue(9003, "Sam Lee")])
    s.set("staging_start", "2030-11-18 09:00")
    s.set("staging_end", "2030-11-18 12:00")
    s.set("subject", "Billing: rotate keys")
    s.set("description", "<p>Rotate keys.</p>")
    s.set("needs_review", True)
    s.set("review_scope", "Code")
    s.set("planned_start_date", "2030-11-26 19:00")
    s.set("planned_end_date", "2030-11-26T22:00:00+07:00")
    s.set("downtime", "No")
    s.set("signoff_date", "01/12/2030")
    s.set("risk", "Medium")
    return s


def test_change_payload_shape(change_state: FormState) -> None:
    s = _filled_change(change_state)
    assert s.missing_required() == []
    body = build_change_payload(s, "jane.doe@acme.example", "Jane Doe")
    token = body.pop(BR_VALIDATION_EXCLUDES)
    assert token == build_br_validation_excludes(s.evaluated, "change")
    assert body == {
        "subject": "Billing: rotate keys",
        "description": "<p>Rotate keys.</p>",
        "priority": 1,
        "change_type": 1,
        "risk": 2,
        "impact": 1,
        "group_id": 602,
        "department_id": 501,
        "agent_id": 701,
        "planned_start_date": "2030-11-26T19:00:00+07:00",
        "planned_end_date": "2030-11-26T22:00:00+07:00",
        "custom_fields": {
            "team": "Platform",
            "services": ["Billing"],
            "cloud_only": "No",
            "stage": "Staging",
            "rollout_tool": "Manual",
            "reviewers": [9003],
            "staging_start": "2030-11-18T09:00:00+07:00",
            "staging_end": "2030-11-18T12:00:00+07:00",
            "needs_review": True,
            "review_scope": ["Code"],
            "downtime": "No",
            "signoff_date": "01/12/2030",
        },
        "email": "Jane Doe <jane.doe@acme.example>",
    }
    assert "status" not in body
    assert list(body)[:4] == ["subject", "description", "priority", "change_type"]


def test_change_payload_status_only_when_explicit(change_state: FormState) -> None:
    s = _filled_change(change_state)
    assert build_change_payload(s, "jane.doe@acme.example", status="Open")["status"] == 1
    assert build_change_payload(s, "jane.doe@acme.example", status=2)["status"] == 2
    with pytest.raises(PayloadError):
        build_change_payload(s, "jane.doe@acme.example", status="Done")


def test_change_payload_never_accepts_a_user_token(change_state: FormState) -> None:
    s = _filled_change(change_state)
    with pytest.raises(TypeError):
        build_change_payload(s, "jane.doe@acme.example", br_validation_excludes="opaque-token")  # type: ignore[call-arg]
    body = build_change_payload(s, "jane.doe@acme.example")
    assert decode_br_validation_excludes(body[BR_VALIDATION_EXCLUDES])["kind"] == "flat"


def test_change_payload_drops_hidden_values(change_state: FormState) -> None:
    s = _filled_change(change_state)
    s.set("change_type", "Emergency")
    s.set("emergency_reason", "Outage")
    s.set("category", "Hardware")
    body = build_change_payload(s, "jane.doe@acme.example")
    assert body["custom_fields"]["emergency_reason"] == "Outage"
    assert body["category"] == "Hardware"
    s.set("change_type", "Normal")  # hides emergency_reason and category again
    body = build_change_payload(s, "jane.doe@acme.example")
    assert "emergency_reason" not in body["custom_fields"] and "category" not in body
    s.set("cloud_only", "Yes")  # hides staging dates, reviewers, rollout tool
    body = build_change_payload(s, "jane.doe@acme.example", allow_incomplete=True)
    for gone in ("staging_start", "reviewers", "rollout_tool"):
        assert gone not in body["custom_fields"]
    assert body["custom_fields"]["stage"] == "Live"  # populated value follows the matching rule


def test_change_payload_uses_requester_from_state(change_state: FormState) -> None:
    s = _filled_change(change_state)
    assert build_change_payload(s)["email"] == "Jane Doe <jane.doe@acme.example>"


def test_change_payload_refuses_incomplete_or_blocked(change_state: FormState) -> None:
    with pytest.raises(PayloadError) as e:
        build_change_payload(change_state, "jane.doe@acme.example")
    assert "Team is required" in e.value.problems
    assert not any(p.startswith("Requester") for p in e.value.problems)  # requester comes from the argument
    draft = build_change_payload(change_state, "jane.doe@acme.example", allow_incomplete=True)
    assert draft["custom_fields"] == {} and draft["email"] == "jane.doe@acme.example"
    assert draft["change_type"] == 1 and "status" not in draft
    s = _filled_change(change_state)
    s.set("priority", "Urgent")
    with pytest.raises(PayloadError) as e2:
        build_change_payload(s, "jane.doe@acme.example")
    assert e2.value.problems == [
        "blocked by rule (acknowledge to continue): Urgent changes need the duty manager's approval"
    ]
    s.acknowledge(13)
    assert build_change_payload(s, "jane.doe@acme.example")["priority"] == 4


def test_change_payload_requires_requester(change_spec: FormSpec, change_rules: RulesEngine) -> None:
    st = FormState(change_spec, change_rules)
    with pytest.raises(PayloadError):
        build_change_payload(st, allow_incomplete=True)
    with pytest.raises(PayloadError):
        build_change_payload(st, "not-an-email", allow_incomplete=True)


def test_ticket_payload_shape(ticket_spec: FormSpec) -> None:
    st = FormState(ticket_spec)
    st.set("subject", "VPN drops")
    st.set("description", "It drops <every> hour.\nSince Monday.")
    st.set("department", "Engineering")
    st.set("issue", "Login")
    st.set("issue_system", "VPN")
    st.set("ticket_type", "Major Incident")
    st.set("customers_hit", "40")
    st.set("due_on", "2030-02-01")
    st.set("group", 77)
    body = build_ticket_payload(st, "sam.lee@acme.example", "Sam Lee")
    assert decode_br_validation_excludes(body.pop(BR_VALIDATION_EXCLUDES))["hidden"] == []
    assert body == {
        "subject": "VPN drops",
        "description": "<div>It drops &lt;every&gt; hour.<br>Since Monday.</div>",
        "department_id": 501,
        "priority": 1,
        "source": 3,
        "type": "Major Incident",
        "group_id": 77,
        "custom_fields": {"issue": "Login", "issue_system": "VPN", "customers_hit": 40, "due_on": "2030-02-01"},
        "email": "Sam Lee <sam.lee@acme.example>",
    }
    assert build_ticket_payload(st, "sam.lee@acme.example", status="Open")["status"] == 2


def test_sr_payload_shape(sr_spec: FormSpec) -> None:
    st = FormState(sr_spec)
    st.set("Title", "Dark mode")
    st.set("Details", "Please add dark mode.")
    st.set("Needed by", "2030-12-03")
    st.set("goal", "More revenue (Total USD)")
    st.set("goal_1_kpi", "+5%")
    st.set("Priority", "low - cosmetic, reports")
    body = build_sr_payload(sr_spec.item, st, "jane.doe@acme.example",
                            field_attachments={"Overview deck (.pptx)": [555]}, attachments=[556])
    assert list(body) == ["email", "custom_fields", BR_VALIDATION_EXCLUDES, "attachments"]
    token = body.pop(BR_VALIDATION_EXCLUDES)
    assert decode_br_validation_excludes(token)["items"] == {
        "4400001": {"optional": [], "disabled": [], "hidden": []}
    }
    assert body == {
        "email": "jane.doe@acme.example",
        "custom_fields": {
            "title_100": "Dark mode",
            "paragraph_101": "Please add dark mode.",
            "date_103": "2030-12-03",
            "goal": "More revenue  (Total USD)",
            "goal_1_kpi": "+5%",
            "priority": "Low - cosmetic, reports",
            "overview_deck": [555],
        },
        "attachments": [556],
    }
    with pytest.raises(TypeError):
        build_sr_payload(None, st, "jane.doe@acme.example", br_validation_excludes="t")  # type: ignore[call-arg]


def test_sr_payload_hidden_child_dropped(sr_spec: FormSpec) -> None:
    st = FormState(sr_spec)
    for k, v in {"title_100": "T", "paragraph_101": "D", "date_103": "2030-01-02", "priority": "Low - cosmetic, reports"}.items():
        st.set(k, v)
    st.set("goal", "More revenue (Total USD)")
    st.set("goal_1_kpi", "+5%")
    st.set("goal", "Other")
    body = build_sr_payload(None, st, "jane.doe@acme.example", field_attachments={"overview_deck": [1]},
                            attachments=[2])
    assert "goal_1_kpi" not in body["custom_fields"]


def test_sr_payload_attachment_rules(sr_spec: FormSpec) -> None:
    st = FormState(sr_spec)
    for k, v in {"title_100": "T", "paragraph_101": "D", "date_103": "2030-01-02", "goal": "Other",
                 "priority": "Low - cosmetic, reports"}.items():
        st.set(k, v)
    with pytest.raises(PayloadError) as e:
        build_sr_payload(None, st, "jane.doe@acme.example")
    assert "Overview deck (.pptx) is required" in e.value.problems
    assert "this item requires at least one attachment" in e.value.problems
    with pytest.raises(PayloadError) as e2:
        build_sr_payload(None, st, "jane.doe@acme.example", field_attachments={"title_100": [1]}, attachments=[2])
    assert any("not a file field" in p for p in e2.value.problems)
    st.set("overview_deck", "deck.pptx")  # chosen but not uploaded
    with pytest.raises(PayloadError) as e3:
        build_sr_payload(None, st, "jane.doe@acme.example", attachments=[2])
    assert "upload" in str(e3.value)
    assert sr_spec.item is not None
    sr_spec.item.allow_attachments = False
    with pytest.raises(PayloadError) as e4:
        build_sr_payload(None, st, "jane.doe@acme.example", field_attachments={"overview_deck": [1]},
                         attachments=[2])
    assert "this item does not accept attachments" in e4.value.problems


def test_sr_payload_needs_item(change_state: FormState) -> None:
    with pytest.raises(PayloadError):
        build_sr_payload(None, change_state, "jane.doe@acme.example")


def test_helpers() -> None:
    assert format_requester("a@acme.example") == "a@acme.example"
    assert format_requester(" a@acme.example ", " A B ") == "A B <a@acme.example>"
    assert text_to_html("<p>x</p>") == "<p>x</p>"
    assert text_to_html("a\n\nb & c") == "<div>a</div><div>b &amp; c</div>"
    assert text_to_html("") == "<div></div>"
