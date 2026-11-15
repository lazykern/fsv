# fsv

Freshservice from your terminal: browse records and saved views, inspect forms, and send writes after reviewing them.

> [!WARNING]
> In development and unstable. Commands, flags, JSON output, cache and journal formats can change or break without notice. Use `--dry-run` before any `--yes` write.
>
> fsv uses your browser session, not an API key. It sends your Freshservice session cookies to internal website endpoints (and some public APIs), so every call runs as you, with your permissions, and shows up in audit logs as you.
>
> - The stored session is a credential. Treat it like a password.
> - Internal endpoints are undocumented and can change at any time.
> - Check your organization's policy before using a browser session in scripts or agents.
> - Sessions expire; run `fsv auth login` again.
> - Test with a separate config dir and tenant.

## Getting started

Requires Python 3.12+ and [uv](https://docs.astral.sh/uv/).

```bash
uv tool install git+https://github.com/lazykern/fsv.git
fsv setup
# Or sign in directly:
fsv auth login --browser --domain acme.freshservice.com
fsv whoami                                              # agent, tenant and config dir
fsv config set tenant.confirmed acme.freshservice.com   # once: allow --yes writes here
fsv config show
fsv tickets ls --where status=Open --json
fsv changes get CHN-1234 --json
fsv tui --read-only
```

Finish SSO or MFA in the browser. `fsv config get KEY` and `fsv config set KEY VALUE` manage settings. Browser login uses a temporary browser profile and does not read your existing one. Set `FSV_BROWSER` if the browser can't be found. `fsv auth login --domain acme.freshservice.com` instead prompts for a Cookie header; treat it as a password and don't pass it on the command line. The session is stored in Keychain on macOS and a mode-0600 file elsewhere; `--store argon` uses an encrypted file.

### Know your tenant

The config dir (default `~/.config/fsv`, or `FSV_CONFIG_DIR`) decides which tenant every command talks to. It holds the domain `fsv auth login` configured last.

Each command that talks to a tenant prints `fsv: tenant <domain> (config <dir>)` once on stderr. `--quiet` or `FSV_QUIET=1` hides it, and `--json` stdout stays pure JSON.

`--yes` writes are refused until the tenant is confirmed. Confirm it with one of:

- `fsv config set tenant.confirmed <tenant>`, once per config dir
- `--tenant <tenant>`, per command
- `FSV_ALLOW_TENANT=<tenant>`

`--dry-run` never needs confirmation. With more than one tenant, give each its own `FSV_CONFIG_DIR` and set it explicitly.

### Reviewed writes

Inspect the plan first, then send the same arguments with its full hash:

```bash
fsv tickets add-note INC-1234 --body 'Investigation started' --dry-run --json
# Copy plan_hash from the output and use it in place of FULL_HASH:
fsv tickets add-note INC-1234 --body 'Investigation started' --yes --expect-plan FULL_HASH --json
```

`FULL_HASH` is a placeholder. Plan hashes bind the tenant, and `--json` reviews and results carry `tenant` and `config_dir`, so a hash reviewed on one tenant is refused on another. Check the target, payload, recipients and effects. If the plan changes, review it again. Some older form flows accept a hash prefix, but the full hash works everywhere.

Confirmation and flags vary by command; check `-h`. `--json` is available on many commands, not all. List commands also take `--output`. Use `--refresh` where offered (for example `fsv tickets fields --refresh`) to refresh the cached schema. `fsv cache status` and `fsv cache refresh` inspect and refresh stale caches; see [views](docs/views.md).

## Commands

Run `fsv -h`, `fsv COMMAND -h` or `fsv COMMAND SUBCOMMAND -h` for arguments and options. The list below follows the CLI help; which operations work depends on your account's permissions and metadata. See the [command cheat sheet](docs/commands.md) and the [documentation index](docs/index.md).

- General: `help`, `search`, `setup`, `tui`, `whoami`; `auth` (`login`, `logout`, `status`); `config` (`list`, `get`, `set`, `show`, `defaults show/set/clear`); `cache` (`status`, `refresh`, `clear`); `journal` (`ls`, `tail`, `path`); `completion` (`show`, `install`, `refresh`, `doctor`).
- `tickets`: `ls`, `get`, `activity`, `export`, `fields`, `lookup`, `views`, `search`, `url`, `create`, `update`, `download`, `conversations`, `approvals`, `associations`, `add-note`, `note-edit`, `note-delete`, `reply`, `forward`, `assign`, `pick-up`, `merge`, `resolve`, `close`, `reopen`, `status`, `bulk-priority`; nested `assets`, `columns`, `watchers`, `tasks`, `private-views`, `canned`, `scenarios`, `time`.
- `sr`: `catalog`, `fields`, `create`, `get`; nested `assets` and `approvals` (`list`, `request`, `approve`, `reject`, `cancel`, `remind`).
- `problems`: `ls`, `get`, `activity`, `tasks`, `export`, `fields`, `lookup`, `views`, `search`, `url`, `create`, `update`, `download`, `notes`, `add-note`, `note`, `status`, `link-incident`, `unlink-incident`, `link-solution`, `unlink-solution`, `mark-known-error`, `planning-text`, `write-child`; nested `assets`, `columns`.
- `changes`: `ls`, `get`, `activity`, `tasks`, `tasks-add`, `tasks-update`, `tasks-delete`, `approvals`, `approval-request`, `approval-respond`, `approval-cancel`, `state`, `export`, `asset-picker`, `planning-text`, `fields`, `lookup`, `views`, `search`, `url`, `update`, `download`, `clone`, `create`, `notes`, `add-note`; nested `associations`, `assets`, `columns`, `watchers`.
- `assets search` searches integrated inventory. `solutions create` reviews draft knowledge articles. `attachments delete` reviews a deletion with an explicit owner.
- `form fields`, `explain`, `dump` and `diff` inspect form metadata and business rules. `sr fields` inspects catalog item forms. See [forms](docs/forms.md) and [form UI](docs/formui.md).

Saved views are available through `tickets views`, `problems views` and `changes views`, and can be selected with `ls --view`. Filters and labels depend on the account; see [views](docs/views.md).

`fsv tickets create` reviews incident creation and refuses to send when required rules or context can't be proven. Live incident creation has not been tested; see [incidents](docs/incidents.md).

## TUI keys

Run `fsv tui`, or `fsv tui --read-only` to disable writes. Press `?` for help on the current screen.

| Keys | Action |
|---|---|
| `Tab`, `j/k`, `J/K`, `G/gg` | Switch pane; move or scroll, faster, or to bottom/top |
| `h/l`, `gt/gp/gc`, `1–9` | Switch entity or detail tab; jump to tickets/problems/changes |
| `v`, `i`, `[ / ]` | Select a view, browse associations, resize panes |
| `f`, `/` | Global search, or filter by field; in search `h/l` narrows and `s` changes sort |
| `y`, `o`, `r` | Copy ID, open in browser, reload list (`r` replies in ticket detail) |
| `N`, `e`, `s`, `n`, `a` | Create, edit change, set status, note, or open actions (disabled in read-only mode) |
| `!`, `?`, `q` / `Ctrl+C` | Reopen last persistent failure, help, quit |

In filters, `Tab` completes, `Enter` applies and `Esc` cancels. In note and reply dialogs, `Ctrl+T` toggles visibility where offered, `Ctrl+O` opens ticket options, `Ctrl+S` continues and `Esc` cancels. See [form UI keys](docs/formui.md) for interactive forms.

## Safety model

Reviewed operations build a plan before sending. Scripted sends need confirmation and the matching plan hash, as that command specifies.

A reviewed sender never replays an ambiguous write on its own. It checks the result with an independent readback where one exists, and reports an incomplete or unknown result instead of claiming success. If identity, metadata, required rules, payload or verification can't be proven, the flow refuses to send.

After an uncertain or partial operation, inspect the record and run `fsv journal tail -n 20` before deciding what to do. Don't blindly retry. fsv can't predict external notifications, downstream workflows or other account-specific effects, and multi-step operations can partly succeed.

Legacy compatibility paths have narrower guards and are not all reviewed single-send workflows. Prefer a documented reviewed operation; see [reviewed operation coverage and legacy-write guards](docs/reviewed-parity.md).

## Limitations

- Approval delegation is unsupported. Approval actions can send irreversible notifications.
- Physical-asset association writes are disabled. Integrated device and business-service support has narrower limits. Inventory IDs are not legacy display IDs.
- Mentions and ticket split are unsupported. Some operations, account combinations and live responses are unverified.
- Legacy-write guards reduce risk but don't make every compatibility path a reviewed single-send operation. See the [coverage notes](docs/reviewed-parity.md).
- [Command coverage](docs/coverage.md) lists, per command, what has been proven live with an independent readback, and the known gaps.

## Development

```bash
git clone https://github.com/lazykern/fsv.git
cd fsv
uv sync
uv run fsv -h
uv run pytest -q
```

Upgrade an installed copy with `uv tool upgrade fsv`. The [documentation index](docs/index.md) links the feature and safety guides.

Contributors and coding agents should start with [contributing for agents](docs/contributing-agents.md) and the [live-testing methodology](docs/live-testing.md). The [Freshservice knowledge base](docs/freshservice/overview.md) describes how the platform and its agent web UI behave (API families, forms and business rules, lifecycles and workflows, wire contracts, quirks). Each statement is marked verified live, captured or inferred.
