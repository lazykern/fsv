# Create an incident

`fsv tickets create` opens an interactive form in a terminal. The form and its
review use the ticket fields and business rules for the selected account. The
requester is selected explicitly (or from `defaults.requester`), never silently
the signed-in agent. Creation may notify people and trigger workflows.

For scripts, review first and use the resulting hash:

```sh
fsv tickets create --requester jane@example.com \
  --set 'Subject=Service unavailable' --set 'Description=Please investigate' \
  --dry-run --json
fsv tickets create --requester jane@example.com \
  --set 'Subject=Service unavailable' --set 'Description=Please investigate' \
  --yes --expect-plan HASH --no-input
```

The tenant may require other fields; use its labels and choices. `--from` reads
a YAML draft, `--preset` loads a preset and `--edit` opens an annotated YAML
draft in a terminal. `--ack` acknowledges blocking rule messages. Dry-run
shows the exact request and generated validation token, so treat its output as
private. Metadata is reloaded before a send; a changed plan must be reviewed
again. Unsupported or unavailable rule inputs cause a refusal rather than a
static-only validation. Incident `--attach FILE` uses a full-hash-reviewed
upload transaction when an explicit byte size limit is available in runtime
metadata; see [reviewed attachments](reviewed-parity.md#local-attachments).
Missing limit metadata blocks uploads rather than guessing a constant.

There is no automatic create retry, including on a CSRF error. If the result is
unknown, check the server before taking any further action; even
`--allow-duplicate` will not override that unknown outcome. A known completed
incident can be intentionally repeated with `--allow-duplicate`. The local
journal contains hashes and outcomes, not request bodies or cookies. This
command is based on a blocked browser submission and offline tests, not on a
confirmed live incident creation.
