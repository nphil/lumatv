Round 18c, 2026-10-29 (the set's clock EDT 13:42 to 14:53), forge/main 5888f19 (includes 34118b6, the shell gate's reader fix), the set on f2e4b5d as installed by round 18b (compositor md5 fe01e3b8..., lumatv-ui b1e7f83e..., read back on the set). No install this round. Callbacks `commit` (the default), Rust kinds off, receiver muted through Home Assistant's MCP and put back.
start-state.json, end-state.json: the set before the first gate and after the last standby.
shell/: `--only shell`, PASS (quick-app ok).
gate-full/: `--skip volume --apps-repeat 15`, every blocking check PASS, frames FAIL on the minor class (frames.json, per-check files, stdout.txt).
x1/x1: x1 of the full gate; x1/x2..x6: `--only x1` five more times (three PASS, three INCONCLUSIVE in all six, none with a compositor-attributable gap).
golive/log.txt: the persistent enable, the live read, the compositor restart.
