# Reviewed change–ticket associations

These commands manage **initiating tickets** on a change. They do not change
archived-ticket, caused-by-change, asset, problem or release relationships.
Existing internal wire shapes are retained; no accepted live write response is
claimed or required. Offline tests exercise synthetic Acme/Globex records.

## CLI

```bash
fsv changes associations ls CHN-1234 --json
fsv changes associations search CHN-1234 "Acme access"
fsv changes associations add CHN-1234 SR-5678 --dry-run --json
fsv --no-input changes associations add CHN-1234 SR-5678 --yes --expect-plan FULL_HASH --json
fsv changes associations remove CHN-1234 SR-5678 --dry-run --json
fsv --no-input changes associations remove CHN-1234 SR-5678 --yes --expect-plan FULL_HASH --json
```

Replace `FULL_HASH` with the entire `plan_hash` from the exact dry-run review.
The hash binds tenant, method, endpoint, payload and query parameters. Review
shows resolved **internal ticket IDs**, not just display labels. Search/ID
resolution may read the server during a dry run; no write or journal entry is
made. In an interactive terminal, review the full plan and confirm instead of
using `--yes`. `--yes` alone and mismatched hashes are refused. `--read-only`
refuses a send; it can be combined with `--dry-run` to inspect a plan.

`add` can link multiple tickets in one reviewed PUT. `remove` accepts exactly
one ticket per review, because the observed DELETE contract is per ticket.
Repeat the review process separately for each removal. `pick` and the deprecated
legacy association syntax use the same review/hash gates.

## Single attempt and truthful results

Before a send, fsv loads a complete initiating-ticket inventory and prepares the
session. If the requested state already holds, it reports
`already_in_requested_state` without a write. Otherwise it persists a `started`
plan journal entry and sends exactly one request, bypassing the shared 403 CSRF
refresh/replay path. It does not retry the PUT or DELETE after any failure.

After an accepted response, only a **complete** paginated initiating-ticket
inventory can verify success: every linked internal ID must be present, or the
unlinked ID must be absent. A malformed/empty success envelope is not evidence
of success; complete readback may still establish state. Wrong state, failed or
partial readback, transport timeouts and ambiguous server errors leave the plan
`started`, report an unknown/unverified outcome and block the identical plan.
Inspect the exact change and journal; **do not resubmit to recover a response**.
Definitive client rejection (including HTTP 403, excluding 408/429) records
`failed` without automatic retry. Completed plans are also duplicate-blocked.
This is a local journal guard, not a server-side idempotency guarantee.

Association inventories are bounded to 50 pages per relationship and detect
continuation cycles. They preserve earlier rows on a later-page failure and
expose per-kind `meta`, aggregate `complete` and `warnings` in JSON. Table/CSV
modes print incompleteness warnings to stderr; an unavailable inventory is not
reported as "no associations". Invalid pages and invalid IDs are unavailable,
not silently discarded into a complete empty list.

## Main TUI

Select a change, press **a**, and choose **Link initiating ticket** or
**Unlink initiating ticket**. Review shows the same tenant, exact wire request,
resolved IDs and full hash. Cancellation and read-only mode do not send. The
confirmed immutable plan goes through the same journaled sender; only complete
readback produces a verified-success notice. Partial target inventories remain
visible with warnings, but an incomplete fresh preflight blocks the write.
Tenant workflows may notify people; linking is not a no-notification operation.
