"""Synthetic native planning detail responses, not request echoes."""
import pytest

from fsv import planning_text as planning
from test_planning_text import Client, setup  # noqa: F401


@pytest.mark.parametrize("module,section,key", [
    ("changes", "Rollout Plan", "acme_plan"),
    ("problems", "Root Cause", "problem_cause"),
])
@pytest.mark.parametrize("stored,expected", [
    ("Acme & Globex details", "matched"),
    ("<p>Acme &amp; Globex&nbsp; details</p>", "matched"),
    ("Acme wrong details", "mismatched"),
    (None, "unavailable"),
    (42, "unavailable"),
])
def test_native_content_readback(module, section, key, stored, expected):
    class NativeClient(Client):
        def _request(self, method, path, **kw):
            result = super()._request(method, path, **kw)
            self.rows = [{"name": key, "label": section, "content": stored,
                          "files": [], "empty": False}]
            return result

    client = NativeClient(module)
    client.rows = [{"name": key, "label": section, "content": "", "files": [], "empty": True}]
    plan = planning.build_plan(client, module, 123, section, "<div>Acme &amp; Globex\n details</div>")
    result = planning.send(plan, client, reviewed_hash=plan.hash)
    assert result["description"] == expected
    assert result["verified"] is (expected == "matched")
    assert len(client.writes) == 1


def test_normalization_preserves_case_and_word_boundaries():
    from fsv.readback import same_text

    assert same_text("Acme Globex", "<div>Acme</div><div>Globex</div>")
    assert not same_text("AcmeGlobex", "<div>Acme</div><div>Globex</div>")
    assert not same_text("acme", "Acme")
    assert not same_text(None, "")
