"""Pytest options shared by local integration and container snapshot tests."""

import os
from pathlib import Path
import subprocess

import pytest

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="session")
def release_binary():
    target = ROOT / os.environ.get("CARGO_TARGET_DIR", "target")
    subprocess.run(
        ["cargo", "build", "--locked", "--release", "--target-dir", str(target)],
        cwd=ROOT,
        check=True,
    )
    return target / "release/herdrpp"


def pytest_addoption(parser):
    group = parser.getgroup("sidebar snapshots")
    group.addoption(
        "--record", action="store_true", help="record sidebar baselines outside CI"
    )
    group.addoption("--repeat", type=int, default=1, help="repeat each snapshot case")
    group.addoption(
        "--snapshot-case", action="append", default=[], help="select a sidebar case"
    )


def pytest_configure(config):
    if config.getoption("--record") and os.environ.get("CI"):
        raise pytest.UsageError("Recording snapshots is disabled in CI")
    if config.getoption("--repeat") < 1:
        raise pytest.UsageError("--repeat must be positive")


def pytest_generate_tests(metafunc):
    if "snapshot_case" not in metafunc.fixturenames:
        return
    cases = metafunc.config.getoption("--snapshot-case") or list(metafunc.module.CASES)
    unknown = set(cases) - set(metafunc.module.CASES)
    if unknown:
        raise pytest.UsageError(f"unknown snapshot cases: {', '.join(sorted(unknown))}")
    metafunc.parametrize("snapshot_case", cases * metafunc.config.getoption("--repeat"))


@pytest.fixture
def record(request):
    return request.config.getoption("--record")
