"""Visual and focus regressions from isolated Herdr sessions and controlled UI states."""

import argparse
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
import traceback
import unittest
from unittest.mock import Mock, patch

import pyte
from PIL import Image, ImageChops, ImageDraw, ImageFont
from snapshot_actions import completed_action, stop_startup_watcher

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = Path("/output")
BASELINES = Path("/baselines")
FONTS = ROOT / "tests/snapshots/fonts"
CASES = (
    "showcase-dracula", "hierarchical-unfiltered", "flat-search", "no-matches",
    "narrow-search", "scrolled-selection", "light-search",
    "empty-snapshot", "loading", "tiny-terminal", "tab-cycling",
    "tasks-search", "tasks-diagnostics",
    "palette-popup", "palette-search", "palette-empty", "palette-rename", "palette-create", "palette-confirm", "palette-narrow", "palette-loading",
)
CONTROLLED = {"tasks-search", "tasks-diagnostics", "empty-snapshot", "loading", "tiny-terminal",
              "palette-search", "palette-empty", "palette-rename", "palette-create", "palette-confirm", "palette-narrow", "palette-loading"}
SHOWCASE_PRS = (
    (95, "open-passing", "OPEN", False, "SUCCESS", "pr_open", "238636", "✅"),
    (96, "open-failing", "OPEN", False, "FAILURE", "pr_open", "238636", "❌"),
    (97, "open-pending", "OPEN", False, "PENDING", "pr_open", "238636", "🟡"),
    (98, "draft-passing", "OPEN", True, "SUCCESS", "pr_draft", "8b949e", "✅"),
    (99, "draft-failing", "OPEN", True, "FAILURE", "pr_draft", "8b949e", "❌"),
    (100, "draft-pending", "OPEN", True, "PENDING", "pr_draft", "8b949e", "🟡"),
    (101, "closed-passing", "CLOSED", False, "SUCCESS", "pr_closed", "f85149", "✅"),
    (102, "closed-failing", "CLOSED", False, "FAILURE", "pr_closed", "f85149", "❌"),
    (103, "closed-pending", "CLOSED", False, "PENDING", "pr_closed", "f85149", "🟡"),
    (104, "merged", "MERGED", False, "SUCCESS", "pr_merged", "a371f7", ""),
)
PR_TOKENS = ("pr_number", "pr_open", "pr_draft", "pr_closed", "pr_merged")
ANSI = dict(zip(
    ("black", "red", "green", "brown", "blue", "magenta", "cyan", "white",
     "brightblack", "brightred", "brightgreen", "brightbrown", "brightblue",
     "brightmagenta", "brightcyan", "brightwhite"),
    ("21222c", "ff5555", "50fa7b", "f1fa8c", "bd93f9", "ff79c6", "8be9fd",
     "f8f8f2", "6272a4", "ff6e6e", "69ff94", "ffffa5", "d6acff", "ff92df",
     "a4ffff", "ffffff"),
))


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
        fg = color(cell["bg"], background) if cell["reverse"] else color(cell["fg"], foreground)
        font = emoji if cell["text"] in ("✅", "❌", "🟡") else bold if cell["bold"] else regular
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
        print(f"FAIL {name}: missing baseline; run just record-snapshots {name}")
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
        self.screen.write_process_input = lambda text: os.write(self.master, text.encode())
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.raw = bytearray()
        self.process = subprocess.Popen(
            ["herdr", "--session", "snapshot"], stdin=slave, stdout=slave,
            stderr=slave, env=environment, cwd=directory, start_new_session=True,
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
                cells.append(dict(text=char.data, fg=char.fg, bg=char.bg,
                                  bold=char.bold, underline=char.underscore,
                                  reverse=char.reverse))
        return dict(width=self.screen.columns, height=self.screen.lines, cells=cells)

    def wait(self, *needles, timeout=15, absent=()):
        deadline = time.monotonic() + timeout
        previous, stable_since = None, time.monotonic()
        while time.monotonic() < deadline:
            self.read()
            grid = self.grid()
            text = "\n".join(self.screen.display)
            if grid != previous:
                previous, stable_since = grid, time.monotonic()
            if (all(needle in text for needle in needles)
                    and not any(needle in text for needle in absent)
                    and time.monotonic() - stable_since >= 0.5):
                return grid
            if self.process.poll() is not None:
                raise RuntimeError(f"Herdr client exited: {self.process.returncode}")
        raise TimeoutError(f"Screen did not settle with {needles!r}, absent={absent!r}:\n{text}")

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
        self.directory, self.case = directory, case
        self.server = self.client = None
        self.log = None
        self.environment = {key: value for key, value in os.environ.items()
                            if not key.startswith(("HERDR_", "XDG_", "GIT_", "GH_"))
                            and key not in ("ENV", "BASH_ENV", "NO_COLOR", "COLORTERM")}
        self.environment.update(
            XDG_CONFIG_HOME=str(directory / "config"),
            XDG_STATE_HOME=str(directory / "state"),
            XDG_DATA_HOME=str(directory / "data"),
            HERDR_CONFIG_PATH=str(directory / "config/herdr/config.toml"),
            SHELL="/bin/sh", PS1="$ ", PS2="> ", ENV="/dev/null",
            TERM="xterm-256color", COLORTERM="truecolor", LANG="C.UTF-8",
            LC_ALL="C.UTF-8", TZ="UTC",
        )

    def run(self, *args):
        result = subprocess.run(args, env=self.environment, cwd=self.directory,
                                text=True, capture_output=True, timeout=15)
        if result.returncode:
            raise RuntimeError(f"{args!r} exited {result.returncode}: {result.stderr}{result.stdout}")
        return result.stdout

    def api(self, *args):
        return json.loads(self.run("herdr", "--session", "snapshot", *args))["result"]

    def setup(self):
        config = Path(self.environment["HERDR_CONFIG_PATH"])
        config.parent.mkdir(parents=True)
        theme = "catppuccin-latte" if self.case == "light-search" else "dracula"
        sidebar_width = "sidebar_width = 38\n" if self.case == "showcase-dracula" else ""
        config.write_text(f"""onboarding = false
[theme]
name = "{theme}"
auto_switch = false
[terminal]
default_shell = "/bin/sh"
shell_mode = "non_login"
[update]
version_check = false
manifest_check = false
[ui]
{sidebar_width}tab_bar_right = []
host_cursor = "native"
[ui.toast]
enabled = false
[ui.sound]
enabled = false
""")
        plugin = self.directory / "plugin"
        (plugin / "target/release").mkdir(parents=True)
        shutil.copy(ROOT / "herdr-plugin.toml", plugin)
        shutil.copy(OUTPUT / "build/release/herdrpp", plugin / "target/release")
        self.run("herdr", "plugin", "link", str(plugin))
        if self.case == "showcase-dracula":
            self.run(str(plugin / "target/release/herdrpp"), "configure")
            self.prepare_showcase_github()
        plugin_config = Path(self.run("herdr", "plugin", "config-dir", "herdrpp").strip())
        plugin_config.mkdir(parents=True, exist_ok=True)
        mode = "flat" if self.case == "flat-search" else "hierarchical"
        (plugin_config / "config.toml").write_text(f'[picker]\nsearch_mode = "{mode}"\n')
        self.log = (self.directory / "server.log").open("wb")
        self.server = subprocess.Popen(
            ["herdr", "--session", "snapshot", "server"], cwd=self.directory,
            env=self.environment, stdout=self.log, stderr=self.log, start_new_session=True,
        )
        self.wait_ready()
        if self.case == "showcase-dracula":
            stop_startup_watcher(self.api, self.directory / "state/herdr/plugins/herdrpp")
            self.seed_showcase()
        else:
            self.seed()
        width, height = {
            "narrow-search": (32, 20),
            "scrolled-selection": (160, 12),
            "tab-cycling": (60, 20),
        }.get(self.case, (160, 40))
        self.client = Client(self.environment, width, height, self.directory)
        self.client.wait("Codex", "$")
        if self.case == "showcase-dracula":
            completed = completed_action(self.api, "refresh")
            assert "already in progress" not in (completed.get("stdout") or ""), completed
            self.assert_showcase_metadata()
            self.client.wait(*(f"#{case[0]}" for case in SHOWCASE_PRS))
            self.assert_showcase_sidebar()

    def prepare_showcase_github(self):
        directory = self.directory / "bin"
        directory.mkdir()
        fixtures = []
        for number, label, state, draft, check, _, _, _ in SHOWCASE_PRS:
            fixtures.append(dict(
                branch=label,
                association=dict(number=number, created_at="2026-10-01T00:00:00Z",
                                 state="closed" if state == "MERGED" else state.lower(),
                                 head=dict(ref=label, repo=dict(full_name="snapshot/herdrpp"))),
                details=dict(number=number, state=state, isDraft=draft,
                             statusCheckRollup=[dict(__typename="StatusContext", state=check)]),
            ))
        (directory / "prs.json").write_text(json.dumps(fixtures))
        fake = directory / "gh"
        fake.write_text("""#!/usr/bin/env python3
import json, pathlib, sys
fixtures = json.loads(pathlib.Path(__file__).with_name("prs.json").read_text())
args = sys.argv[1:]
if args[:2] == ["repo", "view"]:
    print(json.dumps({"nameWithOwner":"snapshot/herdrpp", "url":"https://github.com/snapshot/herdrpp"}))
elif args[:2] == ["pr", "view"]:
    print(json.dumps(next(case["details"] for case in fixtures if case["details"]["number"] == int(args[2]))))
elif args[0] == "api":
    print(json.dumps([case["association"] for case in fixtures if "head=snapshot:" + case["branch"] in args]))
else:
    sys.exit("Unexpected gh invocation: " + repr(args))
""")
        fake.chmod(0o755)
        self.environment["PATH"] = str(directory) + os.pathsep + self.environment["PATH"]

    def assert_showcase_metadata(self):
        snapshot = self.api("api", "snapshot")["snapshot"]
        workspaces = {workspace["label"]: workspace for workspace in snapshot["workspaces"]}
        for number, label, _, _, _, token, _, icon in SHOWCASE_PRS:
            tokens = workspaces[label]["tokens"]
            assert tokens["pr_number"] == f"#{number}", (label, tokens)
            assert tokens[token] == f"#{number} {icon}".rstrip(), (label, tokens)
            assert [key for key in PR_TOKENS[1:] if key in tokens] == [token], (label, tokens)
        assert not any(key in workspaces["no-pr"].get("tokens", {}) for key in PR_TOKENS)
        assert not any(key in workspaces["herdrpp"].get("tokens", {}) for key in PR_TOKENS)

    def assert_showcase_sidebar(self):
        rows = [(y, line[:38]) for y, line in enumerate(self.client.screen.display)]
        for number, label, _, _, _, _, expected_color, icon in SHOWCASE_PRS:
            y, line = next((y, line) for y, line in rows if f"#{number}" in line)
            assert label in line, (label, line)
            column = line.index(f"#{number}")
            assert [
                self.client.screen.buffer[y][x].fg
                for x in range(column, column + len(f"#{number}"))
            ] == [expected_color] * len(f"#{number}"), (label, line)
            assert [
                symbol for symbol in ("✅", "❌", "🟡") if symbol in line
            ] == ([icon] if icon else []), (label, line)
        no_pr = next(line for _, line in rows if "no-pr" in line)
        assert "#" not in no_pr, no_pr
        assert not any(symbol in no_pr for symbol in ("✅", "❌", "🟡")), no_pr

    def wait_ready(self, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                self.api("api", "snapshot")
                break
            except RuntimeError:
                if self.server.poll() is not None:
                    raise RuntimeError("Herdr server exited during startup")
                time.sleep(0.05)
        else:
            raise TimeoutError(f"Herdr server did not start within {timeout} seconds")

    def workspace(self, path, label, tabs):
        workspace = self.api("workspace", "create", "--cwd", str(path), "--label", label,
                             "--no-focus")["workspace"]["workspace_id"]
        self.tabs(workspace, tabs)
        return workspace

    def tabs(self, workspace, labels):
        tab = self.api("tab", "list", "--workspace", workspace)["tabs"][0]["tab_id"]
        self.api("tab", "rename", tab, labels[0])
        for label in labels[1:]:
            self.api("tab", "create", "--workspace", workspace, "--label", label, "--no-focus")

    def seed(self):
        projects = (
            ("herdrpp", ("Codex", "Terminal", "Tests"), (
                ("feature/search", ("Codex", "Tests")),
                ("fix/theme", ("Codex", "Terminal")),
                ("docs/readme", ("Terminal",)),
            )),
            ("TCG", ("Codex", "Terminal", "Tests"), (
                ("feature/collection", ("Codex", "Tests")),
                ("fix/pricing", ("Codex",)),
            )),
            ("dotfiles", ("Codex",), ()),
        )
        snapshot = self.seed_projects(projects)
        assert len(snapshot["workspaces"]) == 8, snapshot
        assert len(snapshot["tabs"]) == 15, snapshot
        assert sum(bool(w.get("worktree", {}).get("is_linked_worktree"))
                   for w in snapshot["workspaces"]) == 5, snapshot

    def seed_showcase(self):
        projects = (
            ("herdrpp", ("Codex", "Terminal", "Tests"),
             tuple((case[1], ("Codex",)) for case in SHOWCASE_PRS)),
            ("no-pr", ("Terminal",), ()),
        )
        snapshot = self.seed_projects(projects, "https://github.com/snapshot/herdrpp.git")
        assert len(snapshot["workspaces"]) == 12, snapshot
        assert len(snapshot["tabs"]) == 14, snapshot
        assert sum(bool(w.get("worktree", {}).get("is_linked_worktree"))
                   for w in snapshot["workspaces"]) == 10, snapshot

    def seed_projects(self, projects, remote=None):
        original = self.api("api", "snapshot")["snapshot"]["workspaces"]
        focus = None
        for name, tabs, worktrees in projects:
            repo = self.directory / name
            self.run("git", "init", "-b", "main", str(repo))
            self.run("git", "-C", str(repo), "-c", "user.name=Snapshot", "-c",
                     "user.email=snapshot@example.invalid", "commit", "--allow-empty", "-m", "Initial")
            if remote is not None and name == "herdrpp":
                self.run("git", "-C", str(repo), "remote", "add", "origin", remote)
            root = self.workspace(repo, name, tabs)
            if name == "herdrpp":
                focus = root
            for branch, labels in worktrees:
                path = self.directory / "worktrees" / name / branch
                self.run("git", "-C", str(repo), "worktree", "add", "-b", branch, str(path))
                result = self.api("worktree", "open", "--workspace", root, "--path", str(path),
                                  "--label", branch, "--no-focus")
                self.tabs(result["workspace"]["workspace_id"], labels)
        self.api("workspace", "focus", focus)
        for workspace in original:
            self.api("workspace", "close", workspace["workspace_id"])
        snapshot = self.api("api", "snapshot")["snapshot"]
        return snapshot

    def open_picker(self):
        self.run("herdr", "--session", "snapshot", "plugin", "action", "invoke", "herdrpp.open")
        return self.client.wait("results", "Esc Cancel")

    def capture(self):
        if self.case == "palette-popup":
            before = self.api("api", "snapshot")["snapshot"]
            self.api("plugin", "action", "invoke", "herdrpp.commands")
            self.client.wait("Search commands", "Command Palette")
            self.client.send(b"spl rgt")
            grid = self.client.wait("> spl rgt", "Split Pane Right", "1 results", "$")
            after = self.api("api", "snapshot")["snapshot"]
            assert focus_state(after) == focus_state(before)
            assert layout_state(after) == layout_state(before)
            return grid
        self.open_picker()
        if self.case == "showcase-dracula":
            grid = self.client.wait("12 results", "no-pr", *(case[1] for case in SHOWCASE_PRS))
            self.assert_showcase_sidebar()
            return grid
        if self.case == "hierarchical-unfiltered":
            return self.client.wait("docs/readme", "results")
        if self.case == "scrolled-selection":
            self.client.send(b"\x0e" * 40)
            grid = self.client.wait("dotfiles", "8/8")
            assert "[●Codex]" not in "\n".join(self.client.screen.display)
            return grid
        if self.case == "tab-cycling":
            self.client.send(b"herdrpp")
            self.client.wait("> herdrpp", "4 results")
            self.client.send(b"\t\t\t\x1b[Z")
            return self.client.wait("[Tests]", "◀", "4 results")
        query = {
            "flat-search": "herdrpp feature/search codex",
            "no-matches": "zzzzzzzz",
        }.get(self.case, "codex")
        self.client.send(query.encode())
        needles = [f"> {query}", "results"]
        if self.case == "no-matches":
            needles += ["0 results", "No matching results"]
        elif self.case == "flat-search":
            needles += ["herdrpp / feature/search", "[Codex]", "1 results"]
        elif self.case != "narrow-search":
            needles += ["feature/search", "fix/pricing", "dotfiles"]
        return self.client.wait(*needles)

    def close(self):
        if self.client:
            (OUTPUT / self.case / "terminal.ansi").write_bytes(self.client.raw)
            (OUTPUT / self.case / "screen.txt").write_text("\n".join(self.client.screen.display))
            self.client.close()
        if self.server:
            try:
                self.run("herdr", "--session", "snapshot", "server", "stop")
            except (RuntimeError, subprocess.TimeoutExpired):
                pass
            if self.server.poll() is None:
                os.killpg(self.server.pid, signal.SIGTERM)
                try:
                    self.server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(self.server.pid, signal.SIGKILL)
                    self.server.wait(timeout=5)
        if self.log:
            self.log.close()
            shutil.copy(self.directory / "server.log", OUTPUT / self.case)


def capture(case):
    if case in CONTROLLED:
        result = subprocess.run([str(OUTPUT / "build/release/examples/snapshot_render"), case],
                                check=True, capture_output=True, text=True, timeout=15)
        return json.loads(result.stdout)
    with tempfile.TemporaryDirectory(prefix="herdr-snap-") as directory:
        session = Session(Path(directory), case)
        try:
            session.setup()
            return session.capture()
        finally:
            session.close()


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name)
        self.enterContext(patch("builtins.print"))

    def test_missing_baseline_fails_without_recording(self):
        self.assertFalse(compare("missing", Image.new("RGB", (2, 2)), baselines=self.path, output=self.path))
        self.assertFalse((self.path / "missing.png").exists())

    def test_changed_pixel_produces_three_images_and_keeps_baseline(self):
        expected = Image.new("RGB", (2, 2))
        expected.save(self.path / "changed.png")
        actual = expected.copy()
        actual.putpixel((0, 0), (255, 0, 0))
        self.assertFalse(compare("changed", actual, baselines=self.path, output=self.path))
        self.assertEqual(Image.open(self.path / "changed.png").getpixel((0, 0)), (0, 0, 0))
        self.assertEqual(sorted(p.name for p in (self.path / "changed").iterdir()),
                         ["actual.png", "difference.png", "expected.png"])

    def test_identical_pixels_pass(self):
        expected = Image.new("RGB", (2, 2))
        expected.save(self.path / "same.png")
        self.assertTrue(compare("same", expected, baselines=self.path, output=self.path))

    def test_recording_is_rejected_in_ci(self):
        with patch.dict(os.environ, {"CI": "true"}):
            with self.assertRaisesRegex(RuntimeError, "disabled in CI"):
                compare("ci", Image.new("RGB", (2, 2)), record=True,
                        baselines=self.path, output=self.path)
        self.assertFalse((self.path / "ci.png").exists())

    def test_server_readiness_has_a_deadline(self):
        session = Session(self.path, "showcase-dracula")
        session.api = Mock(side_effect=RuntimeError("not ready"))
        session.server = Mock()
        session.server.poll.return_value = None
        with patch("time.monotonic", side_effect=[0, 0, 16]), patch("time.sleep"):
            with self.assertRaisesRegex(TimeoutError, "did not start"):
                session.wait_ready()
        session.api.assert_called_once_with("api", "snapshot")

    def test_screen_settling_has_a_deadline(self):
        client = Client.__new__(Client)
        client.screen = pyte.Screen(10, 2)
        client.read = Mock()
        client.process = Mock()
        client.process.poll.return_value = None
        with patch("time.monotonic", side_effect=[0, 0, 0, 0, 16]):
            with self.assertRaisesRegex(TimeoutError, "did not settle"):
                client.wait("never appears")
        client.read.assert_called_once()

    def test_visible_popup_prevents_wait_from_accepting_the_underlying_prompt(self):
        client = Client.__new__(Client)
        client.screen = pyte.Screen(30, 2)
        pyte.Stream(client.screen).feed("$ Esc Cancel")
        client.read = Mock()
        client.process = Mock()
        client.process.poll.return_value = None
        with patch("time.monotonic", side_effect=[0, 0, 0, 0, 16]):
            with self.assertRaisesRegex(TimeoutError, "did not settle"):
                client.wait("$", absent=("Esc Cancel",))
        client.read.assert_called_once()

    def test_cleanup_stops_owned_process_when_api_stop_fails(self):
        session = Session(self.path, "showcase-dracula")
        session.run = Mock(side_effect=RuntimeError("API unavailable"))
        session.server = subprocess.Popen(["sleep", "30"], start_new_session=True)
        self.addCleanup(session.server.kill)
        session.close()
        self.assertIsNotNone(session.server.poll())

    def test_setup_failure_still_runs_cleanup(self):
        with patch.object(Session, "setup", side_effect=RuntimeError("setup failed")), \
                patch.object(Session, "close") as close:
            with self.assertRaisesRegex(RuntimeError, "setup failed"):
                capture("showcase-dracula")
        close.assert_called_once()

    def test_private_queries_do_not_break_truecolor_and_highlight_capture(self):
        screen = SnapshotScreen(10, 2)
        screen.write_process_input = Mock()
        pyte.Stream(screen).feed("\x1b[?996n\x1b[6n\x1b[1;4;38;2;80;250;123mC")
        self.assertEqual(screen.buffer[0][0].fg, "50fa7b")
        self.assertTrue(screen.buffer[0][0].bold)
        self.assertTrue(screen.buffer[0][0].underscore)
        screen.write_process_input.assert_called_once_with("\x1b[1;1R")


def focus_state(snapshot):
    return tuple(snapshot[key] for key in
                 ("focused_workspace_id", "focused_tab_id", "focused_pane_id"))


def layout_state(snapshot):
    return {
        layout["tab_id"]: dict(
            workspace_id=layout["workspace_id"],
            focused_pane_id=layout["focused_pane_id"],
            panes=[pane["pane_id"] for pane in layout["panes"]],
            splits=[{key: split[key] for key in ("id", "direction", "ratio")}
                    for split in layout["splits"]],
            zoomed=layout["zoomed"],
        )
        for layout in snapshot["layouts"]
    }


class FocusTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="herdr-focus-")
        self.addCleanup(self.temporary.cleanup)
        self.case = f"focus-{self._testMethodName}"
        self.output = OUTPUT / self.case
        if self.output.exists():
            shutil.rmtree(self.output)
        self.output.mkdir(parents=True)
        self.session = Session(Path(self.temporary.name), self.case)
        self.addCleanup(self.session.close)
        self.session.setup()
        self.initial = self.snapshot()
        self.origin = self.split_tab(self.initial["focused_tab_id"])
        self.session.client.wait("$")

    def snapshot(self):
        return self.session.api("api", "snapshot")["snapshot"]

    def split_tab(self, tab):
        self.session.api("tab", "focus", tab)
        before = self.snapshot()
        pane = self.session.api("pane", "split", before["focused_pane_id"],
                                "--direction", "right", "--focus")["pane"]
        self.assertNotEqual(pane["pane_id"], before["focused_pane_id"])
        return pane["workspace_id"], pane["tab_id"], pane["pane_id"]

    def prepare_target(self, workspace_label, tab_label, remembered_tab=None):
        workspace = next(workspace for workspace in self.initial["workspaces"]
                         if workspace["label"] == workspace_label)["workspace_id"]
        tabs = {tab["label"]: tab["tab_id"] for tab in self.initial["tabs"]
                if tab["workspace_id"] == workspace}
        target = self.split_tab(tabs[tab_label])
        if remembered_tab is not None:
            self.session.api("tab", "focus", tabs[remembered_tab])
        self.session.api("tab", "focus", self.origin[1])
        return target

    def open_picker(self, query):
        self.before = self.snapshot()
        (self.output / "before.json").write_text(json.dumps(self.before, indent=2))
        self.session.open_picker()
        opened = self.snapshot()
        self.assertEqual(layout_state(opened), layout_state(self.before))
        self.assertEqual(focus_state(opened), focus_state(self.before))
        self.session.client.send(query.encode())
        self.session.client.wait(f"> {query}", "results")

    def click(self, label):
        y, line = next((y, line) for y, line in enumerate(self.session.client.screen.display)
                       if label in line)
        x = line.index(label)
        return f"\x1b[<0;{x + 1};{y + 1}M".encode()

    def finish(self, keys, expected):
        self.session.client.send(keys)
        self.session.client.wait("$", absent=("Workspaces and tabs", "Esc Cancel"))
        after = self.snapshot()
        (self.output / "after.json").write_text(json.dumps(after, indent=2))
        self.assertEqual(layout_state(after), layout_state(self.before))
        self.assertEqual(sorted(pane["pane_id"] for pane in after["panes"]),
                         sorted(pane["pane_id"] for pane in self.before["panes"]))
        remembered = {workspace["workspace_id"]: workspace["active_tab_id"]
                      for workspace in self.before["workspaces"]}
        remembered[expected[0]] = expected[1]
        self.assertEqual({workspace["workspace_id"]: workspace["active_tab_id"]
                          for workspace in after["workspaces"]}, remembered)
        return after

    def test_workspace_enter_restores_remembered_tab_and_non_root_pane(self):
        target = self.prepare_target("TCG", "Terminal")
        self.open_picker("TCG")
        after = self.finish(b"\r", target)
        self.assertEqual(focus_state(after), target)
        self.session.client.send(b"printf 'picker-%s-ready\\n' 'input'\r")
        self.session.client.wait("picker-input-ready", absent=("Esc Cancel",))
        self.assertIn("picker-input-ready", self.session.run(
            "herdr", "--session", "snapshot", "pane", "read", target[2], "--format", "text"))
        self.assertEqual(self.snapshot()["focused_pane_id"], target[2])

    def test_flat_tab_enter_switches_worktree_and_preserves_its_non_root_pane(self):
        target = self.prepare_target("feature/search", "Tests", remembered_tab="Codex")
        config = Path(self.session.run("herdr", "plugin", "config-dir", "herdrpp").strip())
        (config / "config.toml").write_text('[picker]\nsearch_mode = "flat"\n')
        self.open_picker("feature/search tests")
        after = self.finish(b"\r", target)
        self.assertEqual(focus_state(after), target)

    def test_hierarchical_tab_enter_switches_worktree_and_preserves_its_non_root_pane(self):
        target = self.prepare_target("feature/search", "Tests", remembered_tab="Codex")
        self.open_picker("feature/search tests")
        after = self.finish(b"\r", target)
        self.assertEqual(focus_state(after), target)

    def assert_picker_clear_preserves_focus(self, keys):
        self.open_picker("feature/search tests")
        self.session.client.send(keys)
        self.session.client.wait("Search workspaces, worktrees and tabs",
                                 absent=("> feature/search tests",))
        self.assertEqual(focus_state(self.snapshot()), self.origin)
        after = self.finish(b"\x1b", self.origin)
        self.assertEqual(focus_state(after), self.origin)

    def test_command_backspace_through_herdr_clears_picker_without_changing_focus(self):
        self.assert_picker_clear_preserves_focus(b"\x1b[127;9u")

    def test_ghostty_command_backspace_ctrl_u_through_herdr_clears_picker(self):
        self.assert_picker_clear_preserves_focus(b"\x15")

    def test_ok_click_restores_workspace_tab_and_non_root_pane(self):
        target = self.prepare_target("TCG", "Terminal")
        self.open_picker("TCG")
        after = self.finish(self.click("↵ OK"), target)
        self.assertEqual(focus_state(after), target)

    def test_mouse_click_on_sole_tab_preserves_its_non_root_pane(self):
        target = self.prepare_target("dotfiles", "Codex")
        self.open_picker("dotfiles")
        after = self.finish(self.click("[Codex]"), target)
        self.assertEqual(focus_state(after), target)

    def test_escape_after_selecting_another_tab_keeps_original_focus_and_layout(self):
        self.prepare_target("feature/search", "Tests", remembered_tab="Codex")
        self.open_picker("feature/search tests")
        self.session.client.send(b"\x1b[B\t")
        after = self.finish(b"\x1b", self.origin)
        self.assertEqual(focus_state(after), self.origin)
        self.session.client.send(b"printf 'picker-%s-ready\\n' 'cancel'\r")
        self.session.client.wait("picker-cancel-ready", absent=("Esc Cancel",))
        self.assertIn("picker-cancel-ready", self.session.run(
            "herdr", "--session", "snapshot", "pane", "read", self.origin[2], "--format", "text"))
        self.assertEqual(self.snapshot()["focused_pane_id"], self.origin[2])

    def test_compact_picker_preserves_visible_terminal_and_ignores_outside_click(self):
        self.prepare_target("feature/search", "Tests", remembered_tab="Codex")
        self.session.run("herdr", "--session", "snapshot", "pane", "send-text", self.origin[2],
                         "printf 'terminal-%s-visible\\n' 'around-picker'")
        self.session.run("herdr", "--session", "snapshot", "pane", "send-keys", self.origin[2], "enter")
        self.session.client.wait("terminal-around-picker-visible")
        self.open_picker("feature/search tests")
        self.assertIn("terminal-around-picker-visible", "\n".join(self.session.client.screen.display))
        self.session.client.send(b"\x1b[B\t\x1b[<0;80;3M\x1b[<0;80;3m")
        self.session.client.wait("> feature/search tests", "Esc Cancel", "terminal-around-picker-visible")
        self.assertEqual(focus_state(self.snapshot()), self.origin)
        self.assertEqual(layout_state(self.snapshot()), layout_state(self.before))
        after = self.finish(b"\x1b", self.origin)
        self.assertEqual(focus_state(after), self.origin)
        self.session.client.send(b"printf 'picker-%s-ready\\n' 'compact'\r")
        self.session.client.wait("picker-compact-ready", absent=("Esc Cancel",))
        self.assertIn("picker-compact-ready", self.session.run(
            "herdr", "--session", "snapshot", "pane", "read", self.origin[2], "--format", "text"))

    def test_closed_destination_restores_original_tab_and_non_root_pane(self):
        target = self.prepare_target("feature/search", "Tests", remembered_tab="Codex")
        self.open_picker("feature/search tests")
        self.session.api("tab", "close", target[1])
        self.before = self.snapshot()
        (self.output / "destination-closed.json").write_text(json.dumps(self.before, indent=2))
        after = self.finish(b"\r", self.origin)
        self.assertNotIn(target[1], [tab["tab_id"] for tab in after["tabs"]])
        self.assertEqual(focus_state(after), self.origin)

    def open_commands(self, query):
        self.before = self.snapshot()
        invocation = self.session.api("plugin", "action", "invoke", "herdrpp.commands")
        self.command_log = invocation["log"]["log_id"]
        self.session.client.wait("Search commands", "Command Palette")
        self.session.client.send(query.encode())
        self.session.client.wait(f"> {query}", "results")

    def wait_command(self):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            logs = self.session.api("plugin", "log", "list", "--plugin", "herdrpp", "--limit", "20")["logs"]
            completed = next((log for log in logs if log["log_id"] == self.command_log and log["status"] != "running"), None)
            if completed is not None:
                self.assertEqual(completed["status"], "succeeded", completed)
                return completed
            time.sleep(0.05)
        self.fail(f"Palette action did not complete: {logs}")

    def write_tasks(self, task):
        directory = self.session.directory / "herdrpp/.herdr"
        directory.mkdir(exist_ok=True)
        (directory / "tasks.json").write_text(json.dumps([task]))

    def open_tasks(self):
        self.before = self.snapshot()
        invocation = self.session.api("plugin", "action", "invoke", "herdrpp.tasks")
        self.command_log = invocation["log"]["log_id"]
        self.session.client.wait("Search tasks", "Task Palette")

    def test_task_new_tab_reuses_the_same_pane_after_interrupting_a_running_task(self):
        runs = self.session.directory / "herdrpp/runs"
        self.write_tasks({"label":"dev-task", "command":"printf 'task-%s-ready\\n' 'run'; printf 'run\\n' >> runs; sleep 30"})
        self.open_tasks()
        self.session.client.send(b"\r")
        self.wait_command()
        self.session.client.wait("task-run-ready", absent=("Task Palette",))
        first = self.snapshot()
        self.assertNotEqual(first["focused_tab_id"], self.origin[1])
        self.assertEqual(runs.read_text(), "run\n")
        self.session.api("tab", "focus", self.origin[1])
        self.open_tasks()
        self.session.client.send(b"\r")
        self.wait_command()
        after = self.snapshot()
        self.assertEqual(focus_state(after), focus_state(first))
        self.assertEqual(len(after["tabs"]), len(first["tabs"]))
        self.assertEqual(runs.read_text(), "run\nrun\n")

    def test_task_same_tab_uses_the_opening_idle_shell(self):
        self.write_tasks({"label":"test-task", "command":"printf 'task-%s-ready\\n' 'same-pane'", "new_tab":False})
        self.open_tasks()
        self.session.client.send(b"\r")
        self.wait_command()
        self.session.client.wait("task-same-pane-ready", absent=("Task Palette",))
        after = self.snapshot()
        self.assertEqual(focus_state(after), self.origin)
        self.assertEqual(layout_state(after), layout_state(self.before))

    def test_invalid_task_diagnostics_preserve_focus_and_disable_execution(self):
        self.write_tasks({"label":"broken-task", "command":"true", "new_tab":"false", "unknown":1})
        before = self.snapshot()
        invocation = self.session.api("plugin", "action", "invoke", "herdrpp.tasks")
        self.command_log = invocation["log"]["log_id"]
        self.session.client.wait("tasks.json", "Remove the quotes", "unknown field")
        self.session.client.send(b"\r")
        self.session.client.wait("Remove the quotes")
        self.session.client.send(b"\x1b")
        self.wait_command()
        after = self.snapshot()
        self.assertEqual(focus_state(after), self.origin)
        self.assertEqual(layout_state(after), layout_state(before))

    def test_command_palette_reuses_compact_popup_and_keeps_terminal_visible(self):
        self.session.run("herdr", "--session", "snapshot", "pane", "send-text", self.origin[2],
                         "printf 'terminal-%s-visible\\n' 'around-palette'")
        self.session.run("herdr", "--session", "snapshot", "pane", "send-keys", self.origin[2], "enter")
        self.session.client.wait("terminal-around-palette-visible")
        self.open_commands("Split Pane Right")
        self.assertIn("terminal-around-palette-visible", "\n".join(self.session.client.screen.display))
        self.assertEqual(focus_state(self.snapshot()), self.origin)
        self.assertEqual(layout_state(self.snapshot()), layout_state(self.before))
        after = self.finish(b"\x1b", self.origin)
        self.wait_command()
        self.assertEqual(focus_state(after), self.origin)

    def test_command_palette_cancel_restores_original_focus_and_layout(self):
        self.open_commands("split right")
        after = self.finish(b"\x1b", self.origin)
        self.wait_command()
        self.assertEqual(focus_state(after), self.origin)

    def assert_commands_clear_preserves_focus(self, keys):
        self.open_commands("Split Pane Right")
        self.session.client.send(keys)
        self.session.client.wait("Search commands", absent=("> Split Pane Right",))
        self.assertEqual(focus_state(self.snapshot()), self.origin)
        after = self.finish(b"\x1b", self.origin)
        self.wait_command()
        self.assertEqual(focus_state(after), self.origin)

    def test_ghostty_command_backspace_clears_command_palette(self):
        self.assert_commands_clear_preserves_focus(b"\x15")

    def test_command_backspace_clears_command_palette(self):
        self.assert_commands_clear_preserves_focus(b"\x1b[127;9u")

    def test_command_palette_renames_original_tab(self):
        self.open_commands("Rename Tab")
        self.session.client.send(b"\r")
        original = next(tab["label"] for tab in self.before["tabs"] if tab["tab_id"] == self.origin[1])
        self.session.client.wait("> " + original, "Rename Tab")
        self.session.client.send(b"\x15discard this name")
        self.session.client.wait("> discard this name")
        self.session.client.send(b"\x15")
        self.session.client.wait("Enter a name", absent=("> discard this name",))
        self.session.client.send("--世界 Review\r".encode())
        self.wait_command()
        after = self.finish(b"", self.origin)
        self.wait_command()
        renamed = next(tab for tab in after["tabs"] if tab["tab_id"] == self.origin[1])
        self.assertEqual(renamed["label"], "--世界 Review")
        self.assertEqual(focus_state(after), self.origin)

    def test_command_palette_new_tab_prompts_before_creating_and_focuses_named_tab(self):
        self.open_commands("New Tab")
        self.session.client.send(b"\r")
        self.session.client.wait("New Tab", "Enter Create")
        self.assertEqual(layout_state(self.snapshot()), layout_state(self.before))
        self.session.client.send("--世界 Review\r".encode())
        self.wait_command()
        self.session.client.wait("$", absent=("Esc Cancel",))
        after = self.snapshot()
        created = next(tab for tab in after["tabs"] if tab["tab_id"] not in {tab["tab_id"] for tab in self.before["tabs"]})
        self.assertEqual(created["label"], "--世界 Review")
        self.assertEqual(created["workspace_id"], self.origin[0])
        self.assertEqual(after["focused_tab_id"], created["tab_id"])
        self.assertEqual(len(after["tabs"]), len(self.before["tabs"]) + 1)

    def test_command_palette_new_workspace_prompts_before_creating_and_focuses_named_workspace(self):
        self.open_commands("New Workspace")
        self.session.client.send(b"\r")
        self.session.client.wait("New Workspace", "Enter Create")
        self.assertEqual(layout_state(self.snapshot()), layout_state(self.before))
        self.session.client.send(b"Review\r")
        self.wait_command()
        self.session.client.wait("$", absent=("Esc Cancel",))
        after = self.snapshot()
        created = next(workspace for workspace in after["workspaces"] if workspace["workspace_id"] not in {workspace["workspace_id"] for workspace in self.before["workspaces"]})
        self.assertEqual(created["label"], "Review")
        self.assertEqual(after["focused_workspace_id"], created["workspace_id"])
        self.assertEqual(len(after["workspaces"]), len(self.before["workspaces"]) + 1)

    def test_command_palette_cancel_new_tab_preserves_focus_and_layout(self):
        self.open_commands("New Tab")
        self.session.client.send(b"\r")
        self.session.client.wait("New Tab", "Enter Create")
        self.session.client.send(b"Discard\x1b")
        self.session.client.wait("> New Tab", "Enter Run", absent=("Enter Create",))
        after = self.finish(b"\x1b", self.origin)
        self.wait_command()
        self.assertEqual(focus_state(after), self.origin)
        self.assertEqual(layout_state(after), layout_state(self.before))

    def test_command_palette_hands_off_to_workspace_picker_popup(self):
        self.open_commands("herdrpp.open")
        self.session.client.send(b"\r")
        self.session.client.wait("Search workspaces, worktrees and tabs", absent=("Command Palette",))
        self.wait_command()
        after = self.finish(b"\x1b", self.origin)
        self.assertEqual(focus_state(after), self.origin)

    def test_command_palette_split_targets_original_pane_once(self):
        self.open_commands("Split Pane Right")
        self.session.client.send(b"\r")
        self.session.client.wait("$", absent=("Command Palette", "Esc Cancel"))
        self.wait_command()
        after = self.snapshot()
        self.assertEqual(len(after["panes"]), len(self.before["panes"]) + 1)
        self.assertEqual(after["focused_tab_id"], self.origin[1])
        self.assertNotEqual(after["focused_pane_id"], self.origin[2])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--repeat", type=int, default=1, help="capture each case repeatedly to check determinism")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--focus-tests", action="store_true")
    parser.add_argument("cases", nargs="*", metavar="CASE", help=", ".join(CASES))
    args = parser.parse_args()
    if any(case not in CASES for case in args.cases):
        parser.error("Unknown snapshot case; see --help for case names")
    if args.repeat < 1:
        parser.error("--repeat must be at least 1")
    if args.record and os.environ.get("CI"):
        parser.error("Recording snapshots is disabled in CI")
    if args.self_test:
        result = unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(ComparisonTests))
        return 0 if result.wasSuccessful() else 1
    subprocess.run(["cargo", "+1.99.0", "build", "--locked", "--release", "--bin", "herdrpp",
                    "--example", "snapshot_render"], check=True)
    if args.focus_tests:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(FocusTests))
        return 0 if result.wasSuccessful() else 1
    failed = []
    for case in args.cases or CASES:
        directory = OUTPUT / case
        if directory.exists():
            shutil.rmtree(directory)
        directory.mkdir(parents=True)
        try:
            image = rasterize(capture(case), light=case == "light-search")
            image.save(directory / "actual.png")
            for _ in range(args.repeat - 1):
                repeated = rasterize(capture(case), light=case == "light-search")
                if repeated.size != image.size or ImageChops.difference(image, repeated).getbbox():
                    repeated.save(directory / "unstable.png")
                    raise AssertionError("Fresh captures produced different pixels")
            if not compare(case, image, args.record):
                failed.append(case)
            else:
                print(f"{'RECORDED' if args.record else 'PASS'} {case}", flush=True)
        except Exception as error:
            (directory / "error.txt").write_text(traceback.format_exc())
            print(f"FAIL {case}: {error}", flush=True)
            failed.append(case)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
