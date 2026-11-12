"""The Python commit a golden fixture comes from: `{"commit", "dirty"}`.

Every generator records it once per fixture:

- a top-level JSON object gets it as its first key, `"_python"` (`stamped`);
- any other JSON or text fixture gets a sibling `<stem>.provenance.json`
  holding `{"_python": ...}` (`write_sidecar`);
- generated Rust source gets a `Python commit: ...` header line (`header`).

The commit is that of the imported `fsv` package's checkout. To generate at a
pinned commit without touching the live worktree, export it and run there:

    git -C <fsv> archive <sha> | tar -x -C /tmp/fsv-py-<sha>
    cd /tmp/fsv-py-<sha> && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-<sha> FSV_PY_COMMIT=$(git -C <fsv> rev-parse <sha>) \\
        uv run --offline python <generator> ...

A `git archive` tree has no history, so `FSV_PY_COMMIT` names it.
"""
import json
import os
import pathlib
import subprocess


def py_root():
    """The Python checkout the imported `fsv` package lives in (`src/fsv`)."""
    import fsv

    root = pathlib.Path(fsv.__file__).resolve().parents[2]
    src = os.environ.get("FSV_PY_SRC")
    if src and root != pathlib.Path(src).resolve():
        raise SystemExit(f"FSV_PY_SRC is {src} but fsv is imported from {root}")
    return root


def python_rev():
    root = py_root()
    if os.environ.get("FSV_PY_SRC"):
        commit = os.environ.get("FSV_PY_COMMIT")
        if not commit:
            raise SystemExit("FSV_PY_SRC is set: name its commit in FSV_PY_COMMIT")
        return {"commit": commit, "dirty": False}

    def git(*args):
        return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True,
                              check=True).stdout.strip()

    return {"commit": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain"))}


def stamped(doc):
    """`doc` (a dict) with `"_python"` as its first key."""
    return {"_python": python_rev(), **{k: v for k, v in doc.items() if k != "_python"}}


def sidecar(path):
    path = pathlib.Path(path)
    return path.with_name(path.name.split(".")[0] + ".provenance.json")


def write_sidecar(path):
    """Record the commit for a fixture that is not a JSON object."""
    sidecar(path).write_text(json.dumps({"_python": python_rev()}, indent=1) + "\n")


def header():
    rev = python_rev()
    return f"Python commit: {rev['commit']}{' (dirty)' if rev['dirty'] else ''}"
