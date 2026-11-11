# Parity ledger

Each section: Python module → Rust location, status, ported tests, golden
fixtures, known gaps. Statuses: `ported`, `partial`, `missing`.

## Divergences

Fixes of bad Python behavior, plus a few platform differences. The
end-of-port review reads this list. Wire formats (requests, plan hashes,
journal kinds, cache filenames, session bytes, draft keys) are not on it:
those stay identical.

- Files are written with LF on every platform. Python on Windows writes CRLF.
- YAML parse errors say `YAML error: …` but the line wording is not PyYAML's.
- Human terminal output matches content, order, wording and columns. Box
  glyphs, Rich number/URL highlighting and `:emoji:` substitution may differ.
- Click prompt abort prints `Aborted!`. `_err_exc` prints `Aborted.`
- No Windows Credential Manager backend. Windows defaults to `file`, as Python does.
- `keepalive()` is ported and still has no caller.
- `fsv cache clear views` deletes the cache under `CACHE_DIR/filters`.
  Python deletes `CONFIG_DIR/filters` and leaves the views cache in place.
  No test asserts the wrong directory. The command's help says it clears
  the cache.
- Commands that crash with a Python traceback (no domain on `url` and
  `changes associations`, `form explain --id` with a bad id, `completion
  show powershell`) exit 1 with the same message and no traceback. The
  exit code stays.
- The TUI command palette (`ctrl+p`) is not ported. It is Textual's, not
  an fsv screen. Filter history still uses `ctrl+p` / `ctrl+n`.
- A failed search toasts the error. In Python the worker's uncaught
  exception exits the whole app.
- Keys typed in an open form do not reach the main TUI's key handler.
- The description preview keeps focus, so F3 toggles it off again. In
  Textual, hiding the focused editor moves focus away and F3 stops working.
- The status bar is a row above the key hints. Textual draws the hints on
  top of the status bar.
- TUI workers do not drop stale generations, and there is no change-emit
  lock. Python's other staleness checks are kept. A slow reply can still
  land after a newer load.
- `detail_panel` does not wrap a long planning line at 60 columns. Rich
  wraps it. The Rust block leaves wrapping to the console.
- An unquoted YAML key `true:` loads as `"true"`. Python gives `"True"`.
  The loader in `fsv-core` cannot tell a bool key from a quoted one after
  parse. `fsv-flows` golden draft cases remap that one key.
- Read-only notes in a change plan follow the form's visible-field order.
  Python iterates a set, so the order is not stable.

## fsv-api (started)

| Python | Rust | status |
|---|---|---|
| `query.py` filter → `query_hash`, date merge, one-shot percent-decode | `fsv_api::query` | ported (the four edge-case tests). Schema loading and the HTTP directory wait on `fsv-core`. `query._resolve_group` has no caller; `cli._resolve_group` is the one that is used. |
| `task_status.py` | `fsv_api::task_status` | ported (tenant ids, fail-closed choices, no default id). `change_tasks.build_plan` still to come. |
| `schema.py` choice labels, priority/impact/risk, v2 merge | `fsv_api::schema` | ported for the pure helpers. Cache TTL constant is present; the on-disk schema cache still waits on the client. |
| `views.py` dates, html, span, relative | `fsv_api::site::format` | ported. `span(23h 59.67m)` is `24h` (Python does not carry that into `1d`). List items keep the blank line `</li>` inserts. |
| `views.py` saved views, columns, cells, headers, tabs | `fsv_api::site` list | ported, no HTTP. `find_view` / `list_meta` / later-page headers match the Python tests. |
| `views.py` lifecycle, planning bind, flat properties | `fsv_api::site` detail | ported as pure JSON in, struct out. `match_planning_row` keeps the Python error sentences. |
| `views.py` record bits (source, approval, conversations, late badge) | `fsv_api::site::record` | ported. |
| `views.collect_pages` | `fsv_api::site::collect_pages` | ported as a walker the caller feeds. Page-limit, short-page `count`, and foreign `next_page_url` match `test_read_completeness.py`. Incomplete meta inserts `continuation` before `partial`, matching Python's update-then-assign order. |

## fsv-views

| Python | Rust | status |
|---|---|---|
| `pages.py`, `render.py` | `fsv_views::pages`, `fsv_views::render` | ported. `cargo test -p fsv-views --offline` was 87 passed. Expected strings came from running the Python functions. |
| `views.py` loaders (`load_tab`, `load_detail`, conversations, requested items) | `fsv_views::detail`, `fsv_views::source` | ported, covered from `coverage`. |

Known test limits, not behavior changes: relative times use the real clock, so tests check placement rather than a pinned "2 days ago". `_LazyModule` and `_NullConsole` are import-time shims with no Rust counterpart. `print_blocks` and `print_detail` write to the terminal; tests cover `blocks_text` and `detail_blocks`.

## fsv-cli

| Python | Rust | status |
|---|---|---|
| `cli.py` command tree (111 nodes: root, 13 groups, 97 leaves) | `fsv_cli::generated`, `app` | ported. `tests/tree.rs` checks every node's name, hidden flag, options, positionals and help text against a Python dump. |
| `cli.py` `help`, `search`, `auth`, `cache`, `journal`, `config`, `completion`, `views`, `url`, `<res> lookup` | `fsv_cli::cmd::*` | ported. |
| `cli.py` `tickets add-note` (reviewed, no `--set-status`), `changes tasks-delete --dry-run`, `changes update --due-by --dry-run` | `cmd::tickets`, `cmd::changes` | ported. |
| `<res> ls/get/activity/tasks/notes/fields`, `update`, `download`, `tickets reply/create/conversations/approvals/associations`, `changes create/update` (form path), `changes tasks-add/-update/approvals/state/clone/assets/associations`, `sr *`, `form *`, `tui`, `setup` | `fsv_cli::pending` | **not ported.** Each runs its local checks, prints `error: X is not ported to Rust yet`, exits 1. Blocked on schema, views, formmeta, catalog and change-flow wiring in `fsv-api`/`fsv-flows`/`fsv-views`. |
| `gen_tree.py` (generator for `generated.rs`) | none | missing from disk. `generated.rs` was hand-edited after generation: module renamed to `generated` (`gen` is reserved in edition 2024), struct doc comments removed, hint fields fixed. Regenerating needs the script rewritten. |

CLI divergences:
- Usage errors use clap's text. Exit code 2 matches Click.
- Completion scripts call `FSV_THIN_COMPLETE=1 fsv _complete` in place of `python -m fsv._complete`.
- `FSV_COMPLETION_NETWORK=1` gates live completers. It is a new env switch, read only in `cmd/completion.rs`.
- `--backup-replaced` carries a `value_flag` override so completion treats it as taking a value, as Python's generator does.

## fsv-flows

| Python | Rust | status |
|---|---|---|
| `drafts.py`, `change_flow.py`, `ticket_flow.py` | `fsv_flows` | ported. `cargo test -p fsv-flows --offline` was 70 passed. Create (10), update (5), incident (4) and draft parses match `tests/golden/flows.json` from `gen_flows.py`. |
| `formmeta.py` cache and workspace discovery | `fsv_flows::meta` | partial. Only the pure helpers the flows call. Callers pass metadata through `MetaSource`. |
| change execute HTTP | `change::ChangeWriter` | partial. Upload, POST, PUT, attach and planning sit behind the trait. Tests use a recording fake. No live client yet. |

`interactive()` and `editor_edit` are smoke-only. The `~/a.pdf` draft case is skipped unless home is `/Users/phusit.somb`, because the golden file stores the expanded path. `DiskJournal` wraps `fsv_core::journal`; tests use an in-memory journal.

## fsv-py

| Python behavior | Rust | status |
|---|---|---|
| `json.dumps(...)` all option combos used by fsv | `fsv_py::Dumps`, `canonical`, `compact`, `pretty` | ported (unit tests vs CPython output) |
| `float.__repr__` | `fsv_py::float_repr` | ported |
| `str()` / `repr()` of JSON values | `fsv_py::py_str`, `py_repr`, `py_repr_str` | ported |

## fsv-tui

| Python | Rust | status |
|---|---|---|
| `tui/app.py`, `actions.py`, `modals.py` | `fsv_tui` | partial. List, detail, filter, search, and write modals run against a `Backend` trait. `cargo test -p fsv-tui` was 22 passed (16 unit, 6 `TestBackend`). No live HTTP backend yet, so `fsv tui` still needs a `Backend` passed to `launch`. Opening a form toasts that the form UI is not available until `fsv-formui` is connected. Page text goes through a local `Pages` trait (`BasicPages`), not `fsv-views`, which already has the real renderers. |
