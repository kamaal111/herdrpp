"""Drive the real palette owner against a fake Herdr process boundary."""

import json
import os
import pty
import socket
import threading
import tempfile
from pathlib import Path
import subprocess
import sys
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
        "workspaces":[{"workspace_id":"w2","label":"Project","new_workspace_cwd":str(root / "project")}],
        "tabs":[{"tab_id":"w2:t7","workspace_id":"w2","label":"Codex","custom_label":False}],
        "panes":[{"pane_id":"w2:p8","workspace_id":"w2","tab_id":"w2:t7","label":"Terminal"}]
    }}
    if (root / "snapshot.json").exists():
        result["snapshot"] = json.loads((root / "snapshot.json").read_text())
    if os.environ.get("PALETTE_TEST_TARGET_GONE") and (root / "request").exists():
        result["snapshot"]["tabs"] = []
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


class PaletteOwnerHarness:
    action = "commands"
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

    @pytest.fixture
    def socket_requests(self):
        requests = []
        socket_directory = tempfile.TemporaryDirectory(prefix="hpp-", dir="/tmp")
        self.env["HERDR_SOCKET_PATH"] = str(Path(socket_directory.name) / "api.sock")
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(self.env["HERDR_SOCKET_PATH"])
        listener.listen(1)
        listener.settimeout(8)

        def serve():
            with listener.accept()[0] as connection:
                with connection.makefile("rb") as stream:
                    request = json.loads(stream.readline())
                requests.append(request)
                connection.sendall((json.dumps({"id": request["id"], "result": {"type": "ok"}}) + "\n").encode())

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()
        yield requests
        worker.join(timeout=8)
        listener.close()
        socket_directory.cleanup()

    def close_popup(self):
        if self.master is not None:
            os.close(self.master)
            self.master = None
        if self.slave is not None:
            os.close(self.slave)
            self.slave = None

    def start(self, discovery=True):
        self.process = subprocess.Popen([self.binary, self.action], env=self.env,
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


class TestPaletteOwner(PaletteOwnerHarness):
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

    @pytest.mark.parametrize(("command", "method", "params"), [
        ("herdr:tab_create", "tab.create", {"workspace_id":"w2", "focus":True, "label":"--世界 Review"}),
        ("herdr:workspace_create", "workspace.create", {"source_workspace_id":"w2", "focus":True, "label":"--世界 Review"}),
    ])
    def test_named_creation_waits_for_popup_teardown_and_targets_original_workspace(self, socket_requests, command, method, params):
        directory = self.start()
        calls = self.select(directory, {"id":command, "input":"--世界 Review", "confirmed":False})
        assert len(socket_requests) == 1
        assert socket_requests[0]["method"] == method
        assert {key: socket_requests[0]["params"][key] for key in params} == params
        assert calls.count(["api", "snapshot"]) == 2
        assert not directory.exists()

    def test_workspace_creation_uses_the_directory_captured_before_popup(self, socket_requests):
        directory = self.start()
        self.select(directory, {"id":"herdr:workspace_create", "input":"Review", "confirmed":False})
        assert socket_requests[0]["params"]["cwd"] == str(self.root / "project")

    @pytest.mark.parametrize("text", ["", "2"])
    def test_creation_with_blank_or_suggested_name_uses_automatic_naming(self, socket_requests, text):
        directory = self.start()
        self.select(directory, {"id":"herdr:tab_create", "input":text, "confirmed":False})
        assert len(socket_requests) == 1
        assert socket_requests[0]["params"]["label"] is None

    def test_catalog_transfers_snapshot_names_without_replacing_plugin_context(self):
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert catalog["naming"]["workspace_name"] == "Project"
        assert catalog["naming"]["tab_name"] == "Codex"
        assert catalog["naming"]["pane_name"] == "Terminal"
        assert catalog["naming"]["tab_auto_named"] is True
        assert catalog["naming"]["tab_suggestion"] == "2"
        assert catalog["naming"]["workspace_suggestion"] == "project"
        assert "naming" not in catalog["context"]
        self.select(directory, None)

    def test_accepting_unchanged_automatic_tab_name_performs_no_rename(self):
        directory = self.start()
        calls = self.select(directory, {"id":"herdr:tab_rename", "input":"Codex", "confirmed":False})
        assert calls.count(["api", "snapshot"]) == 2
        assert self.process.returncode == 0

    def test_creation_rejects_a_closed_original_target_without_socket_execution(self):
        self.env["PALETTE_TEST_TARGET_GONE"] = "1"
        directory = self.start()
        (directory / "selection.json").write_text(json.dumps({"id":"herdr:tab_create", "input":"Review", "confirmed":False}))
        with pytest.raises(subprocess.TimeoutExpired):
            self.process.communicate(timeout=0.15)
        self.close_popup()
        stdout, stderr = self.process.communicate(timeout=8)
        assert self.process.returncode == 1, (stdout, stderr)
        assert "original tab is no longer available" in stderr


PR_FAKE = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
root = Path(os.environ["PALETTE_TEST_ROOT"])
args = sys.argv[1:]
kind = Path(sys.argv[0]).name
with (root / (kind + "-calls")).open("a") as log:
    log.write(json.dumps({"args":args, "cwd":os.getcwd(), "GH_REPO":os.environ.get("GH_REPO"), "GH_HOST":os.environ.get("GH_HOST")}) + "\n")
if os.environ.get("PALETTE_TEST_FAIL") == kind:
    print(kind + " unavailable", file=sys.stderr)
    sys.exit(1)
if kind == "gh":
    if args[:2] == ["repo", "view"]:
        repository = "team/project"
        if args[2] != "--json":
            repository = "/".join(args[2].split("/")[1:])
        print(json.dumps({"nameWithOwner":repository, "url":"https://github.example/" + repository, "defaultBranchRef":{"name":"main"}}))
    elif args[:2] == ["pr", "view"]:
        assert args == ["pr", "view", "123", "--repo", "github.example/team/project", "--json", "number,url"], args
        print((root / "pr-url.json").read_text())
    else:
        sys.exit(12)
'''


class TestPalettePullRequest(PaletteOwnerHarness):
    @pytest.fixture(autouse=True)
    def pr_setup(self, setup):
        self.project = self.root / "project"
        self.project.mkdir()
        self.env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
                        GH_REPO="unrelated/repository", GH_HOST="unrelated.example")
        subprocess.run(["git", "init", "-q", "-b", "feature", str(self.project)], env=self.env, check=True)
        subprocess.run(["git", "-C", str(self.project), "remote", "add", "origin", "git@github.example:alice/fork.git"], env=self.env, check=True)
        fake_bin = self.root / "bin"
        fake_bin.mkdir()
        for name in ("gh", "open", "xdg-open"):
            fake = fake_bin / name
            fake.write_text(PR_FAKE)
            fake.chmod(0o700)
        self.env["PATH"] = str(fake_bin) + os.pathsep + self.env["PATH"]
        self.snapshot = {
            "workspaces":[
                {"workspace_id":"w1", "active_tab_id":"w1:t1", "tokens":{"pr_number":"#456"}},
                {"workspace_id":"w2", "active_tab_id":"w2:t7", "tokens":{"pr_number":"#123"},
                 "worktree":{"checkout_path":str(self.project)}}
            ],
            "tabs":[{"tab_id":"w2:t7", "workspace_id":"w2", "label":"Codex", "custom_label":False}],
            "panes":[{"pane_id":"w2:p8", "workspace_id":"w2", "tab_id":"w2:t7"}],
            "layouts":[], "focused_workspace_id":"w2"
        }
        self.write_snapshot()
        (self.root / "pr-url.json").write_text(json.dumps({"number":123, "url":"https://github.example/team/project/pull/123"}))

    def write_snapshot(self):
        (self.root / "snapshot.json").write_text(json.dumps(self.snapshot))

    def logged_calls(self, name):
        return [json.loads(line) for line in (self.root / (name + "-calls")).read_text().splitlines()]

    def select_pr(self, directory):
        return self.select(directory, {"id":"herdr:view_pull_request", "input":None, "confirmed":False})

    def select_pr_failure(self, directory):
        (directory / "selection.json").write_text(json.dumps({"id":"herdr:view_pull_request", "input":None, "confirmed":False}))
        with pytest.raises(subprocess.TimeoutExpired):
            self.process.communicate(timeout=0.15)
        self.close_popup()
        stdout, stderr = self.process.communicate(timeout=8)
        assert self.process.returncode == 1, (stdout, stderr)
        return stderr

    @pytest.mark.parametrize("lifecycle_token", ["pr_open", "pr_draft", "pr_closed", "pr_merged"])
    def test_all_pr_states_open_fork_url_after_teardown_without_discovery_lookup(self, lifecycle_token):
        self.snapshot["workspaces"][1]["tokens"][lifecycle_token] = "#123"
        self.write_snapshot()
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert [item["title"] for item in catalog["items"] if item["id"] == "herdr:view_pull_request"] == ["View Pull Request"]
        assert not (self.root / "gh-calls").exists()
        self.select_pr(directory)
        assert self.logged_calls("gh") == [
            {"args":["repo", "view", "--json", "nameWithOwner,url,defaultBranchRef"], "cwd":str(self.project), "GH_REPO":None, "GH_HOST":"github.example"},
            {"args":["repo", "view", "github.example/alice/fork", "--json", "nameWithOwner,url,defaultBranchRef"], "cwd":str(self.project), "GH_REPO":None, "GH_HOST":"github.example"},
            {"args":["pr", "view", "123", "--repo", "github.example/team/project", "--json", "number,url"], "cwd":str(self.project), "GH_REPO":None, "GH_HOST":"github.example"},
        ]
        launcher = {"darwin":"open", "linux":"xdg-open"}[sys.platform]
        assert [call["args"] for call in self.logged_calls(launcher)] == [["https://github.example/team/project/pull/123"]]

    def test_focus_and_status_changes_preserve_captured_pr(self):
        directory = self.start()
        self.snapshot["focused_workspace_id"] = "w1"
        self.snapshot["workspaces"][1]["tokens"]["pr_number"] = "#789"
        self.write_snapshot()
        self.select_pr(directory)
        assert self.logged_calls("gh")[-1]["args"][2] == "123"

    def test_cancellation_never_resolves_or_opens_pr(self):
        directory = self.start()
        self.select(directory, None)
        assert not (self.root / "gh-calls").exists()
        assert not (self.root / "open-calls").exists()
        assert not (self.root / "xdg-open-calls").exists()

    @pytest.mark.parametrize("token", [None, "#0", "invalid"])
    def test_missing_or_invalid_status_omits_command_despite_other_workspace_pr(self, token):
        self.snapshot["workspaces"][1]["tokens"]["pr_number"] = token
        self.write_snapshot()
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert "herdr:view_pull_request" not in [item["id"] for item in catalog["items"]]
        self.select(directory, None)
        assert not (self.root / "gh-calls").exists()

    def test_closed_original_target_prevents_lookup_and_browser_launch(self):
        directory = self.start()
        self.snapshot["tabs"] = []
        self.write_snapshot()
        assert "original tab is no longer available" in self.select_pr_failure(directory)
        assert not (self.root / "gh-calls").exists()
        assert not (self.root / "open-calls").exists()
        assert not (self.root / "xdg-open-calls").exists()

    def test_github_failure_prevents_browser_launch(self):
        self.env["PALETTE_TEST_FAIL"] = "gh"
        directory = self.start()
        assert "gh unavailable" in self.select_pr_failure(directory)
        assert not (self.root / "open-calls").exists()
        assert not (self.root / "xdg-open-calls").exists()

    @pytest.mark.parametrize("response", [
        "not json",
        json.dumps({"number":456, "url":"https://github.example/team/project/pull/123"}),
        json.dumps({"number":123, "url":"file:///tmp/pr"}),
        json.dumps({"number":123, "url":"https://github.example/alice/fork/pull/123"}),
        json.dumps({"number":123, "url":""}),
    ])
    def test_invalid_pr_url_response_prevents_browser_launch(self, response):
        (self.root / "pr-url.json").write_text(response)
        directory = self.start()
        assert "GitHub PR" in self.select_pr_failure(directory)
        assert not (self.root / "open-calls").exists()
        assert not (self.root / "xdg-open-calls").exists()

    def test_launcher_failure_is_reported(self):
        launcher = {"darwin":"open", "linux":"xdg-open"}[sys.platform]
        self.env["PALETTE_TEST_FAIL"] = launcher
        directory = self.start()
        assert launcher + " unavailable" in self.select_pr_failure(directory)
        assert len(self.logged_calls(launcher)) == 1
