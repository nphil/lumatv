# Agent guide

Entry point for an agent working on LumaTV (this repository, formerly LumaGlass): how the parts fit, how a build reaches the TV and comes back off it, and the operational facts that cost time to learn. `HANDOFF.md` is the dated engineering record (installs, measurements, hashes); `plugins/` has one page per plugin; `../CHANGELOG.md` is the release history. Facts here are current state; anything unverified is marked [INFERENCE].

## Rules of the repo

- Commit and push straight to `main`, to both remotes: `origin` (GitHub) and `forge` (git.nateshome.net). No branches or PRs.
- The TV runs only a commit that is on `main` and pushed to both remotes. Installed binary hashes are recorded in `HANDOFF.md`.
- Before pushing: the touched crate's tests and `cargo clippy -D warnings` pass. No WIP, ignored tests or debug logging on `main`.
- Frame loop, input, video and power changes need a measured result (frame gate, video check) before claiming faster or fixed. "Accepted" means checked on the TV; untested cases are listed as untested.
- Surfaces and plugins take colours, glass, radii and type from the active theme's tokens only. Integrations are plugins: off until enabled in Settings.

## Component map

| Part | Where | Owns |
|---|---|---|
| compositor | `compositor/crates/lumatv-compositor` | The live Wayland compositor (64-bit, on LG's socket `/tmp/xdg/wayland-0`, started through a private glibc loader): scene, frames and refresh, input and key routing, video windows and scan-out, luna calls, panel dimming, OLED care. Runs the drawing engine that the UI's layers go to. |
| sm | `compositor/sm/lumatv-sm.c` | The 32-bit supervisor, bind-mounted over `/usr/bin/surface-manager`. Decides whether LumaTV runs, spawns and heartbeats the compositor (500 ms), bridges luna for it, and falls back to LG's compositor in place on a fault, an unsettled start or two consecutive failures. Home held 10 s is the owner's escape. |
| ui | `rustui/ui` (`lumatv-ui`), engine in `rustui/render`, wire types in `rustui/compositor-protocol` | The shell's logic as a 64-bit client with no GL: Home, Settings, panels, cameras, guide. It sends layers and groups to the compositor over a socket pair, gets keys over a key socket, and reads state from the helper. |
| helper | `helper/app` (`lumatv-helper`) | The trusted privileged process: a static aarch64 musl daemon running as root. Owns settings, weather and feeds, cameras, receiver, HA, BLE, the tuner host, the log collector and the plugin host. Loopback API on `127.0.0.1:18731`. Privileged work runs here, and only while its plugin is enabled. |
| dnd | `helper/app/src/dnd.rs`, `rustui/ui/src/dnd.rs`, doc `docs/dnd.md` | Do Not Disturb and snoozes: the helper's gate for camera pop-ups and plugin notes, the `dnd` topic and `/v1/dnd` routes, state in `dnd.json`; the UI's tile, status-bar mark, pop-up snooze sheet, held toasts and the drawer's missed group. |
| plugins | `helper/plugins`, `helper/denon`, `helper/app/src/*`, docs in `docs/plugins/` | First-party: `denon-receiver`, `scrypted-cameras`, `home-assistant`, `live-tv-tuner`, `guide-data`, `ble-proxy`, `tv-sensors`. Off until switched on in Settings, which asks first in the plugin's own words; the owner's approval of those words is kept in `plugins/enabled.json` (`firstParty`). Each plugin thread runs under `plugins::guard`: a panic stops that plugin alone and it retries after 30 s, doubling to 30 min. |
| lumatuner | `helper/tuner` | The antenna-tuner engine (HDHomeRun endpoints, M3U, XMLTV, slates). No host dependencies, so it can ship standalone. The helper's glue is `helper/app/src/tuner*`. |
| tools/lumatv | `tools/lumatv` | The on-device command (POSIX sh, busybox) shipped in the IPK: apply, revert, boot hook, `compositor`, `helper start|stop`, `layout`, `wake`, `standby`. Must match the helper it starts. |
| gfn | `gfn/` | GeForce NOW stream engine and feeder (separate cargo workspace). |

How they talk:

- **ctl socket:** the compositor listens on `lumatv-control` in its runtime dir (`/tmp/xdg`), one text command per line, one JSON line back; client is `lumatv-compositor ctl --runtime-dir /tmp/xdg <cmd>` (`status`, `windows`, `journal`, `shell`, `switcher`, `key`, `remote`, `ring`, `refresh`, `vwin`, `fallback`; the UI's own commands go through `shell`, e.g. `shell show toast LumaTV:_text_with_underscores`). Local only. On the TV run it through the private loader in `/var/lib/lumatv/compositor` (`lib/ld-linux-aarch64.so.1 --library-path lib bin/lumatv-compositor ctl ...`).
- **Helper HTTP, 18731:** loopback only (non-loopback peers and unknown `Host`/`Origin` are refused); every write needs an `X-Luma-Client` header. A client reads `GET /v1/state` (carries `seq` and `boot`), then long-polls `GET /v1/events?after=<seq>&boot=<boot>` (at most 30 s); a client behind the 256-event ring, or from before a helper restart, is told to reset and reads the state again. `GET /v1/state/<topic>` reads one topic. A second listener on the TV's private address (`Mode::Lan`) serves MCP/AI clients with GET and POST only. The UI, the compositor and `tools/lumatv` all use this API.
- **Topics:** the hub in `helper/app/src/hub.rs` holds named JSON topics (`screen`, `receiver`, `weather`, `plugins`, ...); plugin topics are `plugin.<id>.<name>`. Setting a topic to the value it holds is silent. The UI reports its own state back with `POST /v1/ui`.
- **Logs:** producers send Unix datagrams to `/tmp/lumatv-log.sock`; the helper keeps at most 1 MiB in RAM under `/tmp/lumatv-logs/`. Tests cannot read the compositor's or helper's diagnostics from files.

## Build

Host is the Cody workstation; nothing builds on the TV.

- Environment for every cargo run: `TMPDIR=/data/home/tmp`, `CARGO_INCREMENTAL=0`, tests with `-- --test-threads 4` (or `RUST_TEST_THREADS=4`), `CARGO_BUILD_JOBS=8`; `nice -n 10` for release builds.
- Helper: `helper/build.sh test` (clippy `-D warnings` and host tests), `helper/build.sh` (static aarch64 musl into `helper/dist/`). It needs the LumaShow checkout at `/data/home/LumaShow/wt/lumatv` (path dependencies) and zig. The host CC and linker for the helper's build is `gfn/engine/tools/cc-host` (`cargo-zigbuild zig cc`, which rewrites the flags cc-rs passes). `rustui/tools/cc-host` is plain `zig cc` and breaks `ring`'s build; do not use it for the helper.
- Compositor, ui, sm and assets: `compositor/tools/build-tv.sh` produces the bundle `compositor/deploy` (`bin/`, `lib/`, `HASHES`, `cutover.sh`, `gate.sh`); it is not committed. Targets are aarch64 glibc 2.36 through `cargo zigbuild`. Per-crate: `cargo test -p lumatv-compositor`, `cargo test -p lumatv-ui`, each with `cargo clippy --all-targets -- -D warnings`.
- Disk is shared and capped. Use one scratch worktree per job (`/data/home/wt-<name>`) with its own single `target/`, build there, and delete the worktree and its target when done. Check `df -h /tmp /data/agent /data` first. Nothing installed may exist only in a scratch tree: it must be on `main` and on both remotes.

## TV access

- The TV is reached as root over SSH with paramiko: ed25519 host key pinned (`RejectPolicy`, no key lookup, no agent). Address and credentials live with the owner and are not in the repo.
- `luna-send` hangs without a PTY: open the channel with `get_pty()`. Use no PTY for binary-safe output and SFTP. `journalctl` hangs: read `/var/log/messages` (bracketed monotonic stamps, rotates within minutes). The userland is busybox; large programs go over bounded stdin (command under 1 KiB, chunks 16 KiB, Dropbear's receive payload is 32 KiB).
- Hold the lease before any TV-facing action: `/data/home/Marquee/tools/tv-lock.sh` (shared with Marquee). `acquire --why "<what>" [--ttl MIN]`, then `status`, `renew`, `release`; `with --why ... -- CMD` takes and releases around a command; `require` takes an implicit 10-minute lease if nobody holds it. The default TTL is 45 minutes (`PLX_TV_LOCK_TTL`); a lane is `PLX_TV_LOCK_LANE` and each session uses its own, never another's lease. The lock is a directory on the TV (`/tmp/plx-tv.lock`), cleared by a reboot, and times come from the host's clock. A lease cannot see a person watching: `acquire` prints IDLE or `IN USE` from what is in front, and `IN USE` means the owner says go first.
- Read-only while the owner is watching: with the panel on and anything but idle Home in front, or `IN USE`, do reads only (`ctl status`, screenshots, log and file reads). No keys, launches, setting, picture, volume, power or receiver changes. An agreed install is the exception; announce each step with a toast (`ctl shell show toast ...`) and leave Home in front and the receiver as found.
- The receiver is a Denon on HDMI 2 (ARC). Test runs mute it while on and leave it as found (off, unmuted). Its HTTP command page ignored `MUON` once; telnet on port 23 worked.

## Install and rollback

Install only a commit on `main` at both remotes, with its tests, clippy and release build green. A prebuilt bundle (compositor, ui, sm, helper, `tools/lumatv`, with `COMMIT`, `SHA256SUMS` and the build timeline) can be installed without a worktree. Session scripts (`tvs.py`, `inst.py`) live under `/data/home/tmp/lumatv-*-<date>/`, outside git.

1. Lease for at least 900 s.
2. Read the installed sha256 of the compositor, ui, sm, helper (`payload/helper/lumatv-helper` and `/var/lib/lumatv/bin/lumatv-helper`) and `tools/lumatv`; copy the binaries about to be replaced into a `backup/` and check their hashes. They are the rollback.
3. Stage the bundle to `/tmp/lumatv` (`compositor/tools/tv-first-light.sh deploy` builds and copies it; a prebuilt one is only copied) and check the staged hashes against the build.
4. Compositor cutover with `/tmp/lumatv/cutover.sh`: `disable`, `install` (copies to `/var/lib/lumatv/compositor`, records the firmware fingerprint), `probe`, `enable` (persistent; refused with exit 5 on a firmware not in `compositor/firmware/verified.list`). Check `status`: live, mounted, enabled, `failures` 0, not escaped.
5. Helper: put the new `lumatv-helper` at `payload/helper/lumatv-helper` and run `tools/lumatv helper start`, which also migrates `helper.json` atomically. Check the pid changed and the port answers.
6. Compare the installed hashes with the build. Add a `HANDOFF.md` section: date, main sha, installed sha256 (full or 8-hex prefix), what each replaced, the receipts path, what was and was not tested on the TV, and how the set was left. Commit and push it.

Rollback: `cutover.sh disable` returns LG's compositor and Home (`revert` also removes the overlays); the supervisor falls back by itself on a fault; Home held 10 s is the owner's escape. To go back one build, restore the `backup/` binaries by the same steps and `helper start`. A firmware update clears the mod and needs a new gate pass; the TV is webOS 10.2.1-5202.

## Frame gate

`compositor/tools/tv_gate.py` (on the set, `compositor/deploy/gate.sh`) runs the compositor as a session and measures, per surface (Home d-pad, Home widgets, Apps grid, quick settings, Settings scroll, keyboard, input picker, switcher), cycles of open and move: dropped frames above 10 ms at 120 Hz, first frame, burst end (limit 180 ms) and frames over 20 ms, from a kernel trace. `--only frames --refresh 120` is the 120 Hz check; `--frames-quick` the short set. A run whose trace ring overran fails `trace_complete`. It leaves the refresh policy to `gate.sh refresh auto` at the end.

Pitfalls:

- With Recent apps on Back and an app running, the wake's Back opens the app switcher, which takes every key and fails Home surfaces. The gate closes it with a second Back and refuses to run while the switcher is up, the ctl does not answer, or the refresh is not held at the asked rate (30f333f3).
- An unprompted real camera pop-up or full view on screen during a run belongs to a real camera, not the test: exclude those cycles and rerun. The gate turns pop-ups off before its surfaces and back on after.
- After a Dolby Vision page `DE2VSYNC` can sit at 240 a second while `osd_irq` stays at 120, and the gate refuses to start; a few seconds of Live TV clears it.
- A full host disk mid-run logs results late; check `df` first.

Status (2026-11-09, main 752ab5f9): not passing yet, with one to three drops left where there were tens. Nine cycles per surface at 120 Hz, dropped frames: with no load, two runs, Home d-pad 2 and 2, Home widgets 2 and 0, Apps grid 1 and 1 (b6f72b77: 87, 7, 3); under plugin load (the tuner streaming about 11 Mbit/s, camera test pop-ups, a guide refresh) Home d-pad 3, Home widgets 1, Apps grid first frame up to 524 ms (b6f72b77: 63, 10, 1444 ms). The gain came from no longer drawing a frame over one waiting behind the flip in flight (752ab5f9); `6e47df5a` (UI at nice -10, helper threads and plugin processes batch) is hardening only. Do not claim a smoothness fix without a gate run under load.

Open, in this order (timeline and receipts in `HANDOFF.md`, 2026-11-09):

- The first frame after a still screen. A motion's first moving frame on Home (8.6 ms to GPU done against an 8.3 ms refresh) starts only once the flip of the frame before it lands, so it misses the next refresh. Until the pacer counts frames as flowing, `Kms::next_frame_at` returns `None`, `draws_on_damage` is false while a flip is pending, and the idle grid tick is budgeted from the cheap frames before the motion. First record each UI commit's arrival in the ring. If the moving frame's commit was waiting, draw a UI commit with damage at once and queue it behind the flip in flight, only while one is undrawn; if it was not, the UI's first commit after a key has to carry the first moved frame. Accepted at 0 drops on Home d-pad and Home widgets with no load in two runs, nothing worse under load.
- The Apps grid's first frame under load: nothing flips from the key until a camera pop-up arrives or leaves (104 to 518 ms late).

## UI scene limits

The engine accepts at most 256 layers and 96 groups (`rustui/render/src/session.rs`, `compositor-protocol/src/group.rs` `MAX_GROUPS`) per client, plus a fixed number of motion tracks. A frame that would exceed them is refused (`more than N layers|groups`, `scene is full`) and the page loses what did not fit, typically the focus plates and the profile chip, until the UI's self-heal redraws. A resting Home uses at most about 40 groups. Settings opened over the quick panel therefore closes the panel (96797d97); a new overlay must release its groups when something opens over it. After any UI change, a TV check reads `ui.refusals` (must stay 0) and greps the logs for `compositor refused:` and `scene is full`.

## Video rules

- A camera pop-up uses no hardware decoder and no video window while an app other than Home or LumaTV is in front (f4c0ffa8): its picture is the compositor's texture, or stills with textures off, and a granted pop-up window ends when such an app comes forward (reason `app-front`). Holding the sub window's decoder over Netflix's browse screen made the next 4K Dolby Vision title fail with Netflix errorCode 600.
- A video window follows LG's decoder policy, which can take the decoder back; anything that claims one must tolerate losing it.

## Receiver keeper

With `denon-receiver` on and the receiver as the TV's sound output, the helper keeps the receiver on for the whole time the screen is on (`helper/app/src/receiver.rs`, `Keeper`). The output it keeps is remembered in `<data>/receiver-output.json` (`receiver_sync::Intent`), because LG saves `tv_speaker` itself about 1 s after the receiver leaves ARC: a `tv_speaker` saved while the receiver is not on keeps the receiver, one saved after it has read on for `hideAfterMs` releases it. A wake, or a helper start with the screen on, sends `ZMON` to a receiver not read as on; from `powerOnMs` (5 s) after a power command a receiver read as off is powered on again (backoff to `maxBackoffMs`, 30 s, for as long as the screen is on) and set to `SITV` once it reads on; switched off by hand it gets one more `ZMON`. Unreachable or still off, the live route moves to the TV speakers until it reads on. Nothing powers it on while the screen is off. LG's CEC wakes the receiver only when its last output was ARC (`Last ARC(1)` in the TV syslog). See `docs/plugins/denon-receiver.md`.

## Tuner

`live-tv-tuner` (docs in `plugins/live-tv-tuner.md`) serves a one-tuner HDHomeRun on port 5004, private networks only. With its messages on, a request for a tuner in use is answered 200 with 6 s of a slate naming the holder (Plex recording a programme, another client, or the TV's own Live TV), then ends; with them off it is 503 `805`. `GET /status.json` names the holder (`ip`, `name`, `channel`, `channelName`, `since`) or `{"holder": null}` without tuning. A request from the same client address as the current holder takes the tuner over (the earlier stream is told to end); another client's running stream keeps it (`helper/tuner/src/stream.rs`, `Tuning::take`).

## Test events

`POST /v1/cameras/event` fires a test pop-up only with `cameras.testEvents` in `/var/lib/lumatv/helper.json`; otherwise it answers 403 `test_events_off`. Tests that turn it on must put `helper.json` back as found, restart the helper (`tools/lumatv helper stop`, `helper start`), clear the pop-up (`POST /v1/cameras/popup/clear`) and confirm a test event is refused. Leaving it on makes pop-ups appear unprompted and spoils the next frame-gate run.

## Device facts

LG OLED65C4PUA (model code `o22n2`), webOS 10.2.1 build 5202 (`10.2.1-5202`), firmware 33.22.52, MediaTek `O22A3`, 2 GB RAM, armv7 userspace with an aarch64 kernel, 120 Hz panel. Rooted with Homebrew Channel. The compositor's persistent enable is allowed only on firmware builds in `compositor/firmware/verified.list`. A plain `reboot` returns powered on; luna `machineReboot` drops to standby and does not come back. Set-wide detail and the dead ends: `HANDOFF.md`, "The set" onward.

Quick Settings' signal chips (size, frame rate, refresh, VRR, range) read inputs from the source LG's Game Optimizer uses: `luna://com.webos.service.utp.extinputs` `getVideoInfo` and `getVRRInfo` (`{pipelineId, mode: "GameOptimizer", interval: 1000}`; `vrrInfo.frameRate`, `vrrType`), on the pipeline the input's broadcast binds. That service answers only for HDMI/external inputs: with Netflix in front `getVideoInfo` and `getVRRInfo` return -106 (no resource) and `bind` -101 (no active broadcast). Apps therefore keep `videooutput` `getStatus`; the panel's refresh is still counted from `osd_irq` in `/proc/interrupts`.
