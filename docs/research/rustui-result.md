# Rust UI spike: LumaHome in LumaShow's toolkit as a 64-bit Wayland client

Phase 3 spike (`rustui/`). Question: can LumaShow's Rust UI toolkit draw LumaHome as a 64-bit Wayland client on the C4, and is it
faster or smoother than today's QML LumaHome inside LG's Qt 5.12 compositor? The answer decides whether new surfaces are written in Rust.

## Resume here

State (2026-10-25 03:45 EDT): the client is built and passes on the host (Mesa, offscreen and as a window on a headless Weston); the
TV slot has not run yet. Next steps, in order:

1. Ask Main for the slot ("TV slot for RustUi: 20 min, measure"), then
   `python3 rustui/tools/tv-run.py --upload-only`, a short look (`SHOT=4 python3 rustui/tools/tv-run.py look idle:8`, which also saves
   a panel screenshot to `rustui/build/runs/look.png`: check the picture is upright, else the flip pass is wrong), then the measured runs
   listed under "TV runs" below.
2. Fill "TV results" and the recommendation from the numbers, commit, send "TV back".

## What was built

`rustui/` is its own Cargo workspace (`cargo build`, host tools are linked with zig through `rustui/.cargo/config.toml`, as in `compositor/`).

- `render/` (crate `lumatv-render`): LumaShow's compositor renderer, scene and session as a library. The compositor is a binary crate,
  so the modules are copied verbatim by `rustui/tools/sync-lumashow.sh` (LumaShow `0ded555c`), the same arrangement as
  `compositor/tools/sync-lumashow.sh`, plus two patches in `rustui/patches/`: `gl-window.patch` (a GL context on a Wayland window
  surface, readback, no cfg(test) around the Mesa pbuffer platform) and `live-no-gralloc.patch` (drops the Echo Show's gralloc
  importer). `render/src/host.rs` is LumaTV's own: the engine in the app's process. It runs LumaShow's `Session` (checks the app's
  requests), `Scene` (animates layers on a clock) and `Renderer` (draws them) from the host's loop instead of the compositor's render and
  present threads; the app talks to it over a socket pair with the same requests and the same events as to LumaShow's compositor, so
  `ui::Scene` does not know the compositor is in its process.
- `app/` (crate `lumatv-rustui`): `home.rs` (LumaHome), `window.rs` (the Wayland window), `script.rs` (the gate's D-pad sweeps),
  `metrics.rs` (frame log), `output.rs`, `input.rs` (evdev keys), `look.rs` (theme.json and the QML shaders as toolkit glass).
  Dependencies: `ui` and `compositor-protocol` by path into `/data/home/LumaShow/lumashow`, which is not modified.
- The window is made as `docs/research/m64/wl.c` does: `wl_shell` toplevel, `wl_webos_shell_surface` (appId, window type OVERLAY, state
  fullscreen) over a `wl_egl_window` of the 64-bit Mali blob, run through `gfx64/launch.sh` and the heap shim. The Wayland connection is
  libwayland-client's, loaded at run time (the bundle's, which the blob links), through the `wayland-client` crate's system backend;
  without `wl_shell` and `wl_webos_shell` (a host's Weston) it falls back to `xdg_shell`.
- LumaHome subset, from `payload/theme/{theme,layout}.json` and the QML: the Silk wallpaper on the shared 10-a-second tick
  (`Wallpaper{fps:10}`, the compositor steps it), the status bar (profile pill, three buttons), clock card (analog face, hands redrawn once
  a second, weekday, date and week strip), "At home" card (four chips), news card (hero picture, headline, meta, page dots), weather card
  (place, icon, 84 px temperature, forecast rows), the dock (24 tiles, 12 by 2, plates and icons, one accent halo, one name pill per tile),
  all glass over the cached 30 px blur of the wallpaper (`Backdrop::Cached`), Manrope text as glyph quads of one atlas (every label and text
  layer is a run; zero pictures), and `LumaHome.qml`'s focus layers and rule (highlight at once on every key; lift 1.12 / 1.03 and the name
  after 150 ms of rest; nearest-in-direction moves between cards, status bar to cards to dock). Keys: the gate's scripts (`dpad`, `sweep`,
  `mix`) or evdev.
- Window orientation: the renderer's targets are laid out as a LumaShow scan-out page is (panel row 0 first in memory, the top of the
  screen), and a window surface is the other way up, so the scene is drawn into a panel-sized target and one pass copies it into the
  window's framebuffer turned over (`--direct` skips the copy and shows upside down, to measure what it costs).

## Running

- Host, offscreen: `cargo build --release -p lumatv-rustui && ./target/release/lumatv-rustui --assets ../payload --run idle:3,dpad:7 --shot /tmp/home.png`
- Host window on Weston: `cargo build --release --features window`, then `--mode window --libmali libEGL.so.1` with `LIBGL_ALWAYS_SOFTWARE=1`.
- Set: `sh rustui/tools/build-tv.sh`, then `python3 rustui/tools/tv-run.py NAME PHASES [options]` (needs a slot). Options: `--direct`,
  `--gpu-sync` (wait for the GPU each frame so `gpu-done` is the frame's time through the GPU), `--no-trim`.
- Log: one `RESULT` line a phase (fps, gap p50/p90/worst, frames over 20 ms, animated frames and dropped, render CPU p50/p90, client CPU,
  LG compositor CPU, GPU busy, RSS); the ftrace markers `P:<phase>` let `analyse.py` cut the same run by presented frames.

## Host checks (2026-10-25)

Mesa on the workstation's Intel GPU, offscreen: the picture is the QML Home's (look and layout) and every text layer is a glyph run
(`text pictures 0`); a 7 s `dpad` phase holds 55-58 fps with 0.6-0.9 ms CPU to issue a frame (a GPU that is not the set's, paced by sleep:
no figure here says anything about the set). As a window on a headless Weston (llvmpipe, `xdg_shell`): the whole plumbing works (system
Wayland backend, display and surface pointers into EGL, window surface, swap, event dispatch): 198 frames in 5 s of `dpad`, 39 fps because
llvmpipe is the renderer.

## TV runs

Not run yet. Planned (one slot, about 20 minutes, screen on, the QML Home hidden behind the overlay):

1. Upload, 8 s look with a screenshot.
2. `idle:60,mix:30` twice (idle 60 s, then the `dpad` and `sweep` scripts of the QML gate in turn for 30 s).
3. `idle:10,mix:30 --direct` (what the flip pass costs) and `mix:30 --gpu-sync` (frame time through the GPU).
4. The QML numbers to compare with: `docs/research/perf/summary-2026-10-24-phase1.txt` and SurfacesP2b's latest Home gate.

## TV results

Pending.

## Recommendation

Pending the TV results.
