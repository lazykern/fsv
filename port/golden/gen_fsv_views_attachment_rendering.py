"""Golden `pages._attachment_text` (Python ec7a6ef) for fsv-views `pages`:
attachment-only planning, description and conversation lines render once
(paperclip rows only); prose mentioning a file is kept. Mirrors
`test_attachment_rendering.py` (the renderer tests; its CLI/TUI tests reach the
same renderer).

    cd /tmp/fsv-py-ec7a6ef && FSV_PY_SRC=$PWD FSV_PY_COMMIT=ec7a6ef... uv run python \
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


out = {"planning": [], "description": [], "conversation": [], "attachment_text": []}
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
for text, files in [(NAME, []), (f"\n\n{NAME}\nx\n\n", [NAME]), (f"•  {NAME}", [NAME]), (f"•{NAME}", [NAME]),
                    (f"\t{NAME}\r\n{OTHER}", [NAME, OTHER]), ("a\n\nb", ["a"]), ("", [NAME])]:
    out["attachment_text"].append([text, files, pages._attachment_text(text, files)])
Path(sys.argv[1]).write_text(json.dumps(stamped(out), indent=1, ensure_ascii=False) + "\n")
