"""Public CLI reattachment prerequisites against real Herdr and OpenSSH."""
import json
import os
from pathlib import Path
import signal
import tempfile

import pytest

from lab import Lab
from terminal import Terminal


@pytest.fixture
def lab():
    with tempfile.TemporaryDirectory(prefix='reattach-gate-') as directory:
        instance = Lab(Path(directory) / 'lab')
        try:
            yield instance.start()
        finally:
            instance.close()


def attach(lab, session):
    term = Terminal(['herdr', '--remote', lab.targets[session], '--session', session], lab.env, lab.root)
    lab.clients.append(term)
    term.wait('ALPHA_ONLY')
    return term


def test_reattachment_preserves_busy_process_without_shell_prompt(lab):
    term = attach(lab, 'alpha')
    before = lab.snapshot('alpha')['panes'][0]['terminal_id']
    pidfile = lab.root / 'busy.pid'
    term.send(f"sh -c 'echo $$ > {pidfile}; printf BUSY_RUNNING; exec sleep 300'\r".encode())
    term.wait('BUSY_RUNNING')
    pid = int(pidfile.read_text())
    term.close()
    lab.clients.remove(term)
    returned = attach(lab, 'alpha')
    returned.wait('BUSY_RUNNING')
    assert os.kill(pid, 0) is None
    assert lab.snapshot('alpha')['panes'][0]['terminal_id'] == before
    assert 'ALPHA_ONLY' in returned.text
    os.kill(pid, signal.SIGTERM)


def test_public_machine_status_checks_remote_without_switching_local_client(lab):
    term = Terminal(['herdr', '--session', 'default'], lab.env, lab.root)
    lab.clients.append(term)
    term.wait('LOCAL_ONLY')
    machines = json.loads(lab.run('herdr', 'machine', 'list', '--json').stdout)
    status = lab.run('herdr', 'machine', 'status', machines[0]['id'], '--json')
    records = json.loads(status.stdout)
    assert records[0]['status'] == 'reachable'
    term.send(b"printf 'PREFLIGHT_STILL_LOCAL\\n'\r")
    term.wait('PREFLIGHT_STILL_LOCAL')
    assert 'PREFLIGHT_STILL_LOCAL' in lab.pane_text('default', remote=False).splitlines()
    assert 'PREFLIGHT_STILL_LOCAL' not in lab.pane_text('alpha')


def pick(term, label):
    from picker_driver import select_machine
    select_machine(term, label)
    term.wait({'Alpha': 'ALPHA_ONLY', 'Beta': 'BETA_ONLY', 'Local': 'LOCAL_ONLY'}[label])


def test_production_rust_picker_round_trip_keeps_busy_process_and_local_routing(lab):
    profiles = json.loads(lab.run('herdr', 'machine', 'list', '--json').stdout)
    assert profiles[0]['target'] != profiles[1]['target']
    assert {profile['target'] for profile in profiles} == set(lab.targets.values())
    assert lab.remote_endpoint_running('alpha')
    assert lab.remote_endpoint_running('beta')
    term = Terminal(['/output/build/release/herdrpp', 'launch'], lab.env, lab.root)
    lab.clients.append(term)
    term.wait('LOCAL_ONLY')
    pick(term, 'Alpha')
    alpha = lab.snapshot('alpha')['panes'][0]['terminal_id']
    pidfile = lab.root / 'production-busy.pid'
    term.send(f"sh -c 'echo $$ > {pidfile}; printf PRODUCTION_BUSY; exec sleep 300'\r".encode())
    term.wait('PRODUCTION_BUSY')
    pid = int(pidfile.read_text())
    pick(term, 'Beta')
    term.send(b"printf 'PRODUCTION_BETA\\n'\r")
    term.wait('PRODUCTION_BETA')
    pick(term, 'Local')
    term.send(b"printf 'PRODUCTION_LOCAL\\n'\r")
    term.wait('PRODUCTION_LOCAL')
    pick(term, 'Alpha')
    term.wait('PRODUCTION_BUSY')
    assert os.kill(pid, 0) is None
    assert lab.snapshot('alpha')['panes'][0]['terminal_id'] == alpha
    assert 'PRODUCTION_BETA' in lab.pane_text('beta').splitlines()
    assert 'PRODUCTION_LOCAL' in lab.pane_text('default', remote=False).splitlines()
    assert 'PRODUCTION_LOCAL' not in lab.pane_text('beta')
    assert lab.remote_endpoint_running('alpha')
    assert lab.remote_endpoint_running('beta')
    term.send(b'\x1d')
    term.wait('Search workspaces')
    term.send(b'\x1b')
    term.wait('PRODUCTION_BUSY')
    assert os.kill(pid, 0) is None
    term.send(b'\x1c')
    assert term.wait_exit() == 0
    assert __import__('termios').tcgetattr(term.master) == term.initial_termios
    os.kill(pid, signal.SIGTERM)
