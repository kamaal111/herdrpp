# Focus and snapshot testing

Run Docker, then use:

```sh
just test-focus
just test-snapshots
```

The picker harness captures a real Herdr 0.9.5 terminal session, exercises focus
and pane preservation, task allocation/restarts, and renders controlled
loading/empty/tiny states plus task search and validation diagnostics. The
sidebar harness uses isolated server and viewing configurations and fake GitHub
responses to exercise PR colors, check icons, clearing, and config reload.
Both load the same combined plugin. Docker uses pinned base images, system
packages, Herdr binary checksum, fonts, Rust, and Python dependencies.

Picker output lives in `target/snapshots/picker/`; sidebar output lives in
`target/snapshots/sidebar/`. Case folders include actual/expected/diff images
and terminal/server diagnostics. Their separate build/cache directories avoid
cross-suite collisions. Baselines live under the corresponding feature folder
in `tests/snapshots/`.

`just test-snapshots` runs comparison self-tests and repeats both feature suites
twice to detect unstable captures. CI uploads diagnostics even when checks fail.
The macOS native job excludes Docker tests because hosted runners have no daemon;
Linux runs every suite.

## Intentional baseline changes

```sh
just record-picker-snapshots
just record-sidebar-snapshots
```

For one picker case, pass its name, for example
`just record-picker-snapshots showcase-dracula`. For one sidebar case:

```sh
uv run --locked pytest tests/test_sidebar_snapshots.py --record --snapshot-case pr-open-failed
```

The README's command and task showcases use real isolated popup sessions. The
task fixture runs harmless sample commands, checks current-tab execution and
task restart destination reuse, then captures the task list over running output.
To refresh only these two images and verify repeated captures:

```sh
just record-picker-snapshots showcase-commands showcase-tasks
sh tests/snapshots/run.sh --repeat 2 showcase-commands showcase-tasks
```

Inspect every changed image, then run comparison mode again. Never record in CI
or replace a baseline merely to hide a failing behavior. Recording is rejected
when `CI` is set. All recording and capture commands use isolated test settings,
not the user's live Herdr session.
