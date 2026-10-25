# Native helper (LumaTV phase 1)

`lumatv-helper`: a static aarch64 musl binary that runs beside the compositor. Source in `helper/`.

## Resume here

State at 2026-10-25 01:40 EDT (updated at each milestone; newest facts first).

- Done, committed and pushed (last commit "programs resolved through PATH..."): everything in the assignment except the
  final write-up. `helper/` builds (`./build.sh`: static AArch64, 1.3 MB), 233 host tests pass, clippy and rustfmt clean,
  the daemon was exercised end to end on this host (start detached, status, stop, real receiver, real weather and news
  through `openssl` and through `curl`). `helper/smoke.sh` + `helper/smoke-tv.py` are written and dry-run on the host
  (34 checks PASS against a fake `luna-send`).
- Open: (1) fold in the findings of the read-only review (`NativeHelper.HelperReview`, if it has reported), (2) finish this
  note (API reference, tools/lumatv wiring, TV smoke expectations), (3) final `./build.sh check` and size.
- NOT done and needs the TV ("TV is yours"): `python3 helper/smoke-tv.py` (about a minute; uploads to `/tmp`, removes it
  again). Unknowns it settles: the set's `openssl` version and CA bundle, whether `luna-send -i` output reaches a pipe, whether
  `nobody` can run the TLS child, that QML's `XMLHttpRequest` sends no `Origin` header (QML side, not in the smoke test).
- Host commands: `cd helper && ./build.sh test` (clippy + tests), `./build.sh` (aarch64 release). `build.sh` creates the
  `cc` shim it needs in this container (no C compiler here).
- TV: not touched. The receiver was read from the host a few times (GET only).
- Git: shared working tree. Stage only `helper/` and this file; `git push -q forge main`; never `git pull`.

## Facts that shaped the design

- The compositor's QML has no `QtWebSockets` module (inventory `uiinv/qml`), so the push channel is an HTTP
  long poll through `XMLHttpRequest`. A WebSocket is not needed by any QML client.
- The receiver answers `HTTP/1.0 200 OK` from `mongoose2.5` with no `Content-Length`; the body ends at close.
  Status reply on 2026-10-25: `Power ON`, `InputFuncSelect TV`, `VolumeDisplay Absolute`, `MasterVolume -44.5`,
  `Mute off`. `formMainZone_MainZoneXml.xml` on port 8080 answers `403 Forbidden`; only the Lite status is used.
- LumaShow's `fetch` is HTTPS only and runs `openssl s_client` (`/usr/bin/openssl`, CA bundle path given by
  the caller). Plain HTTP to the receiver is a small client of its own.
