"""Acceptance tests for machine cycling inside the real workspace picker."""
import json
import time
from pathlib import Path

from ssh_gate import lab
from ssh_failures_gate import open_client, assert_routed, wait_wire, wait_pane_line, capture_terminal_evidence
from picker_driver import open_picker, browse_machine


def browse(terminal, label, workspace):
    for _ in range(3):
        if f'Machine: {label}' in terminal.text:
            terminal.wait(f'Machine: {label}', workspace)
            return
        terminal.send(b'\x1b[6~')
        deadline = time.monotonic() + 5
        previous = terminal.text
        while terminal.text == previous:
            assert time.monotonic() < deadline, terminal.text
            terminal.read()
    raise AssertionError(terminal.text)


def test_browse_remote_workspaces_and_cancel_keeps_local_attachment_and_focus(lab):
    terminal = open_client(lab)
    terminal.wait('LOCAL_ONLY')
    before = lab.snapshot('default', remote=False)
    terminal.send(b'\x1d')
    terminal.wait('Machine: Local', 'Search workspaces')
    browse(terminal, 'Alpha', 'ALPHA_ONLY')
    browse(terminal, 'Beta', 'BETA_ONLY')
    terminal.send(b'\x1b')
    terminal.wait('LOCAL_ONLY')
    assert_routed(lab, terminal, 'BROWSE_CANCEL_STILL_LOCAL', 'default', remote=False)
    assert lab.snapshot('default', remote=False)['focused_tab_id'] == before['focused_tab_id']
    assert 'BROWSE_CANCEL_STILL_LOCAL' not in lab.pane_text('alpha')
    assert 'BROWSE_CANCEL_STILL_LOCAL' not in lab.pane_text('beta')


def test_existing_cmd_p_opens_workspace_picker_and_selection_commits_machine_and_tab(lab):
    config = Path(lab.env['HERDR_CONFIG_PATH'])
    config.write_text(config.read_text() + '\n[[keys.command]]\nkey="cmd+p"\ntype="plugin_action"\ncommand="herdrpp.open"\n')
    alpha = lab.snapshot('alpha')
    workspace = alpha['workspaces'][0]['workspace_id']
    lab.api('--session', 'alpha', 'tab', 'create', '--workspace', workspace, '--label', 'Logs', env=lab.remote)
    beta = lab.snapshot('beta')
    lab.api('--session', 'beta', 'tab', 'create', '--workspace', beta['workspaces'][0]['workspace_id'], '--label', 'Logs', env=lab.remote)
    terminal = open_client(lab)
    terminal.wait('LOCAL_ONLY')
    terminal.send(b'\x1b[112;9u')
    terminal.wait('Machine: Local', 'Search workspaces')
    browse(terminal, 'Alpha', 'ALPHA_ONLY')
    terminal.send(b'Logs\r')
    terminal.wait('ALPHA_ONLY')
    terminal.send(b"printf 'PICKER_EXACT_ALPHA_LOGS\\n'\r")
    terminal.wait('PICKER_EXACT_ALPHA_LOGS')
    after = lab.snapshot('alpha')
    logs = next(tab for tab in after['tabs'] if tab['label'] == 'Logs')
    assert after['focused_tab_id'] == logs['tab_id']
    pane = next(pane for pane in after['panes'] if pane['tab_id'] == logs['tab_id'])
    assert 'PICKER_EXACT_ALPHA_LOGS' in wait_pane_line(lab, 'PICKER_EXACT_ALPHA_LOGS', 'alpha', pane_id=pane['pane_id'])
    assert 'PICKER_EXACT_ALPHA_LOGS' not in lab.pane_text('beta')
    assert terminal.process.poll() is None


def test_changed_profile_cannot_redirect_a_browsed_selection_to_another_machine(lab):
    terminal = open_client(lab)
    terminal.wait('LOCAL_ONLY')
    terminal.send(b'\x1d')
    terminal.wait('Machine: Local', 'Search workspaces')
    browse(terminal, 'Alpha', 'ALPHA_ONLY')
    catalog = Path(lab.env['XDG_STATE_HOME']) / 'herdr/client/endpoints.json'
    value = json.loads(catalog.read_text())
    value['ssh'][0]['target'] = value['ssh'][1]['target']
    value['ssh'][0]['session'] = value['ssh'][1]['session']
    catalog.write_text(json.dumps(value))
    terminal.send(b'\r')
    wait_wire(terminal, b'selected machine was removed, disabled or changed')
    terminal.wait('Machine: Alpha')
    terminal.send(b'\x1b')
    terminal.wait('LOCAL_ONLY')
    assert_routed(lab, terminal, 'CHANGED_PROFILE_STILL_LOCAL', 'default', remote=False)
    assert 'CHANGED_PROFILE_STILL_LOCAL' not in lab.pane_text('beta')
    assert terminal.process.poll() is None


def test_closed_workspace_restores_original_machine_and_reopens_picker_with_error(lab):
    terminal = open_client(lab)
    terminal.wait('LOCAL_ONLY')
    original = lab.snapshot('default', remote=False)['focused_tab_id']
    open_picker(terminal)
    browse_machine(terminal, 'Alpha')
    workspace = lab.snapshot('alpha')['workspaces'][0]['workspace_id']
    lab.run('herdr', '--session', 'alpha', 'workspace', 'close', workspace, env=lab.remote)
    terminal.send(b'\r')
    wait_wire(terminal, b'selected destination no longer exists')
    terminal.wait('Machine: Alpha')
    terminal.send(b'\x1b')
    terminal.wait('LOCAL_ONLY')
    assert_routed(lab, terminal, 'CLOSED_TARGET_RESTORED_LOCAL', 'default', remote=False)
    assert lab.snapshot('default', remote=False)['focused_tab_id'] == original
    assert terminal.process.poll() is None
    open_picker(terminal)
    terminal.wait('Machine: Local')
    assert 'selected destination no longer exists' not in terminal.text
    from picker_driver import close_picker
    close_picker(terminal)
