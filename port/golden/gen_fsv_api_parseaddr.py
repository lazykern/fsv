"""Golden `email.utils.parseaddr` results (`crates/fsv-api/src/email_addr.rs`).

`ticket_conversation` reduces each CC value with `parseaddr(value)[1]`, and
the result feeds the plan payload and hash. Runs the export's Python (3.12,
strict parsing) over quoted local parts, comments, several `<>`, groups and
malformed values.

    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_parseaddr.py
"""
from __future__ import annotations

import json
import sys
from email.utils import parseaddr

from provenance import stamped

INPUTS = [
    "", " ", "person@example.com", "  person@example.com  ", "Acme User <person@example.com>",
    "<person@example.com>", "< person@example.com >", "\"Acme, User\" <person@example.com>",
    "\"Acme \\\"Q\\\" User\" <person@example.com>", "\"quoted local\"@example.com",
    "\"a\\\\b\"@example.com", "\"a.b\"@example.com", "person(comment)@example.com",
    "person@example.com (Acme User)", "(lead) person@example.com", "Acme (c1) User (c2) <p@example.com>",
    "Acme <p@example.com> (trailing)", "(nested (deep) comment) p@example.com", "(unclosed p@example.com",
    "p@example.com)", "\"(not a comment)\" <p@example.com>", "a <b@x.io> <c@x.io>", "<a@x.io><b@x.io>",
    "a@x.io, b@x.io", "Acme <a@x.io>, Bee <b@x.io>", "a@x.io b@x.io", "alice@example.com <bob@example.com>",
    "group: a@x.io, b@x.io;", "group: a@x.io;", "group:;", "empty group: ;", "Undisclosed recipients:;",
    "<@route1,@route2:person@example.com>", "<@route:person@example.com>", "person@[127.0.0.1]",
    "person@[127.0.0.1", "person@example.com@evil.com", "person@", "@example.com", "person", "Acme User",
    "first.last@example.com", "first . last @ example . com", "first..last@example.com",
    ".lead@example.com", "trail.@example.com", "Ünïcødé <ü@exämple.com>", "ü@exämple.com",
    "tab\tname <t@x.io>", "line\r\nfold <f@x.io>", "a\\@b@x.io", "\"unterminated <q@x.io>",
    "<unterminated@x.io", "unterminated@x.io>", "Name <>", "<>", "\"\" <e@x.io>", ",", ";", ":",
    "a@x.io,", ",a@x.io", "Acme User <person@example.com", "Acme. User <p@x.io>", "A.B. C <p@x.io>",
    "p@x.io;", "p@x.io:", "x <p@x.io> y", "\\", "a\\", "(a\\)) p@x.io", "\"a\\", "p@x.io (",
    "[x] <p@x.io>", "p@x.io [x]", "p@sub.[1.2.3.4]",
]

out = {"cases": [{"input": s, "name": parseaddr(s)[0], "email": parseaddr(s)[1]} for s in INPUTS]}
json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False)
