# Rust UI spike: LumaHome in LumaShow's toolkit as a 64-bit Wayland client

Phase 3 spike (`rustui/`). Question: can LumaShow's Rust UI toolkit draw LumaHome as a 64-bit Wayland client on the C4, and is it
faster or smoother than today's QML LumaHome inside LG's Qt 5.12 compositor? The answer decides whether new surfaces are written in Rust.

## Resume here

State (2026-10-25 05:35 EDT): the client is built and passes on the host (Mesa, offscreen and as a window on a headless Weston); the
aarch64 build is current. No TV run yet: Main postponed the slot (Netflix debugging held the set until 07:00; a 15 minute slot had been
offered for 07:35-07:50, then withdrawn). The whole slot is scripted. Next steps, in order:

1. When Main grants a slot ("go RustUi2"), with the screen on (`tv-run.sh` refuses in Active Standby): `sh rustui/tools/slot.sh [HHMM]`
   (deadline default 0748; extras are skipped when time is short). It uploads, takes the 8 s look (`rustui/build/runs/look.png`: the
   picture must be upright, else the flip pass is wrong), runs `idle:45,mix:30`, then `--direct` and `--gpu-sync` runs if time is left,
   and prints the RESULT lines. Needs about 6 minutes for the first two steps, 15 with the extras.
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

Not run yet; `rustui/tools/slot.sh` is the plan (screen on, the QML Home hidden behind the overlay):

1. Upload, 8 s look with a screenshot.
2. `idle:45,mix:30` (idle 45 s, then the `dpad` and `sweep` scripts of the QML gate in turn for 30 s).
3. If time is left: `idle:15,mix:30 --direct` (what the flip pass costs) and `mix:30 --gpu-sync` (frame time through the GPU).
4. The QML numbers to compare with: `docs/research/surfaces-p2-result.md` Slot 1 (Home idle pooled p90 render 2.57 ms over 69 frames;
   D-pad 2.79 ms, 6 of 533 animated frames dropped; sweep 2.91 ms, 0 of 250; under LG's compositor 17 % of frames of a trivial client
   take two vblanks, the X1 floor) and `docs/research/perf/summary-2026-10-24-phase1.txt` (idle Home 12.5 fps presented, compositor CPU
   4.3 % of a core, GPU busy 10 %; D-pad sweep 3.9-4.4 % of animated frames dropped). The Rust run is an overlay under the same LG
   compositor, so the X1 floor applies to it as well; compare its drops with that floor, not with zero.

## TV results

Pending.

## Recommendation

Not decided: there is no TV measurement yet. Established on the host: LumaShow's toolkit draws LumaHome's look and layout as a 64-bit
Wayland client with zero text pictures, so what is open is speed and smoothness on the C4's Mali. Decision rule, to apply to the TV results:

- Now (under LG's compositor): Rust is better only if its dropped-frame share is clearly under the QML Home's 1.1 % (D-pad) and its idle
  GPU busy and CPU are at or under 10 % and 4.3 % of a core. Within the X1 floor (17 % two-vblank frames for a trivial client) it is a
  tie, and a tie keeps QML for the existing surfaces (no rewrite cost).
- After the cutover (LumaTV's own compositor, which removes the X1 floor): QML Home's 6 of 533 drops are compositor-made, so what matters
  is render p50/p90 and RSS. Rust in the app's process adds a GL context and the glyph atlas [INFERENCE]; if its RSS exceeds QML's by more
  than the compositor-side saving, keep QML for Home and use Rust only for surfaces where QML shows its own floor.

Final text after the TV results.
