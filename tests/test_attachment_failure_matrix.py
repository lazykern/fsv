"""Synthetic byte-accurate upload, replacement and download error contracts."""
import base64
import json

import httpx
import pytest

from fsv import create
from fsv.client import Client
from fsv.errors import APIError, UsageError


@pytest.fixture
def wire(monkeypatch):
    state = {"writes": [], "downloads": [], "fail": None, "upload_result": None}

    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "changes/123":
                return httpx.Response(200, json={"change": {"attachments": [
                    {"id": 71, "name": "plan.txt", "attachment_url": "https://example.com/old"},
                    {"id": 72, "name": "keep.txt"}]}})
            assert str(req.url) == "https://example.com/old"
            state["downloads"].append(str(req.url))
            return httpx.Response(503 if state["fail"] == "download" else 200, content=b"Acme old bytes")
        body = json.loads(req.content)
        state["writes"].append((req.method, path, body))
        if state["fail"] == path:
            return httpx.Response(503, json={"description": "synthetic outage"})
        if path == "attachments":
            default = {"id": 90 + len(state["writes"]), "content_file_name": body["content_file_name"], "content_file_size": body["content_file_size"]}
            return httpx.Response(200, json=state["upload_result"] if state["upload_result"] is not None else default)
        assert path == "changes/123"
        return httpx.Response(200, json={"change": {"id": 123}})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic-token")
    monkeypatch.setattr(c, "_probe_session", lambda: True)
    monkeypatch.setattr(create, "get_client", lambda: c)
    state["client"] = c
    yield state
    c.close()


@pytest.mark.parametrize("mode,backup,ids,names", [
    ("skip", False, [71, 72], []),
    ("append", False, [71, 72, 91], ["plan.txt"]),
    ("replace", False, [72, 91], ["plan.txt"]),
    ("replace", True, [72, 91, 92], ["plan.backup.txt", "plan.txt"]),
])
def test_main_attachment_duplicate_policies_keep_exact_receipts(wire, tmp_path, mode, backup, ids, names):
    path = tmp_path / "plan.txt"
    path.write_bytes(b"Globex new bytes")
    result = create.attach_files_to_change(123, [str(path)], duplicate=mode, backup_replaced=backup, backup_name="plan.backup.txt")
    assert result["attachments"] == ids
    uploads = [b for _, p, b in wire["writes"] if p == "attachments"]
    assert [b["content_file_name"] for b in uploads] == names
    assert [base64.b64decode(b["content"].split(",", 1)[1]) for b in uploads] == ([b"Acme old bytes"] if backup else []) + ([b"Globex new bytes"] if names else [])
    puts = [b for _, p, b in wire["writes"] if p == "changes/123"]
    assert puts == ([{"attachments": ids}] if names else [])
    assert wire["downloads"] == (["https://example.com/old"] if backup else [])
    if mode == "skip":
        assert result["_fsv_noop"] is True and result["skipped"] == ["plan.txt"]


@pytest.mark.parametrize("failure,expected_uploads", [("download", 0), ("attachments", 1), ("changes/123", 2)])
def test_replacement_failure_never_replays_or_saves_after_failed_backup(wire, tmp_path, failure, expected_uploads):
    path = tmp_path / "plan.txt"
    path.write_bytes(b"Globex new bytes")
    wire["fail"] = failure
    with pytest.raises(APIError) as exc:
        create.attach_files_to_change(123, [str(path)], duplicate="replace", backup_replaced=True, backup_name="plan.backup.txt")
    assert exc.value.status == 503
    assert sum(p == "attachments" for _, p, _ in wire["writes"]) == expected_uploads
    assert sum(p == "changes/123" for _, p, _ in wire["writes"]) == (failure == "changes/123")
    # The old asset remains only a remote receipt when the final PUT fails;
    # these legacy helpers do not roll back uploads or prove readback.


@pytest.mark.parametrize("response", [{}, [], {"name": "plan.txt"}])
def test_upload_missing_id_fails_before_attach_save(wire, tmp_path, response):
    path = tmp_path / "plan.txt"
    path.write_bytes(b"new")
    wire["upload_result"] = response
    with pytest.raises(APIError, match="returned no id"):
        create.attach_files_to_change(123, [str(path)], duplicate="append")
    assert [(m, p) for m, p, _ in wire["writes"]] == [("POST", "attachments")]


@pytest.mark.parametrize("response,diagnostic", [
    ({"id": "91", "content_file_name": "plan.txt", "content_file_size": 3}, "ok (3 B)"),
    ({"id": 91, "name": "renamed.txt", "size": 3}, "server name"),
    ({"id": 91, "name": "plan.txt", "size": 99}, "mismatch"),
])
def test_upload_receipt_diagnostics_and_payload_bytes(wire, capsys, response, diagnostic):
    wire["upload_result"] = response
    assert create._upload_bytes("plan.txt", b"new", "text/plain", wire["client"]) == 91
    assert diagnostic in capsys.readouterr().err
    assert wire["writes"] == [("POST", "attachments", {
        "content": "data:text/plain;base64,bmV3", "content_file_name": "plan.txt",
        "content_file_size": 3, "content_content_type": "text/plain"})]


def test_invalid_duplicate_mode_never_uploads(wire):
    with pytest.raises(UsageError, match="--duplicate"):
        create.attach_files_to_change(123, ["missing.txt"], duplicate="typo")
    assert wire["writes"] == []


def test_download_error_preserves_existing_bytes_even_with_force(wire, tmp_path):
    target = tmp_path / "plan.txt"
    target.write_bytes(b"local bytes")
    wire["fail"] = "download"
    with pytest.raises(APIError) as exc:
        create.download_attachment({"name": "plan.txt", "attachment_url": "https://example.com/old"}, tmp_path, force=True)
    assert exc.value.status == 503 and target.read_bytes() == b"local bytes"
    assert wire["downloads"] == ["https://example.com/old"]


def test_missing_download_url_does_not_create_output_directory(wire, tmp_path):
    out = tmp_path / "absent"
    with pytest.raises(APIError, match="has no URL"):
        create.download_attachment({"name": "plan.txt"}, out)
    assert not out.exists() and wire["downloads"] == []
