//! Session cookie storage and login helpers (`session.py`).
//!
//! Backends (byte-compatible with the Python build, both directions):
//! - `file`: `session.json` holding `{"saved_at", "domain", "cookies"}`;
//! - `argon`: the same payload encrypted with AES-256-GCM (domain as AAD)
//!   under an Argon2id key (t=3, m=65536 KiB, p=4), stored as a JSON envelope;
//! - `keychain`: macOS login Keychain via the `security` CLI (service `fsv`,
//!   account `fsv-session`), payload base64-encoded.
//!
//! The backend choice is persisted in `<config>/store`.

mod browser;
mod paste;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use indexmap::IndexMap;
use rand::RngCore;
use serde_json::{Value, json};

pub use browser::{
    BrowserOps, SystemBrowser, browser_executable, login_browser, login_browser_with,
};
pub use paste::{
    extract_cookie_from_input, login_interactive, login_interactive_with, parse_cookie_header,
    read_secret,
};

use crate::config::{self, PRIVATE_MODE, atomic_write_text, ensure_dirs};
use crate::error::{Error, Result};

/// Session cookies, in stored order.
pub type Cookies = IndexMap<String, String>;

/// Cookies a usable Freshservice session must have.
pub const REQUIRED_COOKIES: [&str; 4] = ["_x_m", "_x_d", "_x_w", "fw-session-id"];
pub const KEYCHAIN_SERVICE: &str = "fsv";
pub const KEYCHAIN_ACCOUNT: &str = "fsv-session";

/// Where the session payload is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    File,
    Argon,
    Keychain,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::File => "file",
            Backend::Argon => "argon",
            Backend::Keychain => "keychain",
        }
    }

    pub fn parse(s: &str) -> Option<Backend> {
        match s {
            "file" => Some(Backend::File),
            "argon" => Some(Backend::Argon),
            "keychain" => Some(Backend::Keychain),
            _ => None,
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Decrypted payload cache so argon asks for the passphrase once per process.
static SESSION_CACHE: Mutex<Option<(Backend, String)>> = Mutex::new(None);

fn cache_get(backend: Backend) -> Option<String> {
    let g = SESSION_CACHE.lock().ok()?;
    g.as_ref()
        .filter(|(b, _)| *b == backend)
        .map(|(_, p)| p.clone())
}

fn cache_set(value: Option<(Backend, String)>) {
    if let Ok(mut g) = SESSION_CACHE.lock() {
        *g = value;
    }
}

/// Forget the in-process session cache (tests, re-login).
pub fn clear_cache() {
    cache_set(None);
}

fn store_pref_file() -> PathBuf {
    config::config_dir().join("store")
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------- backend selection ----------

/// Configured backend: the `store` file, else `file` when `session.json`
/// exists, else `keychain` on macOS, else `file`.
pub fn current_backend() -> Backend {
    if let Ok(v) = std::fs::read_to_string(store_pref_file())
        && let Some(b) = Backend::parse(crate::pystr::strip(&v))
    {
        return b;
    }
    if config::session_file().exists() {
        return Backend::File;
    }
    if keychain_available() {
        return Backend::Keychain;
    }
    Backend::File
}

pub fn set_backend(backend: Backend) -> Result<()> {
    ensure_dirs()?;
    atomic_write_text(&store_pref_file(), backend.as_str(), PRIVATE_MODE)
}

// ---------- keychain (macOS) ----------

/// `FSV_NO_KEYCHAIN` unset and running on macOS.
pub fn keychain_available() -> bool {
    if std::env::var_os("FSV_NO_KEYCHAIN").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    cfg!(target_os = "macos")
}

/// Runs `security` with argv and optional stdin; returns `(exit code, stdout)`.
pub(crate) type SecurityRunner<'a> =
    &'a mut dyn FnMut(&[&str], Option<&str>) -> std::io::Result<(i32, String)>;

fn run_security(args: &[&str], input: Option<&str>) -> std::io::Result<(i32, String)> {
    use std::io::Write;
    let mut child = Command::new("security")
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

/// The `security -i` stdin line that stores `value` (base64, so no quoting).
pub(crate) fn keychain_write_line(value: &str) -> String {
    format!(
        "add-generic-password -U -a {KEYCHAIN_ACCOUNT} -s {KEYCHAIN_SERVICE} -w {}\n",
        B64.encode(value.as_bytes())
    )
}

/// Store `value` without exposing it in argv (`security -i` reads the command from stdin).
pub(crate) fn keychain_write_with(value: &str, run: SecurityRunner<'_>) -> Result<()> {
    if !keychain_available() {
        return Err(Error::session("keychain backend is macOS only"));
    }
    match run(&["-i"], Some(&keychain_write_line(value))) {
        Ok((0, _)) => Ok(()),
        _ => Err(Error::session("could not write session to Keychain")),
    }
}

fn keychain_write(value: &str) -> Result<()> {
    keychain_write_with(value, &mut run_security)
}

/// Decode a stored Keychain value (base64; legacy entries are raw JSON).
pub fn keychain_decode(raw: &str) -> String {
    let raw = crate::pystr::strip(raw);
    if raw.starts_with('{') {
        return raw.to_string();
    }
    B64.decode(raw.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| raw.to_string())
}

pub(crate) fn keychain_read_with(run: SecurityRunner<'_>) -> Result<String> {
    if !keychain_available() {
        return Err(Error::session("keychain backend is macOS only"));
    }
    match run(
        &[
            "find-generic-password",
            "-a",
            KEYCHAIN_ACCOUNT,
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
        ],
        None,
    ) {
        Ok((0, out)) => Ok(keychain_decode(&out)),
        _ => Err(Error::session(
            "no session in Keychain; run `fsv auth login`",
        )),
    }
}

fn keychain_read() -> Result<String> {
    keychain_read_with(&mut run_security)
}

fn keychain_delete() {
    if keychain_available() {
        let _ = run_security(
            &[
                "delete-generic-password",
                "-a",
                KEYCHAIN_ACCOUNT,
                "-s",
                KEYCHAIN_SERVICE,
            ],
            None,
        );
    }
}

// ---------- argon2 file encryption ----------

/// Argon2id parameters stored in the envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParams {
    pub time_cost: u32,
    pub memory_cost: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        KdfParams {
            time_cost: 3,
            memory_cost: 65536,
            parallelism: 4,
        }
    }
}

type Key = [u8; 32];

fn derive_key(passphrase: &str, salt: &[u8], p: KdfParams) -> Result<Key> {
    let params = argon2::Params::new(p.memory_cost, p.time_cost, p.parallelism, Some(32))
        .map_err(|e| Error::session(format!("invalid encrypted session file ({e})")))?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key: Key = [0; 32];
    a.hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| Error::session(format!("invalid encrypted session file ({e})")))?;
    Ok(key)
}

/// Port of fsv.session._b64e.
fn b64e(b: &[u8]) -> String {
    B64.encode(b)
}

/// Port of fsv.session._b64d: CPython's non-strict `a2b_base64`. Characters
/// outside the alphabet are skipped, and decoding stops at the padding that
/// completes a quad, so files Python reads also read here.
fn b64d(s: &str) -> std::result::Result<Vec<u8>, String> {
    if !s.is_ascii() {
        return Err("'ascii' codec can't encode the input".into());
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut quad_pos, mut pads, mut left, mut data_chars) = (0u8, 0u8, 0u8, 0usize);
    for c in s.bytes() {
        if c == b'=' {
            if quad_pos >= 2 {
                pads += 1;
                if quad_pos + pads >= 4 {
                    return Ok(out);
                }
            }
            continue;
        }
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        pads = 0;
        data_chars += 1;
        match quad_pos {
            0 => {
                left = v;
                quad_pos = 1;
            }
            1 => {
                out.push((left << 2) | (v >> 4));
                left = v & 0x0f;
                quad_pos = 2;
            }
            2 => {
                out.push((left << 4) | (v >> 2));
                left = v & 0x03;
                quad_pos = 3;
            }
            _ => {
                out.push((left << 6) | v);
                quad_pos = 0;
            }
        }
    }
    match quad_pos {
        0 => Ok(out),
        1 => Err(format!(
            "Invalid base64-encoded string: number of data characters ({data_chars}) cannot be 1 more than a multiple of 4"
        )),
        _ => Err("Incorrect padding".into()),
    }
}

/// Hidden-input prompt: `getpass.getpass` in Python.
type SecretPrompt<'a> = &'a mut dyn FnMut(&str) -> Result<String>;

fn tty_prompt(prompt: &str) -> Result<String> {
    rpassword::prompt_password(prompt).map_err(|_| Error::Aborted)
}

/// Port of fsv.session._passphrase.
fn passphrase(confirm: bool, ask: SecretPrompt<'_>) -> Result<String> {
    let p1 = ask("Passphrase: ")?;
    if p1.is_empty() {
        return Err(Error::session("empty passphrase"));
    }
    if confirm {
        let p2 = ask("Confirm passphrase: ")?;
        if p1 != p2 {
            return Err(Error::session("passphrases do not match"));
        }
    }
    Ok(p1)
}

/// Encrypt `raw` into the argon JSON envelope.
pub fn argon_encrypt_with(raw: &str, passphrase: &str, domain: &str) -> Result<String> {
    let params = KdfParams::default();
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    rand::rng().fill_bytes(&mut salt);
    rand::rng().fill_bytes(&mut nonce);
    let key = derive_key(passphrase, &salt, params)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| Error::other(e.to_string()))?;
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: raw.as_bytes(),
                aad: domain.as_bytes(),
            },
        )
        .map_err(|_| Error::other("encryption failed"))?;
    let doc = json!({
        "version": 1,
        "backend": "argon",
        "kdf": "argon2id",
        "cipher": "aes-256-gcm",
        "params": {"time_cost": params.time_cost, "memory_cost": params.memory_cost, "parallelism": params.parallelism},
        "salt": b64e(&salt),
        "nonce": b64e(&nonce),
        "data": b64e(&ct),
    });
    Ok(fsv_py::Dumps::default().dump(&doc))
}

fn int_param(p: Option<&Value>, lo: i64, hi: i64) -> Option<u32> {
    let v = match p? {
        Value::Bool(b) => i64::from(*b),
        v => crate::value::as_int(v)?,
    };
    if (lo..=hi).contains(&v) {
        u32::try_from(v).ok()
    } else {
        None
    }
}

/// Validated KDF parameters of an argon envelope.
pub fn argon_params(raw: &str) -> Result<KdfParams> {
    let data: Value = serde_json::from_str(raw)?;
    let p = data.get("params").filter(|v| crate::value::truthy(v));
    let get = |k: &str, lo: i64, hi: i64| int_param(p.and_then(|p| p.get(k)), lo, hi);
    let (Some(t), Some(m), Some(par)) = (
        get("time_cost", 1, 10),
        get("memory_cost", 8, 1 << 20),
        get("parallelism", 1, 16),
    ) else {
        return Err(Error::session(
            "invalid encrypted session file (unreasonable KDF parameters)",
        ));
    };
    if data.get("backend").and_then(Value::as_str) != Some("argon")
        || data.get("kdf").and_then(Value::as_str) != Some("argon2id")
    {
        return Err(Error::session("invalid encrypted session file"));
    }
    Ok(KdfParams {
        time_cost: t,
        memory_cost: m,
        parallelism: par,
    })
}

/// Decrypt an argon envelope.
pub fn argon_decrypt_with(raw: &str, passphrase: &str, domain: &str) -> Result<String> {
    let params = argon_params(raw)?;
    let data: Value = serde_json::from_str(raw)?;
    let field = |k: &str| -> Result<Vec<u8>> {
        let s = data
            .get(k)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::session("invalid encrypted session file"))?;
        b64d(s).map_err(|_| Error::session("invalid encrypted session file"))
    };
    let (salt, nonce, ct) = (field("salt")?, field("nonce")?, field("data")?);
    let key = derive_key(passphrase, &salt, params)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| Error::other(e.to_string()))?;
    if nonce.len() != 12 {
        return Err(Error::session("wrong passphrase or corrupted session"));
    }
    let pt = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ct,
                aad: domain.as_bytes(),
            },
        )
        .map_err(|_| Error::session("wrong passphrase or corrupted session"))?;
    String::from_utf8(pt).map_err(|e| Error::value(e.to_string()))
}

/// Port of fsv.session._argon_encrypt: asks for the passphrase twice; the
/// configured domain is the AAD.
fn argon_encrypt(raw: &str, ask: SecretPrompt<'_>) -> Result<String> {
    argon_encrypt_with(raw, &passphrase(true, ask)?, &config::domain())
}

fn argon_decrypt(raw: &str) -> Result<String> {
    argon_params(raw)?;
    argon_decrypt_with(raw, &passphrase(false, &mut tty_prompt)?, &config::domain())
}

// ---------- unified read/write ----------

fn read_file_session(backend: Backend) -> Result<String> {
    if let Some(p) = cache_get(backend) {
        return Ok(p);
    }
    let file = config::session_file();
    if !file.exists() {
        return Err(Error::session("no session; run `fsv auth login`"));
    }
    let raw = std::fs::read_to_string(&file)?;
    let payload = if backend == Backend::Argon {
        argon_decrypt(&raw)?
    } else {
        raw
    };
    cache_set(Some((backend, payload.clone())));
    Ok(payload)
}

fn read_payload(backend: Backend) -> Result<String> {
    if backend == Backend::Keychain {
        keychain_read()
    } else {
        read_file_session(backend)
    }
}

fn missing_list(cookies: &Cookies) -> Vec<&'static str> {
    REQUIRED_COOKIES
        .iter()
        .copied()
        .filter(|c| !cookies.contains_key(*c))
        .collect()
}

fn py_list(items: &[&str]) -> String {
    fsv_py::py_repr(&Value::Array(
        items
            .iter()
            .map(|s| Value::String((*s).to_string()))
            .collect(),
    ))
}

fn cookies_of(data: &Value) -> Cookies {
    match data.get("cookies") {
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| (k.clone(), fsv_py::py_str(v)))
            .collect(),
        _ => Cookies::new(),
    }
}

/// Stored cookies for the configured domain (all [`REQUIRED_COOKIES`] present).
pub fn load_cookies() -> Result<Cookies> {
    let raw = read_payload(current_backend())?;
    let data: Value = serde_json::from_str(&raw)?;
    if !data.is_object() {
        return Err(Error::session("invalid session; run `fsv auth login`"));
    }
    let domain = config::require_domain()?;
    let stored = data.get("domain");
    if stored.and_then(Value::as_str) != Some(domain.as_str()) {
        let shown = fsv_py::py_str(stored.unwrap_or(&Value::Null));
        return Err(Error::session(format!(
            "session is for {shown}, not {domain}; run `fsv auth login`"
        )));
    }
    let cookies = cookies_of(&data);
    let missing = missing_list(&cookies);
    if !missing.is_empty() {
        return Err(Error::session(format!(
            "session missing cookies {}; run `fsv auth login`",
            py_list(&missing)
        )));
    }
    Ok(cookies)
}

fn cookies_value(cookies: &Cookies) -> Value {
    Value::Object(
        cookies
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect(),
    )
}

/// Save cookies for the configured domain; `None` keeps the current backend.
pub fn save_cookies(cookies: &Cookies, backend: Option<Backend>) -> Result<Backend> {
    let backend = backend.unwrap_or_else(current_backend);
    let payload = fsv_py::Dumps::default().dump(&json!({
        "saved_at": now(),
        "domain": config::require_domain()?,
        "cookies": cookies_value(cookies),
    }));
    if backend == Backend::Keychain {
        keychain_write(&payload)?;
        let _ = std::fs::remove_file(config::session_file());
    } else {
        ensure_dirs()?;
        let data = if backend == Backend::Argon {
            argon_encrypt(&payload, &mut tty_prompt)?
        } else {
            payload.clone()
        };
        atomic_write_text(&config::session_file(), &data, PRIVATE_MODE)?;
        keychain_delete();
    }
    cache_set(Some((backend, payload)));
    set_backend(backend)?;
    Ok(backend)
}

/// Best-effort single-cookie update. Silent on argon (needs a passphrase) and on errors.
pub fn update_cookie(name: &str, value: &str) {
    let backend = current_backend();
    if backend == Backend::Argon {
        return;
    }
    let run = || -> Result<()> {
        let raw = read_payload(backend)?;
        let mut data: Value = serde_json::from_str(&raw)?;
        if data
            .get("cookies")
            .and_then(|c| c.get(name))
            .and_then(Value::as_str)
            == Some(value)
        {
            return Ok(());
        }
        let cookies = data
            .get_mut("cookies")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| Error::session("invalid session"))?;
        cookies.insert(name.to_string(), Value::String(value.to_string()));
        let payload = fsv_py::Dumps::default().dump(&data);
        if backend == Backend::Keychain {
            keychain_write(&payload)?;
        } else {
            atomic_write_text(&config::session_file(), &payload, PRIVATE_MODE)?;
        }
        cache_set(Some((backend, payload)));
        Ok(())
    };
    let _ = run();
}

/// Remove the session from every backend and the backend preference.
pub fn logout() {
    cache_set(None);
    let _ = std::fs::remove_file(config::session_file());
    keychain_delete();
    let _ = std::fs::remove_file(store_pref_file());
}

/// `Cookie` header value for the stored session.
pub fn cookie_header() -> Result<String> {
    Ok(load_cookies()?
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; "))
}

/// Hours since the session was saved (`None` when unknown/no session).
pub fn session_age_hours() -> Option<f64> {
    let raw = read_payload(current_backend()).ok()?;
    let data: Value = serde_json::from_str(&raw).ok()?;
    let saved = data
        .get("saved_at")
        .filter(|v| crate::value::truthy(v))?
        .as_f64()?;
    Some((now() - saved) / 3600.0)
}

/// Fail unless all [`REQUIRED_COOKIES`] are present.
pub fn validate(cookies: &Cookies) -> Result<()> {
    let missing = missing_list(cookies);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::session(format!(
            "missing required cookies: {}",
            py_list(&missing)
        )))
    }
}

#[cfg(test)]
mod tests;
