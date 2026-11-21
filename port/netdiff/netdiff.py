"""Run the Python and Rust fsv against one scripted local HTTPS server and compare.

    python3 port/netdiff/netdiff.py [-k substring] [-v]

Both binaries reach the server through an HTTP CONNECT proxy (HTTPS_PROXY) that
tunnels every host to one TLS listener, and trust its certificate through
SSL_CERT_FILE. Compared per scenario: exit code, stdout, stderr and the exact
requests the server received.
"""
import argparse
import functools
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import netsrv  # noqa: E402
from netsrv import Drop, Hang, Raw, Resp, html_resp, json_resp, redirect  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
PY_REPO = Path(os.environ.get("FSV_PY_REPO", "/Users/phusit.somb/lab/work/fsv"))
RUST_BIN = ROOT / "target" / "debug" / "fsv"
PY_CMD = [str(PY_REPO / ".venv/bin/python"), "-c", "import sys; sys.argv[0]='fsv'; import fsv; fsv.main()"]
COOKIES = {"_x_m": "m1", "_x_d": "d1", "_x_w": "w1", "fw-session-id": "fw1"}
DOMAIN = "acme.freshservice.com"
ME = {"agent": {"first_name": "Ann", "last_name": "Lee", "email": "ann@example.com"}}
APP_HTML = '<html><head><meta name="csrf-token" content="tok123"></head></html>'


def write_home(home: Path, cookies=COOKIES, store=None, session=True):
    cfg = home / "config"
    cfg.mkdir(parents=True)
    (cfg / "config.json").write_text(json.dumps({"domain": DOMAIN}))
    if session:
        (cfg / "session.json").write_text(json.dumps({"saved_at": 1.0, "domain": DOMAIN, "cookies": cookies}))
    if store:
        (cfg / "store").write_text(store)
    (home / "cache").mkdir()
    return cfg


def run_impl(impl, args, srv, home, stdin=None, env_extra=None, timeout=200):
    cmd = (PY_CMD if impl == "py" else [str(RUST_BIN)]) + args
    env = {
        "PATH": os.environ["PATH"],
        "HOME": str(home),
        "FSV_CONFIG_DIR": str(home / "config"),
        "FSV_CACHE_DIR": str(home / "cache"),
        "FSV_NO_KEYCHAIN": "1",
        "HTTPS_PROXY": srv.proxy_url,
        "HTTP_PROXY": srv.proxy_url,
        "NO_PROXY": "",
        "SSL_CERT_FILE": str(srv.cert),
        "NO_COLOR": "1",
        "FSV_ALLOW_TENANT": DOMAIN,
        "COLUMNS": "200",
        "TERM": "dumb",
        "PYTHONDONTWRITEBYTECODE": "1",
    }
    env.update(env_extra or {})
    t0 = time.time()
    try:
        p = subprocess.run(cmd, input=(stdin or "").encode(), capture_output=True, env=env, timeout=timeout, start_new_session=True,
                           cwd=str(home))
        code, out, err = p.returncode, p.stdout.decode(errors="replace"), p.stderr.decode(errors="replace")
    except subprocess.TimeoutExpired:
        code, out, err = "TIMEOUT", "", ""
    return {"code": code, "out": out, "err": err, "secs": round(time.time() - t0, 1)}


SCENARIOS = []
print = functools.partial(print, flush=True)


def scenario(name, args, handler, stdin=None, session=True, store=None, connect_mode=None, env_extra=None,
             after=None, cookies=COOKIES, slow=False, mask=False):
    SCENARIOS.append(dict(name=name, args=args, handler=handler, stdin=stdin, session=session, store=store,
                          connect_mode=connect_mode, env_extra=env_extra, after=after, cookies=cookies, slow=slow, mask=mask))


def norm_headers(headers):
    skip = {"host"}
    return sorted((k.lower(), v) for k, v in headers if k.lower() not in skip)


def summarize(srv):
    out = []
    for e in srv.log:
        out.append({
            "t": e["t"], "scheme": e["scheme"], "method": e["method"], "target": e["target"],
            "host": next((v for k, v in e["headers"] if k.lower() == "host"), None),
            "headers": norm_headers(e["headers"]),
            "body": e["body"], "conn": e["conn"],
        })
    return out


def run_scenario(sc, verbose):
    try:
        return _run_scenario(sc, verbose)
    except Exception as e:
        return {"error": repr(e)}


def _run_scenario(sc, verbose):
    results = {}
    for impl in ("py", "rust"):
        tmp = Path(tempfile.mkdtemp(prefix="netdiff-"))
        try:
            (tmp / "tls").mkdir()
            srv = netsrv.Server(tmp / "tls", sc["handler"])
            srv.connect_mode = sc["connect_mode"]
            home = tmp / "home"
            home.mkdir()
            write_home(home, sc["cookies"], sc["store"], sc["session"])
            r = run_impl(impl, sc["args"], srv, home, sc["stdin"], sc["env_extra"])
            time.sleep(0.2)
            r["reqs"] = summarize(srv)
            r["proxy"] = [x for x in srv.proxy_log if not x.startswith("CONNECT")]
            if any("handshake" in x for x in r["proxy"]):
                r["proxy"] = ["tls handshake failed"]
            if sc["after"]:
                r["after"] = sc["after"](home)
            srv.close()
            results[impl] = r
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
    return results


LIB_MSG = re.compile(r"(-> (?:ProxyError|DecodingError|ConnectError|NetworkError|ReadError|WriteError|ReadTimeout|ConnectTimeout): ).*|(-> RemoteProtocolError: )(?!Server disconnected|peer closed).*")


def short(text, mask_final=False):
    text = re.sub(r"\b\d+ms\b", "Nms", text)
    text = re.sub(r"(?m)^.*GetPassWarning.*\n.*fallback_getpass.*\n", "", text)
    text = "\n".join(LIB_MSG.sub(lambda m: (m.group(1) or m.group(2)) + "<lib msg>", ln) for ln in text.split("\n"))
    if mask_final:
        lines = text.rstrip("\n").split("\n")
        if lines and lines[-1].startswith("error: "):
            lines[-1] = "error: <lib msg>"
        text = "\n".join(lines) + "\n"
    if "Traceback (most recent call last)" in text:
        keep = []
        for ln in text.splitlines():
            if ln.startswith(("╭", "│")):
                break
            keep.append(ln)
        last = [ln for ln in text.splitlines() if ln and not ln.startswith(("│", "╭", "╰"))][-1]
        exc = last.split(": ", 1)[1] if ": " in last else last
        keep.append(f"error: {exc[:300]}")
        text = "\n".join(keep) + "\n"
        if mask_final:
            text = text.rsplit("error: ", 1)[0] + "error: <lib msg>\n"
        return text
    return text[:900]


def compare(name, res, verbose, mask_final=False):
    if "error" in res:
        print(f"[ERROR] {name} {res['error']}")
        return False
    res = {k: {**v, "err": short(v["err"], mask_final), "out": v["out"][:600]} for k, v in res.items()}
    py, rs = res["py"], res["rust"]
    diffs = []
    for key in ("code", "out", "err", "after"):
        if py.get(key) != rs.get(key):
            diffs.append(f"  {key}:\n    py:   {py.get(key)!r}\n    rust: {rs.get(key)!r}")
    pr, rr = py["reqs"], rs["reqs"]
    if len(pr) != len(rr):
        diffs.append(f"  request count: py={len(pr)} rust={len(rr)}")
    for i, (a, b) in enumerate(zip(pr, rr)):
        for k in ("scheme", "method", "target", "host", "body"):
            if a[k] != b[k]:
                diffs.append(f"  req{i}.{k}:\n    py:   {a[k]!r}\n    rust: {b[k]!r}")
        if a["headers"] != b["headers"]:
            sa, sb = set(map(tuple, a["headers"])), set(map(tuple, b["headers"]))
            diffs.append(f"  req{i}.headers only-py={sorted(sa - sb)} only-rust={sorted(sb - sa)}")
        if "retry" in name and abs(a["t"] - b["t"]) > 0.6:
            diffs.append(f"  req{i}.t: py={a['t']} rust={b['t']}")
        if a["conn"] != b["conn"]:
            diffs.append(f"  req{i}.conn: py={a['conn']} rust={b['conn']} (connection reuse)")
    status = "DIFF" if diffs else "same"
    print(f"[{status}] {name}  py={py['code']}/{py['secs']}s rust={rs['code']}/{rs['secs']}s reqs={len(pr)}/{len(rr)}")
    if diffs or verbose:
        for d in diffs:
            print(d)
        if verbose:
            for impl in ("py", "rust"):
                print(f"  {impl} stdout={res[impl]['out']!r} stderr={res[impl]['err']!r}")
    return not diffs


def main():
    import scenarios  # noqa: F401

    ap = argparse.ArgumentParser()
    ap.add_argument("-k", default="")
    ap.add_argument("-v", action="store_true")
    ap.add_argument("--slow", action="store_true", help="include the 30 s timeout scenarios")
    ap.add_argument("-j", type=int, default=6)
    ap.add_argument("--save", default="/tmp/netdiff-results.json")
    ap.add_argument("--replay", help="compare a saved results file instead of running")
    a = ap.parse_args()
    from concurrent.futures import ThreadPoolExecutor

    chosen = [sc for sc in SCENARIOS if a.k in sc["name"] and (a.slow or not sc["slow"])]
    if a.replay:
        saved = json.load(open(a.replay))
        chosen = [sc for sc in chosen if sc["name"] in saved]
        results = [saved[sc["name"]] for sc in chosen]
    else:
        with ThreadPoolExecutor(a.j) as pool:
            results = list(pool.map(lambda sc: run_scenario(sc, a.v), chosen))
        json.dump({sc["name"]: r for sc, r in zip(chosen, results)}, open(a.save, "w"))
    bad = 0
    for sc, res in zip(chosen, results):
        if not compare(sc["name"], res, a.v, sc["mask"]):
            bad += 1
    print(f"{bad} scenario(s) differ")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    sys.modules["netdiff"] = sys.modules["__main__"]
    main()
