# Live-testing methodology

**What this is.** How the Freshservice knowledge in
[docs/freshservice/](freshservice/overview.md) was established and how to
extend it safely: reviewed writes, the blocked-submit capture protocol for
production-like tenants, replicating a production configuration on a trial
tenant, throwaway-record discipline and cleanup, how to read evidence, and
how to probe a new tenant.

**Who it is for.** People and coding agents who test `fsv` (or a similar
client) against real Freshservice tenants.

**Verified on:** a Freshservice Enterprise trial tenant, 2026-11-11 to
2026-11-13 (live writes), and a production-like enterprise tenant (GET
reads and blocked captures only, nothing sent). The procedures below are the
ones that were used; statements about platform behaviour carry the tags
**[VERIFIED LIVE]**, **[CAPTURED]** or **[INFERRED]** as in the knowledge
base.

## 1. Ground rules

1. **Two kinds of tenant.** A *test tenant* (a trial you own) may receive
   real writes. A *production-like tenant* receives **GETs only**; any write
   evidence there comes from blocked captures (section 4) where nothing
   reaches the server.
2. **Explicit authorisation per batch.** Write down which tenant, which
   commands and which records a batch may touch before starting.
3. **Self only.** Every requester, agent, owner, approver, CC, BCC, notify
   and forward recipient is the tester's own identity. Approvals, replies,
   forwards and reminders send real email.
4. **Throwaway records only.** Create the records you mutate; prefix
   subjects with `TEST ONLY`. Keep a list of protected records that must
   never be targets, and compare them before and after the batch.
5. **No secrets in output.** Never print request headers, cookies, CSRF
   tokens or session files. Read captured requests through an allowlist of
   method, path, query and body. Redact opaque tokens such as
   `br_validation_excludes` and signed URL queries.
6. **Stop on surprise.** If a write touches something outside the batch,
   stop, record exactly what happened, and do not try to "fix" it with more
   writes.

## 2. Isolate the client

* Give each tenant its own configuration and cache:
  `FSV_CONFIG_DIR=~/.config/fsv-trial FSV_CACHE_DIR=~/.cache/fsv-trial`.
  A test must never fall back to another tenant's session or metadata.
* Clear metadata caches before a batch (`fsv cache clear all`) so admin
  changes (rules, lifecycles, statuses) are picked up.
* Run non-interactively (`fsv --no-input ...`) so a prompt cannot hang or
  be answered by accident.
* To run a dry-run against a tenant you may not write to, wrap the client's
  request method so any non-GET raises before sending, then run the
  command's `--dry-run`. The dry-run must still build its full plan from
  live metadata.

## 3. Reviewed writes

Every reviewed `fsv` write is two invocations with **identical arguments**:

```bash
fsv --no-input tickets add-note INC-42 --body 'TEST ONLY note' --dry-run --json
# read the plan: target, payload, recipients, warnings, plan_hash
fsv --no-input tickets add-note INC-42 --body 'TEST ONLY note' --yes --expect-plan FULL_HASH --json
```

* The plan hash binds the exact payload and the reviewed state of the
  target and metadata. If anything changed between the dry-run and the
  send (record, rules, recipients, metadata), the send is refused before
  any request; dry-run again. [VERIFIED LIVE]
* Destructive operations additionally need `--confirm-destructive`;
  bulk operations take `--max-records N`, and the review lists every
  selected id. [VERIFIED LIVE]
* Every write is journaled before it is sent (`fsv journal tail`). A second
  send of an identical plan that the journal records as sent, or with an
  unknown outcome, is refused: change the intent (or inspect and resolve
  the old one) instead of replaying. [VERIFIED LIVE]
* The result reports `verified: true`, `verified: false` with mismatched
  fields, or an unknown outcome. Fields the server never echoes are
  reported as *unavailable*, separately from mismatches.
* **After an unknown or failed outcome: read the exact record and the
  journal; never resend.** A timeout, a 5xx or a 403 after sending can
  hide a successful write.
* Selections built from filters are a hazard: one batch's subject filter
  expanded to unrelated records and a bulk write was sent to them. Before
  sending any filtered bulk plan, check that every selected id is in the
  batch's allowlist and satisfies the predicate. [VERIFIED LIVE]

## 4. Blocked-submit capture protocol

Use this to learn the exact request a production-like tenant's UI sends for
a write without delivering it. The result is **CAPTURED** evidence: the
request shape only, no server response and no side effects.

1. **Own session.** Use an isolated browser session and your own tab. Never
   drive a tab another person or agent is using.
2. **Block autosaves first.** Before navigating to any ticket page, route
   `**/reply_draft` to abort; ticket pages autosave a reply draft on load.
3. **Preload.** While the network still works, open the target dialog and
   load everything it needs through GETs: pickers, lookups, CAB or group
   members, templates, asynchronous defaults. Wait for real choices, not
   just a visible heading.
4. **Abort everything.** Install a global route that aborts every request
   (`**/*`).
5. **Prove the abort.** Run a harmless authenticated same-origin GET from
   the page, for example `fetch('/api/v2/agents/me', {credentials:
   'include', cache: 'no-store'})`. It must fail ("Failed to fetch"). If it
   succeeds, stop.
6. **Fill.** Enter synthetic `TEST ONLY` content; select only yourself as
   recipient. Rich editors append or prepend on fill: select all, insert,
   and check the exact final text.
7. **Submit precisely.** Click the submit button of the visible dialog you
   mean (scope by visible dialog, print the button's ancestor first). Never
   click by bare text: hidden vendor dialogs (feedback, demo, chat) have
   their own Submit buttons.
8. **Read the aborted request by exact path.** Filter the recorder by the
   expected method and path first, then read only the body. Exclude
   telemetry (`metrics`, `languages/detect`) and session-refresh routes.
   Persist a sanitised contract immediately: browser recorders lose older
   bodies after navigation.
9. **Close with the abort still active.** Then, from a normal session,
   verify with GETs that nothing changed (target `updated_at`, child
   collections, the newest activity entry).
10. **Record the boundaries.** An aborted first stage means a dependent
    second stage (for example a link after an article create, or a status
    change after a resolution-note save) was never emitted: report it as
    blocked, never invent it. Repeated clicks after an abort may resend
    the same forward action because the UI rolled back optimistically.

What a capture does **not** prove: that the server accepts the body, the
response shape, side effects (emails, workflows, SLA), or any variant you
did not click. [VERIFIED LIVE]

<a id="trial-replication"></a>

## 5. Trial replication

A fresh trial accepts almost any well-formed write because it has no rules,
locks or automations. To test behaviour that matters on a production-like
tenant, **replicate the production configuration on the trial first**.

What to replicate, in order of value:

| Configuration | Read it from (production-like tenant, GET) | Why it matters |
|---|---|---|
| Change lifecycles per change type: states, transitions, entry criteria, field locks | `matching-lifecycle-info?change_type=N`, `change_management/<flow>?include=state_info` | transition pre-checks and lock handling |
| Business rules for ticket new/edit, change, problem and the main catalog items | `ws/<n>/business_rules` per module and form type | required/hidden sets, `current_entity` and `enable` handling |
| Workflow automators that rewrite status, add tasks, stamp fields, move on approval | the activity feeds of recent records (admin pages are often 403) | readback tolerance and two-phase writes |
| SLA policies and business hours | due-date offsets observed on records; admin pages where readable | due-date side effects |
| CABs and agent groups (members: the tester only) | `GET /api/_/cabs`, `cab-members` | approval recipient expansion |
| Status and choice lists by **label** | form-field metadata | ids will differ on the trial, which tests id-independence |
| Problem closure rules | 400 responses (the rules are not readable) | closure pre-checks |
| Representative catalog items (one per field type) | catalog item metadata | SR form coverage |

How:

* Map production ids to trial ids **by label**, and keep the mapping in a
  private log. Never copy production names into public files.
* Export any lifecycle flow before editing it, and note which records it
  binds; a damaged flow made state reads fail. [VERIFIED LIVE]
* Recipients in replicated workflows and approvals are the tester only; no
  external email.
* Some differences cannot be replicated because they are account features
  (parallel approvals on or off, legacy vs integrated asset model).
  Clients must detect and support both. [VERIFIED LIVE]

## 6. Throwaway records and cleanup

* Create fixtures through the client under test where possible (dry-run,
  then send), so fixture creation is itself a test.
* Tag batch fixtures uniquely (a tag or subject suffix) so a filtered
  inventory can be checked against the allowlist.
* Keep a cleanup ledger while you work: id, how it was created, how it will
  be removed.
* Clean up through reviewed commands or the native UI, and verify by exact
  readback (`deleted: true`, 404 after delete-forever) **and** absence from
  the complete list. Check the HTTP status before parsing JSON: a deleted
  target can answer an HTML 404.
* Some things cannot be cleaned up; report them as left behind:
  - approval history (approved, rejected and cancelled entries stay);
  - sent emails and notifications;
  - merged tickets (no unmerge);
  - inline images (no delete route);
  - records in trash when no delete-forever path is used;
  - anything created on the vendor side by a mis-click.
* Re-check the protected records at the end (full JSON equal to the start,
  ignoring signed URLs).

## 7. Reading evidence

* **A 2xx is not proof.** Verify with an independent read of the exact
  target (exact conversation, exact task, complete association list).
  A merge precheck answers 200 and does nothing; a bulk unassign answers
  200 and does nothing. [VERIFIED LIVE]
* **Captured is not accepted.** Keep CAPTURED and VERIFIED LIVE apart in
  reports, per capability; metadata and UI options do not prove a write.
* **Source code is not acceptance evidence.** A key found in the shipped web
  JavaScript was rejected by the server. [VERIFIED LIVE]
* **UI requiredness is not server requiredness** (and the reverse): compare
  metadata flags, rules, UI validation and server errors separately.
* **Timing.** Saves and bulk actions are asynchronous and browsers cache
  `/api/v2` reads; re-read with no-store before calling a mismatch.
* **Workflows.** A changed value after a write may be a workflow; check the
  activity feed for your entry and a newer workflow entry.
* **Lost bodies.** Recorder response bodies can disappear after navigation.
  Never reconstruct them; rely on independent reads.
* **Time zones.** Responses use the account's offset; compare instants.

## 8. Probing a new tenant

Do not hardcode endpoints, ids or feature assumptions; probe with GETs and
record the answers.

| Question | Probe | Interpretation |
|---|---|---|
| Who am I, which workspaces, which privileges? | `GET /api/v2/agents/me`, `GET /api/_/bootstrap/me`, `GET /api/_/workspaces` | ids and privileges for every later step |
| Plan and features | `GET /api/_/bootstrap/account` | flag names are hints only |
| Form metadata per module | `GET /api/_/ws/<n>/<module>_form_fields`, then the plain path | which path family the tenant answers |
| Business rules per module and form | `GET /api/_/ws/<n>/business_rules?...` | 403/404 unavailable; a persistent 5xx is "unavailable", not "empty" |
| Change lifecycles | `matching-lifecycle-info` for each change type | `state_flow: {}` = no lifecycle |
| Approval model | `GET /api/_/changes/<id>/approval-chains` on any change | 400 "Parallel approvals feature needs to be enabled" = legacy model |
| Asset model | `GET /api/_/assets-to-associate?...` and `GET /itam/core/device/api/meta/` | 403 `require_feature` on one side tells you the other |
| Templates, windows, CABs | `GET /api/_/ticket-templates`, `GET /api/_/maintenance_windows`, `GET /api/_/cabs` | empty is not "unsupported"; 403 is role-dependent |
| List encodings | a read-only `--debug` list with a known filter | check the rows satisfy it; watch for 206 `search_error` |

Then:

* Run every write command's `--dry-run` against the new tenant with writes
  blocked (section 2) and read the plans.
* Add the tenant's shapes to offline tests as a *shifted* synthetic tenant
  (different status ids, reminder values, workspaces) rather than adding
  literals; see [contributing for agents](contributing-agents.md).
* Space your probes; bursts of GETs hit HTTP 429.

## 9. Reporting a batch

A useful batch report lists, per command: the exact invocation (with
synthetic placeholders), PASS or FAIL, the independent evidence, and
whether a failure is a client bug or tenant behaviour. It ends with what
was cleaned up, what was left behind and why, and any deviation from the
ground rules. Keep tenant-specific reports private; move only
platform-generic, sanitised findings into this knowledge base.
