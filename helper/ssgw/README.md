# ssgw

A second-screen (SSAP) gateway for LG webOS TVs, wire-compatible with LG's `com.webos.service.secondscreen.gateway` on
port 3001. It reads LG's interface and permission files, serves clients it already knows, runs the apiadapter methods
those clients use itself, and relays first-time pairing and anything it does not implement to LG's own gateway on
port 3000. One thread around `poll`; no async runtime.

Two static binaries:

- `ssgw`, the gateway.
- `ssgw-accept`, run by xinetd for each connection on port 3001 in place of LG's `nsl`. It hands the connection to
  `ssgw`, or execs `nsl` with xinetd's arguments when `ssgw` is not there to take it. It never writes to its standard
  streams, which are the client's socket.

## Environment

`ssgw`:

- `SSGW_STATE_DIR` (default `/var/lib/lumatv/ssgw`): the client store, `clients.json`. It holds SHA-256 fingerprints
  of client keys, never the keys.
- `SSGW_RUN_DIR` (default `/tmp/lumatv-ssgw`): `accept.sock`, `control.sock` and toast icons. Created mode 0755; an
  existing directory must belong to the gateway's user or root and must not be writable by others.
- `SSGW_LG_ROOT` (default `/`), `SSGW_LG_HTTP` (default `127.0.0.1:3000`), `SSGW_LUNA_SEND` (default
  `/usr/bin/luna-send`): LG's files, LG's plain gateway, and the bus client.
- `SSGW_LOG`: `debug` adds per-message lines. Logs go to stderr as `<LEVEL> <message>`, LEVEL one of INFO, WARN,
  ERROR.
- `SSGW_BROKER_FD`: an inherited `SOCK_SEQPACKET` descriptor to the broker (below).

`ssgw-accept` reads `SSGW_RUN_DIR` and `SSGW_NSL` (default `/usr/bin/nsl`).

## Accept handshake

For port 3001 only, `ssgw-accept` connects to `accept.sock` after checking that the run directory is a real directory
not writable by others and that the listening peer is root or the directory's owner. The gateway writes `R` when it
can take a connection; `ssgw-accept` sends one byte, `T` (TLS) or `P` (plain), with the client's socket as
`SCM_RIGHTS`; the gateway answers `T` once it holds the socket. No `R` within 200 ms, a closed socket or any failure
before the descriptor is sent means `nsl` serves the client. Once the descriptor is sent, a missing `T` within 3 s
closes the connection rather than letting two servers answer it.

The gateway sends no `R` while it is stopping or already has 16 sessions and handoffs.

## Control socket

`control.sock` takes one line and answers one JSON line within a second:

- `ping`: `{"ok":true,"version":…,"uptimeS":…,"sessions":…,"recent":[…],"standby":…,"access":…}`. `recent` is the
  last 20 SSAP URIs, without payloads. `standby` is true while the TV refuses new connections (Active Standby);
  `access` is the TV's mobile device access setting.
- `forget`: drops every stored client, so each pairs through LG again. Answers `{"ok":true,"forgotten":N}`.

Anything else answers `{"ok":false,"error":"unknown command"}`.

## Broker

The netinput pointer sockets are root-only. With `SSGW_BROKER_FD` set, the gateway sends each socket path as one
datagram and reads one answer per request, in order: `K` with the opened descriptor attached, or `E<errno>`. Without a
broker it connects itself. A lost broker fails pending and later pointer connections with close code 1011.

## Exit status

0 after SIGTERM or SIGINT, once sessions are closed (at most 2 s). 1 when another gateway answers on the control
socket or a socket cannot be bound. 78 when the environment is unusable: LG's key or certificate cannot be had, or LG's
interface, permission or script files differ from the hashes pinned in `src/pins.rs`; retrying will not help. A panic
logs `ERROR panic: …` and exits 101. Anything but 0 is a crash.

## Toasts

`system.notifications/createToast` calls `com.webos.notification/createToast` with `sourceId`
`com.webos.service.secondscreen.gateway` (`adapter::TOAST_SOURCE_ID`) and `noaction: true`. LG's gateway uses a
random token per process. Icons sent inline are written under `$SSGW_RUN_DIR/icons`, which is cleared on start.

## Differences from LG's gateway

- JSON keys are written in sorted order.
- Active Standby closes new connections with 1008 "Try Again Later (EWS)"; per-message EWS errors are not produced,
  since those sessions no longer exist.
- A subscription whose bus call fails ends there; one-shot calls time out after 60 s.
- Unknown `/resources/` paths are relayed to LG; other plain GETs except `/` are closed, where LG never answers them.
- At most 1024 resource URLs are kept, oldest first. Socket URLs serve one connection. Paths with spaces are escaped,
  so they resolve (LG's 404).
- Pointer frames carry their length in bytes, not UTF-16 units.
- `_`-prefixed payload fields are stripped before bus calls.
- Bus subscribers get no register or disconnect events.
- Relayed clients reach LG's gateway from 127.0.0.1.
- Unmasked client frames are accepted.
- At most 16 sessions; beyond that LG's gateway serves new connections.
