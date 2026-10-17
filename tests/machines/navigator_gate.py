"""Production native path must commit routes without replacing its viewing client."""
from pathlib import Path
import re
import os
import signal
from contextlib import contextmanager
import tempfile
import time

import pytest

from lab import Lab
from terminal import Terminal
from native_process import viewer_pid
from picker_driver import browse_machine, open_picker

BINARY = '/output/build/release/herdrpp'


@pytest.fixture
def native_lab(request):
    with tempfile.TemporaryDirectory(prefix='navigator-product-') as directory:
        lab = Lab(Path(directory) / 'lab')
        try:
            yield lab
        finally:
            artifacts = Path('/output/native-diagnostics') / request.node.name
            artifacts.mkdir(parents=True, exist_ok=True)
            for index, terminal in enumerate(lab.clients):
                (artifacts / f'client-{index}.ansi').write_bytes(terminal.raw)
                (artifacts / f'client-{index}.txt').write_text(terminal.text)
            lab.close()


def viewer(term):
    return viewer_pid(term.process.pid)

def open_native(lab):
    terminal = Terminal([BINARY, 'launch'], lab.env, lab.root)
    lab.clients.append(terminal)
    terminal.send(b"printf 'BOOTSTRAP_VIEW_READY\\n'\r")
    terminal.wait('BOOTSTRAP_VIEW_READY')
    wait_route(lab, terminal, 'default', 'BOOTSTRAP_VIEW_READY')
    return terminal


def pane_lines(lab, session):
    remote = session != 'default'
    panes = lab.snapshot(session, remote=remote)['panes']
    if not panes:
        return []
    return lab.pane_text(session, remote=remote).splitlines()


def routed_pick(lab, term, label, session, token):
    open_picker(term)
    browse_machine(term, label)
    term.send(b'\r' + f"printf '{token}\\n'\r".encode())
    deadline = time.monotonic() + 50
    while time.monotonic() < deadline:
        term.read()
        if token in pane_lines(lab, session):
            return viewer(term)
    raise AssertionError((token, term.text))


def assert_native_only(term):
    fallbacks = re.findall(rb'herdrpp:[^\r\n]*using reattachment', term.raw)
    assert fallbacks == []


def test_native_controller_keeps_viewer_and_routes_typeahead(native_lab):
    lab = native_lab.start()
    catalog = Path(lab.env['XDG_STATE_HOME'], 'herdr/client/endpoints.json')
    original = catalog.read_bytes()
    term = open_native(lab)
    bootstrap = viewer(term)
    native = routed_pick(lab, term, 'Alpha', 'alpha', 'NATIVE_ALPHA_PROOF')
    assert_native_only(term)
    assert native != bootstrap
    assert routed_pick(lab, term, 'Beta', 'beta', 'NATIVE_BETA_PROOF') == native
    assert routed_pick(lab, term, 'Local', 'default', 'NATIVE_LOCAL_PROOF') == native
    assert_native_only(term)
    assert catalog.read_bytes() == original


def hidden_configuration(lab):
    config = Path(lab.env['HERDR_CONFIG_PATH'])
    config.write_text(config.read_text().replace('[ui]\n', '[ui]\nsidebar_start_collapsed = true\nsidebar_collapsed_mode = "hidden"\nmouse_capture = false\n') + '\n[keys]\nprefix = "ctrl+a"\ngoto = []\n')
    return config


def wait_file(term, path):
    deadline = time.monotonic() + 15
    while not path.exists():
        assert time.monotonic() < deadline, term.text
        term.read()


def wait_absent(term, text):
    deadline = time.monotonic() + 20
    while text in term.text:
        assert time.monotonic() < deadline, term.text
        term.read()


def apply_theme(term):
    term.send(b'\x02s')
    term.wait('settings', 'theme')
    term.send(b'\x1b[B\r')
    wait_absent(term, 'settings')


def test_native_private_public_chord_preserves_hidden_custom_bindings(native_lab):
    lab = native_lab
    config = hidden_configuration(lab)
    local_ctrl_u = lab.root / 'local-control-u'
    remote_ctrl_u = lab.root / 'remote-control-u'
    config.write_text(config.read_text() + f'\n[[keys.command]]\nkey = "ctrl+u"\ntype = "shell"\ncommand = "echo local >> {local_ctrl_u}"\n')
    original = config.read_bytes()
    remote = Path(lab.remote['HERDR_CONFIG_PATH'])
    marker = lab.root / 'remote-control-f12'
    remote.write_text(remote.read_text() + f'\n[[keys.command]]\nkey = "ctrl+f12"\ntype = "shell"\ncommand = "echo handled >> {marker}"\n')
    remote.write_text(remote.read_text() + f'\n[[keys.command]]\nkey = "ctrl+u"\ntype = "shell"\ncommand = "echo remote >> {remote_ctrl_u}"\n')
    remote_original = remote.read_bytes()
    lab.start()
    term = open_native(lab)
    native = routed_pick(lab, term, 'Alpha', 'alpha', 'HIDDEN_NATIVE_ALPHA')
    assert_native_only(term)
    assert not local_ctrl_u.exists()
    assert not remote_ctrl_u.exists()
    term.send(b'\x15')
    wait_file(term, remote_ctrl_u)
    assert remote_ctrl_u.read_text() == 'remote\n'
    term.send(b'\x1b[24;5~')
    wait_file(term, marker)
    assert marker.read_text() == 'handled\n'
    assert routed_pick(lab, term, 'Beta', 'beta', 'HIDDEN_NATIVE_BETA') == native
    assert marker.read_text() == 'handled\n'
    assert remote_ctrl_u.read_text() == 'remote\n'
    assert not local_ctrl_u.exists()
    assert_native_only(term)
    assert config.read_bytes() == original
    assert remote.read_bytes() == remote_original


def test_native_settings_and_external_edit_reconcile_without_private_bindings(native_lab):
    import tomllib
    lab = native_lab.start()
    config = Path(lab.env['HERDR_CONFIG_PATH'])
    term = open_native(lab)
    native = routed_pick(lab, term, 'Alpha', 'alpha', 'SETTINGS_NATIVE_ALPHA')
    apply_theme(term)
    config.write_text(config.read_text().replace('sidebar_width = 30', 'sidebar_width = 33'))
    assert routed_pick(lab, term, 'Beta', 'beta', 'SETTINGS_NATIVE_BETA') == native
    saved = tomllib.loads(config.read_text())
    assert saved['theme']['name'] != 'dracula'
    assert saved['ui']['sidebar_width'] == 33
    assert 'ctrl+]' not in config.read_text()
    assert 'ctrl+backslash' not in config.read_text()
    assert_native_only(term)


def private_config(term):
    environment = Path(f'/proc/{viewer(term)}/environ').read_bytes().split(b'\0')
    return Path(os.fsdecode(next(value.split(b'=', 1)[1] for value in environment if value.startswith(b'HERDR_CONFIG_PATH='))))


@contextmanager
def paused_launcher(term):
    os.kill(term.process.pid, signal.SIGSTOP)
    deadline = time.monotonic() + 5
    while '\nState:\tT' not in Path(f'/proc/{term.process.pid}/status').read_text():
        assert time.monotonic() < deadline
        time.sleep(.01)
    try:
        yield
    finally:
        os.kill(term.process.pid, signal.SIGCONT)


def test_native_settings_conflict_preserves_user_and_recovery_copies(native_lab):
    import tomllib
    lab = native_lab.start()
    config = Path(lab.env['HERDR_CONFIG_PATH'])
    term = open_native(lab)
    routed_pick(lab, term, 'Alpha', 'alpha', 'CONFLICT_NATIVE_ALPHA')
    private = private_config(term)
    with paused_launcher(term):
        private.write_text(private.read_text().replace('name = "dracula"', 'name = "nord"'))
        config.write_text(config.read_text().replace('name = "dracula"', 'name = "gruvbox"'))
        expected = config.read_bytes()
    assert term.wait_exit(timeout=20) == 1
    assert config.read_bytes() == expected
    recovery = Path(lab.env['XDG_STATE_HOME'], 'herdrpp/native-config')
    assert tomllib.loads((recovery / 'settings-current.toml').read_text())['theme']['name'] == 'gruvbox'
    assert tomllib.loads((recovery / 'settings-pending.toml').read_text())['theme']['name'] == 'nord'
    assert tomllib.loads(private.read_text())['theme']['name'] == 'nord'


def wait_sidebar(term, column):
    deadline = time.monotonic() + 10
    while not all(term.screen.buffer[row][column].data == '│' for row in (13, 14, 15, 16)):
        assert time.monotonic() < deadline, term.text
        term.read()


def test_external_configuration_reloads_native_layout_without_opening_the_picker(native_lab):
    lab = native_lab.start()
    config = Path(lab.env['HERDR_CONFIG_PATH'])
    term = open_native(lab)
    native = routed_pick(lab, term, 'Alpha', 'alpha', 'LIVE_RELOAD_NATIVE_ALPHA')
    wait_absent(term, 'Go to')
    wait_sidebar(term, 29)
    config.write_text(config.read_text().replace('sidebar_width = 30', 'sidebar_width = 33'))
    wait_sidebar(term, 32)
    assert viewer(term) == native
    assert 'ctrl+]' not in config.read_text()
    assert_native_only(term)


def wait_route(lab, term, session, token):
    deadline = time.monotonic() + 30
    while token not in lab.pane_text(session, remote=session != 'default').splitlines():
        assert time.monotonic() < deadline, term.text
        term.read()


def test_native_manual_goto_is_verified_before_the_next_launcher_switch(native_lab):
    lab = native_lab.start()
    term = open_native(lab)
    native = routed_pick(lab, term, 'Alpha', 'alpha', 'MANUAL_INITIAL_ALPHA')
    term.send(b'\x02g')
    term.wait('Go to')
    term.send(b'/Beta')
    term.wait(' / Beta')
    term.send(b'\x1b[27u\r')
    term.wait('new · Beta')
    term.send(b"printf 'MANUAL_BETA_PROOF\\n'\r")
    wait_route(lab, term, 'beta', 'MANUAL_BETA_PROOF')
    assert viewer(term) == native
    assert routed_pick(lab, term, 'Local', 'default', 'MANUAL_THEN_LOCAL_PROOF') == native
    assert_native_only(term)


def test_empty_machine_cannot_activate_or_replace_the_current_native_viewer(native_lab):
    lab = native_lab.start()
    term = open_native(lab)
    native = routed_pick(lab, term, 'Alpha', 'alpha', 'EMPTY_INITIAL_ALPHA')
    assert routed_pick(lab, term, 'Beta', 'beta', 'EMPTY_CURRENT_BETA') == native
    alpha = lab.snapshot('alpha')
    lab.api('--session', 'alpha', 'workspace', 'close', alpha['workspaces'][0]['workspace_id'], env=lab.remote)
    assert lab.snapshot('alpha')['workspaces'] == []
    open_picker(term)
    browse_machine(term, 'Alpha')
    term.wait('0 results')
    term.send(b'\r')
    term.wait('Machine: Alpha', '0 results')
    assert viewer(term) == native
    from picker_driver import close_picker
    close_picker(term)
    term.send(b"printf 'EMPTY_BROWSE_STILL_BETA\\n'\r")
    wait_route(lab, term, 'beta', 'EMPTY_BROWSE_STILL_BETA')
    assert_native_only(term)
    assert lab.remote_endpoint_running('beta')


def test_narrow_viewport_fallback_focuses_the_selected_remote_tab(native_lab):
    lab = native_lab.start()
    workspace = lab.snapshot('alpha')['workspaces'][0]['workspace_id']
    lab.api('--session', 'alpha', 'tab', 'create', '--workspace', workspace, '--label', 'Logs', '--no-focus', env=lab.remote)
    term = Terminal([BINARY, 'launch'], lab.env, lab.root, width=60, height=20)
    lab.clients.append(term)
    term.wait('LOCAL_ONLY')
    open_picker(term)
    browse_machine(term, 'Alpha')
    term.send(b'Logs\r')
    from ssh_failures_gate import wait_wire
    wait_wire(term, b'using reattachment')
    term.wait('ALPHA_ONLY')
    term.send(b"printf 'FALLBACK_SELECTED_LOGS\\n'\r")
    term.wait('FALLBACK_SELECTED_LOGS')
    snapshot = lab.snapshot('alpha')
    tab = next(tab for tab in snapshot['tabs'] if tab['label'] == 'Logs')
    pane = next(pane for pane in snapshot['panes'] if pane['tab_id'] == tab['tab_id'])
    from ssh_failures_gate import wait_pane_line
    output = wait_pane_line(lab, 'FALLBACK_SELECTED_LOGS', 'alpha', pane_id=pane['pane_id'])
    assert snapshot['focused_tab_id'] == tab['tab_id']
    assert 'FALLBACK_SELECTED_LOGS' in output
    assert 'FALLBACK_SELECTED_LOGS' not in lab.pane_text('beta')
