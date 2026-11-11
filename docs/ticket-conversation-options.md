# Reply CC and send-and-status

`fsv tickets reply INC-123 'Working on this' --cc jane@example.com --set-status Pending --dry-run` shows the exact payload and hash. Repeat `--cc` for additional users. Each address must match exactly one user in the requester lookup; the outgoing array contains the lookup's formatted name and email. Repeat the same command with `--yes --expect-plan FULL_HASH` after review. A reply is public and may email the requester and CC recipients.

`fsv tickets add-note INC-123 --body 'Internal update' --set-status Pending --dry-run` reviews a private note and status change. Use `--public --confirm-public` for an explicitly public note. The same full-hash confirmation applies. Without these new reply options, the original reply flow is unchanged.

Only Pending was captured for the nested status change. Its ID is looked up from fresh ticket status metadata, never assumed; other statuses, including Resolved and Closed, are refused because their requirements were not captured. Rule evaluation and a current ticket read are required. Read-back checks the exact returned conversation ID and ticket status; missing fields remain unverified. Ambiguous outcomes must not be retried without inspection.

The web capture included a tenant-specific `from_email`; the existing reply API flow does not resolve a sender, so this implementation omits that field rather than guessing. The captured payload was blocked, not accepted by the server; sender inference and actual acceptance remain unverified. The terminal UI does not expose these options.
