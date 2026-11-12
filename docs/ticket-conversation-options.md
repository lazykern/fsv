# Reply CC and send-and-status

`fsv tickets reply INC-123 'Working on this' --cc jane@example.com --set-status Pending --dry-run` shows the exact payload and hash. Repeat `--cc` for additional users. Each address must match exactly one user in the requester lookup; the outgoing array contains the lookup's formatted name and email. Repeat the same command with `--yes --expect-plan FULL_HASH` after review. A reply is public and may email the requester and CC recipients.

`fsv tickets add-note INC-123 --body 'Internal update' --set-status Pending --dry-run` reviews a private note and status change. Use `--public --confirm-public` for an explicitly public note. The same full-hash confirmation applies. Without these reply options, the original reply flow is unchanged.

## Resolved and Closed

Accepted native captures cover reply/note with **Resolved** and **Closed**. The combined operation nests the status, preserved group/agent routing and fresh business-rule validation under `helpdesk_ticket`. Status IDs and closure-required fields come from runtime metadata, never copied from another tenant.

```sh
fsv tickets reply INC-123 'Recovery verified.' --set-status Resolved --dry-run
fsv tickets add-note SR-742 --body 'Closure verified.' --set-status Closed --dry-run
```

A nonempty resolution note and all required custom answers must already be persisted. Use [`tickets resolve --note ... --set FIELD=VALUE`](ticket-lifecycle.md) first if necessary. Combined conversation commands do not silently add or overwrite required custom answers. Missing requirements are listed and no conversation is sent. Unknown rules, stale metadata or a changed ticket invalidate authorization.

Both reply and note combined-status wires were accepted in a user-owned test tenant. CLI self-only reply+Resolved and private-note+Closed were also accepted and independently verified. Default reply sender selection is server-controlled; the CLI does not infer an address. An explicit `--sender` is user-asserted, not verified mailbox configuration. Other tenants' sender policies still apply.

Readback requires the exact returned conversation ID, conversation fields and ticket status. Closure transitions also check preserved routing. Internal note readback exposes notification recipients as an explicit `to_emails` array; missing arrays are unverified, never assumed empty. Ambiguous or incomplete closure readbacks leave the journal started and block identical resubmission. Inspect before any recovery; never resend blindly. The terminal UI does not expose these options.
