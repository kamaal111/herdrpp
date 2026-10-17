"""Real native inspection cancellation, resize fallback and viewer cleanup."""
import fcntl
import json
import os
from pathlib import Path
import signal
import struct
import termios
import time

import pytest

from native_process import blocked_viewer_output, stop_viewer, viewer_pid
from picker_driver import browse_machine
from navigator_gate import assert_native_only, native_lab, open_native, routed_pick


def user_state(lab):
    catalog = Path(lab.env['XDG_STATE_HOME'], 'herdr/client/endpoints.json')
    config = Path(lab.env['HERDR_CONFIG_PATH'])
    return catalog.read_bytes(), config.read_bytes()


def viewer_catalog(pid):
    environment = Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
    state_home = next(value.split(b'=', 1)[1] for value in environment
                      if value.startswith(b'XDG_STATE_HOME='))
    return Path(os.fsdecode(state_home), 'herdr/client/endpoints.json')


def open_picker(term):
    term.send(b'\x1d')
    term.wait('Search workspaces')
    browse_machine(term, 'Beta')


def wait_alias_inspection(term, catalog):
    deadline = time.monotonic() + 20
    while True:
        assert term.process.poll() is None, term.text
        contents = json.loads(catalog.read_text())
        profiles = contents['ssh']
        if profiles and all(profile['label'].startswith('pp') for profile in profiles):
            return contents
        assert time.monotonic() < deadline, term.text
        term.read(.01)


def wait_server_line(lab, term, session, expected):
    deadline = time.monotonic() + 20
    text = lab.pane_text(session)
    while expected not in text.splitlines():
        assert term.process.poll() is None, term.text
        assert time.monotonic() < deadline, text
        term.read()
        text = lab.pane_text(session)
    return text


def wait_diagnostic(term, expected, offset, timeout=3):
    started = time.monotonic()
    deadline = started + timeout
    while expected not in term.raw[offset:]:
        assert term.process.poll() is None, term.text
        assert time.monotonic() < deadline, term.raw[offset:]
        term.read(.01)
    return time.monotonic() - started


def interruption_evidence(name, state, elapsed):
    output = Path('/output/native-interruptions')
    output.mkdir(parents=True, exist_ok=True)
    (output / f'{name}.json').write_text(json.dumps(dict(
        viewer_state_before_interrupt=state,
        native_open_output_fenced=True,
        interruption_seconds=elapsed), indent=2))


def resize(term, columns, lines):
    term.width, term.height = columns, lines
    term.screen.resize(lines, columns)
    fcntl.ioctl(term.master, termios.TIOCSWINSZ, struct.pack('HHHH', lines, columns, 0, 0))
    os.kill(term.process.pid, signal.SIGWINCH)


@pytest.mark.parametrize('quit_key', [b'\x1b[92;5u', b'\x1b[92:124:92;5:1u'],
                         ids=['enhanced', 'alternate-layout'])
def test_enhanced_quit_during_alias_inspection_restores_catalog_and_stops_only_viewer(native_lab, quit_key):
    lab = native_lab.start()
    original = user_state(lab)
    term = open_native(lab)
    pid = routed_pick(lab, term, 'Alpha', 'alpha', 'QUIT_NATIVE_READY')
    assert_native_only(term)
    catalog = viewer_catalog(pid)
    open_picker(term)
    with blocked_viewer_output(pid):
        term.send(b'\r')
        aliases = wait_alias_inspection(term, catalog)
        assert all(profile['label'].startswith('pp') for profile in aliases['ssh'])
        state = stop_viewer(term, pid)
        started = time.monotonic()
        term.send(quit_key)
        assert term.wait_exit(timeout=3) == 0
        elapsed = time.monotonic() - started
        assert elapsed < 3
    interruption_evidence(f'quit-{quit_key.hex()}', state, elapsed)
    assert user_state(lab) == original
    assert not Path(f'/proc/{pid}').exists()
    assert lab.remote_endpoint_running('alpha')
    assert lab.remote_endpoint_running('beta')
    assert b'using reattachment' not in term.raw


def test_resize_during_alias_inspection_falls_back_promptly_and_routes_to_selected_machine(native_lab):
    lab = native_lab.start()
    original = user_state(lab)
    term = open_native(lab)
    pid = routed_pick(lab, term, 'Alpha', 'alpha', 'RESIZE_NATIVE_READY')
    assert_native_only(term)
    catalog = viewer_catalog(pid)
    open_picker(term)
    with blocked_viewer_output(pid):
        term.send(b'\r')
        aliases = wait_alias_inspection(term, catalog)
        assert all(profile['label'].startswith('pp') for profile in aliases['ssh'])
        state = stop_viewer(term, pid)
        offset = len(term.raw)
        resize(term, 66, 24)
        term.send(b"printf 'RESIZE_FALLBACK_OK\\n'\r")
        elapsed = wait_diagnostic(term, b'terminal resized during native inspection', offset)
        assert elapsed < 3
    interruption_evidence('resize', state, elapsed)
    text = wait_server_line(lab, term, 'beta', 'RESIZE_FALLBACK_OK')
    assert 'RESIZE_FALLBACK_OK' in text.splitlines()
    assert b'terminal resized during native inspection' in term.raw
    assert b'using reattachment' in term.raw
    assert viewer_pid(term.process.pid) != pid
    assert user_state(lab) == original
    assert lab.remote_endpoint_running('alpha')
    assert lab.remote_endpoint_running('beta')


def test_owned_native_viewer_sigterm_exits_cleanly_preserving_user_state_and_remote_servers(native_lab):
    lab = native_lab.start()
    original = user_state(lab)
    term = open_native(lab)
    pid = routed_pick(lab, term, 'Alpha', 'alpha', 'VIEWER_TERM_READY')
    assert_native_only(term)
    os.kill(pid, signal.SIGTERM)
    assert term.wait_exit(timeout=15) == 0
    assert user_state(lab) == original
    assert not Path(f'/proc/{pid}').exists()
    assert lab.remote_endpoint_running('alpha')
    assert lab.remote_endpoint_running('beta')
    assert 'VIEWER_TERM_READY' in lab.pane_text('alpha').splitlines()
