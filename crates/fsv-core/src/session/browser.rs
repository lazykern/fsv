//! Browser login: launch an isolated Chromium-family browser and read its
//! cookies over the DevTools protocol.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Cookies, validate};
use crate::config;
use crate::error::{Error, Result};

/// Browser executable: `FSV_BROWSER`, Chrome/Chromium/Edge on PATH, then the
/// usual macOS and Windows install locations.
pub fn browser_executable() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(env) = std::env::var_os("FSV_BROWSER").filter(|v| !v.is_empty()) {
        candidates.push(PathBuf::from(env));
    }
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "msedge",
    ] {
        if let Ok(p) = which::which(name) {
            candidates.push(p);
        }
    }
    for p in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    ] {
        candidates.push(PathBuf::from(p));
    }
    let windows = [
        ("ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
        ("ProgramFiles(x86)", r"Google\Chrome\Application\chrome.exe"),
        ("LOCALAPPDATA", r"Google\Chrome\Application\chrome.exe"),
        (
            "ProgramFiles(x86)",
            r"Microsoft\Edge\Application\msedge.exe",
        ),
        ("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
        (
            "ProgramFiles",
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
        ),
        (
            "ProgramFiles(x86)",
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
        ),
        (
            "LOCALAPPDATA",
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
        ),
    ];
    for (var, rel) in windows {
        if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()) {
            candidates.push(Path::new(&base).join(rel));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Side effects of a browser login, injectable for tests.
pub trait BrowserOps {
    /// Start the browser with `args`.
    fn launch(&mut self, exe: &Path, args: &[String]) -> Result<()>;
    /// Wait for the DevTools port the browser writes into the profile dir.
    fn devtools_port(&mut self, profile_dir: &Path) -> Result<u16>;
    /// Current cookies for `domain`.
    fn cookies(&mut self, port: u16, domain: &str) -> Result<Cookies>;
    fn sleep(&mut self, d: Duration);
    /// Terminate the browser (kill after 5s).
    fn shutdown(&mut self);
}

/// Real browser process + CDP over a websocket.
#[derive(Default)]
pub struct SystemBrowser {
    child: Option<Child>,
}

impl BrowserOps for SystemBrowser {
    fn launch(&mut self, exe: &Path, args: &[String]) -> Result<()> {
        self.child = Some(Command::new(exe).args(args).spawn()?);
        Ok(())
    }

    fn devtools_port(&mut self, profile_dir: &Path) -> Result<u16> {
        let port_file = profile_dir.join("DevToolsActivePort");
        for _ in 0..100 {
            if let Some(child) = self.child.as_mut()
                && matches!(child.try_wait(), Ok(Some(_)))
            {
                return Err(Error::session("browser exited before DevTools started"));
            }
            if let Ok(text) = std::fs::read_to_string(&port_file) {
                // The file can be observed half-written; retry until the first line parses.
                if let Some(port) = text
                    .lines()
                    .next()
                    .and_then(|l| l.trim().parse::<u16>().ok())
                {
                    return Ok(port);
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(Error::session("browser DevTools port did not start"))
    }

    fn cookies(&mut self, port: u16, domain: &str) -> Result<Cookies> {
        direct_browser_cookies(port, domain)
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }

    fn shutdown(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        #[cfg(unix)]
        {
            // SAFETY: plain signal to our own child process.
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Port of fsv.session._http_json.
fn http_json(url: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| Error::other(e.to_string()))?;
    let r = client
        .get(url)
        .send()
        .map_err(|e| Error::other(format!("<urlopen error {e}>")))?;
    let text = r.text().map_err(|e| Error::other(e.to_string()))?;
    Ok(serde_json::from_str(&text)?)
}

/// Port of fsv.session._page_ws_url.
fn page_ws_url(port: u16, domain: &str) -> Result<String> {
    page_target(
        &http_json(&format!("http://127.0.0.1:{port}/json/list"))?,
        domain,
    )
}

/// The DevTools websocket URL of the first page showing `domain`, else of the first page.
fn page_target(targets: &Value, domain: &str) -> Result<String> {
    let Value::Array(targets) = targets else {
        return Err(Error::session("browser DevTools target list was invalid"));
    };
    let pages: Vec<&Value> = targets
        .iter()
        .filter(|t| {
            t.get("type").and_then(Value::as_str) == Some("page")
                && t.get("webSocketDebuggerUrl")
                    .is_some_and(crate::value::truthy)
        })
        .collect();
    let target = pages
        .iter()
        .find(|t| {
            fsv_py::py_str(t.get("url").unwrap_or(&Value::String(String::new()))).contains(domain)
        })
        .or(pages.first())
        .ok_or_else(|| Error::session("browser page target not found"))?;
    Ok(fsv_py::py_str(&target["webSocketDebuggerUrl"]))
}

/// Port of fsv.session._cdp_call. tungstenite does the framing that
/// Python hand-rolls in `_ws_read_exact`, `_ws_send_text` and `_ws_recv_text`.
fn cdp_call(ws_url: &str, method: &str, params: Value) -> Result<Value> {
    let url = url::Url::parse(ws_url).map_err(|e| Error::session(e.to_string()))?;
    let host = url.host_str().unwrap_or("127.0.0.1").to_string();
    let port = url.port().unwrap_or(80);
    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| Error::session("browser DevTools websocket handshake failed"))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let (mut ws, _) = tungstenite::client(ws_url, stream)
        .map_err(|_| Error::session("browser DevTools websocket handshake failed"))?;
    let msg = fsv_py::Dumps::default().dump(&json!({"id": 1, "method": method, "params": params}));
    ws.send(tungstenite::Message::text(msg))
        .map_err(|_| Error::session("browser DevTools websocket closed"))?;
    loop {
        let frame = ws
            .read()
            .map_err(|_| Error::session("browser DevTools websocket closed"))?;
        let text = match frame {
            tungstenite::Message::Text(t) => t.to_string(),
            tungstenite::Message::Close(_) => {
                return Err(Error::session("browser DevTools websocket closed"));
            }
            _ => continue,
        };
        let msg: Value = serde_json::from_str(&text)?;
        if msg.get("id") != Some(&json!(1)) {
            continue;
        }
        if let Some(e) = msg.get("error") {
            return Err(Error::session(fsv_py::py_str(e)));
        }
        let _ = ws.close(None);
        return Ok(match msg.get("result") {
            Some(r @ Value::Object(_)) => r.clone(),
            _ => json!({}),
        });
    }
}

/// Port of fsv.session._direct_browser_cookies: cookies of the page target
/// via `Network.getCookies` (fallback `Network.getAllCookies`).
pub fn direct_browser_cookies(port: u16, domain: &str) -> Result<Cookies> {
    let ws_url = page_ws_url(port, domain)?;
    let data = match cdp_call(
        &ws_url,
        "Network.getCookies",
        json!({"urls": [format!("https://{domain}/")]}),
    ) {
        Ok(d) => d,
        Err(Error::Session(_)) => cdp_call(&ws_url, "Network.getAllCookies", json!({}))?,
        Err(e) => return Err(e),
    };
    cookies_from_cdp(&data)
}

/// `{name: value}` of the string-valued, named entries of a CDP cookie list.
fn cookies_from_cdp(data: &Value) -> Result<Cookies> {
    let Some(Value::Array(items)) = data.get("cookies") else {
        return Err(Error::session(
            "browser cookie output did not contain a cookie list",
        ));
    };
    let mut out = Cookies::new();
    for item in items {
        if let (Some(Value::String(name)), Some(Value::String(value))) =
            (item.get("name"), item.get("value"))
            && !name.is_empty()
        {
            out.insert(name.clone(), value.clone());
        }
    }
    Ok(out)
}

/// Open an isolated browser and poll its cookies until the session is complete.
pub fn login_browser() -> Result<Cookies> {
    config::require_domain()?;
    let exe = browser_executable().ok_or_else(|| {
        Error::session(
            "browser login requires Chrome, Chromium, Edge, or Brave; set FSV_BROWSER=/path/to/browser, or use `--header`",
        )
    })?;
    login_browser_with(&mut SystemBrowser::default(), &exe)
}

/// [`login_browser`] with injectable side effects.
pub fn login_browser_with(ops: &mut dyn BrowserOps, exe: &Path) -> Result<Cookies> {
    let domain = config::require_domain()?;
    let url = format!("https://{domain}/");
    let timeout: u64 = match std::env::var("FSV_BROWSER_LOGIN_TIMEOUT") {
        Ok(v) => crate::pystr::strip(&v).parse().map_err(|_| {
            Error::value(format!(
                "invalid literal for int() with base 10: {}",
                fsv_py::py_repr_str(&v)
            ))
        })?,
        Err(_) => 600,
    };
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut last_error = String::new();
    eprintln!("Opening browser for {url}");
    eprintln!(
        "Complete SSO/MFA in that window. fsv will check cookies every 2s. Press Ctrl-C to cancel."
    );
    let profile = tempfile::Builder::new().prefix("fsv-browser-").tempdir()?;
    let args = vec![
        format!("--user-data-dir={}", profile.path().display()),
        "--remote-debugging-port=0".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--new-window".to_string(),
        url,
    ];
    ops.launch(exe, &args)?;
    let result = (|| -> Result<Cookies> {
        let port = ops.devtools_port(profile.path())?;
        while Instant::now() < deadline {
            let cookies = ops.cookies(port, &domain)?;
            match validate(&cookies) {
                Ok(()) => return Ok(cookies),
                Err(e) => {
                    let msg = e.to_string();
                    if msg != last_error {
                        eprintln!("Waiting for login: {msg}");
                        last_error = msg;
                    }
                    ops.sleep(Duration::from_secs(2));
                }
            }
        }
        Err(Error::session(format!(
            "browser login timed out after {timeout}s"
        )))
    })();
    ops.shutdown();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testenv::{self, isolated};

    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    type Received = Arc<Mutex<Vec<Value>>>;

    /// One local endpoint serving `/json/list` over HTTP and the page
    /// websocket, replying to each websocket connection with the next
    /// scripted frame list (`["text", json] | ["ping"] | ["close"]`).
    fn fake_devtools(scenario: Vec<Value>) -> (u16, Received, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let received: Received = Arc::default();
        let rec = received.clone();
        let handle = std::thread::spawn(move || {
            let targets = json!([
                {"type": "background_page", "url": "https://acme.freshservice.com/", "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/bg")},
                {"type": "page", "url": "https://other.example/", "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/other")},
                {"type": "page", "url": "https://acme.freshservice.com/a/tickets", "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/page/ACME")},
            ]);
            let mut replies = scenario.into_iter();
            while let Ok((mut stream, _)) = listener.accept() {
                let mut head = [0u8; 16];
                let n = stream.peek(&mut head).unwrap();
                if head[..n].starts_with(b"GET /json") {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = stream.read(&mut buf).unwrap();
                        req.extend_from_slice(&buf[..n]);
                    }
                    let body = targets.to_string();
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    continue;
                }
                let Some(frames) = replies.next() else { return };
                let mut path = String::new();
                let mut ws = tungstenite::accept_hdr(
                    stream,
                    |req: &tungstenite::handshake::server::Request, resp| {
                        path = req.uri().path().to_string();
                        Ok(resp)
                    },
                )
                .unwrap();
                let text = ws.read().unwrap().into_text().unwrap().to_string();
                rec.lock()
                    .unwrap()
                    .push(json!({"path": path, "text": text}));
                for f in frames.as_array().unwrap() {
                    let msg = match f[0].as_str().unwrap() {
                        "text" => tungstenite::Message::text(f[1].to_string()),
                        "ping" => tungstenite::Message::Ping(b"hi".to_vec().into()),
                        _ => tungstenite::Message::Close(None),
                    };
                    ws.send(msg).unwrap();
                }
                while ws.read().is_ok() {}
                if replies.as_slice().is_empty() {
                    return;
                }
            }
        });
        (port, received, handle)
    }

    /// `_http_json`, `_page_ws_url`, `_cdp_call` and `_direct_browser_cookies`
    /// against the scenarios Python ran in `gen_fsv_core_session.py`: the
    /// cookies or error, and the CDP requests the endpoint received.
    #[test]
    fn direct_browser_cookies_matches_python_against_fake_devtools() {
        let all: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/session_expected.json"))
                .unwrap();
        for (name, case) in all["cdp"].as_object().unwrap() {
            let (port, received, handle) =
                fake_devtools(case["scenario"].as_array().unwrap().clone());
            let got = match direct_browser_cookies(port, "acme.freshservice.com") {
                Ok(c) => json!(c),
                Err(e) => json!({"error": e.to_string()}),
            };
            handle.join().unwrap();
            assert_eq!(got, case["out"], "{name}");
            assert_eq!(
                Value::Array(received.lock().unwrap().clone()),
                case["received"],
                "{name}"
            );
        }
    }

    #[test]
    fn page_ws_url_and_http_json_read_the_target_list() {
        let (port, _, handle) = fake_devtools(vec![]);
        let url = page_ws_url(port, "acme.freshservice.com").unwrap();
        assert_eq!(url, format!("ws://127.0.0.1:{port}/devtools/page/ACME"));
        let list = http_json(&format!("http://127.0.0.1:{port}/json/list")).unwrap();
        assert_eq!(list.as_array().map(Vec::len), Some(3));
        drop(handle);
    }

    #[test]
    fn browser_executable_prefers_existing_fsv_browser() {
        let env = isolated();
        let exe = env.root().join("br");
        std::fs::write(&exe, "").unwrap();
        testenv::set("FSV_BROWSER", &exe);
        assert_eq!(browser_executable(), Some(exe));
        let missing = env.root().join("nope");
        testenv::set("FSV_BROWSER", &missing);
        assert_ne!(
            browser_executable(),
            Some(missing),
            "a missing FSV_BROWSER is skipped"
        );
    }

    #[test]
    fn devtools_port_reads_first_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("DevToolsActivePort"),
            "9222\n/devtools/browser/x\n",
        )
        .unwrap();
        assert_eq!(
            SystemBrowser::default().devtools_port(dir.path()).unwrap(),
            9222
        );
    }

    #[cfg(unix)]
    #[test]
    fn devtools_port_fails_when_browser_exits() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = SystemBrowser::default();
        b.launch(Path::new("/usr/bin/true"), &[]).unwrap();
        assert_eq!(
            b.devtools_port(dir.path()).unwrap_err().to_string(),
            "browser exited before DevTools started"
        );
        b.shutdown();
        assert!(b.child.is_none());
    }

    #[test]
    fn page_target_selection() {
        let d = "acme.freshservice.com";
        let pick = |v: Value| page_target(&v, d).map_err(|e| e.to_string());
        assert_eq!(
            pick(json!([
                {"type": "page", "url": "https://other/", "webSocketDebuggerUrl": "ws://a"},
                {"type": "page", "url": "https://acme.freshservice.com/a", "webSocketDebuggerUrl": "ws://b"},
            ])),
            Ok("ws://b".into())
        );
        assert_eq!(
            pick(json!([
                {"type": "background_page", "url": "https://acme.freshservice.com", "webSocketDebuggerUrl": "ws://x"},
                {"type": "page", "url": "https://other", "webSocketDebuggerUrl": "ws://first"},
            ])),
            Ok("ws://first".into())
        );
        assert_eq!(
            pick(
                json!([{"type": "page", "url": "https://acme.freshservice.com", "webSocketDebuggerUrl": ""}])
            ),
            Err("browser page target not found".into())
        );
        assert_eq!(
            pick(json!({"x": 1})),
            Err("browser DevTools target list was invalid".into())
        );
        assert_eq!(pick(json!([])), Err("browser page target not found".into()));
        assert_eq!(
            pick(json!(["s", {"type": "page", "webSocketDebuggerUrl": 5}])),
            Ok("5".into())
        );
    }

    #[test]
    fn cdp_cookie_list() {
        let data = json!({"cookies": [
            {"name": "a", "value": "1"}, {"name": "", "value": "2"}, {"name": "b", "value": 3}, "x", {"name": "c", "value": ""},
        ]});
        let got = cookies_from_cdp(&data).unwrap();
        assert_eq!(
            got.into_iter().collect::<Vec<_>>(),
            [("a".into(), "1".into()), ("c".into(), String::new())]
        );
        assert_eq!(
            cookies_from_cdp(&json!({"cookies": "x"}))
                .unwrap_err()
                .to_string(),
            "browser cookie output did not contain a cookie list"
        );
    }

    #[derive(Default)]
    struct Fake {
        launched: Vec<String>,
        rounds: Vec<Cookies>,
        sleeps: Vec<Duration>,
        shutdown: bool,
    }

    impl BrowserOps for Fake {
        fn launch(&mut self, _exe: &Path, args: &[String]) -> Result<()> {
            self.launched = args.to_vec();
            Ok(())
        }
        fn devtools_port(&mut self, _profile_dir: &Path) -> Result<u16> {
            Ok(9)
        }
        fn cookies(&mut self, port: u16, domain: &str) -> Result<Cookies> {
            assert_eq!((port, domain), (9, "acme.freshservice.com"));
            Ok(if self.rounds.len() > 1 {
                self.rounds.remove(0)
            } else {
                self.rounds[0].clone()
            })
        }
        fn sleep(&mut self, d: Duration) {
            self.sleeps.push(d);
        }
        fn shutdown(&mut self) {
            self.shutdown = true;
        }
    }

    fn cookies(pairs: &[(&str, &str)]) -> Cookies {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn login_browser_polls_until_session_is_complete() {
        let _env = isolated().with_domain("acme.freshservice.com");
        let full = cookies(&[
            ("_x_m", "1"),
            ("_x_d", "2"),
            ("_x_w", "3"),
            ("fw-session-id", "4"),
        ]);
        let mut fake = Fake {
            rounds: vec![cookies(&[("_x_m", "1")]), full.clone()],
            ..Fake::default()
        };
        assert_eq!(
            login_browser_with(&mut fake, Path::new("/x/chrome")).unwrap(),
            full
        );
        assert_eq!(fake.sleeps, [Duration::from_secs(2)]);
        assert!(fake.shutdown);
        assert!(
            fake.launched[0].starts_with("--user-data-dir=")
                && fake.launched[0].contains("fsv-browser-")
        );
        assert_eq!(
            fake.launched[1..],
            [
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--new-window",
                "https://acme.freshservice.com/"
            ]
        );
    }

    #[test]
    fn login_browser_timeout() {
        let _env = isolated().with_domain("acme.freshservice.com");
        testenv::set("FSV_BROWSER_LOGIN_TIMEOUT", "0");
        let mut fake = Fake {
            rounds: vec![Cookies::new()],
            ..Fake::default()
        };
        assert_eq!(
            login_browser_with(&mut fake, Path::new("/x"))
                .unwrap_err()
                .to_string(),
            "browser login timed out after 0s"
        );
        assert!(fake.shutdown);
        testenv::set("FSV_BROWSER_LOGIN_TIMEOUT", "abc");
        let e = login_browser_with(&mut fake, Path::new("/x")).unwrap_err();
        assert_eq!(
            e.to_string(),
            "invalid literal for int() with base 10: 'abc'"
        );
        assert_eq!(e.exit_code(), 2);
    }

    #[test]
    fn login_browser_without_domain() {
        let _env = isolated();
        crate::testenv::set("FSV_BROWSER", "/nonexistent/browser");
        let e = login_browser().unwrap_err().to_string();
        assert!(e.starts_with("no Freshservice domain"), "{e}");
        let mut fake = Fake::default();
        assert!(
            login_browser_with(&mut fake, Path::new("/x"))
                .unwrap_err()
                .to_string()
                .starts_with("no Freshservice domain")
        );
        assert!(fake.launched.is_empty());
    }
}
