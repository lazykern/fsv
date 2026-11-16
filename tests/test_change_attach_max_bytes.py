"""changes update --attach --max-bytes: the total upload size limit (the
tenant's published limit by default) is checked before any upload and, on
the reviewed form path, bound into the plan hash. Acme fixtures, mocked client."""
import json

import httpx
import pytest
from typer.testing import CliRunner

import acme_change_form as A
import fsv.change_flow as flow
from fsv import cli


@pytest.fixture
def fake(monkeypatch, tmp_path):
    monkeypatch.delenv("FSV_TZ", raising=False)
    c = A.FakeClient()
    # the agent page publishes the limit in MiB
    c._send = lambda method, url, **kw: httpx.Response(
        200, text="<html><script>var MAX_ATTACHMENT_LIMIT = 15;</script></html>")
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.chdir(tmp_path)
    (tmp_path / "plan.pdf").write_bytes(b"pdf-bytes" * 20)
    return c


def run(*args):
    return CliRunner().invoke(cli.app, ["changes", "update", "CHN-501", *args])


def plan_hash(*args):
    r = run("--set", "Subject=Billing: rotate all keys", "--attach", "plan.pdf", *args, "--dry-run", "--json")
    assert r.exit_code == 0, r.output
    return json.loads(r.stdout)["plan_hash"]


def test_form_path_binds_the_limit_into_the_hash(fake):
    default = plan_hash()
    assert default and plan_hash("--max-bytes", "1000") != default
    assert plan_hash("--max-bytes", "1000") == plan_hash("--max-bytes", "1000")
    assert not fake.writes()


def test_form_path_refuses_over_the_limit_before_upload(fake):
    r = run("--set", "Subject=Billing: rotate all keys", "--attach", "plan.pdf", "--max-bytes", "100", "--yes")
    assert r.exit_code == 2 and "exceed the 100-byte limit" in " ".join(r.output.split())
    assert not fake.writes()


def test_quick_attach_path_refuses_over_the_limit(fake):
    fake.change["id"] = 501  # the reviewed attach path reads the exact owner
    r = run("--attach", "plan.pdf", "--max-bytes", "100", "--no-input", "--duplicate", "append", "--yes")
    assert r.exit_code == 2 and "100-byte limit" in " ".join(r.output.split())
    assert not fake.writes()
    ok = run("--attach", "plan.pdf", "--max-bytes", "1000", "--no-input", "--duplicate", "append", "--dry-run")
    assert ok.exit_code == 0, ok.output
    assert json.loads(ok.stdout)["attachment_size_limit"]["max_bytes"] == 1000
