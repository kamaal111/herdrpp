"""Production launcher routing, configuration and failure proofs through real SSH."""
import json
import re
from pathlib import Path
import shutil
import time

import pytest

from ssh_gate import lab
from terminal import Terminal

BINARY = '/output/build/release/herdrpp'


@pytest.fixture(autouse=True)
def capture_terminal_evidence(lab, request):
    yield
    output = Path('/output/ssh-diagnostics') / request.node.name
    output.mkdir(parents=True, exist_ok=True)
    for index, terminal in enumerate(lab.clients):
        (output / f'client-{index}.ansi').write_bytes(terminal.raw)
        (output / f'client-{index}.txt').write_text(terminal.text)


def profiles(lab):
    return json.loads(lab.run('herdr', 'machine', 'list', '--json').stdout)


def open_client(lab, *arguments, environment=None):
    terminal = Terminal([BINARY, 'launch', *arguments], environment or lab.env, lab.root)
    lab.clients.append(terminal)
    return terminal


def wait_wire(terminal, needle, offset=0):
    deadline = time.monotonic() + 20
    while needle not in terminal.raw[offset:]:
        assert time.monotonic() < deadline, terminal.text
        assert terminal.process.poll() is None, terminal.text
        terminal.read()


from picker_driver import select_machine, browse_machine, open_picker


def wait_pane_line(lab, marker, session, remote=True, pane_id=None):
    deadline = time.monotonic() + 20
    while True:
        lines = lab.pane_text(session, remote=remote, pane_id=pane_id).splitlines()
        if marker in lines:
            return lines
        assert time.monotonic() < deadline, lines
        time.sleep(0.05)


def assert_routed(lab, terminal, marker, session, remote=True):
    terminal.send(f"printf '{marker}\\n'\r".encode())
    terminal.wait(marker)
    assert marker in wait_pane_line(lab, marker, session, remote=remote)


def test_remembered_remote_does_not_override_requested_local_attachment(lab):
    selected = Path(lab.env['XDG_STATE_HOME']) / 'herdr/client/endpoint-selection.json'
    selected.parent.mkdir(parents=True, exist_ok=True)
    selected.write_text(json.dumps(dict(version=1, selected_profile=profiles(lab)[1]['id'])))
    before = selected.read_bytes()
    catalog = selected.with_name('endpoints.json').read_bytes()
    terminal = open_client(lab)
    terminal.wait('LOCAL_ONLY')
    assert_routed(lab, terminal, 'REMEMBERED_REMOTE_STILL_LOCAL', 'default', remote=False)
    assert selected.read_bytes() == before
    assert selected.with_name('endpoints.json').read_bytes() == catalog
    assert 'REMEMBERED_REMOTE_STILL_LOCAL' not in lab.pane_text('beta')


def test_duplicate_unicode_labels_select_by_session_and_preserve_profile_identity(lab):
    records = profiles(lab)
    lab.run('herdr', 'machine', 'rename', records[0]['id'], '--label', '開発 🐈')
    lab.run('herdr', 'machine', 'rename', records[1]['id'], '--label', '開発 🐈')
    terminal = open_client(lab)
    terminal.wait('LOCAL_ONLY')
    select_machine(terminal, 'beta')
    terminal.wait('BETA_ONLY')
    assert_routed(lab, terminal, 'DUPLICATE_LABEL_CHOSE_BETA', 'beta')
    assert 'DUPLICATE_LABEL_CHOSE_BETA' not in lab.pane_text('alpha')
    assert profiles(lab)[1]['id'] == records[1]['id']
    fallbacks = re.findall(rb'herdrpp:[^\r\n]*using reattachment', terminal.raw)
    assert fallbacks == []


def test_hidden_sidebar_disabled_native_goto_and_custom_prefix_allow_machine_roundtrip(lab):
    local_config = Path(lab.env['HERDR_CONFIG_PATH'])
    remote_config = Path(lab.remote['HERDR_CONFIG_PATH'])
    custom = local_config.read_text().replace('[ui]\n', '[ui]\nsidebar_start_collapsed = true\nsidebar_collapsed_mode = "hidden"\nmouse_capture = false\n') + '\n[keys]\nprefix = "ctrl+a"\ngoto = []\n'
    local_config.write_text(custom)
    remote_config.write_text(custom)
    lab.run('herdr', '--session', 'alpha', 'server', 'reload-config', env=lab.remote)
    terminal = open_client(lab)
    wait_wire(terminal, b'\x1b[?7l')
    assert_routed(lab, terminal, 'HIDDEN_INITIAL_LOCAL_ROUTE', 'default', remote=False)
    assert 'spaces' not in '\n'.join(terminal.screen.display)
    offset = len(terminal.raw)
    select_machine(terminal, 'Alpha')
    wait_wire(terminal, b'\x1b[?7l', offset)
    assert_routed(lab, terminal, 'HIDDEN_REMOTE_ROUTE', 'alpha')
    offset = len(terminal.raw)
    select_machine(terminal, 'Local')
    assert_routed(lab, terminal, 'HIDDEN_LOCAL_ROUTE', 'default', remote=False)
    assert local_config.read_text() == custom
    assert remote_config.read_text() == custom


def test_unreachable_preflight_preserves_current_local_client_and_server(lab):
    catalog = Path(lab.env['XDG_STATE_HOME']) / 'herdr/client/endpoints.json'
    value = json.loads(catalog.read_text())
    value['ssh'][0]['target'] = 'ssh://root@127.0.0.1:1'
    catalog.write_text(json.dumps(value))
    terminal = open_client(lab)
    terminal.wait('LOCAL_ONLY')
    before = lab.snapshot('default', remote=False)['panes'][0]['terminal_id']
    offset = len(terminal.raw)
    select_machine(terminal, 'Alpha')
    terminal.wait('machine request failed', '0 results')
    assert terminal.process.poll() is None
    terminal.send(b'\x1b')
    terminal.wait('LOCAL_ONLY')
    assert_routed(lab, terminal, 'FAILED_PREFLIGHT_STILL_LOCAL', 'default', remote=False)
    assert lab.snapshot('default', remote=False)['panes'][0]['terminal_id'] == before
    assert 'FAILED_PREFLIGHT_STILL_LOCAL' not in lab.pane_text('alpha')


def test_nonzero_candidate_exit_reattaches_exact_previous_local_server(lab):
    wrapper = lab.root / 'herdr-fail-once'
    marker = lab.root / 'candidate-failed'
    real = shutil.which('herdr')
    wrapper.write_text(f'''#!/usr/bin/python3
import os, pathlib, sys
marker = pathlib.Path({str(marker)!r})
if sys.argv[1:2] == ['--remote'] and not marker.exists():
    marker.write_text('failed once')
    sys.exit(17)
os.execv({real!r}, [{real!r}, *sys.argv[1:]])
''')
    wrapper.chmod(0o700)
    # A narrow native viewport deliberately exercises the documented hybrid
    # reattachment fallback; this wrapper cannot intercept an in-client switch.
    terminal = Terminal([BINARY, 'launch'], dict(lab.env, HERDR_BIN_PATH=str(wrapper)), lab.root, width=60, height=20)
    lab.clients.append(terminal)
    terminal.wait('LOCAL_ONLY')
    before = lab.snapshot('default', remote=False)['panes'][0]['terminal_id']
    select_machine(terminal, 'Alpha')
    wait_wire(terminal, b'restoring previous machine')
    terminal.wait('Machine: Alpha')
    terminal.send(b'\x1b')
    terminal.wait('LOCAL_ONLY')
    assert_routed(lab, terminal, 'CANDIDATE_FAILURE_RESTORED_LOCAL', 'default', remote=False)
    assert marker.read_text() == 'failed once'
    assert lab.snapshot('default', remote=False)['panes'][0]['terminal_id'] == before
    assert 'CANDIDATE_FAILURE_RESTORED_LOCAL' not in lab.pane_text('alpha')


def test_enter_and_immediate_command_preserve_every_byte_for_selected_machine(lab):
    terminal = open_client(lab)
    terminal.wait('LOCAL_ONLY')
    terminal.send(b'\x1d')
    terminal.wait('Search workspaces')
    browse_machine(terminal, 'Alpha')
    terminal.send(b"\rprintf 'COALESCED_FIRST_BYTE_SURVIVED\\n'\r")
    terminal.wait('COALESCED_FIRST_BYTE_SURVIVED', timeout=50)
    assert 'COALESCED_FIRST_BYTE_SURVIVED' in wait_pane_line(lab, 'COALESCED_FIRST_BYTE_SURVIVED', 'alpha')
    assert 'COALESCED_FIRST_BYTE_SURVIVED' not in lab.pane_text('default', remote=False)


def test_direct_remote_startup_preserves_input_typed_before_first_frame(lab):
    identifier = profiles(lab)[0]['id']
    terminal = open_client(lab, '--machine', identifier)
    terminal.send(b"printf 'IMMEDIATE_REMOTE_FIRST_BYTE_SURVIVED\\n'\r")
    terminal.wait('IMMEDIATE_REMOTE_FIRST_BYTE_SURVIVED')
    assert 'IMMEDIATE_REMOTE_FIRST_BYTE_SURVIVED' in wait_pane_line(lab, 'IMMEDIATE_REMOTE_FIRST_BYTE_SURVIVED', 'alpha')
    assert 'IMMEDIATE_REMOTE_FIRST_BYTE_SURVIVED' not in lab.pane_text('default', remote=False)
