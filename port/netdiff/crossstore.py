"""A session written by one binary must be read by the other (file and argon stores).

    python3 port/netdiff/crossstore.py
"""
import json
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import netdiff  # noqa: E402
import netsrv  # noqa: E402
from netdiff import ME, json_resp, run_impl, write_home  # noqa: E402

GOOD = "_x_m=a; _x_d=b; _x_w=c; fw-session-id=d"


def handler(req, n):
    return json_resp(ME) if req.path == "/api/v2/agents/me" else json_resp({"changes": []})


def main():
    bad = 0
    for store, stdin in (("file", ""), ("argon", "pw1\npw1\n")):
        for writer, reader in (("py", "rust"), ("rust", "py")):
            tmp = Path(tempfile.mkdtemp(prefix="cross-"))
            try:
                (tmp / "tls").mkdir()
                srv = netsrv.Server(tmp / "tls", handler)
                home = tmp / "home"
                home.mkdir()
                write_home(home, session=False)
                w = run_impl(writer, ["auth", "login", "--store", store, "--header", GOOD], srv, home, stdin)
                read_stdin = "pw1\n" if store == "argon" else ""
                outs = {}
                for impl in (reader, writer):
                    outs[impl] = run_impl(impl, ["auth", "status", "-v"], srv, home, read_stdin)
                    outs[impl]["out"] = outs[impl]["out"].replace(" ", "")
                a, b = outs[reader], outs[writer]
                strip = lambda t: "\n".join(x for x in t.splitlines() if "GetPass" not in x and "fallback_getpass" not in x)
                ok = w["code"] == 0 and a["code"] == b["code"] == 0 and a["out"].split("sessionage")[0] == b["out"].split("sessionage")[0]
                ok = ok and netdiff.short(a["err"]) == netdiff.short(b["err"])
                bad += not ok
                print(f"[{'same' if ok else 'DIFF'}] {store}: written by {writer}, read by {reader} (login={w['code']} read={a['code']} control={b['code']})")
                if not ok:
                    print("  login err:", w["err"][-300:], "\n  reader:", a, "\n  writer:", b)
                srv.close()
            finally:
                shutil.rmtree(tmp, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
