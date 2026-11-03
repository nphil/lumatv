# Review of work since 2026-10-26

Scope: LumaTV OS, LumaGame (GeForce NOW feeder, Game Mode) and Marquee, everything committed since 2026-10-26. Focus: performance, stability, UI/UX. Findings are fixed only after the owner picks them.

Baseline: `8f6c04ab` (this repo) and Marquee `f08456d48`. The installed TV build is this tree: compositor `bc5a069b`, UI `0a8467e6`, supervisor `f9625a8a`, helper `94ebaded`; the other installed files match byte for byte.

## Method

Machines check first; careful reading is reserved for the riskiest code.

1. No-TV pass: `tools/check-all.sh` (every suite and clippy); Marquee's suite as a non-root user; a scan for unfinished work; files ranked by churn, diff size and hot path (frame loop, input, video, power), with the top files reviewed per area; cited proof paths checked.
2. Stop point: the owner reads the short list before any TV time.
3. One unattended TV run with the existing gates: frame gate at 60/120 Hz, video check, apps x15 close test, standby/wake cycles, reboot, soak, screenshot walk against the mocks.
4. Ranked report: broken / slow / rough, one line of evidence each.

Docs and mocks (about 4,600 changed files) are not reviewed beyond the claims check. Not covered yet: the supervisor (`compositor/sm/lumatv-sm.c`, helper `crash.rs`).

## Status

| Step | State |
|---|---|
| Repo hygiene | Done |
| No-TV pass | Done |
| Owner stop point | Waiting |
| TV run | Needs a 2-3 hour TV slot |
| Report | Pending |

## Findings

P1 = fix soon, P2 = should fix, P3 = cleanup.

### Process

- Fixed. The installed build lived only in an uncommitted scratch worktree (`/tmp/lumatv-r28`, 865 files); GitHub was one commit behind and the forge 15. Committed as `8f6c04ab` and pushed to both; build output and `.impeccable/` review screenshots are ignored; six merged scratch worktrees and eight merged branches removed; superseded pop-up edits kept on local branch `archive/subwindow2b-popup-wip`. `CLAUDE.md` now requires installing only from pushed `main`.
- Code changed without its tests: 15 LumaTV tests and 25 Marquee tests assert behavior that was deliberately changed (see Tests). Three compositor transition tests now run without the UI engine they depend on and no longer exercise the real path.
- Logs moved to the collector at `/tmp/lumatv-log.sock`, so tests can no longer read the compositor's or helper's diagnostics; a vacuous check resulted: `helper/tests/voice.rs:536` ("the log holds no secret") reads the removed `log-helper` file and passes on an empty string.

### LumaGame and helper

1. P1 Feeder: `pcm_sticky` is set when a PCM retry is attempted, not when it succeeds; smp refuses PCM, so after one Opus refusal every later AUTO-mode session fails without audio until the feeder restarts. `gfn/feeder/src/feeder.c:601, 620-627`, `gf_smp.c:176-178`.
2. P2 Helper: feeder socket writes have no timeout; a feeder that stops reading holds the only video lease until the helper restarts. `helper/app/src/live/gf.rs:331-345`, `live/mod.rs:746, 818`.
3. P2 Helper: an Answers-thread setup failure leaves a live session that later starts attach to, stuck at "starting". `live/mod.rs:1596-1603`.
4. P2 Feeder: a refused smp unit is retried for the full 8 ms and reported as a FULL drop instead of a refusal. `feeder.c:288-296`.

### Marquee

5. P1 Opus direct play whose source fails to open skips the recovery ladder (HLS fallback, trial rollback) and fails permanently. `rust-modules/media/src/player/engine.rs:2242-2257`, `pump.rs:1699`.
6. P2 Opus Load waits for the demuxer open inside the 20 s Load budget: slower starts, slow opens reported as hangs. `engine.rs:2242-2248`.
7. P2 Word-by-word subtitle alignment rebuilds and reparses the whole script about every 2 s of dialogue even when words are unchanged. `sidecar.rs:215-226, 538-546`.
8. P2 A new subtitle hold starts at epoch 0 while alignment starts at 1, so the first restyle swaps the renderer's source (likely a reopen/flash on every SRT style change). `sidecar.rs:512, 694`, `alignment.rs:217`.
9. P2 Text no longer fits: the jail-help note (en, be, ru) after the URL change in `7531f3c03`; the Russian privacy title and the French consent screen after `25c0c7be4`.
10. P3 `PRIVACY.md` was cut from 349 to 9 lines, removing the generated data-schema table; nine telemetry tests that enforced "every collected value is disclosed" now fail. Product decision: restore the table or move the guarantee to the in-app privacy text.

### Compositor

11. P2 A failed linear page setup leaks a dumb buffer (8.3 MB CMA; free CMA can be 3.5-12 MB), reachable at runtime via `ctl afbc off`/`pagealpha`. `kms/mod.rs:348-352`.
12. P2 A refused warm target leaks a texture and FBO on every direct-scanout attempt. `render/gl.rs:820-829, 1832-1844, 1211`.
13. P2 One-shot topics (`homekey`, `menu.open`, `confirm.ask`) are replayed to a reconnecting UI, which can open the switcher or a menu unprompted. `shell.rs:151-158`.
14. P2 Screen recording reads frames back synchronously on the loop thread (~110 MB/s at 30 fps). Off by default. `render/gl.rs:1425-1452`.
15. P3 `vwin_sync` rebuilds the video-window world and a JSON topic every tick with nothing claimed. `vwin.rs:995-1028`.
16. P3 UI restart limit/backoff is dead code since the stock fallback on first UI crash; a pointer press can lose its release when the cursor hides; a partial key-socket write can corrupt the next line. `shell_rust.rs:580-589, 240-256`, `server/seat.rs:517-530`.

### Rust UI

17. P2 With 61+ dock entries the 61st tile reuses the dock card's id and the dock disappears. `home/surface.rs:597-601`.
18. P3 Per-input and idle work: the full state report (layout clones, JSON) is rebuilt on every input including pointer motion; theme/layout JSON is re-read and parsed every 2 s at idle; the quick panel and Home re-lay out and re-serialise on every redraw. `shell.rs:1138-1144`, `home/model.rs:756-762`, `panels/quick_surface.rs:980-985`, `home/surface.rs:416-423`.

### Tests

Logs: `/data/home/tmp/review-20261103/suites/`.

| Suite | Result | Verdict |
|---|---|---|
| Compositor (`--features ui`) | 736 passed, 10 failed | 9 stale, 1 timing flake (OLED-care orbit, `scanout.rs`) |
| Rust UI | 1512 passed, 1 failed | stale (gear now opens Quick Settings) |
| Helper | 1185 passed, 5 failed | stale (`base.logging` setting, compact toast size, socket logger) |
| Marquee (non-root) | 29 failed | 4 real (items 8-9), 25 stale (rebrand, new settings rows, two-stop Skip pill, screen-shape pin, `PRIVACY.md`) |
| Marquee native subtitle tests (normally ignored) | 61 passed | libass host artifact present |

Stale compositor tests: `ime_own` (keyboard loss now exits to stock), `livetv` and two transition tests (no UI engine in the harness), `refresh` legacy title (type deleted), `scanout` keyboard (vendor keyboards are never displayed), `screencap_venc` (stderr goes to the collector), `shell` gear (LG Settings fallback retired) and whole-payload Home topic, `shell_rust_attach` (the harness provides no native UI assets; the compositor exits with "native UI assets missing").

Clippy: compositor clean; Rust UI warnings only (`lumatv-render` test modules are stripped on purpose by `sync-lumashow.sh`, so `tools/check-all.sh` excludes its test targets); helper fails on an unhandled `read` amount in a test (`receiver.rs:1925`); Marquee's own gate (`make lint`) passes, default clippy reports style lints only. Python tests need `pytest` installed.

Unfinished-work scan: no `todo!`, `unimplemented!`, TODO/FIXME or `dbg!` in shipping code. Cited proof paths exist; about 60 live under `/tmp` and do not survive a container restart.
