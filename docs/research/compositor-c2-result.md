# Track C2: a 64-bit Rust compositor on a private socket

Design and first-light implementation of LumaTV's own compositor, built
beside LG's `surface-manager` (never replacing it in this step). Code lives
in `compositor/` (Cargo workspace `lumatv-compositor`). Everything here was
done offline on the workstation; TV runs are listed as ready scripts with
their expected results.

## Resume here

State (2026-10-25, start): design being written; no code yet.

- Done: read PLAN (Rules, Native helper, Plugins, Rendering stack, AI
  access, Phases/track C), HANDOFF "Resume here", shell-architecture.md
  (options, X2, X4), `arch/x2/*` (the import path that passed), the X4
  censuses, LumaShow `compositor`, `compositor-protocol`, `planes`,
  `displayd`, `display-mtk`, and `lumashow-lessons.md` sections A, B, C,
  F, G.
- Decisions taken (details below): one 64-bit glibc process
  (`aarch64-unknown-linux-gnu`, linked with `cargo zigbuild` against glibc
  2.36, run on the set through the private Debian loader like `x2srv64`);
  Wayland server from the `wayland-server` crate (libwayland-server C
  backend on the set so the Mali blob's `eglBindWaylandDisplayWL` serves
  `mali_buffer_sharing`; the pure-Rust backend for host tests); LG
  protocol code generated from XML by `wayland-scanner`; a 32-bit luna
  bridge is a separate later piece (a 64-bit process cannot load LG's
  32-bit `libluna-service2`).
- Half-done: nothing.
- Next command: `cd /data/home/lumaglass/compositor && cargo test` (host),
  `cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36`
  (set).
- TV state: not touched by this slice.

## Design
