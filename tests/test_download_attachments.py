from __future__ import annotations

from fsv import config, create


class Response:
    status_code = 200
    headers = {"content-disposition": 'attachment; filename="from-header.txt"'}
    text = ""
    content = b"hello"


class HTTPClient:
    def __init__(self):
        self.urls = []

    def get(self, url, follow_redirects=False):
        self.urls.append((url, follow_redirects))
        return Response()


class FakeClient:
    def __init__(self):
        self._client = HTTPClient()


class PlanningClient:
    def __init__(self, existing):
        self.existing = existing
        self.puts = []
        self.posts = []

    def int_get(self, path):
        assert path == "changes/1/planning-fields"
        return {"change_planning_fields": [self.existing] if self.existing else []}

    def int_put(self, path, body):
        self.puts.append((path, body))
        return body

    def int_post(self, path, body):
        self.posts.append((path, body))
        return body


class MainAttachClient:
    def __init__(self, attachments):
        self.attachments = attachments
        self.puts = []

    def int_get(self, path):
        assert path == "changes/1"
        return {"change": {"attachments": self.attachments}}

    def int_put(self, path, body):
        self.puts.append((path, body))
        return body


def test_download_attachment_uses_name_and_skips_existing(tmp_path):
    client = FakeClient()
    att = {"attachment_url": "https://example.test/a", "name": "a/b.xlsx"}

    first = create.download_attachment(att, tmp_path, c=client)
    second = create.download_attachment(att, tmp_path, c=client)

    assert first["name"] == "a-b.xlsx"
    assert first["size"] == 5
    assert not first["skipped"]
    assert second["skipped"]
    assert (tmp_path / "a-b.xlsx").read_bytes() == b"hello"
    assert client._client.urls == [("https://example.test/a", True)]


def test_download_attachment_uses_canonical_path(tmp_path, monkeypatch):
    client = FakeClient()
    monkeypatch.setattr(config, "DOMAIN", "fresh.example")

    result = create.download_attachment({"canonical_path": "/helpdesk/attachments/123"}, tmp_path, c=client)

    assert result["name"] == "from-header.txt"
    assert client._client.urls == [("https://fresh.example/helpdesk/attachments/123?download=true", True)]


def test_update_planning_field_defaults_description_for_file_only_existing_empty(monkeypatch, tmp_path):
    path = tmp_path / "evidence.xlsx"
    path.write_bytes(b"x")
    client = PlanningClient({"name": "change_impact", "attachments": []})
    monkeypatch.setattr(create, "upload_file", lambda p, c: 123)

    create.update_planning_field(1, "change_impact", file_paths=[str(path)], c=client)

    assert client.puts == [("changes/1/planning-fields/change_impact", {
        "description": "evidence.xlsx",
        "attachments": [123],
    })]


def test_update_planning_field_preserves_existing_description(monkeypatch, tmp_path):
    path = tmp_path / "evidence.xlsx"
    path.write_bytes(b"x")
    client = PlanningClient({"name": "change_impact", "description_text": "Existing", "attachments": [{"id": 7}]})
    monkeypatch.setattr(create, "upload_file", lambda p, c: 123)

    create.update_planning_field(1, "change_impact", file_paths=[str(path)], c=client)

    assert client.puts == [("changes/1/planning-fields/change_impact", {"attachments": [7, 123]})]


def test_update_planning_field_duplicate_skip(monkeypatch, tmp_path):
    path = tmp_path / "evidence.xlsx"
    path.write_bytes(b"x")
    client = PlanningClient({"name": "change_impact", "description_text": "Existing", "attachments": [{"id": 7, "name": "evidence.xlsx"}]})
    uploads = []
    monkeypatch.setattr(create, "upload_file", lambda p, c: uploads.append(p) or 123)

    result = create.update_planning_field(1, "change_impact", file_paths=[str(path)], c=client, duplicate="skip")

    assert uploads == []
    assert client.puts == []
    assert result == {"_fsv_noop": True, "skipped": ["evidence.xlsx"], "planning_field": "change_impact"}


def test_update_planning_field_duplicate_append(monkeypatch, tmp_path):
    path = tmp_path / "evidence.xlsx"
    path.write_bytes(b"x")
    client = PlanningClient({"name": "change_impact", "description_text": "Existing", "attachments": [{"id": 7, "name": "evidence.xlsx"}]})
    monkeypatch.setattr(create, "upload_file", lambda p, c: 123)

    create.update_planning_field(1, "change_impact", file_paths=[str(path)], c=client, duplicate="append")

    assert client.puts == [("changes/1/planning-fields/change_impact", {"attachments": [7, 123]})]


def test_update_planning_field_duplicate_replace_no_backup(monkeypatch, tmp_path):
    path = tmp_path / "evidence.xlsx"
    path.write_bytes(b"x")
    client = PlanningClient({"name": "change_impact", "description_text": "Existing", "attachments": [{"id": 7, "name": "evidence.xlsx"}, {"id": 8, "name": "other.xlsx"}]})
    monkeypatch.setattr(create, "upload_file", lambda p, c: 123)

    create.update_planning_field(1, "change_impact", file_paths=[str(path)], c=client, duplicate="replace", backup_replaced=False)

    assert client.puts == [("changes/1/planning-fields/change_impact", {"attachments": [8, 123]})]


def test_attach_files_to_change_duplicate_skip_noop(monkeypatch, tmp_path):
    path = tmp_path / "evidence.xlsx"
    path.write_bytes(b"x")
    client = MainAttachClient([{"id": 7, "name": "evidence.xlsx"}])
    uploads = []
    monkeypatch.setattr(create, "upload_file", lambda p, c: uploads.append(p) or 123)

    result = create.attach_files_to_change(1, [str(path)], c=client, duplicate="skip")

    assert uploads == []
    assert client.puts == []
    assert result == {"_fsv_noop": True, "skipped": ["evidence.xlsx"], "attachments": [7]}


def test_attach_files_to_change_duplicate_append(monkeypatch, tmp_path):
    path = tmp_path / "evidence.xlsx"
    path.write_bytes(b"x")
    client = MainAttachClient([{"id": 7, "name": "evidence.xlsx"}])
    monkeypatch.setattr(create, "upload_file", lambda p, c: 123)

    result = create.attach_files_to_change(1, [str(path)], c=client, duplicate="append")

    assert client.puts == [("changes/1", {"attachments": [7, 123]})]
    assert result == {"attachments": [7, 123], "skipped": []}


def test_attach_files_to_change_duplicate_replace_no_backup(monkeypatch, tmp_path):
    path = tmp_path / "evidence.xlsx"
    path.write_bytes(b"x")
    client = MainAttachClient([{"id": 7, "name": "evidence.xlsx"}, {"id": 8, "name": "other.xlsx"}])
    monkeypatch.setattr(create, "upload_file", lambda p, c: 123)

    result = create.attach_files_to_change(1, [str(path)], c=client, duplicate="replace", backup_replaced=False)

    assert client.puts == [("changes/1", {"attachments": [8, 123]})]
    assert result == {"attachments": [8, 123], "skipped": []}
