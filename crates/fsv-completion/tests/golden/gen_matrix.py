"""Python side of `src/dynamic/matrix.rs`: upstream
`tests/test_completion_matrix.py` and `tests/test_completion_coverage_subprocess.py`
run against `fsv.completion` / `fsv._complete`, one scenario per test fixture.

Each scenario starts from fresh config/cache dirs (domain `acme.freshservice.com`,
as in the upstream conftest), records the exact file text the Rust side must
write, then the output of every call. Run with the Python tree on the path:

    FSV_PY_SRC=/tmp/py2956fce/src FSV_PY_COMMIT=<sha> PYTHONPATH=$FSV_PY_SRC \\
        python gen_matrix.py > ../fixtures/matrix_expected.json
"""
import contextlib
import io
import json
import os
import pathlib
import sys
import tempfile
from types import SimpleNamespace

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[3] / "fsv-cli" / "tests" / "golden"))
from pyrev import python_rev  # noqa: E402

DOMAIN = "acme.freshservice.com"

FIELDS = [
    {"name": "status", "label": "State", "default_field": True, "field_type": "default_status",
     "choices": [{"id": 47, "value": "Review"}, {"id": 83, "name": "Done"}, {"id": 47, "value": "Review"}]},
    {"name": "cf_enabled", "label": "Enabled", "field_type": "custom_checkbox"},
    {"name": "planned_start_date", "label": "Start", "field_type": "default_date"},
    {"name": "cf_plan", "label": "Test Plan", "field_type": "planning_field"},
    {"name": "cf_owner", "label": "Owner", "field_type": "custom_lookup", "lookup_config": {"link": "/lookups/people"}},
    {"name": "requester", "label": "Requester", "field_type": "custom_lookup"},
    {"name": "agent", "label": "Agent", "field_type": "default_agent"},
    {"name": "cf_title", "label": "Title", "field_type": "custom_text"},
    {"label": "Nameless"},
]
GROUPS = [{"id": 91, "name": "Acme Ops", "description": "Acme support"}, {"id": 92, "name": "Globex"}, {"id": 93}]
PEOPLE = [{"id": 71, "email": "Alex@example.com", "value": "Alex Acme"},
          {"id": 72, "name": "Al Globex"}, {"id": 73, "value": "Unrelated"}, {}]
LOOKUP = [{"name": "Unrelated"}, {"name": "Al Globex"}, {"name": "Alex Acme", "email": "Alex@example.com"}]


class Fake:
    """`client.get_client()` stand-in; `None` rows make `get_client` raise."""

    def __init__(self, autocomplete=None, lookup=None, fail=False):
        self.rows, self.lookup, self.fail, self.calls = autocomplete, lookup, fail, []

    def get_client(self):
        if self.fail:
            raise RuntimeError("synthetic denial")
        return SimpleNamespace(autocomplete=self._auto, lookup_choices=self._lookup)

    def _auto(self, kind, text, params=None):
        self.calls.append(["autocomplete", kind, text, params])
        return list(self.rows or [])

    def _lookup(self, link, token):
        self.calls.append(["lookup_choices", link, token])
        return list(self.lookup or [])


class Scenario:
    def __init__(self, name):
        self.name = name
        self.root = pathlib.Path(tempfile.mkdtemp(prefix="fsvmatrix"))
        os.environ["FSV_CONFIG_DIR"] = str(self.root / "cfg")
        os.environ["FSV_CACHE_DIR"] = str(self.root / "cache")
        os.environ.pop("FSV_COMPLETION_NETWORK", None)
        for mod in [m for m in sys.modules if m == "fsv" or m.startswith("fsv.")]:
            del sys.modules[mod]
        from fsv import cache, client, completion, config
        self.cache, self.client, self.C, self.config = cache, client, completion, config
        config.refresh_paths()
        config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
        config.CONFIG_FILE.write_text(json.dumps({"domain": DOMAIN}))
        config.reload()
        completion._CFG = config.CONFIG_DIR
        self.cases = []
        self.files = None

    def snapshot(self):
        """Freeze the file set the Rust side writes before running the calls."""
        self.files = {str(p.relative_to(self.root)): p.read_text() for p in sorted(self.root.rglob("*")) if p.is_file()}

    def call(self, fn, args, *, network=None, fake=None, extra=None):
        if self.files is None:
            self.snapshot()
        fake = fake or Fake()
        if network is not None:
            if network:
                os.environ["FSV_COMPLETION_NETWORK"] = "1"
            else:
                os.environ.pop("FSV_COMPLETION_NETWORK", None)
        self.client.get_client = fake.get_client
        from fsv.resources import REGISTRY
        C = self.C
        res = REGISTRY.get(args[0]) if args and fn not in ("thin", "remote_user_values", "complete") else None
        if fn == "thin":
            out = io.StringIO()
            from fsv import _complete
            argv, sys.argv = sys.argv, ["fsv._complete", *args]
            try:
                with contextlib.redirect_stdout(out):
                    _complete.main()
            finally:
                sys.argv = argv
            want = out.getvalue().splitlines()
        elif fn == "complete":
            from fsv import _complete
            completer, resource, token, *rest = args
            want = [list(p) for p in _complete._completions(completer, resource, token, rest)]
        elif fn == "cached_fields":
            want = C._cached_fields(res)
        elif fn == "cached_filters":
            want = [[f["id"], f["name"]] for f in C._cached_filters(res)]
        elif fn == "cached_groups":
            want = C._cached_groups()
        elif fn == "find_field":
            want = C._find_field(res, args[1])
        elif fn == "complete_update_choice":
            want = [list(p) for p in C.complete_update_choice(res, args[1])(args[2])]
        elif fn == "remote_user_values":
            want = [list(p) for p in C._remote_user_values(*args)]
        elif fn == "network":
            want = C._completion_network()
        elif fn == "config_load":
            want = C._config_load()
        else:
            raise SystemExit(f"unknown fn {fn}")
        try:
            network = C._completion_network()
        except Exception:  # `"completion": null`; `_complete.main` swallows it
            network = None
        self.cases.append({"fn": fn, "args": args, "network": network,
                           "env": os.environ.get("FSV_COMPLETION_NETWORK"),
                           "remote": {"autocomplete": fake.rows, "lookup": fake.lookup, "fail": fake.fail},
                           "want": want, "calls": fake.calls})
        return want

    def done(self):
        if self.files is None:
            self.snapshot()
        return {"name": self.name, "files": self.files, "cases": self.cases}


def cached(s):
    s.cache.save(s.config.schema_cache_path("changes"), "schema", FIELDS)
    s.cache.save(s.config.groups_cache_path(), "groups", {"groups": GROUPS})
    s.cache.save(s.config.filters_cache_path("changes"), "filters", {"filters": [
        {"id": 17, "name": "Acme active"}, {"id": "all", "name": "All Changes"}, None, {}]})


def thin_caches(s):
    s.cache.save(s.config.schema_cache_path("changes"), "schema", [
        {"name": "status", "label": "State", "field_type": "default_status", "default_field": True,
         "choices": [{"id": 47, "value": "Review"}]},
        {"name": "test_plan", "label": "Test Plan", "field_type": "planning_field"}])
    s.cache.save(s.config.groups_cache_path(), "groups", {"groups": [{"id": 91, "name": "Acme Ops", "description": "Acme support"}]})
    s.cache.save(s.config.filters_cache_path("changes"), "filters", {"filters": [{"id": "all", "name": "All Changes"}]})


def thin_subprocess():
    out = []
    s = Scenario("thin")
    thin_caches(s)
    os.environ["FSV_COMPLETION_NETWORK"] = "0"
    for shell in ("fish", "bash", "zsh"):
        s.call("thin", [shell, "complete_format", "-", "j"])
    for name, token in [("complete_where", "sta"), ("complete_set", "status=R"), ("complete_filter_name", "All"),
                        ("complete_field_names", "sta"), ("complete_choice_field_names", "sta"),
                        ("complete_planning_field_names", "Test"), ("complete_lookup_kind", "gr"),
                        ("complete_update_group_id", "Ac"), ("complete_update_agent_id", "al")]:
        s.call("thin", ["fish", name, "changes", token])
    for tokens in (["--kind", "group"], ["-k", "group"], ["--kind=group"], ["-k=group"]):
        s.call("thin", ["fish", "complete_lookup_query", "changes", "Ac", "--", "changes", "lookup", *tokens])
    for args in ([], ["fish"], ["fish", "complete_where", "changes"], ["fish", "unknown", "changes", ""],
                 ["fish", "complete_where", "not-a-resource", "st"], ["fish", "complete_format", "-", "json"],
                 ["fish", "complete_config_value", "-", "off"],
                 ["fish", "complete_lookup_query", "changes", "Ac", "--", "--kind"],
                 ["fish", "complete_lookup_query", "changes", "Ac", "--", "unrelated"]):
        s.call("thin", args)
    out.append(s.done())

    s = Scenario("thin_malformed_config")
    thin_caches(s)
    os.environ["FSV_COMPLETION_NETWORK"] = "0"
    s.config.CONFIG_FILE.write_text(json.dumps({"domain": s.config.DOMAIN, "completion": None}))
    s.call("thin", ["fish", "complete_update_agent_id", "-", "al"])
    out.append(s.done())
    os.environ.pop("FSV_COMPLETION_NETWORK", None)
    return out


def matrix():
    out = []
    s = Scenario("cached")
    cached(s)
    s.call("cached_fields", ["changes"])
    s.call("cached_filters", ["changes"])
    s.call("cached_groups", [])
    s.call("complete", ["complete_filter_name", "changes", "Ac"])
    s.call("complete", ["complete_group_query", "-", "ac"])
    s.call("complete", ["complete_update_group_id", "-", "9"])
    s.call("complete", ["complete_update_group_id", "-", "Ac"])
    for name in ("default:State", "custom:Enabled", "custom:State", "title", "cf_"):
        s.call("find_field", ["changes", name])
    s.call("complete", ["complete_field_names", "changes", "sta"])
    s.call("complete", ["complete_choice_field_names", "changes", "sta"])
    s.call("complete", ["complete_planning_field_names", "changes", "Test"])
    s.call("complete", ["complete_lookup_kind", "changes", "gr"])
    for token in ("cf_enabled=t", "cf_enabled=", "status=Re", "unknown=x", "cf_title=Acme", "'tit",
                  "planned_start_date=2025"):
        s.call("complete", ["complete_set", "changes", token])
    s.call("complete", ["complete_where", "changes", "due_by>=2025"])
    s.call("complete_update_choice", ["changes", "status", ""])
    for token in ("cf_owner=al", "requester=al", "agent=al"):
        offline = Fake(autocomplete=PEOPLE, lookup=LOOKUP)
        s.call("complete", ["complete_set", "changes", token], network=False, fake=offline)
        s.call("complete", ["complete_where", "changes", token], network=False, fake=offline)
    s.call("complete", ["complete_update_agent_id", "-", "al"], network=False, fake=Fake(autocomplete=PEOPLE))
    for name, token in [("complete_format", "j"), ("complete_store", "arg"), ("complete_config_key", "forms"),
                        ("complete_config_value", "of"), ("complete_cache_target", "v")]:
        s.call("complete", [name, "-", token])

    # remote fixture: opted in, every autocomplete kind answers PEOPLE
    for kind in ("requesters", "agents"):
        s.call("remote_user_values", [kind, " al "], network=True, fake=Fake(autocomplete=PEOPLE))
        s.call("remote_user_values", [kind, "a"], network=True, fake=Fake(autocomplete=PEOPLE))
    for kind in ("requester", "agent", "group", "status", ""):
        token = "Re" if kind == "status" else "Ac" if kind == "group" else "al"
        s.call("complete", ["complete_lookup_query", "changes", token, "--kind", kind], network=True,
               fake=Fake(autocomplete=PEOPLE))
    s.call("remote_user_values", ["agents", "al"], network=True,
           fake=Fake(autocomplete=[{"email": f"alex{i}@example.com"} for i in range(20)]))
    s.call("remote_user_values", ["agents", "al"], network=True, fake=Fake(fail=True))
    s.call("complete", ["complete_update_agent_id", "-", "al"], network=True, fake=Fake(fail=True))
    s.call("complete", ["complete_set", "changes", "cf_owner=al"], network=True, fake=Fake(lookup=LOOKUP))
    many = [{"name": f"Alex {i}"} for i in range(25)]
    s.call("complete", ["complete_set", "changes", "cf_owner=al"], network=True, fake=Fake(lookup=many))
    s.call("complete", ["complete_set", "changes", "cf_owner=a"], network=True, fake=Fake(lookup=many))
    s.call("complete", ["complete_set", "changes", "cf_owner=al"], network=True, fake=Fake(fail=True))
    s.call("complete", ["complete_set", "changes", "requester=al"], network=True, fake=Fake(autocomplete=PEOPLE))
    s.call("complete", ["complete_where", "changes", "agent!=al"], network=True, fake=Fake(autocomplete=PEOPLE))
    s.call("complete", ["complete_where", "changes", "requester=al"], network=True, fake=Fake(autocomplete=PEOPLE))
    s.call("complete", ["complete_update_agent_id", "-", "al"], network=True, fake=Fake(autocomplete=PEOPLE))
    out.append(s.done())

    for shape in ("legacy", "list", "dict", "fallback"):
        s = Scenario(f"families_{shape}")
        for path, key, rows in [(s.config.groups_cache_path(), "groups", [{"id": 91, "name": "Acme"}]),
                                (s.config.filters_cache_path("changes"), "filters", [{"id": 17, "name": "Acme active"}])]:
            path.parent.mkdir(parents=True, exist_ok=True)
            doc = ({key: rows} if shape == "legacy" else {"data": rows} if shape == "list"
                   else {"data": {key: rows}} if shape == "dict" else {"data": "invalid", key: rows})
            path.write_text(json.dumps(doc))
        s.call("cached_groups", [])
        s.call("cached_filters", ["changes"])
        out.append(s.done())

    for i, body in enumerate(("not json", "[]", "{}")):
        s = Scenario(f"corrupt_{i}")
        path = s.config.schema_cache_path("changes")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)
        s.call("cached_fields", ["changes"])
        s.call("complete", ["complete_where", "changes", "sta"], fake=Fake(fail=True))
        s.call("complete", ["complete_where", "changes", "status=Pe"], fake=Fake(fail=True))
        s.call("cached_groups", [])
        s.call("cached_filters", ["changes"])
        out.append(s.done())

    s = Scenario("stale")
    cached(s)
    s.cache.mark_stale(s.config.schema_cache_path("changes"))
    s.call("cached_fields", ["changes"])
    out.append(s.done())

    s = Scenario("other_tenant")
    cached(s)
    s.cache.mark_stale(s.config.schema_cache_path("changes"))
    s.config.CONFIG_FILE.write_text(json.dumps({"domain": "globex.example.com"}))
    s.config.reload()
    s.call("cached_fields", ["changes"])
    s.call("cached_groups", [])
    s.call("cached_filters", ["changes"])
    out.append(s.done())

    for i, setting in enumerate(("on", True, "true", "1", "off", False)):
        s = Scenario(f"network_setting_{i}")
        s.config.CONFIG_FILE.write_text(json.dumps({"completion": {"network": setting}}))
        s.call("network", [])
        out.append(s.done())

    s = Scenario("config_missing")
    s.config.CONFIG_FILE.unlink()
    s.call("config_load", [])
    s.call("network", [])
    out.append(s.done())
    s = Scenario("config_broken")
    s.config.CONFIG_FILE.write_text("broken")
    s.call("config_load", [])
    s.call("network", [])
    s.call("network", [], network=True)
    out.append(s.done())
    return out


def main():
    scenarios = thin_subprocess() + matrix()
    doc = {"_python": python_rev(), "domain": DOMAIN, "scenarios": scenarios}
    print(json.dumps(doc, indent=1, ensure_ascii=False))


if __name__ == "__main__":
    main()
