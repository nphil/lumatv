# Step 1 under LumaTV's compositor: the remote's key routing and the keyboard

Build-order item 1 of "Surfaces under LumaTV's compositor" (`compositor-c2-result.md`): the key filters LG's compositor runs
in its own QML (gear, Input, the app keys, Quick Access digits, Mute held, Exit; Volume and Mute are done) and IME routing
(`text_model` to `input_method`, maliit's own keyboard window), without drawing anything of LumaTV's.

## Resume here

State (2026-10-26): safety timer committed first (aa3a2eb). Key routing built and host-tested (`keys.rs` the decisions, `input.rs`
the routing, `system_ui.rs` the asks, `tests/keys.rs`; clippy clean with and without `ui`). IME routing is written
(`server/ime.rs`, the keyboard window's placement in `scene.rs`, `ctl ime`) and compiles, but its host tests (`tests/ime.rs`) are
the next piece. After that: the gate's `keys` and `ime` checks (`tv-gate.sh`, `tv_gate.py`, Python tests), then the sections
still missing here (what was built, assumed codes, the TV command, pass criteria). The TV is not touched by this work: Main runs
the TV test.

## Safety timer

`cutover.sh enable --session [--max-minutes N]` (default 15, whole minutes 1 to 1440; a trial gets its seconds plus 5
minutes) starts a detached job on the set that runs `cutover.sh disable` after N minutes unless the session has ended by then;
`disable` and `revert` cancel it, a new session replaces it, a persistent enable has none. A session whose job did not write
its record within 3 s is not mounted (exit 8). The gate's first session is 12 minutes, each later one 6 (the last gate: 4
minutes, then 33-35 s each). `cutover.sh status` shows the minutes left; `safety-timer.log` in the state directory says what
the job did. Details: `cutover-result.md`, "Safety timer". Host tests: `tests/test_cutover.py`, `SafetyTimer` (the job runs
for real at one second to the minute).
