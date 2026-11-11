# fsv Rust port — architecture and conventions

Reference implementation: the Python package at
`/Users/phusit.somb/lab/work/fsv/src/fsv` (main worktree, including its
uncommitted files such as `task_status.py`). Python tests live in
`/Users/phusit.somb/lab/work/fsv/tests`. Run Python with
`cd /Users/phusit.somb/lab/work/fsv && uv run python ...` (read-only: never
edit the Python tree).

The Rust workspace lives at `/Users/phusit.somb/lab/work/fsv-rust`
(git branch `rust-port`). The full CLI surface (every command, option,
default and help string) is in `port/cli-surface.txt`.

## Goal

Behavioral parity with the Python tool: same commands and flags, same
JSON/CSV/TSV output, same files on disk (config, session, cache, journal,
drafts) so an existing user's state keeps working, same HTTP requests
(paths, params, bodies, headers), same safety checks and plan hashes.

Fix the Python tool where it is wrong. Dead code stays unported. A crash,
a swallowed error, a command that writes the wrong directory, an exit code
that disagrees with the README table, or a key that cannot be pressed again
gets the behavior a user would expect. Record every such fix in
`port/PARITY.md` under "Divergences": what Python does, what Rust does, why.
That list is the end-of-port review.

Do not "fix" wire formats. Request paths, bodies, plan hashes, journal
kinds, cache filenames, session bytes and draft YAML keys stay identical,
including when the Python shape looks ugly. A hash change silently disables
duplicate-send protection.

## Crates (dependency order)

| crate | Python modules | notes |
|---|---|---|
| `fsv-py` | — | CPython-exact `json.dumps` (`canonical`, `Dumps`), `float_repr`, `py_str`/`py_repr`. Done. |
| `fsv-core` | `errors`, `config`, `cache`, `journal`, `session`, `client`, `resources`, `yamlio`, `mdlite`, `editor`, `safety`, `review` (table part), text helpers (`render.strip_html`, `render.fmt_dt`, `render.fmt_value`, `render.emit_json`, `render.emit_delimited`), terminal output (`rich` Console/markup/Table/rule replacement), prompts (`typer.prompt/confirm`) | infrastructure; no Freshservice domain logic beyond the HTTP client |
| `fsv-forms` | `forms/*` (`spec`, `rules`, `state`, `payload`, `plan`, `fetch`, `tz`, `errors`), `formui/fuzzy`, `formui/when` | pure form engine, no I/O; depends only on `fsv-py` |
| `fsv-api` | `schema`, `query`, `service`, `create`, `views` (as `site`: loaders only), `state_flow` fetch half, `task_status`, `change_tasks`, `change_approvals`, `change_approval_actions`, `change_associations`, `ticket_notes`, `formmeta`, `attachments` (`cli_sr._attachment_rows`/`_download`, `create._attachment_url`, `catalog.is_file_value`/`safe_target`/`domain_url`), completion data helpers | Freshservice operations. `views.py` lives here because `change_tasks`, approvals, associations and `ticket_notes` call it and it calls them back. |
| `fsv-flows` | `drafts`, `change_flow`, `ticket_flow`, `catalog`, `cli_sr.render_sr_template` | plan → review → send. Reuses `fsv_api::attachments`. |
| `fsv-views` | `pages`, `render` (Rich tables/panels), `state_flow` render half (`render_flow`, `render_compact`, `render_state_diagram`) | markup strings only. Depends on fsv-api, not on fsv-flows. |
| `fsv-formui` | `formui/session`, `formui/lookups`, `formui/widgets`, `formui/screens`, `formui/app` | ratatui form UI; `session` logic stays UI-agnostic. Depends on fsv-core, fsv-forms only; lookups go through a seam trait. |
| `fsv-tui` | `tui/app`, `tui/modals`, `tui/actions` | ratatui. Depends on fsv-api and fsv-core only; every network action goes through the `Backend` trait. |
| `fsv-completion` | `completion_gen`, `_complete` | takes `&clap::Command` from fsv-cli. Depends on fsv-core, fsv-forms; must not depend on fsv-cli. |
| `fsv-app` | — | live adapters for every seam trait (TUI `Backend`, formui lookups, completion data, flow journals). The only crate that wires `fsv_core::Client` into the UIs. |
| `fsv-cli` (package `fsv`, binary `fsv`) | `cli`, `change_cli`, `ticket_cli`, `cli_sr`, `form_cli`, `__init__` | clap command tree, glue only |

Real edges (every crate also depends on fsv-py):

| crate | depends on |
|---|---|
| fsv-core | — |
| fsv-forms | — |
| fsv-api | core, forms |
| fsv-completion | core, forms |
| fsv-formui | core, forms |
| fsv-flows | api, core, forms |
| fsv-views | api, core, forms |
| fsv-tui | api, core |
| fsv-app | api, completion, core, flows, forms, formui, tui, views |
| fsv-cli | api, app, completion, core, flows, forms, formui, views |

```
fsv-py ← fsv-core ─┬─ fsv-api ─┬─ fsv-flows ─┐
fsv-py ← fsv-forms ┤           ├─ fsv-views ─┤
                   │           └─ fsv-tui ───┤
                   ├─ fsv-formui ────────────┼─ fsv-app ← fsv-cli
                   └─ fsv-completion ────────┘
```

fsv-core and fsv-forms are independent; form errors reach the CLI as
`Error::Value`. UI crates never name a concrete HTTP client. Full rationale:
`port/specs/design-review.md`.

## Libraries

Declared once in the root `Cargo.toml` `[workspace.dependencies]`; crates
use `foo.workspace = true`. Preferred choices:

- JSON: `serde_json` with `preserve_order` (dict insertion order matters).
  Server records stay `Value`. Do not use `#[serde(flatten)]` on anything
  re-emitted or hashed: it reorders keys. Do not enable `arbitrary_precision`.
  Type only Python dataclasses and on-disk envelopes (cache, journal, session,
  argon), with field order equal to Python's dict order. `config.json` stays
  `Value` (unknown keys survive; written `sort_keys=True`).
- Python truthiness (`x.get(k) or {}`) goes through `fsv_py::ValueExt`
  (`truthy`, `get_truthy`, `obj_or_empty`, `list_or_empty`, `py_get_str`).
  Hand-rolled `.get().and_then(Value::as_object)` treats `""`/`0`/`false`
  differently from Python and is a parity bug.
- Anything hashed or compared with Python output: `fsv_py::canonical` /
  `fsv_py::Dumps`, with the flags named at the call site. There is no shared
  "write JSON" helper. `config.json`: indent 2, sort_keys, ensure_ascii.
  Cache: indent 2, ensure_ascii false. Journal: default separators,
  ensure_ascii false. Session payload: default separators, ensure_ascii true.
  `plan_hash`: sort_keys, separators `(",", ":")`, ensure_ascii false.
- HTTP: blocking `reqwest` + rustls. `Client: Send + Sync`, shared as
  `Arc<Client>`; cookie jar and CSRF behind a `Mutex`; no `&mut self`.
  httpx `timeout=30` is per phase, so set connect and read timeouts to 30s
  and never a 30s total deadline (uploads and downloads exceed it).
  Redirects are a manual loop (max 20) with per-request `follow`: cookie jar
  updates on every hop, and Freshservice cookies go only to the tenant host.
  `Client::fetch_raw(url, follow)` is what the download call sites use
  (no retry, no journal, no status mapping). TUI workers do not cancel
  in-flight HTTP; staleness uses Python's own guards (`pending_detail_key`,
  `list_epoch`, the search sequence). `keepalive()` has no Python caller; port it
  but do not build the TUI around it. The transport is a trait so tests
  inject a mock that records `gets`, `writes` and `paths` (the Acme and
  Globex tenants). The real transport is reqwest.
- Regex: `regex`; `fancy-regex` only when backreferences/lookaround are needed.
- Time zones/dates: `jiff` (IANA tz). Python `zoneinfo` → `jiff::tz`.
- YAML: `yaml-rust2` with our own plain-scalar resolution. The contract
  (verified against Python) is in `port/specs/design-review.md` item 8:
  no merge keys, duplicate keys last-wins at the first position, explicit
  `!!str`/`!!int`/`!!float`/`!!bool`/`!!null` tags honored, aliases copied,
  a single document, typed keys stringified with `fsv_py::py_str`. Error
  wording may differ from PyYAML; exit code stays 2.
- Crypto: `argon2` (Argon2id, v19), `aes-gcm`, `sha2`, `md-5`, `base64`.
- Terminal: `crossterm` (+ `ratatui` for full-screen UIs).
- CLI: `clap` derive.
- Errors: `thiserror`.

## Context, not process globals

Library crates never read the environment and never a process-wide config.
`cargo test` runs tests in parallel, and edition 2024 makes `env::set_var`
unsafe. Tests build a `Ctx`; they do not rebind the process.

```rust
pub struct Paths { pub config_dir: PathBuf, pub cache_dir: PathBuf, pub state_home: PathBuf }
impl Paths {
    pub fn from_env() -> Paths;     // only reader of FSV_CONFIG_DIR, FSV_CACHE_DIR, XDG_STATE_HOME, HOME
    pub fn under(root: &Path) -> Paths;
}
pub struct Tenant { pub domain: String, pub api_v2: String, pub api_int: String, pub login_url: String }
pub struct Flags { pub verbose: bool, pub no_input: bool }
pub struct Ctx { pub paths: Paths, pub tenant: Option<Tenant>, pub flags: Flags, pub command_path: Option<String> }
```

`journal`, `session` and `config` take `&Ctx` or `&Paths`. `cache::load` /
`save` take a path the caller computed from `ctx.paths`. `Client` holds
`Arc<Ctx>` and is `Send + Sync`. `fsv auth login --domain` builds a new
`Ctx` and a new `Client`; it does not mutate the old one.

`fsv_core::runtime::install(Arc<Ctx>)` (`OnceLock`) exists for the binary
edge only: fsv-cli `main`, and fsv-tui / fsv-completion when they have no
`Ctx` in hand. Library code does not call it.

Session `update_cookie` is an unlocked read-modify-write in Python. Rust
TUI workers overlap, so the session store takes one process mutex.

## Errors and exit codes

`exit_code_for` walks `__cause__ or __context__`. `raise X from None` hides
the cause from `str()` but the context still decides the exit code: a
`TaskError` raised inside `except APIError(409)` exits 4, inside
`except APIError(500)` exits 5. `ValueError` is checked before the context,
so a usage error exits 2 even when an `APIError` was swallowed.
`Display` never includes the context. `--verbose` appends `APIError.detail()`.

```rust
pub enum Error {                                 // crates/fsv-core/src/error.rs
    Api(ApiError),                               // 409 or duplicate regex → 4, else 5
    Session(String),                             // 3
    Value(String),                               // ValueError family → 2 (FormError lands here)
    Exit { code: i32, message: String },         // FlowExit / typer.Exit; empty = already printed
    EditorAbort(String),
    Http { kind: String, message: String },      // httpx.TransportError; kind = Python class name
    Other(String),                               // RuntimeError etc. → 1
    Io(std::io::Error),
    Aborted,                                     // click Abort prints "Aborted!"
    Context { message: String, source: Box<Error> }, // `raise X(message) from source`
}
impl Error {
    pub fn exit_code(&self) -> i32;
    pub fn context(self, message: impl Into<String>) -> Error;
}
```

An empty message falls back to `kind` (`str(exc) or type(exc).__name__`).
Domain crates may keep a typed error internally, but every Python
`except APIError: raise X(...) from None` becomes `api_err.context(msg)`.
Partial-send failures (`created_id`, `uploaded`, `steps`) are typed in the
domain crates (`fsv_api::catalog::SendFailure`, fsv-app `SendError`).

## Terminal output

`pages.py` returns Rich markup that both the CLI and the TUI render.
Call sites keep markup strings (~450 of them). fsv-core owns the only parser:

```rust
pub struct Span { pub text: String, pub style: Style }
pub fn parse(markup: &str) -> Vec<Vec<Span>>;
pub fn escape(s: &str) -> String;   // byte-exact rich.markup.escape
pub fn plain(markup: &str) -> String;
```

The CLI turns spans into ANSI (TTY, no `NO_COLOR`) or plain text. fsv-tui
maps `Span` to ratatui in one adapter; fsv-core does not depend on ratatui.
`term::Table` cells are markup, measured with `plain()` and `unicode-width`,
folded like Rich `overflow="fold"`. A stray closing tag is rendered literally
(Rich would raise). `err` is dim by default.

`term::prompt` / `term::confirm` match typer: `Text [default]: `, `Text [y/N]: `,
re-prompt on invalid, abort → exit 1.

Byte-exact parity is required for `--json`, `-o json|csv|tsv`, YAML drafts
and plan output. Human tables match content, order, wording and columns.
Box glyphs, Rich auto-highlight and `:emoji:` substitution may differ.

## Terminal output

Python uses `rich`. Replace with `fsv_core::term`:

- `term::out()` / `term::err()` print strings containing Rich markup
  (`[red]error[/red]`, `[bold cyan]…[/]`, `[dim]`); markup becomes ANSI
  when the stream is a TTY and `NO_COLOR` is unset, otherwise it is
  stripped. `err` is dim by default (Python `Console(stderr=True, style="dim")`).
- `term::escape()` ≙ `rich.markup.escape`.
- `term::Table` ≙ `rich.table.Table` (title, columns with style/justify/
  no_wrap, rows, Rich's default heavy-head box), sized to the terminal
  width (80 when not a TTY, like Rich).
- `term::rule(title)` ≙ `console.rule`.
- `term::prompt` / `term::confirm` ≙ `typer.prompt` / `typer.confirm`
  (same prompt text, `[default]` rendering, `(y/N)` suffixes, abort
  semantics → `Error::Aborted` / exit 1 with "Aborted!").
- `Console::print` word-wraps at the console width like Rich;
  `Console::print_soft` matches Rich's `soft_wrap=True`. `render()` does not wrap.

Exact byte parity is required for machine formats (`--json`, `-o json`,
csv, tsv, YAML drafts, `--print-plan` style outputs). Human tables and
panels should match Python's content, order and wording; box-drawing
details may differ slightly.

## Files on disk (must stay compatible both ways)

- Config: `$FSV_CONFIG_DIR` or `~/.config/fsv` (all platforms, not the
  platform config dir): `config.json` (indent=2, sort_keys, trailing
  newline), `session.json`, `store`, `hints.yaml`, presets.
- Cache: `$FSV_CACHE_DIR` or `~/.cache/fsv`: `schema/<domain>--<name>.json`,
  `filters/…`, `groups--<domain>.json`, `lookup/<md5>.json`, envelope
  `{_kind, saved_at, checked_at, changed_at, hash, etag, latency_ms,
  fetch_count, data}` written with `json.dumps(indent=2, ensure_ascii=False)`.
- Journal: `$XDG_STATE_HOME/fsv/journals/<sha256(py_resolve(config dir))>/<sha256(domain)|unknown>/journal.jsonl`.
  `py_resolve` matches `Path.resolve()`: non-strict (the directory need not
  exist), symlinks resolved (`/var` → `/private/var` on macOS), and no
  Windows `\\?\` prefix. `std::fs::canonicalize` is wrong here and would
  split the journal from Python's, which disables duplicate-send protection.
  Use the `dunce` crate plus a non-strict walk. Journal `kind`, plan action
  and step strings are load-bearing; keep them as constants in one module
  (the list is in design-review item 9).
- Atomic writes: temp file in the same directory + rename, mode 0600.
  Write LF on every platform. Python on Windows writes CRLF; readers accept
  both. This is a recorded divergence.
- Session stores, exactly Python's set and no others: `file` (plain JSON),
  `argon` (Argon2id t=3 m=65536 p=4, 32-byte key, AES-256-GCM, domain as
  AAD), `keychain` (macOS `security -i` on stdin, base64 payload, legacy
  raw JSON still reads, service `fsv`, account `fsv-session`). Unknown
  `store` values are ignored. Default: `session.json` exists → `file`,
  else darwin and not `FSV_NO_KEYCHAIN` → `keychain`, else `file`. No
  Windows Credential Manager in this port: writing `keychain` there makes
  the Python build shell out to `security` and fail.

Timestamps written by Python with `time.time()` are floats; write floats
too (`saved_at` etc.) so both versions read each other's files.

## Code style

- Idiomatic Rust; no `unwrap()` on I/O or server data. `cargo clippy
  --all-targets -- -D warnings` clean, `cargo fmt`.
- Comments only for constraints the code cannot show (e.g. "Freshservice
  returns 403 for expired sessions on this endpoint"). Port Python
  docstrings that state such constraints; drop narration.
- Names: Rust conventions; singular/plural fixed where Python was
  inconsistent, but CLI command names and flags stay exactly as Python.
- Tests: port the Python tests that cover the module (in
  `/Users/phusit.somb/lab/work/fsv/tests`) as Rust unit/integration
  tests. For pure functions, generate golden fixtures by running the
  Python implementation (`uv run python`) and assert the Rust output
  matches; commit the fixture generator script under `port/golden/`.
  Early oracles that tests cite as `/tmp/fsv_oracle/*`, `/tmp/fsv_port/*`
  and `/tmp/fsv_api_views/*` live in `port/golden/oracles/<same dir>/`.
  Run them from the Python repo with that directory on `PYTHONPATH`
  (`fsv_oracle` scripts import `common`). They still write scratch files
  under `/tmp`, so the paths baked into the fixtures stay stable.

## Building while other agents work

Several agents build the workspace at once. Always build with your own
target dir to avoid lock contention, and build only your crate:

    CARGO_TARGET_DIR=/Users/phusit.somb/lab/work/fsv-rust/target/<crate> cargo test -p <crate>

Only edit files inside the crate(s) you own plus (if needed) new entries
in the root `[workspace.dependencies]`. Never run `git commit`, `git
stash`, `git checkout` or `cargo fmt` at the workspace level; format only
your crate (`cargo fmt -p <crate>`).

## CLI contract

`port/specs/cli.md` is the command spec (handler, calls, prompts, output,
exit code, test file). The clap tree must reproduce these typer quirks;
`global = true` is not a substitute:

- Bare `fsv` and bare groups print help to stdout and exit 2. `fsv -v` and
  `fsv --no-input` print help and exit 0. `-h`/`--help` exit 0.
- `-v`/`--verbose` after a subcommand is moved to the front before parsing,
  except after `--` and except when it is an option value (`--body -v`).
  `--no-input` and `-V` are not moved. `--version` is only valid before the
  subcommand and prints `fsv 2026.5.29` to stdout.
- Uncaught Ctrl-C exits 130 with no message. A refused prompt prints
  `Aborted.` and exits 1.
- `fsv changes assets CHN-1 …` and `associations` rewrite to the hidden
  `legacy` command and print the deprecation warning.
- `--expect-plan`: create/update/`sr create` accept a prefix of at least 12
  hex digits. tasks-add, the approval commands, associations and ticket
  add-note require the full hash when `--yes` is set.
- Root `--no-input` is ignored by tasks-add, the approval commands, ticket
  add-note and `sr create`. `auth status` with no session exits 2; other
  commands exit 3. `tickets create` maps a form error to exit 1; `changes
  create`/`update` map it to 2.
- Seven JSON writers differ in indent, `ensure_ascii`, internal-key stripping
  and the trailing newline. Name the flags at each call site. CSV/TSV use
  `\r\n`, write `None` as empty, and print nothing when there are no rows
  (global search still prints the header).
- Do not port the dead code listed in cli.md §5. `changes update` reaches
  the form engine for `--set`/`--edit`/`--from`/`--preset`; the quick path
  only handles status, priority, agent, group and `--attach`.

`cache clear views` deletes `CONFIG_DIR/filters` in Python, so the views
cache is never cleared. Rust clears `CACHE_DIR/filters`. No test asserts
the wrong directory. Recorded as a divergence.

## TUI contract

`port/specs/tui.md` is the form UI and main TUI spec: screens, keys,
workers, theme colors, and which Python test pins which behavior.

- `fsv-formui` owns a `kit` (event loop, focus, modal stack, theme, widgets)
  that `fsv-tui` reuses. Split it into its own crate only if a third
  consumer appears.
- Widgets: `ratatui-textarea` 0.9.4 for multi-line text. Do not use
  `tui-textarea` (stuck on ratatui 0.29). `tui-markdown` 0.3.13 for the F3
  preview. `tui-input` is optional. Popup, scroll-view and widget-list
  crates are skipped; the kit draws those itself. Clipboard copy uses
  crossterm `osc52`. `y` on macOS still shells out to `pbcopy`.
- Key order is Textual's: priority bindings, focused widget, ancestors,
  screen, then the app handler. A `g` prefix sits on the app handler.
  Keys from a pushed form never reach the main app handler.
- Workers are threads plus a generation counter and the same epoch checks
  Python uses. A worker error toasts. It does not exit the process.
- Combobox: remote people search debounces 0.25s and drops a late answer
  whose query is no longer current. Enter never picks a result from an
  earlier search. Typed person text is not a committed value.
- Field commits debounce 0.3s or happen on blur. Setting a value from code
  does not emit a change. An unreadable date stays in the box and clears
  the plan hash until it is fixed.
- Write flows (task, link, note, download) are an enum of continuations.
  None of them retry a send.
- Tests use ratatui `TestBackend` with an inline executor and a manual
  clock: `press("ctrl+s")`, `settle()` drains due timers and workers.
  Snapshots are `insta` and stay few; assertions carry the parity.
- The status bar is its own row above the key hints. Textual docks both to
  the bottom, so the hints hide the status. Showing both is the divergence.

## Design review

Decisions that are expensive to undo (crate boundaries, client sync vs async,
error types, on-disk formats, public APIs other crates will bind to) get a
second opinion from an advisor agent before the next wave of crates is
written. Verdicts land in `port/specs/design-review.md`; accepted changes are
folded back into this file. Do not relitigate a settled verdict without new
evidence.

## Parity ledger

Each crate has `port/parity/<crate>.md` mapping every Python module and
public function it owns to the Rust counterpart and status (`ported`,
`partial`, `missing`), plus ported tests, golden fixtures, known gaps and
divergences. Write only your own crate's file. `port/PARITY.md` is the
index maintained by the lead.
