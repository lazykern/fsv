"""Which workspace's business rules a form loads, on accounts that differ.

Invented Acme/Globex data only; no network (a fake ``int_get``)."""
from __future__ import annotations

import copy
from typing import Any

import pytest
from typer.testing import CliRunner

import acme_change_form as A
import fsv.cli as cli
from fsv import config, formmeta
from fsv.errors import APIError


class Account:
    """Acme change form whose fields carry no workspace ids; the rules live
    in ``rules_ws`` (None: only the workspace-less path has them)."""

    def __init__(self, *, rules_ws: Any = 3, me: Any = None, workspaces: Any = None) -> None:
        self.rules_ws = rules_ws
        self.me = me
        self.workspaces = workspaces
        self.calls: list[str] = []
        f = A.fields()
        for x in f["change_fields"]:
            x.pop("workspace_id", None)
        self.fields = f

    def int_get(self, path: str, params: Any = None) -> Any:
        self.calls.append(path)
        if path == "change_form_fields":
            return copy.deepcopy(self.fields)
        if path == "bootstrap/me":
            if self.me is None:
                raise APIError(404, "not found", "GET", path)
            return self.me
        if path == "workspaces":
            if self.workspaces is None:
                raise APIError(403, "denied", "GET", path)
            return self.workspaces
        if path.endswith("business_rules"):
            want = "business_rules" if self.rules_ws is None else f"ws/{self.rules_ws}/business_rules"
            if path == want:
                r = A.rules()
                for x in r["business_rules"]:
                    x["workspace_id"] = self.rules_ws if self.rules_ws is not None else 1
                return r
            raise APIError(404, "not found", "GET", path)
        raise APIError(404, "not found", "GET", path)

    def rules_paths(self) -> list[str]:
        return [p for p in self.calls if p.endswith("business_rules")]


def test_workspaces_in_reads_current_primary_and_lists():
    assert formmeta.workspaces_in({"user": {"current_workspace_id": "4", "workspace_ids": [1, 4, 6]}}) == [4, 1, 6]
    assert formmeta.workspaces_in({"workspaces": [{"id": 1}, {"id": 5, "primary": True}]}) == [5, 1]
    assert formmeta.workspaces_in({"account": {"primary_workspace_id": 2}}) == [2]
    assert formmeta.workspaces_in({"nothing": True}) == []
    assert formmeta.workspaces_in([1, 2]) == []


def test_rules_default_to_the_account_workspace_from_bootstrap_me():
    acct = Account(rules_ws=3, me={"user": {"current_workspace_id": 3}})
    meta = formmeta.load_meta(acct, "change", "new", fresh=True)
    assert meta.rules["business_rules"]
    assert meta.workspace_id == 3
    assert acct.rules_paths() == ["ws/3/business_rules"]
    assert meta.engine().rules and not meta.engine().skipped
    assert formmeta.rules_note(meta.rules) is None


def test_rules_try_the_primary_workspace_from_the_workspaces_list():
    acct = Account(rules_ws=5, workspaces={"workspaces": [{"id": 1, "name": "IT"}, {"id": 5, "primary": True}]})
    meta = formmeta.load_meta(acct, "change", "new", fresh=True)
    assert meta.rules["business_rules"] and meta.workspace_id == 5
    assert acct.rules_paths()[0] == "ws/5/business_rules"


def test_rules_fall_back_to_the_workspace_less_path():
    acct = Account(rules_ws=None, me={"user": {"current_workspace_id": 3}})
    meta = formmeta.load_meta(acct, "change", "new", fresh=True)
    assert meta.rules["business_rules"]
    assert acct.rules_paths() == ["ws/3/business_rules", "business_rules"]
    assert meta.workspace_id is None


def test_account_lookup_only_when_the_metadata_workspace_fails():
    fake = A.FakeClient()  # fields carry workspace 2 and the rules answer there
    formmeta.load_meta(fake, "change", "new", fresh=True)
    paths = [p for _, p, _ in fake.calls]
    assert "bootstrap/me" not in paths and "workspaces" not in paths


def test_unloadable_rules_give_a_one_line_reason():
    acct = Account(rules_ws="nowhere")
    meta = formmeta.load_meta(acct, "change", "new", fresh=True)
    note = formmeta.rules_note(meta.rules)
    assert note is not None and "\n" not in note
    assert "no workspace found" in note and "HTTP 404" in note and "forms.workspace_id" in note
    assert any("could not be loaded" in w for w in formmeta.rules_warnings(meta.rules))


def test_config_workspace_is_an_override(monkeypatch):
    monkeypatch.setattr(config, "load_config", lambda: {"forms": {"workspace_id": "3"}})
    acct = Account(rules_ws=3, me={"user": {"current_workspace_id": 9}})
    meta = formmeta.load_meta(acct, "change", "new", fresh=True)
    assert meta.workspace_id == 3 and meta.rules["business_rules"]
    assert "bootstrap/me" not in acct.calls


def test_form_fields_prints_the_rules_note(monkeypatch):
    acct = Account(rules_ws="nowhere")
    monkeypatch.setattr(cli, "_client", lambda: acct)
    r = CliRunner().invoke(cli.app, ["form", "fields", "changes"])
    assert r.exit_code == 0, r.output
    flat = " ".join(r.output.split())
    assert "note: business rules not loaded" in flat


@pytest.mark.parametrize("status", [403, 404])
def test_workspace_path_refusal_then_account_workspace(status):
    class Acct(Account):
        def int_get(self, path: str, params: Any = None) -> Any:
            if path == "ws/7/business_rules":
                self.calls.append(path)
                raise APIError(status, "no", "GET", path)
            return super().int_get(path, params)

    acct = Acct(rules_ws=3, me={"user": {"current_workspace_id": 3}})
    acct.fields["change_fields"][5]["workspace_id"] = 7  # a stale metadata workspace
    meta = formmeta.load_meta(acct, "change", "new", fresh=True)
    assert acct.rules_paths() == ["ws/7/business_rules", "ws/3/business_rules"]
    assert meta.workspace_id == 3
