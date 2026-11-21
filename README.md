# fsv

Freshservice from your terminal. Browse changes, tickets and problems, update records, and fill out change and service-request forms.

> [!WARNING]
> In development and unstable. Commands, flags, JSON output and on-disk formats can change or break without notice. Use `--dry-run` before any `--yes` write.
>
> fsv uses your browser session, not an API key. It sends your Freshservice session cookies to the same internal endpoints the website uses (plus a few public v2 endpoints), so every call runs as you, with your permissions, and shows up in audit logs as you.
>
> - The stored session is a credential. Treat it like a password.
> - Internal endpoints are undocumented and can change at any time.
> - Check your organization's policy before using a browser session in scripts or agents.
> - Sessions expire; run `fsv auth login` again.
> - Test with a separate config dir and tenant.

## Get started

This branch is the Rust port. Build the `fsv` binary from a checkout:

```bash
cargo build --release -p fsv-cli --bin fsv
./target/release/fsv setup
```

The setup wizard asks for your domain and handles login, shell completion and defaults. To sign in directly:

```bash
fsv auth login --browser --domain acme.freshservice.com
```

Finish login and any SSO or MFA prompts in the browser window. Then try some read-only commands:

```bash
fsv auth status
fsv changes ls --where status=Open
fsv changes get CHN-1234
fsv tickets ls --where requester=alice@example.com
```

Commands can output JSON for other tools. The full-screen TUI is being redesigned and is not in this build.

Check which agent, tenant and config dir this shell targets with `fsv whoami` (or `fsv config show`, which also shows how writes are confirmed).

### Know your tenant

The config dir (default `~/.config/fsv`, or `FSV_CONFIG_DIR`) decides which tenant every command talks to. It holds the domain `fsv auth login` configured last.

Each command that talks to a tenant prints `fsv: tenant <domain> (config <dir>)` once on stderr. `--quiet` or `FSV_QUIET=1` hides it, and `--json` stdout stays pure JSON.

`--yes` writes are refused until the tenant is confirmed. Confirm it with one of:

- `fsv config set tenant.confirmed <tenant>`, once per config dir
- `--tenant <tenant>`, per command
- `FSV_ALLOW_TENANT=<tenant>`

`--dry-run` never needs confirmation. Plan hashes bind the tenant, and `--json` reviews and results of write commands carry `tenant` and `config_dir`, so a hash reviewed on one tenant is refused on another. With more than one tenant, give each its own `FSV_CONFIG_DIR` and set it explicitly.

## Fill out a form

Run these in a terminal to open an interactive form:

```bash
fsv changes create
fsv changes update CHN-1234
fsv tickets create
fsv sr create ITEM_ID
```

Fields, labels, choices and business rules come from your Freshservice account. Dropdowns filter as you type, people fields search, and required fields update as you answer. You review before submitting. Edit forms start with the record's current values and send only what changed.

To try a change form without creating anything:

```bash
fsv changes create --dry-run
```

To inspect a form without filling it out:

```bash
fsv form fields changes --choices Priority
fsv form explain changes --set "Cloud Only=No"
fsv sr catalog
fsv sr fields ITEM_ID
```

`Cloud Only` is an example label; use the labels from your account. See [the form UI guide](docs/formui.md) for keys and widgets, [the forms guide](docs/forms.md) for rules and drafts, and [service requests](docs/service-requests.md) for catalog forms.

## Common commands

### Find and read records

```bash
fsv changes views
fsv changes ls --view "All Changes"
fsv changes ls --all
fsv tickets ls --where status=Open --where priority=High
fsv tickets search "database error"
fsv changes get CHN-1234 --all-tabs
fsv changes get CHN-1234 --json | jq .
fsv changes activity CHN-1234 -n 20
fsv changes tasks CHN-1234
fsv changes approvals CHN-1234
fsv changes state CHN-1234
fsv changes download CHN-1234 --all --dir ./evidence
```

View names and status labels depend on your account. Lists use the website's saved views and columns. Detail pages show properties, planning or requested items, conversations and available tabs. See [the views guide](docs/views.md) for filters, pagination and output formats.

### Make changes

```bash
fsv tickets update INC-5678 --status Pending --agent alice@example.com
fsv changes update CHN-1234 --status Closed --dry-run
fsv changes update CHN-1234 --planning "Reference Documents" --description "Evidence attached" --file evidence.xlsx
fsv changes add-note CHN-1234 "Verification passed"       # private by default
fsv changes tasks-add CHN-1234 --title "Acme verification" --dry-run --json
fsv tickets reply INC-5678 "Investigation is in progress." --dry-run
fsv changes clone CHN-1234 --with-tasks --with-planning --dry-run
fsv changes assets add CHN-1234 myapp --category "Application Portfolio" --dry-run
fsv changes associations add CHN-1234 SR-5678 --dry-run
```

Use `--dry-run` first. Writes can trigger your account's workflows, send notifications or create tasks, and fsv can't see every downstream effect.

[Change-task creation](docs/change-tasks.md) uses an exact reviewed hash for scripted sends (`--yes --expect-plan FULL_HASH --no-input`), defaults to no owner, and never retries an uncertain POST. Task creation is based on a captured request that was blocked; a successful live response has not been observed.

[Change–ticket associations](docs/change-associations.md) use a reviewed hash, one write attempt and complete paginated readback. Scripted sends require `--yes --expect-plan FULL_HASH`; uncertain outcomes block resubmission. Lists keep partial rows and warn that they are incomplete.

For the full command list, run `fsv --help` or a command's help:

```bash
fsv changes --help
fsv tickets --help
fsv problems --help
fsv sr --help
```

`fsv tickets create` supports incident creation with review and dry run. It refuses to send when rules or required rule context are unavailable. Live incident creation has not been tested; see [the incident guide](docs/incidents.md).

## Drafts and scripts

For repeatable work, keep answers in YAML instead of filling out the form each time:

```yaml
requester: jane.doe@example.com
answers:
  Subject: "Billing: rotate keys"
  Change Type: Normal
  Cloud Only: No
  Stage: Staging
  Staging Start: 2030-11-18 09:00
description_md: |
  Rotate the **billing** keys.
attachments: [evidence/plan.pdf]
planning:
  Go-Live Checklist: [evidence/checklist.xlsx]
```

Labels and planning fields must match your account. Dates use your Freshservice time zone unless you supply an offset; `FSV_TZ` overrides the default zone.

```bash
fsv changes create --from change.yaml --dry-run
fsv changes create --from change.yaml --dry-run -o json
fsv changes create --from change.yaml --yes --expect-plan 0cfda1921f64
fsv changes update CHN-1234 --set "Stage=Live" --dry-run
```

Copy the plan hash from your own dry run; the one above is an example. `--expect-plan` accepts at least 12 hex characters and refuses to send if the answers, files or form rules changed. Without a terminal, writes that require confirmation need `--yes`.

`fsv changes create --edit` opens an annotated YAML draft in `$VISUAL` or `$EDITOR`.

The [forms guide](docs/forms.md) covers presets, answer precedence, rule messages, local workflow hints and attachment handling.

## Login and session storage

Browser login opens a temporary profile, waits for you to sign in, and saves the session cookies locally. It doesn't read your existing browser profile or saved passwords. It needs Chrome, Chromium, Edge or Brave; set `FSV_BROWSER=/path/to/browser` if fsv can't find one.

If browser login fails, sign in normally, open DevTools → Network, copy the `Cookie` header from a Freshservice `/api/_/` request, and paste it into the prompt:

```bash
fsv auth login --domain acme.freshservice.com
```

Treat that header like a password. It includes HttpOnly cookies that `document.cookie` can't read. Don't put it in a shell command, where it can end up in your history.

Choose where to store the session with `--store`:

| Backend | Storage |
|---|---|
| `keychain` | macOS Keychain; the default on macOS |
| `file` | Plain JSON with file mode 0600; the default elsewhere |
| `argon` | Encrypted file using Argon2id and AES-256-GCM; asks for a passphrase |

For example:

```bash
fsv auth login --browser --domain acme.freshservice.com --store keychain
fsv auth status
fsv auth logout
```

On macOS, Keychain may ask for access the first time. Switching backends removes the old stored copy. Log in again when the session expires.

## Account differences and limits

fsv reads your account's fields, choices, planning labels and rules rather than shipping a fixed configuration. It supports accounts without workspaces or business rules, and optional endpoints that return 403 or 404. When rules can't be loaded, forms warn and use static flags; incident creation refuses to send.

Not every Freshservice feature or account configuration has been tested. Known limits:

- The change form's **Request for** field is display-only for now. Its request key is unverified, so the review marks it "not sent".
- Your role and plan determine which tabs, lookups and actions are available.
- Custom workflows can add work that the form metadata doesn't show. You can record known effects in a local `hints.yaml`; see [the forms guide](docs/forms.md).
- A create can succeed even if a later attachment or planning step fails. fsv prints the created ID and partial results. Check that record before retrying.
- Form creates with the same plan hash are blocked for 24 hours unless you pass `--allow-duplicate`. Reviewed task creates instead keep identical sent/unknown plans blocked without an override.

## Shell completion

```bash
fsv completion install
fsv completion refresh
exec $SHELL
```

Completion uses your cached schema for field names and choices. People lookups need network completion enabled:

```bash
fsv config set completion.network on
fsv tickets ls --where requester=ali<TAB>
```

Remote lookups need at least two characters. Run `fsv completion doctor` to troubleshoot, or `fsv cache refresh` after an admin changes the form.

## Troubleshooting

Add `-v` for request and response details. List commands also accept `--debug` to show the resolved query hash.

`fsv journal tail -n 20` shows recent writes, including failed requests and created IDs. The journal stores metadata, not request bodies or cookies. Each resolved configuration directory and configured domain has its own journal under `$XDG_STATE_HOME/fsv/journals/` (default `~/.local/state/fsv/journals/`). The directory names are hashes; `fsv journal path` prints the active path. Without a configured domain, it uses an `unknown` tenant directory. An explicit `XDG_STATE_HOME` controls the root for every profile.

The old shared journal at `$XDG_STATE_HOME/fsv/journal.jsonl` is left untouched. It is plain JSONL, and there is no migration flag. It isn't used for duplicate checks because its entries can't be assigned to a profile and tenant. After upgrading, review old entries before retrying a create.

| Exit code | Meaning |
|---|---|
| 0 | Success |
| 1 | Other error |
| 2 | Invalid input or refused confirmation |
| 3 | Missing or expired session; log in again |
| 4 | Conflict, duplicate or changed plan |
| 5 | Server rejection or a partial operation; inspect the error and any created ID |

## Update or work on the code

```bash
git clone https://github.com/lazykern/fsv.git
cd fsv
git switch rust-port
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
```

The binary is `fsv-cli`. Domain code is split across `fsv-core` (HTTP, session, journal), `fsv-api` (Freshservice operations), `fsv-forms` (form engine), `fsv-flows` (change, ticket, and catalog flows), `fsv-views` (rendering), `fsv-formui` (interactive forms), and `fsv-app` (the adapters that wire them together). The Python source this port tracks lives in a separate checkout, not in this tree. JSON fixtures under `crates/` and `port/golden` are frozen expected values.
