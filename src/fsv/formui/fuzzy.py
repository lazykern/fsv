"""Type-to-filter matching for dropdowns, checklists and pickers.

Case-insensitive and accent-insensitive (``"anderung"`` finds
``"Änderung"``), over the label and the choice description. Ranking, best
first: label starts with the query, a word of the label starts with it, the
label contains it, the description contains it, the label contains the
query's letters in order (``"stg"`` -> ``"Staging"``). Ties keep the form's
order.
"""
from __future__ import annotations

import unicodedata
from typing import Sequence


def fold(text: object) -> str:
    """Casefolded, NFKD with combining marks dropped, whitespace collapsed."""
    s = unicodedata.normalize("NFKD", str(text or ""))
    s = "".join(ch for ch in s if not unicodedata.combining(ch))
    return " ".join(s.casefold().split())


def _subsequence(needle: str, hay: str) -> bool:
    it = iter(hay)
    return all(ch in it for ch in needle)


def score(query: str, label: str, description: str = "") -> int | None:
    """Rank of a match (lower is better) or None for no match."""
    q = fold(query)
    if not q:
        return 0
    lab = fold(label)
    if lab.startswith(q):
        return 0
    words = lab.replace("-", " ").replace("/", " ").replace("(", " ").split()
    if any(w.startswith(q) for w in words):
        return 1
    if q in lab:
        return 2
    if description and q in fold(description):
        return 3
    compact = q.replace(" ", "")
    if compact and _subsequence(compact, lab.replace(" ", "")):
        return 4
    return None


def fuzzy_filter(query: str, items: Sequence[tuple[str, str]]) -> list[int]:
    """Indexes of ``items`` (label, description) matching ``query``, best first."""
    ranked = []
    for i, (label, desc) in enumerate(items):
        s = score(query, label, desc)
        if s is not None:
            ranked.append((s, i))
    ranked.sort()
    return [i for _, i in ranked]
