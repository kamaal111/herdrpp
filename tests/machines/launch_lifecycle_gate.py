"""Cold-start ownership and persistence through the real production launcher."""
import json
import os
from pathlib import Path
import tempfile
import time

import pytest

from lab import Lab
from terminal import Terminal

BINARY = '/output/build/release/herdrpp'


@pytest.fixture
def cold_lab():
    with tempfile.TemporaryDirectory(prefix='launch-lifecycle-') as directory:
        instance = Lab(Path(directory) / 'lab')
        try:
            yield instance
        finally:
            instance.close()
            instance.run('herdr', 'server', 'stop', check=False)


def launch(lab):
    terminal = Terminal([BINARY, 'launch'], lab.env, lab.root)
    lab.clients.append(terminal)
    return terminal


def wait_server(lab, terminal):
    deadline = time.monotonic() + 20
    result = lab.run('herdr', 'api', 'snapshot', check=False)
    while result.returncode != 0:
        assert terminal.process.poll() is None, terminal.text
        assert time.monotonic() < deadline, result.stderr
        terminal.read()
        result = lab.run('herdr', 'api', 'snapshot', check=False)
    return json.loads(result.stdout)['result']['snapshot']


def workspace_pane(snapshot, workspace):
    return next(pane for pane in snapshot['panes'] if pane['workspace_id'] == workspace)


def wait_pane_line(lab, terminal, pane, expected):
    deadline = time.monotonic() + 20
    result = lab.run('herdr', 'pane', 'read', pane, '--source', 'recent', '--lines', '100')
    while expected not in result.stdout.splitlines():
        assert terminal.process.poll() is None, terminal.text
        assert time.monotonic() < deadline, result.stdout
        terminal.read()
        result = lab.run('herdr', 'pane', 'read', pane, '--source', 'recent', '--lines', '100')
    return result.stdout


def wait_fixture_file(terminal, path):
    deadline = time.monotonic() + 10
    while not path.exists():
        assert terminal.process.poll() is None, terminal.text
        assert time.monotonic() < deadline, f'busy fixture did not create {path.name}'
        terminal.read()
    return path.read_text()


def busy_fixture(root):
    script = root / 'busy-lifecycle.sh'
    pid = root / 'busy.pid'
    trigger = root / 'busy-progress-trigger'
    progressed = root / 'busy-progressed'
    finish = root / 'busy-finish'
    script.write_text(
        "set -e\n"
        f"printf '%s\\n' $$ > {pid}.tmp\nmv {pid}.tmp {pid}\n"
        f"while [ ! -e {trigger} ]; do sleep .02; done\n"
        f"printf 'PICKER_BACKGROUND_PROGRESS\\n' > {progressed}.tmp\n"
        f"mv {progressed}.tmp {progressed}\n"
        f"while [ ! -e {finish} ]; do sleep .02; done\n"
    )
    return script, pid, trigger, progressed, finish


def close_launcher(lab, terminal):
    terminal.send(b'\x1c')
    status = terminal.wait_exit()
    terminal.close()
    lab.clients.remove(terminal)
    return status


def test_cold_launcher_bootstraps_persistent_server_and_reuses_live_workspace(cold_lab):
    missing = cold_lab.run('herdr', 'api', 'snapshot', check=False)
    assert missing.returncode != 0
    first = launch(cold_lab)
    initial = wait_server(cold_lab, first)
    created = cold_lab.api('workspace', 'create', '--label', 'COLD_START', '--focus')
    workspace = created['workspace']['workspace_id']
    first.wait('COLD_START')
    pane = workspace_pane(cold_lab.snapshot('default', remote=False), workspace)
    first.send(b"LIFECYCLE_TOKEN=COLD_SHELL_STATE; printf 'LIFECYCLE_FIRST_OK\\n'\r")
    first_text = wait_pane_line(cold_lab, first, pane['pane_id'], 'LIFECYCLE_FIRST_OK')
    assert 'LIFECYCLE_FIRST_OK' in first_text.splitlines()
    busy_script, pidfile, trigger, progressed, finish = busy_fixture(cold_lab.root)
    first.send(f"/bin/sh {busy_script}; printf 'BUSY_FINISHED\\n'\r".encode())
    busy_pid = int(wait_fixture_file(first, pidfile))
    first.send(b'\x1d')
    first.wait('Search workspaces')
    trigger.touch()
    assert wait_fixture_file(first, progressed) == 'PICKER_BACKGROUND_PROGRESS\n'
    assert os.kill(busy_pid, 0) is None
    assert 'Search workspaces' in first.text
    first.send(b'\x1b')
    first.wait('COLD_START')
    assert close_launcher(cold_lab, first) == 0
    detached = cold_lab.snapshot('default', remote=False)
    detached_pane = workspace_pane(detached, workspace)
    assert detached_pane['pane_id'] == pane['pane_id']
    assert detached_pane['terminal_id'] == pane['terminal_id']
    assert os.kill(busy_pid, 0) is None
    second = launch(cold_lab)
    second.wait('COLD_START')
    resumed = wait_server(cold_lab, second)
    resumed_pane = workspace_pane(resumed, workspace)
    assert resumed_pane['pane_id'] == pane['pane_id']
    assert resumed_pane['terminal_id'] == pane['terminal_id']
    assert os.kill(busy_pid, 0) is None
    finish.touch()
    busy_finished = wait_pane_line(cold_lab, second, pane['pane_id'], 'BUSY_FINISHED')
    assert 'BUSY_FINISHED' in busy_finished.splitlines()
    second.send(b"printf 'LIFECYCLE_SECOND_OK\\n%s\\n' \"$LIFECYCLE_TOKEN\"\r")
    second_text = wait_pane_line(cold_lab, second, pane['pane_id'], 'LIFECYCLE_SECOND_OK')
    token_text = wait_pane_line(cold_lab, second, pane['pane_id'], 'COLD_SHELL_STATE')
    assert 'LIFECYCLE_SECOND_OK' in second_text.splitlines()
    assert 'COLD_SHELL_STATE' in token_text.splitlines()
    assert close_launcher(cold_lab, second) == 0
    final_pane = workspace_pane(cold_lab.snapshot('default', remote=False), workspace)
    assert final_pane['terminal_id'] == pane['terminal_id']
    output = Path('/output/launch-lifecycle')
    output.mkdir(parents=True, exist_ok=True)
    (output / 'identity.json').write_text(json.dumps(dict(
        server_started_without_prestarted_fixture=True, initial_workspaces=len(initial['workspaces']),
        workspace_id=workspace, pane_id=pane['pane_id'], terminal_id=pane['terminal_id'],
        first_marker='LIFECYCLE_FIRST_OK', second_marker='LIFECYCLE_SECOND_OK',
        live_shell_token='COLD_SHELL_STATE', busy_pid=busy_pid,
        progress_while_picker_open='PICKER_BACKGROUND_PROGRESS'), indent=2))
    (output / 'first.ansi').write_bytes(first.raw)
    (output / 'second.ansi').write_bytes(second.raw)
