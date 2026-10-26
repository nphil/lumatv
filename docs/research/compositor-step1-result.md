# Step 1 under LumaTV's compositor: the remote's key routing and the keyboard

Build-order item 1 of "Surfaces under LumaTV's compositor" (`compositor-c2-result.md`): the key filters LG's compositor runs
in its own QML (gear, Input, the app keys, Quick Access digits, Mute held, Exit; Volume and Mute are done) and IME routing
(`text_model` to `input_method`, maliit's own keyboard window), without drawing anything of LumaTV's.

## Resume here

State (2026-10-26): safety timer committed first (aa3a2eb); key routing (21b6013) and the keyboard path (83faf05) built and
host-tested (`keys.rs` the decisions, `input.rs` the routing, `system_ui.rs` the asks, `server/ime.rs` the text model to input
method path, `scene.rs` the keyboard window's place; `tests/keys.rs`, `tests/ime.rs`); the gate's `keys` and `ime` checks and
`tv_gate.py --only` are in, with their Python tests (`GateKeysAndIme`, `GateDriver`). Still to do: the sections missing here
(what was built, assumed codes, the TV command, pass criteria), then the final test run and the reply. The TV is not touched by
this work: Main runs the TV test.

## Safety timer

`cutover.sh enable --session [--max-minutes N]` (default 15, whole minutes 1 to 1440; a trial gets its seconds plus 5
minutes) starts a detached job on the set that runs `cutover.sh disable` after N minutes unless the session has ended by then;
`disable` and `revert` cancel it, a new session replaces it, a persistent enable has none. A session whose job did not write
its record within 3 s is not mounted (exit 8). The gate's first session is 12 minutes, each later one 6 (the last gate: 4
minutes, then 33-35 s each). `cutover.sh status` shows the minutes left; `safety-timer.log` in the state directory says what
the job did. Details: `cutover-result.md`, "Safety timer". Host tests: `tests/test_cutover.py`, `SafetyTimer` (the job runs
for real at one second to the minute).
