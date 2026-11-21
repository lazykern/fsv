import base64
import gzip
import json
import socket
import ssl
import subprocess
import tempfile
import threading
import time
from pathlib import Path

HOSTS = ["acme.freshservice.com", "sub.acme.freshservice.com", "freshservice.com", "other.example.com", "idp.myfreshworks.com", "files.example.net"]


def make_cert(directory: Path) -> tuple[Path, Path, Path]:
    ca, cert, key = directory / "ca.pem", directory / "cert.pem", directory / "key.pem"
    if cert.exists():
        return ca, cert, key
    ca_key, csr, ext = directory / "ca.key", directory / "leaf.csr", directory / "ext.cnf"
    run = lambda *a: subprocess.run(a, check=True, capture_output=True)
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(ca_key), "-out", str(ca),
        "-days", "30", "-subj", "/CN=netdiff test CA")
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key), "-out", str(csr),
        "-subj", "/CN=acme.freshservice.com")
    ext.write_text("basicConstraints=CA:FALSE\nsubjectAltName=" + ",".join(f"DNS:{h}" for h in HOSTS) + "\n")
    run("openssl", "x509", "-req", "-in", str(csr), "-CA", str(ca), "-CAkey", str(ca_key), "-CAcreateserial",
        "-out", str(cert), "-days", "30", "-extfile", str(ext))
    return ca, cert, key


class Req:
    def __init__(self, method, target, headers, body, scheme, conn):
        self.method = method
        self.target = target
        self.headers = headers
        self.body = body
        self.scheme = scheme
        self.conn = conn
        self.path, _, self.query = target.partition("?")

    def header(self, name):
        for k, v in self.headers:
            if k.lower() == name.lower():
                return v
        return None

    @property
    def host(self):
        return self.header("host")


class Resp:
    def __init__(self, status=200, headers=None, body=b"", mode="cl", reason="OK"):
        self.status = status
        self.headers = list(headers or [])
        self.body = body.encode() if isinstance(body, str) else body
        self.mode = mode
        self.reason = reason


def json_resp(obj, status=200, headers=None, **kw):
    h = [("Content-Type", "application/json")] + list(headers or [])
    return Resp(status, h, json.dumps(obj), **kw)


def html_resp(text, status=200, headers=None, **kw):
    h = [("Content-Type", "text/html; charset=utf-8")] + list(headers or [])
    return Resp(status, h, text, **kw)


def redirect(status, location, headers=None):
    return Resp(status, [("Location", location)] + list(headers or []), b"", reason="Redirect")


class Drop:
    pass


class Hang:
    def __init__(self, seconds=120):
        self.seconds = seconds


class Reset:
    pass


class Raw:
    def __init__(self, data: bytes, then_close=True):
        self.data = data


def _read_head(f):
    line = f.readline(65537)
    if not line:
        return None
    parts = line.decode("latin-1").rstrip("\r\n").split(" ")
    headers = []
    while True:
        h = f.readline(65537)
        if h in (b"\r\n", b"\n", b""):
            break
        k, _, v = h.decode("latin-1").rstrip("\r\n").partition(":")
        headers.append((k, v.strip()))
    return parts, headers


def _read_body(f, headers):
    te = next((v for k, v in headers if k.lower() == "transfer-encoding"), "")
    cl = next((v for k, v in headers if k.lower() == "content-length"), None)
    if "chunked" in te.lower():
        out = b""
        while True:
            size = int(f.readline().split(b";")[0].strip() or b"0", 16)
            if size == 0:
                while f.readline() not in (b"\r\n", b"\n", b""):
                    pass
                return out
            out += f.read(size)
            f.readline()
    if cl:
        return f.read(int(cl))
    return b""


class Server:
    def __init__(self, directory: Path, handler):
        self.handler = handler
        self.log = []
        self.counts = {}
        self.proxy_log = []
        self.lock = threading.Lock()
        self.t0 = None
        self.conn_seq = 0
        ca, cert, key = make_cert(directory)
        self.ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.ctx.load_cert_chain(cert, key)
        self.cert = ca
        self.tls = socket.socket()
        self.tls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.tls.bind(("127.0.0.1", 0))
        self.tls.listen(64)
        self.tls_port = self.tls.getsockname()[1]
        self.proxy = socket.socket()
        self.proxy.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.proxy.bind(("127.0.0.1", 0))
        self.proxy.listen(64)
        self.proxy_port = self.proxy.getsockname()[1]
        self.stop = False
        self.conns = []
        for target in (self._accept_tls, self._accept_proxy):
            threading.Thread(target=target, daemon=True).start()

    @property
    def proxy_url(self):
        return f"http://127.0.0.1:{self.proxy_port}"

    def close(self):
        self.stop = True
        for s in (self.tls, self.proxy, *self.conns):
            try:
                s.close()
            except OSError:
                pass

    def _accept_tls(self):
        while not self.stop:
            try:
                sock, _ = self.tls.accept()
            except OSError:
                return
            self.conns.append(sock)
            threading.Thread(target=self._serve_tls, args=(sock,), daemon=True).start()

    def _serve_tls(self, sock):
        try:
            sock = self.ctx.wrap_socket(sock, server_side=True)
        except (ssl.SSLError, OSError) as e:
            with self.lock:
                self.proxy_log.append(f"tls handshake failed: {e}")
            sock.close()
            return
        with self.lock:
            self.conn_seq += 1
            conn = self.conn_seq
        self._serve(sock, "https", conn)

    def _accept_proxy(self):
        while not self.stop:
            try:
                sock, _ = self.proxy.accept()
            except OSError:
                return
            self.conns.append(sock)
            threading.Thread(target=self._serve_proxy, args=(sock,), daemon=True).start()

    def _serve_proxy(self, sock):
        f = sock.makefile("rb")
        try:
            head = _read_head(f)
            if not head:
                return
            parts, headers = head
            if parts[0] == "CONNECT":
                with self.lock:
                    self.proxy_log.append(f"CONNECT {parts[1]}")
                    mode = getattr(self, "connect_mode", None)
                if mode == "refuse":
                    sock.sendall(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                    return
                if mode == "hang":
                    time.sleep(120)
                    return
                if mode == "close":
                    return
                sock.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
                up = socket.create_connection(("127.0.0.1", self.tls_port))
                self.conns.append(up)

                def pump(a, b):
                    try:
                        while True:
                            d = a.recv(65536)
                            if not d:
                                break
                            b.sendall(d)
                    except OSError:
                        pass
                    for s in (a, b):
                        try:
                            s.shutdown(socket.SHUT_RDWR)
                        except OSError:
                            pass

                threading.Thread(target=pump, args=(up, sock), daemon=True).start()
                pump(sock, up)
                return
            with self.lock:
                self.conn_seq += 1
                conn = self.conn_seq
            self._serve(sock, "http", conn, first=(parts, headers, f))
        except OSError:
            pass
        finally:
            try:
                sock.close()
            except OSError:
                pass

    def _serve(self, sock, scheme, conn, first=None):
        f = first[2] if first else sock.makefile("rb")
        pending = first
        try:
            while True:
                head = (pending[0], pending[1]) if pending else _read_head(f)
                pending = None
                if not head:
                    return
                parts, headers = head
                target = parts[1]
                if "://" in target:
                    target = "/" + target.split("://", 1)[1].partition("/")[2]
                body = _read_body(f, headers)
                req = Req(parts[0], target, headers, body, scheme, conn)
                if self.t0 is None:
                    self.t0 = time.time()
                with self.lock:
                    key = (req.method, req.path)
                    self.counts[key] = self.counts.get(key, 0) + 1
                    n = self.counts[key]
                    self.log.append({
                        "t": round(time.time() - self.t0, 2), "conn": conn, "scheme": scheme, "method": req.method,
                        "target": target, "headers": [[k, v] for k, v in headers],
                        "body": base64.b64encode(body).decode(),
                    })
                action = self.handler(req, n)
                if isinstance(action, Drop):
                    return
                if isinstance(action, Reset):
                    import struct
                    sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
                    self.dirty_close = True
                    sock.close()
                    return
                if isinstance(action, Hang):
                    time.sleep(action.seconds)
                    return
                if isinstance(action, Raw):
                    sock.sendall(action.data)
                    return
                keep = self._write(sock, action)
                if not keep:
                    return
        except (OSError, ValueError, ssl.SSLError):
            pass
        finally:
            try:
                if isinstance(sock, ssl.SSLSocket) and not getattr(self, "dirty_close", False):
                    sock = sock.unwrap()
                sock.close()
            except (OSError, ssl.SSLError, ValueError):
                pass

    def _write(self, sock, r: Resp):
        body = r.body
        headers = list(r.headers)
        names = {k.lower() for k, _ in headers}
        if r.mode == "gzip":
            body = gzip.compress(body)
            headers.append(("Content-Encoding", "gzip"))
            r = Resp(r.status, headers, body, "cl", r.reason)
        head = f"HTTP/1.1 {r.status} {r.reason}\r\n"
        keep = True
        if r.mode in ("cl", "gzip"):
            if "content-length" not in names:
                headers.append(("Content-Length", str(len(body))))
            out = body
        elif r.mode == "chunked":
            headers.append(("Transfer-Encoding", "chunked"))
            out = b""
            for i in range(0, len(body), 7):
                chunk = body[i:i + 7]
                out += f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n"
            out += b"0\r\n\r\n"
        elif r.mode == "close":
            headers.append(("Connection", "close"))
            out = body
            keep = False
        elif r.mode == "truncate":
            headers.append(("Content-Length", str(len(body) + 50)))
            out = body
            keep = False
        elif r.mode == "badchunk":
            headers.append(("Transfer-Encoding", "chunked"))
            out = f"{len(body):x}\r\n".encode() + body
            keep = False
        else:
            raise ValueError(r.mode)
        for k, v in headers:
            head += f"{k}: {v}\r\n"
        sock.sendall(head.encode("latin-1") + b"\r\n" + out)
        return keep
