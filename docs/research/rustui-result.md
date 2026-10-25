# Rust UI spike: LumaHome in LumaShow's toolkit as a 64-bit Wayland client

Phase 3 spike (`rustui/`). Question: can LumaShow's Rust UI toolkit draw LumaHome as a 64-bit Wayland client on the C4, and is it
faster or smoother than today's QML LumaHome inside LG's Qt 5.12 compositor? The answer decides whether new surfaces are written in Rust.

## Resume here

State (2026-10-25 07:16 EDT): the TV slot ran; results and the recommendation are below (keep QML now). The spike is closed. The one open
follow-up, if Rust surfaces are wanted after the cutover: draw the scene flipped (projection) instead of the copy pass, measure again with
`python3 rustui/tools/tv-run.py NAME idle:30,mix:30 [--direct] [--gpu-sync] --frames /tmp/rustui/NAME.csv` (screen on; a slot from Main), and
add damage tracking so an idle frame costs under 1 ms of GPU. The set was returned in Active Standby (powerOff remoteKey).

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

## TV runs (2026-10-25 07:00-07:16 EDT, one slot, screen on, LG's compositor, QML Home hidden behind the overlay)

Files are in `docs/research/rustui-runs/` (client logs, per-frame CSVs, 1 s system samples, `qml-gate.txt`, `look.png`). All through `tools/tv-run.py`:

- `look`: 8 s with a screenshot. The picture is the QML Home's, upright, text sharp (the flip pass is right). The Home builds in 255 ms.
- `rust1`: idle 60 s, then `mix` 30 s (the gate's `dpad` and `sweep` scripts in turn, 116 keys). `rust2`: idle 30 s, `mix` 30 s.
- `rust-direct2`: idle 30 s, `mix` 30 s with `--direct` (no flip copy; upside down, irrelevant to the measurement). `rust-gpusync` and
  `rust-direct`: `--gpu-sync`, the CPU waits for the GPU after each frame, which gives GPU time per frame.
- QML, same slot and set state: `perf/gate.py --surfaces home-idle,home-dpad,home-sweep --cycles 4`, plus a `capture.sh` sampler (45 s
  idle, then 25 s of D-pad and sweep keys through `kb.py`) for GPU busy and compositor CPU.

## TV results

"Over 20 ms" is a gap between presents. GPU is the whole Mali (`mali0` busy over the phase: it includes LG's compositor, which still draws the
hidden QML Home at 10 fps and composites the overlay). CPU is utime+stime; "LG" is surface-manager.

| | Idle, QML | Idle, Rust | D-pad + sweep, QML | D-pad + sweep, Rust (flip pass) | D-pad + sweep, Rust `--direct` |
|---|---|---|---|---|---|
| presented fps | not logged | 10.0 | not logged | 54.3, 54.1 | 54.5 |
| animated frames over 20 ms | 0 of 32 | 0 | D-pad 2 of 517 (0.4 %), sweep 1 of 212 (0.5 %) | 72 of 1512 (4.8 %), 81 of 1506 (5.4 %) | 6 of 1514 (0.4 %) |
| animated gap p50 / p90 | | 100 / 100.4 ms (10 fps by design) | | 17.0 / 19.9 ms | 16.5 / 17.6 ms |
| render p50 / p90 | 2.74 ms p90 (gate) | CPU to issue 1.4 / 1.9 ms; GPU-done p50 6.2 ms (5.1 direct) | 2.61 ms p90 D-pad, 2.93 ms sweep (gate) | draw call 16.6 / 19.3 ms (it blocks on the GPU queue); `--gpu-sync`: CPU 1.6, GPU-done 12.9 / 16.9 ms | draw call 16.1 / 17.2 ms; `--gpu-sync`: GPU-done 5.6 / 16.6 ms |
| client CPU | 0 | 2.6 % of a core | 0 | 11.8 %, 11.5 % | 10.3 % |
| LG CPU | 4.9 % | 8.9 % | 11.3 % | 22.2 %, 21.9 % | 22.2 % |
| GPU busy | 9.9 % | 23.4 %, 23.7 % | 25.2 % | 70.9 %, 70.7 % | 66.5 % |
| client RSS | n/a | 132-136 MB | n/a | 134-138 MB | 135-138 MB |

The two D-pad columns are not the same workload: the Rust `mix` presses keys for the whole 30 s (about 1510 animated frames), the QML sampler's
25 s has pauses (860 frames at 31.8 fps), so its GPU and CPU are for fewer animated frames. What compares directly is the drop share and
the per-frame time (the gate's render p90 of 2.6-2.9 ms against the client's GPU-done 12.9 ms p50 with the flip pass). Idle is the same
workload on both sides.

1. The client runs on the set: 64-bit Mali-G52 blob (ES 3.2), as an LG overlay, 127 layers, glyph-run text, 116 scripted keys, no GL error, no
   crash across six runs.
2. Idle costs more than QML: GPU 23.5 % against 9.9 %, 6.2 ms of GPU for each 10 fps frame, of which the flip copy is 1.1 ms (6.24 against
   5.14 ms).
3. In motion a frame takes 12.9 ms p50 and 16.9 ms p90 through the GPU with the flip pass (budget 16.67 ms) with the GPU 66-71 % busy;
   5 % of frames pass 20 ms (10 % under `--gpu-sync`). Without the copy pass, 6 of 1514 (0.4 %): the same share as QML under this compositor.
   The copy pass is what tips it over. Drawing the scene flipped (projection) instead of copying was not tried.
4. Client RSS grew 1.3-3.6 MB during each 30 s `mix` phase; not soaked, so not known to be a leak.
5. LG's screen saver starts 180 s after the last remote key and it came on between runs; `tv-run.py` now wakes it with a Home key through
   `kb.py`. The screenshot call needs 960x540 on this firmware (1920x1080 returns ERROR_INTERNAL; `tv-run.sh` fixed).

## Recommendation

Now, under LG's compositor: keep QML for LumaHome and every existing surface. The Rust client is neither faster nor smoother: idle GPU 23.5 %
against 9.9 %, 5 % of animated frames over 20 ms against 0.4 %, the GPU 66-71 % busy in motion with no headroom, and 132-138 MB of
client RSS on top of LG's compositor. Without the copy pass it only ties QML on drops, at 66.5 % GPU for the whole set.

After the cutover: the copy pass, the app-in-process indirection and LG's two-vblank floor (17 % of frames of a trivial client) do not
exist under LumaTV's compositor, so drops should match or beat QML. What does not change is the renderer's cost per frame, 5-6 ms of GPU
for a full redraw. Before Home moves to Rust: an idle 10 fps frame must cost under 1 ms of GPU (redraw only what changed: not in the toolkit
today [INFERENCE: the idle cost is the same with only the clock hand changing, not tested]) and Home must pass `gate.py` (animated p90 under
3 ms, none over 20 ms) in LumaTV's compositor. New surfaces: QML until that gate passes with a Rust surface; the toolkit stays the
compositor's own renderer.

Not measured: pointer movement, video behind or in front, Quick Settings in Rust, the same frame under LumaTV's compositor (not live yet).
