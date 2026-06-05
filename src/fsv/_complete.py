"""Thin dynamic completion entrypoint — no typer import.

Usage:
    python -m fsv._complete <shell> <completer_name> <resource_or_-> <token> [-- <cmdline_tokens...>]

<shell>           fish | bash | zsh
<completer_name>  module-level function name from fsv.completion, or a factory name
<resource_or_->   resource name (changes/tickets/problems) or '-' for plain completers
<token>           the current incomplete token
[-- <tokens>]     optional full commandline tokens for ctx-dependent completers
"""
from __future__ import annotations

import os
import sys
from typing import Iterable

os.environ.setdefault("FSV_THIN_COMPLETE", "1")


def _completions(
    completer_name: str,
    resource_name: str,
    token: str,
    extra_tokens: list[str],
) -> Iterable[tuple[str, str]]:
    from fsv import completion as _c
    from fsv.resources import REGISTRY

    FACTORIES: dict[str, object] = {
        "complete_where": _c.complete_where,
        "complete_set": _c.complete_set,
        "complete_filter_name": _c.complete_filter_name,
        "complete_field_names": _c.complete_field_names,
        "complete_choice_field_names": _c.complete_choice_field_names,
        "complete_planning_field_names": _c.complete_planning_field_names,
        "complete_lookup_kind": _c.complete_lookup_kind,
        "complete_lookup_query": _c.complete_lookup_query,
        "complete_update_agent_id": None,
        "complete_update_group_id": None,
    }
    PLAIN: dict[str, object] = {
        "complete_format": _c.complete_format,
        "complete_store": _c.complete_store,
        "complete_shell": _c.complete_shell,
        "complete_sort_order": _c.complete_sort_order,
        "complete_config_key": _c.complete_config_key,
        "complete_config_value": _c.complete_config_value,
        "complete_cache_target": _c.complete_cache_target,
        "complete_search_sort": _c.complete_search_sort,
        "complete_duplicate_mode": _c.complete_duplicate_mode,
        "complete_group_query": _c.complete_group_query,
        "complete_update_agent_id": _c.complete_update_agent_id,
        "complete_update_group_id": _c.complete_update_group_id,
    }

    if completer_name in ("complete_update_agent_id", "complete_update_group_id"):
        yield from _c.complete_update_agent_id(token) if completer_name == "complete_update_agent_id" else _c.complete_update_group_id(token)
        return

    if completer_name in PLAIN:
        fn = PLAIN[completer_name]
        yield from fn(token)  # type: ignore[operator]
        return

    if completer_name in FACTORIES:
        res = REGISTRY.get(resource_name)
        if res is None:
            return

        if completer_name == "complete_lookup_query":
            # Needs ctx.params['kind'] — extract from extra_tokens
            kind = _extract_flag(extra_tokens, "--kind", "-k")
            fn = _c.complete_lookup_query(res)
            ctx = _MockCtx({"kind": kind})
            yield from fn(ctx, token)  # type: ignore[operator]
            return

        factory = FACTORIES[completer_name]
        fn = factory(res)  # type: ignore[operator]
        yield from fn(token)  # type: ignore[operator]
        return


def _extract_flag(tokens: list[str], *flags: str) -> str:
    for i, tok in enumerate(tokens):
        if tok in flags and i + 1 < len(tokens):
            return tokens[i + 1]
        for f in flags:
            if tok.startswith(f + "="):
                return tok.split("=", 1)[1]
    return ""


class _MockCtx:
    def __init__(self, params: dict[str, object]) -> None:
        self.params = params


def main() -> None:
    args = sys.argv[1:]
    if len(args) < 4:
        return

    shell = args[0]
    completer_name = args[1]
    resource_name = args[2]
    token = args[3]

    extra_tokens: list[str] = []
    if "--" in args[4:]:
        sep = args.index("--", 4)
        extra_tokens = args[sep + 1 :]

    try:
        results = list(_completions(completer_name, resource_name, token, extra_tokens))
    except Exception:
        return

    for item in results:
        if isinstance(item, tuple) and len(item) == 2:
            value, help_text = item[0], item[1]
        else:
            value, help_text = str(item), ""
        if not value or value == token:
            continue
        if shell == "fish":
            print(f"{value}\t{help_text}" if help_text else value)
        else:
            print(value)


if __name__ == "__main__":
    main()
