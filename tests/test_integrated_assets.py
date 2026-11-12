"""Synthetic integrated ITAM envelopes; write echoes never prove persistence."""
import json
from types import SimpleNamespace
import pytest
from typer.testing import CliRunner
from fsv import config, integrated_assets as a, journal
from fsv.cli import app
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture
def client(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.freshservice.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.freshservice.com/api/_")
    monkeypatch.setenv("FSV_CACHE_DIR", str(tmp_path))
    rows = []
    calls = []
    def get(path, params=None):
        calls.append(("GET", path, params))
        return {"data": list(rows), "meta": {"total_count": len(rows), "total_pages": 1 if rows else 0}}
    def request(method, url, **kw):
        calls.append((method, url, kw))
        if method == "GET":
            return {"page": 1, "error": None, "total_rows": 1, "data": [{"device_pk": 81, "name": {"id": 81, "label": "Acme"}}]}
        payload = json.loads(kw["content"])
        if "itam_cmdb_request_id" in payload:
            rows.clear()
        else:
            kind = "businessapp_ids" if "businessapp_ids" in payload else "device_ids"
            rows.extend({"item_id": i, "itam_cmdb_request_id": int(i) + 900, "category": "Business service" if kind == "businessapp_ids" else "Device"} for i in payload[kind])
        return {}
    return SimpleNamespace(int_get=get, _request=request, _get_csrf=lambda: "synthetic", rows=rows, calls=calls)


@pytest.mark.parametrize("module", ["tickets", "sr", "problems", "changes"])
@pytest.mark.parametrize("kind", ["devices", "services"])
def test_bulk_and_exact_link_removal(client, module, kind):
    plan = a.build_plan(client, module, 23, [81, 82], kind=kind)
    result = a.send(plan, client, module, [81, 82], kind=kind, reviewed_hash=plan.hash)
    assert result["verified"]
    remove = a.build_plan(client, module, 23, [81], kind=kind, remove=True)
    assert remove.payload == {"itam_cmdb_request_id": 981}
    # Model independent persistence for one-row unlink.
    original = client._request
    def unlink(*args, **kwargs):
        client.rows[:] = [r for r in client.rows if int(r["item_id"]) != 81]
        client.calls.append(("PUT", args[1], kwargs))
        return None
    client._request = unlink
    assert a.send(remove, client, module, [81], kind=kind, remove=True, reviewed_hash=remove.hash)["verified"]
    assert len(client.rows) == 1
    assert len([c for c in client.calls if c[0] == "PUT"]) == 2


def test_search_integrated(client):
    assert a.search(client, "Acme")["data"][0]["name"]["id"] == 81
    assert "/itam/core/device/api/" in client.calls[-1][1]


def test_duplicate_incomplete_and_wrong_hash(client):
    client.int_get = lambda *a, **k: {"data": [], "meta": {"total_count": 1, "total_pages": 1}}
    with pytest.raises(ReviewedWriteError):
        a.build_plan(client, "tickets", 23, [81])


def test_cli_dry_run_and_replay_guard(client, monkeypatch):
    import fsv.cli as cli
    monkeypatch.setattr(cli, "_client", lambda: client)
    runner = CliRunner()
    r = runner.invoke(app, ["sr", "assets", "add", "SR-23", "81", "82", "--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    plan = json.loads(r.stdout)
    assert plan["payload"] == {"device_ids": ["81", "82"]}
    assert not any(c[0] == "PUT" for c in client.calls)
    r = runner.invoke(app, ["sr", "assets", "add", "SR-23", "81", "--yes", "--expect-plan", "wrong"])
    assert r.exit_code == 2
    assert not any(c[0] == "PUT" for c in client.calls)
