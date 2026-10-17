# Navigator bridge: research and compatibility evidence

This directory preserves the mechanism experiments that informed the native
controller. The production implementation is `herdrpp launch`, documented in
[the main README](../../README.md#machine-search). Its required tests are under
[`tests/machines`](../../tests/machines/README.md); historical results below do
not substitute for those production gates.

The investigated solution owns a local Herdr client, borrows its terminal for a
local machine picker, then drives Herdr's native GoTo navigator through a private
client configuration. Herdr itself is unmodified. The private configuration is
needed to support disabled/custom native navigator bindings and hidden layouts.

## Corrected public control chord: real-SSH proof passed

The launcher already reserves `Ctrl+]` for its machine picker. Assigning that
same public chord to native GoTo in the owned client avoids adding a second,
hidden reservation. The launcher intercepts physical `Ctrl+]`; only its native
controller injects that byte into the child PTY.

`test_public_reserved_control_chord_preserves_remote_control_f12_custom_command`
passed against real Herdr **0.9.5**, real SSH, and independent servers:

1. The private viewing-client config adds `ctrl+]` to GoTo.
2. Native GoTo selects Alpha, whose existing `ctrl+f12` custom command runs
   exactly once and writes the expected marker.
3. Injected `Ctrl+]` still opens native GoTo afterward. Both original config
   files remain byte-identical.

The recorded run passed in **22.73 seconds**. See
[public-chord.xml](evidence/public-chord.xml) and
[the captured native screen](evidence/public-reserved-control-keeps-remote-command.txt).
This disproves the earlier conclusion that this particular collision requires
abandoning the native-controller approach. It proves the corrected control-key
mechanism, not the complete production launcher or every configuration.

Reproduce the corrected proof:

```sh
NAVIGATOR_GATE_SKIP_BUILD=1 NAVIGATOR_GATE_IMAGE=herdrpp-machine-experiments \
  sh experiments/machines/run-gate.sh public-chord
```

## Earlier hidden-chord gate that failed

`test_private_navigator_binding_preserves_remote_custom_command_on_the_same_chord`
uses real Herdr **0.9.5**, real SSH, independent servers, and two viewing clients:

1. Alpha defines an existing `ctrl+f12` custom command that writes `handled` to a
   marker file. The first ordinary viewing client invokes that command exactly
   once; the test checks the file's exact contents.
2. The second client uses a private local config adding `ctrl+f12` to GoTo.
   Alpha's remote config is unchanged. The same key opens GoTo and the remote
   custom command does not run during a ten-second observation window.
3. The required preservation assertion fails. The captured native GoTo screen
   proves where the key went.

The local collision checker cannot see this remote binding. Herdr's public
`SessionSnapshot` omits command/keybinding manifests; its internal client snapshot
combines remote custom commands with local built-ins and the private built-in
wins. Native keybinding help displays effective bindings after resolution. An
additional hidden internal key reservation would change the accepted requirement
to preserve existing custom bindings. The corrected public-chord proof above
avoids that additional reservation.

This is a failure of the earlier hidden-`Ctrl+F12` implementation choice,
**not proof that same-client approaches are impossible**. Production integration
was paused at that gate. The corrected public-chord experiment now supplies a
passing alternative, while the original failure remains preserved for review.

Recorded evidence is in [remote-collision.xml](evidence/remote-collision.xml) and
[the native screen](evidence/remote-custom-command-intercepted.txt). The full ANSI
capture remains under `target/navigator-gate/navigator-gate/`.

## Reproduce

With Docker running, this command builds an isolated pinned Herdr image and runs
only the required gate. **Exit 1 is the recorded compatibility failure.**

```sh
sh experiments/machines/run-gate.sh collision
```

To reuse the already available original experiment image:

```sh
NAVIGATOR_GATE_SKIP_BUILD=1 NAVIGATOR_GATE_IMAGE=herdrpp-machine-experiments \
  sh experiments/machines/run-gate.sh collision
```

Reports are written under `target/navigator-gate`; `NAVIGATOR_GATE_OUTPUT_DIR`
changes that artifact directory. The research never connects to personal SSH
machines or changes normal Herdr configuration.

The positive mechanism checks can be run separately:

```sh
NAVIGATOR_GATE_SKIP_BUILD=1 NAVIGATOR_GATE_IMAGE=herdrpp-machine-experiments \
  sh experiments/machines/run-gate.sh passing
```

The final run recorded **4 passed** in `evidence/passing.xml`. They cover private client binding injection with a hidden sidebar/mouse disabled,
completed-frame parsing, busy-pane SIGWINCH redraw, and native Alpha current-pane
ancestry. The hidden-chord failure and corrected public-chord success remain separate
experiments with different control bindings.

## Relationship to the production implementation

The production controller uses a private configuration, per-profile ASCII
aliases and complete native VT frames. It acknowledges the committed current
pane and restores original labels before releasing input. Its explicit
reattachment fallback handles unsupported geometry or unverified activation.
The production gates require unchanged native viewer PIDs and reject fallback
messages, so reattachment cannot conceal a failed native path.

The research helpers reuse the production SSH lab and PTY transport. Only the
historical frame-buffered screen observer remains separate: it publishes raw
synchronized-frame boundaries, while production observes parsed VT tokens.
Reproductions now use the shared fixture's distinct loopback SSH endpoints;
both still run in one container and on one physical host.

The earlier scaffold's `just ready` result covered formatting, Clippy, Rust,
90 selected Python tests and 28 PTY scenarios. Those historical results and the
recorded XML files describe the mechanism experiments at that time. Run
`just ready-all` for the current production release gate.
