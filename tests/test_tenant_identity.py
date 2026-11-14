"""`fsv whoami`, `fsv config show` and `fsv auth status` name the tenant and
config dir; command JSON goes through the one tenant-aware serializer."""
import json
import re
from pathlib import Path

from typer.testing import CliRunner

from fsv import cli, config

AGENT = {"first_name": "Ada", "last_name": "Acme", "email": "ada@example.com"}


class FakeClient:
    def __init__(self, agent=AGENT):
        self.agent, self.calls = agent, []

    def check_session(self):
        self.calls.append("agents/me")
        return self.agent

    def rate_limit_remaining(self):
        return 99, 100


def run(*args, **kw):
    return CliRunner().invoke(cli.app, list(args), **kw)


def test_whoami_text_and_json(monkeypatch):
    fake = FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: fake)
    r = run("whoami")
    assert r.exit_code == 0, r.output
    assert "Ada Acme" in r.stdout and "ada@example.com" in r.stdout
    assert f"tenant:      {config.DOMAIN}" in r.stdout
    assert f"config dir:  {config.display_path(config.CONFIG_DIR)}" in r.stdout
    data = json.loads(run("whoami", "--json").stdout)
    assert data == {"name": "Ada Acme", "email": "ada@example.com", "tenant": config.DOMAIN,
                    "config_dir": str(config.CONFIG_DIR)}
    assert fake.calls == ["agents/me", "agents/me"]  # read-only profile GETs


def test_whoami_prints_the_banner_and_hidden_identity(monkeypatch):
    monkeypatch.delenv("FSV_QUIET")
    monkeypatch.setattr(cli, "_client", lambda: FakeClient({}))
    r = run("whoami")
    assert r.exit_code == 0 and r.stderr.startswith(f"fsv: tenant {config.DOMAIN} (config ")
    assert "identity unavailable" in r.stdout


def test_config_show_names_tenant_dir_and_confirmation(monkeypatch):
    monkeypatch.delenv("FSV_ALLOW_TENANT")
    r = run("config", "show")
    assert r.exit_code == 0, r.output
    lines = r.stdout.splitlines()
    assert lines[0] == f"tenant:      {config.DOMAIN}"
    assert lines[1] == f"config dir:  {config.display_path(config.CONFIG_DIR)}"
    assert lines[2].startswith("writes:      not confirmed")
    assert run("config", "set", "tenant.confirmed", config.DOMAIN.upper()).exit_code == 0
    data = json.loads(run("config", "show", "--json").stdout)
    assert (data["tenant"], data["config_dir"]) == (config.DOMAIN, str(config.CONFIG_DIR))
    assert data["writes_confirmed_by"] == "tenant.confirmed"
    assert data["values"]["tenant.confirmed"] == config.DOMAIN
    assert run("config", "set", "tenant.confirmed", "not a domain").exit_code == 2


def test_auth_status_shows_tenant_and_config_dir(monkeypatch):
    import fsv.session as session
    monkeypatch.setattr(session, "session_age_hours", lambda: 1.0)
    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    r = run("auth", "status")
    assert r.exit_code == 0, r.output
    assert f"tenant:      {config.DOMAIN}" in r.stdout
    assert f"config dir:  {config.display_path(config.CONFIG_DIR)}" in r.stdout
    monkeypatch.setattr(session, "session_age_hours", lambda: None)
    missing = run("auth", "status")
    assert missing.exit_code == 2 and f"no session for tenant {config.DOMAIN}" in missing.output


def test_command_json_goes_through_the_tenant_serializer():
    """Write results/reviews get tenant/config_dir from fsv.tenant.tenant_json;
    printing json.dumps directly would silently drop them."""
    src = Path(cli.__file__).parent
    direct = re.compile(r"\b(typer\.echo|print)\(\s*json\.dumps\(")
    offenders = [str(p.relative_to(src)) for p in src.rglob("*.py")
                 if "tui" not in p.parts and direct.search(p.read_text(encoding="utf-8"))]
    assert offenders == []
