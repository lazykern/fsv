"""Golden values for fsv.session helpers: base64, passphrase prompts, hidden
input over a pty, and the CDP cookie read against a local fake browser.

Usage (from the Python worktree):
    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/port/golden/gen_fsv_core_session.py \
        ../fsv-rust/crates/fsv-core/tests/fixtures/session_expected.json
"""
import base64
import hashlib
import json
import os
import pty
import socket
import sys
import threading
from pathlib import Path

from fsv import session
from fsv.errors import SessionError
from provenance import stamped

OUT = Path(sys.argv[1])
out = {}

# ---------- _b64e / _b64d ----------
b64d_inputs = ["", "YWJj", "YW Jj", "YWJj\n", "YWI=", "YWI", "YWJj=YWJj", "YQ==YQ==", "Y",
               "****", "YW*Jj", "YWI==", "YQ", "=YWJj", "YW=Jj", "AP9oaQ==", "Y\nW\nJ\nj", "YWJjZA", "YWJjZ"]
rows = []
for s in b64d_inputs:
    try:
        rows.append([s, list(session._b64d(s))])
    except Exception as exc:
        rows.append([s, f"{type(exc).__name__}: {exc}"])
out["b64d"] = rows
out["b64e"] = [[list(b), session._b64e(b)] for b in (b"", b"\x00\xffhi", bytes(range(0, 256, 17)), b"\xff\xfe")]

# ---------- _passphrase ----------
rows = []
for confirm, answers in [(False, ["pw"]), (False, [""]), (True, ["pw", "pw"]), (True, ["pw", "px"]),
                         (True, ["", "pw"])]:
    prompts = []
    it = iter(answers)

    def fake(prompt="Password: "):
        prompts.append(prompt)
        return next(it)

    session.getpass.getpass = fake
    try:
        got = session._passphrase(confirm=confirm)
    except SessionError as exc:
        got = {"error": str(exc)}
    rows.append({"confirm": confirm, "answers": answers, "prompts": prompts, "out": got})
out["passphrase"] = rows

# ---------- _read_secret over a pty ----------


def read_secret(data: bytes, tty: bool):
    if tty:
        # setcbreak flushes pending input and waits for echoed output to
        # drain, so write only after it has switched echo off.
        master, slave = pty.openpty()
        threading.Timer(0.3, os.write, (master, data)).start()
        stdin = os.fdopen(slave, "r")
    else:
        r, w = os.pipe()
        os.write(w, data)
        os.close(w)
        stdin = os.fdopen(r, "r")
    old_in, old_err = sys.stdin, sys.stderr
    err = open(os.devnull, "w")
    sys.stdin, sys.stderr = stdin, err
    try:
        got = session._read_secret("> ")
    except KeyboardInterrupt:
        got = {"error": "KeyboardInterrupt"}
    finally:
        sys.stdin, sys.stderr = old_in, old_err
        stdin.close()
        err.close()
        if tty:
            os.close(master)
    return got


# Ctrl-C is left out: cbreak keeps ISIG, so the pty turns it into a signal.
secret_inputs = ["abc\n", "a\x7fb\n", "\x7fx\n", "l1\nl2\n", "l1\nl2\x04", "ab\x04cd\n",
                 "caf\xc3\xa9\n", "\n", "a\tb\n", "a\rb\r"]
out["read_secret_tty"] = [[s, read_secret(s.encode("latin-1"), True)] for s in secret_inputs]
out["read_secret_pipe"] = [[s, read_secret(s.encode("utf-8"), False)]
                           for s in ["abc\n", "abc\n\n", "a\r\n", "l1\nl2\n", ""]]

# ---------- _direct_browser_cookies against a fake DevTools endpoint ----------
GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def frame(opcode: int, payload: bytes) -> bytes:
    head = bytes([0x80 | opcode])
    n = len(payload)
    if n < 126:
        head += bytes([n])
    elif n <= 0xFFFF:
        head += bytes([126]) + n.to_bytes(2, "big")
    else:
        head += bytes([127]) + n.to_bytes(8, "big")
    return head + payload


def read_exact(conn, n):
    buf = b""
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk:
            raise EOFError
        buf += chunk
    return buf


def recv_text(conn) -> str:
    b1, b2 = read_exact(conn, 2)
    n = b2 & 0x7F
    if n == 126:
        n = int.from_bytes(read_exact(conn, 2), "big")
    elif n == 127:
        n = int.from_bytes(read_exact(conn, 8), "big")
    mask = read_exact(conn, 4) if b2 & 0x80 else b""
    data = read_exact(conn, n)
    if mask:
        data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
    return data.decode()


def serve(listener, scenario, received):
    port = listener.getsockname()[1]
    targets = [
        {"type": "background_page", "url": "https://acme.freshservice.com/", "webSocketDebuggerUrl": f"ws://127.0.0.1:{port}/bg"},
        {"type": "page", "url": "https://other.example/", "webSocketDebuggerUrl": f"ws://127.0.0.1:{port}/other"},
        {"type": "page", "url": "https://acme.freshservice.com/a/tickets", "webSocketDebuggerUrl": f"ws://127.0.0.1:{port}/devtools/page/ACME"},
    ]
    replies = list(scenario)
    while True:
        try:
            conn, _ = listener.accept()
        except OSError:
            return
        with conn:
            req = b""
            while b"\r\n\r\n" not in req:
                req += conn.recv(4096)
            line = req.split(b"\r\n", 1)[0].decode()
            path = line.split(" ")[1]
            if path.startswith("/json"):
                body = json.dumps(targets).encode()
                conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: "
                             + str(len(body)).encode() + b"\r\nConnection: close\r\n\r\n" + body)
                continue
            key = [h.split(b":", 1)[1].strip() for h in req.split(b"\r\n") if h.lower().startswith(b"sec-websocket-key:")][0]
            accept = base64.b64encode(hashlib.sha1(key + GUID.encode()).digest())
            conn.sendall(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                         b"Sec-WebSocket-Accept: " + accept + b"\r\n\r\n")
            received.append({"path": path, "text": recv_text(conn)})
            for kind, payload in replies.pop(0):
                if kind == "text":
                    conn.sendall(frame(1, json.dumps(payload).encode()))
                elif kind == "ping":
                    conn.sendall(frame(9, b"hi"))
                elif kind == "close":
                    conn.sendall(frame(8, b""))


COOKIES = {"cookies": [{"name": "_x_m", "value": "m"}, {"name": "", "value": "e"}, {"name": "n", "value": 3}, "x"]}
SCENARIOS = {
    "get_cookies": [[["ping", None], ["text", {"method": "Network.event"}], ["text", {"id": 2, "result": {}}],
                     ["text", {"id": 1, "result": COOKIES}]]],
    "fallback_all_cookies": [[["text", {"id": 1, "error": {"code": -32601, "message": "nope"}}]],
                             [["text", {"id": 1, "result": {"cookies": [{"name": "a", "value": "1"}]}}]]],
    "both_error": [[["text", {"id": 1, "error": {"code": -32601, "message": "nope"}}]],
                   [["text", {"id": 1, "error": "bad"}]]],
    "result_not_object": [[["text", {"id": 1, "result": [1]}]]],
    "closed": [[["close", None]], [["close", None]]],
}
rows = {}
for name, scenario in SCENARIOS.items():
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    received = []
    t = threading.Thread(target=serve, args=(listener, scenario, received), daemon=True)
    t.start()
    try:
        got = session._direct_browser_cookies(listener.getsockname()[1], "acme.freshservice.com")
    except SessionError as exc:
        got = {"error": str(exc)}
    listener.close()
    rows[name] = {"scenario": scenario, "out": got, "received": received}
out["cdp"] = rows

OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(stamped(out), indent=1, ensure_ascii=False) + "\n")
