# LumaGame architecture

One native webOS app that plays games from several engines: GeForce NOW, Punktfunk, Sunshine/Apollo and the TV's own game apps
first; emulators and Steam Remote Play later. An engine is an integration, off until it is switched on in the app. The library
has one card per game however many engines can play it.

The rules that touch no TV are code: `game/crates/lumagame-core` (engine contract, catalog, launch resolution, session machine,
saved state), tested on the host. This document holds the decisions around it. Display features and their qualification steps are
in `docs/lumagame-gaming-features.md`; product and visual rules in `PRODUCT.md` and `DESIGN.md` beside this file.

Nothing below has run on the TV as LumaGame. A fact marked **measured** names its receipt; **inferred** and **untested** items are
listed again at the end and must not be built on as if they held.

## Processes

```
SAM ─► main  lumagame            32-bit C, gfn/feeder sources
        │                        owns SAM/LS2 registration, LG's player (NDL) and the video plane, the UI's lifetime
        │  lifecycle channel: JSON lines on a socket pair
        ▼
       bin64/lumagame-ui         64-bit Rust on rustui
        │                        owns the window and its exported video element, input focus, the catalog, launch resolution,
        │                        the session machine, engine supervision, the saved state, the helper lease
        │  engine IPC: JSON lines on the engine's stdin/stdout (lumagame_core::ipc)
        ▼
       engines/<id>/…            64-bit, one process per engine that is switched on
        │                        owns its protocol, its credentials, its network, the controller reader while a game runs
        │  gf_proto on /tmp/org.nphil.lumagame/feed.sock: coded video and audio
        ▼
       main (the feeder) ─► NDL ─► decoder ─► video plane under the UI's transparent window

LumaTV's helper (root): the `gaming` plugin, on the loopback API. Optional; the app plays without it.
```

- **`main` is the feeder.** An app's LS2 role is generated at install for `main`'s exact path, and NDL registers under it; any
  other program needs a role written as root (`docs/research/gfn-app-result.md`). The GeForce NOW app already runs this way.
- **The UI cannot be `main`.** It is 64-bit and needs the package's private loader and Mali runtime, which a 32-bit launcher starts.
- **Engines are separate programs.** A crash becomes a failed session, not a dead app. A GPL engine stays a separate work. Each
  keeps its own dependencies (OpenNOW's patches, a QUIC stack) out of the others. Only switched-on engines run, and none while the
  app is in the background with no session: the UI closes their pipes and starts them again when it returns, over the saved catalog.
- **One feeder for every stream engine.** One owner of the plane and of the NDL rules below. A second feeder playing into another
  process's exported window fed frames and stayed black (**measured**, `docs/research/round9-result.md`), so the feeder takes one
  engine connection at a time and refuses a second.
- **Rejected:** a 32-bit monolith like Punktfunk's client (gives up the measured 64-bit UI path and the tested GeForce NOW engine,
  and needs every engine rebuilt for armv7); one app per engine (no single library, no chooser).

Death is handled by the kernel, not by messages: `main` sets `PR_SET_PDEATHSIG` on the UI, the UI sets it on each engine, an
engine exits when its stdin closes, and the feeder unloads the player when its engine's socket closes.

LumaGame's Settings › Diagnostics is off with no server, topic or token by default. A private `diagnostics.json` in the app data directory stores its settings and write token (0600). Events use an allowlist and exclude account details, URLs, controller keys and log tails. One worker batches bounded events at five-second intervals without an idle heartbeat; a short-lived sender child uses HTTPS and can be killed while resolving or sending. Turning diagnostics off or changing the destination discards the queue and stops the child; messages already delivered cannot be recalled. On a reported fatal error, the app attempts a final batch for at most two seconds; delivery is not guaranteed and abrupt termination may prevent the attempt. Draw and swap records are not evidence of pixels reaching the panel.

Settings › Diagnostics also has a Log level, Info by default and for a `diagnostics.json` written before levels existed. Each level sends everything in its row and the rows above it; changing the level discards the queue and puts a `level` line first in the next batch. The allowlist is the same at every level: each field is a number, a bool or a fixed word chosen by the app.

| Level | Adds (event: fields) | KB a minute, this level and above |
|---|---|---|
| Error | `error`: kind. `test`. `level`: level. `session_end`: session, engine, outcome, reason, by, seconds run, when the session failed. | under 0.5 |
| Warn | `hdr_missing`: session, engine, why. `frame_trouble`: drops by cause, stalls, longest gap, and the packets the TV's own network stack dropped since the last report where the engine counts them. `net_loss`: loss, RTT, jitter. `engine_error`: engine, code. `no_picture`: session, whether the video window is muted, its size, source rate and the engine's fps, when no sized, unmuted window shows five seconds after the stream ran. `controller`: slot, when a pad leaves. `session_end` when the engine, its exit or a timeout ended the session. | 2.0 |
| Info | Unchanged: `lifecycle`, `draw`, `swap`, `render`, `stream`, `frames`. `session`: phase, milliseconds since the session began, queue position and estimate. `stream_format`: codec, size, fps, bit depth, HDR given and asked, why SDR, audio channels, fps and height asked. `display`: size, HDR type, whether the video window is muted, refresh, source rate, VRR, low delay, at stream start, on change, at its end and three seconds after. `first_picture`: session, milliseconds from the stream running and from the feeder's first frame to the first sized, unmuted video window. `controller` when a pad joins. Other `session_end`. | 22.7 |
| Debug | `detail` every two seconds: arrival rate, frame gap and arrival-to-feed time (p50, p95, p99, max), queue depth, late frames, stalls, client and host time, pad bursts and gap, panel refresh counters. | 29.3 |
| Trace | `hist` every six seconds: counts of frame gaps (7 bins), feed times (8) and queue depths (5). `trip` after the Lab's round-trip test: press, network, host, arrival and client times. | 30.5 |

A batch holds at most 3,072 bytes and goes at most every five seconds (36.9 KB a minute), and the queue holds 128 lines; these follow ntfy's 4,096-byte message and request rate and the sender child's 8 KiB input. A Trace line is refused once the queue holds three quarters of a batch and a Debug line once it would exceed one, so Info and above are shed only by a full queue; the next batch starts with a `dropped` count. The figures are worst-case estimates from a synthetic stream (a `render` line every second, 2.5 % loss, a stall every five seconds), not measured on the TV; an ordinary stream is about a quarter smaller. `display`, `first_picture` and `no_picture` need the helper's Gaming plugin. The panel is read once a second during a stream (at Warn only its video window, until a picture shows or is reported missing). The two picture lines come once a session, from a stream the app saw start, and count only a window read after the feeder took a frame. Frame timing is of frames arriving at and fed to the player: the app has no decode or present counts, and the panel refresh counters are the nearest evidence of the display.

## Engines

The app knows no engine by name. It reads `engines/<id>/engine.json` (`engine::Manifest`: name, summary, program, licence, source
address) to list an engine while it is off, starts it when it is switched on, and from then on speaks only the IPC. A new engine
is a new directory and program.

| Engine | id | Targets | Linking | Picture | Built from | Licence |
|---|---|---|---|---|---|---|
| GeForce NOW | `gfn` | accounts | device code | feeder | shim over `opennow-core` and `opennow-streamer` (`gfn/engine`) | MIT |
| Punktfunk | `punktfunk` | PCs | PIN typed on the TV, or approval on the PC | feeder | `punktfunk-core`, `pf-client-core`, client-webos session logic | MIT OR Apache-2.0 |
| Sunshine, Apollo | `gamestream` | PCs | PIN typed on the PC | feeder | moonlight-common-c, in its own repository | GPL-3.0 |
| TV games | `native` | this TV | none | the game's own app | inside the UI process | MIT |
| later: emulators | per system | this TV | none | frames to the UI | a runner process, cores supplied by the owner | per core |
| later: Steam Remote Play | `steam-link` | PCs | PIN typed on the PC | feeder | ihslib, in its own process | LGPL-3.0 |

Contract (`lumagame_core::engine`, `::ipc`, protocol 1):

- `Engine` is a message-passing trait: `send(ToEngine)`, `poll() -> Option<FromEngine>`. An in-process engine implements it; a
  process is reached through an adapter that writes and reads lines. Tests script one.
- Requests: `hello`, `settings`, `targets`, `add-target`, `forget`, `wake`, `auth`, `auth-pin`, `auth-cancel`, `library`, `check`,
  `launch`, `stop`, `route`, `stats`, `action`. Events: `ready`, `targets`, `auth`, `library`, `readiness`, `session`, `stats`, `key`,
  `overlay`, `error`.
- Session states are the ones the app shows: checking, sign-in, queue, starting, running, ended, failed.
- An engine declares capabilities (`Feature`: discovery, wake, queue, resume, end-game, stats, overlay-keys, …); the app offers a
  control only when its engine names it.
- The app owns every screen. An engine supplies a code, a PIN or a wait (`AuthState`), and a settings page as data
  (`settings::SettingsSchema`); it never draws.
- Settings values are kept by the app and sent in `hello`. Credentials are kept by the engine under `engines/<id>/` and never read
  by the app.
- Unknown event types, enum words and fields are tolerated in both directions, so an engine built elsewhere survives a newer app.
  A change either side could not make sense of takes a new protocol number, which the manifest names.

Engine decisions:

- **GeForce NOW:** `lumagame-engine-gfn` translates the IPC to OpenNOW's core protocol; `opennow-core` and `opennow-streamer` stay as
  they are. The protocol rules now in `rustui/app/src/gfn/` move into the shim. Its settings schema takes its defaults from a
  migrated `prefs.json` when one is present (see Migration).
- **Punktfunk:** a 64-bit engine on `punktfunk-core` and `pf-client-core`, with the session logic of client-webos ported, feeding
  the shared feeder. Rejected: running Punktfunk's own 32-bit client as a child (it is not `main`, so it has no NDL role, and it
  would be a second NDL path). Risks: the `aws-lc-rs` cross-build for aarch64 glibc 2.36; parity with the stock client, to be
  measured A/B on the TV. DualSense extras are out of the first version.
- **Sunshine/Apollo:** moonlight-common-c, as the reference client, in one GPL-3 engine program in a repository of its own. That
  repository depends on this one's MIT crates; nothing GPL is vendored or linked here. The LumaGame package carries the built
  program as a separate work with its licence and source address (the manifest refuses a copyleft engine without one), fetched
  at build by a pinned version and checksum. Reasons: wire compatibility and loss recovery that years of hosts are tested against,
  and the least code. Revisit with a clean-room MIT client modelled on punktfunk-host's `gamestream/` if the release logistics
  or an untunable latency path block it. Built after Punktfunk.
- **TV games:** the owner picks which installed apps are games. Launch goes through `main`'s LS2 handle
  (`applicationManager/launch`) on the lifecycle channel. The session ignores the app going to the background and asks for no
  low-latency lease; playtime runs until the game leaves the foreground. Foreground tracking is **untested**.
- **Emulators:** a runner process hands finished frames to the UI over shared memory (`Presentation::Frames`). Cores and ROMs are
  the owner's; none ship.
- **Steam Remote Play:** ihslib in its own LGPL process, entries as launchers.

## Media path

Coded video and audio go from the engine to the feeder over `gf_proto`; nothing media-related crosses the JSON channel. The
controller is read in the engine's process and never passes through the UI's frame loop.

Feeder changes, one owner (`gfn/feeder`). Each is Punktfunk's rule for NDL on this TV family
(`client-webos/docs/NOTES.md`) and needs its own A/B measurement here before it is called an improvement:

1. Timestamps are milliseconds since the load call, rounded up, non-decreasing per plane, enforced in the feeder.
2. An Opus audio plane is always loaded and primed with the empty frame through the load window.
3. No flush mid-stream or before load completes. On loss the feeder holds and asks the engine for a key frame; it flushes only
   on a decode error.
4. Real audio starts after load completes and is stamped from the player's clock.
5. HDR metadata is applied once and again only when it changes; never for SDR.
6. A load that never completes ends the session with an error instead of hanging; a fatal player error latches, refuses further
   units and tells the engine once, so the session ends as failed (`video`) rather than black.
7. App id, runtime directory and UI program are build settings, so the same sources build LumaGame's `main`, the old app's and
   the helper's live TV sink.
8. The lifecycle channel gains `launch`/`launched` and `foreground` for TV games.

The refused-unit accounting and sticky-PCM faults are fixed (`f692e399`). 4K120 HEVC Main10 at 80 Mbit/s fed for 300 s without a
drop (**measured**, `docs/research/round9-result.md`): that is throughput, not latency.

## Catalog

`lumagame_core::catalog`.

- Two entries are one game when they share an external id (`steam:1245620`, `gfn:<id>`, `webos:<app id>`, `igdb:<id>`) or the owner
  linked them. A title never merges; equal titles are only offered for linking.
- A launcher (Desktop, Big Picture) is listed and never merges.
- An engine's own id counts only on its engine and target.
- `Identity` is saved: a game keeps its `GameId` while engines are switched off and PCs sleep. A source that bridges two known
  games folds the newer into the older and reports it, and favourites, history and remembered targets follow.
- The owner's link or unlink of one source outranks every id.
- `Listing` (`catalog.json`) is the last library of every target, so shelves draw before any engine answers. It is read back as
  unchecked: a saved "ready" is not a checked one.
- Saves are called shared only when store, store account and the store's cloud-save support are all known; otherwise separate or
  unknown.

Artwork and metadata: Steam's own images by app id lead when a game has a Steam id (uniform sizes), then each source's own, then
a drawn card. All of it is fetched and cached on the TV (`cache/art/`, bounded, least recently used first out) and never
committed or redistributed. SteamGridDB and IGDB are optional, with the owner's own keys in `secrets.json` (0600) and their
credits in About.

History (`history`): only sessions that reached Running. Totals per game and the last 200 sessions. The running session's time is
checkpointed every minute to `session.json` and recorded once, so a power cut loses at most a minute and never counts twice. It is
LumaGame's playtime on this TV and is labelled so.

## Play

`launch::resolve` picks in a fixed order: the target pinned for the game, the engine preferred for all games, the target that
last played it, the only target checked ready, then the chooser.

- Play never switches silently. A remembered target that is offline, lacks the game, or is gone opens the chooser on itself with
  its state; nothing else starts in its place.
- Only `ready` is a checked fact. A source is no readier than its target (`Candidate::of`). `unknown`, `asleep` and `sign-in` may
  be tried when the owner already chose that target; they never make a target the automatic pick.
- A game with one way to play it starts that way unless it is known to be blocked; there is nothing to choose.
- A target is remembered only when its session reaches Running.

`session::Session` is a pure machine: inputs with a time, effects out. Low-latency mode is taken at Running and given back after
the stream has stopped. Leaving stops the stream, not the game, unless the owner asks to end it. Checking times out at 30 s,
Starting at 120 s; a stop the engine does not confirm in 10 s ends the engine's process. Every exit ends in one `Finished`.

## Saved state

The data directory is the first writable of the feeder's `persistent_dir` bases for the app id (which one the jail allows is
**untested**).

| Path | Holds | Lost means |
|---|---|---|
| `state.json`, `state.json.bak` | profiles (favourites, hidden, collections, remembered targets, history), engines on/off and settings, identities | the owner's choices; written by rename, previous copy kept |
| `session.json` | the running session's playtime | under a minute of playtime |
| `catalog.json` | last libraries | nothing |
| `secrets.json` | metadata API keys, 0600 | keys are typed again |
| `engines/<id>/` | that engine's credentials and pairings | one sign-in or pairing |
| `cache/` | artwork, engine caches | nothing |

`state.json` has one `schema` number. An added field with a default needs no new number. Anything else takes a new number and one
step function; an older file is brought forward step by step and the old file kept aside. A file from a newer build is refused
and not written over: the app runs on defaults without saving and says so. A corrupt file falls back to the `.bak`.

Profiles hold choices and history. Engine accounts are the household's, not per profile.

**Migration of the GeForce NOW sign-in.** Jails are per app id, so the new app cannot read the old one's files. A root step
(`lumatv game adopt-gfn`) copies `config/`, `data/` and `prefs.json` from the old app's data directory into
`engines/gfn/`, keeping modes, without moving them. The shim uses `prefs.json` for its defaults. If the copy is missing, the cost
is one device-code sign-in. The feeder makes the app's directory under every base it tries, so the step takes LumaGame's to be the
one holding the `cache/` the UI makes on every start (LumaGame has to have started once), gives the copy to that directory's owner,
and never writes over an existing `engines/gfn/config/`. The copy is **untested on the TV**: the GeForce NOW app there kept its
sign-in under `/tmp`, which a restart clears.

## Identity and cutover

App id `org.nphil.lumagame`, runtime directory `/tmp/org.nphil.lumagame/`. It installs beside `org.nphil.lumatv.gfn`. When the
`gfn` engine has passed on the TV, the old package, `rustui/app/src/gfn/`, `bin/gfn.rs`, `tools/gfnapp.sh` and the `gfn` names in
the helper are removed in one change. `gfn/feeder` and `gfn/engine` stay where they are.

## Helper: the `gaming` plugin

First-party, off until enabled, routes under `/v1/gaming/` on the loopback API, refused while off. The app works without it: NDL
already selects game mode and low delay by itself.

- **Lease.** `POST /v1/gaming/lease {pid, controls}` saves each control's value, sets it, reads it back, and answers what was
  applied and what was skipped and why. `POST …/renew` every 2 s; `DELETE` restores. The holder is the UI.
- **Restore without the holder.** The helper holds a pidfd on the UI: its death is seen at once, which is the "pipe close" of the
  features contract with no new transport. A lease not renewed for 6 s is restored too (a hung UI). So are plugin-off and standby.
- **Restore without the helper.** Saved values go to a journal on tmpfs, with the boot id, before the first set. A starting helper
  replays a journal from the same boot and discards an older one. **Untested**; until it passes, the lifecycle counts as
  unqualified, as the features contract says.
- **Controls.** Only those that passed the features contract's qualification on the TV are applied; the rest are reported as
  unqualified. At the start that is none, so a lease changes nothing yet.
- **Controller boost.** `gfn_boost.rs` generalised: same policy (FIFO 10, off CPU 0, 90 s lease; built and host-tested, its
  effect on the TV not yet measured, `docs/research/gfn-input-result.md`), gated by this plugin, accepting both app ids until
  cutover.
- Rejected: a Unix socket with peer credentials (whether the jail can reach a root socket is unknown; the loopback API is the path
  the controller boost already uses).

## UI

rustui. Its frame cost has a model fitted on this TV (about 1.5 ms of GPU plus 0.085 ms a layer, `docs/research/frames-120-result.md`),
it redraws by damaged tile, it carries the token and theme system every surface must take after, and it is already the 64-bit window
the GeForce NOW app streams under. Rejected: Punktfunk's Skia shell (a second theme system, GPU raster as its slow path, no
stability promise).

Three parts under `rustui/app/src/game/`, so screens, rules and plumbing change apart:

- **Shell** (`bin/lumagame.rs`, `host.rs`, `store.rs`, `helper.rs`, `native.rs`): window, engine processes, files, lease, lifecycle.
- **Model** (`model.rs`): a pure reducer over `lumagame-core`. Inputs are keys, engine events and ticks; outputs are commands to
  the shell and a view. Tested without a window.
- **Screens** (`screens/`): home, library, hub, chooser, integrations, settings, sign-in, pairing, overlay, states. Read the view;
  colours, glass, radii and type only from tokens.

While a game runs the window stays mapped and transparent and draws nothing unless the overlay is open.

## Who restores what

| Dies | Seen by | Result |
|---|---|---|
| Engine | UI (pipe closes) | session fails (`engine`); feeder unloads on socket close; lease released; engine restarted on next use |
| Engine hangs on stop | session machine (10 s) | UI ends the process; as above |
| Feeder's player | feeder | fatal latch, engine told, session fails (`video`) |
| UI | `main` (child exit), helper (pidfd) | `main` exits; engines exit on closed stdin; helper restores the picture |
| `main` | kernel | UI gets SIGTERM; as above |
| Helper | UI (renew fails) | session goes on without the lease; journal replay restores at helper start (**untested**) |
| Power | — | TV state resets with the boot; `session.json` is recorded at next start |

## Build and test

- Host: `cargo test` and `cargo clippy` in `game/` (core, shim, feeder client, each engine against a scripted far side and a fake
  feeder), `rustui` (`game` model and screen sheets), `helper` (the plugin against fake controls and a fake `/proc`),
  `tests/test_gfn_feeder.py` (feeder against the fake NDL). No test sleeps; clocks are injected.
- Target: engines and UI with `cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36`; `main` with
  `gfn/feeder/build.sh`. `game/build-ipk.sh` packs `org.nphil.lumagame`, which LumaTV's package carries as `payload/game/`.
- TV, each in a leased window with its receipt: role and NDL under the new app id; data directory; sign-in migration; each engine
  from link to stop; each feeder change A/B; helper lease set, verify, restore and crash paths; 120 Hz frame gate for the UI.

## Slices

Each has one owner and its own files. "Core" is `game/crates/lumagame-core`, done.

| Slice | Files | Needs | TV |
|---|---|---|---|
| Feeder | `gfn/feeder/**`, `tests/test_gfn_feeder.py` | — | yes |
| Feeder client | `game/crates/lumagame-feeder/**`, `gfn/engine/crates/opennow-streamer-platform/src/{feeder,runtime}.rs`, `gfn/engine/Cargo.toml` | Feeder (`gf_proto`) | no |
| GeForce NOW engine | `game/crates/lumagame-engine-gfn/**` | Core | yes |
| Punktfunk engine | `game/engines/punktfunk/**` (own workspace) | Core, Feeder client | yes |
| Sunshine/Apollo engine | separate GPL-3 repository; here only `game/tools/fetch-gamestream.sh`, `game/engines.lock` | Core, Feeder client, decision 1 | yes |
| Helper plugin | `helper/app/src/gaming/**`, `helper/app/src/gfn_boost.rs`, registration in `helper/app/src/{api.rs,plugins/local.rs}`, `docs/plugins/gaming.md` | — | yes |
| UI shell | `rustui/app/Cargo.toml`, `rustui/app/src/bin/lumagame.rs`, `rustui/app/src/game/{mod,host,store,helper,native}.rs`, `rustui/app/src/lifecycle.rs` | Core, Feeder (lifecycle lines) | yes |
| UI model | `rustui/app/src/game/model.rs` | Core | no |
| UI screens | `rustui/app/src/game/screens/**`; theme additions in `rustui/tokens/**` | UI model, `DESIGN.md` | yes |
| Artwork and metadata | `game/crates/lumagame-meta/**` | Core | no |
| Package and install | `game/package/**`, `game/build-ipk.sh`, `tools/gameapp.sh`, `tools/lumatv` (game commands), `tools/build-ipk.sh`, `tests/test_gameapp.py` | UI shell, one engine | yes |
| CI | `tools/check-all.sh`, `.github/workflows/build.yml`, `.gitea/workflows/build.yml` | — | no |
| Cutover | removal of the old app (see Identity) | GeForce NOW engine accepted | yes |

Contracts between slices: the engine IPC (Core); `gf_proto` (Feeder); lifecycle lines (Feeder); `/v1/gaming/*` (Helper plugin);
the model's inputs, commands and view (UI model); the data directory layout and manifest (this document, Core).

## Not yet known

- Whether the installer's role covers NDL for the new app id, and which data directory the jail allows.
- Every feeder change's effect on this TV; the Punktfunk engine's parity with the stock client.
- Foreground tracking and return to LumaGame for TV games.
- Every privileged display control (ids and paths are from disassembly), and the helper-crash rollback.
- Whether the new app's jail can read the old app's data directory (the root copy step assumes it cannot).

## Decisions for the owner

1. A second public repository under GPL-3 for the Sunshine/Apollo engine, with its program shipped inside the LumaGame package.
   Without it the engine is a clean-room MIT client, which takes longer and starts less compatible.
2. The app id `org.nphil.lumagame`. It is permanent once installed; a later change is another migration.
