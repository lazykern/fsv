"""YAML for drafts, presets and hints: strict, predictable scalars.

PyYAML's default (YAML 1.1) loader turns ``No`` into ``False``, ``22:30``
into ``1350`` and ``2030-01-01`` into a date. Form answers are labels, so
here every plain scalar stays a string except ``null``/``~``/empty and
``true``/``false``. Quote a value to keep ``"true"`` as text.
"""
from __future__ import annotations

import json
import re
from typing import Any

import yaml


class _Loader(yaml.SafeLoader):
    pass


# drop every implicit resolver (bool yes/no, int, float, timestamp, ...)
_Loader.yaml_implicit_resolvers = {}
_Loader.add_implicit_resolver(
    "tag:yaml.org,2002:null", re.compile(r"^(?:~|null|Null|NULL|)$"), ["~", "n", "N", ""]
)
_Loader.add_implicit_resolver(
    "tag:yaml.org,2002:bool", re.compile(r"^(?:true|True|TRUE|false|False|FALSE)$"), list("tTfF")
)


def loads(text: str) -> Any:
    """Parse YAML; raises ValueError (with line info) on syntax errors."""
    try:
        return yaml.load(text, Loader=_Loader)  # noqa: S506 - restricted SafeLoader subclass
    except yaml.YAMLError as e:
        raise ValueError(f"YAML error: {' '.join(str(e).split())}") from None


def load_file(path: str) -> Any:
    with open(path, encoding="utf-8") as fh:
        return loads(fh.read())


_PLAIN = re.compile(r"^[^\s#:\-?\[\]{},&*!|>'\"%@`][^#\n]*$")
_RESERVED = re.compile(r"^(?:~|null|Null|NULL|true|True|TRUE|false|False|FALSE)$")


def scalar(value: Any) -> str:
    """One YAML scalar that reads back as the same value with :func:`loads`."""
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    s = str(value)
    if (
        _PLAIN.match(s)
        and not _RESERVED.match(s)
        and s == s.strip()
        and ": " not in s
        and not s.endswith(":")
    ):
        return s
    return json.dumps(s, ensure_ascii=False)


def key(text: str) -> str:
    """A mapping key (labels may contain ':' or '#')."""
    return scalar(text)


_FLOW_PLAIN = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.@/ +()-]*$")


def flow_list(values: list[Any]) -> str:
    def item(v: Any) -> str:
        if isinstance(v, str) and not (_FLOW_PLAIN.match(v) and v == v.strip() and not _RESERVED.match(v)):
            return json.dumps(v, ensure_ascii=False)
        return scalar(v)

    return "[" + ", ".join(item(v) for v in values) + "]"


def block(text: str, indent: int) -> str:
    """``|-`` block scalar for multi-line text (reads back without a final
    newline)."""
    pad = " " * indent
    lines = str(text).replace("\r\n", "\n").rstrip("\n").split("\n")
    return "|-\n" + "\n".join((pad + ln) if ln else "" for ln in lines)
