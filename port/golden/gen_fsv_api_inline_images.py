"""Golden `inline_images` for fsv-api `inline_images`: plan hash and review of
an inline-image note and reply on both tenants of
`tests/test_inline_images.py`.

    cd /tmp/fsv-py-<sha> && FSV_PY_SRC=$PWD FSV_PY_COMMIT=<sha> python \
        port/golden/gen_fsv_api_inline_images.py
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import config, inline_images as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

PNG = b"\x89PNG\r\n\x1a\n" + b"synthetic-image-bytes"
out = {}
for domain in ("acme.example.com", "globex.example.com"):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    config.DOMAIN = domain
    config.API_INT = f"https://{domain}/api/_"
    tmp = Path(tempfile.mkdtemp(prefix="fsv-img-"))
    files = [tmp / "acme.png", tmp / "globex.gif"]
    for f in files:
        f.write_bytes(PNG)

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if path == "ticket_form_fields":
            return httpx.Response(200, json={"attachment_settings": {"max_attachment_size_bytes": 4096}})
        if path == "tickets/123":
            return httpx.Response(200, json={"ticket": {"id": 123, "attachments": []}})
        raise AssertionError(path)

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    for kind in ("note", "reply"):
        p = flow.plan(c, "INC-123", "Acme evidence", [str(f) for f in files], kind=kind,
                      sender="support@example.com" if kind == "reply" else None)
        out[f"{domain} {kind}"] = {"hash": p.hash, "review": p.review()}
json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False)
