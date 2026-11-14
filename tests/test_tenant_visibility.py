"""The target tenant is impossible to miss: one stderr line per tenant command,
tenant/config_dir in write JSON, tenant-bound plan hashes, and a named refusal
for a plan reviewed on another tenant."""
import hashlib
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, tenant
from fsv.forms.plan import plan_hash as form_plan_hash
from test_ticket_notes import mock_client

ACME, GLOBEX = "acme.example.com", "globex.example.com"


def _use(monkeypatch, domain):
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")


@pytest.fixture
def fake(monkeypatch, tmp_path):
    _use(monkeypatch, ACME)
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"spam": {32: False}, "writes": []}

    def handler(req):
        tid = int(req.url.path.removeprefix("/api/_/").split("/")[1])
        if req.method == "GET":
            return httpx.Response(200, json={"ticket": {"id": tid, "spam": s["spam"][tid], "deleted": False}})
        s["writes"].append(req.url.host)
        s["spam"][tid] = True
        return httpx.Response(204)

    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: c)
    return s


def run(*args, **env):
    return CliRunner().invoke(cli.app, list(args), env=env or None)


def test_banner_once_on_stderr_and_stdout_stays_pure_json(fake, monkeypatch):
    monkeypatch.delenv("FSV_QUIET")
    r = run("tickets", "spam", "32", "--dry-run")
    assert r.exit_code == 0, r.output
    lines = [x for x in r.stderr.splitlines() if x.startswith("fsv: tenant")]
    assert lines == [f"fsv: tenant {ACME} (config {config.display_path(config.CONFIG_DIR)})"]
    review = json.loads(r.stdout)  # nothing but JSON on stdout
    assert review["tenant"] == ACME and review["config_dir"] == str(config.CONFIG_DIR)
    assert "fsv: tenant" not in r.stdout


@pytest.mark.parametrize("args,env", [(("--quiet", "tickets", "spam", "32", "--dry-run"), {}),
                                      (("tickets", "spam", "32", "--dry-run", "--quiet"), {}),
                                      (("tickets", "spam", "32", "--dry-run"), {"FSV_QUIET": "1"})])
def test_quiet_flag_and_env_suppress_the_banner(fake, monkeypatch, args, env):
    monkeypatch.delenv("FSV_QUIET")
    r = run(*args, **env)
    assert r.exit_code == 0, r.output
    assert "fsv: tenant" not in r.output
    json.loads(r.stdout)


@pytest.mark.parametrize("args", [("config", "list"), ("journal", "path"), ("--help",),
                                  ("tickets", "spam", "--help"), ("--version",), ("completion", "show", "bash")])
def test_local_commands_print_no_banner(monkeypatch, args):
    monkeypatch.delenv("FSV_QUIET")
    r = run(*args)
    assert not [x for x in r.output.splitlines() if x.startswith("fsv: tenant")]


def test_result_json_names_tenant_and_config_dir(fake):
    digest = json.loads(run("tickets", "spam", "32", "--dry-run").stdout)["plan_hash"]
    done = run("tickets", "spam", "32", "--yes", "--expect-plan", digest, "--confirm-destructive")
    assert done.exit_code == 0, done.output
    result = json.loads(done.stdout)
    assert (result["tenant"], result["config_dir"]) == (ACME, str(config.CONFIG_DIR))
    assert fake["writes"] == [ACME]


def test_read_json_is_not_annotated(monkeypatch):
    assert tenant.annotate({"id": 1}) == {"id": 1}  # outside a write command
    assert tenant.tenant_json([1]) == "[1]"


def test_plan_hash_input_includes_the_tenant(monkeypatch):
    body = {"status": 4}
    _use(monkeypatch, ACME)
    on_acme, form_acme = journal.plan_hash(body), form_plan_hash(body)
    _use(monkeypatch, GLOBEX)
    assert journal.plan_hash(body) != on_acme
    assert form_plan_hash(body) != form_acme
    raw = hashlib.sha256(json.dumps(body, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    assert on_acme == tenant.bind_plan(raw, ACME)


def test_plan_reviewed_on_another_tenant_is_refused_by_name(fake, monkeypatch):
    digest = json.loads(run("tickets", "spam", "32", "--dry-run").stdout)["plan_hash"]
    _use(monkeypatch, GLOBEX)
    r = run("tickets", "spam", "32", "--yes", "--expect-plan", digest, "--confirm-destructive",
            "--tenant", GLOBEX)
    assert r.exit_code == 2
    assert f"plan {digest[:12]} was reviewed on tenant {ACME}" in r.stderr
    assert GLOBEX in r.stderr and fake["writes"] == []


def test_cross_tenant_refusal_survives_a_new_process(fake, monkeypatch):
    digest = json.loads(run("tickets", "spam", "32", "--dry-run").stdout)["plan_hash"]
    monkeypatch.setattr(tenant, "_TENANT_OF", {})  # only the on-disk registry remains
    _use(monkeypatch, GLOBEX)
    assert tenant.reviewed_elsewhere(digest) == ACME
    assert tenant.reviewed_elsewhere(digest[:12]) == ACME
    _use(monkeypatch, ACME)
    assert tenant.reviewed_elsewhere(digest) is None


def test_tenant_option_must_match_configured_domain(fake):
    r = run("tickets", "spam", "32", "--dry-run", "--tenant", GLOBEX)
    assert r.exit_code == 2 and f"--tenant {GLOBEX} does not match the configured tenant {ACME}" in r.stderr
    assert run("tickets", "spam", "32", "--dry-run", f"--tenant={ACME}").exit_code == 0


def test_legacy_journal_hashes_still_guard_replays(monkeypatch, tmp_path):
    _use(monkeypatch, ACME)
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    child = {"id": 1}
    raw = lambda body: hashlib.sha256(json.dumps(body, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    legacy_batch = raw([raw(child)])  # composed hash written before tenant binding
    (tmp_path / "journal").write_text(json.dumps({"kind": "plan:x", "event": "started", "plan": legacy_batch}) + "\n")
    batch = journal.plan_hash([journal.plan_hash(child)])
    assert [e["plan"] for e in journal.read()] == [batch]
    journal.record_plan("done", batch, action="x")
    assert [e["plan"] for e in journal.read()] == [batch, batch]
