"""Research gates against unmodified Herdr 0.9.5, never production promises."""
import codecs
import os
from pathlib import Path
import signal
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests/machines"))

from lab import Lab
import pyte

from terminal import Screen
from frame_terminal import Terminal

OUTPUT = Path('/output/navigator-gate')


def completed(term, predicate, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        term.read()
        if term.frame_serial and predicate(term):
            return term.text
    raise AssertionError((term.frame_serial, repr(term.text), repr(term.pending_frame[-200:])))


def open_navigator(term):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        term.send(b'\x1b[24;5~')
        try:
            return completed(term, lambda t: 'Go to' in t.text, timeout=0.25)
        except AssertionError:
            pass
    raise AssertionError(('native GoTo never opened', term.text))


def accept_first_filtered_pane(term):
    previous = term.frame_serial
    term.send(b'\x1b')
    completed(term, lambda t: t.frame_serial > previous and t.screen.cursor.hidden)
    previous = term.frame_serial
    term.send(b'\x1b[H')
    completed(term, lambda t: t.frame_serial > previous)
    term.send(b'\r')


def record(term, name):
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / (name + '.txt')).write_text(term.text)
    (OUTPUT / (name + '.ansi')).write_bytes(term.raw)


def test_private_client_binding_overlay_opens_navigator_with_hidden_sidebar_and_mouse_off():
    with tempfile.TemporaryDirectory(prefix='navigator-gate-') as directory:
        lab = Lab(Path(directory) / 'lab')
        original = Path(lab.env['HERDR_CONFIG_PATH'])
        original.write_text(original.read_text().replace('sidebar_width = 30', 'sidebar_width = 30\nsidebar_start_collapsed = true\nsidebar_collapsed_mode = "hidden"\nmouse_capture = false') + '\n[keys]\ngoto = []\n')
        before = original.read_bytes()
        try:
            lab.start()
            private = lab.root / 'client-config.toml'
            private.write_text(original.read_text().replace('goto = []', 'goto = "ctrl+f12"'))
            env = dict(lab.env, HERDR_CONFIG_PATH=str(private))
            term = Terminal(['herdr'], env, lab.root)
            lab.clients.append(term)
            completed(term, lambda t: 'LOCAL_ONLY' in t.text)
            term.send(b'\x1b[24;5~')
            completed(term, lambda t: 'Go to' in t.text)
            record(term, 'private-binding-hidden-sidebar')
            assert 'Go to' in term.text
            assert original.read_bytes() == before
            assert 'new · Local' not in term.text
        finally:
            lab.close()


def test_sigwinch_emits_native_full_frame_without_shell_prompt():
    with tempfile.TemporaryDirectory(prefix='navigator-gate-') as directory:
        lab = Lab(Path(directory) / 'lab')
        try:
            lab.start()
            term = Terminal(['herdr'], lab.env, lab.root)
            lab.clients.append(term)
            completed(term, lambda t: 'LOCAL_ONLY' in t.text)
            term.send(b'while :; do printf "\\rBUSY_GATE_%s" "$((n+=1))"; sleep 0.02; done\r')
            completed(term, lambda t: 'BUSY_GATE_' in t.text)
            term.raw.clear()
            os.kill(term.process.pid, signal.SIGWINCH)
            completed(term, lambda t: b'\x1b[?2026l' in t.raw and b'\x1b[?100' in t.raw)
            record(term, 'sigwinch-busy-frame')
            assert b'\x1b[?2026l' in term.raw
            assert b'\x1b[?100' in term.raw
        finally:
            lab.close()


def test_hidden_sidebar_machine_identity_uses_current_pane_ancestry_after_keyboard_selection():
    with tempfile.TemporaryDirectory(prefix='navigator-gate-') as directory:
        lab = Lab(Path(directory) / 'lab')
        original = Path(lab.env['HERDR_CONFIG_PATH'])
        original.write_text(original.read_text().replace('sidebar_width = 30', 'sidebar_width = 30\nsidebar_start_collapsed = true\nsidebar_collapsed_mode = "hidden"\nmouse_capture = false') + '\n[keys]\ngoto = ["ctrl+f12", "prefix+g"]\n')
        try:
            lab.start()
            term = Terminal(['herdr'], lab.env, lab.root)
            lab.clients.append(term)
            completed(term, lambda t: '$' in t.text)
            term.send(b'\x1b[24;5~')
            completed(term, lambda t: 'Go to' in t.text and 'Alpha' in t.text and 'Beta' in t.text and 'connecting' not in t.text)
            term.send(b'/Alpha')
            completed(term, lambda t: ' / Alpha' in t.text)
            accept_first_filtered_pane(term)
            completed(term, lambda t: 'Go to' not in t.text and '$' in t.text)
            open_navigator(term)
            completed(term, lambda t: 'Go to' in t.text and '◆' in t.text)
            record(term, 'hidden-sidebar-alpha-current-pane')
            rows = term.screen.display
            current = next(index for index, row in enumerate(rows) if '◆' in row)
            ancestors = [row for row in rows[:current] if '│ Alpha ' in row]
            assert len(ancestors) == 1
            assert 'ALPHA_ONLY' in rows[current]
            assert 'new · Alpha' not in term.text
        finally:
            lab.close()


def test_empty_machine_has_no_current_pane_identity_in_native_navigator():
    with tempfile.TemporaryDirectory(prefix='navigator-gate-') as directory:
        lab = Lab(Path(directory) / 'lab')
        try:
            lab.start()
            term = Terminal(['herdr'], lab.env, lab.root)
            lab.clients.append(term)
            completed(term, lambda t: 'LOCAL_ONLY' in t.text and 'Beta' in t.text)
            term.send(b'\x02g')
            completed(term, lambda t: 'Go to' in t.text and 'Alpha' in t.text and 'Beta' in t.text and 'connecting' not in t.text)
            term.send(b'/Alpha')
            completed(term, lambda t: ' / Alpha' in t.text)
            accept_first_filtered_pane(term)
            completed(term, lambda t: 'Go to' not in t.text and 'new · Alpha' in t.text)
            alpha = lab.snapshot('alpha')
            lab.api('--session', 'alpha', 'workspace', 'close', alpha['workspaces'][0]['workspace_id'], env=lab.remote)
            completed(term, lambda t: 'new · Alpha' in t.text and 'ALPHA_ONLY' not in t.text)
            term.send(b'\x02g')
            completed(term, lambda t: 'Go to' in t.text)
            record(term, 'empty-alpha-no-current-pane')
            assert lab.snapshot('alpha')['workspaces'] == []
            assert 'new · Alpha' in term.text
            assert '◆' not in term.text
        finally:
            lab.close()


def test_old_complete_marker_does_not_publish_a_fragmented_new_navigator_frame():
    term = Terminal.__new__(Terminal)
    term.screen = Screen(80, 24)
    term.stream = pyte.Stream(term.screen)
    term.decoder = codecs.getincrementaldecoder('utf-8')('replace')
    term.raw = bytearray()
    term.pending_frame = bytearray()
    term.frame_serial = 0
    term.feed_output(b'\x1b[?2026hOLD\x1b[?2026l')
    term.feed_output(b'\x1b[?2026h\x1b[HGo to')
    assert term.frame_serial == 1
    assert 'Go to' not in term.text
    term.feed_output(b' Alpha\x1b[?2026l')
    assert term.frame_serial == 2
    assert 'Go to Alpha' in term.text


def wait_file(path, term, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return
        term.read(.05)
    raise AssertionError(f'remote custom command did not create {path}')


def observe_file(path, term, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return True
        term.read(.05)
    return False


def test_private_navigator_binding_preserves_remote_custom_command_on_the_same_chord():
    with tempfile.TemporaryDirectory(prefix='navigator-collision-') as directory:
        lab = Lab(Path(directory) / 'lab')
        marker = lab.root / 'remote-custom-command'
        remote_config = Path(lab.remote['HERDR_CONFIG_PATH'])
        remote_config.write_text(remote_config.read_text() + f'\n[[keys.command]]\nkey = "ctrl+f12"\ntype = "shell"\ncommand = "printf handled > {marker}"\n')
        try:
            lab.start()
            baseline = Terminal(['herdr'], lab.env, lab.root)
            lab.clients.append(baseline)
            completed(baseline, lambda t: 'ALPHA_ONLY' in t.text and 'BETA_ONLY' in t.text)
            baseline.send(b'\x02g')
            completed(baseline, lambda t: 'Go to' in t.text)
            baseline.send(b'/Alpha')
            completed(baseline, lambda t: ' / Alpha' in t.text)
            accept_first_filtered_pane(baseline)
            completed(baseline, lambda t: 'Go to' not in t.text and 'new · Alpha' in t.text)
            baseline.send(b'\x02g')
            completed(baseline, lambda t: 'Go to' in t.text and '◆' in t.text)
            baseline.send(b'\x1b')
            completed(baseline, lambda t: 'Go to' not in t.text)
            baseline.send(b'\x1b[24;5~')
            wait_file(marker, baseline)
            assert marker.read_text() == 'handled'
            baseline.close()
            lab.clients.remove(baseline)
            marker.unlink()
            private = lab.root / 'private-config.toml'
            private.write_text(Path(lab.env['HERDR_CONFIG_PATH']).read_text() + '\n[keys]\ngoto = ["prefix+g", "ctrl+f12"]\n')
            injected = Terminal(['herdr'], dict(lab.env, HERDR_CONFIG_PATH=str(private)), lab.root)
            lab.clients.append(injected)
            completed(injected, lambda t: 'new · Alpha' in t.text)
            injected.send(b'\x1b[24;5~')
            completed(injected, lambda t: 'Go to' in t.text)
            record(injected, 'remote-custom-command-intercepted')
            assert observe_file(marker, injected), 'required gate failed: private navigator key intercepted an existing remote command'
        finally:
            lab.close()


def open_public_reserved_navigator(term):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        term.send(b'\x1d')
        try:
            return completed(term, lambda t: 'Go to' in t.text, timeout=.25)
        except AssertionError:
            pass
    raise AssertionError(('public reserved GoTo never opened', term.text))


def test_public_reserved_control_chord_preserves_remote_control_f12_custom_command():
    with tempfile.TemporaryDirectory(prefix='navigator-public-chord-') as directory:
        lab = Lab(Path(directory) / 'lab')
        marker = lab.root / 'remote-custom-command'
        remote_config = Path(lab.remote['HERDR_CONFIG_PATH'])
        remote_config.write_text(remote_config.read_text() + f'\n[[keys.command]]\nkey = "ctrl+f12"\ntype = "shell"\ncommand = "printf handled > {marker}"\n')
        remote_original = remote_config.read_bytes()
        user_config = Path(lab.env['HERDR_CONFIG_PATH'])
        user_original = user_config.read_bytes()
        try:
            lab.start()
            private = lab.root / 'private-config.toml'
            private.write_text(user_config.read_text() + '\n[keys]\ngoto = ["prefix+g", "ctrl+]"]\n')
            term = Terminal(['herdr'], dict(lab.env, HERDR_CONFIG_PATH=str(private)), lab.root)
            lab.clients.append(term)
            completed(term, lambda t: 'ALPHA_ONLY' in t.text and 'BETA_ONLY' in t.text)
            open_public_reserved_navigator(term)
            completed(term, lambda t: 'Alpha' in t.text and 'connecting' not in t.text)
            term.send(b'/Alpha')
            completed(term, lambda t: ' / Alpha' in t.text)
            accept_first_filtered_pane(term)
            completed(term, lambda t: 'Go to' not in t.text and 'new · Alpha' in t.text)
            open_public_reserved_navigator(term)
            completed(term, lambda t: '◆' in t.text)
            term.send(b'\x1b')
            completed(term, lambda t: 'Go to' not in t.text)
            term.send(b'\x1b[24;5~')
            wait_file(marker, term)
            assert marker.read_text() == 'handled'
            open_public_reserved_navigator(term)
            record(term, 'public-reserved-control-keeps-remote-command')
            assert 'Go to' in term.text
            assert 'ALPHA_ONLY' in term.text
            assert user_config.read_bytes() == user_original
            assert remote_config.read_bytes() == remote_original
        finally:
            lab.close()
