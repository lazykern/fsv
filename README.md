# fsv

Freshservice from your terminal. Browse changes, tickets and problems, update records, or fill out change and service-request forms without opening an editor.

fsv signs in with your browser session, so you don't need an API key. It uses the same internal endpoints as the Freshservice website, plus a few public v2 endpoints. Those internal APIs aren't a stable contract: a Freshservice update can break things.

## Get started

Install with [uv](https://docs.astral.sh/uv/):

```bash
uv tool install git+https://github.com/lazykern/fsv.git
fsv setup
```

The setup wizard asks for your domain and helps with login, shell completion and defaults. If you'd rather just sign in:

```bash
fsv auth login --browser --domain acme.freshservice.com
```

Finish login and any SSO or MFA prompts in the browser window. Then try a few read-only commands:

```bash
fsv auth status
fsv changes ls --where status=Open
fsv changes get CHN-1234
fsv tickets ls --where requester=alice@example.com
fsv tui
```

The full-screen TUI has saved views, filter suggestions, search and keyboard navigation. You can also use individual commands and pipe JSON into other tools.

## Fill out a form

Run these in a terminal to open the interactive form:

```bash
fsv changes create
fsv changes update CHN-1234
fsv tickets create
fsv sr create ITEM_ID
```

Fields, labels, choices and business rules come from your Freshservice account. Dropdowns filter as you type, people fields offer search, and required fields change as you answer. You get a review before submitting. Edit forms start with the record's current values and send only what changed.

To try a change form without creating anything:

```bash
fsv changes create --dry-run
```

You can also inspect the form without filling it out:

```bash
fsv form fields changes --choices Priority
fsv form explain changes --set "Cloud Only=No"
fsv sr catalog
fsv sr fields ITEM_ID
```

Custom labels such as `Cloud Only` are examples; use the labels from your own account. See [the form UI guide](docs/formui.md) for keys and widgets, [the forms guide](docs/forms.md) for rules and drafts, and [service requests](docs/service-requests.md) for catalog forms.

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

View names and status labels depend on your account. Lists use the website's saved views and columns; detail pages show properties, planning or requested items, conversations and available tabs. See [the views guide](docs/views.md) for filters, pagination and output formats.

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

Use `--dry-run` first when you're unsure. Writes can trigger your account's workflows, send notifications or create tasks. fsv can't discover every downstream effect.

[Change-task creation](docs/change-tasks.md) uses an exact reviewed hash for scripted sends (`--yes --expect-plan FULL_HASH --no-input`), defaults to no owner, and never retries an uncertain POST. In the main TUI, select a change and use **a → Add task**. Task CREATE is backed by a blocked request capture; its accepted live response remains unobserved.

For the full command list, run `fsv --help` or a module's help:

```bash
fsv changes --help
fsv tickets --help
fsv problems --help
fsv sr --help
```

`fsv tickets create` supports incident creation with review and dry run. It refuses to send when rules or required rule context are unavailable. Live incident creation has not yet been tested; see [the incident guide](docs/incidents.md).

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

If you prefer an external editor, `fsv changes create --edit` opens an annotated YAML draft in `$VISUAL` or `$EDITOR`. It's optional, not the default.

The [forms guide](docs/forms.md) covers presets, answer precedence, rule messages, local workflow hints and attachment handling.

## Login and session storage

Browser login opens a temporary profile, waits for you to sign in, and saves the session cookies locally. It doesn't read your existing browser profile or saved passwords. You'll need Chrome, Chromium, Edge or Brave; set `FSV_BROWSER=/path/to/browser` if fsv can't find it.

If browser login doesn't work, sign in normally, open DevTools → Network, and copy the `Cookie` header from a Freshservice `/api/_/` request. Paste it into the prompt:

```bash
fsv auth login --domain acme.freshservice.com
```

Treat that header like a password. It includes HttpOnly cookies that `document.cookie` can't read. Avoid putting it directly in a shell command, where it could end up in your history.

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

On macOS, Keychain may ask for access the first time. Switching backends removes the old stored copy. Run login again when your session expires.

## Account differences and limits

fsv reads your account's fields, choices, planning labels and rules rather than shipping a fixed configuration. It supports accounts without workspaces or business rules, and optional endpoints that return 403 or 404. When rules can't be loaded, forms warn and use static flags; incident creation refuses to send.

That doesn't mean every Freshservice feature or account configuration has been tested. A few limits worth knowing:

- The change form's **Request for** field is display-only for now. Its request key is unverified, so the review marks it "not sent".
- Your role and plan determine which tabs, lookups and actions are available.
- Custom workflows can add work that isn't visible in the form metadata. You can document known effects in local `hints.yaml`; see [the forms guide](docs/forms.md).
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

Remote lookups need at least two characters. Use `fsv completion doctor` to troubleshoot completion, or `fsv cache refresh` after an admin changes the form.

## Troubleshooting

Add `-v` for request and response details. List commands also accept `--debug` to show the resolved query hash.

`fsv journal tail -n 20` shows recent writes, including failed requests and created IDs. The journal stores metadata, not request bodies or cookies. Each resolved configuration directory and configured domain has its own journal under `$XDG_STATE_HOME/fsv/journals/` (default `~/.local/state/fsv/journals/`). The directory names are hashes; `fsv journal path` prints the active path. Without a configured domain, it uses an `unknown` tenant directory. An explicit `XDG_STATE_HOME` controls the root for every profile.

The former shared journal at `$XDG_STATE_HOME/fsv/journal.jsonl` is left untouched. You can inspect it directly as JSONL; there is no legacy-view or migration flag. It is not used for duplicate checks because its entries cannot reliably be assigned to a profile and tenant. Review old entries before retrying a create after upgrading.

| Exit code | Meaning |
|---|---|
| 0 | Success |
| 1 | Other error |
| 2 | Invalid input or refused confirmation |
| 3 | Missing or expired session; log in again |
| 4 | Conflict, duplicate or changed plan |
| 5 | Server rejection or a partial operation; inspect the error and any created ID |

## Update or work on the code

For an installed copy:

```bash
uv tool upgrade fsv
```

For a local checkout:

```bash
git clone https://github.com/lazykern/fsv.git
cd fsv
uv sync
uv run fsv --help
uv run pytest -q
```

Use `uv tool install --editable .` if you want the checkout available as `fsv` in your shell. Pull updates with `git pull --ff-only`, then run `uv sync`.

Most command handling lives in `src/fsv/cli.py`. The `forms/` package handles form metadata and rules, `formui/` contains the interactive forms, and `views.py` and `pages.py` load and render the site-style views. The full-screen app lives in `tui/`.
