"""Real Herdr sidebar PNG regressions with independent server/client configs."""

import codecs
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import time

import pytest
import pyte
from PIL import Image, ImageChops, ImageDraw, ImageFont
from snapshot_actions import completed_action, stop_startup_watcher

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = Path("/output")
BASELINES = Path("/baselines")
FONTS = ROOT / "tests/snapshots/fonts"
CASES = (
    "grouped-worktree-pr",
    "client-config-without-token",
    "client-config-with-token",
    "client-config-reload",
    "cleared-pr",
    "pr-draft-pending",
    "pr-open-failed",
    "pr-merged-passed",
    "pr-closed-no-checks",
    "ci-passed",
    "ci-failed",
    "ci-pending",
    "cleared-ci",
)
PR_CASES = {
    "pr-draft-pending": ("OPEN", True, "PENDING", "8b949e", "🟡"),
    "pr-open-failed": ("OPEN", False, "FAILURE", "238636", "❌"),
    "pr-merged-passed": ("MERGED", False, "SUCCESS", "a371f7", ""),
    "pr-closed-no-checks": ("CLOSED", False, None, "f85149", ""),
}
ANSI = dict(
    zip(
        (
            "black",
            "red",
            "green",
            "brown",
            "blue",
            "magenta",
            "cyan",
            "white",
            "brightblack",
            "brightred",
            "brightgreen",
            "brightbrown",
            "brightblue",
            "brightmagenta",
            "brightcyan",
            "brightwhite",
        ),
        (
            "21222c",
            "ff5555",
            "50fa7b",
            "f1fa8c",
            "bd93f9",
            "ff79c6",
            "8be9fd",
            "f8f8f2",
            "6272a4",
            "ff6e6e",
            "69ff94",
            "ffffa5",
            "d6acff",
            "ff92df",
            "a4ffff",
            "ffffff",
        ),
    )
)


def color(value, default):
    if value in ("default", "Reset"):
        return default
    rgb = re.fullmatch(r"Rgb\((\d+), (\d+), (\d+)\)", value)
    if rgb:
        return tuple(map(int, rgb.groups()))
    value = ANSI.get(value, value)
    return tuple(bytes.fromhex(value))


def rasterize(grid, light=False):
    foreground = (76, 79, 105) if light else (248, 248, 242)
    background = (239, 241, 245) if light else (40, 42, 54)
    image = Image.new("RGB", (grid["width"] * 10, grid["height"] * 20), background)
    draw = ImageDraw.Draw(image)
    regular = ImageFont.truetype(str(FONTS / "DejaVuSansMono.ttf"), 16)
    bold = ImageFont.truetype(str(FONTS / "DejaVuSansMono-Bold.ttf"), 16)
    emoji = ImageFont.truetype(str(FONTS / "NotoEmoji.ttf"), 16)
    for index, cell in enumerate(grid["cells"]):
        x, y = index % grid["width"] * 10, index // grid["width"] * 20
        fg, bg = color(cell["fg"], foreground), color(cell["bg"], background)
        if cell["reverse"]:
            fg, bg = bg, fg
        draw.rectangle((x, y, x + 9, y + 19), fill=bg)
    for index, cell in enumerate(grid["cells"]):
        x, y = index % grid["width"] * 10, index // grid["width"] * 20
        fg = (
            color(cell["bg"], background)
            if cell["reverse"]
            else color(cell["fg"], foreground)
        )
        font = (
            emoji
            if cell["text"] in ("✅", "❌", "🟡")
            else bold
            if cell["bold"]
            else regular
        )
        draw.text((x, y + 1), cell["text"], font=font, fill=fg)
        if cell["underline"]:
            draw.line((x, y + 18, x + 9, y + 18), fill=fg)
    return image


def compare(name, actual, record=False, baselines=BASELINES, output=OUTPUT):
    directory = output / name
    directory.mkdir(parents=True, exist_ok=True)
    actual.save(directory / "actual.png")
    baseline = baselines / f"{name}.png"
    if record:
        if os.environ.get("CI"):
            raise RuntimeError("Recording snapshots is disabled in CI")
        actual.save(baseline)
        return True
    if not baseline.exists():
        print(
            f"FAIL {name}: missing baseline; run uv run --locked pytest "
            f"tests/test_sidebar_snapshots.py --record --snapshot-case {name}"
        )
        return False
    with Image.open(baseline) as reference:
        expected = reference.convert("RGB")
    expected.save(directory / "expected.png")
    if expected.size != actual.size:
        size = tuple(max(a, b) for a, b in zip(expected.size, actual.size))
        left, right = Image.new("RGB", size), Image.new("RGB", size)
        left.paste(expected, (0, 0))
        right.paste(actual, (0, 0))
        difference = ImageChops.difference(left, right)
    else:
        difference = ImageChops.difference(expected, actual)
    if difference.getbbox() is None and expected.size == actual.size:
        return True
    difference.save(directory / "difference.png")
    print(f"FAIL {name}: pixels changed; see {directory}")
    return False


class SnapshotScreen(pyte.Screen):
    def report_device_status(self, mode, **kwargs):
        if not kwargs.get("private"):
            super().report_device_status(mode)


class Client:
    def __init__(self, environment, width, height, directory):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        self.screen = SnapshotScreen(width, height)
        self.screen.write_process_input = lambda text: os.write(
            self.master, text.encode()
        )
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.raw = bytearray()
        self.process = subprocess.Popen(
            ["herdr"],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=environment,
            cwd=directory,
            start_new_session=True,
        )
        os.close(slave)

    def read(self):
        if select.select([self.master], [], [], 0.05)[0]:
            try:
                data = os.read(self.master, 1_048_576)
            except OSError as error:
                if error.errno == errno.EIO:
                    raise RuntimeError("Herdr client closed unexpectedly") from error
                raise
            if not data:
                raise RuntimeError("Herdr client output ended")
            self.raw.extend(data)
            self.stream.feed(self.decoder.decode(data))

    def grid(self):
        cells = []
        for y in range(self.screen.lines):
            for x in range(self.screen.columns):
                char = self.screen.buffer[y][x]
                cells.append(
                    dict(
                        text=char.data,
                        fg=char.fg,
                        bg=char.bg,
                        bold=char.bold,
                        underline=char.underscore,
                        reverse=char.reverse,
                    )
                )
        return dict(width=self.screen.columns, height=self.screen.lines, cells=cells)

    def wait(self, *needles, timeout=15):
        deadline = time.monotonic() + timeout
        previous, stable_since = None, time.monotonic()
        while time.monotonic() < deadline:
            self.read()
            grid = self.grid()
            text = "\n".join(self.screen.display)
            if grid != previous:
                previous, stable_since = grid, time.monotonic()
            if (
                all(needle in text for needle in needles)
                and time.monotonic() - stable_since >= 0.5
            ):
                return grid
            if self.process.poll() is not None:
                raise RuntimeError(f"Herdr client exited: {self.process.returncode}")
        raise TimeoutError(f"Screen did not settle with {needles!r}:\n{text}")

    def send(self, keys):
        os.write(self.master, keys)

    def close(self):
        try:
            if self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGTERM)
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(self.process.pid, signal.SIGKILL)
                    self.process.wait(timeout=5)
        finally:
            os.close(self.master)


class Session:
    def __init__(self, directory, case):
        self.directory = directory
        self.case = case
        self.server = self.client = self.log = None
        self.environment = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith(("HERDR_", "XDG_", "GIT_", "GH_"))
            and k not in ("ENV", "BASH_ENV", "NO_COLOR", "COLORTERM")
        }
        self.environment.update(
            XDG_CONFIG_HOME=str(directory / "config"),
            XDG_STATE_HOME=str(directory / "state"),
            XDG_DATA_HOME=str(directory / "data"),
            HERDR_CONFIG_PATH=str(directory / "config/herdr/config.toml"),
            SHELL="/bin/sh",
            PS1="$ ",
            PS2="> ",
            ENV="/dev/null",
            TERM="xterm-256color",
            COLORTERM="truecolor",
            LANG="C.UTF-8",
            LC_ALL="C.UTF-8",
            TZ="UTC",
            PATH=str(directory / "bin") + os.pathsep + os.environ["PATH"],
        )

    def run(self, *args):
        result = subprocess.run(
            args,
            env=self.environment,
            cwd=self.directory,
            text=True,
            capture_output=True,
            timeout=30,
        )
        if result.returncode:
            raise RuntimeError(f"{args!r}: {result.stderr}{result.stdout}")
        return result.stdout

    def api(self, *args):
        return json.loads(self.run("herdr", "--session", "snapshot", *args))["result"]

    def action(self, action):
        return completed_action(self.api, action)

    def refresh(self):
        completed = self.action("refresh")
        assert "already in progress" not in (completed.get("stdout") or ""), completed

    def stop_startup_watcher(self):
        stop_startup_watcher(self.api, self.directory / "state/herdr/plugins/herdrpp")

    def wait_api(self, predicate, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            result = self.api("api", "snapshot")["snapshot"]
            if predicate(result):
                return result
            time.sleep(0.05)
        raise TimeoutError(f"Expected API state not reached: {result}")

    def setup(self):
        config = Path(self.environment["HERDR_CONFIG_PATH"])
        config.parent.mkdir(parents=True)
        self.base_config = """onboarding = false
[theme]
name = "dracula"
[terminal]
default_shell = "/bin/sh"
shell_mode = "non_login"
[update]
version_check = false
manifest_check = false
[ui]
sidebar_width = 30
host_cursor = "native"
[ui.toast]
delivery = "off"
[ui.sound]
enabled = false
"""
        self.sidebar_config = (ROOT / "examples/herdr.toml").read_text()
        config.write_text(self.base_config + self.sidebar_config)
        fake = self.directory / "bin/gh"
        fake.parent.mkdir()
        fake.write_text("""#!/usr/bin/env python3
import json,sys,pathlib
if sys.argv[1:3] == ["repo","view"]:
    print(json.dumps({"nameWithOwner":"snapshot/TCG","url":"https://github.com/snapshot/TCG","defaultBranchRef":{"name":"main"}}))
elif sys.argv[1:3] == ["api","graphql"]:
    response = "details.json" if any("pullRequest(number:" in arg for arg in sys.argv) else "history.json"
    print(pathlib.Path(__file__).with_name(response).read_text())
else:
    print(json.dumps([{"number":95,"created_at":"2026-10-01T00:00:00Z","head":{"ref":"export-import","repo":{"full_name":"snapshot/TCG"}}}] if "head=snapshot:export-import" in sys.argv and not pathlib.Path(__file__).with_name("empty").exists() else []))
""")
        state, draft, ci, _, _ = PR_CASES.get(
            self.case, ("OPEN", False, None, "238636", "")
        )
        checks = [] if ci is None else [{"__typename": "StatusContext", "state": ci}]
        if ci == "PENDING":
            checks = [
                {"__typename": "CheckRun", "status": "IN_PROGRESS", "conclusion": ""}
            ]
        (fake.parent / "details.json").write_text(
            json.dumps(
                {"data": {"repository": {"pullRequest": {
                    "number": 95, "state": state, "isDraft": draft,
                    "commits": {"nodes": [{"commit": {
                        "oid": "a" * 40,
                        "statusCheckRollup": {"contexts": {
                            "nodes": checks,
                            "pageInfo": {"hasNextPage": False, "endCursor": None}
                        }}
                    }}]}
                }}}}
            )
        )
        ci_state = {"ci-passed": "SUCCESS", "ci-failed": "FAILURE", "ci-pending": "PENDING", "cleared-ci": "SUCCESS"}.get(self.case)
        self.ci_case = ci_state is not None
        self.ci_token = {"SUCCESS": "ci_passed", "FAILURE": "ci_failed", "PENDING": "ci_pending"}.get(ci_state)
        self.ci_icon = {"SUCCESS": "✅", "FAILURE": "❌", "PENDING": "🟡"}.get(ci_state)
        self.history_file = fake.parent / "history.json"
        self.history_file.write_text(json.dumps({"data": {"repository": {"ref": {"target": {"history": {"nodes": [
            {"oid": "a" * 40, "statusCheckRollup": {"contexts": {"nodes":
                [{"__typename": "StatusContext", "state": ci_state}] if self.ci_case else [],
                "pageInfo": {"hasNextPage": False, "endCursor": None}}}}
        ]}}}}}}))
        fake.chmod(0o755)
        plugin = self.directory / "plugin"
        (plugin / "target/release").mkdir(parents=True)
        shutil.copy(ROOT / "herdr-plugin.toml", plugin)
        shutil.copy(OUTPUT / "build/release/herdrpp", plugin / "target/release")
        self.run("herdr", "plugin", "link", str(plugin))
        self.log = (self.directory / "server.log").open("wb")
        self.server = subprocess.Popen(
            ["herdr", "--session", "snapshot", "server"],
            env=self.environment,
            cwd=self.directory,
            stdout=self.log,
            stderr=self.log,
            start_new_session=True,
        )
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                self.api("api", "snapshot")
                break
            except RuntimeError:
                time.sleep(0.05)
        else:
            raise TimeoutError("Herdr server did not start")
        self.stop_startup_watcher()
        original = self.api("api", "snapshot")["snapshot"]["workspaces"]
        repo = self.directory / "TCG"
        self.run("git", "init", "-b", "main", str(repo))
        self.run(
            "git",
            "-C",
            str(repo),
            "-c",
            "user.name=Snapshot",
            "-c",
            "user.email=snapshot@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Initial",
        )
        self.run(
            "git",
            "-C",
            str(repo),
            "remote",
            "add",
            "origin",
            "https://github.com/snapshot/TCG.git",
        )
        root = self.api(
            "workspace", "create", "--cwd", str(repo), "--label", "TCG", "--no-focus"
        )["workspace"]["workspace_id"]
        for branch in ("export-import", "no-pr"):
            path = self.directory / "worktrees" / branch
            self.run("git", "-C", str(repo), "worktree", "add", "-b", branch, str(path))
            self.api(
                "worktree",
                "open",
                "--workspace",
                root,
                "--path",
                str(path),
                "--label",
                branch,
                "--no-focus",
            )
        self.api("workspace", "focus", root)
        for workspace in original:
            self.api("workspace", "close", workspace["workspace_id"])
        socket_path = next(
            line.removeprefix("socket: ")
            for line in self.run(
                "herdr", "--session", "snapshot", "status", "server"
            ).splitlines()
            if line.startswith("socket: ")
        )
        client_environment = dict(self.environment, HERDR_SOCKET_PATH=socket_path)
        self.client_config = self.directory / "client-config.toml"
        client_has_token = self.case not in (
            "client-config-without-token",
            "client-config-reload",
        )
        self.client_config.write_text(
            self.base_config + (self.sidebar_config if client_has_token else "")
        )
        client_environment["HERDR_CONFIG_PATH"] = str(self.client_config)
        self.client = Client(client_environment, 120, 30, self.directory)
        self.client.wait("TCG", "no-pr", "$")
        self.refresh()
        self.wait_api(
            lambda s: any(
                (w.get("tokens", {}).get("pr_number") == "#95" for w in s["workspaces"])
            )
        )

    def capture(self):
        if self.ci_case:
            self.wait_api(lambda s: any(w.get("tokens", {}).get(self.ci_token) == f"CI {self.ci_icon}" for w in s["workspaces"]))
            grid = self.client.wait(f"CI {self.ci_icon}", "#95", "no-pr")
            line = next(y for y, text in enumerate(self.client.screen.display) if "CI " in text)
            column = self.client.screen.display[line].index("CI ")
            expected_color = {"ci-passed": "238636", "ci-failed": "f85149", "ci-pending": "d29922", "cleared-ci": "238636"}[self.case]
            assert [self.client.screen.buffer[line][x].fg for x in range(column, column + 4)] == [expected_color] * 4
            root = next(w for w in self.api("api", "snapshot")["snapshot"]["workspaces"] if w.get("label") == "TCG")
            assert "pr_number" not in root.get("tokens", {})
            assert root["tokens"][self.ci_token] == f"CI {self.ci_icon}"
            if self.case == "cleared-ci":
                self.history_file.write_text(json.dumps({"data": {"repository": {"ref": {"target": {"history": {"nodes": []}}}}}}))
                self.refresh()
                self.wait_api(lambda s: all(all(token not in w.get("tokens", {}) for token in ("ci_passed", "ci_failed", "ci_pending")) for w in s["workspaces"]))
                self.client.wait("TCG", "#95", "no-pr")
                deadline = time.monotonic() + 10
                while "CI " in "\n".join(self.client.screen.display) and time.monotonic() < deadline:
                    self.client.read()
                grid = self.client.grid()
                assert "CI " not in "\n".join(self.client.screen.display)
            return grid
        if self.case == "client-config-without-token":
            grid = self.client.wait("export-import", "no-pr")
            assert "#95" not in "\n".join(self.client.screen.display)
            return grid
        if self.case == "client-config-reload":
            self.client.wait("export-import", "no-pr")
            assert "#95" not in "\n".join(self.client.screen.display)
            self.client_config.write_text(self.base_config + self.sidebar_config)
            self.client.send(b"\x02R")
        _, _, _, expected_color, icon = PR_CASES.get(
            self.case, ("OPEN", False, None, "238636", "")
        )
        grid = self.client.wait(f"#95 {icon}".rstrip(), "no-pr")
        line_index = next(
            y for y, line in enumerate(self.client.screen.display) if "#95" in line
        )
        column = self.client.screen.display[line_index].index("#95")
        assert [
            self.client.screen.buffer[line_index][x].fg
            for x in range(column, column + 3)
        ] == [expected_color] * 3
        assert "#95" in "\n".join(self.client.screen.display)
        assert [
            symbol
            for symbol in ("✅", "❌", "🟡")
            if symbol in self.client.screen.display[line_index]
        ] == ([icon] if icon else [])
        if self.case == "cleared-pr":
            (self.directory / "bin/empty").touch()
            self.refresh()
            self.wait_api(
                lambda s: all(
                    ("pr_number" not in w.get("tokens", {}) for w in s["workspaces"])
                )
            )
            grid = self.client.wait("export-import", "no-pr")
            assert "#95" not in "\n".join(self.client.screen.display)
        return grid

    def close(self, output):
        if self.client:
            (output / "terminal.ansi").write_bytes(self.client.raw)
            (output / "screen.txt").write_text("\n".join(self.client.screen.display))
            self.client.close()
        if self.server:
            try:
                logs = self.api(
                    "plugin",
                    "log",
                    "list",
                    "--plugin",
                    "herdrpp",
                    "--limit",
                    "20",
                )
                (output / "plugin.log").write_text(json.dumps(logs, indent=2))
                self.api("plugin", "action", "invoke", "herdrpp.stop")
                self.run("herdr", "--session", "snapshot", "server", "stop")
            except RuntimeError, subprocess.TimeoutExpired:
                pass
            try:
                self.server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.server.pid, signal.SIGKILL)
                self.server.wait(timeout=5)
        if self.log:
            self.log.close()
            shutil.copy(self.directory / "server.log", output / "server.log")


def capture_sidebar(case, record):
    output = OUTPUT / case
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pr-sidebar-") as directory:
        session = Session(Path(directory), case)
        try:
            session.setup()
            grid = session.capture()
            text = "\n".join(session.client.screen.display)
            assert "#95" not in next(
                (line for line in text.splitlines() if "no-pr" in line)
            ), text
            return compare(case, rasterize(grid), record=record)
        except Exception as error:
            (output / "error.txt").write_text(str(error))
            if session.client:
                rasterize(session.client.grid()).save(output / "actual.png")
            raise
        finally:
            session.close(output)


@pytest.fixture(scope="session")
def sidebar_capture(release_binary):
    if os.environ.get("HERDR_SNAPSHOT_CONTAINER") == "1":
        return capture_sidebar

    output = ROOT / "target/snapshots/sidebar"
    output.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            "docker",
            "build",
            "--platform",
            "linux/amd64",
            "-f",
            "tests/snapshots/sidebar.Dockerfile",
            "-t",
            "herdrpp-sidebar-snapshots",
            ".",
        ],
        cwd=ROOT,
        check=True,
    )

    def capture_in_container(case, record):
        baseline_mode = "rw" if record else "ro"
        command = [
            "docker",
            "run",
            "--rm",
            "--platform",
            "linux/amd64",
            "-e",
            f"CI={os.environ.get('CI', '')}",
            "-v",
            f"{ROOT}:/workspace:ro",
            "-v",
            f"{output}:/output",
            "-v",
            f"{ROOT / 'tests/snapshots/sidebar/baselines'}:/baselines:{baseline_mode}",
            "herdrpp-sidebar-snapshots",
            "uv",
            "run",
            "--locked",
            "pytest",
            "tests/test_sidebar_snapshots.py",
            "--snapshot-case",
            case,
        ]
        if record:
            command.append("--record")
        return subprocess.run(command, cwd=ROOT, check=False).returncode == 0

    return capture_in_container


@pytest.mark.snapshot
def test_sidebar(snapshot_case, record, sidebar_capture):
    assert sidebar_capture(snapshot_case, record)
