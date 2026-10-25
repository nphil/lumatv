# Native helper (LumaTV phase 1)

`lumatv-helper` is a static aarch64 musl daemon that runs beside the compositor, in Rust, on LumaShow OS's crates. Phase 1
gives QML a loopback JSON API, a schema-checked settings store, saved-first weather and news, and the Denon receiver's
status, all idle while the screen is off. Source: `helper/`. TV-side status: built and tested on the host only; the TV
smoke test is written and waits for "TV is yours".

## Resume here

State at 2026-10-25 01:50 EDT. Everything in the assignment is committed and pushed (last commit "review fixes").

- Done: `helper/` builds (`./build.sh check` passes: rustfmt, clippy `-D warnings`, 243 host tests, release build); static
  AArch64 binary `helper/dist/lumatv-helper`, 1,415,744 bytes (1382 KiB), sha256 `e60762abcd8e8e0714c2c971bdf9cc6142250e2866431cefdc138f53d56b7375`.
  The suite ran ten times in a row with no failure (the first runs found four races between tests, all fixed).
  The daemon was run on this host end to end (detached start, status, stop, the real receiver, the real weather and news
  through `openssl` and through `curl`); `helper/smoke.sh` passed 34 checks on the host against a fake `luna-send`.
- NOT done, needs the TV: `python3 helper/smoke-tv.py` (about a minute, uploads to `/tmp`, removes it again). It settles what
  the host cannot: the set's `openssl` version and CA bundle, whether a `luna-send` subscription (`-n 1000000`, the form `tools/lumatv` uses) reaches a pipe, whether the
  `nobody` user can run the TLS child, the real power-state strings. Expected results are below.
- Not mine, to do in `tools/lumatv` and the ipk build: start/stop wiring (snippet below) and QML consuming the API.
- If the helper misbehaves on the set: `lumatv-helper stop --brand <app dir>/brand.json`, read `/var/lib/lumatv/log-helper`,
  set `"fetch": {"backend": "curl"}` or `"fetch": {"user": null}` in `/var/lib/lumatv/helper.json` for TLS trouble.
- To have the TV test run for you: message `NativeHelper` "TV is yours" (this agent wakes and runs `python3 helper/smoke-tv.py`),
  or run it yourself; it needs the set idle for about a minute and refuses nothing else.
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
  panel shows in Absolute mode); `volumeText` is what the panel shows for the receiver's display mode. `updatedAt` moves on
  every read without raising an event.
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

## Seams for later phases

- `service::Service::start` is the list of components; each takes a `ctx::Ctx` (config, hub, settings, stop). The MCP server
  (phase 3) and the plugin broker (phase 9) are two more components. A second listener reuses `http::Server` with its own
  handler and bind policy (the LAN one is the owner's to enable from the TV).
- Plugins register an owner with `SettingsStore::register(schema, None)`: the API then serves its page and values. They
  publish topics `plugin.<id>.<name>` on the hub and appear in `/v1/state` and the event stream unchanged.
- Home's weather and news widgets become first-party plugins by moving `homefeeds` behind the broker without changing the topics.

## Measured on the host (the TV is not touched)

- 243 tests pass (denon 27, feeds 69, app 147; one live test is `#[ignore]`d and was run by hand against the real services).
  Fixtures: real receiver replies (on and off), real Open-Meteo and zippopotam replies, real BBC and NYT feeds trimmed to 6 items.
- Release binary 1382 KiB (opt-level `s`, fat LTO, unwinding kept so a handler panic is a 500, not a dead helper). `file` is not
  installed on this host; `readelf` shows ELF64, type EXEC, machine AArch64, no `INTERP` header, no dynamic section (`build.sh` checks all four).
- Idle on the host with the screen on and the real receiver: RSS 4.7 to 5.6 MB, 8 threads, 10 descriptors, 0.2 % of a core
  (2 ticks in 10 s). With the screen off the receiver and feeds threads block on their channels; the screen watcher alone
  wakes (once a minute for its cross-check, plus `pollMs` if it has fallen back to polling).

## TV smoke test: `python3 helper/smoke-tv.py` (needs `helper/build.sh` first)

Uploads the binary and `smoke.sh` to `/tmp/lumatv-helper-smoke`, runs the helper detached on port 18741 with a temporary data
dir, and prints PASS/FAIL per check, FACT lines, and `VERDICT PASS|FAIL` (exit status 0 = pass; output saved to
`helper/dist/smoke-<time>.txt`). Phase A uses the real screen detection; if the panel is not on, Phase B repeats the receiver
and feed checks with `screen.source: "on"`. Expected: every check PASS, `screen source` `luna-subscribe` or `luna-poll`,
`the helper's screen state equals luna-send's`, weather and news fetched on the set, the receiver's volume equal to a direct
read, idle CPU under 5 % (expect well under 1 %), RSS under 20 MB (expect 5 to 10), no child process left after stop.
FACT lines to read: `openssl: ... OpenSSL x.y`, `luna-send -n 1000000 through a pipe delivers/delivered nothing` (and the same for `-i`), `fetchBackend`,
CA bundle, `nobody`. If the weather check fails, the `weather status` FACT carries the error code (`denied`/`untrusted` mean
set `fetch.user` to `null` or force `backend: curl`).

## Unknowns the TV settles

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
