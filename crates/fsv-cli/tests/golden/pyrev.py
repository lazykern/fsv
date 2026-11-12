"""The Python commit the dumps and goldens come from: `{"commit", "dirty"}`."""
import os
import pathlib
import subprocess


def python_rev(repo=None):
    """`repo` defaults to the imported `fsv` package. A `FSV_PY_SRC` tree (a
    `git archive`, no history) is named by `FSV_PY_COMMIT` instead."""
    if os.environ.get("FSV_PY_SRC"):
        commit = os.environ.get("FSV_PY_COMMIT")
        if not commit:
            raise SystemExit("FSV_PY_SRC is set: name its commit in FSV_PY_COMMIT")
        return {"commit": commit, "dirty": False}
    if repo is None:
        import fsv

        repo = pathlib.Path(fsv.__file__).resolve().parent

    def git(*args):
        return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True,
                              check=True).stdout.strip()

    return {"commit": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain"))}


def stamp(rev):
    """The header line emitted test files carry."""
    return f"Python commit: {rev['commit']}{' (dirty)' if rev['dirty'] else ''}"
