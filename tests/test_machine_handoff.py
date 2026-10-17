"""Own the host PTY and coalesce selector closing keys with shell typeahead."""
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import termios
import time

import pytest

from pty_smoke import read_until, stable_terminal_attributes, write_all

from machine_fake import API_ROUTES, wait_workspace_picker

FAKE = API_ROUTES + r'''import json, os, signal, sys, tty, time
from pathlib import Path
args = sys.argv[1:]
if args == ['machine', 'list', '--json']:
    print(json.dumps([dict(id='alpha', label='Alpha', target='alice@build', session='engineering', enabled=True)]))
    sys.exit(0)
if args[:2] == ['machine', 'status']:
    if Path(os.environ['HANDOFF_FAILURE_FILE']).read_text():
        Path(os.environ['HANDOFF_FAILURE_FILE']).with_name('status-started').touch()
        time.sleep(60 if Path(os.environ['HANDOFF_FAILURE_FILE']).read_text() == 'checking' else 2)
    if Path(os.environ['HANDOFF_FAILURE_FILE']).read_text() == 'preflight':
        print(json.dumps([dict(id='alpha', status='error', error='EXPECTED_STATUS_FAILURE')]))
        sys.exit(1)
    print(json.dumps([dict(id='alpha', status='reachable', error=None)]))
    sys.exit(0)
name = 'remote' if '--remote' in args else 'local'
if name == 'remote' and Path(os.environ['HANDOFF_FAILURE_FILE']).read_text() == 'candidate':
    sys.exit(17)
if name == 'remote' and Path(os.environ['HANDOFF_FAILURE_FILE']).read_text() == 'startup':
    time.sleep(60)
tty.setraw(0)
signal.signal(signal.SIGWINCH, lambda *_: os.write(1, ('REDRAW_' + name + '\r\n').encode()))
os.write(1, ('CLIENT_READY_' + name + '\r\n').encode() + b'\x1b[?7l\x1b[?2026h\x1b[?2026l')
received = b''
while True:
    data = os.read(0, 65536)
    if not data:
        break
    received += data
    with open(os.path.join(os.environ['HANDOFF_INPUT_DIRECTORY'], name), 'wb') as output:
        output.write(received)
    os.write(1, ('RECEIVED_' + name + '\r\n').encode())
'''


def wait_exit(process, master, timeout=2):
    deadline = time.monotonic() + timeout
    while process.poll() is None:
        assert time.monotonic() < deadline, 'launcher did not exit'
        if select.select([master], [], [], .05)[0]:
            os.read(master, 65536)


@pytest.fixture
def handoff_client(tmp_path, release_binary):
    executable = tmp_path / 'herdr'
    executable.write_text(f'#!{sys.executable}\n' + FAKE)
    executable.chmod(0o700)
    logs = tmp_path / 'logs'
    logs.mkdir()
    failure = tmp_path / 'failure'
    failure.write_text('')
    config = tmp_path / 'config.toml'
    config.write_text('[theme]\nname="dracula"\n')
    environment = dict(os.environ, HERDR_BIN_PATH=str(executable), HERDR_CONFIG_PATH=str(config),
                       HANDOFF_INPUT_DIRECTORY=str(logs), HANDOFF_FAILURE_FILE=str(failure), TERM='xterm-256color',
                       XDG_STATE_HOME=str(tmp_path / 'state'))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
    original = stable_terminal_attributes(slave)
    process = subprocess.Popen([release_binary, 'launch'], env=environment, stdin=slave,
                               stdout=slave, stderr=slave, start_new_session=True)
    try:
        read_until(master, b'CLIENT_READY_local')
        os.write(master, b'\x1d')
        os.write(master, b'\x1b[6~')
        wait_workspace_picker(master, 'Alpha')
        yield process, master, logs
    finally:
        try:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                wait_exit(process, master, timeout=5)
            assert stable_terminal_attributes(slave) == original
        finally:
            os.close(master)
            os.close(slave)


def wait_input(master, path, expected):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if path.exists() and path.read_bytes() == expected:
            return path.read_bytes()
        if select.select([master], [], [], .05)[0]:
            os.read(master, 65536)
    assert path.exists(), 'attachment never received the queued input'
    return path.read_bytes()


@pytest.mark.parametrize('enter', [b'\r', b'\x1b[13;1u'], ids=['legacy', 'enhanced'])
def test_coalesced_selection_enter_and_command_reaches_only_new_attachment(handoff_client, enter):
    process, master, logs = handoff_client
    command = b'TYPEAHEAD_AFTER_SELECTION\r'
    os.write(master, b'Alpha' + enter + command)
    assert wait_input(master, logs / 'remote', command) == command
    assert not (logs / 'local').exists()
    assert process.poll() is None


def test_no_hit_enter_keeps_editing_then_delivers_typeahead_after_a_valid_selection(handoff_client):
    process, master, logs = handoff_client
    command = b'AFTER_NO_HIT_RECOVERY\r'
    os.write(master, b'Missing\r' + b'\x7f' * 7 + b'Alpha\r' + command)
    assert wait_input(master, logs / 'remote', command) == command
    assert not (logs / 'local').exists()
    assert process.poll() is None


def test_same_machine_selection_delivers_coalesced_typeahead_to_original_attachment(handoff_client):
    process, master, logs = handoff_client
    command = b'SAME_MACHINE_TYPEAHEAD\r'
    os.write(master, b'\x1b[5~')
    wait_workspace_picker(master, 'Local', reset=False)
    os.write(master, b'Local\r' + command)
    assert wait_input(master, logs / 'local', command) == command
    assert not (logs / 'remote').exists()
    assert process.poll() is None


def test_enhanced_escape_cancel_preserves_coalesced_typeahead_for_original_attachment(handoff_client):
    process, master, logs = handoff_client
    command = b'CANCELLED_PICKER_TYPEAHEAD\r'
    os.write(master, b'\x1b[27u' + command)
    assert wait_input(master, logs / 'local', command) == command
    assert not (logs / 'remote').exists()
    assert process.poll() is None


def test_pasted_newlines_edit_search_without_confirming_and_later_enter_hands_off_typeahead(handoff_client):
    process, master, logs = handoff_client
    command = b'AFTER_PASTED_SEARCH\r'
    os.write(master, b'\x1b[200~Alpha\r\n\x1b[201~\r' + command)
    assert wait_input(master, logs / 'remote', command) == command
    assert not (logs / 'local').exists()
    assert process.poll() is None


def wait_failure_recovery(master, marker, ready, received):
    deadline = time.monotonic() + 40
    while marker not in received or b'Machine: Alpha' not in received:
        assert time.monotonic() < deadline, received[-1000:]
        if select.select([master], [], [], .05)[0]:
            received += os.read(master, 65536)


@pytest.mark.parametrize('failure,marker,ready', [
    ('candidate', b'restoring previous machine', b'CLIENT_READY_local'),
    ('preflight', b'EXPECTED_STATUS_FAILURE', b'REDRAW_local'),
    ('startup', b'terminal setup did not complete within 30 seconds', b'CLIENT_READY_local'),
])
def test_failed_switch_discards_coalesced_typeahead_instead_of_replaying_it_to_local(handoff_client, failure, marker, ready):
    process, master, logs = handoff_client
    (logs.parent / 'failure').write_text(failure)
    command = b'ONLY_FOR_FAILED_REMOTE_' + b'x' * 20_000 + b'\r'
    received = write_all(master, b'Alpha\r' + command, timeout=10)
    wait_failure_recovery(master, marker, ready, received)
    os.write(master, b'\x1b')
    read_until(master, ready)
    probe = b'SAFE_LOCAL_PROBE\r'
    os.write(master, probe)
    assert wait_input(master, logs / 'local', probe) == probe
    assert process.poll() is None


QUIT_KEYS = [b'\x1c', b'\x1b[92;5u', b'\x1b[1098::92;69u']


@pytest.mark.parametrize('key', QUIT_KEYS, ids=['legacy', 'enhanced', 'alternate-layout-caps-lock'])
def test_quit_from_open_picker_exits_launcher_and_restores_host_terminal(handoff_client, key):
    process, master, logs = handoff_client
    os.write(master, key)
    wait_exit(process, master)
    assert process.returncode == 0


def wait_status_started(master, path):
    deadline = time.monotonic() + 5
    while not path.exists():
        assert time.monotonic() < deadline, 'machine status did not start'
        if select.select([master], [], [], .05)[0]:
            os.read(master, 65536)


@pytest.mark.parametrize('key', QUIT_KEYS, ids=['legacy', 'enhanced', 'alternate-layout-caps-lock'])
def test_quit_during_preflight_cancels_probe_and_restores_host_terminal(handoff_client, key):
    process, master, logs = handoff_client
    (logs.parent / 'failure').write_text('checking')
    os.write(master, b'Alpha\r')
    wait_status_started(master, logs.parent / 'status-started')
    os.write(master, key)
    wait_exit(process, master)
    assert process.returncode == 0


def test_quit_keys_inside_paste_spanning_picker_and_preflight_reach_only_selected_attachment(handoff_client):
    process, master, logs = handoff_client
    (logs.parent / 'failure').write_text('paste')
    os.write(master, b'Alpha\r\x1b[200~')
    wait_status_started(master, logs.parent / 'status-started')
    body = b'\x1c\x1b[92;5u\x1b[1098::92;69uPASTED_CONTENT'
    os.write(master, body + b'\x1b[201~')
    expected = b'\x1b[200~' + body + b'\x1b[201~'
    assert wait_input(master, logs / 'remote', expected) == expected
    assert not (logs / 'local').exists()
    assert process.poll() is None
