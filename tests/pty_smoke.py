"""Exercise the release binary in a real PTY with a fake Herdr CLI (no server)."""

import errno
import fcntl
import json
import os
import pty
import selectors
import struct
import subprocess
import tempfile
import termios
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/release/herdrpp"
FIXTURE = ROOT / "tests/fixtures/snapshot.json"
FAKE = """#!/usr/bin/env python3
import json, os, sys, time
with open(os.environ["PICKER_TEST_LOG"], "a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\\n")
args = sys.argv[1:]
if args == ["api", "snapshot"]:
    if os.environ.get("PICKER_TEST_WAIT"):
        time.sleep(30)
    print(open(os.environ["PICKER_TEST_FIXTURE"]).read())
elif args[:2] in (["tab", "focus"], ["workspace", "focus"]) and os.environ.get("PICKER_TEST_FAIL") == args[2]:
    print(json.dumps({"error":{"code":"not_found","message":"destination closed"}}))
    sys.exit(1)
elif args[:2] == ["workspace", "focus"]:
    print(json.dumps({"result":{"type":"workspace_info", "workspace":{"workspace_id":args[2]}}}))
elif args[:2] == ["tab", "focus"]:
    print(json.dumps({"result":{"type":"tab_info", "tab":{"tab_id":args[2]}}}))
else:
    print(json.dumps({"result":{"type":"ok"}}))
"""


def read_until(fd, needle):
    deadline = time.monotonic() + 8
    received = b""
    with selectors.DefaultSelector() as selector:
        selector.register(fd, selectors.EVENT_READ)
        while needle not in received:
            assert time.monotonic() < deadline, received[-1000:]
            if selector.select(0.1):
                received += os.read(fd, 65536)
    return received


def case(
    keys,
    mode="hierarchical",
    failure="",
    expected=(),
    exit_code=0,
    loading=False,
    fixture=FIXTURE,
    theme="[theme]\nname = 'dracula'\n",
    resize=True,
    appearance_reports=(),
):
    with tempfile.TemporaryDirectory(prefix="herdrpp-pty-") as directory:
        directory = Path(directory)
        fake = directory / "herdr"
        fake.write_text(FAKE)
        fake.chmod(0o700)
        (directory / "config.toml").write_text(f'[picker]\nsearch_mode = "{mode}"\n')
        herdr_config = directory / "herdr.toml"
        herdr_config.write_text(theme)
        log = directory / "calls.jsonl"
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 80, 0, 0))
        before = termios.tcgetattr(slave)
        environment = dict(
            os.environ,
            HERDR_BIN_PATH=str(fake),
            HERDR_CONFIG_PATH=str(herdr_config),
            HERDR_PLUGIN_CONFIG_DIR=str(directory),
            PICKER_TEST_LOG=str(log),
            PICKER_TEST_FIXTURE=str(fixture),
            PICKER_TEST_FAIL=failure,
            TERM="xterm-256color",
            PICKER_TEST_WAIT="1" if loading else "",
        )
        environment.pop("NO_COLOR", None)
        process = subprocess.Popen(
            [str(BINARY), "pick"],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=environment,
            start_new_session=True,
        )
        try:
            output = read_until(master, b"Loading" if loading else b"results")
            assert b"\x1b[>1u" in output, "enhanced keyboard input not requested"
            for report, color in appearance_reports:
                os.write(master, report)
                output += read_until(master, color)
            log_deadline = time.monotonic() + 3
            while not log.exists():
                assert time.monotonic() < log_deadline, "CLI did not start"
                time.sleep(0.01)
            if resize:
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 12, 60, 0, 0))
            os.write(master, keys)
            deadline = time.monotonic() + (2 if loading else 8)
            while process.poll() is None:
                assert time.monotonic() < deadline, "picker did not exit"
                with selectors.DefaultSelector() as selector:
                    selector.register(master, selectors.EVENT_READ)
                    if selector.select(0.1):
                        try:
                            output += os.read(master, 65536)
                        except OSError as error:
                            assert error.errno == errno.EIO
            assert process.returncode == exit_code, process.returncode
            assert b"\x1b[?1000l" in output, "mouse reporting not disabled"
            assert b"\x1b[?2004l" in output, "paste reporting not disabled"
            assert b"\x1b[<1u" in output, "enhanced keyboard input not restored"
            if "auto_switch = true" in theme:
                assert b"\x1b[?2031h" in output, "appearance reporting not enabled"
                assert b"\x1b[?2031l" in output, "appearance reporting not disabled"
            assert termios.tcgetattr(slave) == before, "terminal mode not restored"
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            assert calls == [["api", "snapshot"], *expected], calls
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)


def palette_case(keys, expected, loading=False, recent_ids=None):
    with tempfile.TemporaryDirectory(prefix="herdrpp-palette-pty-") as directory:
        directory = Path(directory)
        catalog = dict(context={}, warning=None, recent_ids=recent_ids or [],
                       naming=dict(workspace_name="Project", tab_name="Codex", pane_name="",
                                   tab_auto_named=True, workspace_cwd=None,
                                   workspace_suggestion="workspace", tab_suggestion="3"),
                       items=[dict(id="herdr:split_right", title="Split Pane Right", source="Herdr",
                                   description="", command={"Split":"Right"}),
                              dict(id="herdr:tab_rename", title="Rename Tab", source="Herdr",
                                   description="", command={"Rename":"Tab"}),
                              dict(id="herdr:tab_create", title="New Tab", source="Herdr",
                                   description="", command={"Create":"Tab"})])
        if not loading:
            (directory / "catalog.json").write_text(json.dumps(catalog))
        config = directory / "herdr.toml"
        config.write_text("[theme]\nname='dracula'\n")
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 80, 0, 0))
        before = termios.tcgetattr(slave)
        environment = dict(os.environ, HERDRPP_PALETTE_REQUEST=str(directory),
                           HERDR_CONFIG_PATH=str(config), TERM="xterm-256color")
        process = subprocess.Popen([str(BINARY), "pick-command"], stdin=slave,
                                   stdout=slave, stderr=slave, env=environment,
                                   start_new_session=True)
        try:
            output = read_until(master, b"Loading commands" if loading else b"Search commands")
            os.write(master, keys)
            deadline = time.monotonic() + 5
            with selectors.DefaultSelector() as selector:
                selector.register(master, selectors.EVENT_READ)
                while not (directory / "selection.json").exists():
                    assert time.monotonic() < deadline, output[-1000:]
                    if selector.select(0.05):
                        output += os.read(master, 65536)
                while selector.select(0.05):
                    output += os.read(master, 65536)
            assert json.loads((directory / "selection.json").read_text()) == expected
            assert b"\x1b[?1000l" in output
            assert b"\x1b[?2004l" in output
            assert termios.tcgetattr(slave) == before
        finally:
            process.terminate()
            process.wait(timeout=5)
            os.close(master)
            os.close(slave)


def main():
    palette_case(b"\rReview\r", dict(id="herdr:tab_create", input="Review", confirmed=False), recent_ids=["herdr:tab_create"])
    palette_case(b"split\x15\rReview\r", dict(id="herdr:tab_create", input="Review", confirmed=False), recent_ids=["herdr:tab_create"])
    palette_case(b"spl rgt\r", dict(id="herdr:split_right", input=None, confirmed=False))
    palette_case(b"unmatched\x15spl rgt\r", dict(id="herdr:split_right", input=None, confirmed=False))
    palette_case(b"unmatched\x1b[117;5uspl rgt\r", dict(id="herdr:split_right", input=None, confirmed=False))
    palette_case(b"unmatched\x1b[127;9uspl rgt\r", dict(id="herdr:split_right", input=None, confirmed=False))
    palette_case(b"\x1b", None)
    palette_case(b"\x1b", None, loading=True)
    palette_case(b"Rename Tab\r Review\r", dict(id="herdr:tab_rename", input="Codex Review", confirmed=False))
    palette_case(b"New Tab\rReview\r", dict(id="herdr:tab_create", input="Review", confirmed=False))
    palette_case(b"New Tab\r\r", dict(id="herdr:tab_create", input="3", confirmed=False))
    palette_case(b"New Tab\r\x15\r", dict(id="herdr:tab_create", input="", confirmed=False))
    case(b"\x1b")
    case(b"\x1b", loading=True)
    case(b"col\x1b")
    case(b"colorpickxr\x1b[D\x7fe\r", expected=[["workspace", "focus", "w1"]])
    case(b"colorpicker\x1b[D\x1b[D\x15\r", expected=[["workspace", "focus", "w2"]])
    case(b"colorpicker\x1b[D\x1b[127;9u\r", expected=[["workspace", "focus", "w2"]])
    palette_case(b"spl rxt\x1b[D\x7fg\r", dict(id="herdr:split_right", input=None, confirmed=False))
    palette_case(b"Rename Tab\r\x1b[D\x7f\r", dict(id="herdr:tab_rename", input="Codx", confirmed=False))
    palette_case(b"Rename Tab\r\x1b[D\x1b[127;9uReview\r", dict(id="herdr:tab_rename", input="Review", confirmed=False))

    case(b"\r", expected=[["workspace", "focus", "w1"]])
    case(b"codex\r", expected=[["workspace", "focus", "w3"]])
    case(b"server\r", mode="flat", expected=[["tab", "focus", "w2:t2"]])
    case(b"colorpicker\t\t\x1b[Z\r", expected=[["tab", "focus", "w1:t9"]])
    case(b"colorpicker\x1b[127;9u\r", expected=[["workspace", "focus", "w2"]])
    case(b"colorpicker\x15\r", expected=[["workspace", "focus", "w2"]])
    case(b"colorpicker\x1b[117;5u\r", expected=[["workspace", "focus", "w2"]])
    case(b"colorpicker\t\x1b[Z\r", mode="flat", expected=[["tab", "focus", "w1:t8"]])
    case(b"\x1b[C\x1b[B\x1b[A\r", expected=[["workspace", "focus", "w1"]])
    case(b"colorpicker\t\t\t\t\r", expected=[["tab", "focus", "w1:t9"]])
    case(b"colorpicker\x1b[Z\r", mode="flat", expected=[["tab", "focus", "w1:t4"]])
    case(b"colorpicker\t\x1b[Z\r", expected=[["tab", "focus", "w1:t8"]])
    case(b"exact terminal\r", expected=[["tab", "focus", "w3:t1"]])
    case(b"exact terminal\r", mode="flat", expected=[["tab", "focus", "w3:t1"]])
    case(b"pickercodex\r", mode="flat", expected=[["tab", "focus", "w1:t8"]])
    case(b"exact terminal\t\t\r", expected=[["tab", "focus", "w3:t1"]])
    case(b"exact terminal\x1b[Z\r", mode="flat", expected=[["tab", "focus", "w3:t1"]])
    case(b"codexterminal\r", expected=[["tab", "focus", "w3:t1"]])
    case(
        b"\r",
        failure="w1",
        exit_code=1,
        expected=[
            ["workspace", "focus", "w1"],
            ["tab", "focus", "w2:t7"],
            [
                "notification",
                "show",
                "herdr++",
                "--body",
                "Herdr not_found: destination closed",
            ],
        ],
    )
    WORKTREE_FIXTURE = ROOT / "tests/fixtures/worktrees.json"
    case(
        b"herdrppmultimachinecodex\r",
        fixture=WORKTREE_FIXTURE,
        expected=[["tab", "focus", "w2:t1"]],
    )
    case(
        b"herdrpp just\r",
        fixture=WORKTREE_FIXTURE,
        expected=[["workspace", "focus", "w4"]],
    )
    case(
        b"herdrpp multimachine codex\r",
        mode="flat",
        fixture=WORKTREE_FIXTURE,
        expected=[["tab", "focus", "w2:t1"]],
    )
    case(b"\x1b[<0;31;20M", resize=False, expected=[["workspace", "focus", "w1"]])
    case(b"\x1b[<0;39;20M", resize=False)
    case(b"\x1b[<0;39;20M", resize=False, loading=True)
    case(b"nonexistenttttt\x1b[<0;31;20M\x1b[<0;39;20M", resize=False)
    AUTO_THEME = "[theme]\nname = 'catppuccin'\nauto_switch = true\n"
    case(b"\x1b", theme=AUTO_THEME, loading=True)
    case(
        b"exact terminal\r", theme=AUTO_THEME,
        appearance_reports=[
            (b"\x1b[?997;2n", b"48;2;239;241;245"),
            (b"\x1b[?997;1n", b"48;2;24;24;37"),
        ],
        expected=[["tab", "focus", "w3:t1"]],
    )
    print(
        "PTY smoke scenarios passed (Escape including slow discovery, filtering, workspace/tab focus, nested worktrees, flat mode, failure recovery; resize and terminal cleanup)"
    )


if __name__ == "__main__":
    main()
