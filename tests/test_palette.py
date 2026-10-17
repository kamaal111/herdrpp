"""Drive the real palette owner against a fake Herdr process boundary."""

import json
import os
import pty
from pathlib import Path
import subprocess
import time

import pytest


FAKE = '''#!/usr/bin/env python3
import json, os, sys, time
from pathlib import Path
args = sys.argv[1:]
root = Path(os.environ["PALETTE_TEST_ROOT"])
with (root / "calls").open("a") as log:
    log.write(json.dumps(args) + "\\n")
result = {"type":"ok"}
if args == ["api", "snapshot"]:
    result = {"snapshot": {
        "workspaces":[{"workspace_id":"w2"}],
        "tabs":[{"tab_id":"w2:t7","workspace_id":"w2"}],
        "panes":[{"pane_id":"w2:p8","workspace_id":"w2","tab_id":"w2:t7"}]
    }}
elif args[:3] == ["plugin", "pane", "open"] and "--env" in args:
    request = args[args.index("--env") + 1].split("=", 1)[1]
    (root / "request").write_text(request)
    result = {"type":"ok"}
elif args == ["plugin", "list", "--json"]:
    if os.environ.get("PALETTE_TEST_WAIT"):
        time.sleep(30)
    if os.environ.get("PALETTE_TEST_DISCOVERY_FAIL"):
        print("discovery unavailable", file=sys.stderr)
        sys.exit(1)
    result = {"plugins":[]}
elif args == ["plugin", "action", "list"]:
    result = {"actions":[]}
print(json.dumps({"result":result}))
'''


def await_file(path):
    deadline = time.monotonic() + 8
    while not path.exists():
        assert time.monotonic() < deadline, path
        time.sleep(0.02)
    return path


class TestPaletteOwner:
    @pytest.fixture(autouse=True)
    def setup(self, tmp_path, release_binary):
        self.root = tmp_path
        self.binary = release_binary
        fake = tmp_path / "herdr"
        fake.write_text(FAKE)
        fake.chmod(0o700)
        self.env = dict(os.environ, HERDR_BIN_PATH=str(fake),
                        PALETTE_TEST_ROOT=str(tmp_path),
                        HERDR_SOCKET_PATH=str(tmp_path / "unused.sock"),
                        HERDR_PLUGIN_CONTEXT_JSON=json.dumps({"workspace_id":"w2", "tab_id":"w2:t7", "focused_pane_id":"w2:p8"}))
        self.process = None
        self.master = self.slave = None
        yield
        if self.process is not None and self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=5)
        self.close_popup()

    def close_popup(self):
        if self.master is not None:
            os.close(self.master)
            self.master = None
        if self.slave is not None:
            os.close(self.slave)
            self.slave = None

    def start(self, discovery=True):
        self.process = subprocess.Popen([self.binary, "commands"], env=self.env,
                                        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        path = await_file(self.root / "request")
        directory = Path(path.read_text())
        self.master, self.slave = pty.openpty()
        (directory / "terminal.json").write_text(json.dumps(os.ttyname(self.slave)))
        if discovery:
            await_file(directory / "catalog.json")
        return directory

    def select(self, directory, selection):
        pending = directory / "pending.json"
        pending.write_text(json.dumps(selection))
        pending.replace(directory / "selection.json")
        with pytest.raises(subprocess.TimeoutExpired):
            self.process.communicate(timeout=0.15)
        self.close_popup()
        stdout, stderr = self.process.communicate(timeout=8)
        assert self.process.returncode == 0, (stdout, stderr)
        return [json.loads(line) for line in (self.root / "calls").read_text().splitlines()]

    def test_owner_waits_for_host_popup_teardown_before_exactly_one_original_pane_split(self):
        directory = self.start()
        calls = self.select(directory, {"id":"herdr:split_right", "input":None, "confirmed":False})
        split = ["pane", "split", "--pane", "w2:p8", "--direction", "right", "--focus"]
        assert [call for call in calls if call[:3] == ["plugin", "pane", "close"]] == []
        assert calls.count(split) == 1
        assert not directory.exists()

    def test_owner_waits_for_host_popup_teardown_before_opening_workspace_picker(self):
        directory = self.start()
        calls = self.select(directory, {"id":"herdr:search_workspaces", "input":None, "confirmed":False})
        assert calls.count(["plugin", "pane", "open", "--plugin", "herdrpp", "--entrypoint", "picker"]) == 1
        assert not directory.exists()

    def test_cancellation_during_slow_discovery_stops_worker_without_execution(self):
        self.env["PALETTE_TEST_WAIT"] = "1"
        directory = self.start(discovery=False)
        calls = self.select(directory, None)
        assert [call for call in calls if call[:2] == ["pane", "split"]] == []
        assert not directory.exists()

    def test_discovery_failure_retains_searchable_builtins_and_visible_warning(self):
        self.env["PALETTE_TEST_DISCOVERY_FAIL"] = "1"
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert len(catalog["items"]) == 16
        assert "Plugin commands unavailable" in catalog["warning"]
        calls = self.select(directory, None)
        assert [call for call in calls if call[:3] == ["plugin", "pane", "close"]] == []
