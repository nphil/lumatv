# Test health, 2026-10-26

Full runs of every suite on a clean worktree of `df3351e` (`git worktree add /tmp/th-df3351e HEAD`; `/tmp/LumaShow` is the symlink the
helper's path dependencies and `tests/test_ha_plugin_package.py` expect). Python modules run one process each (own Xvfb), five at a time,
`HOST_TIMEOUT=900`; cargo workspaces with `CARGO_TARGET_DIR` under `/tmp/th` and a `cc` shim (`cargo-zigbuild zig cc`; this container has no
C compiler) first on `PATH`.

## Resume here

LooseEnds (restart of TestHealth, 06:30). Clean worktree `/tmp/th-df3351e` (detached; `git checkout --detach <HEAD>` to refresh; host builds
`ambient/capture/build.sh`, `jpeg/build.sh`, `vplane/build.sh`, `gfn/feeder/build.sh` are built in it, so those modules run instead of skip).
Runner `/tmp/th/run_all.py <root> <jobs> <budget s> [module.py...]` (one process and one Xvfb per module, JSON row per test in
`$TH_LOGS/<module>.jsonl`); cargo through `/tmp/th/cargo_all.sh` (a `cc` shim, `cargo-zigbuild zig cc`, first on `PATH`; this container has no C
compiler). Everything below is committed. Not done: a final full Python pass after the last fixes (the modules changed were re-run singly).


## Results

Cargo, all green at `2886b62`: helper 730 passed (4 ignored), compositor 120, rustui 67, gfn/engine 10, ambient 107. compile_all: 120 QML files,
0 failed. bun: green.

Python at `2886b62`, 2 modules at a time, `HOST_TIMEOUT=900`: 1 088 tests in 63 modules; every module ok except the rows under Open. The 46
tests that skipped for want of a host build ran once the four builds above existed (capture_service 9, jpeg_service 31, capture_install 7,
vplane_install 12, gfn_feeder 28 + 1 skip that needs qemu, ambient_slot 23).

### Fixed

- helper `receiver_telnet` and `ai` tests (earlier run): the read form of a receiver command ends in `?`; the lock-out test waited on a bound listener
  by polling the port, now on the published document.
- compositor `vplane_host` (`the_veil...`): the veil's opaque margins since vplane 5ee7f40.
- compositor `protocol`: a read-back after a scene change ticks the server once (`harness::drawn`) instead of sleeping 30 ms, which lost the race to
  the loop thread under load.
- `test_shell_overlay` door test: read by a step; the door's own 1 s tick falls among the residents' builds at a machine-dependent time.
- QML host: the compositor's `/tmp/lumatv-*` files (door, beat, wallsample.xpm, saver mailbox) are a directory of the run's own (`QMLHOST_TMP`, else
  under the run's staging dir). Two hosts at once shared `lumatv-wallsample.xpm` (a read of the other's half-written file warned, rc 1) and the doors.
- QML host: a host without `--fake-helper` waits out the ones serving the stand-in on port 18731; `test_homefeeds` "saved file" and "later
  reading" read another run's weather.
- `test_homefeeds` "later reading": looked for 3.5 s after the publish (it failed at 1.2 s when two modules ran at once).
- `test_settings` AI page: the D-pad path to Cody and to the lock-out follows the two-column layout (it still described the one-column one).
- `test_quick_panels`: LG's Quick Settings launch is matched by the service's last name (`com.webos.service.systemUIManager`).
- `test_saver_own` (SaverOwn3, e660403): the vote test counted every `cancel`, and since 2886b62 the Inputs readings cancel their previous subscription too; it counts the vote's own only.
- `test_vplane_saver` (SaverOwn3): a mailbox of its own per test; the 11 failures seen after the host change were the old shared-path version.

### Open

| Test | Failure | Owner |
|---|---|---|
| `test_settings` `test_choosing_writes_one_entry_each_and_leaving_the_default_removes_it` | 1 of 3 runs: `[LumaHome] theme.json missing/invalid` while a choice is being written (a read of `theme.json` during the XHR write) | Settings owner (SettingsRest4) |
| `test_vplane_player` `test_the_caption_fades_with_its_print_and_not_with_the_dim` | 1 of 3 runs under load: the next print's caption comes back before the veil's peak (commit cadence 600 ms / fade 400 ms is wall-clock) | video-plane owner |
| `test_quick_panels` slide tests | passed once `b009de3`/`1530582` landed; failed at `7bd4e72` | closed |

Timing-sensitive display modules (`test_settings`, `test_vplane_saver`, `test_quick_panels`) take 700 to 2 600 s at load above 10; at load 5 they
finish in 100 to 800 s. Run them one at a time.
