# Filtered write safety

Filtered ticket bulk actions require `--select-all --max-records N`, with an
explicit bound from 1 to 50. Selection is never truncated or chunked. Every
selected ID is read independently and every predicate must be verifiable;
a contradiction, missing field or unsupported predicate rejects the whole plan.
The review prints the complete ID and subject inventory and binds it to the
review hash. The same proof is rebuilt before submission.

Form fields are not filter-operator metadata. Runtime subject form metadata
exposes `default_subject` but no accepted text operator. The old shared and CLI
builders incorrectly assigned the choice operator `is_in` to arbitrary text.
Free-text predicates now fail closed before listing rather than guessing another
operator. Use explicit reviewed IDs until an accepted text-filter contract exists.
Read lists warn on visible contradictions for supported simple predicates.

Ticket type filters use the name form found in saved ticket views
(`ticket_type is_in ["Service Request"]`). A numeric type choice ID is ignored
by the tenant and broadens the list, so fsv resolves a label or ID to exactly
one active runtime choice name and fails closed otherwise. Independent proof
reads the native exact `type` key and requires any `ticket_type` echo to agree.

Audit: `ticket_bulk.selection` is the only filter-to-mutation expansion in the
source tree. All seven bulk actions, including delegated pickup and merge, pass
through that gate. Other bulk priority/assignment/merge builders consume explicit
IDs, not list filters. Saved-view export creates an export job, not ticket writes;
it must not be used as authorization for record mutations.

Offline regressions use a server that ignores filters, including contradictory,
missing and unsupported fields; they assert zero writes for every bulk action.
No live write was used to validate this fix.
