"""Create and clone resources from schema-driven templates."""

from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import Any

from fsv.client import APIError, Client, get_client
from fsv.resources import Resource, CHANGES
from fsv.schema import load as load_schema

READ_ONLY_FIELDS = {
    # system IDs / identity
    "id", "display_id", "human_display_id", "change_id", "workspace_id",
    "import_id", "project_id", "wf_event_id",
    # timestamps
    "created_at", "updated_at", "closed_at", "due_by", "nr_due_by",
    "first_response_time", "ola_stopped_at",
    # computed status / display
    "status_name", "priority_name", "impact_name", "risk_name",
    "approval_status", "approval_type", "summary_status",
    "resolution_time_taken", "total_time_spent", "timer_active_agents",
    "task_due_status", "is_editable", "status_ola_timer",
    # workflow / state machine
    "state_flow_id", "state_traversal", "cab_meeting_ids",
    "trashed", "deleted", "is_escalated", "escalation_level",
    "manual_dueby", "planned",
    # embedded objects (use *_id instead)
    "requester", "department", "agent", "group",
    # associations (read-only lists)
    "attachments", "cloud_files", "shared_attachment_ids",
    "assoc_assets", "assoc_asset_ids", "assoc_change_ids",
    "assoc_release_id", "blackout_window",
    # misc internal
    "source", "business_calendar_id", "sla_policy_id", "cc_email",
    "email_config_id", "group_name", "owner_name",
    "description_text", "problem_id", "release_id",
    "associated_cis_and_services_ids",
    "sub_category", "item_category", "initiated_by",
    "stack_rank", "taskable_display_id", "taskable_type", "user_id",
    "force_authenticated", "tasks_dependency_type",
    # task-specific
    "notify_before",
}

SKIP_TYPES = {
    "custom_content", "planning_field",
}

# Embedded FK fields are read-only as objects; the create API takes a *_id key
# instead. The template must scaffold the *_id placeholder, not drop the field.
_TEMPLATE_FK_PLACEHOLDERS = {
    "requester": ("requester_id", "<requester email or ID>"),
    "agent": ("agent_id", "<agent email/name or ID>"),
    "group": ("group_id", "<group name or ID>"),
    "department": ("department_id", "<department name or ID>"),
}

TASK_READ_ONLY_FIELDS = READ_ONLY_FIELDS | {
    "stack_rank", "taskable_display_id", "taskable_type", "user_id",
    "task_due_status", "is_editable", "status_ola_timer", "ola_stopped_at",
    "agent", "group", "status_name",
}


def _build_template(schema_fields: list[dict[str, Any]], level: str) -> dict[str, Any]:
    """Build a JSON template from schema fields.

    level: "required" | "optional" | "all"
    """
    template: dict[str, Any] = {}
    for f in schema_fields:
        name = f.get("name", "")
        if not name:
            continue
        ftype = str(f.get("field_type") or "")
        if ftype in SKIP_TYPES:
            continue
        required = f.get("required", False)
        if name in _TEMPLATE_FK_PLACEHOLDERS:
            if level == "required" and not required:
                continue
            key, placeholder = _TEMPLATE_FK_PLACEHOLDERS[name]
            template[key] = placeholder
            continue
        if name in READ_ONLY_FIELDS:
            continue
        if level == "required" and not required:
            continue
        val = _field_default(f)
        template[name] = val
    return template


def _field_default(f: dict[str, Any]) -> Any:
    """Generate a sensible default/placeholder for a schema field."""
    ftype = str(f.get("field_type") or "")
    choices = f.get("choices") or []

    if ftype in ("default_checkbox", "custom_checkbox") or choices and all(
        isinstance(c.get("id"), bool) or c.get("id") in (True, False) or c.get("value") in ("true", "false")
        for c in choices[:2]
    ):
        return False
    if choices:
        if len(choices) <= 5:
            return [c.get("value", c.get("name", c.get("id"))) for c in choices]
        examples = [c.get("value", c.get("name", c.get("id"))) for c in choices[:2]]
        return f"<choose: {', '.join(str(e) for e in examples)}, ...>"
    if ftype.endswith("_date"):
        return "2026-01-01T10:00:00+07:00"
    if ftype.endswith("_number") or ftype.endswith("_decimal"):
        return 0
    if ftype == "default_description":
        return "<div>description here</div>"
    if ftype == "default_subject":
        return "subject here"
    if ftype in ("default_requester", "default_agent"):
        return "<email or name>"
    if ftype in ("default_group", "default_department"):
        return "<name or ID>"
    if ftype.endswith("_multi_select_dropdown"):
        return []
    if name := f.get("name"):
        if name.endswith("_id"):
            return "<ID>"
    return ""


def change_template(level: str = "required") -> dict[str, Any]:
    """Generate a change create template."""
    fields = _get_change_schema_fields()
    return _build_template(fields, level)


def change_clone_data(change_id: int, c: Client | None = None) -> dict[str, Any]:
    """Get change data ready for cloning (strips read-only/computed fields, resets status)."""
    if c is None:
        c = get_client()
    data = c.int_get(f"changes/{change_id}")
    change = data.get("change", data)

    # Flatten custom_fields so the editor sees a flat dict
    if "custom_fields" in change:
        cf = change.pop("custom_fields")
        if isinstance(cf, dict):
            change.update(cf)

    out: dict[str, Any] = {}
    for k, v in change.items():
        if k in READ_ONLY_FIELDS:
            continue
        # skip None-valued fields that aren't core to reduce editor noise
        out[k] = v

    # Always start a clone as Open
    out["status"] = 1

    return out


def clone_tasks(
    source_change_id: int,
    new_change_id: int,
    c: Client | None = None,
) -> list[dict[str, Any]]:
    """Clone all tasks from source change into new change. Returns created tasks."""
    if c is None:
        c = get_client()
    data = c.int_get(f"changes/{source_change_id}/tasks")
    tasks = data.get("tasks", [])
    created = []
    for task in tasks:
        body = _task_clone_body(task)
        result = c.int_post(f"changes/{new_change_id}/tasks", {"change_task": body})
        created.append(result.get("task", result))
    return created


def _task_clone_body(task: dict[str, Any]) -> dict[str, Any]:
    """Strip read-only fields and nulls from a task, reset status to Open."""
    out: dict[str, Any] = {}
    for k, v in task.items():
        if k in TASK_READ_ONLY_FIELDS or v is None:
            continue
        out[k] = v
    out["status"] = 1
    if isinstance(out.get("custom_fields"), dict):
        cf = {k: v for k, v in out["custom_fields"].items() if v is not None}
        if cf:
            out["custom_fields"] = cf
        else:
            out.pop("custom_fields", None)
    return out


def clone_assets(
    source_change_id: int,
    new_change_id: int,
    c: Client | None = None,
) -> list[int]:
    """Associate same assets from source change onto new change. Returns display_ids added."""
    if c is None:
        c = get_client()
    assoc = get_change_assets(source_change_id, c)
    display_ids = [
        int(entry.get("display_id") or (entry.get("config_item") or {}).get("display_id") or 0)
        for entry in assoc
        if entry.get("display_id") or (entry.get("config_item") or {}).get("display_id")
    ]
    if not display_ids:
        return []
    c.int_put(f"changes/{new_change_id}/assets/associate", {"item_ids": display_ids})
    return display_ids


def clone_planning_fields(
    source_change_id: int,
    new_change_id: int,
    c: Client | None = None,
) -> list[str]:
    """Clone planning fields (text + attachments) from source to new change. Returns field names cloned."""
    if c is None:
        c = get_client()

    data = c.int_get(f"changes/{source_change_id}/planning-fields")
    fields = data.get("change_planning_fields", [])
    cloned = []

    for field in fields:
        name = field.get("name")
        if not name:
            continue
        description = field.get("description") or ""
        attachments = field.get("attachments", [])
        if not description.strip() and not attachments:
            continue

        new_att_ids: list[int] = []
        for att in attachments:
            att_id = _reupload_attachment(att, c)
            if att_id is not None:
                new_att_ids.append(att_id)

        body: dict[str, Any] = {"description": description}
        if new_att_ids:
            body["attachments"] = new_att_ids

        c.int_post(f"changes/{new_change_id}/planning-fields?id={name}", body)
        cloned.append(name)

    return cloned


def _attachment_url(att: dict[str, Any]) -> str:
    url = str(att.get("attachment_url") or att.get("canonical_url") or "")
    if not url and att.get("canonical_path"):
        from fsv import config
        url = f"https://{config.DOMAIN}{att['canonical_path']}"
    if url and "?" not in url and "/helpdesk/attachments/" in url:
        url += "?download=true"
    return url


def _safe_filename(name: str) -> str:
    cleaned = name.replace("/", "-").replace("\x00", "").strip()
    return cleaned or "attachment"


def download_attachment(att: dict[str, Any], out_dir: str | Path, force: bool = False, c: Client | None = None) -> dict[str, Any]:
    if c is None:
        c = get_client()
    url = _attachment_url(att)
    if not url:
        raise APIError(0, f"attachment has no URL: {att}")
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    name = _safe_filename(str(att.get("name") or f"attachment-{att.get('id', '')}"))
    path = out / name
    if path.exists() and not force:
        return {"name": name, "path": str(path), "size": path.stat().st_size, "skipped": True}
    sys.stderr.write(f"  downloading {name}...\n")
    sys.stderr.flush()
    r = c._client.get(url, follow_redirects=True)
    if r.status_code >= 400:
        raise APIError(r.status_code, r.text[:500])
    cd = r.headers.get("content-disposition", "")
    if not att.get("name") and "filename=" in cd:
        from email.message import Message
        msg = Message()
        msg["content-disposition"] = cd
        filename = msg.get_filename()
        if filename:
            name = _safe_filename(filename)
            path = out / name
            if path.exists() and not force:
                return {"name": name, "path": str(path), "size": path.stat().st_size, "skipped": True}
    path.write_bytes(r.content)
    return {"name": name, "path": str(path), "size": len(r.content), "skipped": False}


def _reupload_attachment(att: dict[str, Any], c: Client) -> int | None:
    import base64

    url = _attachment_url(att)
    if not url:
        return None

    r = c._client.get(url, follow_redirects=True)
    if r.status_code != 200:
        return None

    mime_full = att.get("content_type", "application/octet-stream")
    mime = mime_full.split(";")[0].strip()  # strip ;charset=... — API rejects parameters
    name = att.get("name", "attachment")
    content_b64 = base64.b64encode(r.content).decode("ascii")

    import time
    body = {
        "content": f"data:{mime};base64,{content_b64}",
        "content_updated_at": time.time(),
        "content_file_name": name,
        "content_file_size": len(r.content),
        "content_content_type": mime,
    }
    result = c.int_post("attachments", body)
    if isinstance(result, dict):
        return result.get("id") or (result.get("attachment") or {}).get("id")
    return None


def _upload_bytes(name: str, raw: bytes, mime: str, c: Client) -> int:
    import base64

    body = {
        "content": f"data:{mime};base64,{base64.b64encode(raw).decode('ascii')}",
        "content_file_name": name,
        "content_file_size": len(raw),
        "content_content_type": mime,
    }
    sys.stderr.write(f"  uploading {name}... ")
    sys.stderr.flush()
    result = c.int_post("attachments", body)
    att_id = result.get("id") if isinstance(result, dict) else None
    if not att_id:
        raise APIError(0, f"attachment upload returned no id: {result}")
    srv_size = result.get("content_file_size") or result.get("size")
    srv_name = result.get("content_file_name") or result.get("name")
    if srv_size == len(raw) and srv_name == name:
        sys.stderr.write(f"ok ({len(raw):,} B)\n")
    elif srv_name != name:
        sys.stderr.write(f"ok ({len(raw):,} B, server name \"{srv_name}\" differs)\n")
    else:
        sys.stderr.write(f"mismatch (local {len(raw):,} B, server {srv_size} B)\n")
    sys.stderr.flush()
    return int(att_id)


def upload_file(path: str, c: Client | None = None) -> int:
    """Upload a local file and return its attachment ID."""
    import mimetypes

    if c is None:
        c = get_client()

    with open(path, "rb") as f:
        raw = f.read()

    mime = mimetypes.guess_type(path)[0] or "application/octet-stream"
    name = path.split("/")[-1]
    return _upload_bytes(name, raw, mime, c)


def attach_files_to_change(
    change_id: int,
    file_paths: list[str],
    c: Client | None = None,
    duplicate: str = "prompt",
    backup_replaced: bool | None = None,
    backup_name: str | None = None,
) -> dict[str, Any]:
    """Upload files and attach them to a change's main Attachments field."""
    if c is None:
        c = get_client()

    data = c.int_get(f"changes/{change_id}")
    change = data.get("change", data)
    attachments = list(change.get("attachments") or [])
    all_ids = [att_id for att_id in (_attachment_id(a) for a in attachments) if att_id is not None]

    mode = duplicate.casefold()
    if mode not in {"prompt", "skip", "replace", "append"}:
        raise APIError(0, "--duplicate must be one of: prompt, skip, replace, append")

    changed = False
    skipped: list[str] = []
    for path in file_paths:
        name = Path(path).name
        matches = [a for a in attachments if _attachment_name(a) == name]
        action = mode
        if matches and action == "prompt":
            action = _prompt_duplicate_action(name, len(matches))
        if matches and action == "skip":
            sys.stderr.write(f"  skipped {name} (already attached)\n")
            skipped.append(name)
            continue
        if matches and action == "replace":
            replace_ids = {_attachment_id(a) for a in matches}
            all_ids = [att_id for att_id in all_ids if att_id not in replace_ids]
            changed = True
            if _should_backup_replaced(name, backup_replaced):
                for i, att in enumerate(matches):
                    all_ids.append(_backup_attachment(att, c, _choose_backup_name(name, backup_name, i, backup_replaced is None)))
        all_ids.append(upload_file(path, c))
        changed = True

    if not changed:
        return {"_fsv_noop": True, "skipped": skipped, "attachments": all_ids}
    c.int_put(f"changes/{change_id}", {"attachments": all_ids})
    return {"attachments": all_ids, "skipped": skipped}


def set_due_by(change_id: int, due_by: str, c: Client | None = None) -> dict[str, Any]:
    """Set resolution due date via PATCH (separate endpoint from PUT update)."""
    if c is None:
        c = get_client()
    return c.int_patch(f"changes/{change_id}", {"due_by": due_by})


def get_change_for_edit(change_id: int, c: Client | None = None) -> dict[str, Any]:
    """Fetch change data prepared for editing (flat, read-only fields stripped)."""
    if c is None:
        c = get_client()
    data = c.int_get(f"changes/{change_id}")
    change = data.get("change", data)

    if "custom_fields" in change:
        cf = change.pop("custom_fields")
        if isinstance(cf, dict):
            change.update(cf)

    return {k: v for k, v in change.items() if k not in READ_ONLY_FIELDS}


def update_change(change_id: int, body: dict[str, Any], c: Client | None = None) -> dict[str, Any]:
    """PUT update a change via internal API."""
    if c is None:
        c = get_client()

    schema_fields = _get_change_schema_fields()
    sf_names = {f.get("name"): f for f in schema_fields}

    CORE_NAMES = {
        "requester_id", "email", "group_id", "agent_id", "change_type",
        "priority", "impact", "risk", "status", "category",
        "subject", "description", "planned_start_date", "planned_end_date",
        "planned_effort", "department_id", "change_window_id", "attachments",
    }
    core: dict[str, Any] = {}
    custom_fields: dict[str, Any] = {}

    for k, v in body.items():
        sf = sf_names.get(k, {})
        is_default = sf.get("default_field", False)
        if k in CORE_NAMES or is_default:
            core[k] = v
        else:
            custom_fields[k] = v

    if custom_fields:
        core["custom_fields"] = custom_fields

    try:
        data = c.int_put(f"changes/{change_id}", core)
        return data.get("change", data)
    except APIError as e:
        err_body = e.body
        if isinstance(err_body, dict):
            errors = err_body.get("errors")
            if isinstance(errors, list):
                msgs = [f"{err.get('field', '?')}: {err.get('message', '?')}" for err in errors]
                raise RuntimeError("update failed:\n  " + "\n  ".join(msgs)) from e
            desc = err_body.get("description", str(err_body))
            raise RuntimeError(f"update failed: {desc}") from e
        raise


def _planning_key(value: Any) -> str:
    text = str(value or "").casefold()
    for prefix in ("planning-field-", "cf_"):
        if text.startswith(prefix):
            text = text[len(prefix):]
    return "".join(ch for ch in text if ch.isalnum())


def _singular_key(value: Any) -> str:
    text = str(value or "").casefold()
    for prefix in ("planning-field-", "cf_"):
        if text.startswith(prefix):
            text = text[len(prefix):]
    words: list[str] = []
    current: list[str] = []
    for ch in text:
        if ch.isalnum():
            current.append(ch)
        elif current:
            word = "".join(current)
            words.append(word[:-1] if word.endswith("s") else word)
            current = []
    if current:
        word = "".join(current)
        words.append(word[:-1] if word.endswith("s") else word)
    return "".join(words)


def _is_planning_field(f: dict[str, Any]) -> bool:
    ftype = str(f.get("field_type") or "")
    return ftype in {"planning_field", "default_change_reason", "default_change_impact", "default_change_plan", "default_backout_plan"}


def resolve_planning_field(value: str, schema_fields: list[dict[str, Any]]) -> str:
    raw = value.strip()
    common_aliases = {"reason_for_change": "change_reason", "rollout_plan": "change_plan"}
    raw = common_aliases.get(raw.casefold(), raw)
    planning_fields = [f for f in schema_fields if _is_planning_field(f)]
    aliases = [raw]
    if raw.startswith("planning-field-"):
        aliases.append(raw.removeprefix("planning-field-"))
    aliases.extend(a.removeprefix("cf_") for a in list(aliases) if a.startswith("cf_"))
    keys = {_planning_key(a) for a in aliases} | {_singular_key(a) for a in aliases}

    for f in planning_fields:
        candidates = [
            f.get("name"), f.get("id"), f.get("label"), f.get("export_label"),
            f.get("helptext"), f.get("description"), f"planning-field-{f.get('name')}",
            f"planning-field-cf_{f.get('name')}", f"cf_{f.get('name')}",
        ]
        cand_keys = {_planning_key(c) for c in candidates} | {_singular_key(c) for c in candidates}
        if keys & cand_keys:
            return str(f["name"])
    known = ", ".join(str(f.get("label") or f.get("name")) for f in planning_fields[:12])
    raise APIError(0, f"planning field not found: {value!r}; choices: {known}")


def _planning_description(field: dict[str, Any] | None) -> str:
    if not field:
        return ""
    for key in ("description", "description_text", "body", "content"):
        value = field.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return ""


def _default_planning_description(paths: list[str]) -> str:
    return ", ".join(Path(p).name for p in paths)


def _attachment_name(att: dict[str, Any]) -> str:
    return str(att.get("name") or att.get("content_file_name") or "")


def _attachment_id(att: dict[str, Any]) -> int | None:
    value = att.get("id")
    return int(value) if value is not None else None


def _prompt_duplicate_action(name: str, count: int) -> str:
    if not sys.stdin.isatty():
        raise APIError(0, f"attachment already exists: {name}; use --duplicate skip|replace|append")
    sys.stderr.write(f"  duplicate {name} ({count} existing). Choose [s]kip, [r]eplace, [a]ppend: ")
    sys.stderr.flush()
    answer = input().strip().casefold()
    if answer in ("r", "replace", "overwrite", "o"):
        return "replace"
    if answer in ("a", "append"):
        return "append"
    return "skip"


def _backup_name(original: str, backup_name: str | None = None, index: int = 0) -> str:
    if backup_name:
        return backup_name if index == 0 else f"{Path(backup_name).stem}-{index}{Path(backup_name).suffix}"
    from datetime import datetime
    p = Path(original)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    suffix = f".backup-{stamp}" if index == 0 else f".backup-{stamp}-{index}"
    return f"{p.stem}{suffix}{p.suffix}"


def _choose_backup_name(original: str, backup_name: str | None, index: int, prompt: bool) -> str:
    default = _backup_name(original, backup_name, index)
    if backup_name or index or not prompt or not sys.stdin.isatty():
        return default
    sys.stderr.write(f"  backup name [{default}]: ")
    sys.stderr.flush()
    return input().strip() or default


def _should_backup_replaced(name: str, backup_replaced: bool | None) -> bool:
    if backup_replaced is not None:
        return backup_replaced
    if not sys.stdin.isatty():
        return False
    sys.stderr.write(f"  backup old {name} before replace? [Y/n]: ")
    sys.stderr.flush()
    return input().strip().casefold() not in ("n", "no")


def _backup_attachment(att: dict[str, Any], c: Client, name: str) -> int:
    import mimetypes

    url = _attachment_url(att)
    if not url:
        raise APIError(0, f"cannot backup attachment with no URL: {att}")
    r = c._client.get(url, follow_redirects=True)
    if r.status_code >= 400:
        raise APIError(r.status_code, r.text[:500])
    mime = str(att.get("content_type") or mimetypes.guess_type(name)[0] or "application/octet-stream")
    return _upload_bytes(name, r.content, mime, c)


def update_planning_field(
    change_id: int,
    field_id: str,
    description: str | None = None,
    file_paths: list[str] | None = None,
    c: Client | None = None,
    duplicate: str = "prompt",
    backup_replaced: bool | None = None,
    backup_name: str | None = None,
) -> dict[str, Any]:
    """Update a planning field, merging new files with existing ones.

    If description is None, preserves existing description. When uploading files to
    an empty planning field, uses the filename as default description because
    Freshservice requires planning-field text in addition to attachments.
    duplicate controls same-name attachments: prompt, skip, replace, append.
    replace removes same-name attachment IDs; optional backup reuploads old bytes
    under a backup filename before replacing.
    Uses PUT for existing fields, POST to create new ones.
    """
    if c is None:
        c = get_client()

    # Fetch existing field content
    data = c.int_get(f"changes/{change_id}/planning-fields")
    fields = {f["name"]: f for f in data.get("change_planning_fields", []) if f.get("name")}
    existing = fields.get(field_id)

    attachments = list((existing or {}).get("attachments", []) or [])
    all_att_ids = [att_id for att_id in (_attachment_id(a) for a in attachments) if att_id is not None]

    paths = list(file_paths or [])
    if description is None and paths and not _planning_description(existing):
        description = _default_planning_description(paths)

    mode = duplicate.casefold()
    if mode not in {"prompt", "skip", "replace", "append"}:
        raise APIError(0, "--duplicate must be one of: prompt, skip, replace, append")

    changed_attachments = False
    skipped_files: list[str] = []
    for path in paths:
        name = Path(path).name
        matches = [a for a in attachments if _attachment_name(a) == name]
        action = mode
        if matches and action == "prompt":
            action = _prompt_duplicate_action(name, len(matches))
        if matches and action == "skip":
            sys.stderr.write(f"  skipped {name} (already attached)\n")
            skipped_files.append(name)
            continue
        if matches and action == "replace":
            replace_ids = {_attachment_id(a) for a in matches}
            all_att_ids = [att_id for att_id in all_att_ids if att_id not in replace_ids]
            changed_attachments = True
            if _should_backup_replaced(name, backup_replaced):
                for i, att in enumerate(matches):
                    backup_id = _backup_attachment(att, c, _choose_backup_name(name, backup_name, i, backup_replaced is None))
                    all_att_ids.append(backup_id)
        all_att_ids.append(upload_file(path, c))
        changed_attachments = True

    if existing:
        # Field exists — use PUT /api/_/changes/{id}/planning-fields/{field_id}
        body: dict[str, Any] = {}
        if description is not None:
            body["description"] = description
        if changed_attachments:
            body["attachments"] = all_att_ids
        if not body:
            return {"_fsv_noop": True, "skipped": skipped_files, "planning_field": field_id}
        return c.int_put(f"changes/{change_id}/planning-fields/{field_id}", body)
    else:
        # Field doesn't exist — use POST to create
        final_description = description or _default_planning_description(paths)
        body = {"description": final_description}
        if all_att_ids:
            body["attachments"] = all_att_ids
        return c.int_post(f"changes/{change_id}/planning-fields?id={field_id}", body)


def get_change_approvals(change_id: int, c: Client | None = None) -> list[dict[str, Any]]:
    """Fetch approval list for a change."""
    if c is None:
        c = get_client()
    data = c.int_get(f"changes/{change_id}/approvals")
    return data.get("approvals", [])


def get_change_assets(change_id: int, c: Client | None = None) -> list[dict[str, Any]]:
    if c is None:
        c = get_client()
    try:
        data = c.int_get(f"changes/{change_id}/assets")
        if "associated_assets" in data:
            return data.get("associated_assets") or []
    except APIError as e:
        if e.status != 404:
            raise
    items: list[dict[str, Any]] = []
    for atype in ("cis", "services", "softwares"):
        try:
            data = c.int_get(
                f"changes/{change_id}/associated-cis",
                params={"page": 1, "per_page": 100, "association_type": atype},
            )
        except APIError as e:
            if e.status != 404:
                raise
            continue
        items.extend(data.get("data", []))
    return items


def search_assets_for_change(change_id: int, query: str, page: int = 1, per_page: int = 30,
                             c: Client | None = None) -> dict[str, Any]:
    """Search assets available for association with a change."""
    if c is None:
        c = get_client()
    params: dict[str, Any] = {
        "entity": "change",
        "entity_id": change_id,
        "include_meta": "true",
        "page": page,
        "per_page": per_page,
    }
    if query:
        params["search_term"] = query
    return c.int_get("assets-to-associate", params)


def associate_assets(change_id: int, asset_ids: list[int], c: Client | None = None) -> None:
    """Associate asset display IDs with a change."""
    if c is None:
        c = get_client()
    c.int_put(f"changes/{change_id}/assets/associate", {"item_ids": asset_ids})


def dissociate_assets(change_id: int, asset_display_ids: list[int], c: Client | None = None) -> None:
    """Dissociate assets from a change by their display IDs."""
    if c is None:
        c = get_client()
    current = get_change_assets(change_id, c)
    by_display_id: dict[int, int] = {
        ci.get("display_id"): a["id"]
        for a in current
        if (ci := a.get("config_item", {})) and ci.get("display_id")
    }
    for did in asset_display_ids:
        cmdb_id = by_display_id.get(did)
        if cmdb_id is None:
            raise ValueError(f"asset {did} not found on change #{change_id}")
        c.int_put(f"changes/{change_id}/assets/detach", {"cmdb_request_id": cmdb_id})


def get_change_associations(change_id: int, c: Client | None = None) -> dict[str, list]:
    """Fetch associated tickets, problems, and releases for a change."""
    if c is None:
        c = get_client()
    def _get(path: str, key: str) -> list:
        try:
            return c.int_get(path).get(key, [])
        except APIError:
            return []
    tickets = _get(f"changes/{change_id}/tickets", "tickets")
    problems = _get(f"changes/{change_id}/problems", "problems")
    releases = _get(f"changes/{change_id}/releases", "releases")
    return {"tickets": tickets, "problems": problems, "releases": releases}


def search_change_tickets(query: str, c: Client | None = None) -> list[dict[str, Any]]:
    """Search tickets available for association with a change."""
    if c is None:
        c = get_client()
    data = c.int_get("changes/tickets/search", {"module_type": "change", "q": query})
    return data.get("tickets", [])


def get_change_activities(change_id: int, c: Client | None = None) -> list[dict[str, Any]]:
    """Fetch activity timeline for a change (follows next_page_url cursor)."""
    import re as _re
    if c is None:
        c = get_client()
    all_acts: list[dict[str, Any]] = []
    params: dict[str, Any] = {}
    while True:
        data = c.int_get(f"changes/{change_id}/activities", params=params)
        acts = data.get("activities", [])
        all_acts.extend(acts)
        nxt = data.get("next_page_url")
        if not nxt or not acts:
            break
        m = _re.search(r"start_token=(\S+)", nxt)
        if not m:
            break
        params = {"start_token": m.group(1)}
    return all_acts


def associate_ticket(change_id: int, ticket_ids: list[int], c: Client | None = None) -> None:
    """Associate tickets with a change."""
    if c is None:
        c = get_client()
    c.int_put(f"changes/{change_id}/tickets/associate", {"item_ids": ticket_ids, "module_type": "ticket"})


def dissociate_ticket(change_id: int, ticket_id: int, c: Client | None = None) -> None:
    """Remove a ticket association from a change."""
    if c is None:
        c = get_client()
    c.int_delete(f"changes/{change_id}/tickets/{ticket_id}",
                 body={"type": "unlink"}, params={"module_type": "ticket"})


def get_task_for_edit(change_id: int, task_id: int, c: Client | None = None) -> dict[str, Any]:
    """Fetch a single task prepared for editing."""
    if c is None:
        c = get_client()
    data = c.int_get(f"changes/{change_id}/tasks/{task_id}")
    task = data.get("task", data)
    return {k: v for k, v in task.items() if k not in TASK_READ_ONLY_FIELDS and v is not None}


TASK_READONLY_CF = {"team_name"}


def delete_task(change_id: int, task_id: int, c: Client | None = None) -> None:
    """Delete a task from a change."""
    if c is None:
        c = get_client()
    c.int_delete(f"changes/{change_id}/tasks/{task_id}")


def update_task(change_id: int, task_id: int, body: dict[str, Any], c: Client | None = None) -> dict[str, Any]:
    """PUT update a task on a change.

    Merges with existing task data. Custom fields are preserved when present,
    but not hardcoded per-tenant — server-side validation decides what is
    required for the current workspace.
    """
    if c is None:
        c = get_client()
    existing = c.int_get(f"changes/{change_id}/tasks/{task_id}").get("task", {})
    merged = {k: v for k, v in existing.items() if k not in TASK_READ_ONLY_FIELDS and v is not None}
    existing_cf = existing.get("custom_fields") or {}
    body_cf = body.pop("custom_fields", {}) or {}
    merged.update(body)
    final_cf = {
        k: v
        for k, v in {**existing_cf, **body_cf}.items()
        if k not in TASK_READONLY_CF and v is not None
    }
    if final_cf:
        merged["custom_fields"] = final_cf
    else:
        merged.pop("custom_fields", None)
    data = c.int_put(f"changes/{change_id}/tasks/{task_id}", {"change_task": merged})
    return data.get("task", data)


def download_planning_attachments(
    change_id: int,
    field_names: list[str] | None = None,
    out_dir: str | Path | None = None,
    include_description: bool = False,
    c: Client | None = None,
) -> list[Path]:
    """Download planning field attachments from a change.

    Args:
        change_id: Change ID
        field_names: Internal field names to download (None = all)
        out_dir: Output directory (default: ./CHN-{id}/)
        include_description: Also save description text as .md
        c: Client instance

    Returns:
        List of downloaded file paths
    """
    if c is None:
        c = get_client()

    data = c.int_get(f"changes/{change_id}/planning-fields")
    fields: list[dict[str, Any]] = data.get("change_planning_fields", [])

    out = Path(out_dir) if out_dir else Path.cwd() / f"CHN-{change_id}"
    out.mkdir(parents=True, exist_ok=True)

    downloaded: list[Path] = []
    seen_names: set[str] = set()

    for field in fields:
        name = field.get("name")
        label = (field.get("label") or name or "").strip()
        if not name:
            continue

        if field_names is not None and name not in field_names:
            continue

        description = (field.get("description") or "").strip()
        attachments: list[dict[str, Any]] = field.get("attachments", [])

        if not description and not attachments:
            continue

        # Save description as .md
        if include_description and description:
            safe_label = re.sub(r"[^a-zA-Z0-9_-]+", "_", label).strip("_") or name
            md_path = out / f"{safe_label}.md"
            if md_path.name not in seen_names:
                md_path.write_text(description, encoding="utf-8")
                downloaded.append(md_path)
                seen_names.add(md_path.name)

        # Download attachments
        for att in attachments:
            att_url = att.get("attachment_url") or att.get("canonical_url")
            if not att_url:
                continue

            att_name = att.get("name", "attachment")
            # Prepend field label for context, deduplicate by name
            safe_field = re.sub(r"[^a-zA-Z0-9_-]+", "_", label).strip("_") or ""
            final_name = f"{safe_field}_{att_name}" if safe_field else att_name
            att_path = out / final_name

            # Deduplicate filenames
            counter = 1
            while att_path.name in seen_names:
                stem = att_path.stem
                suffix = att_path.suffix
                att_path = out / f"{stem}_{counter}{suffix}"
                counter += 1
            seen_names.add(att_path.name)

            r = c._client.get(att_url, follow_redirects=True)
            if r.status_code != 200:
                continue

            att_path.write_bytes(r.content)
            downloaded.append(att_path)

    return downloaded


def _get_change_schema_fields() -> list[dict[str, Any]]:
    """Load change schema fields, preferring cache."""
    schema = load_schema(CHANGES)
    fields: list[dict[str, Any]] = schema.get("fields", []) or []
    if not fields:
        raw = schema.get("data", schema)
        if isinstance(raw, dict):
            fields = raw.get("change_fields", raw.get("fields", [])) or []
    return fields


def submit_change(body: dict[str, Any], c: Client | None = None) -> dict[str, Any]:
    """POST a new change via internal API (cookie + CSRF auth)."""
    if c is None:
        c = get_client()

    schema_fields = _get_change_schema_fields()
    sf_names = {f.get("name"): f for f in schema_fields}

    CORE_NAMES = {
        "requester_id", "email", "group_id", "agent_id", "change_type",
        "priority", "impact", "risk", "status", "category",
        "subject", "description", "planned_start_date", "planned_end_date",
        "planned_effort", "department_id", "change_window_id",
        "br_validation_excludes",
    }
    core: dict[str, Any] = {}
    custom_fields: dict[str, Any] = {}

    for k, v in body.items():
        sf = sf_names.get(k, {})
        is_default = sf.get("default_field", False)
        if k in CORE_NAMES or is_default:
            core[k] = v
        else:
            custom_fields[k] = v

    if custom_fields:
        core["custom_fields"] = custom_fields

    try:
        data = c.int_post("changes", core)
        return data.get("change", data)
    except APIError as e:
        err_body = e.body
        if isinstance(err_body, dict):
            errors = err_body.get("errors")
            if isinstance(errors, list):
                msgs = [f"{err.get('field', '?')}: {err.get('message', '?')}" for err in errors]
                raise RuntimeError("create failed:\n  " + "\n  ".join(msgs)) from e
            desc = err_body.get("description", str(err_body))
            raise RuntimeError(f"create failed: {desc}") from e
        raise
