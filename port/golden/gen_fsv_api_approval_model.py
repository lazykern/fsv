"""Golden `approval_model` for fsv-api `approval_model`: the legacy flat
approvals texts (`describe`, `NOT_SUPPORTED`, the groups refusal and the
per-action refusals).

    cd /tmp/fsv-py-<sha> && FSV_PY_SRC=$PWD FSV_PY_COMMIT=<sha> python \
        port/golden/gen_fsv_api_approval_model.py
"""
from __future__ import annotations

import json
import os
import sys
import tempfile

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import approval_model as am, change_approvals as ca  # noqa: E402
from fsv.errors import APIError  # noqa: E402
from provenance import stamped  # noqa: E402


class Absent:
    def __init__(self, status):
        self.status = status

    def int_get(self, path, params=None):
        raise APIError(self.status, {"error": "synthetic"}, "GET", path)


def groups(status):
    try:
        ca.get_change_approval_groups(5, Absent(status))
    except ca.LegacyChainsError as exc:
        return str(exc)


def refusal(action, status):
    return ca.LegacyChainsError(status, "x").refusal(action).args[0]


def require(action, status):
    try:
        am.require_chains(Absent(status), "tickets", 123, action)
    except am.LegacyApprovalsError as exc:
        return str(exc)


doc = {
    "describe": {"400": am.describe(400), "404": am.describe(404), "none": am.describe(None)},
    "not_supported": am.NOT_SUPPORTED,
    "groups_400": groups(400),
    "refusals": {"approval-request": refusal("approval-request", 400),
                 "approval-cancel": refusal("approval-cancel", 404),
                 "approve": refusal("approval-respond --approve", 400),
                 "reject": refusal("approval-respond --reject", 404),
                 "sr_cancel": require("sr approvals cancel", 404)},
}
json.dump(stamped(doc), sys.stdout, indent=1, ensure_ascii=False)
