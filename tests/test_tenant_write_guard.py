"""Writes need the target tenant confirmed explicitly (--tenant, FSV_ALLOW_TENANT
or tenant.confirmed), through one shared hook that no command can bypass."""
import json

import httpx
import pytest
import typer
import typer.main
from typer.testing import CliRunner

from fsv import cli, config, journal, tenant
from test_ticket_notes import mock_client

ACME, GLOBEX = "acme.example.com", "globex.example.com"
# Flag names that would mean "really write" if a command ever used them
# instead of --yes (and so escaped the guard keyed on --yes).
_WRITE_FLAG_ALIASES = {"--send", "--confirm", "--apply", "--execute", "--commit", "--go"}


@pytest.fixture
def fake(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", ACME)
    monkeypatch.setattr(config, "API_INT", f"https://{ACME}/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"spam": {32: False}, "writes": []}

    def handler(req):
        tid = int(req.url.path.removeprefix("/api/_/").split("/")[1])
        if req.method == "GET":
            return httpx.Response(200, json={"ticket": {"id": tid, "spam": s["spam"][tid], "deleted": False}})
        s["writes"].append(req.method)
        s["spam"][tid] = True
        return httpx.Response(204)

    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: c)
    return s


def run(*args, env=None):
    return CliRunner().invoke(cli.app, list(args), env=env)


def send(*extra, env=None):
    digest = json.loads(run("tickets", "spam", "32", "--dry-run").stdout)["plan_hash"]
    return run("tickets", "spam", "32", "--yes", "--expect-plan", digest, "--confirm-destructive", *extra, env=env)


def test_conftest_confirms_the_fake_tenant(fake):
    assert tenant.confirmation() == "FSV_ALLOW_TENANT"


def test_write_refused_without_confirmation_and_dry_run_never_needs_it(fake, monkeypatch):
    monkeypatch.delenv("FSV_ALLOW_TENANT")
    assert run("tickets", "spam", "32", "--dry-run").exit_code == 0
    assert run("tickets", "spam", "32", "--dry-run", "--yes").exit_code == 0  # --dry-run wins
    r = send()
    assert r.exit_code == 2 and fake["writes"] == []
    assert f"refusing to write: tenant {ACME}" in r.stderr
    assert f"--tenant {ACME}" in r.stderr and f"FSV_ALLOW_TENANT={ACME}" in r.stderr
    assert f"fsv config set tenant.confirmed {ACME}" in r.stderr


def test_success_with_tenant_option(fake, monkeypatch):
    monkeypatch.delenv("FSV_ALLOW_TENANT")
    r = send("--tenant", ACME)
    assert r.exit_code == 0, r.output
    assert json.loads(r.stdout)["spam"] is True and fake["writes"] == ["PUT"]


def test_env_confirmation_and_mismatch(fake, monkeypatch):
    monkeypatch.delenv("FSV_ALLOW_TENANT")
    r = send(env={"FSV_ALLOW_TENANT": GLOBEX})
    assert r.exit_code == 2 and f"FSV_ALLOW_TENANT={GLOBEX} does not match" in r.stderr
    assert fake["writes"] == []
    assert send(env={"FSV_ALLOW_TENANT": ACME.upper()}).exit_code == 0
    assert fake["writes"] == ["PUT"]


def test_config_confirmation_set_once(fake, monkeypatch):
    monkeypatch.delenv("FSV_ALLOW_TENANT")
    bad = run("config", "set", "tenant.confirmed", GLOBEX)
    assert bad.exit_code == 2 and f"configured tenant is {ACME}" in bad.output
    ok = run("config", "set", "tenant.confirmed", ACME)
    assert ok.exit_code == 0, ok.output
    assert tenant.confirmation() == "tenant.confirmed"
    assert send().exit_code == 0 and fake["writes"] == ["PUT"]


def test_confirmation_goes_stale_when_the_configured_tenant_changes(fake, monkeypatch):
    monkeypatch.delenv("FSV_ALLOW_TENANT")
    assert run("config", "set", "tenant.confirmed", ACME).exit_code == 0
    monkeypatch.setattr(config, "DOMAIN", GLOBEX)
    assert tenant.confirmation() is None
    assert f"tenant.confirmed={ACME} does not match" in tenant.refusal()


def _leaves():
    root = typer.main.get_command(cli.app)
    stack = [((), root)]
    while stack:
        path, cmd = stack.pop()
        subs = getattr(cmd, "commands", None)
        if subs:
            stack.extend(((*path, n), c) for n, c in subs.items())
        else:
            yield root, path, cmd


def _ctx_chain(root, path, leaf):
    ctx = typer.Context(root, info_name="fsv")
    ctx.params = {"tenant": None, "quiet": True}
    cmd = root
    for name in path:
        cmd = cmd.commands[name] if hasattr(cmd, "commands") else leaf
        ctx = typer.Context(cmd, parent=ctx, info_name=name)
    return ctx


def test_every_yes_command_routes_through_the_tenant_guard(monkeypatch):
    """Walk all registered commands: each with --yes must refuse an
    unconfirmed tenant before its body runs. A new write command that does
    not route through FSVCommand.invoke (or skips the guard) fails here."""
    monkeypatch.delenv("FSV_ALLOW_TENANT")
    guarded = []
    for root, path, cmd in _leaves():
        opts = {o: p for p in cmd.params for o in getattr(p, "opts", [])}
        name = " ".join(path)
        assert not (_WRITE_FLAG_ALIASES & set(opts)) or "--yes" in opts, f"{name}: write flag without --yes"
        if "--dry-run" in opts or "--expect-plan" in opts:
            assert "--yes" in opts, f"{name}: reviewed command without --yes"
        if "--yes" not in opts:
            continue
        assert isinstance(cmd, cli.FSVCommand) and type(cmd).invoke is cli.FSVCommand.invoke, name
        if name in tenant.LOCAL_YES_COMMANDS:
            continue
        ran = []
        monkeypatch.setattr(cmd, "callback", lambda *a, **k: ran.append(name))
        ctx = _ctx_chain(root, path, cmd)
        ctx.params = {p.name: None for p in cmd.params}
        ctx.params[opts["--yes"].name] = True
        with pytest.raises(typer.Exit) as exc:
            cmd.invoke(ctx)
        assert exc.value.exit_code == 2 and not ran, name
        guarded.append(name)
    assert len(guarded) > 90 and "tickets bulk" in guarded and "changes delete" in guarded
    assert all(p in {" ".join(x[1]) for x in _leaves()} for p in tenant.LOCAL_YES_COMMANDS)


def test_a_new_write_command_cannot_bypass_the_guard(monkeypatch):
    monkeypatch.delenv("FSV_ALLOW_TENANT")
    sent = []
    app = typer.Typer()

    @app.command()
    def push(yes: bool = typer.Option(False, "--yes")) -> None:
        sent.append(yes)

    @app.command()
    def other() -> None:
        pass

    assert CliRunner().invoke(app, ["push", "--yes"]).exit_code == 2 and sent == []
    assert CliRunner().invoke(app, ["push", "--yes"], env={"FSV_ALLOW_TENANT": config.DOMAIN}).exit_code == 0
    assert sent == [True]
