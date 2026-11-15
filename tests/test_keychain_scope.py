"""Keychain items are scoped to the config dir: a logout (or a file-backend
save) in a scratch/trial FSV_CONFIG_DIR never touches another dir's session.
Offline: a fake Keychain backend replaces the macOS `security` tool."""
from __future__ import annotations

import hashlib
import os

import pytest
from typer.testing import CliRunner

import fsv.config as config
import fsv.session as session

COOKIES = {"_x_m": "a", "_x_d": "b", "_x_w": "c", "fw-session-id": "tok"}


class FakeKeychain:
    def __init__(self):
        self.items: dict[str, str] = {}

    def write(self, account, encoded):
        self.items[account] = encoded
        return True

    def read(self, account):
        return self.items.get(account)

    def delete(self, account):
        self.items.pop(account, None)


@pytest.fixture
def keychain(monkeypatch, tmp_path):
    fake = FakeKeychain()
    monkeypatch.setattr(session, "_KEYCHAIN", fake)
    monkeypatch.setattr(session, "_keychain_available", lambda: True)
    monkeypatch.setattr(session.Path, "home", classmethod(lambda cls: tmp_path / "home"))
    return fake


def use_dir(monkeypatch, path):
    path.mkdir(parents=True, exist_ok=True)
    (path / "config.json").write_text('{"domain": "acme.freshservice.com"}')
    monkeypatch.setenv("FSV_CONFIG_DIR", str(path))
    config.reload()
    monkeypatch.setattr(session, "_SESSION_CACHE", None)


def test_account_is_scoped_to_the_resolved_config_dir(keychain, monkeypatch, tmp_path):
    use_dir(monkeypatch, tmp_path / "a")
    digest = hashlib.sha256(os.fsencode((tmp_path / "a").resolve())).hexdigest()[:12]
    assert session.keychain_account() == f"fsv-session-{digest}"
    first = session.keychain_account()
    use_dir(monkeypatch, tmp_path / "b")
    assert session.keychain_account() != first


def test_logout_of_one_config_dir_leaves_the_other(keychain, monkeypatch, tmp_path):
    use_dir(monkeypatch, tmp_path / "a")
    session.save_cookies(COOKIES, "keychain")
    account_a = session.keychain_account()
    use_dir(monkeypatch, tmp_path / "b")
    session.save_cookies({**COOKIES, "fw-session-id": "other"}, "keychain")
    assert len(keychain.items) == 2
    session.logout()
    assert list(keychain.items) == [account_a]
    with pytest.raises(session.SessionError):
        session.load_cookies()
    use_dir(monkeypatch, tmp_path / "a")
    assert session.load_cookies()["fw-session-id"] == "tok"


def test_cli_logout_and_status_use_the_config_dirs_own_item(keychain, monkeypatch, tmp_path):
    from fsv import cli
    use_dir(monkeypatch, tmp_path / "real")
    session.save_cookies(COOKIES, "keychain")
    use_dir(monkeypatch, tmp_path / "scratch")
    session.save_cookies(COOKIES, "keychain")
    result = CliRunner().invoke(cli.app, ["auth", "logout", "--yes"])
    assert result.exit_code == 0, result.output
    status = CliRunner().invoke(cli.app, ["auth", "status"])
    assert status.exit_code != 0 and "no session" in status.output
    use_dir(monkeypatch, tmp_path / "real")
    assert session.session_age_hours() is not None and len(keychain.items) == 1


def test_file_save_in_a_scratch_dir_keeps_other_keychain_items(keychain, monkeypatch, tmp_path):
    use_dir(monkeypatch, tmp_path / "real")
    session.save_cookies(COOKIES, "keychain")
    use_dir(monkeypatch, tmp_path / "scratch")
    session.save_cookies(COOKIES, "file")
    assert len(keychain.items) == 1


def test_legacy_unscoped_item_only_for_the_default_config_dir(keychain, monkeypatch, tmp_path):
    import base64
    import json
    legacy = json.dumps({"saved_at": 1, "domain": "acme.freshservice.com", "cookies": COOKIES})
    keychain.items[session.KEYCHAIN_ACCOUNT] = base64.b64encode(legacy.encode()).decode()
    # a scratch dir neither reads nor deletes the legacy item
    use_dir(monkeypatch, tmp_path / "scratch")
    (config.CONFIG_DIR / "store").write_text("keychain")
    with pytest.raises(session.SessionError):
        session.load_cookies()
    session.logout()
    assert session.KEYCHAIN_ACCOUNT in keychain.items
    # the default dir (~/.config/fsv) still reads it as a fallback ...
    use_dir(monkeypatch, tmp_path / "home" / ".config" / "fsv")
    (config.CONFIG_DIR / "store").write_text("keychain")
    assert session.load_cookies() == COOKIES
    # ... prefers its scoped item once written, and removes both on logout
    session.save_cookies({**COOKIES, "fw-session-id": "new"}, "keychain")
    assert session.load_cookies()["fw-session-id"] == "new"
    session.logout()
    assert keychain.items == {}



class FakeAgentClient:
    """Offline stand-in for the tenant client used by login and status."""

    def check_session(self):
        return {"first_name": "Acme", "last_name": "Agent", "email": "agent@example.com"}

    def rate_limit_remaining(self):
        return 100, 100


@pytest.fixture
def offline_cli(monkeypatch):
    from fsv import cli, client
    fake = FakeAgentClient()
    monkeypatch.setattr(client, "get_client", lambda *a, **k: fake)
    monkeypatch.setattr(client, "reset_client", lambda *a, **k: None)
    monkeypatch.setattr(cli, "get_client", lambda *a, **k: fake, raising=False)
    monkeypatch.setattr(cli, "reset_client", lambda *a, **k: None, raising=False)
    monkeypatch.setattr(cli, "_client", lambda *a, **k: fake)
    monkeypatch.setattr(cli, "_refresh_cache", lambda *a, **k: None)
    return cli


def _legacy_item(keychain):
    import base64
    import json
    legacy = json.dumps({"saved_at": 1, "domain": "acme.freshservice.com", "cookies": COOKIES})
    keychain.items[session.KEYCHAIN_ACCOUNT] = base64.b64encode(legacy.encode()).decode()
    return keychain.items[session.KEYCHAIN_ACCOUNT]


def _login(cli, store="keychain", cookie="tok"):
    header = "; ".join(f"{k}={v}" for k, v in {**COOKIES, "fw-session-id": cookie}.items())
    return CliRunner().invoke(cli.app, ["auth", "login", "--header", "-", "--store", store, "--no-input", "--force"],
                              input=header)


def test_login_status_logout_for_two_config_dirs(keychain, offline_cli, monkeypatch, tmp_path):
    """login --store keychain -> status -> logout in two scratch dirs: each dir
    gets its own item, and logging out of one leaves the other and the
    legacy unscoped item untouched."""
    cli = offline_cli
    legacy = _legacy_item(keychain)
    accounts = {}
    for name in ("one", "two"):
        use_dir(monkeypatch, tmp_path / name)
        r = _login(cli, cookie=name)
        assert r.exit_code == 0, r.output
        accounts[name] = session.keychain_account()
        status = CliRunner().invoke(cli.app, ["auth", "status"])
        assert status.exit_code == 0 and "session:     valid" in status.output, status.output
    assert accounts["one"] != accounts["two"]
    assert set(keychain.items) == {accounts["one"], accounts["two"], session.KEYCHAIN_ACCOUNT}
    # logout of "two" (the current dir)
    assert CliRunner().invoke(cli.app, ["auth", "logout", "--yes"]).exit_code == 0
    assert set(keychain.items) == {accounts["one"], session.KEYCHAIN_ACCOUNT}
    assert keychain.items[session.KEYCHAIN_ACCOUNT] == legacy
    status = CliRunner().invoke(cli.app, ["auth", "status"])
    assert status.exit_code != 0 and "no session" in status.output
    use_dir(monkeypatch, tmp_path / "one")
    assert CliRunner().invoke(cli.app, ["auth", "status"]).exit_code == 0
    assert session.load_cookies()["fw-session-id"] == "one"


def test_default_dir_without_env_falls_back_to_the_legacy_item(keychain, offline_cli, monkeypatch, tmp_path):
    """No FSV_CONFIG_DIR: the resolved default dir reads the legacy item,
    status works from it, and logout removes only the default dir's items."""
    cli = offline_cli
    _legacy_item(keychain)
    use_dir(monkeypatch, tmp_path / "scratch")
    scratch = session.keychain_account()
    session.save_cookies({**COOKIES, "fw-session-id": "scratch"}, "keychain")
    default = tmp_path / "home" / ".config" / "fsv"
    default.mkdir(parents=True, exist_ok=True)
    (default / "config.json").write_text('{"domain": "acme.freshservice.com"}')
    (default / "store").write_text("keychain")
    monkeypatch.delenv("FSV_CONFIG_DIR")
    config.reload()
    monkeypatch.setattr(session, "_SESSION_CACHE", None)
    assert config.CONFIG_DIR.resolve() == default.resolve()
    assert session.keychain_account() not in keychain.items  # no scoped item yet
    assert session.load_cookies() == COOKIES
    assert CliRunner().invoke(cli.app, ["auth", "status"]).exit_code == 0
    assert CliRunner().invoke(cli.app, ["auth", "logout", "--yes"]).exit_code == 0
    assert set(keychain.items) == {scratch}


def test_file_store_login_and_logout_never_touch_keychain_items(keychain, offline_cli, monkeypatch, tmp_path):
    """--store file in a scratch dir: the session is a file in that dir;
    status reads it, logout deletes it; other dirs' and the legacy Keychain
    items stay."""
    cli = offline_cli
    legacy = _legacy_item(keychain)
    use_dir(monkeypatch, tmp_path / "real")
    session.save_cookies(COOKIES, "keychain")
    before = dict(keychain.items)
    use_dir(monkeypatch, tmp_path / "scratch")
    r = _login(cli, store="file", cookie="file")
    assert r.exit_code == 0, r.output
    assert session.current_backend() == "file" and keychain.items == before
    assert CliRunner().invoke(cli.app, ["auth", "status"]).exit_code == 0
    assert session.load_cookies()["fw-session-id"] == "file"
    assert CliRunner().invoke(cli.app, ["auth", "logout", "--yes"]).exit_code == 0
    assert keychain.items == before and keychain.items[session.KEYCHAIN_ACCOUNT] == legacy
    with pytest.raises(session.SessionError):
        session.load_cookies()
