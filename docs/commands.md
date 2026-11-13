# Command cheat sheet

## Captured-write verification boundaries

New bulk/relationship/time/task/requested-item inventories validate raw pages before the display
paginator can discard malformed rows or deduplicate identities. Duplicate IDs,
conflicting totals, invalid continuation flags and incomplete histories block
both authorization and verification. Initial-owner readback checks the hash-bound resolved requester ID when the
server exposes requester_id rather than echoing email; conflicting exposed
requester representations fail verification. Submitted dates use semantic instant
comparison, and returned owner IDs are journaled before independent readback.
Explicit sender readback compares the
email identity (the server can add a display name), not configured-mailbox
ownership. Recipient aliases must agree when simultaneously exposed.
Initial change uploads retain prior create-hash ambiguity guards without an
expiry/bypass; uncertain older transactions cannot be retried by upgrading the
review format. `changes create --max-bytes` requires `--attach-flow before-create`
rather than silently ignoring a limit on the legacy after-create route.


## Remaining accepted-wire blockers

The supplied capability ledgers do not establish the following writes. These are
**not implemented**, not guessed from public APIs, metadata choices, or usage:

| Remaining scope | Evidence boundary / required capture |
| --- | --- |
| Ticket/SR closure with planned dates and conditional required answers | No accepted combined closure/date/branch submission plus matching rule metadata and exact-record readback. Standalone lifecycle/property writes do not establish this combination. |
| Alert-to-incident actions or alert association add/remove | Usage proves attached alerts, not a manual action contract. Need exact inventory, alert/incident identity joins, accepted mutation and independent readback. |
| Arbitrary custom person/lookup branches and unsupported field kinds | Need accepted lookup resolution and exact branch create/update payload/readback; no fallback opaque IDs. |
| Inline-image upload on replies/notes | Successful upload response and final HTML are recorded, but upload request body/media type were omitted. No bodyless or multipart upload is assumed. |
| Existing-project association | The accepted project add is create-and-associate only. Existing-project linking needs its own wire. Project-task add/remove and project unlink are separately implemented. |
| Native ticket/change templates | Empty native inventories; a local preset is not native application evidence. |
| Reply-all, editable reply To, alternate sender configuration | No accepted variant in the supplied fixtures. Explicit CLI sender remains user-asserted, not mailbox discovery. |
| Maintenance/window create or association | Matching response was empty; backend/window controls did not yield an accepted association. A matching POST is not a write contract. |
| Native clone count/options | No native clone control or accepted options/count payload. Existing local clone utilities are not certified by these captures. |
| Approval delegation/group editing and uncaptured recipient kinds | No accepted mutation and exact approval-chain readback. |
| Combined BCC+CC/status, forward files/CC/BCC/status, cloud inserts | Existing separate variants do not prove these combinations. |
| Problem time update, running timers/other owners/date edits | Only problem create/delete and change manual CRUD are established here. |
| General task custom updates, ticket-task due/reminder/effort variants | Problem optional-field evidence is not a ticket-task contract; generic custom fields are not enabled. |

New scope stays CLI-only; no main-TUI parity, notification suppression, physical
inline-storage deletion, native all-pages bulk flag, or live-tenant certification
is claimed. Tests use synthetic offline transports; no live writes are required.


## Change/problem time and problem-task writes

```sh
fsv changes time list CHN-123 --json
fsv changes time write CHN-123 create --spent 00:01 --executed-at 2026-01-01T09:00:00+00:00 --note 'Acme work' --billable --dry-run --json
fsv changes time write CHN-123 update --entry 901 --spent 00:02 --note Updated --non-billable --dry-run --json
fsv changes time write CHN-123 delete --entry 901 --dry-run --json
fsv problems time write PRB-123 create --spent 00:01 --executed-at 2026-01-01T09:00:00+00:00 --note 'Acme work' --billable --dry-run --json
fsv problems time write PRB-123 delete --entry 902 --dry-run --json
fsv problems task-write PRB-123 71 update --title 'Acme task' --note Updated --planned-effort 1h30m --due-date 2026-01-03T09:00:00+00:00 --dry-run --json
fsv problems task-write PRB-123 71 complete --dry-run --json
fsv problems task-write PRB-123 71 delete --dry-run --json
```

Send with `--yes --expect-plan FULL_HASH`. Time is self-owned/manual only;
create defaults billable (as captured), and change update can set billable.
Durations/instants are normalized only for independent v2 readback. Problem time
update, timer actions, other-owner time and executed-date edit remain blocked.
Problem task update also accepts runtime status/owner/group/reminder and planned
start/end. No inferred task custom-field or null-clearing contract. Exact native
task GET verifies owner_id, reminder, dates, description and semantic effort.
Completion is status-only with fresh closure requirements; the server may rewrite
planned dates and stop OLA timers. Delete uses native PUT tasks/update with its
captured string task identity, then complete inventory readback (never guessed
HTTP DELETE). Unknown or unverified operations must not be replayed.


## SR requested-item writes

```sh
fsv sr items list SR-123 --json
fsv sr items write SR-123 edit --item 901 --answers-json '{"title":"Acme request","details":"Updated details"}' --dry-run --json
fsv sr items write SR-123 stage --item 901 --stage 2 --dry-run --json
fsv sr items write SR-123 add --catalog-item 71 --answers-json '{"title":"Acme request","details":"Additional item"}' --dry-run --json
fsv sr items write SR-123 remove --item 902 --dry-run --json
```

Send with `--yes --expect-plan FULL_HASH`. Answers are changed API-name plain
text/paragraph fields only, validated through fresh catalog metadata and rules.
Nonempty/unavailable exact-item editability restrictions fail closed.
Only stage 2 (observed Delivered in the accepted capture) is enabled; other stage
IDs/labels require evidence, not inference. Answer and stage writes are separate.
Removal requires an explicitly non-parent item; add preserves the captured
catalog display-ID string wire. Internal item IDs, catalog display IDs and
requested-item IDs are distinct. Readback uses complete exact-ticket inventory
and exact-item detail, checking every submitted answer and preserving other items.


## Change relationships

```sh
fsv changes relationships list CHN-123 caused-incident --json
fsv changes relationships write CHN-123 caused-incident add --item 71 --dry-run --json
fsv changes relationships write CHN-123 problem remove --item 81 --dry-run --json
fsv changes relationships write CHN-123 release add --item 91 --dry-run --json
fsv changes relationships write CHN-123 project-task add --item ACME-1 --project-key ACME --name 'Acme task' --dry-run --json
fsv changes relationships write CHN-123 project remove --item ACME --dry-run --json
```

Kinds: `caused-incident`, `problem`, `release`, `project`, `project-task`.
Use `write ... remove` for unlink and `list` for exact association inventories.
Send with `--yes --expect-plan FULL_HASH`. Project proxy writes validate nested
status/body and independent linked inventories; incomplete/multiple pages fail
closed. Task add takes an explicit existing task key and name, not a task create.
The project must already be linked. `project add --project-json` explicitly
**creates and associates** a new private project; JSON requires `name`,
`description`, `start_date`, `end_date`, `owner_email`, `project_category` text.
It is not an existing-project link: that wire is absent and remains blocked.
A project create is not rolled back if association/readback fails; never replay.
Caused incidents use their own associated-caused filter, never the candidate list.
Release unlink uses PUT, not DELETE. No maintenance-window contract is inferred.


## Frozen bulk ticket actions

```sh
fsv tickets bulk status INC-123 INC-124 --status Pending --dry-run --json
fsv tickets bulk assign INC-123 INC-124 --agent agent@example.com --group 'Acme Desk' --dry-run --json
fsv tickets bulk pickup INC-123 INC-124 --dry-run --json
fsv tickets bulk merge INC-123 INC-124 --primary INC-123 --note-primary Combined --note-secondary Merged --dry-run --json
fsv tickets bulk spam INC-123 INC-124 --dry-run --json
fsv tickets bulk unspam INC-123 INC-124 --dry-run --json
fsv tickets bulk delete --where 'Priority=Low' --select-all --dry-run --json
```

Send with `--yes --expect-plan FULL_HASH`. Spam and delete additionally require
`--confirm-destructive`; every invocation prints the exact ID list on stderr.
Filtered selection uses the supported list-query builder, complete pagination,
and at most 100 unique IDs; it expands to explicit IDs, not an unproven native
all-pages mutation flag. Rebuild checks selection/metadata/records before send.
No chunking, ambiguous retry, null assignment or bulk closure. Group and agent
must both be supplied for the accepted assignment variant. Pickup and merge
reuse their existing reviewed planners, durable guards and independent readback.


## Exact-conversation forward

`fsv tickets forward INC-123 'Acme update' --conversation 901 --to user@example.com
--sender support@example.com --dry-run --json` reviews the captured ticket forward
wire with an integer `conversation_id`. The source must exist exactly once in a
complete history; source edits invalidate the review. Send using
`--yes --expect-plan FULL_HASH`. Source selection is bound to the review; returned
conversations do not expose source identity, so readback verifies delivery fields,
not an independently returned source link (`source_link_verified: false` is
reported separately). No attachment/CC/BCC/status combinations.


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
fsv tickets ls --where 'created_at>=2030-01-01' --where 'created_at<=2030-01-31' --json
fsv tickets ls --where 'created_at=last:one_week' --where 'Impact Score>=3' --json
fsv tickets ls --where status=Open --where priority=High --or --json
fsv tickets ls --view 'All Tickets'
fsv tickets get INC-1234 --json
fsv tickets activity INC-1234
fsv tickets fields --refresh
fsv tickets search 'Acme service'
fsv tickets conversations INC-1234
fsv templates list
fsv tickets create --template 'Acme outage' --requester jane@example.com --dry-run
fsv tickets add-note INC-1234 --body 'FYI' --notify-agent 'Robin Agent' --dry-run
fsv tickets add-note INC-1234 --body 'See screenshot' --inline-image shot.png --dry-run
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
fsv tickets private-views list
fsv changes private-views list
fsv tickets problems link INC-1234 PRB-7 --dry-run
fsv tickets changes link INC-1234 CHN-9 --kind initiating --dry-run
fsv tickets children link INC-1234 INC-1235 --dry-run
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
fsv problems delete PRB-12 --dry-run
fsv problems restore PRB-12 --dry-run
fsv problems delete-forever PRB-12 --confirm-destructive --yes --expect-plan FULL_HASH
fsv solutions delete-forever 1234 --dry-run
fsv problems get PRB-1234
fsv problems tasks PRB-1234
fsv problems note PRB-1234 --body 'Investigating' --dry-run
fsv problems link-incident PRB-1234 INC-1234 --dry-run
fsv problems status PRB-1234 --to Open --dry-run
fsv problems assets list PRB-1234
fsv problems columns list
fsv changes views
fsv changes ls --where status=Open --json
fsv changes windows list
fsv changes windows create --name 'Acme freeze' --start '2030-11-21 10:00' --end '2030-11-21 12:00' --criteria change_type=Normal --dry-run
fsv changes windows create --name 'Acme freeze' --start '2030-11-21 22:00' --end '2030-11-22 00:00' --timezone Asia/Tokyo --criteria change_type=Normal --dry-run
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

`<module> assets list` probes the tenant with read-only GETs (404 means
absent): the integrated-ITAM `<module>/<id>/associated-cis` is used when it
exists, else the legacy list (`tickets/<id>/associated_assets`,
`changes/<id>/assets`); neither fails closed. The JSON names the `driver`.

## Inventory, solutions, attachments, and forms

```bash
fsv assets search 'Acme'
fsv solutions create --title 'Restore service' --body 'Restart the service' --type workaround --dry-run
fsv attachments delete 123 --owner tickets/INC-1234 --dry-run
fsv form fields changes --choices Priority
fsv form explain changes --set 'Priority=High'
fsv form dump changes > snapshot.json
fsv form diff snapshot.json
fsv form fields problems
fsv form explain sr --item 77 --set 'Access level=Admin'
fsv form dump sr --item 77 > item77.json
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
fsv changes private-views -h
fsv changes private-views list -h
fsv changes private-views show -h
fsv changes private-views write -h
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
fsv changes assets candidates -h
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
fsv tickets assets candidates -h
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
fsv tickets problems -h
fsv tickets problems list -h
fsv tickets problems candidates -h
fsv tickets problems link -h
fsv tickets problems unlink -h
fsv tickets changes -h
fsv tickets changes list -h
fsv tickets changes candidates -h
fsv tickets changes link -h
fsv tickets changes unlink -h
fsv tickets children -h
fsv tickets children list -h
fsv tickets children candidates -h
fsv tickets children link -h
fsv tickets children unlink -h
fsv tickets private-views -h
fsv tickets private-views list -h
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
fsv problems assets candidates -h
fsv problems assets add -h
fsv problems assets remove -h
fsv problems columns -h
fsv problems columns list -h
fsv problems columns set -h
```

`problems update --set` nests custom fields in `custom_fields` (only the
changed keys) and refuses a deleted (trashed) problem before sending; the
server answers 405 there. `problems status` reads back the status: a
workflow that moves it on right after the write (or back, when the first
activity page shows the workflow) is `verified` with `verified_with_note`.
`problems create` sends only the captured create keys and names any other
top-level field it would need; set those with `problems update` afterwards.
An `update --dry-run` on a cached schema makes one GET of the form fields
and warns (`warnings`) when the server has choices the cache lacks; run
`fsv <module> fields --refresh` then.

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
fsv sr assets candidates -h
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
