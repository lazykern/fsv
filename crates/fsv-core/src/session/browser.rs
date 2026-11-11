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
    for name in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "chrome", "msedge"] {
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
        ("ProgramFiles(x86)", r"Microsoft\Edge\Application\msedge.exe"),
        ("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
        ("ProgramFiles", r"BraveSoftware\Brave-Browser\Application\brave.exe"),
        ("ProgramFiles(x86)", r"BraveSoftware\Brave-Browser\Application\brave.exe"),
        ("LOCALAPPDATA", r"BraveSoftware\Brave-Browser\Application\brave.exe"),
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
            if let Some(child) = self.child.as_mut() {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return Err(Error::session("browser exited before DevTools started"));
                }
            }
            if let Ok(text) = std::fs::read_to_string(&port_file) {
                // The file can be observed half-written; retry until the first line parses.
                if let Some(port) = text.lines().next().and_then(|l| l.trim().parse::<u16>().ok()) {
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
        let Some(mut child) = self.child.take() else { return };
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

fn http_json(url: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| Error::other(e.to_string()))?;
    let r = client.get(url).send().map_err(|e| Error::other(format!("<urlopen error {e}>")))?;
    let text = r.text().map_err(|e| Error::other(e.to_string()))?;
    Ok(serde_json::from_str(&text)?)
}

fn page_ws_url(port: u16, domain: &str) -> Result<String> {
    let targets = http_json(&format!("http://127.0.0.1:{port}/json/list"))?;
    let Value::Array(targets) = targets else {
        return Err(Error::session("browser DevTools target list was invalid"));
    };
    let pages: Vec<&Value> = targets
        .iter()
        .filter(|t| {
            t.get("type").and_then(Value::as_str) == Some("page")
                && t.get("webSocketDebuggerUrl").is_some_and(crate::value::truthy)
        })
        .collect();
    let target = pages
        .iter()
        .find(|t| fsv_py::py_str(t.get("url").unwrap_or(&Value::String(String::new()))).contains(domain))
        .or(pages.first())
        .ok_or_else(|| Error::session("browser page target not found"))?;
    Ok(fsv_py::py_str(&target["webSocketDebuggerUrl"]))
}

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
    ws.send(tungstenite::Message::text(msg)).map_err(|_| Error::session("browser DevTools websocket closed"))?;
    loop {
        let frame = ws.read().map_err(|_| Error::session("browser DevTools websocket closed"))?;
        let text = match frame {
            tungstenite::Message::Text(t) => t.to_string(),
            tungstenite::Message::Close(_) => return Err(Error::session("browser DevTools websocket closed")),
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

/// Cookies of the page target via `Network.getCookies` (fallback `Network.getAllCookies`).
pub fn direct_browser_cookies(port: u16, domain: &str) -> Result<Cookies> {
    let ws_url = page_ws_url(port, domain)?;
    let data = match cdp_call(&ws_url, "Network.getCookies", json!({"urls": [format!("https://{domain}/")]})) {
        Ok(d) => d,
        Err(Error::Session(_)) => cdp_call(&ws_url, "Network.getAllCookies", json!({}))?,
        Err(e) => return Err(e),
    };
    let Some(Value::Array(items)) = data.get("cookies") else {
        return Err(Error::session("browser cookie output did not contain a cookie list"));
    };
    let mut out = Cookies::new();
    for item in items {
        if let (Some(Value::String(name)), Some(Value::String(value))) = (item.get("name"), item.get("value")) {
            if !name.is_empty() {
                out.insert(name.clone(), value.clone());
            }
        }
    }
    Ok(out)
}

/// Open an isolated browser and poll its cookies until the session is complete.
pub fn login_browser() -> Result<Cookies> {
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
        Ok(v) => crate::pystr::strip(&v)
            .parse()
            .map_err(|_| Error::value(format!("invalid literal for int() with base 10: {}", fsv_py::py_repr_str(&v))))?,
        Err(_) => 600,
    };
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut last_error = String::new();
    eprintln!("Opening browser for {url}");
    eprintln!("Complete SSO/MFA in that window. fsv will check cookies every 2s. Press Ctrl-C to cancel.");
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
        Err(Error::session(format!("browser login timed out after {timeout}s")))
    })();
    ops.shutdown();
    result
}
