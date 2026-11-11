"""A tiny, safe Markdown -> HTML converter for descriptions.

Supports what change/ticket descriptions need: paragraphs, line breaks,
``#`` headings, ``-``/``*``/``1.`` lists (one level), fenced code blocks,
``inline code``, **bold**, *italic*, and ``[text](url)`` links with
http(s)/mailto URLs only. All text is HTML-escaped first, so raw HTML in the
Markdown is shown as text, never injected. For raw HTML use
``description_html``.
"""
from __future__ import annotations

import html
import re

_HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
_UL = re.compile(r"^\s*[-*+]\s+(.*)$")
_OL = re.compile(r"^\s*\d+[.)]\s+(.*)$")
_FENCE = re.compile(r"^\s*(```|~~~)")
_LINK = re.compile(r"\[([^\]]+)\]\(([^)\s]+)\)")
_CODE = re.compile(r"`([^`]+)`")
_BOLD = re.compile(r"\*\*(.+?)\*\*|__(.+?)__")
_ITALIC = re.compile(r"(?<![*\w])\*(?!\s)(.+?)(?<!\s)\*(?![*\w])|(?<![_\w])_(?!\s)(.+?)(?<!\s)_(?![_\w])")
_SAFE_URL = re.compile(r"^(?:https?://|mailto:)", re.I)


def _inline(text: str) -> str:
    """Escape, then apply inline markup. Code spans are protected."""
    codes: list[str] = []

    def stash(m: re.Match[str]) -> str:
        codes.append(f"<code>{html.escape(m.group(1), quote=False)}</code>")
        return f"\x00{len(codes) - 1}\x00"

    text = _CODE.sub(stash, text)
    out = html.escape(text, quote=False)

    def link(m: re.Match[str]) -> str:
        label, url = m.group(1), html.unescape(m.group(2))
        if not _SAFE_URL.match(url):
            return m.group(0)
        return f'<a href="{html.escape(url, quote=True)}">{label}</a>'

    out = _LINK.sub(link, out)
    out = _BOLD.sub(lambda m: f"<strong>{m.group(1) or m.group(2)}</strong>", out)
    out = _ITALIC.sub(lambda m: f"<em>{m.group(1) or m.group(2)}</em>", out)
    return re.sub(r"\x00(\d+)\x00", lambda m: codes[int(m.group(1))], out)


def to_html(md: str) -> str:
    lines = str(md).replace("\r\n", "\n").split("\n")
    out: list[str] = []
    para: list[str] = []
    items: list[str] = []
    list_tag = ""
    i = 0

    def flush_para() -> None:
        if para:
            out.append("<p>" + "<br>".join(_inline(p) for p in para) + "</p>")
            para.clear()

    def flush_list() -> None:
        nonlocal list_tag
        if items:
            out.append(f"<{list_tag}>" + "".join(f"<li>{_inline(x)}</li>" for x in items) + f"</{list_tag}>")
            items.clear()
        list_tag = ""

    while i < len(lines):
        line = lines[i]
        if _FENCE.match(line):
            flush_para()
            flush_list()
            fence = _FENCE.match(line).group(1)  # type: ignore[union-attr]
            code: list[str] = []
            i += 1
            while i < len(lines) and not lines[i].strip().startswith(fence):
                code.append(lines[i])
                i += 1
            out.append("<pre><code>" + html.escape("\n".join(code), quote=False) + "</code></pre>")
            i += 1
            continue
        if not line.strip():
            flush_para()
            flush_list()
            i += 1
            continue
        m = _HEADING.match(line)
        if m:
            flush_para()
            flush_list()
            n = len(m.group(1))
            out.append(f"<h{n}>{_inline(m.group(2))}</h{n}>")
            i += 1
            continue
        mu, mo = _UL.match(line), _OL.match(line)
        if mu or mo:
            flush_para()
            tag = "ul" if mu else "ol"
            if list_tag and list_tag != tag:
                flush_list()
            list_tag = tag
            items.append((mu or mo).group(1))  # type: ignore[union-attr]
            i += 1
            continue
        if items and line.startswith((" ", "\t")):
            items[-1] += " " + line.strip()  # continuation of a list item
            i += 1
            continue
        flush_list()
        para.append(line.strip())
        i += 1
    flush_para()
    flush_list()
    return "".join(out) or "<p></p>"
