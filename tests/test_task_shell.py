"""Run real interactive shells behind the task owner's Herdr transport boundary."""
import json
import os
from pathlib import Path
import pty
import signal
import select
import threading
import subprocess
import sys
import termios
import time

import pytest

from test_tasks import TaskOwnerHarness

SHELLS = [path for path in ("/bin/sh", "/bin/bash", "/bin/zsh") if Path(path).is_file()]


class TestTaskShell(TaskOwnerHarness):
    @pytest.fixture(autouse=True, params=SHELLS, ids=lambda path: Path(path).name)
    def shell_setup(self, tasks_setup, request):
        self.shell_path = request.param
        self.shell_process = None
        self.shell_master = None
        self.shell_slave = None
        self.task_pane = None
        self.terminal_output = bytearray()
        self.terminal_stop = threading.Event()
        self.terminal_reader = None
        self.shell_home = self.root / "shell-home"
        self.shell_home.mkdir()
        self.rc_directory = self.root / "shell-config"
        self.rc_directory.mkdir()
        self.started = self.rc_directory / "started"
        self.release = self.rc_directory / "release"
        self.loaded = self.rc_directory / "loaded"
        self.task_result = self.checkout / "task-result"
        self.prompt_result = self.checkout / "prompt-result"
        script = '''printf started > "$TASK_RC_DIR/started"
while [ ! -e "$TASK_RC_DIR/release" ]; do :; done
export TASK_USER_SETTING=loaded-from-user-config
task_user_function() { printf '%s' "$TASK_USER_SETTING"; }
printf loaded > "$TASK_RC_DIR/loaded"
'''
        (self.rc_directory / ".zshrc").write_text(script)
        (self.rc_directory / "shell-rc").write_text(script)
        (self.shell_home / ".bashrc").write_text(script)
        (self.shell_home / ".zshrc").write_text("export TASK_USER_SETTING=wrong-home-config\n")
        self.task_file.write_text(json.dumps([{
            "label":"test config",
            "command": "printf '%s' \"$TASK_USER_SETTING\" > task-result",
        }]))
        yield
        self.release.touch()
        self.terminal_stop.set()
        if self.terminal_reader is not None:
            self.terminal_reader.join(timeout=1)
        (self.rc_directory / "terminal.txt").write_bytes(self.terminal_output)
        print(f"shell output ({self.shell_path}): {bytes(self.terminal_output)!r}")
        for fd in (self.shell_master, self.shell_slave):
            if fd is not None:
                os.close(fd)
        if self.shell_process is not None:
            try:
                os.killpg(self.shell_process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.shell_process.kill()
            self.shell_process.wait(timeout=3)

    def wait_value(self, path, expected):
        deadline = time.monotonic() + 8
        value = None
        while value != expected:
            try:
                value = path.read_text()
            except FileNotFoundError:
                value = None
            assert time.monotonic() < deadline, f"Shell did not write {expected!r} to {path}: {value!r}"
            time.sleep(0.01)
        return value

    def read_terminal(self):
        while not self.terminal_stop.is_set():
            if select.select([self.shell_master], [], [], 0.02)[0]:
                try:
                    self.terminal_output.extend(os.read(self.shell_master, 65536))
                except OSError:
                    return

    def start_shell(self, pane):
        self.task_pane = pane
        self.shell_master, self.shell_slave = pty.openpty()
        termios.tcsetwinsize(self.shell_slave, (24, 120))
        self.terminal_reader = threading.Thread(target=self.read_terminal, daemon=True)
        self.terminal_reader.start()
        environment = dict(os.environ, HOME=str(self.shell_home),
                           ZDOTDIR=str(self.rc_directory), ENV=str(self.rc_directory / "shell-rc"),
                           TASK_RC_DIR=str(self.rc_directory), TERM="xterm-256color", PS1="$ ")
        environment.pop("TASK_USER_SETTING", None)
        environment.pop("BASH_ENV", None)
        helper = "import os,sys; os.login_tty(int(sys.argv[1])); os.execve(sys.argv[2], [sys.argv[2], '-i'], os.environ)"
        self.shell_process = subprocess.Popen(
            [sys.executable, "-c", helper, str(self.shell_slave), self.shell_path],
            env=environment, cwd=self.checkout, pass_fds=(self.shell_slave,),
        )
        self.wait_value(self.started, "started")

    def respond(self, method, params):
        result = super().respond(method, params)
        if method == "tab.create":
            tab = result["tab"]["tab_id"]
            pane = next(p for p in self.snapshot["panes"] if p["tab_id"] == tab)
            self.start_shell(pane["pane_id"])
        if params.get("pane_id") == self.task_pane:
            if method == "pane.send_text":
                os.write(self.shell_master, params["text"].encode())
            if method == "pane.send_keys":
                keys = {"ctrl+c":b"\x03", "ctrl+u":b"\x15", "enter":b"\r"}
                os.write(self.shell_master, b"".join(keys[key] for key in params["keys"]))
                if "enter" in params["keys"]:
                    self.release.touch()
        return result

    def test_task_waits_for_user_startup_and_returns_to_the_same_configured_shell(self):
        self.launch()
        assert self.wait_value(self.task_result, "loaded-from-user-config") == "loaded-from-user-config"
        assert self.loaded.read_text() == "loaded"
        os.write(self.shell_master, b"task_user_function > prompt-result\r")
        assert self.wait_value(self.prompt_result, "loaded-from-user-config") == "loaded-from-user-config"
        assert self.shell_process.poll() is None
