# Ticket notes (reviewed UI contract)

Ticket **notes** are not ticket **replies**. A public note is visible to the
customer; it is not the reply/email action. This dedicated module does not change
`TICKETS.has_notes` or conversation-reader behavior.

The observed, blocked UI request is:

```http
POST /api/_/tickets/<display-id>/notes
Content-Type: application/json
```

```json
{
  "body": "<div>Acme review complete</div>",
  "conversation_from_summary": false,
  "private": true,
  "notify_emails": []
}
```

Private is the default. Public selection sets `private:false` and requires an
additional visibility confirmation. Recipients must be explicit; the web UI's
default assignee recipient is **not** inherited. Empty `notify_emails` does not
prove that tenant workflows will never notify anyone.

**Evidence boundary:** the request shape and public/private UI distinction were
observed with submission blocked. Accepted live submission and a newly created
note's read-back have not been demonstrated. Synthetic transport tests are not
live-wire proof. Attachments, reply, summary-note creation and other payload keys
are not supported here.

## CLI registration

The parent CLI registers the helper only on its tickets Typer app:

```python
from fsv.ticket_notes import register as register_ticket_notes

# Inside resource command registration, with sub = the tickets Typer app:
if res.name == "tickets":
    register_ticket_notes(sub)
```

The module itself does not modify `cli.py`. Do not enable the generic resource
note path just to expose this command.

After registration:

```sh
fsv tickets add-note SR-123 --body 'Acme review complete' --dry-run
fsv tickets add-note SR-123 --body 'Acme review complete'
# Scripted send: use the complete hash from that exact dry-run/body review.
fsv tickets add-note SR-123 --body 'Acme review complete' \
  --yes --expect-plan FULL_REVIEW_HASH --no-input
# Public notes require a separate confirmation in addition to selection:
fsv tickets add-note SR-123 --body 'Customer-visible update' --public \
  --confirm-public --yes --expect-plan FULL_REVIEW_HASH --no-input
```

Every send prints the exact HTML payload, plain preview, visibility and explicit
recipient list before confirmation. `--notify-email agent@example.com` can be
repeated. Plain text is escaped by default, with div paragraphs and br line
breaks matching form-payload conventions. `--html` retains supplied HTML exactly;
it does not sanitize it. Unlike automatic HTML detection, plain mode does not
interpret a leading `<` as markup. Empty bodies and markup-only HTML are rejected.

`--yes` needs a complete `--expect-plan` hash from prior review. A mismatched
hash never sends. Without `--yes`, declining either review prompt cancels.
`--read-only` refuses a send. Dry-run, cancellation and read-only guards do not
construct a client, post anything or write journal entries. Dry-run remains
available in read-only mode.

## Reusable TUI interface

```python
from fsv.ticket_notes import build_plan, send

plan = build_plan(
    ticket_id=123,                   # display ID; also accepts INC-123 / SR-123
    body="Acme review complete",
    public=False,                   # separate user selection; default private
    notify_emails=[],               # omitted / None also becomes []
    body_format="plain",            # or explicit "html"
)
review = plan.review()              # exact HTML payload + body_text + warnings
# Display review, including body and recipients; wait for user confirmation.
# Ask a distinct public-visibility confirmation when plan.public is true.
result = send(
    plan,
    client,                         # shared Client; lazy default if omitted
    reviewed_hash=plan.hash,         # ONLY after confirming that exact review
    public_confirmed=False,         # True ONLY after separate public consent
    read_only=effective_read_only,   # pass the TUI's resolved read-only setting
)
```

`NotePlan` is frozen; `payload` returns a fresh object. Its hash includes tenant,
ticket endpoint and every payload field. Changing the tenant or reviewed values
requires a new review. `send(..., reviewed_hash=None)` cancels. A dry-run can use
`send(plan, reviewed_hash=None, dry_run=True)`. Integrators must pass the effective
read-only setting; this module does not import TUI configuration.

## Single-attempt delivery and identity

The sender resolves CSRF using existing read-only client preparation and records
a hash-only `plan:ticket_note` journal event before sending. Failure to persist
that event stops the operation. It deliberately bypasses `Client.int_post`,
whose internal 403 refresh path can send a second POST, and uses the existing
lower-level `_request` once. No retry, alternate endpoint, reply fallback, or
implicit recipient is introduced. Body text is not included in verbose request
previews or plan-journal events.

A successful response with an exact positive numeric note ID is retained as
`result['note_id']` and `result['note']`. Supported decoding forms are a `note`
object, `conversation` object or direct ID-bearing object; these are defensive
response decoders, not assertions of an observed accepted-response shape.
The existing paginated conversation loader reads the same ticket and matches
**only that exact ID**. It never substitutes the newest entry or a body-text
match. `result['readback']` contains the matching conversation and `verified` is
true only if it was found. A missing/incomplete/failed read-back returns
`verified:false` plus a warning while retaining the returned ID. Submission is
not retried because read-back failed.

Transport errors, ambiguous HTTP outcomes, malformed responses and missing IDs
leave a `started` journal entry and raise `NoteError` with a check-before-retry
warning. A matching unresolved or completed plan cannot be resubmitted by this
module. Definitive HTTP rejections record `failed`; any later attempt still
requires a new explicit call and review, never an automatic resend. No CLI flag
bypasses ambiguous-outcome protection. Journal protection is local to a profile
and tenant, not a server idempotency guarantee or cross-process transaction lock.
