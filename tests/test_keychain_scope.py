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
