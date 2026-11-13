# fsv

Freshservice from your terminal: browse records and saved views, inspect forms, and review supported writes. fsv uses a browser session rather than an API key. It uses internal website endpoints as well as some public APIs; internal endpoints can change without notice.

## Getting started

Requires Python 3.12+ and [uv](https://docs.astral.sh/uv/).

```bash
uv tool install git+https://github.com/lazykern/fsv.git
fsv setup
# Alternatively, sign in directly:
fsv auth login --browser --domain acme.freshservice.com
fsv auth status
fsv config list
fsv tickets ls --where status=Open --json
fsv changes get CHN-1234 --json
fsv tui --read-only
```

Complete SSO or MFA in the browser. The wizard configures the domain and login; `fsv config get KEY` and `fsv config set KEY VALUE` manage settings. Browser login uses a temporary browser profile; it does not read your existing profile. Set `FSV_BROWSER` if the browser cannot be found. Alternatively, `fsv auth login --domain acme.freshservice.com` prompts for a Cookie header. Treat it as a password; avoid passing it on the command line. The default session backend is Keychain on macOS and a mode-0600 file elsewhere; `--store argon` uses an encrypted file.

For a supported reviewed write, first inspect the plan, then send **the same arguments** with its complete hash:

```bash
fsv tickets add-note INC-1234 --body 'Investigation started' --dry-run --json
# Inspect the full plan and copy its plan_hash, then substitute it for FULL_HASH:
fsv tickets add-note INC-1234 --body 'Investigation started' --yes --expect-plan FULL_HASH --json
```

`FULL_HASH` is a placeholder, not a literal value. Review the target, payload, recipients and effects; a changed plan requires a new review. Some older form flows accept a hash prefix, but using the full hash works across reviewed workflows. Interactive confirmation and supported flags vary by command: check its `-h`. `--json` is available on many, not all, commands; list commands also support `--output` formats. Use `--refresh` where offered, for example `fsv tickets fields --refresh`, to refresh cached schema. `fsv cache status` and `fsv cache refresh` inspect and refresh stale caches; see [views](docs/views.md) for list behavior.

## Command surface

Run `fsv -h`, `fsv COMMAND -h`, or `fsv COMMAND SUBCOMMAND -h` for arguments and options. In particular, use `fsv tickets --help`, `fsv sr --help`, `fsv problems --help`, `fsv changes --help`, `fsv assets --help`, `fsv solutions --help`, `fsv attachments --help`, and `fsv form --help`. The inventory below follows the CLI help; account permissions and metadata determine which operations are available. See the [command cheat sheet](docs/commands.md) for examples and the [documentation index](docs/index.md) for detailed guides.

- General: `help`, `search`, `setup`, `tui`; `auth` (`login`, `logout`, `status`); `config` (`list`, `get`, `set`, `defaults show/set/clear`); `cache` (`status`, `refresh`, `clear`); `journal` (`ls`, `tail`, `path`); `completion` (`show`, `install`, `refresh`, `doctor`).
- `tickets`: `ls`, `get`, `activity`, `export`, `fields`, `lookup`, `views`, `search`, `url`, `create`, `update`, `download`, `conversations`, `approvals`, `associations`, `add-note`, `note-edit`, `note-delete`, `reply`, `forward`, `assign`, `pick-up`, `merge`, `resolve`, `close`, `reopen`, `status`, `bulk-priority`; nested `assets`, `columns`, `watchers`, `tasks`, `private-views`, `canned`, `scenarios`, and `time`.
- `sr`: `catalog`, `fields`, `create`, `get`; nested `assets` and `approvals` (`list`, `request`, `approve`, `reject`, `cancel`, `remind`).
- `problems`: `ls`, `get`, `activity`, `tasks`, `export`, `fields`, `lookup`, `views`, `search`, `url`, `create`, `update`, `download`, `notes`, `add-note`, `note`, `status`, `link-incident`, `unlink-incident`, `link-solution`, `unlink-solution`, `mark-known-error`, `planning-text`, `write-child`; nested `assets` and `columns`.
- `changes`: `ls`, `get`, `activity`, `tasks`, `tasks-add`, `tasks-update`, `tasks-delete`, `approvals`, `approval-request`, `approval-respond`, `approval-cancel`, `state`, `export`, `asset-picker`, `planning-text`, `fields`, `lookup`, `views`, `search`, `url`, `update`, `download`, `clone`, `create`, `notes`, `add-note`; nested `associations`, `assets`, `columns`, and `watchers`.
- `assets search` searches integrated inventory; `solutions create` reviews draft knowledge articles; `attachments delete` reviews deletion with an explicit owner.
- `form fields`, `explain`, `dump`, and `diff` inspect form metadata and business rules. `sr fields` inspects catalog item forms. Interactive forms and YAML drafts are covered in [forms](docs/forms.md) and [form UI](docs/formui.md).

Saved views are available through `tickets views`, `problems views`, and `changes views`, and can be selected with `ls --view`. Filters and labels depend on the account; see [views](docs/views.md). `fsv tickets create` reviews incident creation and refuses to send when required rules or context cannot be proven. Live incident creation has not yet been tested; see [incidents](docs/incidents.md).

## TUI keys

Launch `fsv tui` (or `fsv tui --read-only` to disable writes); press `?` for context-sensitive help.

| Keys | Action |
|---|---|
| `Tab`, `j/k`, `J/K`, `G/gg` | Switch pane; move or scroll, faster, or to bottom/top |
| `h/l`, `gt/gp/gc`, `1–9` | Switch entity or detail tab; jump to tickets/problems/changes |
| `v`, `i`, `[ / ]` | Select a view, browse associations, resize panes |
| `f`, `/` | Global search, or filter by field; in search `h/l` narrows and `s` changes sort |
| `y`, `o`, `r` | Copy ID, open in browser, reload list (`r` replies in ticket detail) |
| `N`, `e`, `s`, `n`, `a` | Create, edit change, set status, note, or open actions (writes disabled in read-only mode) |
| `!`, `?`, `q` / `Ctrl+C` | Reopen last persistent failure, help, quit |

In filters, `Tab` completes, `Enter` applies and `Esc` cancels. In note/reply dialogs, `Ctrl+T` toggles visibility where offered, `Ctrl+O` opens ticket options, `Ctrl+S` continues and `Esc` cancels. See [form UI keys](docs/formui.md) for interactive forms.

## Safety model

Reviewed operations construct a plan before a single send; scripted sends require confirmation and the matching plan hash as specified by that command. Reviewed senders do not automatically replay an ambiguous write. They use independent readback where available and report incomplete or unknown results rather than claiming success. When identity, metadata, required rules, payload or verification cannot be proven, supported reviewed flows fail closed. Inspect the record and `fsv journal tail -n 20` before deciding what to do after an uncertain or partial operation; never blindly retry. External notifications, downstream workflows and all account-specific effects cannot always be predicted. Multi-step operations may partially succeed.

This is not a blanket guarantee for every historical command: legacy compatibility paths have narrower guards and are not all certified reviewed single-send workflows. Prefer a documented reviewed operation and consult [reviewed operation coverage and legacy-write guards](docs/reviewed-parity.md).

## Limitations

- Approval delegation is unsupported. Approval actions can send irreversible notifications.
- Physical-asset association writes are disabled; integrated device/business-service support has narrower reviewed limits. Inventory IDs are not legacy display IDs.
- Mentions and ticket split are unsupported. Some operations, account combinations and live responses remain unverified.
- Legacy-write guards reduce risk but do not make every compatibility path a reviewed single-send operation. See the [coverage notes](docs/reviewed-parity.md).
- Which commands have been proven live, with an independent readback, is listed per command in [command coverage](docs/coverage.md), together with the known gaps.

## Development

```bash
git clone https://github.com/lazykern/fsv.git
cd fsv
uv sync
uv run fsv -h
uv run pytest -q
```

Upgrade an installed copy with `uv tool upgrade fsv`. The [documentation index](docs/index.md) links the feature and safety guides.

Contributors and coding agents should start with [contributing for agents](docs/contributing-agents.md) and the [live-testing methodology](docs/live-testing.md). The [Freshservice knowledge base](docs/freshservice/overview.md) describes how the platform and its agent web UI behave (API families, forms and business rules, lifecycles and workflows, wire contracts, quirks), with each statement marked as verified live, captured or inferred.
