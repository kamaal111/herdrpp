# Machine launcher gates

`just test-machines` builds the pinned Herdr 0.9.5 environment and production
`herdrpp launch` binary once, then runs both real SSH and rendered graphics gates.
Use `sh tests/machines/run.sh ssh` or `sh tests/machines/run.sh graphics` for a
narrow run. `MACHINE_SKIP_BUILD=1` reuses existing images and binary;
`MACHINE_OUTPUT_DIR` overrides the default `target/machine-switching` artifacts.

SSH fixtures live inside disposable containers. Alpha and Beta use distinct
OpenSSH daemons and loopback ports, each routing to an independent Herdr session.
They share authentication fixtures and still run in one container on one physical
host. Production round-trip tests assert different profile targets and that both
SSH endpoints and Herdr servers remain alive. The runner
names `ssh_gate.py`, `ssh_failures_gate.py`, `navigator_gate.py`,
`navigator_lifecycle_gate.py`, and `launch_lifecycle_gate.py` explicitly so ordinary host pytest
does not collect these Docker-only gates.
JUnit results are written to `ssh.xml` in the output directory. The expanded
routing gates cover remembered remote selection, duplicate Unicode labels,
hidden sidebars with custom prefix and disabled GoTo, unreachable preflight,
nonzero candidate recovery, coalesced selection/typeahead, and direct remote
input before the first frame. Routing assertions inspect actual server pane
lines. `ssh-diagnostics/` retains terminal output and rendered text;
`binary.json` identifies the exact binary and Herdr version used.

The graphics gate runs the actual launcher and workspace picker inside reference
Kitty on Xvfb with Mesa software rendering. A continuously busy pane displays a
red image before the picker opens, then waits for a fixture handshake. Only
after the picker and its physical resize redraw are confirmed does the test
release the fixture to emit a blue image. A completion marker and exact emitted
control bytes prove emission happened before cancellation; the picker must
still hide both images at that point.
The test checks old and new image pixels after cancellation, a manual proxy
PTY resize, and actual Kitty X11 window resizing while the picker is open. The
bridge forwards the host SIGWINCH and complete winsize, then verifies matching
host/proxy geometry. It reopens the picker, restores the physical window size,
and checks both images again after cancellation.
The busy process PID and pane terminal IDs must survive. The offscreen Xvfb framebuffer stays at 1200×800 while the actual Kitty window
and host PTY change size; `graphics/host-resize.json` records both geometries.
`graphics/` contains rendered PNGs, raw output, exact binary SHA and terminal
versions, identity assertions, result counts, and an explicit PASS/FAIL outcome.
The real Kitty host also checks direct `--machine` startup: a command is sent
immediately after spawning the launcher, before reading its first output byte,
and its exact output line must reach Alpha. A second rendered gate switches Local → Alpha → Local → Alpha over real SSH,
checks the destination image colors, preserves the native viewer PID across
the round trip, rejects fallback diagnostics, and preserves both busy PIDs and
terminal IDs. Pixel retransmission is recorded without requiring it from a viewer
that can reuse its image cache.
Its artifacts are under `graphics-ssh/`. The runner checks both outcomes outside Kitty so a child assertion cannot be
mistaken for a successful terminal process exit.

Native terminal ownership tests are part of `just ready` in
`tests/test_machine_launch.py`. On macOS the kernel sets `PENDIN` when returning
from raw to canonical mode, including a plain Python `tty.setraw` / `tcsetattr`
roundtrip. Tests compare all other attributes exactly and mask only that
kernel-managed pending-input bit. Reference Kitty graphics evidence does not
claim native macOS GUI screenshots.

The native controller gate requires the same viewer PID across Alpha → Beta →
Local and rejects any fallback diagnostic. Unsupported geometry and empty native
views use the explicitly tested reattachment fallback. Native transaction output
is retained in `native-wire.ansi` and `native-screen.txt`.

The cold-local lifecycle gate starts without a prestarted Herdr server. A busy
pane must continue making progress while the actual picker is open. Quitting and
relaunching the viewer must preserve the server, workspace and terminal IDs, busy
process PID, and live shell state. `launch-lifecycle/` retains identity assertions
and both launcher transcripts. Native lifecycle gates also exercise both enhanced
Quit encodings during alias inspection, resize-triggered fallback, and owned
viewer SIGTERM cleanup while preserving both remote servers and user configuration.

The integrated picker gates (`picker_gate.py`) open the workspace picker through
Ctrl+] and configured Cmd+P, cycle Local/Alpha/Beta without changing attachment,
confirm an exact remote tab despite duplicate names, reject changed profiles and
closed destinations, and verify rollback. The same gates run on both approaches;
the navigator branch additionally verifies retained viewers and exact-tab fallback.
