# Native helper (LumaTV phase 1)

`lumatv-helper`: a static aarch64 musl binary that runs beside the compositor. Source in `helper/`.

## Resume here

State at 2026-10-25 00:35 EDT (updated at each milestone; newest facts first).

- Done: workspace skeleton in `helper/` (crates `app` = `lumatv-helper`, `feeds`, `denon`), pinned toolchain,
  aarch64 musl cross build proven with the skeleton (ELF64 AArch64, no INTERP, 331 KB).
- Next: (1) `feeds` crate (weather, news, cache), (2) `denon` crate, (3) `app` crate: config, log, lock,
  hub (state + events), HTTP server, API, settings store, screen source, runners, (4) `helper/build.sh`,
  `helper/smoke-tv.sh`, (5) tests, size, result note.
- Host build needs a C linker for crates' build scripts and this container has no `cc`; a shim that
  runs `cargo-zigbuild zig cc -- "$@"` as `cc` on `PATH` is enough (build.sh creates it).
  `PATH=/tmp/zigcc:$PATH cargo check --workspace` works with the shim in `/tmp/zigcc/cc`.
- TV: not touched (busy with another agent). Fixtures came from the public feeds and one read-only GET to the
  receiver (`/goform/formMainZone_MainZoneXmlStatusLite.xml`, port 8080).
- Git: shared working tree with other agents. Stage only `helper/` and this file; `git push -q forge main`.

## Facts that shaped the design

- The compositor's QML has no `QtWebSockets` module (inventory `uiinv/qml`), so the push channel is an HTTP
  long poll through `XMLHttpRequest`. A WebSocket is not needed by any QML client.
- The receiver answers `HTTP/1.0 200 OK` from `mongoose2.5` with no `Content-Length`; the body ends at close.
  Status reply on 2026-10-25: `Power ON`, `InputFuncSelect TV`, `VolumeDisplay Absolute`, `MasterVolume -44.5`,
  `Mute off`. `formMainZone_MainZoneXml.xml` on port 8080 answers `403 Forbidden`; only the Lite status is used.
- LumaShow's `fetch` is HTTPS only and runs `openssl s_client` (`/usr/bin/openssl`, CA bundle path given by
  the caller). Plain HTTP to the receiver is a small client of its own.
