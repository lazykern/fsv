"""Which approval model a tenant runs: parallel approval chains/groups, or the
legacy flat per-member approvals list.

Tenants without the parallel-approvals feature answer
``GET <module>/<id>/approval-chains`` with 400 ("feature needs to be
enabled") or 404. They still expose the flat ``<module>/<id>/approvals``
list (rows with ``member_id``, ``level_id``, ``status{id,name}``,
``type{id,name}``), so read-only listing works on both models, while the
group-based writes (named group request, group cancel, decisions verified
through chains) do not apply there and must refuse clearly instead of failing
on an unexpected 400.
"""
from __future__ import annotations

from typing import Any

from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError

CHAINS = "chains"
LEGACY = "legacy"
UNKNOWN = "unknown"
LEGACY_LABEL = "legacy flat approvals"
NOT_SUPPORTED = f"{LEGACY_LABEL}: not yet supported"
_ABSENT = (400, 404)


class LegacyApprovalsError(ReviewedWriteError):
    """The tenant has no parallel approval chains (legacy flat approvals)."""

    def __init__(self, message: str, status: int | None = None) -> None:
        super().__init__(message)
        self.status = status


def is_absent(exc: BaseException) -> bool:
    """An approval-chains error that means "no parallel approvals here"."""
    return isinstance(exc, APIError) and exc.status in _ABSENT


def describe(status: int | None) -> str:
    return f"this tenant uses {LEGACY_LABEL} (approval-chains answered HTTP {status if status is not None else '?'})"


def detect(client: Any, module: str, target: int) -> tuple[str, int | None]:
    """``(CHAINS, None)``, ``(LEGACY, status)`` or ``(UNKNOWN, status)`` from
    one read-only GET.

    Only 400/404 prove the legacy model. Any other API error (403, 5xx) is
    ``UNKNOWN``: callers keep their existing chains-driver path, which has
    its own fail-closed inventory checks. Session errors propagate."""
    if module not in ("changes", "tickets"):
        raise ValueError("unsupported approval module")
    try:
        client.int_get(f"{module}/{target}/approval-chains", params={"page": 1, "per_page": 100})
    except APIError as exc:
        if exc.status in _ABSENT:
            return LEGACY, exc.status
        return UNKNOWN, exc.status
    return CHAINS, None


def require_chains(client: Any, module: str, target: int, action: str) -> None:
    """Refuse a chains/groups-only action on a proven legacy tenant, naming
    the model."""
    model, status = detect(client, module, target)
    if model == LEGACY:
        raise LegacyApprovalsError(f"{action} refused: {describe(status)}; {NOT_SUPPORTED}", status)
