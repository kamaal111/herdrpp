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

One executable dispatches picker and PR commands. `src/picker` owns search,
navigation, rendering, and saved-theme support. `src/pr` owns GitHub association,
metadata reporting, and watcher lifecycle. The shared command runner owns
bounded subprocess I/O, process-group cleanup, timeout, and cancellation.
Feature snapshots retain the distinct validation they need.

Plugin settings have feature sections, starting with `[picker]`. Extend this
schema with explicit settings when adding a feature; unknown settings are errors.
The picker never depends on successful GitHub discovery.

The PR source is `herdrpp`. `pr_number` contains only `#123`; exactly one
lifecycle token contains the styled number and check icon. Reports clear
inactive tokens atomically. Checkout selection prefers explicit worktree paths,
then the focused pane's foreground cwd or original cwd in the active tab.
Tracking remotes take priority over origin; inherited `GH_REPO` is ignored.
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
Once completed, setup does not reapply after user edits or deletion.

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
