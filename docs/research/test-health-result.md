# Test health, 2026-10-26

Full runs of every suite on a clean worktree of `df3351e` (`git worktree add /tmp/th-df3351e HEAD`; `/tmp/LumaShow` is the symlink the
helper's path dependencies and `tests/test_ha_plugin_package.py` expect). Python modules run one process each (own Xvfb), five at a time,
`HOST_TIMEOUT=900`; cargo workspaces with `CARGO_TARGET_DIR` under `/tmp/th` and a `cc` shim (`cargo-zigbuild zig cc`; this container has no
C compiler) first on `PATH`.

## Resume here

LooseEnds (restart of TestHealth, 05:30): the full Python suite is running at current HEAD on `/tmp/th-df3351e` (detached at HEAD; runner
`/tmp/th/run_all.py <root> 2 1800`, logs in `/tmp/th/logs7`, summary `/tmp/th/all7-summary.txt`). Cargo (helper, compositor, rustui,
gfn/engine, ambient) is green. The host builds behind the 46 skipped tests are built in the worktree (`ambient/capture/build.sh`,
`jpeg/build.sh`, `vplane/build.sh`, `gfn/feeder/build.sh`) and those modules pass. Open: whatever the summary lists as not ok.

## Results

(filled in as the suites finish)
