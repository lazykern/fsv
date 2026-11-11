# Incident CC

Use `fsv tickets create --requester jane@example.com --set 'Subject=Example' --cc team@example.com --dry-run --json` with the form's other required answers. Repeat `--cc` for each plain email address. Review the resulting payload and plan hash, then repeat the same arguments with `--yes --expect-plan HASH` to submit. CC is included in the hash and may notify recipients; requester remains unchanged. The terminal form does not retain this flag, so use a draft, `--set`, preset or `--edit` with it.

The `cc_emails` key and plain-email array come from a blocked UI submission, not an accepted live write. After a response with an ID, the exact ticket is read back; missing or mismatched CC fields are reported as unverified and must not prompt resubmission. Tenant rules and notifications may differ. No successful write is claimed.
