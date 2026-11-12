from __future__ import annotations

import importlib
import json
import sys

from typer.testing import CliRunner


def fresh_config(monkeypatch, tmp_path):
    monkeypatch.setenv("FSV_CONFIG_DIR", str(tmp_path))
    monkeypatch.setenv("FSV_CACHE_DIR", str(tmp_path / "cache"))
    for name in list(sys.modules):
        if name == "fsv.config" or name.startswith("fsv."):
            sys.modules.pop(name, None)
    import fsv.config as config

    return importlib.reload(config)


def test_no_tenant_domain_default(monkeypatch, tmp_path):
    config = fresh_config(monkeypatch, tmp_path)

    assert config.DOMAIN == ""
    try:
        config.require_domain()
    except RuntimeError as e:
        assert "fsv auth login" in str(e)
    else:
        raise AssertionError("missing domain should fail")


def test_bad_saved_domain_does_not_break_help(monkeypatch, tmp_path):
    tmp_path.mkdir(exist_ok=True)
    (tmp_path / "config.json").write_text('{"domain":"bad"}')
    config = fresh_config(monkeypatch, tmp_path)

    assert config.DOMAIN == ""


def test_set_domain_normalizes_and_persists(monkeypatch, tmp_path):
    config = fresh_config(monkeypatch, tmp_path)

    assert config.set_domain("https://Acme.freshservice.com/a/tickets/") == "acme.freshservice.com"
    assert config.DOMAIN == "acme.freshservice.com"
    assert config.API_V2 == "https://acme.freshservice.com/api/v2"
    assert json.loads((tmp_path / "config.json").read_text())["domain"] == "acme.freshservice.com"



def test_cache_paths_are_tenant_scoped_with_legacy_fallback(monkeypatch, tmp_path):
    config = fresh_config(monkeypatch, tmp_path)
    config.set_domain("demo.freshservice.com")

    cache = tmp_path / "cache"
    assert config.schema_cache_path("changes") == cache / "schema" / "demo.freshservice.com--wsauto--changes.json"
    assert config.filters_cache_path("changes") == cache / "filters" / "demo.freshservice.com--changes.json"
    assert config.groups_cache_path() == cache / "groups--demo.freshservice.com.json"
    assert config.schema_cache_candidates("changes") == [
        cache / "schema" / "demo.freshservice.com--wsauto--changes.json",
    ]


def test_schema_and_form_caches_are_workspace_scoped_and_ignore_old_keys(monkeypatch, tmp_path):
    from fsv import cache as fcache, config, formmeta
    cache = tmp_path / "cache"
    monkeypatch.setattr(config, "CONFIG_FILE", tmp_path / "config.json")
    monkeypatch.setattr(config, "CACHE_DIR", cache)
    monkeypatch.setattr(config, "SCHEMA_DIR", cache / "schema")
    monkeypatch.setattr(config, "DOMAIN", "demo.freshservice.com")
    # a pre-migration file (no workspace in the key) is never read
    old = cache / "forms" / "demo.freshservice.com--change-fields.json"
    old.parent.mkdir(parents=True, exist_ok=True)
    fcache.save(old, "schema", {"change_fields": [{"name": "stale"}]})
    assert formmeta._cached("change-fields") is None
    formmeta._save("change-fields", "schema", {"change_fields": [{"name": "ws-auto"}]})
    assert formmeta._cached("change-fields") == {"change_fields": [{"name": "ws-auto"}]}
    config.save_config({"domain": "demo.freshservice.com", "forms": {"workspace_id": 5}})
    assert config.schema_cache_path("changes") == cache / "schema" / "demo.freshservice.com--ws5--changes.json"
    assert formmeta._cached("change-fields") is None  # another workspace never reuses ws-auto fields
    formmeta._save("change-fields", "schema", {"change_fields": [{"name": "ws5"}]})
    config.save_config({"domain": "demo.freshservice.com", "forms": {"workspace_id": 9}})
    assert formmeta._cached("change-fields") is None


def test_login_accepts_domain_without_prior_setup(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli
    import fsv.client as client

    monkeypatch.setattr(client.Client, "check_session", lambda self: {"first_name": "Ada"})
    monkeypatch.setattr(client, "reset_client", lambda: None)
    monkeypatch.setattr(client, "get_client", lambda: client.Client.__new__(client.Client))
    monkeypatch.setattr(cli, "_refresh_cache", lambda **_: None)

    result = CliRunner().invoke(cli.app, [
        "auth",
        "login",
        "--domain", "demo.freshservice.com",
        "--store", "file",
        "--header", "_x_m=a; _x_d=b; _x_w=c; fw-session-id=tok",
    ])

    assert result.exit_code == 0, result.output
    assert "saved 4 cookies" in result.output
    assert json.loads((tmp_path / "config.json").read_text())["domain"] == "demo.freshservice.com"
    session = json.loads((tmp_path / "session.json").read_text())
    assert session["domain"] == "demo.freshservice.com"


def test_no_input_login_requires_domain(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, ["--no-input", "auth", "login", "--header", "x=y"])

    assert result.exit_code == 2
    assert "--domain" in result.output


def test_local_no_input_login_requires_domain(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, ["auth", "login", "--no-input", "--header", "x=y"])

    assert result.exit_code == 2
    assert "--domain" in result.output


def test_no_input_login_requires_header(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, [
        "--no-input",
        "auth",
        "login",
        "--domain", "demo.freshservice.com",
        "--store", "file",
    ])

    assert result.exit_code == 2
    assert "pass --header" in result.output


def test_local_no_input_login_requires_header(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, [
        "auth",
        "login",
        "--no-input",
        "--domain", "demo.freshservice.com",
        "--store", "file",
    ])

    assert result.exit_code == 2
    assert "pass --header" in result.output


def test_browser_login_saves_cookies(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli
    import fsv.client as client
    import fsv.session as session

    monkeypatch.setattr(session, "login_browser", lambda: {"_x_m": "a", "_x_d": "b", "_x_w": "c", "fw-session-id": "tok"})
    monkeypatch.setattr(client, "reset_client", lambda: None)
    monkeypatch.setattr(client, "get_client", lambda: type("C", (), {"check_session": lambda self: {"first_name": "Ada", "last_name": "Lovelace", "email": "ada@example.com"}})())
    monkeypatch.setattr(cli, "_refresh_cache", lambda **_: None)
    monkeypatch.setattr(client.Client, "check_session", lambda self: {"first_name": "Ada"})

    result = CliRunner().invoke(cli.app, [
        "auth",
        "login",
        "--browser",
        "--domain", "demo.freshservice.com",
        "--store", "file",
    ])

    assert result.exit_code == 0, result.output
    assert "saved 4 cookies" in result.output
    assert "logged in as" in result.output
    session_data = json.loads((tmp_path / "session.json").read_text())
    assert session_data["cookies"]["fw-session-id"] == "tok"


def test_browser_login_rejects_no_input(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, [
        "auth",
        "login",
        "--browser",
        "--no-input",
        "--domain", "demo.freshservice.com",
    ])

    assert result.exit_code == 2
    assert "--browser is interactive" in result.output


def test_browser_login_polls_until_cookies_ready(monkeypatch, tmp_path):
    config = fresh_config(monkeypatch, tmp_path)
    config.set_domain("demo.freshservice.com")
    import fsv.session as session

    calls = []
    sleeps = []
    cookie_reads = iter([
        {"_x_m": "a"},
        {"_x_m": "a", "_x_d": "b", "_x_w": "c", "fw-session-id": "tok"},
    ])

    class FakeProc:
        def poll(self):
            return None
        def terminate(self):
            calls.append(["terminate"])
        def wait(self, timeout=None):
            calls.append(["wait", timeout])

    class FakeTempDir:
        def __init__(self, prefix):
            self.prefix = prefix
        def __enter__(self):
            return str(tmp_path / "profile")
        def __exit__(self, exc_type, exc, tb):
            return False

    monkeypatch.setattr(session, "_browser_executable", lambda: "/bin/chrome")
    monkeypatch.setattr(session.tempfile, "TemporaryDirectory", FakeTempDir)
    monkeypatch.setattr(session.subprocess, "Popen", lambda args: calls.append(args) or FakeProc())
    monkeypatch.setattr(session, "_devtools_port", lambda profile_dir, proc: 9222)
    monkeypatch.setattr(session, "_direct_browser_cookies", lambda port, domain: next(cookie_reads))
    monkeypatch.setattr(session.time, "sleep", lambda seconds: sleeps.append(seconds))

    cookies = session.login_browser()

    assert cookies["fw-session-id"] == "tok"
    assert sleeps == [2]
    assert calls[0][0] == "/bin/chrome"
    assert "--remote-debugging-port=0" in calls[0]
    assert "--new-window" in calls[0]
    assert calls[-2:] == [["terminate"], ["wait", 5]]
