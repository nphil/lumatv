# Native helper (LumaTV phase 1)

`lumatv-helper` is a static aarch64 musl daemon that runs beside the compositor, in Rust, on LumaShow OS's crates. Phase 1
gives QML a loopback JSON API, a schema-checked settings store, saved-first weather and news, and the Denon receiver's
status, all idle while the screen is off. It also holds the AI-access (MCP) server and the plugin broker. Source: `helper/`.
TV-side status: the phase 1 and AI smoke tests ran on the set and passed (below).

## Resume here

State at 2026-10-25 03:20 EDT. The helper is built, committed and pushed in three slices; the first two are PROVEN ON THE SET.

1. Phase 1 (API, settings, weather and news, receiver): Main ran `helper/smoke-tv.py`: `VERDICT PASS`, 35 of 35.
2. AI access (the MCP server): I ran `helper/smoke-ai-tv.py --run --confirm-neutral-screen` on the set in my slot: `VERDICT PASS`,
   42 of 42 (2 skipped by design: `--actions` and `--receiver`, which touch the TV and the receiver). See "AI access" below.
3. Plugin broker (LumaShow plugin API level 1): built and tested on the host only (12 end-to-end tests through LumaShow's own
   SDK, 5 unit tests, no TV run yet). See "Plugin broker" below.

Offline recovery (2026-10-25): receiver heartbeats now reach displays even at unchanged volume. The QML OSD expires from the
receiver's timestamp, not message arrival. `./build.sh check` passes; the static AArch64 binary is 2,455,376 bytes, SHA-256
`b1e6f885e8fec7dec8e1ad98d134658315f766ba36ef7ca0b715ef8edb74c184`. This change has not been deployed to the TV.

- Host gate: `cd helper && ./build.sh check` passes (rustfmt, clippy `-D warnings`, 468 host tests, the aarch64 release build, 2,461,360 bytes, sha256 `9d774c7915ae3ce1099a80996c9701075be505a4144a93982ab681842465d17f`).
- Next, not mine: (a) QML for the AI prompt (`ai`, `ai.prompt` topics, `/v1/ai`) and for plugin cards, tiles, pages and toasts
  (topics below); (b) the settings flow that writes `plugins/enabled.json` behind the physical remote; (c) a TV smoke test of the
  broker (ask Main for a slot; plan under "Plugin broker"); (d) start/stop wiring in `tools/lumatv` (snippet below, unchanged).
- If the helper misbehaves on the set: `lumatv-helper stop --brand <app dir>/brand.json`, read `/var/lib/lumatv/log-helper`,
  set `"fetch": {"backend": "curl"}` or `"fetch": {"user": null}` in `/var/lib/lumatv/helper.json` for TLS trouble.
- Git: shared working tree; stage only `helper/` and this file; `git push -q forge main`; never `git pull`.

## What is in `helper/`

| Path | What |
|---|---|
| `app/` | package `lumatv-helper` (library + binary): `hub` (topics and events), `http` and `api` (the loopback server and v1 routes), `settings` (store on `settings-schema`), `screen`, `receiver`, `homefeeds`, `fetcher`, `service`, `cli`, `sys` (all `unsafe`), `schemas/*.json`; tests in `app/tests/` |
| `feeds/` | package `lumatv-feeds`: weather and news fetch, normalise, saved-first cache; no threads, no clock |
| `denon/` | package `lumatv-denon`: the Lite status parser and a plain-HTTP client |
| `build.sh` | `./build.sh` (aarch64 release), `test`, `check`, `clean` |
| `smoke.sh`, `smoke-tv.py` | the on-set smoke test and its uploader (same pattern as `gfx64/smoke-tv.py`) |

LumaShow crates used by path (`../../LumaShow/lumashow/crates`, never copied or edited): `fetch` (HTTPS GET through
`openssl s_client`, certificate and host name checked) and `settings-schema` (schema, values, validation). `service-api`,
`plugin-api`, `lumashow-sdk`, `ha-link` and `mdns` are not needed yet.

Toolchain: `helper/rust-toolchain.toml` pins LumaShow's nightly. This container has no C compiler, and the dependencies'
build scripts need a host linker, so `build.sh` creates a `cc` shim (`cargo-zigbuild zig cc`) under `target/`.

## Run it: how `tools/lumatv` starts and stops it

Every verb prints one JSON object and exits 0 (ok), 1 (failed), 2 (usage or config), or 3 (`status`: not running).

```sh
lumatv-helper start  --brand <app dir>/brand.json   # detaches, returns when the API listens
lumatv-helper stop   --brand <app dir>/brand.json   # SIGTERM, waits for the lock, SIGKILL after 8 s
lumatv-helper status --brand <app dir>/brand.json   # {"running":true,"pid":..,"port":..,"healthy":true}
lumatv-helper check-config --brand ...              # what the set has: luna-send, openssl, curl, CA bundle, the chosen backend
```

- `start` is idempotent (`"already":true` when running) and safe from SSH or a boot hook: the child is its own session, all
  inherited descriptors are closed (a captured stdout reaches end of file, a shell's lock fd does not stay held), SIGHUP is
  ignored. A single instance is enforced by `flock` on `/var/lib/lumatv/helper.lock`, which holds the pid and port.
- Files, all under the brand's data dir: `log-helper` (+ `log-helper.1`, 256 KiB each, UTC stamps), `helper.lock`,
  `helper.json` (optional process config), `cache/weather.json`, `cache/news.json`, `settings/<owner>.json`.
  `--brand` reads `dataDir` by `tools/brand.sh`'s rule (directly under `/var/lib`).
- Port: 127.0.0.1:18731 (`helper.json` `listen.port`). A non-loopback `listen.host` is refused at config load.
- Start it after `/var/lib/lumatv` is writable (about 13 s into boot): in `autostart.sh --worker` after `boot-bind`, and at
  the end of `apply`. Stop it in `revert`.
- An ipk update cannot overwrite a running executable (text busy), so stage a copy and swap it:

```sh
HELPER_SRC="$APPDIR/payload/helper/lumatv-helper"     # build-ipk.sh copies helper/dist/lumatv-helper here, mode 0755
HELPER="$STATE/bin/lumatv-helper"
helper_start() {
  [ -x "$HELPER_SRC" ] || return 1
  if ! cmp -s "$HELPER_SRC" "$HELPER"; then
    [ -x "$HELPER" ] && "$HELPER" stop --brand "$APPDIR/brand.json" >/dev/null 2>&1
    mkdir -p "$STATE/bin" && cp -f "$HELPER_SRC" "$HELPER.new" && chmod 755 "$HELPER.new" && mv -f "$HELPER.new" "$HELPER" || return 1
  fi
  "$HELPER" start --brand "$APPDIR/brand.json"
}
helper_stop() { [ -x "$HELPER" ] && "$HELPER" stop --brand "$APPDIR/brand.json"; }
```

## `helper.json` (optional, how the process runs)

All keys optional; an unknown key or a bad value stops `start` with the reason (a typo must not be ignored). People's choices
(ZIP, news source, the receiver's address) are settings, not here.

| Key | Default | Meaning |
|---|---|---|
| `listen.host`, `listen.port` | `127.0.0.1`, 18731 | loopback only; 0 asks for a free port (tests) |
| `log.level`, `log.maxBytes` | `info`, 262144 | `error`, `warn`, `info`, `debug`; size at which `log-helper` becomes `log-helper.1` |
| `screen.source`, `screen.pollMs` | `luna`, 5000 | `on` treats the screen as always on (development); the poll period if subscribing fails |
| `receiver.pollMs`, `fastPollMs`, `burstMs`, `maxBackoffMs`, `hideAfterMs` | 2000, 250, 3000, 30000, 6000 | polling rhythm; `connectTimeoutMs` 1500 and `requestTimeoutMs` 3000 also exist |
| `fetch.backend`, `fetch.openssl`, `fetch.curl`, `fetch.caBundle`, `fetch.user` | `auto`, `/usr/bin/openssl`, `curl`, first existing of the usual paths, `nobody` | HTTPS transport; `user: null` runs the TLS child as the helper |
| `luna.send` | `luna-send` | found in `PATH` and then `/usr/bin`, `/usr/sbin`, `/bin`, `/sbin` |
| `nice` | 10 | scheduling priority given up |
| `allowedOrigins` | `[]` | browser origins allowed to call the API (a web app would need `"file://"`) |

## The API (v1)

JSON over HTTP on `127.0.0.1:18731`; QML uses `XMLHttpRequest` (async only, `JSON.parse(xhr.responseText)`, `status 0` = the
helper is down: retry in a couple of seconds). Every answer is `{"ok":true,"api":1,"now":<ms>,...}` or
`{"ok":false,"api":1,"error":{"code","message"}}`. `GET /v1` lists the endpoints. A breaking change is `/v2`.

| Request | Answer |
|---|---|
| `GET /v1/health` | liveness, `boot` |
| `GET /v1/state` | `boot`, `seq`, `helper` (version, pid, rssKb, threads, fetch backend), `topics` (all of them, one read) |
| `GET /v1/state/{topic}`, `/v1/screen`, `/v1/receiver`, `/v1/weather`, `/v1/news` | one topic with its `seq` |
| `GET /v1/events?after=&boot=&timeout=&topics=` | long poll, up to 30 s: `{"seq","reset","events":[{"seq","at","topic","kind","data"}]}` |
| `POST /v1/receiver/nudge` | read the receiver now and fast for 3 s (send it when a volume key is pressed) |
| `POST /v1/refresh` | fetch weather and news now, even with the screen off |
| `GET /v1/settings`, `GET /v1/settings/{owner}`, `POST /v1/settings/{owner}` | the settings layer |

Client loop: one `GET /v1/state` (it carries `seq` and `boot`; topics appear as components publish, so tolerate missing
ones), then `GET /v1/events?after=<seq>&boot=<boot>&timeout=25` over and over with the `seq` of the last answer. `reset: true`
means the client fell behind the 256-event ring or the helper restarted: read the state again. Setting a topic to what it
already holds raises no event.

Writes need `X-Luma-Client: <name>` and `Content-Type: application/json`.

```js
function helper(method, path, body, done) {                 // QML, Qt 5.12
    var x = new XMLHttpRequest()
    x.onreadystatechange = function() {
        if (x.readyState !== 4) return
        done(x.status === 200 ? JSON.parse(x.responseText) : null)
    }
    x.open(method, "http://127.0.0.1:18731" + path)
    if (method === "POST") { x.setRequestHeader("X-Luma-Client", "home"); x.setRequestHeader("Content-Type", "application/json") }
    x.send(body === undefined ? null : JSON.stringify(body))
}
```

Topic documents (compact JSON, keys sorted):

- `screen`: `{"on","state","since","source","error"}`. `state` is `tvpower`'s (`Active`, `Active Standby`, `Screen Saver`...).
  `Active` and `Screen Saver` are on. `source`: `luna-subscribe`, `luna-poll`, `config`, `assumed`.
- `receiver`: `{"enabled","host","polling","reachable","fresh","updatedAt","error","power","input","volumeDb","volumeAbs","volumeDisplay","volumeText","mute"}`.
  The six values are present only while `fresh` (read within 6 s while polling); otherwise `null`. `power` is `on`, `standby`
  or `unknown`; `input` is the receiver's code (`TV`); `volumeDb` is `MasterVolume`; `volumeAbs` is dB + 80 (the number the
  panel shows in Absolute mode); `volumeText` is what the panel shows for the receiver's display mode. Each successful read
  publishes its new `updatedAt`, even if the volume is unchanged, so displays can expire readings when the helper disappears.
- `weather`, `news`: `{"doc": <document|null>, "status": {"failures","error","lastAttemptAt"}}`, decorated by the API with
  `state` (`fresh`, `stale`, `expired`, `none`) and `ageMs`; an expired document is `null`. Events carry the stored value.

## Weather and news

The document is also the file in `cache/` (byte for byte), so QML can read `file:///var/lib/lumatv/cache/weather.json`
before the helper is up. Fields: `fetchedAt`, `staleAt`, `expiresAt` (ms), then for weather `zip`, `units`, `place`, `lat`,
`lon`, `current`, `daily` (7 days) and `display`; for news `feeds` (the cache key) and `items`.

- `weather.doc.display` is exactly what `WeatherWidget.qml` builds: `place`, `temp` ("54°"), `cond`, `stats` ("Feels like
  50° · 72% humidity · 8 mph"), `nowKind` (icon kind), `days[{day, kind, hi, lo}]` ("Today", "Mon", ...). The card trims to
  its `maxDays`. Ported with JavaScript semantics (`Math.round` on negatives, the `kindFor`/`textFor` order).
- `news.doc.items[{tag, src, title, summary, link, img, pub, pubMs}]` is `parseFeed` + `merge` of `NewsWidget.qml` (4 per
  feed, round robin, duplicates by lower-cased title; the NYT feeds are the fallback when the chosen source gives nothing). `img` is
  the feed's URL; the card still rewrites `/standard/NNN/` to its own width. `pubMs` saves `Date.parse`.
- Policy: the saved documents are on the hub within a moment of start, before the network and whatever the screen does. Fetch
  only while the screen is on, 15 minutes after the last fetch (news: `refresh_minutes`, 5 to 180), after failures at 2,
  10, then every 30 minutes, the saved reading served meanwhile. Not shown after 6 hours. A reading stamped in the future
  (the clock was wrong when it was saved, or is now: the set's clock may be unset at a cold start) is shown as stale and
  fetched again at once, never dropped. Another ZIP or news source drops the old document at once. The ZIP is geocoded once
  (zippopotam) and the place kept in the document; later refreshes ask Open-Meteo only.
- Transport: LumaShow's `fetch` when `openssl version` is 3 or later (it passes `-no-CAstore`, unknown to 1.1.1); otherwise
  the set's `curl` (HTTPS only, TLS 1.2+, no redirects, hard deadline and size cap). `helper.json` `fetch.backend`
  (`auto`, `openssl`, `curl`) forces one. The TLS child runs as `nobody` when the helper is root (`fetch.user`); if the system refuses that (`Denied`: the user cannot run the program, read the bundle or open a socket) it runs as the helper's user from then on, with one warning in the log.
- QML change for the main agent: drop `fetchWeather`/`fetchNews` and their timers; draw `topics.weather.doc.display` and
  `topics.news.doc.items` from `/v1/state` at once, update on events. The card no longer needs `live` for fetching.

## Receiver

`GET /goform/formMainZone_MainZoneXmlStatusLite.xml` on 192.168.1.93:8080 (read on 2026-10-25): HTTP/1.0 from `mongoose2.5`,
no `Content-Length`; `Power` `ON` or `OFF` (the S760H writes OFF in standby and keeps reporting its last input and volume),
`InputFuncSelect`, `VolumeDisplay` `Absolute`, `MasterVolume` -44.5 (dB), `Mute`. The full status page answers 403 on this
port. Polled every 2 s while the screen is on and the receiver is enabled; a nudge reads at once and every 250 ms for 3 s;
failures back off 4, 8, 16 s to a cap of 30 s; a reading is hidden 6 s after it was last confirmed. No telnet yet (phase 4).
`volumeText` for Relative mode ("-44.5 dB") and the fractional rule for Absolute are inferred from the panel, not measured.

## Settings

Three owners, JSON schemas in `app/src/schemas/` in LumaShow's format 1, the pages the TV's Settings will draw:

| Owner | Fields (key: kind, default) |
|---|---|
| `base.weather` | `zip`: text, 30311 (five digits, checked by the owner's rule); `units`: choice fahrenheit/celsius |
| `base.news` | `edition`: choice bbc/nyt, bbc; `refresh_minutes`: integer 5 to 180 step 5, 15 |
| `base.receiver` | `enabled`: bool, true; `host`: text, 192.168.1.93 (letters, digits, dots, dashes); `port`: integer, 8080 |

`POST /v1/settings/{owner}` takes `{"changes":[{"key","value":{"type","value"}}],"ifVersion":n}` and is all-or-nothing: the
schema (type, range, step, listed choices, predicates, no duplicates), then the owner's rule, then an atomic write and a
`settings` event; a refusal is 422 with `fields`, a stale `ifVersion` is 409. A stored value the schema no longer accepts is
reported (`invalid`), kept in the file and replaced by the default in `effective`. Values live in `settings/<owner>.json`.
The theme file's `weather` and `news` sections are no longer read by anything once QML uses the helper; to keep a
customised ZIP or units, post it once.

## Safety (Principle 0)

- The listener is loopback only: the config refuses another address, the server refuses another peer. No AI-facing surface exists.
- Refused before any handler: a `Host` that is not this listener (`127.0.0.1:port` or `localhost:port`); any `Origin` not in
  `allowedOrigins` (empty by default; QML and curl send none, a web page always does); a request that carries Fetch
  Metadata from a page (`Sec-Fetch-Site` other than `none`); a write without `X-Luma-Client` or without a JSON body type.
  The first refusal of each kind is logged. Limits: 16 KiB of headers, 64 KiB of body, 32 connections, 10 s to send a request.
- The one assumption that only QML can confirm: `XMLHttpRequest` in the compositor sends no `Origin` and no
  `Sec-Fetch-Site`. If it does, the answer is `403 origin_not_allowed` and the log says so; `allowedOrigins` is the lever.
- No keys or tokens in the repository. The helper writes only under its data dir. It runs at `nice 10`, one thread per
  component, 8 threads in all; the TLS child holds no privilege.

## AI access (the MCP server): `helper/app/src/ai/`

An MCP server (Streamable HTTP, JSON answers, protocol 2025-06-18 and two older) for assistants on the home LAN, built so that no
AI, script or API of the helper can give itself access (Principle 0). Off by default: no state file means no listener and no socket.

- **Only the remote turns it on.** Turning the listener on, opening pairing, giving an assistant more, resuming a paused one and
  raising the volume limit each open a prompt on the TV (`ai.prompt` topic) and complete only when the helper itself sees a key press
  from a physical remote's input device (`LGE RCU`, `LGE M-RCU*`). Keys from `Smart Remote RCU Input` and `LGE Network Input` (where
  Home Assistant's `webostv.button`, SSAP and the module's own media keys arrive) never count. A press must start after the prompt
  has been up 300 ms; OK approves, Back denies and pauses the asking assistant. `GateApproval` has no constructor outside the gate,
  so every widening function needs one by type. The loopback `/v1/ai` routes can ask (202) and narrow (done at once: off, pause,
  revoke, lower a level, lower the volume limit, lock-out); no route answers a prompt or makes a token.
- **LAN door.** Binds the TV's private address (the one the kernel uses for the LAN, never `0.0.0.0`), port 18732 (`ai.port`),
  only while on; source networks limited to private ranges; `POST /pair` (one-time code, shown only after the physical OK) gives a
  bearer token `ltv_...`; `POST /mcp` needs it; 8 failures from one address in 60 s answer 429.
- **13 typed tools**, no shell, files, luna, network or settings tool: `tv_get_power`, `tv_get_foreground_app`, `tv_list_inputs`,
  `tv_list_apps`, `tv_get_now_playing`, `receiver_get_status` (group `state`, default always); `receiver_set_volume`,
  `receiver_change_volume`, `receiver_set_mute`, `tv_media_key` (five playback keys only) (`playback`, always); `tv_launch_app`,
  `tv_switch_input` (`launch`, ask); `tv_standby` (`power`, ask). The LumaTV app itself cannot be opened by an assistant.
  Levels per group are `never`, `ask`, `always`; each assistant's levels are the protected settings owner `ai.<id>`.
- **Every call is audited**; the pipeline is validate, pause, level, prompt, act, audit.
- Config (`helper.json` `ai`): `port`, `bind`, `allowedNets`, `physicalDevices`, `networkDevices`, `approveCodes` (28, 352),
  `denyCodes` (412, 158, 174), `promptSeconds` (30), `inputDir`, `sysInputDir`, `injectDevice`, `mediaKeys`. Nothing there switches it on.

### AI smoke test: `python3 helper/smoke-ai-tv.py --run --confirm-neutral-screen`. Ran on the set: PASS 42/42

Uploads the binary and `smoke-ai.py` to `/tmp/lumatv-ai-smoke`, runs a separate helper there (loopback 18741, AI port 18743, its own
data dir; the production helper and its files are not touched), prints PASS/FAIL/FACT/SKIP lines and `VERDICT`, saves the output to
`helper/dist/smoke-ai-<time>.txt`, removes everything. It presses OK and Back through the network input devices, so leave the TV on
Home with no dialog; flags: `--actions` (opens the app in front, presses Pause), `--receiver` (re-sends the receiver's current mute and
volume), `--lan-check` (the host reaches the listener), `--human` (asks for one real remote OK), `--probe-keys N` (prints raw key events).

What the run on the set proved (2026-10-25, 192.168.1.242, aarch64, Python 3.10):

| Claim | Check that passed |
|---|---|
| Not bound by default | state says off, unbound, nobody paired; nothing in `/proc/net/tcp` on the AI port |
| No helper API enables it | `POST /v1/ai/enable`, `PUT /v1/ai/listener`, `POST /v1/settings/ai`, `GET /v1/ai/listener`, `DELETE /v1/ai`, `POST /v1/ai` with `{"listener":{"enabled":true}}`: the state stays off and the port stays closed |
| Asking opens a prompt, nothing more | `POST /v1/ai/listener` answers 202 and `ai.prompt` holds `enable_listener` |
| Network keys never count | OK and Back pressed on both `Smart Remote RCU Input` and `LGE Network Input`: the prompt is still open, listener still off, port closed |
| Lock-out | withdraws the prompt; later: the port closes and refuses connections, no assistant stays paired, the switch is off |
| Enabling, pairing, matrix, once on (test-only path: `LGE Network Input` is declared the physical remote in the test's own `helper.json`) | the stand-in OK turns it on and it binds `192.168.1.242:18743`, not `0.0.0.0`; the pairing code appears only after the physical OK (not before, and not after a network OK from `Smart Remote RCU Input`); the code gives a token; a wrong token and no token are both 401 |
| MCP and levels | `initialize` answers, 13 tools listed, the six read tools answer with real TV and receiver data |
| Ask flow | `tv_standby` opens a prompt naming the assistant; a network OK does not approve it; Back refuses it ("denied"), the TV stays on, the assistant is paused, a paused assistant is refused even for reads, resuming needs a physical OK and works |

The stand-in is only in the test's config: production `physicalDevices` stay `LGE RCU` and `LGE M-RCU*`. Facts the run printed: the
set has `LGE RCU`, `LGE M-RCU - Builtin [0..3,5]`, `LGE W-RCU`, `LGE TONE+`, `LGE Simple Premium`, `Smart Remote RCU Input`,
`LGE Network Input`; the key presses of the run opened the Apps app once (OK on Home), so run it only with Main's slot and relaunch
`com.webos.app.home` after. Not run on the set: `--actions` (real playback key reaching `Smart Remote RCU Input`), `--receiver`,
`--lan-check` (needs the host on the LAN: `192.168.1.242` is reachable from the host) and `--human`.

Review fixes made to the smoke scripts at this restart: `smoke-ai-tv.py` built its remote command with a broken quote (syntax error),
now `shlex.quote`; the remote script ran in the foreground of the ssh call, so the call could not return until the test ended
(it is now detached); a timeout killed only a pid file, now `pkill -f` on the test's own path. A host dry run (`smoke-ai.py
--fake-input`, FIFOs for the devices) passed 42/42 before the set run.

## Plugin broker: `helper/app/src/plugins/`

LumaShow's plugin API level 1 (`/data/home/LumaShow/lumashow/docs/plugin-api.md`) on the TV, using LumaShow's `plugin-api` crate by
path (framing, manifest, wire messages, conversions into `service-api` records through their own `validate()`); its `lumashow-sdk`
is a dev-dependency, so the tests speak to the broker as a real plugin does. The release binary grew from 1,739,568 to 2,449,968 bytes (1698 to 2392 KiB), sha256 `a27f0220b80baa50536a443d6af12896d9711df4efbee9be795d0eb4e17bfb37`.

| Part | What |
|---|---|
| `roster.rs` | which plugins are on: `<data>/plugins/enabled.json` `{"plugins":[{"id","uid","approve"}]}`; manifests at `<data>/plugins/<id>/plugin.json`. `approve` is the SHA-256 of the manifest's consent document: a changed manifest stays inert. Inert with reason: bad id, duplicate, uid outside `firstUid`..+63, manifest missing or for another id, level too new, permissions "later" at level 1 (`net.internet`...), permissions or leases this TV does not serve (`net.*`, `ha.*`, `ui.surface`, `signal.*`, any `lease:`), not approved |
| `session.rs` | one plugin's conversation with no socket: hello and welcome (level negotiation), register through `Plan::new` and `Context`, cards, tiles, notifications, facts, settings page and values, actions, expiry; publishes topics |
| `broker.rs` | the unix socket, `SO_PEERCRED` identity, one reader and one writer thread per connection, 64 KiB frames, 100 messages a second (burst 200, 50 strikes then hang-up), 256 frames or 1 MiB queued at most, handshake time, descriptors closed unread |
| `routes.rs` | `/v1/plugins`: list; `POST /v1/plugins/rescan`; `POST /v1/plugins/{id}/actions/{action}` `{"target"?}` (202 `{"op"}`) |
| `mod.rs` | `Plugins`: a manager thread reads the roster every `plugins.rescanSeconds` (30) and keeps the broker and its socket only while a plugin is on |

- **Nothing the helper does switches a plugin on.** The roster file is only read; no route writes it, installs a plugin or changes
  its permissions (Principle 0: installing software on the TV is the owner's act). The future settings flow writes it behind the
  physical remote and then calls `POST /v1/plugins/rescan`. A damaged file switches nothing on and says why in the `plugins` topic.
- **Identity is the connection**: the kernel's user id for the peer, looked up in the roster (`plugins.firstUid` + n, default 20000
  as in LumaShow; only tests change it). An unknown user makes the file be read again once a second at most; still unknown gets the
  fatal `not_a_plugin` (the SDK reads it as "not yet"). A second connection of a plugin closes the first.
- **Socket**: `/run/lumashow-plugins/broker.sock` (LumaShow's path: plugins find it nowhere else; `plugins.socket` changes it for
  tests), directory 0750 and socket 0660, given to group 2000 when the helper is root. `/run` is tmpfs, not the rootfs; the socket
  exists only while a plugin is on. Unconfirmed on the set: that `/run` is writable there and that plugin users 20000+ can be made.
- **Served at level 1**: settings page and values (registered in the settings store under the plugin's id, so `/v1/settings/<id>`
  serves it; a change reaches the plugin as `settings.apply` `user`, and after registering it gets `reconcile` with the chosen values;
  a rejection is a `settings_rejected` event, not a rollback), cards (at most 8; `ringing` refused), tiles (16), notifications (20;
  `ongoing` refused, channels at most `high`), facts (16 lines of 160), declared actions with a 10 s answer time, `bye`.
  Expired cards and notifications go on the clock. Everything of a plugin goes in one step when it disconnects, except its stored
  setting values, which stay for next time.
- **Refused in words (`not_served`)**: timeline sources, choice providers, secret fields, `lease.*` fields, images, surfaces, Home
  Assistant, network and Bluetooth brokering, signals. `welcome.features` lists only `settings:1`, `cards:1`, `tiles:1`, `notify:1`
  and `facts:1`, and `max_assets` is 0.
- **Topics** (what QML draws): `plugins` `{plugins:[{id,name,version,class,state:connected|waiting|inert,registered,reason?}],problem}`;
  `plugin.<id>.cards` `{items:[{id,revision,createdAt,expiresAt,card}]}` (`card` is the plugin's `card.put`, already validated);
  `plugin.<id>.tiles` `{items:[<tile>]}` (a tap is `POST /v1/plugins/<id>/actions/<tile.action>`); `plugin.<id>.notifications`
  `{items:[{id,channel,revision,receivedAt,expiresAt,post}]}` plus events `posted` and `ended` (toasts); `plugin.<id>.facts`
  `{facts:{key:line}}`; `plugin.<id>` events `settings_rejected`, `settings_restart_required` and `action_result`
  `{op,action,outcome:{type:ok|failed,reason?}}`. Times are Unix seconds.
- **Weaker than the Show** (stated in the module docs): the set's kernel has no user or network namespaces and no seccomp, so a
  plugin is only a separate user. Only reviewed plugins belong in `enabled.json`.

Tests (host, no TV): `app/tests/plugins.rs`, 12 tests over a real socket with the SDK: off means no socket; registration and every
topic; settings page, reconcile, change, rejection, unknown txn, page retired on disconnect and the values kept; actions answered and
unanswered; refusals (ringing, ongoing, undeclared view and channel, secret, lease, source, urgent channel, card cap, `ha.watch`,
`net.listen`, `signal.put`, anything before `register`); a user that is not a plugin and a changed manifest; a damaged
`enabled.json`; replacement by a second connection; the end of a connection; switching off (`shutdown` "disabled", socket gone);
the helper stopping (`shutdown` "stopping"); bad frames, unknown types, an oversized frame and a flood. Plus unit tests of the roster
(six kinds of inert, the approval hash) and of expiry on a fake clock. Fixed while writing them: closing a socket with the client's
unread `hello` in it reset the connection and lost the `not_a_plugin` reason, so a connection now ends with a half close and a short
drain.

TV smoke plan for the broker (needs a slot; nothing is installed on the set by it): with a temporary data dir and `plugins.socket`
under `/tmp`, a plugin that is the root user's own is not possible, so use `firstUid` set to a uid the test plugin runs as (for
example `su -s` `nobody`, if the set has it) and check: no socket with no `enabled.json`; connect, register, topics; `/run` writable.
Not done yet.

## Preload policy: `helper/app/src/preload.rs`

Requested by SurfacesP2b. webOS 10 forgets the preload policy at every switch-on, so at start with the screen on and at each
screen off-to-on change the helper reads `<data>/preload.json`, `{"off":["<app id>", ...]}` (written by `lumatv preload on`, deleted by
`off`; missing means do nothing), and runs `luna-send -n 1 luna://com.webos.service.preloadmanager/setPreloadPolicy
'{"application":{"id":"<id>","isEnabled":false}}'` for each id, 5 s each, a failure only logged, no topic. The file is read strictly (at most
32 ids of letters, digits and `. _ -`, at most 16 KiB, no other keys); a bad file does nothing and logs why. The payload is built with
`serde_json`, never by pasting the id into text. One idle thread (`preload`) that waits on the `screen` events. Tests: 5 unit tests
(list parsing, calls, a failure in the middle, a hung `luna-send` killed at the deadline, the off-to-on rule on a fake hub) and
`app/tests/preload.rs` (the whole service, a fake `luna-send`: told at start, again after off and on, nothing after the file is removed).
Not run on the set.

## Seams for later phases

- `service::Service::start` is the list of components; each takes a `ctx::Ctx` (config, hub, settings, stop). The MCP server and
  the plugin broker are now two of them (`ai::Ai`, `plugins::Plugins`).
- Home's weather and news widgets become first-party plugins by moving `homefeeds` behind the broker without changing the topics.
  A first-party plugin would have to be a separate program on the socket (the broker has no in-process plugin); until then
  `homefeeds` stays a component and its topics `weather` and `news` are unchanged.

## Measured on the host (the TV is not touched)

- 468 tests pass now (243 at phase 1, before the AI module and the plugin broker; one live feeds test is `#[ignore]`d and was run by hand against the real services).
  Fixtures: real receiver replies (on and off), real Open-Meteo and zippopotam replies, real BBC and NYT feeds trimmed to 6 items.
- Release binary at phase 1: 1382 KiB (opt-level `s`, fat LTO, unwinding kept so a handler panic is a 500, not a dead helper). `file` is not
  installed on this host; `readelf` shows ELF64, type EXEC, machine AArch64, no `INTERP` header, no dynamic section (`build.sh` checks all four).
- Idle on the host with the screen on and the real receiver: RSS 4.7 to 5.6 MB, 8 threads, 10 descriptors, 0.2 % of a core
  (2 ticks in 10 s). With the screen off the receiver and feeds threads block on their channels; the screen watcher alone
  wakes (once a minute for its cross-check, plus `pollMs` if it has fallen back to polling).

## TV smoke test: `python3 helper/smoke-tv.py` (needs `helper/build.sh` first). Ran on the set: PASS 35/35

Uploads the binary and `smoke.sh` to `/tmp/lumatv-helper-smoke`, runs the helper detached on port 18741 with a temporary data
dir, and prints PASS/FAIL per check, FACT lines, and `VERDICT PASS|FAIL` (exit status 0 = pass; output saved to
`helper/dist/smoke-<time>.txt`). Phase A uses the real screen detection; if the panel is not on, Phase B repeats the receiver
and feed checks with `screen.source: "on"`. Expected: every check PASS, `screen source` `luna-subscribe` or `luna-poll`,
`the helper's screen state equals luna-send's`, weather and news fetched on the set, the receiver's volume equal to a direct
read, idle CPU under 5 % (expect well under 1 %), RSS under 20 MB (expect 5 to 10), no child process left after stop.
FACT lines to read: `openssl: ... OpenSSL x.y`, `luna-send -n 1000000 through a pipe delivers/delivered nothing` (and the same for `-i`), `fetchBackend`,
CA bundle, `nobody`. If the weather check fails, the `weather status` FACT carries the error code (`denied`/`untrusted` mean
set `fetch.user` to `null` or force `backend: curl`).

## Unknowns the TV settled (see Main's run output for the values; the helper worked with whatever it found)

- `openssl` version and bundle on the set (decides openssl or curl; either works by design), and whether `curl` trusts the bundle.
- Whether a `luna-send` subscription flushes through a pipe (if not, the watcher polls every 5 s: one process spawn per poll).
- The state strings `getPowerState` really returns beyond `Active` (the helper treats `Active` and `Screen Saver` as on).
- That QML's XHR is accepted (see Safety), and what it costs on the compositor thread (parsing a 6 KB state document).

## For the main agent to fold in

- PLAN.md "Native helper": built in phase 1 as this note describes; the Status line "Next build step ... the Rust helper"
  becomes "helper phase 1 built; wire it in tools/lumatv and move Home's weather and news to it". HANDOFF.md "Resume here":
  add the helper, its smoke test and the files under `/var/lib/lumatv`.
- CHANGELOG, Unreleased: "The native helper (`helper/`): a static aarch64 daemon with a loopback JSON API for QML, a
  schema-checked settings store, saved-first weather and news, and the Denon receiver's volume, mute and input, idle while the
  screen is off."
- `tools/build-ipk.sh` copies `helper/dist/lumatv-helper` to `payload/helper/`; `.gitignore` already ignores `dist/`.
