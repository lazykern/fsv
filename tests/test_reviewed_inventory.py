"""Raw mutation inventories must not inherit lossy display pagination semantics."""
import copy

import pytest

from fsv.reviewed_inventory import complete_inventory, single_page_inventory
from fsv.reviewed_write import ReviewedWriteError


class Pages:
    def __init__(self, pages):
        self.pages = pages
        self.requests = []

    def int_get(self, path, params=None):
        self.requests.append((path, params))
        page = (params or {}).get("page", 1)
        return copy.deepcopy(self.pages[page - 1])

    v2_get = int_get


@pytest.mark.parametrize("raw", [
    {"rows": [{"id": 71}, "malformed"], "has_more": False},
    {"rows": [{"id": 71}, {"id": 71}], "has_more": False},
    {"rows": [{"id": True}], "has_more": False},
    {"rows": [{"id": "superscript"}], "has_more": False},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"total": 2}},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"total": "1"}},
    {"rows": [{"id": 71}], "has_more": "false"},
    {"rows": [{"id": 71}], "has_more": False, "meta": "malformed"},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"has_more": True}},
    {"rows": [{"id": 71}], "has_more": True, "next_page_url": None},
    {"rows": [{"id": 71}], "has_more": False, "next_page_url": "?page=2"},
    {"rows": [{"id": 71}], "next_page": 2},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"page": True}},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"total_pages": 2}},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"total_pages": 0}},
])
def test_raw_page_rejected_before_display_coercion(raw):
    with pytest.raises(ReviewedWriteError, match="complete"):
        complete_inventory(Pages([raw]), "example", "rows", {"page": 1, "per_page": 100})


def test_duplicate_across_pages_and_conflicting_totals_fail_closed():
    for second in [{"id": 71}, {"id": 72}]:
        pages = Pages([{"rows": [{"id": 71}], "has_more": True, "total": 2},
                       {"rows": [second], "has_more": False, "total": 3}])
        with pytest.raises(ReviewedWriteError, match="complete"):
            complete_inventory(pages, "example", "rows", {"page": 1, "per_page": 1})


@pytest.mark.parametrize("api", ["internal", "v2"])
def test_complete_multi_page_inventory_preserves_filters(api):
    pages = Pages([{"rows": [{"id": 72}], "has_more": True, "meta": {"total_count": 2}},
                   {"rows": [{"id": 71}], "has_more": False, "meta": {"total_count": 2}}])
    rows = complete_inventory(pages, "example", "rows", {"page": 1, "per_page": 1, "query_hash": "synthetic"}, api=api)
    assert [r["id"] for r in rows] == [71, 72]
    assert len(pages.requests) == 2
    assert all(params["query_hash"] == "synthetic" for _, params in pages.requests)


@pytest.mark.parametrize("metadata", [{"total_count": 2}, {"has_more": True}, {"next_page_url": "?page=2"}, {"total_pages": 2}])
def test_single_page_inventory_never_follows_unsupported_continuation(metadata):
    with pytest.raises(ReviewedWriteError, match="complete"):
        single_page_inventory({"rows": [{"id": 71}], "meta": metadata}, "rows")


def test_single_page_native_unpaginated_and_empty_shapes():
    assert single_page_inventory({"rows": [{"id": 71}]}, "rows") == [{"id": 71}]
    assert single_page_inventory({"rows": [], "meta": {"page": 1, "total_pages": 0, "total": 0}}, "rows") == []
