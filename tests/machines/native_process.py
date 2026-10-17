"""Linux process evidence shared by native navigator and rendered graphics gates."""
from contextlib import contextmanager
import errno
import os
import signal
import termios
import time
from pathlib import Path


def viewer_pid(launcher_pid):
    children = Path(f'/proc/{launcher_pid}/task/{launcher_pid}/children').read_text().split()
    for pid in children:
        command = Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
        if command and Path(os.fsdecode(command[0])).name == 'herdr':
            return int(pid)
    raise AssertionError('owned Herdr viewer is absent')


@contextmanager
def blocked_viewer_output(pid):
    """Fence native-open frames before picker SIGCONT can resume the viewer."""
    descriptor = os.open(f'/proc/{pid}/fd/1', os.O_RDWR | os.O_NOCTTY)
    try:
        assert os.isatty(descriptor), 'owned viewing client output is not a PTY'
        termios.tcflow(descriptor, termios.TCOOFF)
        yield
    finally:
        try:
            termios.tcflow(descriptor, termios.TCOON)
        except termios.error as error:
            if error.args[0] != errno.EIO:
                raise
        finally:
            os.close(descriptor)
        try:
            os.kill(pid, signal.SIGCONT)
        except ProcessLookupError:
            pass


def stop_viewer(term, pid):
    os.kill(pid, signal.SIGSTOP)
    deadline = time.monotonic() + 2
    status = Path(f'/proc/{pid}/status')
    while True:
        assert term.process.poll() is None, term.text
        state = next(line for line in status.read_text().splitlines()
                     if line.startswith('State:'))
        if state.split()[1] == 'T':
            return state
        assert time.monotonic() < deadline, state
        term.read(.01)
