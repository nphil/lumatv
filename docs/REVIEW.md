# Review of work since 2026-10-26

Scope: LumaTV OS, LumaGame (GeForce NOW feeder, Game Mode) and Marquee, everything committed since 2026-10-26. Focus: performance, stability, UI/UX. Findings are fixed only after the owner picks them.

Baseline: `8f6c04ab` (this repo) and Marquee `f08456d48`. The installed TV build is this tree: compositor `bc5a069b`, UI `0a8467e6`, supervisor `f9625a8a`, helper `94ebaded`; the other installed files match byte for byte.

## Method

Machines check first; careful reading is reserved for the riskiest code.

1. No-TV pass: build and run every test suite and clippy; scan for unfinished work (WIP, placeholders, ignored tests, debug logging, unbounded logs); rank files by churn, diff size and hot path (frame loop, input, video, power); review the top ~15 files; check that "passed"/"accepted" claims in PLAN, HANDOFF and research results have a receipt.
2. Stop point: the owner reads the short list before any TV time.
3. One unattended TV run with the existing gates: frame gate at 60/120 Hz, video check, apps x15 close test, standby/wake cycles, reboot, soak, screenshot walk against the mocks.
4. Ranked report: broken / slow / rough, one line of evidence each.

Docs and mocks (about 4,600 changed files) are not reviewed beyond the claims check.

## Status

| Step | State |
|---|---|
| Repo hygiene | Done |
| No-TV pass | In progress |
| TV run | Needs a 2-3 hour TV slot |
| Report | Pending |

## Findings

1. Fixed. The installed build lived only in an uncommitted scratch worktree (`/tmp/lumatv-r28`, 865 files) while GitHub was one commit behind it and the forge 15 behind. Committed as `8f6c04ab` and pushed to both; build output and `.impeccable/` review screenshots are now ignored. Six merged scratch worktrees and eight merged branches were removed; superseded pop-up edits are kept on the local branch `archive/subwindow2b-popup-wip`.
