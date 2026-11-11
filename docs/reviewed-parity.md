# Reviewed operations and metadata freshness

Change legacy flags use the same form engine, business rules, changed-field
payload, duplicate journal, and independent readback as `--set`:

```sh
fsv changes update CHN-123 --status Planning --dry-run --json
fsv changes update CHN-123 --status Planning --yes --expect-plan REVIEWED_HASH
```

Review the full JSON payload before sending. A mismatched hash refuses the
write. Dry runs do not send mutations. A same-status edit is a no-op.

Ticket and service-request property forms have an explicit reviewed mode:

```sh
fsv tickets update INC-123 --reviewed --set 'Subject=Acme maintenance' --priority High --dry-run
fsv tickets update INC-123 --reviewed --set 'Subject=Acme maintenance' --priority High --yes --expect-plan REVIEWED_HASH
```

It resolves choices and lookups from fresh metadata, evaluates edit rules, and
supports the captured subject/description/requester/assignment, category/type,
priority/urgency/impact/source/status and runtime custom-field update shape.
Tags and planned dates can be included in the same review. Uncaptured core
fields and combined Resolved/Closed workflows are refused. Readback reports
missing fields as unavailable, not verified. Legacy basic update flags remain
available; `--reviewed` selects this stricter path explicitly. A real runtime
`default_requested_for` field uses the separately captured SR
`requested_for_id` update key; this does not enable that field for change or
incident creation. Beneficiary edits discard the previous person's rule
attributes; missing exact beneficiary-dependent context blocks instead of
reusing actor/old-person metadata.

The main TUI's **e** key on a ticket/SR opens a metadata-backed property
picker, then a value/choice picker and exact before/after/hash review. Each
review changes one property. Choices come from current metadata; unsupported
and synthetic fields are not offered. Results remain in a scrollable outcome
screen. Combined multi-field required transitions should use the reviewed CLI
or the web form, not a sequence of single-field edits.

## BCC replies and whole-ticket forwards

```sh
fsv tickets reply INC-123 'Acme update' --bcc person@example.com --sender support@example.com --dry-run
fsv tickets forward INC-123 'Acme update' --to person@example.com --sender support@example.com --dry-run
```

Both require `--yes --expect-plan FULL_HASH` to send after review. Recipients
must resolve uniquely and are checked again before sending. `--sender` must
be the configured support mailbox: it is explicitly user-asserted, never
inferred or certified against an unproven mailbox endpoint. BCC and forward
payloads have blocked-wire evidence, not accepted-send proof. Whole-ticket
forward is not conversation forwarding; BCC+CC/status, alternate-To, and
reply-all remain refused or unsupported. Missing BCC/sender readback fields
remain unavailable, and a successful send is never replayed to obtain them.

The reviewed property and reply/note-Pending paths also have real
`Client`/`httpx.MockTransport` tests with distinct Acme/Globex schemas and
runtime status IDs. They check exact HTTP bytes/headers, readback identity,
missing/contradictory fields, stale metadata and 403/429/500/disconnect guards.
CC display/email resolution is rechecked before send without changing the
reviewed payload. These are offline transport proofs, not accepted live sends.

## Structured output

Watchers (`list`/`set`), time (`list`/`write`), `planning-text`,
`mark-known-error`, `bulk-priority`, and ticket `forward` accept `--json`
and `--output json` (also `-o json`). Their existing JSON default and
review/result schemas are unchanged. Other formats are rejected before a client
is opened; these commands do not flatten structured reviews into tables or CSV.
Output options never bypass `--dry-run` or the full-hash send gate.

## Canned responses and scenarios

```sh
fsv tickets canned folders --json
fsv tickets canned list --folder 702 --output json
fsv tickets canned show 701 --json
fsv tickets canned reply INC-123 --response 701 --content-pointer /canned_response/content --sender support@example.com --dry-run --json
fsv tickets scenarios list --json
fsv tickets scenarios show 703 --json
fsv tickets scenarios apply INC-123 --scenario 703 --dry-run --json
```

Inspect `show` before choosing the content JSON pointer; detail key names are
not assumed portable. Canned insertion sends only the selected literal HTML in
the ordinary reviewed reply body, with no canned-ID wire field. Placeholders
are refused, not expanded. Attachments/signatures are not inserted. Review the
entire body and source hash, then use `--yes --expect-plan FULL_HASH`. The source
is re-read before sending; metadata/content drift blocks the send. Legacy reply
hashes and durable send guards are preserved.

Scenario `show` selects an exact ID from the captured list route, never an
inferred detail endpoint. `apply` uses the captured single-ticket native execute
wire, **not** canned insertion: automation can send mail, mutate records or run
workflows. The definition and exact ticket are hash-bound and refreshed before
a single attempt. Exact ticket readback is attempted, but `verified` remains
false because complete scenario side effects cannot be established. Never
repeat an unverified execution. Both scenario and canned reads expose server
responses without claiming to exhaust any undocumented pagination.

All tests use synthetic sources and offline transports. Blocked browser wires
establish shape only, not accepted live sends or portable template schemas.

## Private ticket views

```sh
fsv tickets private-views show 701 --json
fsv tickets private-views write 701 create --name 'Acme personal queue' --dry-run --json
fsv tickets private-views write 702 edit --name 'Acme renamed queue' --dry-run --json
fsv tickets private-views write 702 delete --dry-run --json
```

`create` clones the source's ordinary query and workspace unchanged; `edit`
renames an exact view. Every action requires fresh self ownership, an explicitly
false default flag, and named private visibility metadata (`Me`, `Only me`, or
`Private`). The numeric visibility enum is taken from that runtime metadata,
never hardcoded. If an account exposes only a number, mutation is **blocked**;
this adapter does not infer privacy from ownership. Synthetic metadata tests do
not prove that every account exposes those labels. Shared/default views,
advanced queries, arbitrary filter edits and other sort modes remain unsupported.

Use `--yes --expect-plan FULL_HASH` after each review. Source/target drift blocks
sending. Creation requires the exact returned view ID and reads that object,
never a same-name or latest match. Deletion requires both exact-target 404 (also
HTML/empty responses) and absence from a complete inventory. Ambiguous write
outcomes keep the durable single-attempt guard. The captured private lifecycle
was accepted, but these adapters were exercised offline only.

## Self watchers

```sh
fsv tickets watchers list INC-123
fsv tickets watchers set INC-123 --watch --dry-run
fsv changes watchers set CHN-123 --unwatch --dry-run
```

Send with `--yes --expect-plan FULL_HASH`. Identity comes from a fresh
signed-in-agent read; arbitrary subscriber changes are not supported. Ticket
and change methods differ and are kept separate. Complete membership is
required before review, and readback compares the full expected membership
including all other subscribers. Already-satisfied requests send nothing.
These exact self-watch lifecycles have accepted-contract evidence; no live
calls are made by the offline tests.

## Manual ticket time

```sh
fsv tickets time list INC-123
fsv tickets time write INC-123 create --spent 00:01 --executed-at 2030-01-02T10:00:00Z --note Acme --dry-run
fsv tickets time write INC-123 update --entry 456 --note Globex --dry-run
fsv tickets time write INC-123 delete --entry 456 --dry-run
```

Use `--yes --expect-plan FULL_HASH` after reviewing each operation. Only
self-owned, nonbillable, stopped manual entries are supported. Durations use
HH:MM and start timestamps require an explicit offset. Updates preserve the
unmodified time/note and do not change start time. Timers, billing, task-linked
time, and change/problem time mutations are not implemented. Reads must be
complete; create verification needs the exact returned entry ID. An absent
response ID or failed readback never triggers a resend or latest-entry guess.
Readback compares HH:MM and HH:MM:SS by seconds and explicit-offset timestamps
by instant, so server normalization does not create false mismatches. Missing
fields remain unavailable; invalid or genuinely different values mismatch.
Note-only updates preserve minute-aligned server durations; an existing duration
with nonzero seconds requires an explicit `--spent` rather than silent rounding.
Task and ticket-property timestamps share the same strict instant comparator;
planning text is verbatim, watchers compare normalized numeric memberships, and
SR recipient fields remain exact (they are not durations or timestamps).

## Main-TUI conversation options

In a ticket/SR note (`n`) or detail-pane reply (`r`), compose the text then
press **Ctrl+O** for recipient/status options. Notes support Notify-to and
Pending; replies support CC and Pending, or the separately captured BCC plus
explicit sender variant. Leave status unchanged by default. Choices and
recipient lookups are evaluated before exact payload/hash review. Public
conversations require a second visibility/mail confirmation. Results stay in
a scrollable outcome screen, including unverified readback and returned IDs;
there is no retry button. **Ctrl+S** keeps the simple compose/review path.
Read-only mode blocks option preparation and sends.

## Explicit-ticket bulk priority

```sh
fsv tickets bulk-priority INC-123 SR-124 --priority High --dry-run
fsv tickets bulk-priority INC-123 SR-124 --priority High --yes --expect-plan REVIEWED_HASH
```

Only captured bulk priority is implemented, with 1–100 explicit unique ticket
IDs and no query/view/all-pages expansion. Fresh per-ticket edit metadata and
rules must agree on the chosen priority enum. Every record/metadata hash is
bound to review, and every target is read independently after the single bulk
request. Partial or missing per-ticket results remain unverified and return
nonzero from the CLI; neither the whole batch nor failed members are retried.
This has blocked-wire/offline transport evidence, not accepted live bulk proof.
Spam/delete/merge/pick-up and scenario execution are not included.

## Problem known-error mark

```sh
fsv problems mark-known-error PRB-123 --dry-run
fsv problems mark-known-error PRB-123 --yes --expect-plan REVIEWED_HASH
```

The main TUI's selected-problem action menu also offers this exact reviewed
mark. This implements only captured `PUT problems/ID {known_error:true}`:
no reversal, resolution/closure, or knowledge publishing is inferred. Exact
problem identity and the boolean field must be readable; an already-marked
problem is a no-op. The fresh record binds the review, independent readback
checks the flag, and ambiguous sends retain a journal guard. Evidence is a
blocked request plus offline tests, not an accepted live lifecycle.

## Native planning text and problem analysis

```sh
fsv changes planning-text CHN-123 --section 'Rollout Plan' --description '<div>Acme plan</div>' --dry-run
fsv problems planning-text PRB-123 --section 'Root Cause' --description '<div>Acme evidence</div>' --dry-run
```

Review, then pass `--yes --expect-plan FULL_HASH`. Change section identities
come from fresh field metadata; problem native sections are Root Cause,
Impact and Symptoms, with the captured semantic section keys. This is not a
custom-field solution editor. Saves use the captured POST with `?id=SECTION`
and a description-only body, never a guessed PUT or array position. Existing
attachments or unavailable attachment inventories block text-only saves.

In the main TUI use **a → Edit native planning/analysis text**, choose the
section, edit the multiline text/HTML, then **Ctrl+S** to review the exact
hash-bound save. Cancel sends nothing; results stay in the outcome screen.
Workaround/permanent-solution/KB publishing and planning-file changes are
not covered by this text-only workflow. Evidence is blocked wire, not an
accepted planning-save lifecycle.

Schema reads use a seven-day cache, including legacy cache envelopes. Stale
reads are identified internally with `stale` and `source`, warn on stderr,
and start a background refresh; JSON stdout remains parseable. Force a
synchronous fetch before inspecting current choices:

```sh
fsv changes fields --choices status --refresh --json
fsv tickets lookup priority High --refresh --json
```

Explicit refresh failures are errors, not silent fallbacks to old definitions.
Reviewed form sends separately re-fetch fields and business rules before
submitting, so the schema display cache is not a write authorization source.
