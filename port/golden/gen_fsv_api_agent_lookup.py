"""Golden agent autocomplete identities (Python e7c536b) for fsv-api."""
import json
import sys
from types import SimpleNamespace

from fsv import change_tasks, query
from fsv.agent_lookup import exact_agent_id
from provenance import stamped

NATIVE = {"id": "agent@example.com", "value": "Alex Acme", "user_id": 81}


def outcome(fn):
    try:
        return {"ok": fn()}
    except Exception as exc:
        return {"error": type(exc).__name__, "msg": str(exc)}


def client(rows):
    return SimpleNamespace(autocomplete=lambda *args: rows)


CASES = {
    "email_id": lambda: exact_agent_id(client([NATIVE]), "AGENT@example.com"),
    "email_key": lambda: exact_agent_id(
        client([{"email": "agent@example.com", "user_id": "81", "value": "Alex Acme"}]),
        "AGENT@example.com",
    ),
    "details": lambda: exact_agent_id(
        client([{"details": "agent@example.com", "user_id": 81}]),
        "agent@example.com",
    ),
    "empty": lambda: exact_agent_id(client([]), "agent@example.com"),
    "duplicate": lambda: exact_agent_id(client([NATIVE, NATIVE]), "agent@example.com"),
    "deleted": lambda: exact_agent_id(client([{**NATIVE, "deleted": True}]), "agent@example.com"),
    "inactive": lambda: exact_agent_id(client([{**NATIVE, "active": False}]), "agent@example.com"),
    "deactivated_label": lambda: exact_agent_id(
        client([{**NATIVE, "value": "Alex Acme (deactivated)"}]), "agent@example.com"
    ),
    "null_user": lambda: exact_agent_id(client([{**NATIVE, "user_id": None}]), "agent@example.com"),
    "bool_user": lambda: exact_agent_id(client([{**NATIVE, "user_id": True}]), "agent@example.com"),
    "other_email": lambda: exact_agent_id(
        client([{**NATIVE, "email": "other@example.com"}]), "agent@example.com"
    ),
    "bad_id": lambda: exact_agent_id(
        client([{**NATIVE, "id": "agent@example.com.invalid"}]), "agent@example.com"
    ),
    "name": lambda: exact_agent_id(client([NATIVE]), "ALEX ACME", name=True),
    "name_miss": lambda: exact_agent_id(client([NATIVE]), "Alex Globex", name=True),
    "owner_email": lambda: change_tasks.resolve_owner("AGENT@example.com", client([NATIVE])),
    "owner_name": lambda: change_tasks.resolve_owner("ALEX ACME", client([NATIVE])),
    "owner_me": lambda: change_tasks.resolve_owner("me", client([NATIVE])),
    "owner_ambiguous": lambda: change_tasks.resolve_owner(
        "agent@example.com", client([NATIVE, NATIVE])
    ),
    "query_email": lambda: query._resolve_agent(client([NATIVE]), "AGENT@example.com"),
    "query_ambiguous": lambda: query._resolve_agent(client([NATIVE, NATIVE]), "agent@example.com"),
    "unreadable": lambda: exact_agent_id(client(["x"]), "agent@example.com"),
}

json.dump(
    stamped({"cases": {name: outcome(fn) for name, fn in CASES.items()}}),
    sys.stdout,
    indent=1,
    ensure_ascii=False,
)
print()
