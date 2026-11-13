"""Single reviewed change property PUT using the shared form builder's plan."""
from __future__ import annotations
import json
from dataclasses import dataclass
from typing import Any
from fsv import config, journal
from fsv.reviewed_snapshot import stable_snapshot
from fsv.formmeta import load_meta
from fsv.readback import same_instant, same_text, submitted_value_matches
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute


@dataclass(frozen=True)
class FormWritePlan(WritePlan):
    source_hash: str

    @property
    def hash(self) -> str:
        return self.source_hash


def send(source: Any, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    if any(source.files.values()) or source.planning:
        raise ReviewedWriteError("Combined change property/files/planning sends are disabled; use separate reviewed operations")
    from fsv.change_flow import _hash
    expected = journal.plan_hash({"legacy": _hash(source.payload, source.files, source.planning, source.target),
                                  "record": source.record_hash, "metadata": source.meta.fingerprint.hash,
                                  "tenant": source.tenant})
    if not source.tenant or source.tenant != config.DOMAIN or source.hash != expected:
        raise ReviewedWriteError("Change tenant or exact reviewed payload changed; review again")
    if not source.ok or not source.payload or not source.record_hash or not source.hash:
        raise ReviewedWriteError("Complete hash-bound property plan required")
    for value in {source.hash, source.legacy_hash}:
        events = [e for e in journal.read() if e.get("kind") == "plan:update" and e.get("plan") == value]
        refusal = journal.replay_refusal(events, "Change update was sent or outcome unknown; do not replay")
        if refusal:
            raise ReviewedWriteError(refusal)
    def record():
        raw = client.int_get(f"changes/{source.target}")
        row = raw.get("change", raw) if isinstance(raw, dict) else {}
        identities = [row.get("id"), row.get("display_id")] if isinstance(row, dict) else []
        human = row.get("human_display_id") if isinstance(row, dict) else None
        if isinstance(human, str) and human.startswith("CHN-"):
            identities.append(human[4:])
        if not isinstance(row, dict) or str(source.target) not in {str(value) for value in identities}:
            raise ReviewedWriteError("Exact change readback unavailable")
        return row
    plan = FormWritePlan("change_form_update", source.target, config.DOMAIN, "PUT", f"changes/{source.target}",
                         json.dumps(source.payload, sort_keys=True), source.record_hash, source.hash)
    def rebuild():
        if journal.snapshot_hash(stable_snapshot(record())) != source.record_hash:
            raise ReviewedWriteError("Change record changed since review")
        fresh = load_meta(client, "change", "edit", fresh=True, module_id=source.meta.module_id,
                          record_workspace=source.meta.record_workspace)
        if fresh.fingerprint.changed(source.meta.fingerprint):
            raise ReviewedWriteError("Change metadata changed since review")
        return plan
    def verify(_response):
        row = record()
        fields = {key: "unavailable" if key not in row else "matched" if
                  (same_text(row[key], value) if key == "description" else
                   same_instant(row[key], value) if key.endswith("_date") else submitted_value_matches(row[key], value))
                  else "mismatched" for key, value in source.payload.items()}
        return {"verified": all(value == "matched" for value in fields.values()), "fields": fields, "readback": row}
    result = execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=verify)
    result.update(id=source.target, result=result.get("readback"),
                  steps=[{"step": "update", "ok": result.get("verified", False), "id": source.target}], uploaded=[])
    return result
