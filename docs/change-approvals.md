# Reviewed manual change approval requests

`fsv changes approval-request` **requests** manual approval from explicitly named
agents. It does not approve a change, change its status, or trigger its state-flow
transition. The only supported rule is **Everyone** (`approval_type: 1`).

**Approval emails are irreversible. No cancel/revoke contract is known.** Review
the exact recipients, tenant, change, group name and escaped message before
sending. Tenant workflows may also notify people. No implicit self, default
recipients, requester substitution, fuzzy names, recipient groups, HTML input,
attachments, approval decisions or revoke operations are supported.

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

The hash binds tenant, change display ID, exact group name, Everyone rule,
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
request exposed the populated per-member approvals list described below. The POST
response is not used as evidence; notification delivery and cancel/revoke remain
unverified. The synthetic illustration preserves its method, path and fields:

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

## Verification and unknown outcomes

The sender reads the **complete paginated approvals inventory** before sending,
prepares CSRF, persists a content-free `started` journal entry, and reads that
entry back. Failure in preparation prevents the POST. It uses exactly one
lower-level POST, never the helper that replays writes on HTTP 403.

**No POST response ID or success-shaped body is trusted.** Verification compares
complete before/after inventories of individual approval **record IDs**, not
groups. All baseline IDs must remain present. The entire set of new records must
contain exactly one row per requested approver: their positive `member_id` values
must equal the requested approver set, with no duplicates or unrelated new rows.
Each new row must have `status: {id: 0, name: "requested"}` and
`type: {id: 1, name: "everyone"}`. An existing requested approval for the same
recipient does not count as new evidence. Concurrent extra requests are ambiguous,
even if some new records match. Missing recipients, already-decided new records,
or a different rule also leave the outcome unknown.

The observed `GET /api/_/changes/<display-id>/approvals` response is a **bare JSON
list** of per-member records; envelope-shaped paginated lists are also supported.
A synthetic illustration (other observed fields omitted for clarity):

```json
[{"id":310000000001,"member_id":9000003001,"level_id":1,"remark":[],"status":{"id":0,"name":"requested"},"type":{"id":1,"name":"everyone"},"member":{"id":9000003001,"name":"Example Agent","email":"agent@example.com"}}]
```

**Group name and group ID cannot be read back from this list.** The review states
this limitation: the requested group name remains bound in the plan hash but is
not claimed verified. Neither `level_id`, timestamps, list order, `user_id`, nor a
POST response ID is treated as a group identity or substituted for `member_id`.
A supplied nested `member.id` must agree with `member_id`. Baseline records may
already have other statuses; only new records must match the request.

Only this complete inventory evidence marks the plan `done` and returns
`verified: true`, `approval_ids`, `group_name_verified: false`, `readback` and
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
