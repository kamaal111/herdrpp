"""Own the host terminal and delay replies while the real Rust selector runs."""
import fcntl
import os
from pathlib import Path
import pty
import signal
import select
import time
import struct
import subprocess
import sys
import termios

import pytest
import pyte

from pty_smoke import read_until, stable_terminal_attributes, write_all

from machine_fake import API_ROUTES

FAKE = API_ROUTES + r'''import json, os, signal, sys, tty
args = sys.argv[1:]
if args == ['machine', 'list', '--json']:
    print(json.dumps([dict(id='alpha', label='Alpha', target='alice@build', session='engineering', enabled=True)]))
    sys.exit(0)
tty.setraw(0)
signal.signal(signal.SIGWINCH, lambda *_: os.write(1, b'HOST_CLIENT_REDRAW\r\n'))
os.write(1, b'\x1b[?7l\x1b[?1016$p\x1b[6n\x1b[?2026hHOST_CLIENT_READY\r\n\x1b[?2026l')
received = b''
while True:
    data = os.read(0, 4096)
    if not data:
        break
    received += data
    with open(os.environ['MACHINE_INPUT_LOG'], 'wb') as log:
        log.write(received)
    os.write(1, f'CHILD_BYTES={len(received)}\r\n'.encode())
'''


def read_text_until(fd, needle):
    screen = pyte.Screen(100, 24)
    stream = pyte.Stream(screen)
    deadline = time.monotonic() + 8
    while needle not in '\n'.join(screen.display):
        assert time.monotonic() < deadline, screen.display
        if select.select([fd], [], [], 0.1)[0]:
            stream.feed(os.read(fd, 65536).decode('utf-8', errors='replace'))
    return screen


@pytest.fixture
def launch(tmp_path, release_binary):
    fake = tmp_path / 'herdr'
    fake.write_text(f'#!{sys.executable}\n' + FAKE)
    fake.chmod(0o700)
    config = tmp_path / 'config.toml'
    config.write_text('[theme]\nname="dracula"\n')
    log = tmp_path / 'input'
    environment = dict(os.environ, HERDR_BIN_PATH=str(fake), HERDR_CONFIG_PATH=str(config),
                       MACHINE_INPUT_LOG=str(log), TERM='xterm-256color', XDG_STATE_HOME=str(tmp_path / 'state'))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
    original = stable_terminal_attributes(slave)
    process = subprocess.Popen([release_binary, 'launch'], env=environment,
                               stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
    try:
        read_until(master, b'HOST_CLIENT_READY')
        yield process, master, slave, original, log
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)


def test_delayed_cursor_clipboard_graphics_replies_go_to_owned_client_during_picker(launch):
    process, master, slave, original, log = launch
    os.write(master, b'\x1d')
    read_until(master, b'Machine: ')
    replies = b'\x1b[?1016;2$y\x1b[12;34R\x1b]52;c;aGVsbG8=\x07\x1b_Gi=42;OK\x1b\\'
    os.write(master, replies + b'UNIQUE_QUERY')
    read_text_until(master, 'UNIQUE_QUERY')
    os.write(master, b'\x1b')
    read_until(master, f'CHILD_BYTES={len(replies)}'.encode())
    assert log.read_bytes() == replies
    os.write(master, b'\x1c')
    process.wait(timeout=5)
    assert process.returncode == 0
    assert stable_terminal_attributes(slave) == original


def test_startup_cursor_reply_does_not_capture_later_modified_f3_in_picker(launch):
    process, master, slave, original, log = launch
    startup = b'\x1b[1;2R'
    os.write(master, startup)
    read_until(master, f'CHILD_BYTES={len(startup)}'.encode())
    os.write(master, b'\x1d')
    read_until(master, b'Machine: ')
    os.write(master, b'\x1b[1;2R\x1b')
    read_until(master, b'HOST_CLIENT_REDRAW')
    assert log.read_bytes() == startup
    os.write(master, b'\x1c')
    process.wait(timeout=5)
    assert process.returncode == 0
    assert stable_terminal_attributes(slave) == original


@pytest.mark.parametrize('prefix', [b'\x1b[', b'\x1b]', b'\x1b]title', b'\x1b_payload'])
def test_incomplete_alt_prefixes_reach_client_after_input_ambiguity_timeout(launch, prefix):
    process, master, slave, original, log = launch
    os.write(master, prefix)
    read_until(master, f'CHILD_BYTES={len(prefix)}'.encode())
    assert log.read_bytes() == prefix
    os.write(master, b'\x1c')
    process.wait(timeout=5)
    assert process.returncode == 0
    assert stable_terminal_attributes(slave) == original


def test_clipboard_reply_larger_than_pty_capacity_is_spooled_until_client_resumes(launch):
    process, master, slave, original, log = launch
    os.write(master, b'\x1d')
    read_until(master, b'Machine: ')
    reply = b'\x1b]52;c;' + b'YWJj' * 100_000 + b'\x07'
    write_all(master, reply)
    os.write(master, b'\x1b')
    read_until(master, f'CHILD_BYTES={len(reply)}'.encode())
    assert log.read_bytes() == reply
    os.write(master, b'\x1c')
    process.wait(timeout=5)
    assert process.returncode == 0
    assert stable_terminal_attributes(slave) == original


@pytest.mark.parametrize('command', ['binary', 'cargo'])
def test_quit_reaps_a_client_blocked_on_pty_output_and_restores_terminal(tmp_path, release_binary, command):
    executable = tmp_path / 'herdr'
    blocked = tmp_path / 'blocked'
    pidfile = tmp_path / 'client-pid'
    executable.write_text(f'#!{sys.executable}\n' + r'''import os, signal, sys, termios, tty
from pathlib import Path
if sys.argv[1:] == ['machine', 'list', '--json']:
    print('[]')
    sys.exit(0)
tty.setraw(0)
signal.signal(signal.SIGTERM, signal.SIG_IGN)
Path(os.environ['SHUTDOWN_PID']).write_text(str(os.getpid()))
os.write(1, b'\x1b[?7l\x1b[?2026hREADY_TO_QUIT\x1b[?2026l')
received = b''
while b'BLOCK' not in received:
    received += os.read(0, 4096)
termios.tcflow(1, termios.TCOOFF)
Path(os.environ['SHUTDOWN_BLOCKED']).write_text('blocked')
os.write(1, b'X' * 200000)
''')
    executable.chmod(0o700)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
    original = stable_terminal_attributes(slave)
    environment = dict(os.environ, HERDR_BIN_PATH=str(executable), XDG_STATE_HOME=str(tmp_path / 'state'), HERDR_CONFIG_PATH=str(tmp_path / 'config.toml'), SHUTDOWN_PID=str(pidfile), SHUTDOWN_BLOCKED=str(blocked))
    arguments = {'binary': [str(release_binary), 'launch'], 'cargo': ['cargo', 'run', '--locked', '--release', '--', 'launch']}
    process = subprocess.Popen(arguments[command], cwd=Path(__file__).resolve().parents[1], env=environment, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
    try:
        read_until(master, b'READY_TO_QUIT')
        os.write(master, b'BLOCK')
        wait_shutdown_blocked(master, blocked)
        owned_pid = int(pidfile.read_text())
        os.write(master, b'\x1c')
        from test_machine_handoff import wait_exit
        wait_exit(process, master, timeout=6)
        assert process.returncode == 0
        with pytest.raises(ProcessLookupError):
            os.kill(owned_pid, 0)
        assert stable_terminal_attributes(slave) == original
    finally:
        close_shutdown_client(process, master, slave)


def wait_shutdown_blocked(master, blocked):
    deadline = time.monotonic() + 8
    while not blocked.exists():
        assert time.monotonic() < deadline, 'owned client did not stop its PTY output'
        if select.select([master], [], [], .05)[0]:
            os.read(master, 65536)


def close_shutdown_client(process, master, slave):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)
    os.close(master)
    os.close(slave)
