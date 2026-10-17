"""PTY client and rendered-screen waits shared by the SSH tests."""
import codecs
import errno
import fcntl
import os
import pty
import select
import signal
import struct
import subprocess
import termios
import time

import pyte


class Screen(pyte.Screen):
    def report_device_status(self, mode, **kwargs):
        pass


class Terminal:
    def __init__(self, command, env, cwd, width=120, height=36, output=None):
        self.master, slave = pty.openpty()
        self.width, self.height = width, height
        self.initial_termios = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
        self.screen = Screen(width, height)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')
        self.output = output
        self.process = subprocess.Popen(command, env=env, cwd=cwd, stdin=slave,
                                        stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)
        self.raw = bytearray()

    @property
    def text(self):
        return '\n'.join(self.screen.display)

    def read(self, timeout=.05):
        if not select.select([self.master], [], [], timeout)[0]:
            return b''
        try:
            data = os.read(self.master, 1048576)
        except OSError as error:
            if error.errno == errno.EIO:
                return b''
            raise
        self.raw.extend(data)
        self.stream.feed(self.decoder.decode(data))
        if self.output is not None:
            os.write(self.output, data)
        return data

    def send(self, data):
        os.write(self.master, data)

    def wait(self, *needles, timeout=20):
        deadline = time.monotonic() + timeout
        previous, stable_since = None, time.monotonic()
        while time.monotonic() < deadline:
            self.read()
            if self.text != previous:
                previous, stable_since = self.text, time.monotonic()
            if all(needle in self.text for needle in needles) and time.monotonic() - stable_since >= .2:
                return self.text
            if self.process.poll() is not None:
                raise RuntimeError(f'Client exited {self.process.returncode}: {self.text}')
        raise TimeoutError(f'Missing {needles}:\n{self.text}')

    def wait_exit(self, timeout=10):
        deadline = time.monotonic() + timeout
        while self.process.poll() is None and time.monotonic() < deadline:
            self.read()
        return self.process.poll()

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=5)
        os.close(self.master)
