"""Editor-based create/clone workflow."""

import json
import os
import shlex
import subprocess
import sys
import tempfile
from typing import Any


def _editor() -> list[str]:
    editor = os.environ.get("EDITOR", os.environ.get("VISUAL", "vi"))
    return shlex.split(editor)


class EditorAbort(Exception):
    pass


def edit_json(data: dict[str, Any], hint: str = "") -> dict[str, Any]:
    """Open $EDITOR with JSON data, return parsed result.

    The file starts with JSONC comment lines explaining the fields.
    Lines starting with // or # are stripped before parsing JSON.
    Raises EditorAbort on cancel/unchanged, SystemExit on editor not found.
    """
    comment = f"// {hint}\n// Edit JSON below. Comment lines ignored.\n// Save + close to continue; leave unchanged to abort.\n\n"
    raw = comment + json.dumps(data, indent=2, ensure_ascii=False, default=str)

    with tempfile.NamedTemporaryFile(mode="w", suffix=".jsonc", delete=False, encoding="utf-8") as f:
        f.write(raw)
        tmp_path = f.name

    mtime_before = os.path.getmtime(tmp_path)
    try:
        subprocess.run([*_editor(), tmp_path], check=False)
    except FileNotFoundError:
        os.unlink(tmp_path)
        print(f"error: editor not found: {_editor()[0]}; set $EDITOR", file=sys.stderr)
        raise SystemExit(1)

    mtime_after = os.path.getmtime(tmp_path)
    if mtime_after == mtime_before:
        os.unlink(tmp_path)
        raise EditorAbort("aborted: file unchanged")

    try:
        edited = _read_json_file(tmp_path)
    finally:
        os.unlink(tmp_path)

    if not edited:
        raise EditorAbort("aborted: empty file")
    return edited


def _read_json_file(path: str) -> dict[str, Any]:
    """Read JSON file, stripping comment lines (#) and empty lines."""
    lines = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            stripped = line.strip()
            if not stripped or stripped.startswith("#") or stripped.startswith("//"):
                continue
            lines.append(line.rstrip("\n"))
    if not lines:
        return {}
    return json.loads("\n".join(lines))
