"""The public CLI validates arguments before contacting Herdr or changing state."""

import os
import subprocess
from pathlib import Path
import tomllib

import pytest


@pytest.fixture
def cli(tmp_path, release_binary):
    log = tmp_path / "calls"
    fake = tmp_path / "herdr"
    fake.write_text(f'#!/bin/sh\ntouch "{log}"\nexit 1\n')
    fake.chmod(0o700)
    environment = dict(os.environ, HERDR_BIN_PATH=str(fake), HOME=str(tmp_path),
                       XDG_STATE_HOME=str(tmp_path / "state"))

    def run(*args):
        return subprocess.run([release_binary, *args], env=environment,
                              capture_output=True, text=True, timeout=5)

    return run, log, tmp_path / "state"


@pytest.mark.parametrize("args", [("open", "extra"), ("commands", "extra"), ("pick-command", "extra"), ("configure", "extra"),
                                  ("--version", "extra"), ("unknown",)])
def test_invalid_arguments_have_no_herdr_or_configuration_side_effects(cli, args):
    run, log, state = cli
    result = run(*args)
    assert result.returncode == 1
    assert "use --help" in result.stderr
    assert not log.exists()
    assert not state.exists()


@pytest.mark.parametrize("args", [(), ("--help",), ("-h",)])
def test_help_lists_both_features_without_a_session(cli, args):
    run, log, state = cli
    result = run(*args)
    assert result.returncode == 0
    assert "Open command palette" in result.stdout
    assert "Search workspaces, worktrees and tabs" in result.stdout
    assert "Refresh workspace PR metadata once" in result.stdout
    assert not log.exists()
    assert not state.exists()


def test_version_is_available_without_a_session(cli):
    run, log, state = cli
    result = run("--version")
    assert result.returncode == 0
    version = tomllib.loads((Path(__file__).resolve().parents[1] / "Cargo.toml").read_text())["package"]["version"]
    assert result.stdout == f"herdrpp {version}\n"
    assert not log.exists()
    assert not state.exists()
