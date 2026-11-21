"""Golden detail lines (Python 7f58023) for fsv-views `pages`.

`7f58023` reverts the attachment-only dedupe: description, planning text and
conversation bodies render in full, and paperclip rows render as well.

    cd /tmp/fsv-py-7f58023 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=7f58023... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_views_attachment_rendering.py \
        /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-views/tests/fixtures/attachment_rendering.json
"""
import datetime as dt
import json
import sys
from pathlib import Path

from fsv import pages, views as V
from fsv.resources import CHANGES
from provenance import stamped

NAME = "Acme_plan_v1.xlsx"
OTHER = "Acme_checklist.txt"
UTC = dt.timezone.utc
BODIES = [
    f"<div>Acme rollout</div><div>{NAME}</div>",
    f'<p>Acme rollout</p><p><a href="https://files.example.com/701">{NAME}</a></p>',
    f'<p>Acme rollout</p><ul><li><a href="https://files.example.com/701">{NAME}</a></li></ul>',
    f"Acme rollout\n{NAME}\n{NAME}",
    NAME,
    f"Review {NAME} before deploying.\nAcme_plan_v1.xlsx.backup\n{NAME}",
    f"  • {NAME}  \n\n\nkeep\n\n{OTHER}\n",
    "",
]


def detail(text, files, record=None):
    p = V.PlanningEntry("go_live_checklist", "Acme planning", "", text, files)
    return V.Detail(CHANGES, record or {"id": 501}, {}, {}, UTC, [], planning=[p])


out = {"planning": [], "description": [], "conversation": []}
for body in BODIES:
    text = V.html_to_text(body)
    out["planning"].append({"body": body, "text": text, "files": [NAME, OTHER], "lines": pages.main_lines(detail(text, [NAME, OTHER]))})
for body in BODIES[:2]:
    record = {"id": 501, "description": body, "attachments": [{"id": 701, "name": NAME, "size": 123}]}
    out["description"].append({"record": record, "lines": pages.main_lines(detail("", [], record))})
for body in [*BODIES[:2], NAME, "", "<div>Acme evidence</div>"]:
    for files in [[{"id": 701, "name": NAME}, {"id": 702, "name": OTHER}], []]:
        cv = {"id": 901, "body": body, "source": 2, "private": True, "user": {"name": "Acme Agent"}, "attachments": files}
        out["conversation"].append({"cv": cv, "lines": pages.conversation_lines(cv, UTC)})
Path(sys.argv[1]).write_text(json.dumps(stamped(out), indent=1, ensure_ascii=False) + "\n")
