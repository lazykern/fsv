from __future__ import annotations

from fsv.resources import PROBLEMS


def test_problem_note_uses_internal_api(monkeypatch):
    import fsv.cli as cli

    calls = []

    class FakeClient:
        def int_post(self, path, body=None):
            calls.append(("int_post", path, body))
            return {"note": {"id": 123}}

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())

    cli.note_resource(PROBLEMS, "PRB-1", "hello", public=False)

    assert calls == [("int_post", "problems/1/notes", {"body": "hello", "private": True})]

    calls.clear()
    cli.note_resource(PROBLEMS, "PRB-1", "hello", public=True)

    assert calls == [("int_post", "problems/1/notes", {"body": "hello", "private": False})]


def test_problem_update_uses_internal_api(monkeypatch):
    import fsv.cli as cli

    calls = []

    class FakeClient:
        def int_put(self, path, body=None):
            calls.append(("int_put", path, body))
            return {"problem": {"id": 1, "human_display_id": "PRB-1"}}

        def v2_put(self, path, body=None):
            calls.append(("v2_put", path, body))
            return {"problem": {"id": 1, "human_display_id": "PRB-1"}}

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli.schema_mod, "load", lambda res, c: {
        "fields": [
            {"name": "priority", "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}]}
        ]
    })

    cli.update_resource(PROBLEMS, "PRB-1", None, "Medium", None, None, False, False, False)

    assert calls == [("int_put", "problems/1", {"priority": 2})]
