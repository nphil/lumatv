# Rust shell, phase 0 and the first slice: result

What `docs/research/rust-shell-plan.md` section 5.4 asked for, built on the host on 2026-10-29. The plan's section 6 holds the
commands the next TV round runs; `rust-shell-kit.md` is the kit's reference for the screen builders.

## Local appearance extension (2026-11-01)

The local TV materials are Clear, Frosted, Liquid and Glazed. Liquid follows the native Marquee topbar’s outward quadratic edge lens; Glazed follows LumaShow’s dense, palette-bound card and sheet surfaces. Strength is saved separately for each material; Depth alone controls geometric elevation. The shared Lucent language and installed TV are unchanged pending rendered-design approval and separate maintenance authorization.

Settings > Home & look > Themes contains20 light and20 dark variants. Focus previews immediately, confirmation saves, and cancellation restores the last confirmed palette. Catalog roles use composited text contrast; thumbnails composite over their own palette rather than forcing translucent roles opaque. Effective native text scale determines gallery columns. Protocol14 requires matching renderer and client artifacts, not replacement of a frozen legacy payload. Final native-gallery and persistence verification remains pending.

The existing scripted host drains overdue actions before one draw, whereas the native set loop attempts a commit between received messages. Palette coverage requires a rendered frame between choices, not a count of state evaluations. Native fatal diagnostics use the existing Unix datagram log sink; stdout alone does not carry them.

## Current compact toast and audio update (2026-10-31)

The approved source changes are host-native tested, not installed or accepted on the TV. At 1080p the normal toast is 320×66 at x800, y960, with 54 px bottom padding and a 24 px caption; its width spans 320–480 px. A short toast at 200% text is 408×100. The newest toast replaces the current one and resets its deadline; none resurfaces after expiry. LG transient feed 28737 feeds the toast only, not NoteLog subscription, history or unread counts; genuine TV, plugin and AI entries remain.

Settings > Home & look > Layout of pop-ups controls position through `layout.overlays.toast {anchor, dx, dy}`; there is no new setting. A native edit moved x800 to x776 and persisted bottom anchor, dx −0.0154 and dy 0; an actual native process restart read it back at x776.

Original RGBA audio PNG glyphs decode and render left of the number and bar; PCM, unknown formats and mute have no fabricated codec fallback. The existing module path, format mapper, helper path and freshness handling were already in place. Host rendering does not diagnose or verify the missing glyph on the actual TV. Volume receives channel audio through `--push`, not `--luna`; receiver helper `updatedAt` must be fresh Unix milliseconds, and `--helper` injects `Up=true`.

Host Cargo UI: 1,236 passed, 3 ignored. Twenty actual native CLI cases exited 0 with `lint: []`: dark/light at 100%/200%, multiline, long word, replacement 1→0→0, saved top-right/offset, four real formats, PCM/unknown/mute, left/right, and volume plus toast with 16 px clearance. Existing `test_rust_system` / `RustSystem.layout_run` covers Settings. The `test_rust_panels` native drawer fixture retained six genuine plugin entries alongside one toast; a transient-only native drawer had zero entries, one toast and `lint: []`. The original native cache before proof showed three stacked 560×99 toasts, confirming the prior bug. Evidence: `/data/home/tmp/lumatv-ui-proof.68u1vel0`.

The placement suite passed 40 tests after removal of the obsolete toast-equals-card-width assumption. Two additional actual native app/video cases exited 0 with `lint: []`: toast alone and toast with Atmos volume over foreground Netflix (`home=false`). The 320×66 toast at x800, y823 ended at y889, preserving the subtitle band; simultaneous volume was 440×61 at y927, with a 38 px gap. Evidence: `native-video-clearance-proof.json` in the native proof directory. These are host results, not TV installation or acceptance.

The existing build script's ARM command `cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-ui` exited 0 in 100.265 s. Artifact: `rustui/target/aarch64-unknown-linux-gnu/release/lumatv-ui`, SHA256 `d979179ce6ee885bad938c557371c345edf28c8a34f7432e62150940ba61a28c`.

## Hidden shell at host 120 (2026-10-31)

The existing native Qt host with disposable frame counters used the default loaded GPU wallpaper, screen on and refresh 120. Home was foreground at 0.5 s and opaque Marquee at 7 s. In the settled 14–18 s interval Home frameSwapped remained 109, system 8 and wallpaper frameTick 61; `homeCurrent=false`, `hostActive=false` and `idleHold=true` while Home remained mapped. No active Animation or Animator QObjects were found at either sample. Quick panel and Back worked at 19 s, keyboard Right moved g→h at 21 s, and returning Home restored hostActive and keys; exit 0. Evidence: `/data/home/tmp/lumatv-ui-proof.68u1vel0/hidden-shell-120.log` and `hidden-shell-120-command.sh`.

No production Shell fix is justified by this probe; keyboard-dependent Shell must not be killed or paused. This host result does not establish the TV GPU's 4.134 ms slot, all configurations or video wallpaper. One-second frame/status timers do not prove redraws.

## Resume here

The following phase 0 resume state is historical.

Everything is on `forge/main` (commits `238e5db`, `77f552c`, `d196b64`, `d7eb540`, `258947f`, `74ea0ff`); nothing is in flight
and nothing is half-built. The default is off: a compositor started without `--shell-rust` (and without a
`compositor/shell-rust.conf` in its data directory) runs exactly as before; the QML shell is untouched.

Done:

- `rustui/tokens` (`lumatv-tokens`): `Tokens::load(data_dir)` / `from_json`, the shipped `theme.json` deep-merged under the
  owner's, resolved into the toolkit's `Look` (`Tokens::look`) through LumaShow's own resolver; type roles, radii, spacing, focus
  scales, states, the motion set, `extra(mode)` (`readingAlpha`), `mode(wallpaper_luma)` for `material: auto`.
- `rustui/ui` (`lumatv-ui`, lib + bin): `host/http.rs` (the long-poll feed client for the UI channel and the helper, one poll thread
  and one request thread per feed), `host/keys.rs` (the key socket: JSON lines `{"key":..}` / `{"pointer":..}`), `kit/`
  (`Kit`, `focus.rs`, `flow.rs`, `glyph.rs`, `hint.rs`, `motion.rs`, `pane.rs`, `text.rs`, `screen.rs`, `kit::z` bands,
  `Kit::room`), `system/` (`volume.rs` model + `volume_surface.rs`, `toast.rs` model + `toast_surface.rs`, `format.rs`),
  `shell.rs` (`Shell`: the kinds, the models fed from the channel/helper/keys/clock, the report `POST /v1/ui`, `state()`,
  `geometry()`, `due_at()`), `main.rs` (the set loop: `LUMATV_UI_FD` 3, `LUMATV_UI_KEYS_FD` 4, `LUMATV_SHELL_PORT`,
  `LUMATV_SHELL_TOKEN_FILE`, `LUMATV_UI_KINDS`, `LUMATV_UI_SYSTEM_Z`; and `--host`, the harness on Mesa with `--push/--luna/--helper/
  --eval/--snap/--frames/--theme/--mode/--text-scale/--log`).
- Compositor: `shell_rust.rs` (`RustShell`, `State::shell_rust_start/tick/poll/handle/restart/set_kinds/takes_key/pointer/report`,
  `KINDS`, `SYSTEM_Z = 1000`, `KINDS_FILE`), `config.rs` `--shell-rust LIST` (+ the kept file), `control.rs` `ctl shell rust
  +kind -kind`, `output.rs` `ui_attach/ui_handshake/ui_detach/ui_attached` (Gl and Kms), `shell.rs` routing (`shell_audio`,
  `shell_luna_event` withholding the toast feed, `shell_topic_values` shared, `shell_can`/`shell_show` per kind), `input.rs`
  (the Rust layer's keys), `main.rs` (the engine attached before the loop, `TAG_SHELL_RUST`, the engine's fd added at the
  handshake), `rustui/render/src/host.rs` `try_finish_handshake`, `ui/mod.rs` `Ui::idle` tidies after applied commits.
- Tests: `rustui/ui` unit tests + `tests/flow.rs` (the flowing lift measured on the track mirrors at 120 Hz), `rustui/tokens`,
  compositor `shell_rust` unit tests and `tests/shell_rust.rs` (end to end with the real binary on Mesa), `tests/test_rust_shell.py`
  (the client against `tests/qmlhost/channel.py`, factored out of `shellhost.py`), renders in `docs/mocks/host-rust/`.
- `compositor/tools/build-tv.sh` bundles `bin/lumatv-ui` when `LUMATV_FEATURES=ui`.

Next (in order), none started:

1. The TV round of the plan's section 6 (owned by Main's next round): `tv_ui_measure.py`, deploy with `LUMATV_FEATURES=ui`,
   `ctl shell rust +osd +toast`, the volume keys and a toast, `tv_gate.py --only shell,frames` twice. `tv_gate.py`/`gate.sh` do
   not yet read `rust.ui` of `ctl shell` for a kind on `rust.kinds`: that is the round's change (`compositor/tools/tv_gate.py`,
   the `shell` check's reading of `ui.osd`; `gate.sh`'s `jstr`/`jnum` of `ctl shell`).
2. Once the gate passes twice: delete `payload/compositor/lumatv/panels/VolumeOsd.qml` and the toast part of `panels/Notify.qml`
   (and their `tests/test_volume_osd.py`, `tests/qmlhost/scenes_notify.py` toast scenes), per the plan.
3. The other slices (RustPanels, RustSettings, RustHome, RustCameras, RustSystem) build on the kit; the shell's per-kind hooks are
   additive (`Kinds`, a field per kind in `Shell`, arms in `key/tick/after/report/state/due_at/restyle`, `KINDS` in
   `shell_rust.rs`). Keys for a modal kind: the client reports `wants: true` and the compositor sends every key but Home/volume/
   mute over the key socket (`shell_rust_takes_key`), pointer samples with it (`shell_rust_pointer` is wired only where
   `input.rs` already calls `shell_takes_key`; the pointer motion hook in `server/seat.rs::pointer_motion` is NOT yet calling
   `shell_rust_pointer`: add the call there when a kind needs the pointer).
4. The helper feed in `main.rs` subscribes to `receiver,screen,cameras.popup`; a kind that needs more topics appends to that list
   (RustSettings proposed a `helper_topics()` on `Shell`).

Decisions taken:

- The engine is attached in `main.rs` before the loop (so KMS makes its UI pages) and its client end is adopted by `RustShell`
  for the first start; a restart detaches and re-attaches (`Output::ui_detach` / `ui_attach`), which drops the old client's layers
  in one frame.
- The Rust client's channel is HTTP on `--shell-port + 1` with its own token (`lumatv-ui.token`), the same messages as the QML
  client's; the key socket is a `UnixStream` pair, JSON lines, one way (compositor to client).
- Feeds are withheld, not duplicated: for `osd` the QML client stops getting `audio`; for `toast` the toast feed's pushes to the
  QML client's subscription are dropped. Switching a kind back pushes the audio state the QML client missed.
- `kit::z` bands: panels +100, cameras +200 (RustCameras adds it), settings +300, editor +500, keyboard +600, system +800 on top.
- The flowing focus is a Lucent TV rule (`rust-shell-kit.md`, last section): one shared pool of lift tracks on 260 ms critically
  damped springs, retargetable, the scroll on the same spring, reduced motion a crossfade; no tvOS tilt until measured at 120 Hz.
- Hover is a cue, click selects (`kit/focus.rs`); D-pad search uses a beam rule (a candidate overlaps the current one across the
  axis), Up/Down may leave the beam into another region, Left/Right never; region memory; guides.
- The volume model keeps the hold-target rule of 8204d58 unchanged (`system/volume.rs`, tests mirror `test_volume_osd.py`'s cases).

## Host numbers (Mesa llvmpipe, 1920x1080)

OSD frame 3.2-4.8 % of the panel (65-100 k px), toast arrival 14 frames at 60 Hz, the glyph atlas lands under 50 ms, 0 text
pictures and 0 glyph fallbacks, 1.1-1.3 ms of CPU per damage frame; the flowing lift lands in about 270 ms, no frame with both tiles
at rest over 60 frames, the velocity continuous across a retarget, 11 rapid moves within the 8-track pool; the compositor starts
the client and draws its display under 1 s. Suites: compositor 295 lib tests and every integration test green with and without
`--features ui`, clippy clean; `test_cutover` 147, `test_shell_client` 6, `test_rust_shell` 2.
