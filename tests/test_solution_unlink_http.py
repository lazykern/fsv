"""Exercise solution unlink through the real HTTP parser, not request mocks."""
import json
import httpx
import pytest
from fsv import config, solutions
from fsv.reviewed_write import ReviewedWriteError
from test_client_session import make_client


@pytest.mark.parametrize("status,body", [(200, b""), (204, b""), (200, b" \n"),
                                        (200, b"null"), (200, b'{"success":true}'),
                                        (200, b'{"problem":{"id":23}}')])
@pytest.mark.parametrize("persist", [True, False])
def test_unlink_body_shapes_require_independent_absence(monkeypatch, tmp_path, status, body, persist):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setenv("FSV_CACHE_DIR", str(tmp_path))
    state = {"linked": True, "writes": 0}
    article = {"id": 91, "article_type": 2, "title": "Acme guide"}
    def handler(request):
        path = request.url.path.removeprefix("/api/_/")
        if request.method == "PUT":
            assert path == "problems/23/solutions/91"
            assert json.loads(request.content) == {"type": "unlink"}
            state["writes"] += 1
            if persist:
                state["linked"] = False
            return httpx.Response(status, content=body, headers={"Content-Type": "application/json"})
        if path == "problems/23":
            return httpx.Response(200, json={"problem": {"id": 23, "subject": "Acme problem"}})
        if path == "problems/23/solutions":
            return httpx.Response(200, json={"articles": [article] if state["linked"] else [], "has_more": False})
        assert path == "solutions/articles/91"
        return httpx.Response(200, json={"article": article})
    client = make_client(handler)
    monkeypatch.setattr(client, "_get_csrf", lambda: "synthetic")
    try:
        plan = solutions.link_plan(client, 23, 91, unlink=True)
        result = solutions.send_link(plan, client, reviewed_hash=plan.hash)
        assert result["status"] == "submitted"
        assert result["verified"] is persist
        assert state["writes"] == 1
        with pytest.raises(ReviewedWriteError, match="outcome unknown"):
            solutions.send_link(plan, client, reviewed_hash=plan.hash)
        assert state["writes"] == 1
    finally:
        client._client.close()


@pytest.mark.parametrize("status,body", [(200, b"{broken"), (400, b""), (500, b"")])
def test_nonempty_malformed_json_and_http_errors_still_fail(monkeypatch, tmp_path, status, body):
    from fsv.errors import APIError
    monkeypatch.setenv("FSV_CACHE_DIR", str(tmp_path))
    client = make_client(lambda request: httpx.Response(status, content=body, headers={"Content-Type": "application/json"}))
    try:
        with pytest.raises(ValueError if status == 200 else APIError):
            client._request("PUT", "https://acme.example.com/api/_/problems/23/solutions/91")
    finally:
        client._client.close()
