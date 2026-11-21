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
//! Cases come from three places. `fixtures/diff_cases.json` is every literal
//! `run_fixture`/`run_files` call in this crate's tests (except `tui`, `setup`,
//! `login` and installers) plus the `"tag": "hand"` rows that
//! `golden/gen_hand_cases.py` writes (`hash_from` runs a review on the Python
//! side and `@HASH@` in the args becomes its plan hash on both sides).
//! `fixtures/diff_env_cases.json` is every `Env::run` of the tests that build
//! their own routes (`golden/collect_env_cases.py`). `FSV_DIFF_TAG`,
//! `FSV_DIFF_ONLY`, `FSV_DIFF_VERBOSE` and `FSV_DIFF_EXITS` narrow or widen
//! the report. Request headers are compared too, except the ones httpx adds
//! below the transport.

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
            headers = {k.lower(): v for k, v in req.headers.items()}
            f.write(json.dumps({"method": req.method, "url": str(req.url), "body": body, "headers": headers}) + "\n")
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
#[derive(Clone)]
struct Case {
    fixture: String,
    args: Vec<String>,
    files: Option<String>,
    stdin: Option<String>,
    pre: Vec<Step>,
    tty: bool,
    env: Vec<(String, String)>,
    routes: Option<Value>,
    seed: Vec<(String, Vec<u8>)>,
    session: bool,
    hash_from: Option<Vec<String>>,
    tag: Option<String>,
}

/// An earlier command of the same home; `routes` replaces the route table
/// for it when the case carries its own.
#[derive(Clone)]
struct Step {
    args: Vec<String>,
    stdin: String,
    routes: Option<Value>,
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

/// Cases collected from the integration tests' `Env` runs (inline routes and
/// seed files): `{"name", "steps": [{"args", "stdin", "routes", "seed",
/// "session", "vars"}]}`, the last step being the compared one.
fn load_env_cases() -> Vec<Case> {
    let rows: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/diff_env_cases.json")).unwrap();
    let text = |v: &Value| v.as_str().map(str::to_string);
    rows.iter()
        .map(|r| {
            let steps = r["steps"].as_array().unwrap();
            let (last, before) = steps.split_last().unwrap();
            let mut seed = Vec::new();
            for st in steps {
                for (rel, v) in st["seed"].as_object().into_iter().flatten() {
                    let bytes = match v["text"].as_str() {
                        Some(t) => t.as_bytes().to_vec(),
                        None => {
                            let h = v["hex"].as_str().unwrap();
                            (0..h.len() / 2)
                                .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
                                .collect()
                        }
                    };
                    seed.push((rel.clone(), bytes));
                }
            }
            let vars = last["vars"].as_array().cloned().unwrap_or_default();
            let tty = vars.iter().any(|kv| kv[0] == "FSV_TEST_TTY");
            Case {
                fixture: format!("env:{}", r["name"].as_str().unwrap()),
                args: strings(&last["args"]),
                files: None,
                stdin: text(&last["stdin"]),
                pre: before
                    .iter()
                    .map(|st| Step {
                        args: strings(&st["args"]),
                        stdin: text(&st["stdin"]).unwrap_or_default(),
                        routes: Some(st["routes"].clone()),
                    })
                    .collect(),
                tty,
                env: vars
                    .iter()
                    .filter(|kv| kv[0] != "FSV_TEST_TTY")
                    .map(|kv| {
                        (
                            kv[0].as_str().unwrap().into(),
                            kv[1].as_str().unwrap().into(),
                        )
                    })
                    .collect(),
                routes: Some(last["routes"].clone()),
                seed,
                session: last["session"].as_bool().unwrap_or(true),
                hash_from: None,
                tag: None,
            }
        })
        .collect()
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
                .map(|p| {
                    p.iter()
                        .map(|a| Step {
                            args: strings(a),
                            stdin: String::new(),
                            routes: None,
                        })
                        .collect()
                })
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
            routes: None,
            seed: Vec::new(),
            session: true,
            hash_from: r.get("hash_from").map(strings),
            tag: text(&r["tag"]),
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
    if case.session {
        std::fs::write(cfg.join("session.json"), session.to_string()).unwrap();
    }
    let work = home.join("work");
    std::fs::create_dir_all(&work).unwrap();
    if let Some(routes) = &case.routes {
        write_routes(home, routes);
    } else {
        std::fs::copy(fixture_routes(fixture), home.join("routes.json")).unwrap();
    }
    for (rel, bytes) in &case.seed {
        let dest = home.join(rel);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let bytes = match String::from_utf8(bytes.clone()) {
            Ok(t) => unhome(&t, home).into_bytes(),
            Err(e) => e.into_bytes(),
        };
        std::fs::write(dest, bytes).unwrap();
    }
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

fn unhome(text: &str, home: &Path) -> String {
    text.replace("@HOME@", &home.to_string_lossy())
}

fn write_routes(home: &Path, routes: &Value) {
    std::fs::write(home.join("routes.json"), unhome(&routes.to_string(), home)).unwrap();
}

fn run(program: &[&str], home: &Path, work: &Path, case: &Case) -> Outcome {
    for step in &case.pre {
        if let Some(routes) = &step.routes {
            write_routes(home, routes);
        }
        step_output(program, home, work, case, &step.args, &step.stdin);
    }
    if let Some(routes) = &case.routes {
        write_routes(home, routes);
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
            let body = mask_boundary(&r["body"].to_string());
            let method = r["method"].as_str().unwrap_or_default().to_string();
            let headers = wire_headers(&r["headers"]);
            if method == "GET" || method == "HEAD" {
                gets.push(format!("{method} {url}{headers}"));
            } else {
                writes.push(format!("{method} {url} {body}{headers}"));
            }
        }
    }
    gets.sort();
    let mask = |bytes: &[u8]| mask_home(&String::from_utf8_lossy(bytes), home);
    Outcome {
        code: out.status.code().unwrap_or(-1),
        out: mask_fetched(&mask(&out.stdout)),
        err: mask_fetched(&mask(&out.stderr)),
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
    let args: Vec<String> = args.iter().map(|a| unhome(a, home)).collect();
    let input = unhome(input, home);
    let mut cmd = Command::new(program[0]);
    cmd.args(&program[1..])
        .args(&args)
        .current_dir(work)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("FSV_CONFIG_DIR", home.join("config"))
        .env("FSV_CACHE_DIR", home.join("cache"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("FSV_NO_KEYCHAIN", "1")
        .env("FSV_QUIET", "1")
        .env("FSV_ALLOW_TENANT", DOMAIN)
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
    mask_epoch(&mask_stamps(&mask_json_fetched(&mask_sent_at(text))))
}

/// `"fetched_at": "2026-11-21T12:04:03+00:00"` in a review's metadata.
fn mask_json_fetched(text: &str) -> String {
    const MARK: &str = "\"fetched_at\": \"";
    const STAMP: usize = "2026-11-21T12:04:03+00:00".len();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(MARK) {
        let start = i + MARK.len();
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        if rest.get(..STAMP).is_some_and(|s| s.as_bytes()[10] == b'T') {
            out.push_str("<time>");
            rest = &rest[STAMP..];
        }
    }
    out.push_str(rest);
    out
}

/// Journal timestamps `2026-11-21T11:47:44+0000`.
fn mask_stamps(text: &str) -> String {
    const STAMP: usize = "2026-11-21T11:47:44+0000".len();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let shaped = i + STAMP <= text.len()
            && text.is_char_boundary(i)
            && text.is_char_boundary(i + STAMP)
            && bytes[i + 10] == b'T'
            && bytes[i + 19] == b'+'
            && bytes[i..i + 4].iter().all(u8::is_ascii_digit)
            && bytes[i + 20..i + STAMP].iter().all(u8::is_ascii_digit);
        if shaped {
            out.push_str("<ts>");
            i += STAMP;
        } else {
            let ch = text[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// `"epoch": 1795261665`.
fn mask_epoch(text: &str) -> String {
    const MARK: &str = "\"epoch\": ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(MARK) {
        let start = i + MARK.len();
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        out.push_str("<epoch>");
        rest = &rest[digits..];
    }
    out.push_str(rest);
    out
}

/// `was already sent at 2026-11-21T11:37:30+0000`: the journal's wall-clock
/// stamp, which can tick between the runs.
fn mask_sent_at(text: &str) -> String {
    const MARK: &str = "was already sent at ";
    const STAMP: usize = "2026-11-21T11:37:30+0000".len();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(MARK) {
        let start = i + MARK.len();
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        if rest.len() >= STAMP && rest.as_bytes().get(10) == Some(&b'T') {
            out.push_str("<time>");
            rest = &rest[STAMP..];
        }
    }
    out.push_str(rest);
    mask_fetched_stamp(&out)
}

fn mask_fetched_stamp(text: &str) -> String {
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
        diffs.push(format!(
            "exit: py {} rs {}\n  py err: {:?}\n  rs err: {:?}",
            a.code, b.code, a.err, b.err
        ));
    }
    if a.out != b.out {
        if std::env::var_os("FSV_DIFF_VERBOSE").is_some() {
            diffs.push(format!(
                "stdout differs:\n  py: {:?}\n  rs: {:?}",
                a.out, b.out
            ));
        } else {
            diffs.push("stdout differs".into());
        }
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
        .chain(load_env_cases())
        .filter(|c| {
            std::env::var("FSV_DIFF_TAG")
                .ok()
                .is_none_or(|t| c.tag.as_deref() == Some(t.as_str()))
        })
        .filter(|c| {
            only.as_deref()
                .is_none_or(|o| format!("{}: fsv {}", c.fixture, c.args.join(" ")).contains(o))
        })
        .collect();
    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut succeeded = 0;
    for case in &cases {
        let mut case = case.clone();
        let label = format!("{}: fsv {}", case.fixture, case.args.join(" "));
        if let Some(review) = case.hash_from.clone() {
            let probe = Case {
                args: review,
                stdin: None,
                hash_from: None,
                ..case.clone()
            };
            let home = tempfile::tempdir().unwrap();
            let work = prepare(home.path(), &probe);
            let out = run(
                &[python.as_str(), shim.as_str()],
                home.path(),
                &work,
                &probe,
            );
            let hash = serde_json::from_str::<Value>(&out.out)
                .ok()
                .and_then(|v| v["plan_hash"].as_str().map(str::to_string))
                .unwrap_or_else(|| format!("NO-HASH {}", out.err));
            for a in case
                .args
                .iter_mut()
                .chain(case.pre.iter_mut().flat_map(|s| s.args.iter_mut()))
            {
                *a = a.replace("@HASH@", &hash);
            }
        }
        let case = &case;
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
        if std::env::var_os("FSV_DIFF_EXITS").is_some() {
            println!("EXIT {} {label}", py.code);
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

/// `------fsv<32 hex>`: a multipart boundary, random per run.
fn mask_boundary(text: &str) -> String {
    const MARK: &str = "----fsv";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(MARK) {
        let start = i + MARK.len();
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let hex = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
        if hex >= 32 {
            out.push_str("<boundary>");
            rest = &rest[32..];
        }
    }
    out.push_str(rest);
    out
}

/// The request headers both clients set themselves. httpx adds `host`,
/// `user-agent`, `accept-encoding`, `connection` and `content-length` below
/// the transport; the Rust mock sees the request before reqwest does.
fn wire_headers(headers: &Value) -> String {
    const SKIP: [&str; 5] = [
        "host",
        "user-agent",
        "accept-encoding",
        "connection",
        "content-length",
    ];
    let Some(map) = headers.as_object() else {
        return String::new();
    };
    let mut rows: Vec<String> = map
        .iter()
        .filter(|(k, _)| !SKIP.contains(&k.as_str()))
        .map(|(k, v)| {
            let v = mask_boundary(v.as_str().unwrap_or_default());
            format!("{k}: {v}")
        })
        .collect();
    rows.sort();
    format!(" [{}]", rows.join("; "))
}
