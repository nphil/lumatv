# Native helper (LumaTV phase 1)

`lumatv-helper`: a static aarch64 musl binary that runs beside the compositor. Source in `helper/`.

## Resume here

State at 2026-10-25 01:10 EDT (updated at each milestone; newest facts first).

- Done and tested on the host: `helper/denon` (status XML parser, plain-HTTP client, tests against a fake receiver, one
  live read of the real receiver) and `helper/feeds` (weather, news, saved-first cache; 69 tests pass). Both are committed.
- Written, not yet compiled together: `helper/app` modules `timeutil`, `log`, `config`, `control`, `hub`, `http`,
  `settings` (+ `schemas/*.json`), `sys`, `ctx`, `screen`, `receiver`. Still to write: `homefeeds` (the runner that uses
  `lumatv-feeds`), `api`, `service`, `lib.rs` wiring, `main.rs` CLI (`run|start|stop|status|version|check-config`).
- Then: IPC tests (`helper/app/tests/`), `helper/build.sh`, `helper/smoke-tv.sh`, size, this note's API reference.
- Host commands: `cd helper && PATH=/tmp/zigcc:$PATH cargo test --workspace` (the shim `/tmp/zigcc/cc` is
  `exec /data/home/.local/bin/cargo-zigbuild zig cc -- "$@"`; this container has no `cc`; `build.sh` will create it).
- TV: not touched. The receiver was read twice from the host (GET only).
- Git: shared working tree. Stage only `helper/` and this file; `git push -q forge main`; never `git pull`.

## Facts that shaped the design

- The compositor's QML has no `QtWebSockets` module (inventory `uiinv/qml`), so the push channel is an HTTP
  long poll through `XMLHttpRequest`. A WebSocket is not needed by any QML client.
- The receiver answers `HTTP/1.0 200 OK` from `mongoose2.5` with no `Content-Length`; the body ends at close.
  Status reply on 2026-10-25: `Power ON`, `InputFuncSelect TV`, `VolumeDisplay Absolute`, `MasterVolume -44.5`,
  `Mute off`. `formMainZone_MainZoneXml.xml` on port 8080 answers `403 Forbidden`; only the Lite status is used.
- LumaShow's `fetch` is HTTPS only and runs `openssl s_client` (`/usr/bin/openssl`, CA bundle path given by
  the caller). Plain HTTP to the receiver is a small client of its own.
