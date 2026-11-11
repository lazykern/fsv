"""Synthetic Acme/Globex read-history and evidence regressions."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, pages, views
from fsv.errors import APIError
from fsv.resources import CHANGES, TICKETS


@pytest.mark.parametrize('resource', ['tickets', 'changes', 'problems'])
def test_root_cli_note_registration_preserves_resource_flows(resource):
    result = CliRunner().invoke(cli.app, [resource, 'add-note', '--help'])
    assert result.exit_code == 0, result.output
    assert ('--expect-plan' in result.output) is (resource == 'tickets')
    assert not TICKETS.has_notes


class History:
    def __init__(self, responses):
        self.responses = iter(responses)
        self.calls = []

    def int_get(self, path, params=None):
        self.calls.append((path, params))
        response = next(self.responses)
        if isinstance(response, Exception):
            raise response
        return response


@pytest.mark.parametrize('key', ['notes', 'tasks', 'activities'])
def test_short_nonterminal_and_full_terminal(key):
    c = History([{key: [{'id': 1}], 'meta': {'has_next': True, 'count': 3}},
                 {key: [{'id': 2}, {'id': 3}], 'meta': {'has_next': False, 'count': 3}}])
    rows, meta, miss = views.collect_pages(c, 'changes/12/' + key, key, {'per_page': 2})
    assert [r['id'] for r in rows] == [1, 2, 3]
    assert meta['complete'] and not meta['has_more'] and meta['count'] == 3
    assert miss is None and len(c.calls) == 2


@pytest.mark.parametrize('continuation', [
    {'next_page_url': '/api/_/changes/12/activities?start_token=acme'},
    {'next_cursor': 'globex'},
])
def test_cursor_loop_is_bounded_and_deduplicated(continuation):
    c = History([{'activities': [{'id': 1}], **continuation}] * 3)
    rows, meta, miss = views.collect_pages(c, 'changes/12/activities', 'activities')
    assert rows == [{'id': 1}] and len(c.calls) == 2
    assert not meta['complete'] and 'cycle' in meta['partial']
    assert meta['continuation'] and miss is None


@pytest.mark.parametrize('status', [403, 429, 500])
def test_detail_page_two_failure_retains_rows_and_metadata(status):
    c = History([{'notes': [{'id': 1, 'body': 'Acme evidence'}], 'meta': {'has_next': True}},
                 APIError(status, {'message': 'Unavailable'})])
    td = views.load_tab(c, CHANGES, {'id': 12}, 'notes')
    assert td.data[0]['id'] == 1 and td.missing is None
    assert not td.meta['complete'] and td.meta['continuation']['page'] == 2
    assert str(status) in td.notes[0]
    assert td.to_json()['meta'] == td.meta
    rendered = '\n'.join(pages.tab_lines(td))
    assert 'Acme evidence' in rendered and 'incomplete' in rendered


def test_association_metadata_survives_parts():
    c = History([{'changes': [{'id': 1}], 'meta': {'has_next': True}},
                 {'changes': [{'id': 2}], 'meta': {'has_next': False}},
                 {'changes': [], 'meta': {'has_next': False}}, {'problems': []}])
    td = views.load_tab(c, TICKETS, {'id': 12}, 'associations')
    assert len(td.parts['Changes']) == 2
    assert td.to_json()['parts_meta']['Changes']['pages'] == 2


class Resolution:
    def __init__(self, response):
        self.response = response

    def v2_get(self, path):
        assert path == 'tickets/12/resolution_note'
        if isinstance(self.response, Exception):
            raise self.response
        return self.response


@pytest.mark.parametrize('status', [403, 500])
def test_resolution_denied_or_unavailable_is_not_absent(status):
    td = views.load_tab(Resolution(APIError(status, {})), TICKETS,
                        {'id': 12, 'stats': {'resolved_at': '2031-01-02T10:00:00Z'}}, 'resolution')
    assert td.missing.status == status and not td.meta.get('absent')
    text = '\n'.join(pages.tab_lines(td))
    assert str(status) in text and 'Resolved' in text and 'no resolution' not in text


def test_resolution_singular_shape_and_absence():
    td = views.load_tab(Resolution({'resolution_note': {'body': '<p>Acme restored</p>'}}),
                        TICKETS, {'id': 12}, 'resolution')
    assert td.data['note'] == 'Acme restored'
    absent = views.load_tab(Resolution(APIError(404, {})), TICKETS, {'id': 12}, 'resolution')
    assert absent.missing is None and absent.to_json()['meta']['absent']
    assert 'no resolution note (404)' in '\n'.join(pages.tab_lines(absent))


@pytest.mark.parametrize('fail', [False, True])
def test_ticket_download_includes_page_two_or_discloses_partial(monkeypatch, tmp_path, fail):
    from fsv import create
    c = History([{'ticket': {'id': 12}},
                 {'conversations': [{'id': 1, 'attachments': [{'name': 'first.txt'}]}],
                  'meta': {'has_next': True}},
                 APIError(403, {}) if fail else
                 {'conversations': [{'id': 2, 'attachments': [{'name': 'later.txt'}]}],
                  'meta': {'has_next': False}}])
    monkeypatch.setattr(cli, '_client', lambda: c)
    def download(att, out_dir, force=False, c=None):
        target = out_dir / att['name']
        target.write_bytes(b'Acme evidence')
        return {'name': att['name'], 'path': str(target), 'size': target.stat().st_size}
    monkeypatch.setattr(create, 'download_attachment', download)
    result = CliRunner().invoke(cli.app, ['tickets', 'download', '12', '--all', '--dir', str(tmp_path), '--json'])
    assert result.exit_code == 0, result.output
    manifest = json.loads(result.stdout)
    assert (tmp_path / 'first.txt').read_bytes() == b'Acme evidence'
    if fail:
        warning = next(r for r in manifest if r['status'] == 'incomplete')
        assert not warning['complete'] and warning['meta']['continuation']['page'] == 2
    else:
        assert (tmp_path / 'later.txt').read_bytes() == b'Acme evidence'
        assert [r['name'] for r in manifest] == ['first.txt', 'later.txt']


def test_list_metadata_is_authoritative():
    assert views.list_meta({'meta': {'has_next': True}}, [1], 1, 30) == (None, True)
    assert views.list_meta({'meta': {'has_next': False}}, [1] * 30, 1, 30) == (None, False)
    assert views.list_meta({'next_page_url': '?page=2'}, [1], 1, 30) == (None, True)
    assert views.list_meta({'meta': {'total_count': 30}}, [1] * 30, 1, 30) == (30, False)


def test_bounded_history_discloses_continuation():
    c = History([{'notes': [{'id': 1}], 'meta': {'has_next': True}}])
    rows, meta, miss = views.collect_pages(c, 'changes/12/notes', 'notes', {'page': 1}, max_pages=1)
    assert rows and miss is None and not meta['complete']
    assert meta['continuation']['page'] == 2 and 'page limit' in meta['partial']


def test_first_page_denied_is_missing_not_empty():
    td = views.load_tab(History([APIError(403, {})]), CHANGES, {'id': 12}, 'notes')
    assert td.data is None and td.missing.status == 403
    assert 'no notes' not in '\n'.join(pages.tab_lines(td))


def test_count_continues_short_page_without_boolean():
    c = History([{'notes': [{'id': 1}], 'meta': {'count': 2}},
                 {'notes': [{'id': 2}], 'meta': {'count': 2}}])
    rows, meta, _ = views.collect_pages(c, 'changes/12/notes', 'notes', {'per_page': 30})
    assert len(rows) == 2 and meta['complete']


def test_continuation_never_switches_endpoint():
    c = History([{'activities': [{'id': 1}],
                  'next_page_url': '/api/v2/changes/12/activities?start_token=acme'}])
    rows, meta, _ = views.collect_pages(c, 'changes/12/activities', 'activities')
    assert rows and not meta['complete'] and len(c.calls) == 1
    assert 'unsupported continuation endpoint' in meta['partial']
