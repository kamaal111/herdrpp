# Development and releases

Use stable Rust/Cargo (minimum Rust 1.89), Just, and uv 0.13.1.
`uv sync --locked` installs the pinned Python 3.15.0 test environment.
End-user installation uses system Python 3 and does not need uv or pytest.

## Local development

```sh
uv sync --locked
just ready
just ready-all
```

`ready` checks formatting, Clippy with warnings denied, Rust tests, isolated
Python installer/configuration/watcher integration tests, and PTY smoke tests.
`ready-all` additionally requires Docker and runs real Herdr focus and visual
regressions. Tests use temporary directories, fake Herdr/GitHub boundaries, and
isolated sessions, without personal credentials or plugin registry access.
After initial downloads, the native tests run offline.

Useful focused commands include `cargo test --locked --test picker_search`,
`uv run --locked pytest tests/test_configure.py`, and `just test-pty`.
Run `just` for all recipes. See [snapshots](snapshots.md) for Docker details.

## Command palette definitions

The palette registry lives in `src/palette/commands.toml` and is embedded in
the binary. Editing a definition and rebuilding ships the change; there is no
runtime registry file to install. User tasks still come from `.herdr/tasks.json`.

Commands using an existing Rust handler need only a TOML entry:

```toml
[[commands]]
id = "review:rename_tab"
title = "Rename Review Tab"
requires = ["workspace", "tab"]
handler = { kind = "rename", scope = "tab" }
steps = [
  { kind = "text", key = "name", initial = "current_label", validator = "visible_name" },
]
```

Use an empty or omitted `steps` array for immediate operations such as
`handler = { kind = "split", direction = "down" }`. A `confirm` step adds a
confirmation before execution. Text and confirmation steps can be composed;
Escape returns to the previous step and Ctrl+C cancels. The current handlers
accept one text value, `name`; handlers that require other inputs need an
explicit Rust binding rather than arbitrary JSON arguments.

Text steps support `initial` (`empty`, `current_label`, `suggested_name`),
`validator` (`visible_name`, `automatic_name`, `new_branch`), `select_all`,
`message`, `help`, and `error`. Defaults show the command title, use
`Enter Save  Esc Back`, and reject blank visible names. Messages support only
`{title}` and `{target}` substitutions. Handler contracts require the appropriate
validator and mandatory confirmation; `requires` adds availability restrictions
and cannot remove a handler's required target.

Screens in `[screens.commands]`, `[screens.tasks]`, and the branch screens own
search/loading/empty/help text, the popup entrypoint, diagnostic presentation,
history ordering, and whole-flow cancellation behavior. Definitions can set
`search` (`all`, `title`, `source_title`), `detail` (`source`, `id`), and `history`
(`command`, `task`). IDs and history identities should remain stable.

Dynamic commands use `template = true` definitions. Task, plugin, and branch
providers supply titles and typed operation data; templates own their search,
prompt, and presentation rules. A `choice` step selects a registered Rust
provider and screen, whose row templates describe the next interaction. The
worktree definitions demonstrate choosing a branch and then naming a checkout.

New domain behavior belongs in the typed bindings in `palette::registry`,
`palette::providers`, and the operation executor, reusing existing domain
services. It must not add command-identity cases to generic activation,
rendering, submission validation, or popup orchestration. Unknown properties,
duplicate IDs/templates, invalid handler properties, and incompatible steps
are rejected before a command can run. Registry tests include a TOML-only
alias exercised through the shared prompt, confirmation, and request builder.

## Local plugin

These commands intentionally modify your local Herdr registry or session;
verification does not run them automatically:

```sh
just plugin-link
just plugin-check
just plugin-check-pr
just plugin-start
just plugin-open
```

`plugin-link` builds and registers this checkout. Check validates discovery
without changing focus; check-pr inspects associations without writing metadata
or state. Other actions rebuild and invoke the selected session. Registry/action
recipes honor `HERDR_BIN_PATH`. Use `just plugin-stop` and `just plugin-unlink`
when finished; unlink preserves source files. Keep the checkout in place while
linked. Configure the viewing client separately with `herdrpp configure` or the
README example. `pick` and `watch` are internal lifecycle commands.

## Architecture

One executable dispatches picker, palette, task, and PR commands. `src/picker` owns search,
navigation, rendering, and saved-theme support. `src/pr` owns GitHub association,
metadata reporting, and watcher lifecycle. The shared command runner owns
bounded subprocess I/O, process-group cleanup, timeout, and cancellation.
Feature snapshots retain the distinct validation they need.

`src/search` owns the shared fuzzy field matcher and search-view components.
The picker retains hierarchy and destination ordering; `src/palette` builds a
flat command catalog and handles prompts. Both call the same matcher and view.
The palette uses the same native popup placement, geometry, and launch helper
as the picker. It exits after writing a selection to a private temporary mailbox;
the action waits for the popup PTY to be released before dispatching against the
opening context.
Plugin actions use the public session socket API to retain that context.
`src/tasks` owns strict task-file validation, checkout discovery, and remembered
terminal destinations. Tasks reuse the palette lifecycle with a task catalog
and scrollable diagnostics. Session-scoped destination records and a launch
lock prevent overlapping allocations. Process-info probes identify idle shells;
restarts use Ctrl+C only and fail after five seconds without a replacement.

Plugin settings have feature sections, starting with `[picker]`. Extend this
schema with explicit settings when adding a feature; unknown settings are errors.
The picker never depends on successful GitHub discovery.

The metadata source is `herdrpp`. Default branches use one of `ci_passed`,
`ci_failed`, or `ci_pending` and clear all PR tokens; other branches clear all CI tokens. `pr_number` contains only
`#123`; exactly one lifecycle token contains the styled number and check icon. Reports clear
inactive tokens atomically. Checkout selection prefers explicit worktree paths,
then the focused pane's foreground cwd or original cwd in the active tab.
Tracking remotes take priority, followed by a sole eligible remote, then the
GitHub CLI selection; inherited `GH_REPO` is ignored.
Repeated checkout contexts share one lookup per cycle. Workspace and branch
identity are rechecked before reporting.

One watcher runs per session socket identity. Refreshes cannot overlap. Binary
replacement and registry path changes trigger reload between cycles; failed
replacements retain the current watcher and retry. Stop, disable, unlink, and
session loss prevent reload. State and append-only logs live under
`HERDR_PLUGIN_STATE_DIR/session-*`; remove logs only while stopped.

## Viewing setup

`src/configure.rs` is the sole owner of viewing config writes. Setup version 1
adds sidebar tokens and an available Cmd+P binding in one atomic change.
Conflicting shortcuts are preserved and reported for manual resolution.
Completion, locks, and original backups live under
`${XDG_STATE_HOME:-$HOME/.local/state}/herdrpp/configs/`, keyed by resolved path.
Version 2 adds only the available Cmd+Shift+K palette binding. Version 3 adds
only the available Cmd+Shift+R workspace task binding. Each completed
migration stays complete after user edits or deletion; upgrades run only pending
migrations and keep per-migration backups.

Version 4 adds the colored CI state tokens beside the PR display or workspace,
preserving custom styles, removed PR tokens, and shortcuts. Completed setup
stays complete after user edits or removal of the CI display.

Advance the setup version only for an ordered, targeted change in required
configuration, never for ordinary releases or restoring user customizations.
Preserve unrelated edits and record completion only after successful persistence.
Add upgrade, interruption/retry, and rerun coverage when adding a migration.

## Release preparation

CI runs `just ready` on Linux and macOS. A Linux job runs all Docker focus and
visual checks, repeats captures, and uploads diagnostics. Baseline recording is
forbidden in CI. Require these checks before publishing a release.

The Release workflow uses Release Please with `GITHUB_TOKEN`. When creating the
GitHub repository later:

1. Create `kamaal111/herdrpp` with `main` as its default branch and push this
   project using a Conventional Commit such as `feat: introduce herdr++`.
2. In Settings → Actions → General, enable **Allow GitHub Actions to create and
   approve pull requests**. Workflows request contents, issues, pull-requests,
   and actions permissions for release automation.
3. Require `verify (ubuntu-latest)`, `verify (macos-latest)`, and
   `Visual snapshots and focus` through branch protection.
4. Review the generated v0.1.0 release PR and wait for CI. Merge preserving its
   generated title and release metadata. The next Release run creates the tag
   and GitHub Release; the public installer can then resolve that release.

The empty release manifest means no version has been released yet. Cargo and
plugin versions begin at 0.1.0. There is no bootstrap SHA from other history.
Release Please maintains Cargo.toml, Cargo.lock, herdr-plugin.toml, the release
manifest, and CHANGELOG.md together. No crates.io publication or binary uploads
are needed for Herdr's source build installation.

Use `fix:` for patch releases, `feat:` for features, and `!` or a BREAKING CHANGE
footer for breaking changes. Before 1.0, breaking changes bump the minor version.
`docs:` and `chore:` alone do not request a release. Use Conventional Commit
squash titles. The first release scans this repository's initial history.

The built-in token does not trigger normal PR workflows when it creates a
release PR. The Release workflow explicitly dispatches CI on each generated PR
branch, including updates. CI also supports manual workflow dispatch as a retry.
For transient release failures, rerun Release on main. No personal token is
required. GitHub creation, permissions, pushes, and publishing are separate
operator steps; local checks cannot prove hosted workflow execution.
