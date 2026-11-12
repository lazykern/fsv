"""Project SR recipient readbacks without confusing user IDs with emails."""
from typing import Any
from fsv.watchers import positive_id


def recipient_fields(row: dict[str, Any], body: dict[str, Any], identity: dict[str, Any]) -> dict[str, str]:
    fields = {}
    for key, party, id_key in (("email", "requester", "requester_id"),
                               ("requested_for", "requested_for", "requested_for_id")):
        if key not in body:
            continue
        matches = []
        expected_id = identity.get(id_key)
        # Every available representation must agree; an email echo cannot
        # overrule a conflicting authoritative ID or nested recipient.
        values = [row[name] for name in {key, party} if name in row]
        if id_key in row and expected_id is not None:
            values.append({"id": row[id_key]})
        for value in values:
            if isinstance(value, str):
                matches.append(value.casefold() == body[key].casefold())
            elif isinstance(value, dict):
                if "email" in value:
                    matches.append(isinstance(value["email"], str) and value["email"].casefold() == body[key].casefold())
                if "id" in value and expected_id is not None:
                    try:
                        matches.append(positive_id(value["id"]) == positive_id(expected_id))
                    except ValueError:
                        matches.append(False)
            elif value is not None:
                matches.append(False)
        fields[key] = "unavailable" if not matches else "matched" if all(matches) else "mismatched"
    if "cc_emails" in body:
        actual = row.get("cc_emails")
        fields["cc_emails"] = "unavailable" if "cc_emails" not in row else (
            "matched" if isinstance(actual, list) and all(isinstance(v, str) for v in actual)
            and sorted(v.casefold() for v in actual) == sorted(v.casefold() for v in body["cc_emails"]) else "mismatched")
    return fields
