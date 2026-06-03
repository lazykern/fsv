from __future__ import annotations

import ast
from pathlib import Path

import fsv.cli as cli
from fsv import completion


CLI_PATH = Path(__file__).resolve().parents[1] / "src" / "fsv" / "cli.py"


def _param_keywords(fn: ast.FunctionDef, param_name: str) -> dict[str, str]:
    pos = fn.args.args
    defaults = fn.args.defaults
    start = len(pos) - len(defaults)
    pairs = []
    for i, arg in enumerate(pos):
        default = defaults[i - start] if i >= start else None
        pairs.append((arg.arg, default))
    for arg, default in zip(fn.args.kwonlyargs, fn.args.kw_defaults):
        pairs.append((arg.arg, default))
    for name, default in pairs:
        if name != param_name or not isinstance(default, ast.Call):
            continue
        if ast.unparse(default.func) not in {"typer.Option", "typer.Argument"}:
            continue
        return {kw.arg: ast.unparse(kw.value) for kw in default.keywords if kw.arg}
    raise AssertionError(f"param {param_name!r} not found in {fn.name}")


def _functions(name: str) -> list[ast.FunctionDef]:
    mod = ast.parse(CLI_PATH.read_text())
    return [node for node in ast.walk(mod) if isinstance(node, ast.FunctionDef) and node.name == name]


def test_choice_completion_helpers(monkeypatch):
    assert ("auth", "help topic") in list(completion.complete_help_topic(["auth", "workflow"])("a"))
    assert ("fish", "Fish shell") in list(completion.complete_shell("f"))
    assert ("created", "created time") in list(completion.complete_search_sort("cr"))
    assert ("desc", "descending") in list(completion.complete_sort_order("d"))
    assert ("replace", "replace existing file") in list(completion.complete_duplicate_mode("r"))

    monkeypatch.setattr(cli, "_task_field_choice_pairs", lambda field: [("Core System", "id=107"), ("107", "Core System")])
    monkeypatch.setattr(cli, "_task_observed_value_pairs", lambda ctx, field: [("109", "observed on current change")])
    values = list(cli._complete_task_system(None, "1"))
    assert ("107", "Core System") in values
    assert ("109", "observed on current change") in values


def test_task_choice_value_uses_dynamic_field_defs(monkeypatch):
    monkeypatch.setattr(cli, "_task_field_defs", lambda: [{"name": "cf_environment", "choices": [{"id": 11, "value": "Production"}, {"id": 12, "value": "UAT"}]}])

    assert cli._task_choice_value("Production", "environment") == 11
    assert cli._task_choice_value("12", "environment") == 12


def test_help_and_completion_commands_wired():
    help_fn = _functions("help_topic")[0]
    assert _param_keywords(help_fn, "topic")["autocompletion"] == "completion.complete_help_topic(HELP_TOPICS)"

    show_fn = _functions("completion_show")[0]
    assert _param_keywords(show_fn, "shell")["autocompletion"] == "completion.complete_shell"

    install_fn = _functions("completion_install")[0]
    assert _param_keywords(install_fn, "shell")["autocompletion"] == "completion.complete_shell"


def test_search_and_list_wiring():
    global_search = _functions("global_search_cmd")[0]
    assert _param_keywords(global_search, "sort")["autocompletion"] == "completion.complete_search_sort"
    assert _param_keywords(global_search, "format_")["autocompletion"] == "completion.complete_format"

    for fn in _functions("search"):
        assert _param_keywords(fn, "sort")["autocompletion"] == "completion.complete_search_sort"

    ls_fn = _functions("ls")[0]
    assert _param_keywords(ls_fn, "filter_name")["autocompletion"] == "completion.complete_filter_name(res)"
    assert _param_keywords(ls_fn, "order_by")["autocompletion"] == "completion.complete_field_names(res)"
    assert _param_keywords(ls_fn, "order_type")["autocompletion"] == "completion.complete_sort_order"


def test_format_wiring_for_target_commands():
    assert _param_keywords(_functions("tasks")[0], "format_")["autocompletion"] == "completion.complete_format"
    assert _param_keywords(_functions("assets")[0], "format_")["autocompletion"] == "completion.complete_format"

    for fn in _functions("approvals"):
        assert _param_keywords(fn, "format_")["autocompletion"] == "completion.complete_format"

    for fn in _functions("associations"):
        assert _param_keywords(fn, "format_")["autocompletion"] == "completion.complete_format"


def test_change_update_wiring():
    task_update = _functions("task_update")[0]
    assert _param_keywords(task_update, "system")["autocompletion"] == "_complete_task_system"

    updates = _functions("update")
    change_update = next(fn for fn in updates if any(arg.arg == "duplicate" for arg in fn.args.args))
    assert _param_keywords(change_update, "duplicate")["autocompletion"] == "completion.complete_duplicate_mode"
