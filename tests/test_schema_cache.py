from __future__ import annotations

import json

import pytest

import fsv.config as config
import fsv.schema as schema
from fsv.cache import save
from fsv.errors import APIError, SessionError
from fsv.resources import CHANGES

FIELDS = [{"name": "status", "label": "Status", "choices": [{"id": 1, "value": "Open"}]}]


class FakeClient:
    def __init__(self, internal=None, v2=None):
        self.internal, self.v2, self.calls = internal, v2, 0

    def _run(self, what):
        self.calls += 1
        if isinstance(what, Exception):
            raise what
        return what

    def int_get(self, path):
        return self._run(self.internal)

    def v2_get(self, path):
        return self._run(self.v2)


def _cache_file():
    return config.schema_cache_path(CHANGES.name)


def test_both_fetches_fail_raises_session_error_and_caches_nothing():
    c = FakeClient(internal=SessionError("expired"), v2=APIError(403, "x"))
    with pytest.raises(SessionError):
        schema.load(CHANGES, c)
    assert not _cache_file().exists()


def test_api_errors_propagate_when_no_session_error():
    c = FakeClient(internal=APIError(500, "boom"), v2=APIError(403, "nope"))
    with pytest.raises(APIError):
        schema.load(CHANGES, c)
    assert not _cache_file().exists()


def test_both_empty_raises_and_is_not_cached():
    c = FakeClient(internal={CHANGES.form_fields_key: []}, v2={CHANGES.form_fields_key: []})
    with pytest.raises(APIError):
        schema.load(CHANGES, c)
    assert not _cache_file().exists()


def test_v2_fallback_used_when_internal_fails():
    c = FakeClient(internal=APIError(404, "x"), v2={CHANGES.form_fields_key: FIELDS})
    assert schema.load(CHANGES, c)["fields"] == FIELDS
    assert json.loads(_cache_file().read_text())["data"] == FIELDS


@pytest.mark.parametrize("payload", [[], None, {"oops": 1}])
def test_poisoned_cache_entry_is_a_miss(payload):
    config.ensure_dirs()
    save(_cache_file(), "schema", payload)
    c = FakeClient(internal={CHANGES.form_fields_key: FIELDS}, v2={CHANGES.form_fields_key: []})
    assert schema.load(CHANGES, c)["fields"] == FIELDS
    assert c.calls == 2  # refetched, not served from the poisoned file
    assert json.loads(_cache_file().read_text())["data"] == FIELDS


def test_valid_cache_is_served_without_fetch():
    config.ensure_dirs()
    save(_cache_file(), "schema", FIELDS)
    c = FakeClient(internal=SessionError("must not be called"))
    assert schema.load(CHANGES, c)["fields"] == FIELDS
    assert c.calls == 0


def test_unwrap_keeps_fields_key_for_empty_list():
    out = schema._unwrap({"saved_at": 5, "data": []})
    assert out == {"saved_at": 5, "fields": []}
    assert schema._unwrap({"fields": FIELDS})["fields"] == FIELDS
