Round 19, 2026-10-29 (the set's clock, EDT): second device pass of the Rust shell on LumaTV's compositor. Results are in docs/research/compositor-gate9-result.md "Round 19"; the PASS/FAIL/MINOR/INFO lines are in log.txt.

install/             the wrapper's log of the three installs (3ff3b31, 5e4fafd, 3f8efad) and the black-panel recovery after the second
start*.json          the set's state at the start, after the installs and before the gate
rust-gate-1/, -2/    `gate.sh rust` on 3ff3b31 and on 3f8efad (GATE rust: PASS)
gate-full-partial/   the full gate with the Rust kinds on, stopped at 17:24: apps rows and plxnative-crash.log (4 SIGSEGV reports)
bootblocks/          the reboot check of the update block and the iot-client stub (pre/, post1/, README.txt)
img/                 stills of the looks and of the fixes
wake/                lginput2's log and the evdev recordings: the 4 s remote wake (capture 1) and the key 1199 at the "on table" sleep (capture 2)
