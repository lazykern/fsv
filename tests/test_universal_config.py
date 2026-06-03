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
    assert config.schema_cache_path("changes") == cache / "schema" / "demo.freshservice.com--changes.json"
    assert config.filters_cache_path("changes") == cache / "filters" / "demo.freshservice.com--changes.json"
    assert config.groups_cache_path() == cache / "groups--demo.freshservice.com.json"
    assert config.schema_cache_candidates("changes") == [
        cache / "schema" / "demo.freshservice.com--changes.json",
        cache / "schema" / "changes.json",
    ]


def test_login_accepts_domain_without_prior_setup(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli

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

    assert result.exit_code == 1
    assert "--domain" in result.output


def test_local_no_input_login_requires_domain(monkeypatch, tmp_path):
    fresh_config(monkeypatch, tmp_path)
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, ["auth", "login", "--no-input", "--header", "x=y"])

    assert result.exit_code == 1
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

    assert result.exit_code == 1
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

    assert result.exit_code == 1
    assert "pass --header" in result.output
