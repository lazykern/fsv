# Service request recipients

Use `fsv sr create ITEM --from request.yaml --requested-for 'Jane Doe' --cc team@example.com --dry-run --output json`. Repeat `--cc` for plain email addresses. Each CC must resolve to exactly one non-deleted user by exact email. The requested-for user must resolve to exactly one non-deleted user with an email address; ambiguous lookups fail. The existing requester is not changed. Review the payload and hash, then repeat the arguments with `--yes --expect-plan HASH`. The recipient fields are included in the plan hash and may affect notifications and approvals.

The interactive form and blank-template mode cannot retain these flags; use a draft, `--set`, preset or `--edit`. If rules require requested-for attributes, creation is blocked because metadata for the selected person's context is not verified. The `requested_for` email and `cc_emails` array are based on a blocked UI submission, not an accepted live write. Tenant behavior may differ.
