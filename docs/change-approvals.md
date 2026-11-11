# Reviewed change approvals

`fsv changes approval-request` **requests** manual approval from explicitly named
agents. It does not approve a change, change its status, or trigger its state-flow
transition. Captured rules are **Everyone** (`1`, default), **Anyone** (`2`),
**Majority** (`3`) and **First responder** (`4`), selected with
`--rule everyone|anyone|majority|first-responder`. The review shows the selected
rule and its exact wire value; the rule is hash-bound.

**Approval emails are irreversible; cancelling does not recall sent emails.** Review
the exact recipients, tenant, change, group name and escaped message before
sending. Tenant workflows may also notify people. No implicit self, default
recipients, requester substitution, fuzzy names, recipient groups, HTML input,
attachments or revoke operations are supported by `approval-request`. Reviewed
individual decisions and reviewed group cancellation are described below.

## Review and send

All examples use invented identifiers. First review without sending:

```bash
fsv changes approval-request CHN-1234 --name "Acme review" \
  --message "Please review the Acme rollout." \
  --approver 9000003001 --approver 9000003002 --dry-run --json
```

`--approver` is required and repeatable. Use positive **agent IDs**, not task
owner user IDs. Exact emails such as `reviewer@example.com` are resolved through
the read-only agents autocomplete: exactly one active, exact-email hit must have
an agent `id`. A distinct `user_id` is never substituted. Duplicate recipients
are rejected, including an email and numeric ID resolving to the same agent.
Numeric-ID dry runs are entirely local: no client, session or journal writes.
Email dry runs may perform read-only lookups. Numeric IDs are explicit operator
input, not a claim that agent existence was checked locally.

After inspecting the review, repeat the exact arguments with the **full**
`plan_hash` in `PLAN_HASH`:

```bash
fsv changes approval-request CHN-1234 --name "Acme review" \
  --message "Please review the Acme rollout." \
  --approver 9000003001 --approver 9000003002 \
  --yes --expect-plan "$PLAN_HASH" --no-input --json
```

The hash binds tenant, change display ID, exact group name, selected rule,
escaped message and explicit recipients. A hash prefix or a changed draft fails.
`--yes` requires the matching hash from a prior review; `--no-input` never
prompts. Without `--yes`, a default-No prompt explicitly confirms sending the
request and irreversible emails. Declining cancels without a POST or journal
write (an email recipient may already have required a read-only lookup).
`--read-only` refuses a send before email lookup. A read-only dry run is allowed.

The CLI is registered alongside the existing read command:

```bash
fsv changes approvals CHN-1234 --json
```

There is no main TUI menu action in this change: wiring the action would require
changes outside the isolated new-file/minimal-actions scope. The reviewed sender
is UI-independent, accepts an effective `read_only` flag, and requires a reviewed
hash for a future TUI integration.

## Captured request contract

An isolated UI **Request for approval > Add approvers > Users > Done > Send**
request was first captured with traffic aborted. A subsequent accepted trial
request exposed populated per-member approvals and a new group in approval chains,
including its readable `name`, group ID and approval record IDs. Approve and cancel
were subsequently accepted and checked by read-back; reject remains an aborted
capture. Write responses are never verification evidence. Notification delivery
and email recall are not claimed. The synthetic illustration preserves the request's
method, path and fields:

```http
POST /api/_/changes/1234/approval-groups
Content-Type: application/json

{"approval_groups":[{"name":"Acme review","approval_type":1,"email_content":"<p dir=\"ltr\">Please review the Acme rollout.</p>","notified_to":[9000003001,9000003002]}]}
```

Messages are always plain text escaped with HTML entities inside
`<p dir="ltr">...</p>`. Newlines become `<br>`; input tags are literal text, never
interpreted as markup. Multiline messages and repeated recipients are supported
local extensions of the single-recipient, single-paragraph capture, tested
offline, not claims of accepted live behavior.

### Rule variants and recipient-kind limits

```bash
fsv changes approval-request CHN-1234 --name "Globex review" \
  --message "Please review the rollout." --approver 9000003001 \
  --rule first-responder --dry-run --json
```

All four `approval_type` values were established by intercepted, aborted POSTs.
Everyone also has prior accepted/read-back evidence; the other variants have
browser serialization evidence only, not demonstrated server acceptance or
notification delivery. Repeat exactly the chosen `--rule` when sending with the
review hash; changing the rule invalidates that hash.

There is deliberately **no `--approver-group`**. Agent groups and CAB selection
resolved to explicit `notified_to` member ID lists, with no persisted group/CAB
ID in the captured request. Passing a group ID as an approver would target the
wrong namespace. Use explicit `--approver` agent IDs instead. Requester groups
and impacted-service owners had no usable capture, so no mapping is inferred.
No group expansion, default members or fabricated group wire key is implemented.

## Verification and unknown outcomes

The sender reads the **complete paginated approvals inventory** before sending,
prepares CSRF, persists a content-free `started` journal entry, and reads that
entry back. Failure in preparation prevents the POST. It uses exactly one
lower-level POST, never the helper that replays writes on HTTP 403.

**No POST response ID or success-shaped body is trusted.** Verification compares
complete before/after inventories of individual approval **record IDs**, not
groups alone. All baseline IDs must remain present. The entire set of new records must
contain exactly one row per requested approver: their positive `member_id` values
must equal the requested approver set, with no duplicates or unrelated new rows.
Each new row must have `status: {id: 0, name: "requested"}` and
a `type.id` equal to the selected captured rule ID. Everyone additionally requires
its observed `type.name: "everyone"`. Read-back names for the other rules were not
captured and are retained raw, not guessed or compared to invented spellings.
An existing requested approval for the same
recipient does not count as new evidence. Concurrent extra requests are ambiguous,
even if some new records match. Missing recipients, already-decided new records,
or a different rule also leave the outcome unknown.

The observed `GET /api/_/changes/<display-id>/approvals` response is a **bare JSON
list** of per-member records; envelope-shaped paginated lists are also supported.
A synthetic illustration (other observed fields omitted for clarity):

```json
[{"id":310000000001,"member_id":9000003001,"level_id":1,"remark":[],"status":{"id":0,"name":"requested"},"type":{"id":1,"name":"everyone"},"member":{"id":9000003001,"name":"Example Agent","email":"agent@example.com"}}]
```

**The flat list does not expose group name or group ID; approval chains do.**
The sender additionally attempts complete chains reads before and after sending.
After complete record-ID verification succeeds, exactly one new group, with all
baseline group IDs preserved, can establish `group_id` and
`group_name_verified: true`. Its `name` must equal the requested name exactly,
its members must equal the requested approvers without duplicates, and its record
IDs, per-record member identities and statuses must agree with the newly verified
flat records. Existing same-name groups, timestamps, levels, list order and write
response IDs are never identity evidence. A supplied nested `member.id` must
agree with `member_id`. Baseline records may already have other statuses; only
new records must match the request.

Unsupported, unreadable, partial, ambiguous or nonmatching chains fall back to
complete record-ID verification: `group_name_verified: false` and no `group_id`.
Chains never bypass missing baseline records, unrelated concurrent new records,
or any other flat-inventory ambiguity. Successful results mark the plan `done`
and return `verified: true`, `approval_ids`, `group_name_verified`, `readback` and
`plan_hash`. No `approval_group_id` is fabricated. Malformed rows, conflicting
member identities, duplicate record IDs, inconsistent totals and incomplete pages
fail closed before generic collection can hide them. An unreadable baseline
blocks sending; an unreadable post-send inventory leaves the outcome unknown.
Empty inventories are valid, but an empty post-send difference proves nothing.
The read-only `fsv changes approvals` command accepts the same bare list, keeps
raw records in JSON output, and shows member, requested status and empty decision
fields sensibly in table/CSV output.

Timeouts, rate limits, server failures, session uncertainty and ambiguous
read-back leave the journal **started** and the outcome **unknown**. They never
trigger a replay. Definite HTTP client rejections are recorded as `failed`.
Identical `started` and `done` plans stay blocked with no expiry or override.
Errors exit with code 2. No claim of approval, email delivery or cancellation is
made by a verified request result.

Inspect the exact change and local journal before any manual recovery. Do not
change a group name or delete journal entries to evade an unknown-outcome guard.
All automated tests use offline synthetic Acme/Globex data; no live Freshservice
calls are required or performed by them.

## Reviewed approve / reject

**These approval actions notify people. Approval/rejection may move the change
workflow. Cancellation is irreversible.** Both decision contracts were initially
captured with traffic aborted; approve was later accepted and verified live.
Rejection has not been observed as an accepted write. The CLI does not claim
email delivery or a particular workflow transition.

```bash
fsv changes approval-respond CHN-1234 310000000001 --approve --dry-run --json
fsv changes approval-respond CHN-1234 310000000001 --reject \
  --remark "Rollback evidence is incomplete." --dry-run --json
```

Exactly one of `--approve` and `--reject` is required. The positional identifier
is the individual **approval record ID**, not an agent, user, group or level ID.
Reject requires a nonempty, non-whitespace `--remark`. Approve allows omission
or an explicitly empty `--remark ""`; these produce distinct reviewed payloads.
Remarks are plain text preserved exactly, not HTML-wrapped.

After reviewing one of those drafts, repeat its exact arguments with the full
hash (a rejection example):

```bash
fsv changes approval-respond CHN-1234 310000000001 --reject \
  --remark "Rollback evidence is incomplete." \
  --yes --expect-plan "$PLAN_HASH" --no-input --json
```

Both commands accept `--dry-run`, `--read-only`, `--yes` / `-y`, `--expect-plan`,
`--no-input` and `--json`. Numeric-ID dry runs do no network or journal I/O.
Read-only mode refuses sends; read-only dry runs are allowed. `--yes` requires
the full prior hash; otherwise a default-No prompt reviews the exact operation.
The immutable plan hash binds tenant, method, path, target and exact body.

The Approvals tab's self-row Approve/Reject actions open an inline Remark form.
Approve's remark is optional; Reject's final button is disabled for blank input,
even though native textarea required attributes do not expose that rule.

```http
PUT /api/_/changes/1234/approvals/310000000001
Content-Type: application/json

{"approval_status":1,"remark":""}
```

Reject uses `{"approval_status":2,"remark":"Rollback evidence is incomplete."}`
on the same path. An omitted optional approve remark omits that property.
These request values do **not** establish a numeric read-back status mapping.
Observed read statuses are `{id: 0, name: "requested"}`,
`{id: 1, name: "approved"}`, and `{id: 3, name: "cancelled"}`. Status 2 is
presumably rejected but has **not** been observed; it is not treated as verified.
After approving, the record's `remark` list contains entries with `delegate_id`,
`status`, `updated_at` and `data`. Those raw fields are preserved in read-back,
not used as substitutes for record identity or decision status.

Before sending, the sender requires the complete approvals inventory, the exact
requested record (`status: {id: 0, name: "requested"}`), consistent member/change
identity, CSRF and a persisted `started` journal entry. It makes exactly one
lower-level PUT, never the helper that replays on 403. The write response is not
trusted. Read-only preparation failures send nothing.

Read-back uses `GET /api/_/changes/<display-id>/approvals`, including all pages,
and locates the exact record `id`. All baseline IDs must remain present; the
record's `member_id`, optional nested `member.id`, and optional `change_id` must
remain consistent. Verification reports `before_status`, `observed_status` and
the raw `readback` record. Status must change away from observed requested/0.
When `status.name` is `approved` for approve or `rejected` for reject, the result
has `verified: true`, `decision_verified: true`, and
`verification_basis: "named_decision"`. Numeric IDs are not guessed from the
write payload. If a valid raw status changes but its name/mapping is unobserved,
`verified: true` means **only the status transition**; `decision_verified: false`,
`verification_basis: "changed_raw_status_only"` and an explicit warning prevent
claiming an approved/rejected decision. A conflicting known decision, unchanged,
requested, malformed, missing or unreadable status leaves the outcome unknown.

Unknown outcomes leave the journal `started`; identical plans remain blocked.
Verified transitions mark `done` and also block resending. Definite HTTP client
rejections are `failed`; timeouts, 408/429, server and session uncertainty are not
replayed. Errors exit 2. Inspect the change and journal; never vary a remark to
evade a duplicate guard.

## Read approval groups and cancel a reviewed group

```bash
fsv changes approvals CHN-1234 --groups
fsv changes approvals CHN-1234 --groups --json
fsv changes approval-cancel CHN-1234 310000000099 --dry-run --json
# Copy the full plan_hash from that review, then:
fsv changes approval-cancel CHN-1234 310000000099 \
  --yes --expect-plan "$PLAN_HASH" --no-input --json
```

`approvals --groups` reads `GET /api/_/changes/<display-id>/approval-chains`.
Table/CSV shows group ID, raw group status and member IDs. JSON is a bare list
of raw group objects, including nested approval record IDs, members and statuses.
Without `--groups`, the existing approvals command and its bare-list JSON output
are unchanged. Group IDs are not approval record IDs, chain IDs, `level_id`, or
`user_id`. All examples here are synthetic.

The observed read envelope is `{"approval_chains":[{"id":310000000098,
"approval_groups":[{"id":310000000099,"name":"Acme review","approval_status":0,"approvals":[
{"id":310000000001,"member_id":9000003001,"status":{"id":0,"name":"requested"},
"member":{"id":9000003001}}]}]}],"meta":{"triggerable_level":1}}`.
The nested **group id** is exactly the ID required by cancellation. The reader
validates all chains, groups, records, member/change identities and raw statuses;
it refuses malformed rows, duplicates, partial pages, inconsistent declared chain
totals, unsupported pagination, page-limit exhaustion and read errors rather
than returning an empty inventory. Recognized page/has-next metadata is followed.

Cancel dry runs are **read-only network reviews**, not local-only: the exact
group must exist in a complete chains inventory and have requested records.
The review includes the raw group, all approval record IDs, members and statuses,
the bodyless path, tenant, warning and full plan hash. `--read-only --dry-run`
is allowed; `--read-only` refuses a send. Bad hashes and missing noninteractive
confirmation are refused before reading. Without `--yes`, confirmation defaults
to No. Cancellation is irreversible and notifies people; it does not recall
emails or promise a particular workflow transition.

The captured Approvals > approval group More > Cancel approval group action
opens a confirmation with no input fields and emits:

```http
PUT /api/_/changes/1234/approval-groups/310000000099/cancel
Content-Type: application/json
X-CSRF-Token: <session token>
```

There is **no body**: not `{}`, not JSON `null`. The transport explicitly sends
zero bytes (`content=b""`) with `Content-Type: application/json` and CSRF.
Without the JSON media type, the live server returned HTTP 415; with it, the
cancel was accepted and chains read-back showed cancelled records
(`{id: 3, name: "cancelled"}`). The plan represents the absent body as
`payload: null` with `body_present: false`. Immediately before sending, the group
is re-read and validated; a missing group refuses the write. CSRF and a persisted
`started` journal entry are required. Exactly one lower-level PUT is attempted,
with no automatic HTTP 403 replay; its response body is not verification.

Verification re-reads the complete chains. It succeeds only if the exact group
is gone (`verification_basis: "group_gone"`) or all of its original approval
record IDs and member identities remain and every status is no longer requested,
with an observed status change (`"changed_raw_statuses_only"`). A changed group
status alone, missing/replaced record, conflicting identity, still-requested
record or unreadable/partial inventory is ambiguous. Results expose raw
`before_status`, `observed_status` and `readback`; no numeric cancellation mapping
or name is invented. `verified: true` describes only that observed disappearance
or transition, not email delivery or a named cancellation status.

Ambiguity, timeout, rate limiting, server or session uncertainty leaves the journal
`started` and blocks an identical retry. Verified observations mark `done` and
also block resubmission. Definite client rejections mark `failed`; no write is
replayed automatically. Inspect the change and journal before manual recovery.
Approval-request verification always retains complete per-record inventories as
the required foundation; a unique matching new chains group additionally verifies
the exact requested name and group ID as described above.
