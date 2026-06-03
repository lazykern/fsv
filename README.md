# fsv

Freshservice CLI. Drives changes, tickets, and problems via session-cookie auth; no API key needed.

## How it works

fsv calls the **internal** `/api/_/` endpoints the Freshservice web UI uses, not the public v2 REST API. These endpoints expose richer data with no published rate cap. Paste a valid browser session cookie once; fsv stores it locally.

fsv also uses the public v2 API (`/api/v2/`) for schema, task writes, and approvals. Both paths share the same cookie.

## Install

### End users

Install from GitHub:

```bash
uv tool install git+https://github.com/lazykern/fsv.git
```

Upgrade later with:

```bash
uv tool upgrade fsv
```

### Contributors / local checkout

```bash
git clone git@github.com:lazykern/fsv.git
cd fsv
uv sync
uv tool install --editable .
```

Run ad hoc from checkout: `uv run fsv ...`

Update local editable install:

```bash
cd /path/to/fsv
git pull --ff-only
uv sync
uv tool install --editable .
```

### Shell completion

```bash
fsv completion install
fsv completion refresh
exec $SHELL
```

Completion is schema-aware after cache refresh. Login auto-refreshes cache.

```bash
fsv tickets ls --where sta<TAB>            # status=
fsv tickets ls --where status=<TAB>        # Open, Pending, ...
fsv changes ls --where requester=ali<TAB>  # requester email (network)
fsv changes fields --choices add_<TAB>     # add_database_task, ...
```

Completion reads the local schema cache and skips network calls by default. Enable network requester/agent completion:

```bash
fsv config set completion.network on
```

Remote completion requires a 2+ character prefix; without one, the shell falls back to file completion.

Debug:

```bash
fsv completion doctor
```

## Upgrade / staying current

Check installed version:

```bash
fsv --version
```

fsv does not auto-update itself. To know when new release ships:

- Watch GitHub releases: <https://github.com/lazykern/fsv/releases>
- In GitHub UI: **Watch → Custom → Releases**

If you installed from GitHub with `uv tool install git+https://github.com/lazykern/fsv.git`, upgrade with:

```bash
uv tool upgrade fsv
```

If you run from local checkout only:

```bash
cd /path/to/fsv
git pull --ff-only
uv sync
```

## Login

fsv does **not** drive a browser or read browser storage. You bring the cookies.

1. Open `https://yourcompany.freshservice.com` in any browser, complete SSO login.
2. Open DevTools → Network tab → click any `/api/_/...` request.
3. Copy the value of the `Cookie:` request header (right-click → Copy value).
4. Paste into fsv:

```bash
fsv auth login --domain yourcompany.freshservice.com              # interactive prompt
pbpaste | fsv auth login --domain yourcompany.freshservice.com --header -
fsv auth login -d yourcompany.freshservice.com -H "_x_m=...; _x_d=...; ..."
```

The Network-tab Cookie header includes HttpOnly cookies (`_itildesk_session`, `user_credentials`) that `document.cookie` cannot read; fsv needs these for API access.

### Storage backends

```bash
fsv auth login --store file       # ~/.config/fsv/session.json (plain JSON, chmod 600)
fsv auth login --store argon      # encrypted file using Argon2id + AES-256-GCM
fsv auth login --store keychain   # macOS Keychain (encrypted at rest)
fsv auth logout                   # wipe file + Keychain + backend preference
```

Omit `--store` during interactive login to choose `file`, `argon`, or `keychain`. Default is Keychain on macOS, otherwise file.
Switching backend re-saves to the new location and removes the old copy.

Argon mode asks for a passphrase on save and read. Keychain first-read prompts macOS access dialog → click "Always Allow" once → silent thereafter. No admin/sudo required.

### Why not username/password?

The Freshworks login endpoint requires reCAPTCHA Enterprise tokens (Google JS sandbox + risk signals) with Cloudflare bot detection on top. No headless path works; cookie paste is the only option.

**Security note**: fsv never reads browser cookie databases, keychains, or profiles. No DLP concerns.

## Quick start

```bash
# Check auth
fsv auth status

# List open changes
fsv changes ls --where status=Open

# Get a change with full detail
fsv changes get CHN-1234

# Update a ticket
fsv tickets update INC-5678 --status Pending --agent alice@example.com

# Add a private note to a change
fsv changes add-note CHN-1234 "PVT result PASS"

# Clone a change
fsv changes clone CHN-1234 --with-tasks --with-planning

# Download all attachments
fsv changes download CHN-1234 --all --out ./evidence
```

## Commands

```
fsv changes  ls | search | get | update | create | clone | download | url | state | approvals | activity | tasks | task-update | task-delete | assets | associations | add-note | notes | fields | lookup | filters
fsv tickets  ls | search | get | update | url | reply | activity | tasks | fields | lookup | filters
fsv problems ls | search | get | update | url | add-note | notes | activity | tasks | fields | lookup | filters

fsv auth login --domain yourcompany.freshservice.com
fsv auth status
fsv config set <key> <value>
fsv cache status | refresh | clear
```

### List

```bash
fsv changes ls                              # list changes
fsv changes ls --all                        # auto-paginate
fsv tickets ls --where requester=alice@example.com --where 'created_at>=2025-05-01T00:00:00+07:00'
fsv tickets ls --where agent="Jane Agent"
fsv tickets ls --where status=Open --where priority=High
fsv tickets ls --where status=Open --where status=Pending --or
fsv changes ls --where "Change Category=Infrastructure"
fsv tickets ls --where status=Open --debug  # show resolved query_hash

# Operators: = (equals), != (not equals). For dates: >=, <=, >, < also work.
fsv tickets fields requester                # schema-discovered fields
fsv tickets fields --default                # portable Freshservice fields
fsv changes fields --custom                 # tenant-specific fields
fsv changes fields --choices "Change Category"
fsv tickets lookup requester alice@example.com
fsv changes lookup "Change Category" Infrastructure
fsv tickets ls --output csv                 # table | json | csv | tsv (-o also works)
fsv problems ls --view "All Problems"
```

### Search / Get / Activity / Tasks

```bash
fsv tickets search "status:2 AND priority:3"
fsv tickets search "status:2" -o tsv
fsv changes get CHN-1234
fsv changes get CHN-1234 --stats       # adds planning_fields + timestamps
fsv changes get CHN-1234 --json | jq .
fsv changes activity CHN-1234 -n 20
fsv changes tasks CHN-1234
fsv changes url CHN-1234
```

### Write

```bash
fsv changes update CHN-1234 --status Closed --priority Medium --dry-run
fsv changes update CHN-1234 --planning "Others Document" --description "Evidence attached" --file evidence.xlsx
fsv changes create --dry-run
fsv changes clone CHN-1234 --with-tasks --with-planning
fsv changes download CHN-1234 --all --out ./evidence
fsv changes assets CHN-1234 --list-categories
fsv changes assets CHN-1234 --search app
fsv changes assets CHN-1234 --search OOS --category "Application Portfolio"
fsv changes assets CHN-1234 --add OOS --category "Application Portfolio" --dry-run
fsv changes assets CHN-1234 --pick --yes
fsv changes associations CHN-1234 --add SR-5678 --dry-run
fsv tickets update INC-9012 --status Pending --agent alice@example.com
fsv tickets update INC-9012 --group "Service Desk"
fsv changes add-note CHN-1234 "PVT result PASS"      # private by default
fsv changes add-note CHN-1234 "..." --public
fsv tickets reply INC-9012 "<HTML or text>"
```

## Notes

- **Internal API**: `/api/_/` endpoints mirror what the Freshservice web UI calls — richer payloads, no published rate cap. v2 API (`/api/v2/`) used for schema, task writes, and approvals.
- **Rate limit**: v2 API = 400 req/day per tenant. `/api/_/` no published cap.
- **Cookies**: Re-login when 401/redirect to freshid. Sessions last ~days to weeks. Login auto-refreshes schema + completion cache.
- **Filter discovery**: `fsv {changes|tickets|problems} filters` lists saved filter names.
- **Field discovery**: `fields` marks `default_field=true` as portable/default and `false` as tenant custom; `fields --choices FIELD` counts/lists choices.
- **Schema filters**: repeat `--where FIELD=VALUE`; fields resolve by current tenant schema, so custom fields remain tenant-specific.
- **AND/OR grouping**: Default AND; add `--or` for OR grouping (e.g., `fsv tickets ls --where status=Open --where status=Pending --or`).
- **Custom field values**: Custom fields use text labels (e.g., `--where 'Business Service=Email'`), default fields use choice IDs in filters.
- **Update values**: `update --status/--priority` accepts labels or IDs; `--agent/--group` accepts names/emails or IDs; `--planning` accepts planning field label/name/id.
- **Autocomplete**: `lookup` searches requesters, agents, groups, and schema choices; `--where requester=...` and `--where agent=...` use the same resolver. `changes assets --category` completes from Freshservice CMDB asset types.
- **Debug**: `--debug` shows resolved query_hash for inspection.
- **Change asset categories**: `changes assets --list-categories` reads category/type labels from `/cmdb/items`. Asset search endpoint `/api/_/assets-to-associate` does not expose server-side category filtering, so `--category` filters matched rows client-side by asset type label. `--pick` requires TTY and prompts for category first when `--category` omitted.
- **Display IDs**: CHN- (changes), INC-/SR- (tickets — discriminate by `type`), PRB- (problems).
- **Config**: `fsv config set completion.network on` enables remote requester/agent completion.
- **Schema cache**: 7d TTL in `~/.config/fsv/schema/`, namespaced by tenant. Use `fsv cache refresh` to force (`fsv completion refresh` is an alias).

## Architecture

```
src/fsv/
  __init__.py     # entry: app()
  config.py       # paths, domain
  session.py      # cookie load/save and paste-based login helpers
  client.py       # singleton httpx wrapper, CSRF for /api/_/, rate limit headers
  resources.py    # Resource dataclass + registry (CHANGES/TICKETS/PROBLEMS)
  schema.py       # form_fields cache + choice_label helper
  editor.py       # JSONC editor workflow for create/clone/edit
  render.py       # rich Table + JSON/CSV/TSV output
  cli.py          # typer app + per-resource sub-apps from registry
```

Adding a new resource = ~10 LoC in `resources.py` when the tenant exposes that Freshservice module.
