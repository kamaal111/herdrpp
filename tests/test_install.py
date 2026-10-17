"""Test the real installer with fake network, build, and Herdr boundaries."""

import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import pytest

ROOT = Path(__file__).resolve().parents[1]
FAKE = r"""#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
root = pathlib.Path(os.environ["INSTALL_TEST_ROOT"])
kind = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with (root / "calls").open("a") as stream:
    stream.write(json.dumps([kind, args]) + "\n")
if kind == os.environ.get("FAIL_COMMAND"):
    sys.exit(17)
if kind == "curl":
    if args[-1].endswith("/releases/latest"):
        sys.stdout.write((root / "release.json").read_text())
    else:
        sys.stdout.buffer.write((root / "source.tar.gz").read_bytes())
elif kind == "cargo":
    target = pathlib.Path(args[args.index("--target-dir") + 1]) / "release/herdrpp"
    target.parent.mkdir(parents=True)
    shutil.copy2(os.environ["INSTALL_TEST_BINARY"], target)
elif kind == "herdr":
    if args[:2] == ["plugin", "list"]:
        print(json.dumps({"result": {"plugins": [{"plugin_id": "herdrpp", "plugin_root": str(root / "plugin")} ]}}))
    elif args == ["api", "snapshot"]:
        sys.exit(int(os.environ.get("NO_SESSION", "0")))
"""


class TestInstaller:
    @pytest.fixture(autouse=True)
    def setup(self, tmp_path, release_binary):
        self.binary = release_binary
        self.root = tmp_path / "installer space"
        self.root.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("herdr", "curl", "cargo", "git", "gh"):
            executable = self.bin / name
            executable.write_text(FAKE)
            executable.chmod(0o755)
        self.config = self.root / "config.toml"
        plugin_binary = self.root / "plugin/target/release/herdrpp"
        plugin_binary.parent.mkdir(parents=True)
        plugin_binary.symlink_to(self.binary)
        (self.root / "release.json").write_text(
            json.dumps({"tag_name": "v0.2.0", "draft": False, "prerelease": False})
        )
        with tarfile.open(self.root / "source.tar.gz", "w:gz") as archive:
            content = b"[package]\nname='test'\n"
            entry = tarfile.TarInfo("source/Cargo.toml")
            entry.size = len(content)
            archive.addfile(entry, io.BytesIO(content))
        self.env = dict(
            os.environ,
            PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
            INSTALL_TEST_ROOT=str(self.root),
            INSTALL_TEST_BINARY=str(self.binary),
            HERDR_BIN_PATH=str(self.bin / "herdr"),
            HOME=str(self.root),
            HERDR_CONFIG_PATH=str(self.config),
            XDG_STATE_HOME=str(self.root / "state"),
            TMPDIR=str(self.root),
        )

    def install(self, *args, piped=False):
        return subprocess.run(
            ["bash", "-s", "--", *args]
            if piped
            else ["bash", str(ROOT / "install.sh"), *args],
            input=(ROOT / "install.sh").read_text() if piped else None,
            env=self.env,
            text=True,
            capture_output=True,
            check=False,
        )

    def calls(self):
        return [
            json.loads(line) for line in (self.root / "calls").read_text().splitlines()
        ]

    def test_installer_shell_syntax(self):
        result = subprocess.run(
            ["bash", "-n", str(ROOT / "install.sh")],
            env=self.env,
            text=True,
            capture_output=True,
            check=False,
        )
        assert result.returncode == 0, result.stderr

    def test_combined_installs_exact_release_configures_and_starts(self):
        result = self.install()
        assert result.returncode == 0, result.stderr
        assert [
            "herdr",
            [
                "plugin",
                "install",
                "kamaal111/herdrpp",
                "--ref",
                "v0.2.0",
                "--yes",
            ],
        ] in self.calls()
        assert [
            "herdr",
            ["plugin", "action", "invoke", "herdrpp.start"],
        ] in self.calls()
        assert "$pr_open" in self.config.read_text()
        assert 'key = "cmd+shift+k"' in self.config.read_text()
        assert "Reload config" in result.stdout

    def test_rerun_keeps_user_config_and_resolves_release_again(self):
        assert self.install().returncode == 0
        self.config.write_text("# user deleted all tokens\n")
        before = self.config.stat().st_mtime_ns
        (self.root / "release.json").write_text('{"tag_name":"v0.2.1"}')
        assert self.install().returncode == 0
        assert self.config.read_text() == "# user deleted all tokens\n"
        assert self.config.stat().st_mtime_ns == before
        assert [
            "herdr",
            [
                "plugin",
                "install",
                "kamaal111/herdrpp",
                "--ref",
                "v0.2.1",
                "--yes",
            ],
        ] in self.calls()

    def test_plugin_only_never_touches_config(self):
        assert self.install("--plugin-only").returncode == 0
        assert not self.config.exists()
        assert not (self.root / "state").exists()

    def test_config_only_builds_without_registering_or_starting_plugin(self):
        result = self.install("--config-only")
        assert result.returncode == 0, result.stderr
        assert "$pr_open" in self.config.read_text()
        assert 'key = "cmd+shift+k"' in self.config.read_text()
        assert "herdr" not in [kind for kind, _ in self.calls()]
        assert list(self.root.glob("herdrpp-install.*")) == []

    def test_piped_execution(self):
        result = self.install(piped=True)
        assert result.returncode == 0, result.stderr
        assert "$pr_open" in self.config.read_text()
        assert 'key = "cmd+shift+k"' in self.config.read_text()

    def test_no_session_reports_startup_hook(self):
        self.env["NO_SESSION"] = "1"
        result = self.install()
        assert result.returncode == 0, result.stderr
        assert "startup hook" in result.stdout
        assert [
            "herdr",
            ["plugin", "action", "invoke", "herdrpp.start"],
        ] not in self.calls()

    def test_network_failure_never_touches_config_or_marks_success(self):
        self.env["FAIL_COMMAND"] = "curl"
        result = self.install()
        assert result.returncode != 0
        assert not self.config.exists()
        assert "installation completed" not in result.stdout

    def test_install_failure_never_touches_config(self):
        self.env["FAIL_COMMAND"] = "herdr"
        assert self.install().returncode != 0
        assert not self.config.exists()

    def test_build_failure_never_touches_config(self):
        self.env["FAIL_COMMAND"] = "cargo"
        assert self.install("--config-only").returncode != 0
        assert not self.config.exists()

    def test_unpublished_release_is_rejected(self):
        (self.root / "release.json").write_text(
            '{"tag_name":"v0.3.0", "prerelease":true}'
        )
        assert self.install().returncode != 0
        assert [kind for kind, _ in self.calls()] == ["curl"]

    def test_release_without_config_helper_reports_actionable_failure(self):
        binary = self.root / "plugin/target/release/herdrpp"
        binary.unlink()
        binary.write_text('#!/bin/sh\nprintf "old release help\\n"\n')
        binary.chmod(0o755)
        result = self.install()
        assert result.returncode != 0
        assert "does not include config setup" in result.stderr
        assert not self.config.exists()
        assert "installation completed" not in result.stdout

    def test_missing_dependency_fails_before_network_or_install(self):
        (self.bin / "cargo").unlink()
        self.env["PATH"] = str(self.bin) + ":/usr/bin:/bin"
        result = self.install()
        assert result.returncode != 0
        assert "install cargo" in result.stderr
        assert not (self.root / "calls").exists()

    def test_invalid_config_never_reports_completion(self):
        self.config.write_text("invalid = [")
        result = self.install()
        assert result.returncode != 0
        assert self.config.read_text() == "invalid = ["
        assert list((self.root / "state").rglob("migration.json")) == []
        assert "installation completed" not in result.stdout

    def test_help_has_no_effects(self):
        assert self.install("--help").returncode == 0
        assert not (self.root / "calls").exists()
        assert not self.config.exists()

    def test_dry_run_has_no_effects(self):
        assert self.install("--dry-run").returncode == 0
        assert not (self.root / "calls").exists()
        assert not self.config.exists()

    def test_conflicting_modes_are_rejected(self):
        assert self.install("--plugin-only", "--config-only").returncode != 0
        assert not (self.root / "calls").exists()
