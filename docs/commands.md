# Command cheat sheet

## Exact-conversation forward

`fsv tickets forward INC-123 'Acme update' --conversation 901 --to user@example.com
--sender support@example.com --dry-run --json` reviews the captured ticket forward
wire with an integer `conversation_id`. The source must exist exactly once in a
complete history; source edits invalidate the review. Send using
`--yes --expect-plan FULL_HASH`. Source selection is bound to the review; returned
conversations do not expose source identity, so readback verifies delivery fields,
not an independently returned source link. No attachment/CC/BCC/status combinations.


## Accepted initial attachment scope

`tickets reply`, `tickets add-note` and `tickets create --attach FILE --max-bytes N`
use hash-bound two-step uploads and exact owning-record readback.
`changes create --attach FILE --attach-flow before-create --max-bytes N` uses
the same durable transaction. Review with `--dry-run --json`; send with
`--yes --expect-plan FULL_HASH`. No replay after an uncertain upload or send.
Initial change attachments cannot combine with devices/planning in this path.
Inline image uploads remain blocked: the accepted ledger records the response
and final image HTML but omits the upload request body/media type. A null capture
is not proof of a bodyless upload; no multipart contract is invented.


Examples use synthetic IDs and names. Replace them with values from your account. Commands that change data are shown with `--dry-run`; it may still read account data. Check each plan and copy its full `plan_hash` before adding `--yes --expect-plan FULL_HASH` to the same command. Some operations need additional account-specific fields or confirmations; see their help. Never send these examples unchanged.

## Session, configuration, and discovery

```bash
fsv help
fsv setup
fsv auth login --browser --domain acme.freshservice.com
fsv auth status
fsv auth logout
fsv config list
fsv config get completion.network
fsv config set completion.network on
fsv config defaults show
fsv cache status
fsv cache refresh
fsv journal tail -n 20
fsv completion doctor
fsv search 'Acme service' --json
fsv tui --read-only
```

## Tickets and views

```bash
fsv tickets views
fsv tickets ls --where status=Open --json
fsv tickets ls --view 'All Tickets'
fsv tickets get INC-1234 --json
fsv tickets activity INC-1234
fsv tickets fields --refresh
fsv tickets search 'Acme service'
fsv tickets conversations INC-1234
fsv tickets approvals INC-1234
fsv tickets associations INC-1234
fsv tickets download INC-1234 --all --dir ./downloads
fsv tickets create --dry-run
fsv tickets add-note INC-1234 --body 'Investigation started' --dry-run --json
fsv tickets reply INC-1234 'Working on this' --dry-run
fsv tickets resolve INC-1234 --note 'Resolved' --dry-run
fsv tickets assign INC-1234 --agent alice@example.com --dry-run
fsv tickets tasks list INC-1234
fsv tickets tasks add INC-1234 --title 'Verify service' --dry-run
fsv tickets watchers list INC-1234
fsv tickets watchers set INC-1234 --watch --dry-run
fsv tickets assets list INC-1234
fsv tickets columns list
fsv tickets canned folders
fsv tickets scenarios list
fsv tickets private-views show -h
fsv tickets time list INC-1234
```

## Service requests, problems, changes

```bash
fsv sr catalog
fsv sr fields 77
fsv sr create 77 --dry-run
fsv sr get SR-1234
fsv sr approvals list SR-1234
fsv sr assets list SR-1234
fsv problems views
fsv problems ls --where status=Open --json
fsv problems get PRB-1234
fsv problems tasks PRB-1234
fsv problems note PRB-1234 --body 'Investigating' --dry-run
fsv problems link-incident PRB-1234 INC-1234 --dry-run
fsv problems status PRB-1234 --to Open --dry-run
fsv problems assets list PRB-1234
fsv problems columns list
fsv changes views
fsv changes ls --where status=Open --json
fsv changes get CHN-1234 --all-tabs
fsv changes activity CHN-1234 -n 20
fsv changes tasks CHN-1234
fsv changes approvals CHN-1234
fsv changes state CHN-1234
fsv changes create --dry-run
fsv changes tasks-add CHN-1234 --title 'Verify service' --dry-run --json
fsv changes associations ls CHN-1234
fsv changes associations add CHN-1234 INC-1234 --dry-run
fsv changes assets list CHN-1234
fsv changes columns list
fsv changes watchers list CHN-1234
```

## Inventory, solutions, attachments, and forms

```bash
fsv assets search 'Acme'
fsv solutions create --title 'Restore service' --body 'Restart the service' --type workaround --dry-run
fsv attachments delete 123 --owner tickets/INC-1234 --dry-run
fsv form fields changes --choices Priority
fsv form explain changes --set 'Priority=High'
fsv form dump changes > snapshot.json
fsv form diff snapshot.json
```

A metadata snapshot may contain account configuration; keep it private. See [forms](forms.md), [views](views.md), and [reviewed operations](reviewed-parity.md).

## Complete syntax lookup

Every public command below has a safe `-h` example. Run it to see its real arguments and options, including the review requirements for that exact operation. Examples above show common operations; help does not require a login or send a request.

### help

```bash
fsv help -h
```

### search

```bash
fsv search -h
```

### setup

```bash
fsv setup -h
```

### tui

```bash
fsv tui -h
```

### auth

```bash
fsv auth -h
fsv auth login -h
fsv auth logout -h
fsv auth status -h
```

### assets

```bash
fsv assets -h
fsv assets search -h
```

### solutions

```bash
fsv solutions -h
fsv solutions create -h
```

### completion

```bash
fsv completion -h
fsv completion show -h
fsv completion install -h
fsv completion refresh -h
fsv completion doctor -h
```

### config

```bash
fsv config -h
fsv config list -h
fsv config get -h
fsv config set -h
fsv config defaults -h
fsv config defaults show -h
fsv config defaults set -h
fsv config defaults clear -h
```

### cache

```bash
fsv cache -h
fsv cache status -h
fsv cache refresh -h
fsv cache clear -h
```

### journal

```bash
fsv journal -h
fsv journal ls -h
fsv journal tail -h
fsv journal path -h
```

### changes

```bash
fsv changes -h
fsv changes ls -h
fsv changes get -h
fsv changes activity -h
fsv changes tasks -h
fsv changes tasks-add -h
fsv changes approval-request -h
fsv changes approval-respond -h
fsv changes approval-cancel -h
fsv changes tasks-update -h
fsv changes tasks-delete -h
fsv changes approvals -h
fsv changes state -h
fsv changes export -h
fsv changes asset-picker -h
fsv changes planning-text -h
fsv changes fields -h
fsv changes lookup -h
fsv changes views -h
fsv changes search -h
fsv changes url -h
fsv changes update -h
fsv changes download -h
fsv changes clone -h
fsv changes create -h
fsv changes notes -h
fsv changes add-note -h
fsv changes associations -h
fsv changes associations ls -h
fsv changes associations search -h
fsv changes associations add -h
fsv changes associations remove -h
fsv changes associations pick -h
fsv changes assets -h
fsv changes assets list -h
fsv changes assets add -h
fsv changes assets remove -h
fsv changes assets search -h
fsv changes columns -h
fsv changes columns list -h
fsv changes columns set -h
fsv changes watchers -h
fsv changes watchers list -h
fsv changes watchers set -h
```

### tickets

```bash
fsv tickets -h
fsv tickets ls -h
fsv tickets get -h
fsv tickets activity -h
fsv tickets export -h
fsv tickets note-edit -h
fsv tickets note-delete -h
fsv tickets assign -h
fsv tickets pick-up -h
fsv tickets merge -h
fsv tickets resolve -h
fsv tickets close -h
fsv tickets reopen -h
fsv tickets status -h
fsv tickets bulk-priority -h
fsv tickets fields -h
fsv tickets lookup -h
fsv tickets views -h
fsv tickets search -h
fsv tickets url -h
fsv tickets add-note -h
fsv tickets create -h
fsv tickets approvals -h
fsv tickets associations -h
fsv tickets update -h
fsv tickets download -h
fsv tickets conversations -h
fsv tickets reply -h
fsv tickets forward -h
fsv tickets assets -h
fsv tickets assets list -h
fsv tickets assets add -h
fsv tickets assets remove -h
fsv tickets columns -h
fsv tickets columns list -h
fsv tickets columns set -h
fsv tickets watchers -h
fsv tickets watchers list -h
fsv tickets watchers set -h
fsv tickets tasks -h
fsv tickets tasks list -h
fsv tickets tasks add -h
fsv tickets tasks update -h
fsv tickets tasks status -h
fsv tickets tasks delete -h
fsv tickets private-views -h
fsv tickets private-views show -h
fsv tickets private-views write -h
fsv tickets canned -h
fsv tickets canned folders -h
fsv tickets canned list -h
fsv tickets canned show -h
fsv tickets canned reply -h
fsv tickets scenarios -h
fsv tickets scenarios list -h
fsv tickets scenarios show -h
fsv tickets scenarios apply -h
fsv tickets time -h
fsv tickets time list -h
fsv tickets time write -h
```

### problems

```bash
fsv problems -h
fsv problems ls -h
fsv problems get -h
fsv problems activity -h
fsv problems tasks -h
fsv problems export -h
fsv problems link-incident -h
fsv problems unlink-incident -h
fsv problems note -h
fsv problems status -h
fsv problems create -h
fsv problems write-child -h
fsv problems link-solution -h
fsv problems unlink-solution -h
fsv problems mark-known-error -h
fsv problems planning-text -h
fsv problems fields -h
fsv problems lookup -h
fsv problems views -h
fsv problems search -h
fsv problems url -h
fsv problems update -h
fsv problems download -h
fsv problems notes -h
fsv problems add-note -h
fsv problems assets -h
fsv problems assets list -h
fsv problems assets add -h
fsv problems assets remove -h
fsv problems columns -h
fsv problems columns list -h
fsv problems columns set -h
```

### attachments

```bash
fsv attachments -h
fsv attachments delete -h
```

### form

```bash
fsv form -h
fsv form explain -h
fsv form fields -h
fsv form dump -h
fsv form diff -h
```

### sr

```bash
fsv sr -h
fsv sr catalog -h
fsv sr fields -h
fsv sr create -h
fsv sr get -h
fsv sr assets -h
fsv sr assets list -h
fsv sr assets add -h
fsv sr assets remove -h
fsv sr approvals -h
fsv sr approvals list -h
fsv sr approvals request -h
fsv sr approvals approve -h
fsv sr approvals reject -h
fsv sr approvals cancel -h
fsv sr approvals remind -h
```
