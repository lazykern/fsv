# Ticket tags and planned dates

`fsv tickets update INC-123 --tag 'review' --tag 'urgent' --planned-start '2030-01-02T09:00:00-05:00' --planned-end '2030-01-02T10:00:00-05:00' --dry-run` reviews before/after values and the exact PUT body. **Tags replace the entire existing list**, not append. Omit `--tag` to leave tags untouched; clearing all tags is not supported. Dates must include an explicit UTC offset; no host timezone is inferred.

Repeat the same options with `--yes --expect-plan FULL_HASH` to submit. Other update flags cannot be mixed with these properties. The target and form rules are re-read before the one PUT; if either changes, review again. After submission the exact ticket is read back; missing or mismatched fields are reported as unverified. An unknown outcome must not be retried without inspection.

The keys and offset date format come from a blocked UI capture; the validation token is generated from evaluated runtime form rules, not copied from the capture. Server acceptance is unverified. Missing rule context or validation blocks the update; use the web form if the rules cannot be evaluated. These fields can trigger workflows.
