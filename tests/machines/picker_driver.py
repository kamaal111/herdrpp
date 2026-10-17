"""Drive machine cycling within the workspace picker while observing complete frames."""
import time


def header(terminal):
    return next(('Machine:' + line.split('Machine:', 1)[1] for line in terminal.text.splitlines() if 'Machine:' in line), '')


def wait_ready(terminal, previous=None):
    deadline = time.monotonic() + 25
    while True:
        terminal.read()
        current = header(terminal)
        if current and current != previous and 'Loading' not in current:
            return current
        assert terminal.process.poll() is None, terminal.text
        assert time.monotonic() < deadline, terminal.text


def browse_machine(terminal, query):
    current = wait_ready(terminal)
    for _ in range(3):
        if query.casefold() in current.casefold():
            return
        terminal.send(b'\x1b[6~')
        current = wait_ready(terminal, previous=current)
    raise AssertionError(f'machine {query} not found: {terminal.text}')


def open_picker(terminal):
    terminal.send(b'\x1d')
    terminal.wait('Machine:', '↵ OK')


def select_machine(terminal, query):
    open_picker(terminal)
    browse_machine(terminal, query)
    terminal.send(b'\r')


def close_picker(terminal):
    terminal.send(b'\x1b')
    deadline = time.monotonic() + 10
    while header(terminal):
        assert terminal.process.poll() is None, terminal.text
        assert time.monotonic() < deadline, terminal.text
        terminal.read()
