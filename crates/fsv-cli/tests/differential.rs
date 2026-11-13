//! Differential test: the Python `fsv` and the Rust `fsv` run the same
//! commands against the same canned routes, and their exit codes, stdout,
//! stderr and HTTP request logs are compared.
//!
//! It needs a Python environment with the pinned upstream `fsv` installed, so it
//! is ignored by default:
//!
//! ```text
//! git -C /path/to/fsv archive "$(cat port/UPSTREAM_BASE)" src | tar -x -C /tmp/fsv-pin
//! FSV_PY_PYTHON=/path/to/fsv/.venv/bin/python FSV_PY_SRC=/tmp/fsv-pin/src \
//!     cargo test -p fsv-cli --test differential -- --ignored --nocapture
//! ```
//!
//! `FSV_PY_SRC` goes first on the Python side's `PYTHONPATH`, so the pinned
//! upstream source is what runs even when the checkout has moved on.
//!
//! The Python side runs an embedded shim (`PY_SHIM`) that answers httpx
//! requests from the same `FSV_TEST_ROUTES` file the Rust mock reads, with the
//! same matching rules, so no `.py` file lives in this repository.
//!
//! Known differences (all in `port/PARITY.md`) are normalised rather than
//! listed: GETs are compared as a multiset (both sides run independent GETs on
//! threads); an escaped Python traceback is compared by its final exception
//! line against the Rust one-line error; Click and clap usage errors are
//! compared by kind; JSON decoder wording and form-metadata fetch times are
//! masked. The rest is compared exactly.
//!
//! `fixtures/diff_cases.json` is every literal `run_fixture`/`run_files` call
//! in this crate's tests (except `tui`, `setup`, `login` and installers).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const DOMAIN: &str = "acme.freshservice.com";

/// The Python side of the differential. Mirrors `crates/fsv-cli/src/mock.rs`.
const PY_SHIM: &str = r#"
import json, os, sys
import httpx

if os.environ.get("FSV_TEST_TTY") == "1":
    import io

    class _Tty(io.TextIOWrapper):
        def isatty(self):
            return True

    sys.stdin = _Tty(sys.stdin.detach(), encoding="utf-8")

routes = json.load(open(os.environ["FSV_TEST_ROUTES"]))
log_path = os.environ.get("FSV_TEST_LOG") or None
used = [False] * len(routes)

def matches(route, req):
    if route.get("method", "GET") != req.method:
        return False
    url = httpx.URL(str(req.url))
    if route.get("path") != url.path:
        return False
    query = route.get("query")
    if isinstance(query, dict):
        pairs = list(url.params.multi_items())
        for k, v in query.items():
            want = v if isinstance(v, str) else json.dumps(v)
            if not any(pk == k and pv == want for pk, pv in pairs):
                return False
    return True

def respond(route):
    status = route.get("status", 200)
    if "text" in route:
        resp = httpx.Response(status, text=route["text"], headers={"content-type": "text/html"})
    else:
        resp = httpx.Response(status, json=route.get("json", {}), headers={"content-type": "application/json"})
    for k, v in (route.get("headers") or {}).items():
        resp.headers[k] = v
    return resp

def handler(req):
    if log_path:
        content = req.content
        if content:
            try:
                body = json.loads(content)
            except ValueError:
                body = content.decode(errors="replace")
        else:
            body = None
        with open(log_path, "a") as f:
            f.write(json.dumps({"method": req.method, "url": str(req.url), "body": body}) + "\n")
    found = None
    for i, route in enumerate(routes):
        if not used[i] and matches(route, req):
            found = i
            break
    if found is None:
        return httpx.Response(404, json={"errors": "no route"}, headers={"content-type": "application/json"})
    if routes[found].get("once") is True:
        used[found] = True
    return respond(routes[found])

_orig_init = httpx.Client.__init__

def _init(self, *a, **kw):
    kw["transport"] = httpx.MockTransport(handler)
    _orig_init(self, *a, **kw)

httpx.Client.__init__ = _init

sys.argv = ["fsv", *sys.argv[1:]]
import fsv

try:
    fsv.main()
except SystemExit as e:
    code = e.code
    if code is None:
        code = 0
    elif not isinstance(code, int):
        sys.stderr.write(str(code) + "\n")
        code = 1
    sys.exit(code)
"#;

/// One row of `fixtures/diff_cases.json`:
/// `{"fixture": NAME, "args": [...], "files"?: DIR, "stdin"?: TEXT,
///   "pre"?: [[...]], "tty"?: true, "env"?: {K: V}}`.
/// `pre` commands run first in the same home with empty stdin; `tty` sets
/// `FSV_TEST_TTY=1`. `@MANIFEST@` in an argument is this crate's directory.
struct Case {
    fixture: String,
    args: Vec<String>,
    files: Option<String>,
    stdin: Option<String>,
    pre: Vec<Vec<String>>,
    tty: bool,
    env: Vec<(String, String)>,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|s| {
                    s.as_str()
                        .unwrap()
                        .replace("@MANIFEST@", env!("CARGO_MANIFEST_DIR"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn load_cases() -> Vec<Case> {
    let rows: Vec<Value> = serde_json::from_str(include_str!("fixtures/diff_cases.json")).unwrap();
    let text = |v: &Value| v.as_str().map(str::to_string);
    rows.iter()
        .map(|r| Case {
            fixture: r["fixture"].as_str().unwrap().to_string(),
            args: strings(&r["args"]),
            files: text(&r["files"]),
            stdin: text(&r["stdin"]),
            pre: r["pre"]
                .as_array()
                .map(|p| p.iter().map(strings).collect())
                .unwrap_or_default(),
            tty: r["tty"].as_bool().unwrap_or(false),
            env: r["env"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

#[derive(Debug, PartialEq)]
struct Outcome {
    code: i32,
    out: String,
    err: String,
    gets: Vec<String>,
    writes: Vec<String>,
}

fn fixture_routes(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/routes")
        .join(format!("{name}.json"))
}

/// Config, session, cwd and routes for one run, in a fresh home.
fn prepare(home: &Path, case: &Case) -> PathBuf {
    let fixture = case.fixture.as_str();
    let cfg = home.join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(
        cfg.join("config.json"),
        json!({"domain": DOMAIN}).to_string(),
    )
    .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let session = json!({
        "saved_at": now,
        "domain": DOMAIN,
        "cookies": {"_x_m": "m", "_x_d": "d", "_x_w": "w", "fw-session-id": "s"},
    });
    std::fs::write(cfg.join("session.json"), session.to_string()).unwrap();
    let work = home.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::copy(fixture_routes(fixture), home.join("routes.json")).unwrap();
    if let Some(files) = &case.files {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/files")
            .join(files);
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), work.join(entry.file_name())).unwrap();
        }
    }
    work
}

fn run(program: &[&str], home: &Path, work: &Path, case: &Case) -> Outcome {
    for step in &case.pre {
        step_output(program, home, work, case, step, "");
    }
    let _ = std::fs::remove_file(home.join("requests.log"));
    let input = case.stdin.clone().unwrap_or_default();
    let out = step_output(program, home, work, case, &case.args, &input);
    let mut gets = Vec::new();
    let mut writes = Vec::new();
    if let Ok(log) = std::fs::read_to_string(home.join("requests.log")) {
        for line in log.lines().filter(|l| !l.trim().is_empty()) {
            let r: Value = serde_json::from_str(line).unwrap();
            let url = r["url"]
                .as_str()
                .unwrap_or_default()
                .replace(DOMAIN, "DOMAIN");
            let body = r["body"].to_string();
            let method = r["method"].as_str().unwrap_or_default().to_string();
            if method == "GET" || method == "HEAD" {
                gets.push(format!("{method} {url}"));
            } else {
                writes.push(format!("{method} {url} {body}"));
            }
        }
    }
    gets.sort();
    let mask = |bytes: &[u8]| mask_home(&String::from_utf8_lossy(bytes), home);
    Outcome {
        code: out.status.code().unwrap_or(-1),
        out: mask_fetched(&mask(&out.stdout)),
        err: mask(&out.stderr),
        gets,
        writes,
    }
}

/// Each side runs in its own temporary home.
fn mask_home(text: &str, home: &Path) -> String {
    let mut text = text.to_string();
    let real = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    for h in [real, home.to_path_buf()] {
        text = text.replace(&*h.to_string_lossy(), "<home>");
    }
    text
}

fn step_output(
    program: &[&str],
    home: &Path,
    work: &Path,
    case: &Case,
    args: &[String],
    input: &str,
) -> std::process::Output {
    let mut cmd = Command::new(program[0]);
    cmd.args(&program[1..])
        .args(args)
        .current_dir(work)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("FSV_CONFIG_DIR", home.join("config"))
        .env("FSV_CACHE_DIR", home.join("cache"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("FSV_NO_KEYCHAIN", "1")
        .env("NO_COLOR", "1")
        .env("COLUMNS", "200")
        .env("TZ", "UTC")
        .env("FSV_TEST_ROUTES", home.join("routes.json"))
        .env("FSV_TEST_LOG", home.join("requests.log"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if case.tty {
        cmd.env("FSV_TEST_TTY", "1");
    }
    for (k, v) in &case.env {
        cmd.env(k, v);
    }
    if program.len() > 1
        && let Ok(src) = std::env::var("FSV_PY_SRC")
    {
        cmd.env("PYTHONPATH", src);
    }
    let mut child = cmd.spawn().unwrap();
    let input = input.to_string();
    let mut pipe = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let _ = pipe.write_all(input.as_bytes());
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    out
}

/// Form metadata lines carry the wall-clock fetch time
/// (`fetched 2026-11-13T05:20:03+00:00`), which can tick between the runs.
fn mask_fetched(text: &str) -> String {
    const STAMP: usize = "2026-11-13T05:20:03+00:00".len();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("fetched ") {
        let start = i + "fetched ".len();
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let stamp = rest.get(..STAMP).unwrap_or_default();
        if stamp.as_bytes().get(10) == Some(&b'T') && stamp.ends_with("+00:00") {
            out.push_str("<time>");
            rest = &rest[STAMP..];
        }
    }
    out.push_str(rest);
    out
}

/// Usage errors exit 2 on both sides. Click draws `Usage: …` and a Rich error
/// panel; clap prints `error: …` and `For more information, try '--help'.`
/// (a documented divergence), so only the kind is compared.
fn is_usage_error(err: &str) -> bool {
    (err.starts_with("Usage: ") && err.contains("Error"))
        || err.ends_with("For more information, try '--help'.\n")
}

/// Python prints an escaped exception as a Rich traceback that ends in
/// `ExceptionName: message`. Rust prints `error: message`. Compare the message.
fn normalise_err(err: &str) -> String {
    if is_usage_error(err) {
        return "<usage error>\n".into();
    }
    // The decoder's own wording follows (CPython `json` vs serde_json).
    const BAD_JSON: &str = "invalid --query-hash JSON: ";
    if let Some(i) = err.find(BAD_JSON) {
        return format!("{}<decoder message>\n", &err[..i + BAD_JSON.len()]);
    }
    if !err.contains("Traceback") {
        return err.to_string();
    }
    let last = err
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty() && !l.starts_with('╰'))
        .unwrap_or_default();
    // `APIError: HTTP 404 …` → `HTTP 404 …`
    match last.split_once(": ") {
        Some((_, msg)) => format!("error: {msg}\n"),
        None => last.to_string(),
    }
}

fn describe(a: &Outcome, b: &Outcome) -> Vec<String> {
    let mut diffs = Vec::new();
    if a.code != b.code {
        diffs.push(format!("exit: py {} rs {}", a.code, b.code));
    }
    if a.out != b.out {
        diffs.push("stdout differs".into());
    }
    let (ea, eb) = (normalise_err(&a.err), normalise_err(&b.err));
    if ea != eb {
        diffs.push(format!("stderr differs:\n  py: {ea:?}\n  rs: {eb:?}"));
    }
    if a.gets != b.gets {
        diffs.push(format!(
            "GETs differ:\n  py: {:?}\n  rs: {:?}",
            a.gets, b.gets
        ));
    }
    if a.writes != b.writes {
        diffs.push(format!(
            "writes differ:\n  py: {:?}\n  rs: {:?}",
            a.writes, b.writes
        ));
    }
    diffs
}

#[test]
#[ignore = "needs FSV_PY_PYTHON, a Python env with the pinned upstream fsv installed"]
fn python_and_rust_agree_on_canned_routes() {
    let python = std::env::var("FSV_PY_PYTHON")
        .expect("set FSV_PY_PYTHON to the Python fsv environment's interpreter");
    let shim_dir = tempfile::tempdir().unwrap();
    let shim = shim_dir.path().join("fsv_shim.py");
    std::fs::write(&shim, PY_SHIM).unwrap();
    let shim = shim.to_string_lossy().into_owned();
    let rust = env!("CARGO_BIN_EXE_fsv");

    // `FSV_DIFF_ONLY=text` runs the cases whose label contains it.
    let only = std::env::var("FSV_DIFF_ONLY").ok();
    let cases: Vec<Case> = load_cases()
        .into_iter()
        .filter(|c| {
            only.as_deref()
                .is_none_or(|o| format!("{}: fsv {}", c.fixture, c.args.join(" ")).contains(o))
        })
        .collect();
    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut succeeded = 0;
    for case in &cases {
        let label = format!("{}: fsv {}", case.fixture, case.args.join(" "));
        let py_home = tempfile::tempdir().unwrap();
        let py_work = prepare(py_home.path(), case);
        let py = run(
            &[python.as_str(), shim.as_str()],
            py_home.path(),
            &py_work,
            case,
        );
        let rs_home = tempfile::tempdir().unwrap();
        let rs_work = prepare(rs_home.path(), case);
        let rs = run(&[rust], rs_home.path(), &rs_work, case);
        if py.code == 0 {
            succeeded += 1;
        }
        let diffs = describe(&py, &rs);
        if !diffs.is_empty() {
            failures.insert(label, diffs);
        }
    }
    let total = cases.len();
    println!("{succeeded} of {total} cases exit 0 on the Python side");
    for (label, diffs) in &failures {
        println!("DIFF {label}");
        for d in diffs {
            println!("  {d}");
        }
    }
    // Many cases are refusals, but a broken setup fails them all alike.
    assert!(
        only.is_some() || succeeded * 3 >= total,
        "too few successful cases to mean anything"
    );
    assert!(
        failures.is_empty(),
        "{} of {total} cases differ",
        failures.len()
    );
}
