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
available; `--reviewed` selects this stricter path explicitly.

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
