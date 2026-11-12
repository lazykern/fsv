"""Stable read-side baselines, never normalization of submitted write payloads."""
from __future__ import annotations

import html
import re
from typing import Any
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

# Attachment delivery locations and server bookkeeping are not file identity.
_ATTACHMENT_FIELDS = frozenset({
    "id", "name", "size", "content_type", "attachable_type", "parent_id",
    "has_access", "uploaded_by", "inline", "is_inline", "deleted", "is_deleted",
})
_VOLATILE_FIELDS = frozenset({
    "created_at", "updated_at", "last_viewed_at", "fetched_at", "expires_at", "url_expires_at",
})
_URL = re.compile(r"https?://[^\s<>\"']+")
_SIGNATURE_KEYS = frozenset({"signature", "policy", "key-pair-id", "awsaccesskeyid"})
_SIGNED_QUERY_KEYS = _SIGNATURE_KEYS | {"expires"}


def _stable_url(match: re.Match[str]) -> str:
    original = match.group()
    parsed = urlsplit(html.unescape(original))
    pairs = parse_qsl(parsed.query, keep_blank_values=True)
    if not any(k.casefold() in _SIGNATURE_KEYS or k.casefold().startswith("x-amz-") for k, _ in pairs):
        return original
    pairs = [(k, v) for k, v in pairs if k.casefold() not in _SIGNED_QUERY_KEYS
             and not k.casefold().startswith("x-amz-")]
    result = urlunsplit(parsed._replace(query=urlencode(pairs)))
    return result.replace("&", "&amp;") if "&amp;" in original else result


def stable_snapshot(value: Any, *, attachment: bool = False) -> Any:
    """Project server reads onto content/identity, keeping unknown business fields.

    File inventories use explicit stable fields, not signed URLs or timestamps.
    Other records retain all fields except server bookkeeping. Business dates,
    status/privacy flags, bodies, metadata and custom fields remain hash-bound.
    Custom-field names are tenant-defined: never interpret them as bookkeeping.
    Signed links embedded in rich text retain their path and non-signing query.
    Does not mutate the source used for review/display or independent readback.
    """
    if isinstance(value, dict):
        return {key: (item if key == "custom_fields" else
                      stable_snapshot(item, attachment=key in {"attachments", "files"}))
                for key, item in value.items()
                if (key in _ATTACHMENT_FIELDS if attachment else key not in _VOLATILE_FIELDS)}
    if isinstance(value, (list, tuple)):
        return [stable_snapshot(item, attachment=attachment) for item in value]
    if isinstance(value, str):
        return _URL.sub(_stable_url, value)
    return value
