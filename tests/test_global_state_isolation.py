"""Process-global state is reset for every test (tests/conftest.py).

The two tests run in file order: the first leaves the state dirty the way a
crashed or unusual command could, the second must still start clean.
"""
from fsv import client, tenant


def test_leaves_global_state_dirty():
    bound = tenant.bind_plan("a" * 64, "globex.freshservice.com")
    tenant._state["depth"] = 3
    tenant._state["write"] = True
    client.set_verbose(True)
    assert tenant._TENANT_OF[bound] == "globex.freshservice.com" and tenant._RAW_OF[bound] == "a" * 64


def test_next_test_starts_clean():
    assert tenant._TENANT_OF == {} and tenant._RAW_OF == {}
    assert tenant._state == {"depth": 0, "write": False, "new": []}
    assert client.is_verbose() is False
    # A leaked depth would make the tenant hook skip its checks; the next
    # command must run them again.
    bound = tenant.bind_plan("a" * 64, "globex.freshservice.com")
    assert tenant.reviewed_elsewhere(bound) == "globex.freshservice.com"
