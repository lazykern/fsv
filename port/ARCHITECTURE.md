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

Behavioral parity with the Python tool: same commands and flags, same exit
codes, same JSON/CSV/TSV output, same files on disk (config, session,
cache, journal, drafts) so an existing user's state keeps working, same
HTTP requests (paths, params, bodies, headers), same safety checks and
plan hashes. Restructure freely where Rust allows a cleaner design, but
never change observable behavior silently. Any deliberate divergence goes
into `port/PARITY.md` under "Divergences" with a reason.

## Crates (dependency order)

| crate | Python modules | notes |
|---|---|---|
| `fsv-py` | — | CPython-exact `json.dumps` (`canonical`, `Dumps`), `float_repr`, `py_str`/`py_repr`. Done. |
| `fsv-core` | `errors`, `config`, `cache`, `journal`, `session`, `client`, `resources`, `yamlio`, `mdlite`, `editor`, `safety`, `review` (table part), text helpers (`render.strip_html`, `render.fmt_dt`, `render.fmt_value`, `render.emit_json`, `render.emit_delimited`), terminal output (`rich` Console/markup/Table/rule replacement), prompts (`typer.prompt/confirm`) | infrastructure; no Freshservice domain logic beyond the HTTP client |
| `fsv-forms` | `forms/*` (`spec`, `rules`, `state`, `payload`, `plan`, `fetch`, `tz`, `errors`), `formui/fuzzy`, `formui/when` | pure form engine, no I/O; depends only on `fsv-py` |
| `fsv-api` | `schema`, `query`, `service`, `create`, `state_flow`, `task_status`, `change_tasks`, `change_approvals`, `change_approval_actions`, `change_associations`, `ticket_notes`, `formmeta`, `cache`-backed lookups, `completion.py` data helpers (`_cached_groups`, `_remote_user_values`, …) | Freshservice resource operations |
| `fsv-flows` | `drafts`, `change_flow`, `ticket_flow`, `catalog`, `cli_sr.render_sr_template` | form create/update orchestration (plan → review → send) |
| `fsv-views` | `views`, `pages`, `render` (tables/panels) | site-style list/detail loading and rendering |
| `fsv-formui` | `formui/session`, `formui/lookups`, `formui/widgets`, `formui/screens`, `formui/app` | ratatui form UI; `session` logic stays UI-agnostic |
| `fsv-tui` | `tui/app`, `tui/modals`, `tui/actions` | ratatui full-screen app |
| `fsv-completion` | `completion_gen`, `_complete`, completion parts of `completion.py` | shell scripts + `_FSV_COMPLETE` runtime protocol |
| `fsv-cli` (package `fsv`, binary `fsv`) | `cli`, `change_cli`, `ticket_cli`, `cli_sr`, `form_cli`, `__init__` | clap command tree, glue only |

Move a function to the lowest crate that can own it (e.g. the TUI imports
`cli_sr._attachment_rows`; that belongs in `fsv-views` or `fsv-api`).
Upstream crates must not depend on downstream ones.

## Libraries

Declared once in the root `Cargo.toml` `[workspace.dependencies]`; crates
use `foo.workspace = true`. Preferred choices:

- JSON: `serde_json` with `preserve_order` (dict insertion order matters).
  Server data stays `serde_json::Value`; use typed structs only where it
  clarifies code without losing unknown keys.
- Anything hashed or compared with Python output: `fsv_py::canonical` /
  `fsv_py::Dumps`. Never `serde_json::to_string` for those.
- HTTP: `reqwest` blocking + rustls, cookie handling done by hand (host
  scoped jar; parse `Set-Cookie`), redirects disabled by default.
- Regex: `regex`; `fancy-regex` only when backreferences/lookaround are needed.
- Time zones/dates: `jiff` (IANA tz). Python `zoneinfo` → `jiff::tz`.
- YAML: `yaml-rust2` event parser with our own scalar resolution (see
  `yamlio.py`: every plain scalar is a string except null and true/false).
- Crypto: `argon2` (Argon2id, v19), `aes-gcm`, `sha2`, `md-5`, `base64`.
- Terminal: `crossterm` (+ `ratatui` for full-screen UIs).
- CLI: `clap` derive.
- Errors: `thiserror`.

## Global state

Python keeps module globals (`config.DOMAIN`, `client._VERBOSE`, the
client singleton, `cli.NO_INPUT`). In Rust:

- `fsv_core::config` owns paths + domain in a process-wide `RwLock` with
  `config::domain()`, `config::paths()`, `config::reload()`,
  `config::set_domain()`. Paths are recomputed from env on `reload()` so
  tests can point `FSV_CONFIG_DIR`/`FSV_CACHE_DIR` at temp dirs.
- `fsv_core::client::get_client()` returns a shared `Arc<Client>`; client
  internals use interior mutability so the TUI can call from worker threads.
- Runtime flags (`verbose`, `no_input`) live in `fsv_core::runtime`.
- The current command path for journal entries (`ctx.command_path`, e.g.
  `"fsv changes create"`) is set by the CLI in `fsv_core::runtime`.

## Errors and exit codes

`fsv_core::Error` mirrors Python exception classes:

- `Api(ApiError { status, body: Value, method, target })` (`APIError`,
  same `Display` as Python `str()`, plus `detail()`),
- `Session(String)` (`SessionError`),
- `Value(String)` (`ValueError` and subclasses → exit 2),
- `Exit { code, message }` (flow exits such as `FlowExit`),
- `Other(String)` (exit 1), `Io`, `Aborted` (Ctrl-C / refused prompt).

`Error::exit_code()` reproduces `errors.exit_code_for` (0 ok, 1 other,
2 usage, 3 session, 4 conflict, 5 rejected). Domain crates may define
their own error enums but must convert into `fsv_core::Error` preserving
the exit-code class (Python `__cause__` chains → wrap the source).

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
- Plain `console.print` text is not hard-wrapped (deliberate divergence,
  Rich soft-wraps at terminal width).

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
- Journal: `$XDG_STATE_HOME/fsv/journals/<sha256(resolved config dir)>/<sha256(domain)|unknown>/journal.jsonl`.
- Atomic writes: temp file in the same directory + rename, mode 0600.
- Session stores: `file` (plain JSON), `argon` (Argon2id t=3 m=65536 p=4,
  32-byte key, AES-256-GCM with the domain as AAD, JSON envelope),
  `keychain` (macOS `security` CLI, base64 payload, service `fsv`,
  account `fsv-session`).

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

## Building while other agents work

Several agents build the workspace at once. Always build with your own
target dir to avoid lock contention, and build only your crate:

    CARGO_TARGET_DIR=/Users/phusit.somb/lab/work/fsv-rust/target/<crate> cargo test -p <crate>

Only edit files inside the crate(s) you own plus (if needed) new entries
in the root `[workspace.dependencies]`. Never run `git commit`, `git
stash`, `git checkout` or `cargo fmt` at the workspace level; format only
your crate (`cargo fmt -p <crate>`).

## Parity ledger

Each crate has `port/parity/<crate>.md` mapping every Python module and
public function it owns to the Rust counterpart and status (`ported`,
`partial`, `missing`), plus ported tests, golden fixtures, known gaps and
divergences. Write only your own crate's file. `port/PARITY.md` is the
index maintained by the lead.
