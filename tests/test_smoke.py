"""Exercise the release binary with real Git and fake Herdr/GitHub boundaries."""

import fcntl
import json
import os
from pathlib import Path
import socket
import shutil
import subprocess
import tempfile
import time
import pytest


FAKE = r"""#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys, time
root = pathlib.Path(os.environ["PR_TEST_ROOT"])
args = sys.argv[1:]
kind = pathlib.Path(sys.argv[0]).name
with (root / "calls").open("a") as f:
    f.write(json.dumps([kind, args]) + "\n")
if kind == "herdr":
    if args == ["api", "snapshot"]:
        print((root / "snapshot.json").read_text())
    elif args == ["plugin", "list", "--json"]:
        print((root / "plugins.json").read_text())
    elif args == ["plugin", "pane", "open", "--plugin", "herdrpp", "--entrypoint", "picker"]:
        print('{"result": {"type": "ok"}}')
    elif args[:2] == ["workspace", "report-metadata"]:
        with (root / "reports").open("a") as f:
            f.write(json.dumps(args) + "\n")
        if (root / "report-fail").exists():
            print("metadata rejected", file=sys.stderr)
            sys.exit(1)
    else:
        sys.exit(11)
else:
    if (root / "fail").exists():
        print("GitHub unavailable", file=sys.stderr)
        sys.exit(1)
    if args[:2] == ["repo", "view"]:
        print((root / "repository.json").read_text() if (root / "repository.json").exists()
              else '{"nameWithOwner":"alice/project","url":"https://github.com/alice/project"}')
    elif args[:2] == ["pr", "view"]:
        if (root / "details-fail").exists():
            print("PR details unavailable", file=sys.stderr)
            sys.exit(1)
        if (root / "details.json").exists():
            print((root / "details.json").read_text())
        else:
            prs = json.loads((root / "prs.json").read_text())
            selected = next(pr for pr in prs if pr["number"] == int(args[2]))
            print(json.dumps({"number": selected["number"], "state": selected["state"].upper(),
                "isDraft": False, "statusCheckRollup": []}))
    elif args[0] == "api":
        if (root / "branch-switch").exists():
            subprocess.run(["git", "checkout", "-q", "other"], check=True)
        if (root / "blocked").exists():
            (root / "entered").touch()
            while not (root / "release").exists():
                time.sleep(.01)
        print((root / "prs.json").read_text())
    else:
        sys.exit(12)
"""


def eventually(assertion, timeout=8):
    deadline = time.monotonic() + timeout
    while True:
        try:
            assert assertion()
            return
        except AssertionError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.02)


def pr(
    number,
    date="2026-10-01T00:00:00Z",
    branch="feature",
    repo="alice/project",
    state="open",
):
    return {
        "number": number,
        "created_at": date,
        "state": state,
        "head": {"ref": branch, "repo": {"full_name": repo}},
    }


class TestPluginSmoke:
    @pytest.fixture(autouse=True)
    def setup(self, request, release_binary):
        self.request = request
        self.release_binary = release_binary
        self.binary = release_binary
        self.temp = tempfile.TemporaryDirectory(prefix="pr-smoke-", dir="/tmp")
        self.request.addfinalizer(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("herdr", "gh"):
            target = self.bin / name
            target.write_text(FAKE)
            target.chmod(0o755)
        self.env = dict(
            os.environ,
            PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
            HERDR_BIN_PATH=str(self.bin / "herdr"),
            PR_TEST_ROOT=str(self.root),
            HERDR_SOCKET_PATH=str(self.root / "socket"),
            HERDR_PLUGIN_STATE_DIR=str(self.root / "state"),
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_CONFIG_NOSYSTEM="1",
            GH_PROMPT_DISABLED="1",
        )
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.bind(self.env["HERDR_SOCKET_PATH"])
        self.request.addfinalizer(self.sock.close)
        self.repo = self.root / "project"
        self.git("init", "-q", "-b", "feature", str(self.repo), cwd=self.root)
        self.git(
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "initial",
        )
        self.git("remote", "add", "origin", "git@github.com:alice/project.git")
        self.git("branch", "other")
        self.write("prs.json", [pr(123)])
        self.write(
            "plugins.json",
            {
                "result": {
                    "plugins": [{"plugin_id": "herdrpp", "enabled": True}]
                }
            },
        )
        self.set_workspaces([self.workspace("w1", self.repo)])
        self.request.addfinalizer(self.stop_watcher)

    def git(self, *args, cwd=None):
        return subprocess.run(
            ["git", *args],
            cwd=cwd or self.repo,
            env=self.env,
            check=True,
            capture_output=True,
            text=True,
        )

    def write(self, name, value):
        pending = self.root / f".{name}.pending"
        pending.write_text(json.dumps(value))
        pending.replace(self.root / name)

    def workspace(self, identifier, path):
        return {
            "workspace_id": identifier,
            "active_tab_id": "t1",
            "tokens": {"unrelated": "keep"},
            "worktree": {"checkout_path": str(path)},
        }

    def set_workspaces(self, workspaces, panes=None, layouts=None):
        self.write(
            "snapshot.json",
            {
                "result": {
                    "type": "session_snapshot",
                    "snapshot": {
                        "workspaces": workspaces,
                        "panes": panes or [],
                        "layouts": layouts or [],
                    },
                }
            },
        )

    def run_plugin(self, action, check=True):
        result = subprocess.run(
            [str(self.binary), action],
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
            timeout=12,
        )
        if check:
            assert result.returncode == 0, (
                f"{action} failed: stdout={result.stdout!r}, stderr={result.stderr!r}"
            )
        return result

    def reports(self):
        return [
            json.loads(line)
            for line in (self.root / "reports").read_text().splitlines()
        ]

    def calls(self):
        return [
            json.loads(line) for line in (self.root / "calls").read_text().splitlines()
        ]

    def watcher_exited(self):
        assert (
            "PR watcher stopped"
            in next((self.root / "state").glob("*/watch.log")).read_text()
        )
        with next((self.root / "state").glob("*/watch.lock")).open("r+") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise AssertionError("PR watcher still holds the session lock")
        return True

    def wake_watcher(self, directory=None):
        directory = directory or next((self.root / "state").glob("session-*"))
        try:
            descriptor = os.open(directory / "wake.pipe", os.O_WRONLY | os.O_NONBLOCK)
            with os.fdopen(descriptor, "wb") as pipe:
                pipe.write(b"wake")
        except FileNotFoundError, BrokenPipeError:
            eventually(self.watcher_exited)

    def stop_watcher(self):
        (self.root / "release").touch()
        for directory in (self.root / "state").glob("session-*"):
            (directory / "stop").touch()
            try:
                descriptor = os.open(
                    directory / "wake.pipe", os.O_WRONLY | os.O_NONBLOCK
                )
            except OSError:
                continue
            with os.fdopen(descriptor, "wb") as pipe:
                pipe.write(b"stop")
        logs = list((self.root / "state").glob("*/watch.log"))
        if logs:
            eventually(self.watcher_exited, timeout=10)

    def test_newest_pr_in_any_state_and_scoped_metadata(self):
        self.write(
            "prs.json",
            [
                pr(123),
                pr(124, "2026-10-02T00:00:00Z", state="merged"),
                pr(125, "2026-10-03T00:00:00Z", state="closed"),
            ],
        )
        self.run_plugin("refresh")
        assert self.reports() == [
            [
                "workspace",
                "report-metadata",
                "w1",
                "--source",
                "herdrpp",
                "--token",
                "pr_number=#125",
                "--clear-token",
                "pr_open",
                "--clear-token",
                "pr_draft",
                "--clear-token",
                "pr_merged",
                "--token",
                "pr_closed=#125",
                "--ttl-ms",
                "180000",
            ]
        ]
        api_args = [
            args for kind, args in self.calls() if kind == "gh" and args[0] == "api"
        ][0]
        assert "state=all" in api_args
        assert "head=alice:feature" in api_args
        assert "--paginate" in api_args

    def test_details_are_scoped_to_selected_pr_base_repository_and_host(self):
        self.run_plugin("refresh")
        details_args = [
            args
            for kind, args in self.calls()
            if kind == "gh" and args[:2] == ["pr", "view"]
        ]
        assert details_args == [
            [
                "pr",
                "view",
                "123",
                "--repo",
                "github.com/alice/project",
                "--json",
                "number,state,isDraft,statusCheckRollup",
            ]
        ]

    def test_fork_pr_details_use_resolved_base_repository_on_enterprise_host(self):
        self.git("remote", "set-url", "origin", "git@github.example:alice/fork.git")
        self.write(
            "repository.json",
            {
                "nameWithOwner": "upstream/project",
                "url": "https://github.example/upstream/project",
            },
        )
        self.write("prs.json", [pr(123, repo="alice/fork")])
        self.run_plugin("refresh")
        details_args = [
            args
            for kind, args in self.calls()
            if kind == "gh" and args[:2] == ["pr", "view"]
        ][0]
        api_args = [
            args for kind, args in self.calls() if kind == "gh" and args[0] == "api"
        ][0]
        assert (
            details_args[details_args.index("--repo") + 1]
            == "github.example/upstream/project"
        )
        assert api_args[api_args.index("--hostname") + 1] == "github.example"
        assert "repos/upstream/project/pulls" in api_args
        assert "pr_open=#123" in self.reports()[0]

    def test_lifecycle_transitions_replace_tokens_and_ci_icons(self):
        self.write(
            "details.json",
            {
                "number": 123,
                "state": "OPEN",
                "isDraft": True,
                "statusCheckRollup": [
                    {"__typename": "CheckRun", "status": "QUEUED", "conclusion": ""}
                ],
            },
        )
        self.run_plugin("refresh")
        self.write(
            "details.json",
            {
                "number": 123,
                "state": "OPEN",
                "isDraft": False,
                "statusCheckRollup": [
                    {"__typename": "StatusContext", "state": "FAILURE"}
                ],
            },
        )
        self.run_plugin("refresh")
        self.write(
            "details.json",
            {
                "number": 123,
                "state": "MERGED",
                "isDraft": False,
                "statusCheckRollup": [
                    {
                        "__typename": "CheckRun",
                        "status": "COMPLETED",
                        "conclusion": "SUCCESS",
                    }
                ],
            },
        )
        self.run_plugin("refresh")
        assert "pr_draft=#123 🟡" in self.reports()[0]
        assert "pr_open=#123 ❌" in self.reports()[1]
        assert "pr_merged=#123" in self.reports()[2]
        assert "pr_merged=#123 ✅" not in self.reports()[2]
        assert [r[r.index("--clear-token") + 1] for r in self.reports()] == [
            "pr_open",
            "pr_draft",
            "pr_open",
        ]
        assert self.reports()[1][9:11] == ["--clear-token", "pr_draft"]
        assert self.reports()[2][9:11] == ["--clear-token", "pr_draft"]

    def test_empty_result_clears_all_owned_tokens(self):
        self.write("prs.json", [])
        self.run_plugin("refresh")
        assert self.reports() == [
            [
                "workspace",
                "report-metadata",
                "w1",
                "--source",
                "herdrpp",
                "--clear-token",
                "pr_open",
                "--clear-token",
                "pr_draft",
                "--clear-token",
                "pr_merged",
                "--clear-token",
                "pr_closed",
                "--clear-token",
                "pr_number",
            ]
        ]
        assert [
            args
            for kind, args in self.calls()
            if kind == "gh" and args[:2] == ["pr", "view"]
        ] == []

    def test_details_failure_preserves_all_metadata_then_recovers(self):
        self.run_plugin("refresh")
        previous = self.reports()
        (self.root / "details-fail").touch()
        result = self.run_plugin("refresh", check=False)
        assert result.returncode == 1
        assert "PR details unavailable" in result.stderr
        assert self.reports() == previous
        (self.root / "details-fail").unlink()
        self.run_plugin("refresh")
        assert self.reports() == previous + previous

    def test_malformed_details_preserve_all_metadata(self):
        self.write("details.json", {"number": 123})
        result = self.run_plugin("refresh", check=False)
        assert result.returncode == 1
        assert not (self.root / "reports").exists()

    def test_metadata_command_failure_is_reported(self):
        (self.root / "report-fail").touch()
        result = self.run_plugin("refresh", check=False)
        assert result.returncode == 1
        assert "metadata rejected" in result.stderr

    def test_empty_result_clears_only_owned_token(self):
        self.write("prs.json", [])
        self.run_plugin("refresh")
        assert self.reports()[0][-2:] == ["--clear-token", "pr_number"]

    def test_plain_directory_skips_github_and_clears(self):
        self.set_workspaces([self.workspace("w1", self.root)])
        self.run_plugin("refresh")
        assert self.reports()[0][-2:] == ["--clear-token", "pr_number"]
        assert [kind for kind, _ in self.calls() if kind == "gh"] == []

    def test_detached_head_skips_github(self):
        self.git("checkout", "-q", "--detach")
        self.run_plugin("refresh")
        assert self.reports()[0][-2:] == ["--clear-token", "pr_number"]
        assert [kind for kind, _ in self.calls() if kind == "gh"] == []

    def test_linked_worktree_has_independent_branch(self):
        linked = self.root / "linked"
        self.git("worktree", "add", "-q", str(linked), "other")
        self.set_workspaces(
            [self.workspace("w1", self.repo), self.workspace("w2", linked)]
        )
        self.write("prs.json", [pr(123), pr(456, branch="other")])
        self.run_plugin("refresh")
        assert [r[6] for r in self.reports()] == ["pr_number=#123", "pr_number=#456"]

    def test_duplicate_checkout_is_looked_up_once(self):
        self.set_workspaces(
            [self.workspace("w1", self.repo), self.workspace("w2", self.repo)]
        )
        self.run_plugin("refresh")
        assert (
            len(
                [
                    args
                    for kind, args in self.calls()
                    if kind == "gh" and args[0] == "api"
                ]
            )
            == 1
        )
        assert len(self.reports()) == 2
        assert (
            len(
                [
                    args
                    for kind, args in self.calls()
                    if kind == "gh" and args[:2] == ["pr", "view"]
                ]
            )
            == 1
        )

    def test_branch_change_during_lookup_clears_stale_number(self):
        (self.root / "branch-switch").touch()
        self.run_plugin("refresh")
        assert self.reports()[0][-2:] == ["--clear-token", "pr_number"]

    def test_lookup_failure_preserves_previous_token_then_recovers(self):
        (self.root / "fail").touch()
        result = self.run_plugin("refresh", check=False)
        assert result.returncode == 1
        assert "GitHub unavailable" in result.stderr
        assert not (self.root / "reports").exists()
        (self.root / "fail").unlink()
        self.run_plugin("refresh")
        assert self.reports()[0][6] == "pr_number=#123"

    def test_malformed_snapshot_does_not_report(self):
        (self.root / "snapshot.json").write_text("not-json")
        result = self.run_plugin("refresh", check=False)
        assert result.returncode == 1
        assert not (self.root / "reports").exists()

    def test_check_does_not_write_state_or_metadata(self):
        result = self.run_plugin("check-pr")
        assert "w1: #123" in result.stdout
        assert not (self.root / "reports").exists()
        assert not (self.root / "state").exists()

    def test_active_pane_directory_fallback(self):
        self.set_workspaces(
            [{"workspace_id": "w1", "active_tab_id": "t1"}],
            [
                {
                    "pane_id": "p1",
                    "workspace_id": "w1",
                    "tab_id": "t1",
                    "cwd": str(self.repo),
                }
            ],
            [{"workspace_id": "w1", "tab_id": "t1", "focused_pane_id": "p1"}],
        )
        self.run_plugin("refresh")
        assert self.reports()[0][6] == "pr_number=#123"

    def test_duplicate_workspace_ids_are_rejected(self):
        self.set_workspaces(
            [self.workspace("w1", self.repo), self.workspace("w1", self.repo)]
        )
        result = self.run_plugin("refresh", check=False)
        assert result.returncode == 1
        assert "duplicate workspace ID" in result.stderr

    def test_refresh_does_not_overlap(self):
        (self.root / "blocked").touch()
        child = subprocess.Popen(
            [str(self.release_binary), "refresh"],
            env=self.env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.request.addfinalizer(child.wait)
        self.request.addfinalizer(lambda: (self.root / "release").touch())
        eventually(lambda: (self.root / "entered").exists())
        result = self.run_plugin("refresh")
        assert "already in progress" in result.stdout
        (self.root / "release").touch()
        child.communicate(timeout=8)
        assert child.returncode == 0
        assert len(self.reports()) == 1

    def test_watcher_is_singleton_and_explicitly_stops(self):
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        result = self.run_plugin("start")
        assert "already running" in result.stdout
        self.run_plugin("stop")
        eventually(self.watcher_exited)
        assert len(self.reports()) == 1
        assert (
            not next((self.root / "state").glob("session-*"))
            .joinpath("wake.pipe")
            .exists()
        )
        assert (
            not next((self.root / "state").glob("session-*"))
            .joinpath("watcher.json")
            .exists()
        )

    def test_start_acknowledges_locked_watcher_before_first_lookup_finishes(self):
        (self.root / "blocked").touch()
        result = self.run_plugin("start")
        assert "PR watcher started" in result.stdout
        eventually(lambda: (self.root / "entered").exists())
        with next((self.root / "state").glob("*/watch.lock")).open("r+") as lock:
            with pytest.raises(BlockingIOError):
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        assert "already running" in self.run_plugin("start").stdout
        self.run_plugin("stop")
        (self.root / "release").touch()
        eventually(self.watcher_exited)
        assert not (self.root / "reports").exists()

    def test_start_replaces_legacy_watcher_without_identity(self):
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        (next((self.root / "state").glob("session-*")) / "watcher.json").unlink(
            missing_ok=True
        )
        result = self.run_plugin("start")
        assert "Replacing outdated PR watcher" in result.stdout
        eventually(lambda: len(self.reports()) == 2)
        self.run_plugin("stop")
        eventually(
            lambda: (
                next((self.root / "state").glob("*/watch.log"))
                .read_text()
                .count("PR watcher stopped")
                == 2
            )
        )

    def test_start_replaces_watcher_from_another_checkout(self):
        self.binary = self.root / "first-plugin"
        shutil.copy2(self.release_binary, self.binary)
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        self.binary = self.root / "second-plugin"
        shutil.copy2(self.release_binary, self.binary)
        result = self.run_plugin("start")
        assert "Replacing outdated PR watcher" in result.stdout
        eventually(lambda: len(self.reports()) == 2)
        assert "already running" in self.run_plugin("start").stdout

    def test_watcher_reloads_replaced_binary_without_start_or_refresh(self):
        self.binary = self.root / "installed-plugin"
        shutil.copy2(self.release_binary, self.binary)
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        self.write("prs.json", [pr(987)])
        replacement = self.root / "replacement-plugin"
        shutil.copy2(self.release_binary, replacement)
        replacement.replace(self.binary)
        self.wake_watcher()
        eventually(lambda: len(self.reports()) == 2, timeout=12)
        assert "pr_open=#987" in self.reports()[1]
        assert (
            "PR watcher binary changed; reloading"
            in next((self.root / "state").glob("*/watch.log")).read_text()
        )
        assert "already running" in self.run_plugin("start").stdout

    def test_watcher_follows_registered_update_after_old_checkout_is_removed(self):
        old_root = self.root / "old-plugin"
        self.binary = old_root / "target/release/herdrpp"
        self.binary.parent.mkdir(parents=True)
        shutil.copy2(self.release_binary, self.binary)
        self.write(
            "plugins.json",
            {
                "result": {
                    "plugins": [
                        {
                            "plugin_id": "herdrpp",
                            "enabled": True,
                            "plugin_root": str(old_root),
                        }
                    ]
                }
            },
        )
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        new_root = self.root / "new-plugin"
        self.binary = new_root / "target/release/herdrpp"
        self.binary.parent.mkdir(parents=True)
        shutil.copy2(self.release_binary, self.binary)
        self.write("prs.json", [pr(987)])
        self.write(
            "plugins.json",
            {
                "result": {
                    "plugins": [
                        {
                            "plugin_id": "herdrpp",
                            "enabled": True,
                            "plugin_root": str(new_root),
                        }
                    ]
                }
            },
        )
        shutil.rmtree(old_root)
        self.wake_watcher()
        eventually(lambda: len(self.reports()) == 2, timeout=12)
        assert "pr_open=#987" in self.reports()[1]
        assert "already running" in self.run_plugin("start").stdout

    def test_watcher_keeps_running_after_failed_reload_and_recovers(self):
        self.binary = self.root / "installed-plugin"
        shutil.copy2(self.release_binary, self.binary)
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        replacement = self.root / "replacement-plugin"
        shutil.copy2(self.release_binary, replacement)
        replacement.chmod(0o600)
        replacement.replace(self.binary)
        self.wake_watcher()
        log = next((self.root / "state").glob("*/watch.log"))
        eventually(
            lambda: "update deferred; current watcher retained" in log.read_text(),
            timeout=12,
        )
        assert "PR watcher stopped" not in log.read_text()
        self.write("prs.json", [pr(987)])
        self.binary.chmod(0o700)
        self.wake_watcher()
        eventually(lambda: len(self.reports()) == 2, timeout=12)
        assert "pr_open=#987" in self.reports()[1]
        assert "already running" in self.run_plugin("start").stdout

    def test_stop_during_binary_update_does_not_reload_or_report(self):
        self.binary = self.root / "installed-plugin"
        shutil.copy2(self.release_binary, self.binary)
        (self.root / "blocked").touch()
        self.run_plugin("start")
        eventually(lambda: (self.root / "entered").exists())
        replacement = self.root / "replacement-plugin"
        shutil.copy2(self.release_binary, replacement)
        replacement.replace(self.binary)
        self.run_plugin("stop")
        (self.root / "release").touch()
        eventually(self.watcher_exited)
        assert not (self.root / "reports").exists()
        assert (
            next((self.root / "state").glob("*/watch.log"))
            .read_text()
            .count("PR watcher started")
            == 1
        )

    def test_watcher_exits_on_plugin_disable(self):
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        self.write(
            "plugins.json",
            {
                "result": {
                    "plugins": [{"plugin_id": "herdrpp", "enabled": False}]
                }
            },
        )
        self.wake_watcher()
        eventually(self.watcher_exited)
        assert len(self.reports()) == 1

    def test_disable_during_lookup_stops_before_next_workspace(self):
        linked = self.root / "linked"
        self.git("worktree", "add", "-q", str(linked), "other")
        self.set_workspaces(
            [self.workspace("w1", self.repo), self.workspace("w2", linked)]
        )
        (self.root / "blocked").touch()
        self.run_plugin("start")
        eventually(lambda: (self.root / "entered").exists())
        self.write("plugins.json", {"result": {"plugins": []}})
        (self.root / "release").touch()
        eventually(self.watcher_exited)
        assert not (self.root / "reports").exists()
        assert (
            len(
                [
                    args
                    for kind, args in self.calls()
                    if kind == "gh" and args[0] == "api"
                ]
            )
            == 1
        )

    def test_watcher_exits_on_plugin_unlink(self):
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        self.write("plugins.json", {"result": {"plugins": []}})
        self.wake_watcher()
        eventually(self.watcher_exited)
        assert len(self.reports()) == 1

    def test_watcher_refreshes_again_without_manual_action(self):
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        self.write("prs.json", [pr(987)])
        eventually(lambda: len(self.reports()) == 2, timeout=75)
        assert self.reports()[1][6] == "pr_number=#987"
        calls = self.calls()
        report_positions = [
            index
            for index, (kind, args) in enumerate(calls)
            if kind == "herdr" and args[:2] == ["workspace", "report-metadata"]
        ]
        assert (
            len(
                [
                    args
                    for kind, args in calls[: report_positions[1]]
                    if kind == "gh" and args[0] == "api"
                ]
            )
            == 2
        )
        self.run_plugin("stop")
        eventually(self.watcher_exited)

    def test_watcher_exits_when_socket_is_replaced(self):
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        Path(self.env["HERDR_SOCKET_PATH"]).unlink()
        replacement = socket.socket(socket.AF_UNIX)
        self.request.addfinalizer(replacement.close)
        replacement.bind(self.env["HERDR_SOCKET_PATH"])
        self.wake_watcher()
        eventually(self.watcher_exited)
        assert len(self.reports()) == 1

    def test_watcher_exits_on_session_shutdown(self):
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        Path(self.env["HERDR_SOCKET_PATH"]).unlink()
        self.wake_watcher()
        eventually(self.watcher_exited)
        assert len(self.reports()) == 1

    def test_watcher_can_restart_immediately_after_stop(self):
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        self.run_plugin("stop")
        self.run_plugin("start")
        eventually(lambda: len(self.reports()) == 2)
        self.run_plugin("stop")
        eventually(
            lambda: (
                next((self.root / "state").glob("*/watch.log"))
                .read_text()
                .count("PR watcher stopped")
                == 2
            )
        )
        assert len(self.reports()) == 2

    def test_malformed_github_response_is_not_treated_as_no_pr(self):
        (self.root / "prs.json").write_text("invalid")
        result = self.run_plugin("refresh", check=False)
        assert result.returncode == 1
        assert not (self.root / "reports").exists()

    def test_upstream_branch_name_is_used_for_github(self):
        self.git("config", "branch.feature.remote", "origin")
        self.git("config", "branch.feature.merge", "refs/heads/remote-feature")
        self.write("prs.json", [pr(333, branch="remote-feature")])
        self.run_plugin("refresh")
        assert self.reports()[0][6] == "pr_number=#333"
        assert (
            "head=alice:remote-feature"
            in [
                args for kind, args in self.calls() if kind == "gh" and args[0] == "api"
            ][0]
        )

    def test_disabled_plugin_cannot_start(self):
        self.write("plugins.json", {"result": {"plugins": []}})
        result = self.run_plugin("start", check=False)
        assert result.returncode == 1
        assert not list((self.root / "state").glob("*/watch.log"))

    def test_picker_opens_and_cancels_while_pr_watcher_remains_running(self):
        import pty
        import struct
        import termios
        from pty_smoke import read_until
        self.write("snapshot.json", {"result": {"type": "session_snapshot", "snapshot": {
            "workspaces": [dict(self.workspace("w1", self.repo), label="demo", number=1)],
            "tabs": [{"tab_id": "t1", "workspace_id": "w1", "label": "Codex", "number": 1}],
            "panes": [], "layouts": [], "focused_workspace_id": "w1", "focused_tab_id": "t1"
        }}})
        self.run_plugin("start")
        eventually(lambda: (self.root / "reports").exists())
        assert self.run_plugin("open").returncode == 0
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 80, 0, 0))
        self.request.addfinalizer(lambda: os.close(master))
        self.request.addfinalizer(lambda: os.close(slave))
        picker = subprocess.Popen([self.binary, "pick"], stdin=slave, stdout=slave, stderr=slave,
                                  env=dict(self.env, HERDR_PLUGIN_CONFIG_DIR=str(self.root),
                                           HERDR_CONFIG_PATH=str(self.root / "herdr.toml"), TERM="xterm-256color"))
        self.request.addfinalizer(lambda: picker.poll() is not None or picker.wait(timeout=8))
        self.request.addfinalizer(lambda: picker.poll() is not None or picker.kill())
        assert b"results" in read_until(master, b"results")
        os.write(master, b"\x1b")
        assert picker.wait(timeout=8) == 0
        assert "already running" in self.run_plugin("start").stdout
        assert len(self.reports()) == 1
        assert ["herdr", ["tab", "focus", "t1"]] not in self.calls()
        self.run_plugin("stop")
        eventually(self.watcher_exited)

    def test_picker_discovery_does_not_invoke_github_or_write_pr_metadata(self):
        self.write("snapshot.json", {"result": {"type": "session_snapshot", "snapshot": {
            "workspaces": [dict(self.workspace("w1", self.repo), label="demo", number=1)],
            "tabs": [{"tab_id": "t1", "workspace_id": "w1", "label": "Codex", "number": 1}],
            "panes": [], "layouts": []
        }}})
        (self.bin / "gh").unlink()
        result = self.run_plugin("check")
        assert "1 workspaces, 1 tabs" in result.stdout
        assert [kind for kind, _ in self.calls() if kind == "gh"] == []
        assert not (self.root / "reports").exists()
