"""Exercise actual task palette owners and persistent launches through Herdr I/O."""
import fcntl
import json
import os
from pathlib import Path
import socketserver
import subprocess
import threading
import tempfile

import pytest
from test_palette import PaletteOwnerHarness

FAKE = '''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
root = Path(os.environ["PALETTE_TEST_ROOT"])
args = sys.argv[1:]
with (root / "calls").open("a") as log:
    log.write(json.dumps(args) + "\\n")
if args == ["api", "snapshot"]:
    result = {"snapshot":json.loads((root / "snapshot").read_text())}
elif args[:3] == ["plugin", "pane", "open"]:
    (root / "request-next").write_text(args[args.index("--env")+1].split("=",1)[1])
    (root / "request-next").replace(root / "request")
    result = {}
else:
    result = {}
print(json.dumps({"result":result}))
'''


class TaskOwnerHarness(PaletteOwnerHarness):
    action = "tasks"

    @pytest.fixture(autouse=True)
    def tasks_setup(self, setup):
        self.checkout = self.root / "checkout with spaces"
        (self.checkout / ".herdr").mkdir(parents=True)
        self.task_file = self.checkout / ".herdr/tasks.json"
        self.task_file.write_text('[{"label":"dev","command":"just","args":["dev-server"]}]')
        (self.root / "herdr").write_text(FAKE)
        self.snapshot = {
            "workspaces":[{"workspace_id":"w2", "worktree":{"checkout_path":str(self.checkout)}}],
            "tabs":[{"tab_id":"w2:t7", "workspace_id":"w2"}],
            "panes":[self.pane("w2:p8", "w2:t7")],
        }
        self.save_snapshot()
        self.requests = []
        self.busy = set()
        self.unknown = set()
        self.interrupt_stops = True
        self.next_destination = 30
        socket_directory = tempfile.TemporaryDirectory(prefix="herdr-task-", dir="/tmp")
        self.socket = str(Path(socket_directory.name) / "api.sock")
        self.env.update(HERDR_SOCKET_PATH=self.socket, HERDR_PLUGIN_STATE_DIR=str(self.root / "plugin-state"))
        owner = self

        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                req = json.loads(self.rfile.readline())
                owner.requests.append(req)
                result = owner.respond(req["method"], req["params"])
                self.wfile.write(json.dumps({"id":req["id"],"result":result}).encode() + b"\n")

        server = socketserver.UnixStreamServer(self.socket, Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        yield
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        socket_directory.cleanup()

    def pane(self, pane, tab, workspace="w2"):
        return {"pane_id":pane,"tab_id":tab,"workspace_id":workspace,"terminal_id":f"tty-{pane}","cwd":str(self.checkout)}

    def save_snapshot(self):
        path = self.root / "snapshot"
        pending = self.root / "snapshot-pending"
        pending.write_text(json.dumps(self.snapshot))
        pending.replace(path)

    def respond(self, method, params):
        if method == "pane.process_info":
            pane = params["pane_id"]
            info = {"pane_id":pane,"shell_pid":10,"foreground_process_group_id":10,
                    "foreground_processes":[{"pid":10,"name":"zsh"}]}
            if pane in self.busy:
                info.update(foreground_process_group_id=20,foreground_processes=[{"pid":20,"name":"just"}])
            if pane in self.unknown:
                info = {"pane_id":pane}
            return {"process_info":info}
        if method == "tab.create":
            self.next_destination += 1
            workspace = params["workspace_id"]
            tab = f"{workspace}:t{self.next_destination}"
            pane = f"{workspace}:p{self.next_destination}"
            self.snapshot["tabs"].append({"tab_id":tab,"workspace_id":workspace})
            self.snapshot["panes"].append(self.pane(pane, tab, workspace))
            self.save_snapshot()
            return {"tab":{"tab_id":tab}}
        if method == "pane.split":
            self.next_destination += 1
            opening = next(p for p in self.snapshot["panes"] if p["pane_id"] == params["target_pane_id"])
            pane = self.pane(f"w2:p{self.next_destination}", opening["tab_id"])
            self.snapshot["panes"].append(pane)
            self.save_snapshot()
            return {"pane":pane}
        if method == "pane.send_keys" and params["keys"] == ["ctrl+c"] and self.interrupt_stops:
            self.busy.discard(params["pane_id"])
        return {}

    def choose(self, directory):
        return self.select(directory, {"id":"task:0","input":None,"confirmed":False})

    def launch(self):
        (self.root / "request").unlink(missing_ok=True)
        directory = self.start()
        self.choose(directory)
        return self.requests[-1]["params"]["pane_id"]

    def finish_error(self, directory):
        (directory / "selection.json").write_text(json.dumps({"id":"task:0","input":None,"confirmed":False}))
        self.close_popup()
        _, stderr = self.process.communicate(timeout=12)
        assert self.process.returncode == 1
        return stderr


class TestTaskOwner(TaskOwnerHarness):
    def test_default_task_creates_tab_and_reruns_same_pane_from_other_tab(self):
        pane = self.launch()
        self.busy.add(pane)
        self.env["HERDR_PLUGIN_CONTEXT_JSON"] = json.dumps({"workspace_id":"w2","tab_id":self.snapshot["tabs"][-1]["tab_id"],"focused_pane_id":pane})
        assert self.launch() == pane
        assert len([r for r in self.requests if r["method"] == "tab.create"]) == 1
        assert len([r for r in self.requests if r["method"] == "pane.send_text"]) == 2
        assert len([r for r in self.requests if "ctrl+c" in r["params"].get("keys", [])]) == 1
        text = next(r["params"]["text"] for r in self.requests if r["method"] == "pane.send_text")
        assert "/bin/sh -c" in text
        assert "checkout with spaces" in text
        assert "dev-server" in text

    def test_same_tab_uses_idle_opening_pane_and_completed_task_reuses_it(self):
        self.task_file.write_text('[{"label":"dev","command":"true","new_tab":false}]')
        self.snapshot["panes"].insert(0, self.pane("w2:p9", "w2:t7"))
        self.save_snapshot()
        assert self.launch() == "w2:p8"
        assert self.launch() == "w2:p8"
        assert not any(r["method"] in ("tab.create", "pane.split") for r in self.requests)
        assert not any("ctrl+c" in r["params"].get("keys", []) for r in self.requests)

    @pytest.mark.parametrize("tab,workspace", [("w2:t9", "w2"), ("w3:t9", "w3")])
    def test_same_tab_does_not_take_idle_panes_from_other_tabs_or_workspaces(self, tab, workspace):
        self.task_file.write_text('[{"label":"dev","command":"true","new_tab":false}]')
        self.busy.add("w2:p8")
        self.snapshot["workspaces"].append({"workspace_id":"w3", "worktree":{"checkout_path":str(self.checkout)}})
        self.snapshot["tabs"].append({"tab_id":tab,"workspace_id":workspace})
        self.snapshot["panes"].append(self.pane("other:p9", tab, workspace))
        self.save_snapshot()
        assert self.launch() not in ("w2:p8", "other:p9")
        assert len([r for r in self.requests if r["method"] == "pane.split"]) == 1
        assert not any("ctrl+c" in r["params"].get("keys", []) for r in self.requests)

    def test_same_tab_uses_another_idle_pane_without_interrupting_busy_opening_pane(self):
        self.task_file.write_text('[{"label":"dev","command":"true","new_tab":false}]')
        self.snapshot["panes"].append(self.pane("w2:p9", "w2:t7"))
        self.save_snapshot()
        self.busy.add("w2:p8")
        assert self.launch() == "w2:p9"
        assert "w2:p8" in self.busy
        assert not any(r["method"] == "pane.split" for r in self.requests)

    @pytest.mark.parametrize("state", ["busy", "unknown"])
    def test_busy_or_unknown_opening_pane_requires_split(self, state):
        self.task_file.write_text('[{"label":"dev","command":"true","new_tab":false}]')
        getattr(self, state).add("w2:p8")
        assert self.launch() != "w2:p8"
        split = next(r["params"] for r in self.requests if r["method"] == "pane.split")
        assert split["target_pane_id"] == "w2:p8"
        assert split["direction"] == "right"
        assert not any("ctrl+c" in r["params"].get("keys", []) for r in self.requests)

    def test_closed_destination_is_replaced(self):
        previous = self.launch()
        self.snapshot["panes"] = self.snapshot["panes"][:1]
        self.snapshot["tabs"] = self.snapshot["tabs"][:1]
        self.save_snapshot()
        assert self.launch() != previous
        assert len([r for r in self.requests if r["method"] == "tab.create"]) == 2
        assert len([r for r in self.requests if r["method"] == "pane.send_text"]) == 2

    def test_interrupt_timeout_preserves_process_and_does_not_submit_replacement(self):
        pane = self.launch()
        self.busy.add(pane)
        self.interrupt_stops = False
        (self.root / "request").unlink()
        directory = self.start()
        stderr = self.finish_error(directory)
        assert "did not become idle" in stderr
        assert pane in self.busy
        assert len([r for r in self.requests if r["method"] == "pane.send_text"]) == 1
        assert len([r for r in self.requests if r["method"] == "tab.create"]) == 1

    def test_invalid_file_shows_all_fixes_and_cannot_execute(self):
        self.task_file.write_text('[{"label":"dev","command":null,"new_tab":"false","arg":[]},{"label":"dev"}]')
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert catalog["items"] == []
        assert catalog["screen"]["diagnostics"] is True
        assert str(self.task_file) in catalog["warning"]
        assert "Remove the quotes" in catalog["warning"]
        assert "duplicates Task 1" in catalog["warning"]
        assert "unknown field" in catalog["warning"]
        stderr = self.finish_error(directory)
        assert "unknown palette command" in stderr
        assert self.requests == []

    @pytest.fixture(params=[False, True], ids=["worktree", "git-root"])
    def ancestor_checkout(self, request):
        if request.param:
            subprocess.run(["git", "init", str(self.checkout)], check=True, capture_output=True)
            self.snapshot["workspaces"][0].pop("worktree")
            self.save_snapshot()

    def test_ancestor_tasks_launch_from_checkout(self, ancestor_checkout):
        parent_file = self.root / ".herdr/tasks.json"
        parent_file.parent.mkdir()
        self.task_file.replace(parent_file)
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert [item["title"] for item in catalog["items"]] == ["dev"]
        self.choose(directory)
        created = next(r["params"] for r in self.requests if r["method"] == "tab.create")
        assert created["cwd"] == str(self.checkout.resolve())
        text = next(r["params"]["text"] for r in self.requests if r["method"] == "pane.send_text")
        assert text.startswith("/bin/sh -c 'cd ")
        assert str(self.checkout.resolve()) in text

    @pytest.mark.parametrize("contents,diagnostic", [
        ("[]", "No tasks configured"),
        ("[{", "line 1 column"),
        ('[{"label":"broken","command":null}]', "nonblank string"),
    ])
    def test_nearest_ancestor_empty_or_invalid_file_blocks_fallback(self, contents, diagnostic):
        farther_file = self.root / ".herdr/tasks.json"
        farther_file.parent.mkdir()
        self.task_file.replace(farther_file)
        nearest = self.root / "projects"
        (nearest / ".herdr").mkdir(parents=True)
        nearest_file = nearest / ".herdr/tasks.json"
        nearest_file.write_text(contents)
        checkout = nearest / "api"
        checkout.mkdir()
        self.snapshot["workspaces"][0]["worktree"]["checkout_path"] = str(checkout)
        self.save_snapshot()
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert catalog["items"] == []
        assert str(nearest_file) in catalog["warning"]
        assert diagnostic in catalog["warning"]
        self.select(directory, None)
        assert self.requests == []

    def test_checkout_tasks_override_parent_tasks(self):
        parent_file = self.root / ".herdr/tasks.json"
        parent_file.parent.mkdir()
        parent_file.write_text('[{"label":"parent","command":"false"}]')
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert [item["title"] for item in catalog["items"]] == ["dev"]
        self.select(directory, None)
        assert self.requests == []

    def test_missing_file_explains_ancestor_search(self):
        self.task_file.unlink()
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert catalog["items"] == []
        assert str(self.task_file) in catalog["warning"]
        assert "Searched the checkout and up to 10 parent folders." in catalog["warning"]
        self.select(directory, None)
        assert self.requests == []

    def test_corrected_file_is_reloaded_on_next_open(self):
        self.task_file.write_text('[{"label":"dev","command":null}]')
        directory = self.start()
        self.select(directory, None)
        self.task_file.write_text('[{"label":"dev","command":"true"}]')
        assert self.launch().startswith("w2:p")
        assert len([r for r in self.requests if r["method"] == "pane.send_text"]) == 1

    def test_session_lock_prevents_concurrent_launch(self):
        self.launch()
        lock_path = next((self.root / "plugin-state").rglob("tasks.lock"))
        with lock_path.open("r+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            (self.root / "request").unlink()
            directory = self.start()
            stderr = self.finish_error(directory)
        assert "another task launch" in stderr
        assert len([r for r in self.requests if r["method"] == "pane.send_text"]) == 1

    @pytest.mark.parametrize("contents", [None, "[]"])
    def test_missing_or_empty_file_shows_path_and_example(self, contents):
        self.task_file.unlink()
        self.prepare_contents(contents)
        directory = self.start()
        catalog = json.loads((directory / "catalog.json").read_text())
        assert catalog["items"] == []
        assert "No tasks configured" in catalog["warning"]
        assert str(self.task_file) in catalog["warning"]
        assert '"command":"just"' in catalog["warning"]
        self.select(directory, None)
        assert self.requests == []

    def prepare_contents(self, contents):
        if contents is not None:
            self.task_file.write_text(contents)

    def test_git_root_is_resolved_from_foreground_subdirectory(self):
        subprocess.run(["git", "init", str(self.checkout)], check=True, capture_output=True)
        subdir = self.checkout / "sources"
        subdir.mkdir()
        self.snapshot["workspaces"][0].pop("worktree")
        self.snapshot["panes"][0]["foreground_cwd"] = str(subdir)
        self.snapshot["panes"][0]["cwd"] = "/does-not-exist"
        self.save_snapshot()
        assert self.launch().startswith("w2:p")
        text = next(r["params"]["text"] for r in self.requests if r["method"] == "pane.send_text")
        assert "checkout with spaces" in text
        assert "sources" not in text

    def test_completed_task_pane_transfers_ownership_and_restarts_only_current_owner(self):
        self.task_file.write_text('[{"label":"first","command":"true","new_tab":false}]')
        assert self.launch() == "w2:p8"
        self.task_file.write_text('[{"label":"second","command":"true","new_tab":false}]')
        assert self.launch() == "w2:p8"
        self.task_file.write_text('[{"label":"third","command":"true","new_tab":false}]')
        assert self.launch() == "w2:p8"
        state = next((self.root / "plugin-state").rglob("tasks.json"))
        records = json.loads(state.read_text())
        assert [(r["label"], r["pane"]) for r in records] == [("third", "w2:p8")]
        assert not any(r["method"] in ("tab.create", "pane.split") for r in self.requests)
        assert not any("ctrl+c" in r["params"].get("keys", []) for r in self.requests)
        self.busy.add("w2:p8")
        assert self.launch() == "w2:p8"
        interrupts = [r["params"]["pane_id"] for r in self.requests if r["params"].get("keys") == ["ctrl+c"]]
        assert interrupts == ["w2:p8"]

    def test_displaced_task_does_not_interrupt_new_owner(self):
        self.task_file.write_text('[{"label":"build","command":"true","new_tab":false}]')
        assert self.launch() == "w2:p8"
        self.task_file.write_text('[{"label":"test","command":"true","new_tab":false}]')
        assert self.launch() == "w2:p8"
        self.busy.add("w2:p8")
        self.task_file.write_text('[{"label":"build","command":"true","new_tab":false}]')
        assert self.launch() != "w2:p8"
        assert "w2:p8" in self.busy
        assert not any("ctrl+c" in r["params"].get("keys", []) for r in self.requests)
        assert len([r for r in self.requests if r["method"] == "pane.send_text" and r["params"]["pane_id"] == "w2:p8"]) == 2
        assert len([r for r in self.requests if r["method"] == "pane.split"]) == 1

    @pytest.mark.parametrize("state", ["busy", "unknown"])
    def test_unavailable_task_pane_is_skipped_for_another_idle_pane(self, state):
        self.task_file.write_text('[{"label":"first","command":"true","new_tab":false}]')
        assert self.launch() == "w2:p8"
        getattr(self, state).add("w2:p8")
        self.snapshot["panes"].append(self.pane("w2:p9", "w2:t7"))
        self.save_snapshot()
        self.task_file.write_text('[{"label":"second","command":"true","new_tab":false}]')
        assert self.launch() == "w2:p9"
        assert not any(r["method"] in ("tab.create", "pane.split") for r in self.requests)
        assert not any("ctrl+c" in r["params"].get("keys", []) for r in self.requests)

    def test_displaced_new_tab_task_allocates_a_new_tab(self):
        first = self.launch()
        tab = self.snapshot["tabs"][-1]["tab_id"]
        self.env["HERDR_PLUGIN_CONTEXT_JSON"] = json.dumps({"workspace_id":"w2","tab_id":tab,"focused_pane_id":first})
        self.task_file.write_text('[{"label":"second","command":"true","new_tab":false}]')
        assert self.launch() == first
        self.busy.add(first)
        self.task_file.write_text('[{"label":"dev","command":"true"}]')
        assert self.launch() != first
        assert first in self.busy
        assert len([r for r in self.requests if r["method"] == "tab.create"]) == 2
        assert not any("ctrl+c" in r["params"].get("keys", []) for r in self.requests)

    def test_identical_task_labels_in_different_workspaces_get_distinct_destinations(self):
        first = self.launch()
        self.snapshot["workspaces"].append({"workspace_id":"w3", "worktree":{"checkout_path":str(self.checkout)}})
        self.snapshot["tabs"].append({"tab_id":"w3:t7","workspace_id":"w3"})
        self.snapshot["panes"].append(self.pane("w3:p8", "w3:t7", "w3"))
        self.save_snapshot()
        self.env["HERDR_PLUGIN_CONTEXT_JSON"] = json.dumps({"workspace_id":"w3","tab_id":"w3:t7","focused_pane_id":"w3:p8"})
        assert self.launch() != first
        assert len([r for r in self.requests if r["method"] == "tab.create"]) == 2

    def test_closing_opening_pane_while_palette_is_open_prevents_execution(self):
        directory = self.start()
        self.snapshot["panes"] = []
        self.save_snapshot()
        stderr = self.finish_error(directory)
        assert "original pane is no longer available" in stderr
        assert self.requests == []

    def test_checkout_change_does_not_reuse_previous_task_destination(self):
        first = self.launch()
        other = self.root / "another-checkout"
        (other / ".herdr").mkdir(parents=True)
        (other / ".herdr/tasks.json").write_bytes(self.task_file.read_bytes())
        self.snapshot["workspaces"][0]["worktree"]["checkout_path"] = str(other)
        self.save_snapshot()
        assert self.launch() != first
        assert len([r for r in self.requests if r["method"] == "tab.create"]) == 2

    def test_successful_tasks_keep_recent_order_after_reordering_and_isolate_checkouts(self):
        self.task_file.write_text('[{"label":"build","command":"true"},{"label":"dev","command":"true"},{"label":"test","command":"true"}]')
        directory = self.start()
        self.select(directory, {"id":"task:2", "input":None, "confirmed":False})
        (self.root / "request").unlink()
        directory = self.start()
        assert json.loads((directory / "catalog.json").read_text())["recent_ids"] == ["task:2"]
        self.select(directory, {"id":"task:1", "input":None, "confirmed":False})
        self.task_file.write_text('[{"label":"dev","command":"echo changed"},{"label":"test","command":"true"},{"label":"build","command":"true"}]')
        (self.root / "request").unlink()
        directory = self.start()
        assert json.loads((directory / "catalog.json").read_text())["recent_ids"] == ["task:0", "task:1"]
        self.select(directory, None)
        other = self.root / "another-checkout"
        (other / ".herdr").mkdir(parents=True)
        (other / ".herdr/tasks.json").write_bytes(self.task_file.read_bytes())
        self.snapshot["workspaces"][0]["worktree"]["checkout_path"] = str(other)
        self.save_snapshot()
        (self.root / "request").unlink()
        directory = self.start()
        assert json.loads((directory / "catalog.json").read_text())["recent_ids"] == []
        self.select(directory, None)

    def test_task_launch_failure_preserves_previous_history(self):
        pane = self.launch()
        history = self.root / "plugin-state/palette-history.json"
        before = history.read_bytes()
        self.busy.add(pane)
        self.interrupt_stops = False
        (self.root / "request").unlink()
        directory = self.start()
        stderr = self.finish_error(directory)
        assert "did not become idle" in stderr
        assert history.read_bytes() == before
