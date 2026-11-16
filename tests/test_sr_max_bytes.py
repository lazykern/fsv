"""``fsv sr create --max-bytes``: the upload size limit (the tenant's
published limit by default, or --max-bytes) is checked before any upload
and bound into the SR plan hash. Synthetic Acme catalog, mocked transport."""
from __future__ import annotations

import json

import pytest
from typer.testing import CliRunner

import acme_catalog as A
import fsv.change_flow as flow
import fsv.cli_sr as cli_sr

DRAFT = """requester: jane.doe@example.com
answers:
  Title: Dark mode
  Details: Please add dark mode.
  Needed by: 2030-12-03
  1. Expected benefit: More revenue (Total USD)
  1.1 Revenue target: "+5%"
  Priority: Low
  Overview deck (.pptx): deck.pptx
"""


@pytest.fixture
def fake(monkeypatch, tmp_path):
    f = A.AcmeCatalog()
    c = f.client()
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.chdir(tmp_path)
    (tmp_path / "deck.pptx").write_bytes(b"pptx-bytes")
    (tmp_path / "brief.pdf").write_bytes(b"pdf-bytes" * 10)
    (tmp_path / "sr.yaml").write_text(DRAFT)
    return f


def dry(*args):
    r = CliRunner().invoke(cli_sr.app, ["create", "77", "--from", "sr.yaml", *args, "--dry-run", "-o", "json"])
    return r


def test_tenant_limit_is_default_and_bound_into_the_hash(fake):
    r = dry("--attach", "brief.pdf")
    assert r.exit_code == 0, r.output
    default = json.loads(r.output)["plan_hash"]
    assert "GET /a/dashboard" in fake.paths()
    r = dry("--attach", "brief.pdf", "--max-bytes", "1000")
    assert r.exit_code == 0, r.output
    bound = json.loads(r.output)["plan_hash"]
    assert bound and bound != default
    assert dry("--attach", "brief.pdf", "--max-bytes", "1000").output == r.output  # stable


def test_over_the_limit_is_refused_before_any_upload(fake):
    r = dry("--attach", "brief.pdf", "--max-bytes", "50")
    out = json.loads(r.output)
    assert out["plan_hash"] is None and any("exceed the 50-byte limit" in e for e in out["errors"])
    sent = CliRunner().invoke(cli_sr.app, ["create", "77", "--from", "sr.yaml", "--attach", "brief.pdf",
                                           "--max-bytes", "50", "--yes"])
    assert sent.exit_code != 0 and fake.writes() == []


def test_send_with_the_bound_limit_uploads_once(fake):
    h = json.loads(dry("--attach", "brief.pdf", "--max-bytes", "1000").output)["plan_hash"]
    r = CliRunner().invoke(cli_sr.app, ["create", "77", "--from", "sr.yaml", "--attach", "brief.pdf",
                                        "--max-bytes", "1000", "--yes", "--expect-plan", h])
    assert r.exit_code == 0, r.output
    assert [p for _m, p, _b in fake.writes()].count("/api/_/attachments") == 2
    stale = CliRunner().invoke(cli_sr.app, ["create", "77", "--from", "sr.yaml", "--attach", "brief.pdf",
                                            "--max-bytes", "2000", "--yes", "--expect-plan", h])
    assert stale.exit_code != 0
