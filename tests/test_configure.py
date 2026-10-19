"""Exercise config migrations through the release CLI with isolated user files."""

import concurrent.futures
import json
import os
import subprocess
import pytest

TOKENS = ("$pr_open", "$pr_draft", "$pr_merged", "$pr_closed")
CI_STYLES = [
    {"token": "$ci_passed", "fg": "#238636"},
    {"token": "$ci_failed", "fg": "#f85149"},
    {"token": "$ci_pending", "fg": "#d29922"},
]


class TestConfigure:
    @pytest.fixture(autouse=True)
    def setup(self, tmp_path, release_binary):
        self.binary = release_binary
        self.root = tmp_path
        self.config = self.root / "config.toml"
        self.state = self.root / "state"
        self.env = dict(
            os.environ,
            HOME=str(self.root),
            HERDR_CONFIG_PATH=str(self.config),
            XDG_STATE_HOME=str(self.state),
        )

    def run_configure(self):
        return subprocess.run(
            [self.binary, "configure"],
            env=self.env,
            text=True,
            capture_output=True,
            check=False,
        )

    def markers(self):
        return list(self.state.rglob("migration.json"))

    def test_first_run_creates_styled_rows_and_external_marker(self):
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        assert all((token in self.config.read_text() for token in TOKENS))
        assert '"branch", "git_status"' in self.config.read_text()
        assert len(self.markers()) == 1
        assert json.loads(self.markers()[0].read_text())["version"] == 4

    def test_rerun_preserves_user_edits_bytes_and_modification_time(self):
        assert self.run_configure().returncode == 0
        self.config.write_text("user intentionally removed tokens; invalid TOML")
        before = self.config.stat().st_mtime_ns
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        assert (
            self.config.read_text() == "user intentionally removed tokens; invalid TOML"
        )
        assert self.config.stat().st_mtime_ns == before

    def test_rerun_preserves_deleted_config(self):
        assert self.run_configure().returncode == 0
        self.config.unlink()
        assert self.run_configure().returncode == 0
        assert not self.config.exists()

    def test_custom_rows_comments_and_styles_survive_legacy_replacement(self):
        original = """# keep my theme
[theme]
name = 'terminal' # leave this alone
[ui.sidebar.spaces]
row_gap = 2
rows = [["workspace", "$pr_number", "branch"], ["$mine"]] # my rows
"""
        self.config.write_text(original)
        self.config.chmod(0o640)
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        actual = self.config.read_text()
        assert "name = 'terminal' # leave this alone" in actual
        assert "# my rows" in actual
        assert '"$mine"' in actual
        assert "$pr_number" not in actual
        assert self.config.stat().st_mode & 511 == 0o640
        assert next(self.state.rglob("before-1.toml")).read_text() == original

    def test_existing_styles_are_preserved_and_missing_tokens_added(self):
        self.config.write_text("""[ui.sidebar.spaces]
rows = [[{token="workspace", bold=false}, {token="$pr_open", fg="#abc"}]]
""")
        assert self.run_configure().returncode == 0
        actual = self.config.read_text()
        assert 'fg="#abc"' in actual
        assert actual.count("$pr_open") == 1
        assert all((token in actual for token in TOKENS))

    def test_legacy_displays_on_separate_rows_are_both_upgraded(self):
        self.config.write_text(
            '[ui.sidebar.spaces]\nrows = [["workspace", "$pr_number"], ["branch", "$pr_number"]]\n'
        )
        assert self.run_configure().returncode == 0
        actual = self.config.read_text()
        assert actual.count("$pr_open") == 2
        assert "$pr_number" not in actual

    def test_no_workspace_gets_separate_row(self):
        self.config.write_text('[ui.sidebar.spaces]\nrows = [["branch"]]\n')
        assert self.run_configure().returncode == 0
        assert all((token in self.config.read_text() for token in TOKENS))
        assert '["branch"]' in self.config.read_text()

    def test_already_configured_records_completion_without_rewriting(self):
        original = '[ui.sidebar.spaces]\nrows = [["$pr_open", "$pr_draft", "$pr_merged", "$pr_closed", {token="$ci_passed", fg="#238636"}, {token="$ci_failed", fg="#f85149"}, {token="$ci_pending", fg="#d29922"}]]\n[[keys.command]]\nkey = "cmd+p"\ntype = "plugin_action"\ncommand = "herdrpp.open"\n'
        original += '[[keys.command]]\nkey = "cmd+shift+k"\ntype = "plugin_action"\ncommand = "herdrpp.commands"\n'
        original += '[[keys.command]]\nkey = "cmd+shift+r"\ntype = "plugin_action"\ncommand = "herdrpp.tasks"\n'
        self.config.write_text(original)
        before = self.config.stat().st_mtime_ns
        assert self.run_configure().returncode == 0
        assert self.config.read_text() == original
        assert self.config.stat().st_mtime_ns == before
        assert len(self.markers()) == 1

    def test_invalid_toml_is_not_changed_or_marked(self):
        self.config.write_text("invalid = [")
        result = self.run_configure()
        assert result.returncode != 0
        assert self.config.read_text() == "invalid = ["
        assert self.markers() == []

    def test_incompatible_structure_is_not_changed_or_marked(self):
        original = "ui = false\n"
        self.config.write_text(original)
        assert self.run_configure().returncode != 0
        assert self.config.read_text() == original
        assert self.markers() == []

    def test_full_row_is_rejected_without_writes(self):
        original = (
            "[ui.sidebar.spaces]\nrows = [[" + ", ".join(['"workspace"'] * 16) + "]]\n"
        )
        self.config.write_text(original)
        assert self.run_configure().returncode != 0
        assert self.config.read_text() == original
        assert self.markers() == []

    def test_full_layout_without_workspace_is_rejected_without_writes(self):
        original = (
            "[ui.sidebar.spaces]\nrows = [" + ", ".join(['["branch"]'] * 16) + "]\n"
        )
        self.config.write_text(original)
        assert self.run_configure().returncode != 0
        assert self.config.read_text() == original
        assert self.markers() == []

    def test_symlink_target_and_link_are_preserved(self):
        target = self.root / "actual.toml"
        target.write_text("# actual file\n")
        self.config.symlink_to(target.name)
        assert self.run_configure().returncode == 0
        assert self.config.is_symlink()
        assert "$pr_open" in target.read_text()
        self.env["HERDR_CONFIG_PATH"] = str(target)
        before = target.stat().st_mtime_ns
        assert self.run_configure().returncode == 0
        assert target.stat().st_mtime_ns == before
        assert len(self.markers()) == 1

    def test_distinct_paths_are_configured_independently(self):
        assert self.run_configure().returncode == 0
        other = self.root / "other.toml"
        self.env["HERDR_CONFIG_PATH"] = str(other)
        assert self.run_configure().returncode == 0
        assert "$pr_open" in other.read_text()
        assert len(self.markers()) == 2

    def test_missing_marker_retries_without_duplicates(self):
        assert self.run_configure().returncode == 0
        original = self.config.read_text()
        self.markers()[0].unlink()
        assert self.run_configure().returncode == 0
        assert self.config.read_text() == original
        assert self.config.read_text().count("$pr_open") == 1

    def test_pending_migration_runs_once_and_preserves_original_backup(self):
        self.config.write_text("# original settings\n")
        assert self.run_configure().returncode == 0
        marker = self.markers()[0]
        state = json.loads(marker.read_text())
        state["version"] = 0
        marker.write_text(json.dumps(state))
        self.config.write_text("# settings before migration\n")
        assert self.run_configure().returncode == 0
        assert "# settings before migration" in self.config.read_text()
        assert "$pr_open" in self.config.read_text()
        assert json.loads(marker.read_text())["version"] == 4
        assert (
            next(self.state.rglob("before-1.toml")).read_text()
            == "# original settings\n"
        )
        before = self.config.stat().st_mtime_ns
        assert self.run_configure().returncode == 0
        assert self.config.stat().st_mtime_ns == before

    def test_newer_migration_state_is_preserved_by_older_binary(self):
        assert self.run_configure().returncode == 0
        marker = self.markers()[0]
        state = json.loads(marker.read_text())
        state["version"] = 5
        marker.write_text(json.dumps(state))
        self.config.write_text("# customized by a newer version\n")
        assert self.run_configure().returncode == 0
        assert self.config.read_text() == "# customized by a newer version\n"
        assert json.loads(marker.read_text())["version"] == 5

    def test_unreadable_config_target_is_not_marked(self):
        self.config.mkdir()
        assert self.run_configure().returncode != 0
        assert self.config.is_dir()
        assert self.markers() == []

    def test_concurrent_runs_apply_migration_once(self):
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(lambda _: self.run_configure(), range(4)))
        assert [result.returncode for result in results] == [0] * 4
        assert self.config.read_text().count("$pr_open") == 1
        assert len(self.markers()) == 1

    def test_default_path(self):
        del self.env["HERDR_CONFIG_PATH"]
        assert self.run_configure().returncode == 0
        assert "$pr_open" in (self.root / ".config/herdr/config.toml").read_text()

    def test_first_run_adds_picker_shortcut(self):
        import tomllib
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        bindings = tomllib.loads(self.config.read_text())["keys"]["command"]
        assert bindings == [{"key": "cmd+p", "type": "plugin_action", "command": "herdrpp.open", "description": "Search workspaces, worktrees and tabs"}, {"key":"cmd+shift+k", "type":"plugin_action", "command":"herdrpp.commands", "description":"Open command palette"}, {"key":"cmd+shift+r", "type":"plugin_action", "command":"herdrpp.tasks", "description":"Run workspace tasks"}]

    def test_conflicting_shortcut_is_preserved_and_reported_once(self):
        original = '[[keys.command]]\nkey = "cmd+p"\ntype = "command"\ncommand = "existing"\n'
        self.config.write_text(original)
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        assert "Cmd+P" in result.stdout
        assert "already bound" in result.stdout
        assert original in self.config.read_text()
        assert "herdrpp.open" not in self.config.read_text()
        before = self.config.read_bytes()
        assert self.run_configure().returncode == 0
        assert self.config.read_bytes() == before

    def test_existing_picker_shortcut_is_not_duplicated(self):
        self.config.write_text('[[keys.command]]\nkey = "cmd+p"\ntype = "plugin_action"\ncommand = "herdrpp.open"\n')
        assert self.run_configure().returncode == 0
        assert self.config.read_text().count("herdrpp.open") == 1

    def test_inline_shortcut_array_is_preserved(self):
        self.config.write_text('[keys]\ncommand = [{key="prefix+f", type="plugin_action", command="herdrpp.open"}]\n')
        assert self.run_configure().returncode == 0
        assert 'key="prefix+f"' in self.config.read_text()
        assert self.config.read_text().count("herdrpp.open") == 2

    def test_invalid_shortcut_structure_prevents_sidebar_write(self):
        original = '[keys]\ncommand = false\n'
        self.config.write_text(original)
        assert self.run_configure().returncode != 0
        assert self.config.read_text() == original
        assert self.markers() == []

    def seed_version_one(self, original):
        assert self.run_configure().returncode == 0
        marker = self.markers()[0]
        state = json.loads(marker.read_text())
        state["version"] = 1
        marker.write_text(json.dumps(state))
        self.config.write_text(original)
        return marker

    def test_version_one_upgrade_only_adds_palette_shortcut_and_preserves_customizations(self):
        original = '# custom\n[ui.sidebar.spaces]\nrows = [["branch"]]\n'
        marker = self.seed_version_one(original)
        backup = next(self.state.rglob("before-2.toml"), None)
        assert backup is None
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        import tomllib
        assert "# custom" in self.config.read_text()
        assert tomllib.loads(self.config.read_text())["ui"]["sidebar"]["spaces"]["rows"] == [["branch"], CI_STYLES]
        assert "herdrpp.commands" in self.config.read_text()
        assert "herdrpp.open" not in self.config.read_text()
        assert "$pr_open" not in self.config.read_text()
        assert next(self.state.rglob("before-2.toml")).read_text() == original
        assert json.loads(marker.read_text())["version"] == 4
        self.config.write_text("# intentionally removed palette\n")
        before = self.config.stat().st_mtime_ns
        assert self.run_configure().returncode == 0
        assert self.config.read_text() == "# intentionally removed palette\n"
        assert self.config.stat().st_mtime_ns == before

    @pytest.mark.parametrize("key", ["cmd+shift+k", "SHIFT+CMD+K", "k+super+shift"])
    def test_conflicting_palette_shortcuts_are_preserved_and_reported(self, key):
        original = f'[[keys.command]]\nkey="{key}"\ntype="shell"\ncommand="existing"\n'
        self.seed_version_one(original)
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        assert original in self.config.read_text()
        assert "Cmd+Shift+K is already bound" in result.stdout
        assert "herdrpp.commands" in result.stdout
        assert "prefix+shift+f" in result.stdout

    def test_builtin_binding_conflict_is_preserved(self):
        original = '[keys]\nhelp="cmd+shift+k"\n'
        self.seed_version_one(original)
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        assert original in self.config.read_text()
        assert "already bound" in result.stdout

    def test_existing_equivalent_palette_binding_is_not_duplicated(self):
        original = '[keys]\ncommand=[{key="shift+super+k",type="plugin_action",command="herdrpp.commands"}]\n'
        self.seed_version_one(original)
        assert self.run_configure().returncode == 0
        import tomllib
        assert tomllib.loads(self.config.read_text())["keys"]["command"][0] == {"key":"shift+super+k", "type":"plugin_action", "command":"herdrpp.commands"}
        assert self.config.read_text().count("herdrpp.commands") == 1

    def test_version_two_config_write_without_marker_can_retry_without_duplicates(self):
        marker = self.seed_version_one("# user config\n")
        assert self.run_configure().returncode == 0
        original = self.config.read_text()
        state = json.loads(marker.read_text())
        state["version"] = 1
        marker.write_text(json.dumps(state))
        before = self.config.stat().st_mtime_ns
        assert self.run_configure().returncode == 0
        assert self.config.read_text() == original
        assert self.config.stat().st_mtime_ns == before
        assert self.config.read_text().count("herdrpp.commands") == 1

    def seed_version_two(self, original):
        assert self.run_configure().returncode == 0
        marker = self.markers()[0]
        state = json.loads(marker.read_text())
        state["version"] = 2
        marker.write_text(json.dumps(state))
        self.config.write_text(original)
        return marker

    def test_version_two_upgrade_adds_only_task_shortcut_and_backs_up_original(self):
        original = '# custom\n[ui.sidebar.spaces]\nrows=[["branch"]]\n'
        marker = self.seed_version_two(original)
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        import tomllib
        assert "# custom" in self.config.read_text()
        assert tomllib.loads(self.config.read_text())["ui"]["sidebar"]["spaces"]["rows"] == [["branch"], CI_STYLES]
        assert "herdrpp.tasks" in self.config.read_text()
        assert "herdrpp.commands" not in self.config.read_text()
        assert "herdrpp.open" not in self.config.read_text()
        assert "$pr_open" not in self.config.read_text()
        assert next(self.state.rglob("before-3.toml")).read_text() == original
        assert json.loads(marker.read_text())["version"] == 4

    @pytest.mark.parametrize("key", ["cmd+shift+r", "SHIFT+CMD+R", "r+super+shift"])
    def test_conflicting_task_shortcut_is_preserved_and_reported_once(self, key):
        original = f'[[keys.command]]\nkey="{key}"\ntype="shell"\ncommand="existing"\n'
        self.seed_version_two(original)
        result = self.run_configure()
        assert result.returncode == 0, result.stderr
        assert original in self.config.read_text()
        assert "$ci_passed" in self.config.read_text()
        assert "Cmd+Shift+R is already bound" in result.stdout
        assert "herdrpp.tasks" in result.stdout
        assert "prefix+shift+r" in result.stdout
        assert self.run_configure().stdout.count("already bound") == 0

    def test_task_migration_retry_does_not_duplicate_shortcut(self):
        marker = self.seed_version_two("# user config\n")
        assert self.run_configure().returncode == 0
        original = self.config.read_bytes()
        state = json.loads(marker.read_text())
        state["version"] = 2
        marker.write_text(json.dumps(state))
        assert self.run_configure().returncode == 0
        assert self.config.read_bytes() == original
        assert self.config.read_text().count("herdrpp.tasks") == 1


    def test_version_three_adds_only_ci_beside_existing_pr_styles(self):
        original = '# custom\n[ui.sidebar.spaces]\nrows=[["workspace", {token="$pr_open", fg="#abc"}, "branch"]]\n'
        marker = self.seed_version_two(original)
        state = json.loads(marker.read_text())
        state["version"] = 3
        marker.write_text(json.dumps(state))
        assert self.run_configure().returncode == 0
        import tomllib
        rows = tomllib.loads(self.config.read_text())["ui"]["sidebar"]["spaces"]["rows"]
        assert rows == [["workspace", {"token": "$pr_open", "fg": "#abc"}, *CI_STYLES, "branch"]]
        assert "herdrpp.tasks" not in self.config.read_text()
        assert "$pr_closed" not in self.config.read_text()
        assert next(self.state.rglob("before-4.toml")).read_text() == original
        before = self.config.read_bytes()
        assert self.run_configure().returncode == 0
        assert self.config.read_bytes() == before

    def test_ci_migration_preserves_existing_colored_token_without_duplication(self):
        original = '[ui.sidebar.spaces]\nrows=[[{token="$ci_passed", fg="#abc"}]]\n'
        marker = self.seed_version_two(original)
        state = json.loads(marker.read_text())
        state["version"] = 3
        marker.write_text(json.dumps(state))
        assert self.run_configure().returncode == 0
        assert self.config.read_text() == original
        assert json.loads(marker.read_text())["version"] == 4

    def test_version_three_upgrade_has_one_ci_migration_and_backup(self):
        original = '# custom\n[ui.sidebar.spaces]\nrows=[["workspace", {token="$mine", fg="#abc"}]]\n'
        marker = self.seed_version_two(original)
        state = json.loads(marker.read_text())
        state["version"] = 3
        marker.write_text(json.dumps(state))
        assert self.run_configure().returncode == 0
        import tomllib
        rows = tomllib.loads(self.config.read_text())["ui"]["sidebar"]["spaces"]["rows"]
        assert rows == [["workspace", *CI_STYLES, {"token": "$mine", "fg": "#abc"}]]
        assert next(self.state.rglob("before-4.toml")).read_text() == original
        assert list(self.state.rglob("before-5.toml")) == []
        assert json.loads(marker.read_text())["version"] == 4
        self.config.write_text(original)
        assert self.run_configure().returncode == 0
        assert self.config.read_text() == original

    def test_ci_migration_rejects_full_row_without_writes(self):
        original = '[ui.sidebar.spaces]\nrows=[["workspace", ' + ', '.join(['"branch"'] * 15) + ']]\n'
        marker = self.seed_version_two(original)
        state = json.loads(marker.read_text())
        state["version"] = 3
        marker.write_text(json.dumps(state))
        assert self.run_configure().returncode != 0
        assert self.config.read_text() == original
        assert json.loads(marker.read_text())["version"] == 3
