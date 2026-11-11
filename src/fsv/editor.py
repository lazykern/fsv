"""Editor-based create/clone workflow."""

import hashlib
import json
import os
import shlex
import subprocess
import sys
import tempfile
from typing import Any, Callable

ERROR_MARK = "ERROR:"


def _editor() -> list[str]:
    editor = os.environ.get("VISUAL") or os.environ.get("EDITOR") or "vi"
    return shlex.split(editor)


class EditorAbort(Exception):
    pass


def _digest(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _strip_error_header(text: str, prefix: str) -> str:
    """Drop the error lines a previous round put at the top."""
    marker = f"{prefix} {ERROR_MARK}"
    out = [ln for ln in text.splitlines() if not ln.lstrip().startswith(marker)]
    return "\n".join(out) + ("\n" if text.endswith("\n") else "")


def edit_text(
    initial: str,
    parse: Callable[[str], Any],
    *,
    suffix: str = ".txt",
    comment_prefix: str = "#",
    max_rounds: int = 20,
) -> Any:
    """Open the editor on *initial*, return ``parse(edited_text)``.

    If ``parse`` raises ValueError (json.JSONDecodeError is one; wrap YAML
    errors in ValueError) the editor is reopened on the same file with the
    error at the top, so edits are not lost. "Unchanged" is decided by content
    hash, not mtime. Raises EditorAbort on unchanged/too many bad rounds,
    SystemExit when the editor cannot be started.
    """
    with tempfile.NamedTemporaryFile(mode="w", suffix=suffix, delete=False, encoding="utf-8") as f:
        f.write(initial)
        tmp_path = f.name
    shown = initial
    try:
        for _ in range(max_rounds):
            try:
                subprocess.run([*_editor(), tmp_path], check=False)
            except FileNotFoundError:
                print(f"error: editor not found: {_editor()[0]}; set $VISUAL or $EDITOR", file=sys.stderr)
                raise SystemExit(1)
            with open(tmp_path, encoding="utf-8") as fh:
                text = fh.read()
            if _digest(text) == _digest(shown):
                raise EditorAbort("aborted: file unchanged")
            body = _strip_error_header(text, comment_prefix)
            try:
                return parse(body)
            except ValueError as e:
                msg = " ".join(str(e).split())
                shown = (f"{comment_prefix} {ERROR_MARK} {msg}\n"
                         f"{comment_prefix} {ERROR_MARK} fix and save, or leave unchanged to abort\n{body}")
                with open(tmp_path, "w", encoding="utf-8") as fh:
                    fh.write(shown)
        raise EditorAbort("aborted: too many invalid edits")
    finally:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass


def edit_json(data: dict[str, Any], hint: str = "") -> dict[str, Any]:
    """Open $VISUAL/$EDITOR with JSON data, return parsed result.

    The file starts with JSONC comment lines explaining the fields.
    Lines starting with // or # are stripped before parsing JSON. On a JSON
    error the editor reopens with the error at the top.
    Raises EditorAbort on cancel/unchanged/empty, SystemExit on editor not found.
    """
    comment = f"// {hint}\n// Edit JSON below. Comment lines ignored.\n// Save + close to continue; leave unchanged to abort.\n\n"
    raw = comment + json.dumps(data, indent=2, ensure_ascii=False, default=str)
    edited = edit_text(raw, _parse_json_text, suffix=".jsonc", comment_prefix="//")
    if not edited:
        raise EditorAbort("aborted: empty file")
    return edited


def _parse_json_text(text: str) -> dict[str, Any]:
    lines = []
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or stripped.startswith("//"):
            continue
        lines.append(line)
    if not lines:
        return {}
    return json.loads("\n".join(lines))


def _read_json_file(path: str) -> dict[str, Any]:
    """Read JSON file, stripping comment lines (#, //) and empty lines."""
    with open(path, encoding="utf-8") as f:
        return _parse_json_text(f.read())
