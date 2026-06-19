from __future__ import annotations

from types import SimpleNamespace

from typer.testing import CliRunner


runner = CliRunner()


def _stub_client(http_client):
    import fsv.client as client

    c = client.Client.__new__(client.Client)
    c._client = http_client
    c._csrf = None
    c._fw_domain = None
    c._rl_rem = None
    c._rl_tot = None
    c._fw_session_id = None
    return c


class _FakeHTTPClient:
    def __init__(self, response):
        self.responses = list(response) if isinstance(response, list) else [response]
        self.cookies = {}
        self.calls = []

    def request(self, method, url, **kw):
        self.calls.append((method, url, kw))
        item = self.responses.pop(0)
        if isinstance(item, BaseException):
            raise item
        return item


class _FakeResponse:
    def __init__(self, status_code=200, headers=None, data=None, text=""):
        self.status_code = status_code
        self.headers = headers or {"content-type": "application/json"}
        self._data = {"ok": True} if data is None else data
        self.text = text or "{\"ok\":true}"

    def json(self):
        return self._data


class _FakeErr:
    def __init__(self):
        self.lines = []

    def print(self, *args, **kwargs):
        self.lines.append(" ".join(str(arg) for arg in args))


def test_main_sets_client_verbose():
    import fsv.cli as cli
    import fsv.client as client

    client.set_verbose(False)

    cli.main(
        ctx=SimpleNamespace(invoked_subcommand="tickets"),
        no_input=False,
        verbose=True,
        version=False,
    )

    assert client.is_verbose() is True
    client.set_verbose(False)


def test_trailing_short_verbose_flag_sets_global_state():
    import fsv.cli as cli
    import fsv.client as client

    client.set_verbose(False)
    result = runner.invoke(cli.app, ["help", "-v"])

    assert result.exit_code == 0, result.output
    assert client.is_verbose() is True
    client.set_verbose(False)


def test_trailing_long_verbose_flag_sets_global_state():
    import fsv.cli as cli
    import fsv.client as client

    client.set_verbose(False)
    result = runner.invoke(cli.app, ["help", "--verbose"])

    assert result.exit_code == 0, result.output
    assert client.is_verbose() is True
    client.set_verbose(False)


def test_trailing_verbose_flag_parses_on_leaf_command(monkeypatch):
    import fsv.cli as cli
    import fsv.client as client

    client.set_verbose(False)
    called = {"ok": False}

    def fake_list_resource(*args, **kwargs):
        called["ok"] = True

    monkeypatch.setattr(cli, "list_resource", fake_list_resource)
    result = runner.invoke(cli.app, ["changes", "ls", "-v"])

    assert result.exit_code == 0, result.output
    assert called["ok"] is True
    assert client.is_verbose() is True
    client.set_verbose(False)


def test_subcommand_help_shows_global_verbose_panel():
    import fsv.cli as cli

    result = runner.invoke(cli.app, ["changes", "ls", "--help"])

    assert result.exit_code == 0, result.output
    assert "Global options" in result.output
    assert "--verbose" in result.output
    assert "-v" in result.output


def test_group_help_shows_global_verbose_panel():
    import fsv.cli as cli

    result = runner.invoke(cli.app, ["changes", "--help"])

    assert result.exit_code == 0, result.output
    assert "Global options" in result.output
    assert "--verbose" in result.output


def test_root_help_does_not_duplicate_global_verbose_panel():
    import fsv.cli as cli

    result = runner.invoke(cli.app, ["--help"])

    assert result.exit_code == 0, result.output
    assert "Global options" not in result.output


def test_request_logs_verbose_get(monkeypatch):
    import fsv.client as client

    fake_err = _FakeErr()
    http = _FakeHTTPClient(
        _FakeResponse(
            headers={
                "content-type": "application/json",
                "x-ratelimit-remaining": "99",
                "x-ratelimit-total": "100",
            }
        )
    )
    stub = _stub_client(http)

    monkeypatch.setattr(client, "err", fake_err)
    client.set_verbose(True)

    result = stub._request(
        "GET",
        "https://fresh.example/api/_/tickets/1",
        params={"include": "requester,stats", "page": 2},
    )

    assert result == {"ok": True}
    assert len(fake_err.lines) == 1
    assert "[api] GET /api/_/tickets/1?include=requester,stats&page=2 -> 200 " in fake_err.lines[0]
    assert " rl=99/100" in fake_err.lines[0]

    client.set_verbose(False)


def test_request_logs_redacted_json_body(monkeypatch):
    import fsv.client as client

    fake_err = _FakeErr()
    http = _FakeHTTPClient(_FakeResponse())
    stub = _stub_client(http)

    monkeypatch.setattr(client, "err", fake_err)
    client.set_verbose(True)

    stub._request(
        "PATCH",
        "https://fresh.example/api/_/changes/1",
        json={
            "status": 3,
            "session_token": "secret-token",
            "nested": {"password": "secret-pass"},
        },
    )

    assert len(fake_err.lines) == 1
    assert "[api] PATCH /api/_/changes/1 body={" in fake_err.lines[0]
    assert '"status":3' in fake_err.lines[0]
    assert '"session_token":"[redacted]"' in fake_err.lines[0]
    assert '"password":"[redacted]"' in fake_err.lines[0]
    assert "secret-token" not in fake_err.lines[0]
    assert "secret-pass" not in fake_err.lines[0]

    client.set_verbose(False)


def test_get_retries_retryable_response(monkeypatch):
    import fsv.client as client

    http = _FakeHTTPClient([
        _FakeResponse(status_code=503, data={"error": "busy"}),
        _FakeResponse(data={"ok": True}),
    ])
    stub = _stub_client(http)
    monkeypatch.setattr(client.time, "sleep", lambda _delay: None)

    assert stub._request("GET", "https://fresh.example/api/_/changes/1") == {"ok": True}
    assert len(http.calls) == 2


def test_post_does_not_retry_retryable_response(monkeypatch):
    import pytest
    import fsv.client as client

    http = _FakeHTTPClient(_FakeResponse(status_code=503, data={"error": "busy"}))
    stub = _stub_client(http)
    monkeypatch.setattr(client.time, "sleep", lambda _delay: None)

    with pytest.raises(client.APIError) as exc:
        stub._request("POST", "https://fresh.example/api/_/changes", json={"change": {}})

    assert exc.value.method == "POST"
    assert exc.value.target == "/api/_/changes"
    assert len(http.calls) == 1
