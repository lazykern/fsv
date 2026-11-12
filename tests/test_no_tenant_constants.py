"""Lint: no numeric status/reminder constants in src outside an allowlist of
platform constants. Tenant-configurable ids (ticket/problem/task statuses,
reminder seconds) must come from runtime metadata; a literal here would pass
on a default tenant and silently send a wrong value on a shifted one."""
from __future__ import annotations

import ast
import pathlib
import re

SRC = pathlib.Path(__file__).resolve().parents[1] / "src" / "fsv"
KEYS = {"status", "status_id", "notify_before", "approval_status", "reminder", "task_status"}
NAME_RE = re.compile(r"(STATUS|NOTIFY|REMINDER)", re.IGNORECASE)

# (file, finding) -> why it is a platform constant, not tenant configuration.
ALLOWLIST = {
    ("change_tasks.py", "assign NOTIFY_BEFORE"): "fallback only for library plans built without runtime "
                                                 "metadata; CLI/TUI/send always use change_task_fields",
    ("client.py", "assign _RETRYABLE_STATUS"): "HTTP status codes",
    ("client.py", "assign last_status"): "HTTP status placeholder",
    ("solutions.py", "dict status"): "knowledge-base article draft status (platform enum)",
}


def _ints(node: ast.AST) -> list[int]:
    """Int literals that are values of a literal (not subscripts/indices)."""
    if isinstance(node, ast.Constant):
        return [node.value] if type(node.value) is int else []
    if isinstance(node, ast.Dict):
        return [i for v in node.values if v is not None for i in _ints(v)]
    if isinstance(node, (ast.Tuple, ast.List, ast.Set)):
        return [i for e in node.elts for i in _ints(e)]
    return []


def findings() -> list[tuple[str, int, str]]:
    out = []
    for path in sorted(SRC.rglob("*.py")):
        rel = str(path.relative_to(SRC))
        for node in ast.walk(ast.parse(path.read_text())):
            if isinstance(node, ast.Dict):
                for k, v in zip(node.keys, node.values):
                    if isinstance(k, ast.Constant) and k.value in KEYS and isinstance(v, ast.Constant) and type(v.value) is int:
                        out.append((rel, node.lineno, f"dict {k.value}"))
            elif isinstance(node, ast.Call):
                for kw in node.keywords:
                    if kw.arg in KEYS and isinstance(kw.value, ast.Constant) and type(kw.value.value) is int:
                        out.append((rel, node.lineno, f"kwarg {kw.arg}"))
            elif isinstance(node, (ast.Assign, ast.AnnAssign)) and node.value is not None:
                for t in node.targets if isinstance(node, ast.Assign) else [node.target]:
                    name = t.id if isinstance(t, ast.Name) else t.attr if isinstance(t, ast.Attribute) else None
                    if name and NAME_RE.search(name) and _ints(node.value):
                        out.append((rel, node.lineno, f"assign {name}"))
            elif isinstance(node, ast.arguments):
                args = node.args + node.kwonlyargs
                defaults = [None] * (len(node.args) - len(node.defaults)) + list(node.defaults) + list(node.kw_defaults)
                for a, d in zip(args, defaults):
                    if a.arg in KEYS and isinstance(d, ast.Constant) and type(d.value) is int:
                        out.append((rel, a.lineno, f"default {a.arg}"))
    return out


def test_no_numeric_status_or_reminder_constants_outside_the_allowlist():
    bad = [f"{rel}:{line}: {what}" for rel, line, what in findings() if (rel, what) not in ALLOWLIST]
    assert not bad, "tenant-configurable numeric constants (use runtime metadata or allowlist a platform enum):\n" + "\n".join(bad)


def test_allowlist_has_no_stale_entries():
    seen = {(rel, what) for rel, _, what in findings()}
    assert set(ALLOWLIST) <= seen, set(ALLOWLIST) - seen


def test_the_lint_catches_a_planted_constant(tmp_path, monkeypatch):
    planted = tmp_path / "fsv"
    planted.mkdir()
    (planted / "bad.py").write_text("TASK_STATUS = {'Open': 1}\n\ndef f(status_id=1):\n    return {'status': 2}\n")
    monkeypatch.setattr(__import__(__name__), "SRC", planted)
    assert sorted(w for _, _, w in findings()) == ["assign TASK_STATUS", "default status_id", "dict status"]
