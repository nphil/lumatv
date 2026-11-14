# Replacing LG's second-screen gateway (`ss.gateway`) with Rust

Research and plan, 2026-11-13. Nothing here is built or installed. TV facts were read on the set (OLED65C4PUA, webOS 10.2.1,
32-bit ARM userland on an aarch64 kernel) without changing it, or read from its firmware files; inferences are marked as such.

## Verdict

- A Rust gateway can take over without writing firmware: LG's xinetd entry for port 3001 is bind-mounted so that each connection
  starts a small accept program, which hands the connection to the Rust gateway, or to LG's own `nsl` when the Rust gateway is not
  answering. LG keeps issuing and storing every pairing, so falling back never asks for a new pairing.
- CPU is not a reason to do it. With the sysbus repair loaded, LG's host takes 0.067 % of a core with Home Assistant connected and
  0.45 % in Active Standby, where each of Home Assistant's reconnect attempts starts it (HANDOFF, 2026-11-10 and 2026-11-13).
  Without the repair it took 23.7 % (measured 2026-11-13).
- Memory is the gain, and only if the six other services sharing the host stay off as well. LG's activities start homeconnect,
  billing and magicnum at boot and on network changes and keep them busy, so the node host stays resident whether or not the
  gateway is Rust. With the gateway in Rust and the six off, node is expected not to run in normal use (step 1 of the build plan
  checks this): 27–41 MB resident (31.5 MB PSS measured with all guests) would be replaced by an estimated 3–6 MB.
- The cost is about 4,500 lines of Rust and 2,000 of tests for a LAN-facing service with privileged bus access, plus keeping it in
  step with LG firmware. PLAN parks LG service rewrites until after LumaTV and lets each go ahead only on a measured gain; step 1 of
  the build plan is that measurement and needs no Rust.

## 1. What the host runs

`/usr/bin/run-unified-service-server` with no arguments runs `node --max_old_space_size=220 … unified_service_server.js
--disable-timeouts`. ls-hubd starts it on the first call to any of the 14 `-u` services in `/usr/share/luna-service2/services.d/`
(all `Type=dynamic`); further services load into the same process through `luna://com.webos.service.jsserver/run`. The process
title is `ss.<first guest>`: `ss.gateway`, or `ss.apiadapter` when apiadapter came first. It quits once every guest is idle; a
guest held by a continuous activity stays busy.

| Service | What it does | Started by | Luna API | Needed | Verdict |
|---|---|---|---|---|---|
| `jsserver` | The host's control name | ls-hubd, on the first call to any `-u` service | `run`, `infoServices` | While any guest is | Stays LG's; leaves with its guests |
| `secondscreen.gateway` | SSAP server for phone apps and Home Assistant: port 3000 (loopback only) and 3001 (LAN, TLS) through xinetd and `nsl` | `nsl`, on every connection to 3000/3001; luna callers (`voiceinput.network` and `systemui.mobileconnection` subscribe to `getPairedDeviceList`) | heartbeat `ping`, `notifyNamedSocket`; `getPairedDeviceList`, `unpair`, `unpairAll`, `getServiceList`, `getHostMessage`, `getPowerState`, `setMobileAccess`; pairing `requestPairing`, `acceptPairing`, `rejectPairing`, `setPin`, `register`, `getPairingStatus`; app2app `registerAppChannel`, `createAppChannel`, `connectToApp`, `sendMessage`, `sendJoinResponse` | Yes: Home Assistant's `webostv` | Port 3001 to Rust; LG's stays for pairing and fallback |
| `secondscreen.client` | The identity the gateway's outgoing calls use | With the gateway | none | With the gateway | Goes with it |
| `homeconnect` | "Home Dashboard service agent": LG's IoT hub (ThinQ, Matter, Google Home runtime, LG cloud) | Continuous activity when the internet comes up | `start`, `stop`, `get`, `set`, `relayEvent` | Only for LG's Home Hub app (installed) or the TV's ThinQ/Matter role | Switch off, if Nitin confirms |
| `billing` | LG Content Store payments | Continuous activity when the internet comes up | `start`, `stop`, `get`, `set`, `request`, `proceed`, `confirm`, `relayEvent` | Only to pay through LG's store | Switch off |
| `igallery.preview` | Gallery+ previews for LG's Home, from LG's cloud | Activity when the internet comes up | `getstatusByActivitymanager` | No: LumaTV replaces LG's Home | Switch off |
| `universalqcard` | Quick cards for LG's Home | Activity when the internet comes up | `getstatusByActivitymanager`, `clickAlert` | No | Switch off |
| `sportsalert` | LG Sports Alert scores | Activities on internet, LG account, device sign-in, locale and settings changes | `…ByActivitymanager` callbacks, `invoke` | Only if Sports Alert is used | Switch off, if Nitin confirms |
| `magicnum` | Number-key shortcuts (Magic Number) | Continuous activities at boot and on launch-point changes; channel-list changes | `setCountry`, `checkLauncher`, `checkChannel` | Only if number-key shortcuts are used | Switch off, if Nitin confirms |

Service names are `com.webos.service.*` except `com.webos.app.igallery.preview` and `secondscreen.client`. The activities are static
files in `/etc/palm/activities/<service>/` (read-only overlay). On 2026-11-13 the host held all seven guests after 2 h 41 min, with
billing, homeconnect and magicnum busy. Other `-u` services that also start the host and stay LG's: `apiadapter` and
`apiadapter.pub` (the gateway sends most of Home Assistant's calls there), `dial` (casting from phones), `google.signin`,
`settingsui`, `tvstorepolicy`.

"Switch off" means not started in normal use, not removed: the boot hook cancels the six services' twelve activities through
`com.webos.service.activitymanager/cancel`. `activitymanager/list` shows all twelve, created by the services' own ids and waiting
on their triggers (2026-11-13); the activity manager loads them from the files again at every boot. A direct call (opening Home
Hub, a store purchase) still starts a service on demand. Untested: whether `luna-send` may cancel another service's activity,
and whether anything re-creates one before the next boot (the host runs with `--disable-activity-creation`).

## 2. What Home Assistant uses

Home Assistant 2026.11.0 runs `webostv` on aiowebostv 1.0.0, whose protocol is the same as 0.10.0's. Nitin's setup uses
`media_player.living_room_tv` (play, pause, stop, previous, next, volume set/step/mute, source, play media, on/off), automations
sending `webostv.button` from the HomeKit remote (UP, DOWN, LEFT, RIGHT, ENTER, BACK, HOME, FASTFORWARD, REWIND), the
`webostv.turn_on` trigger (answered with Wake-on-LAN, not SSAP) and `notify.lg_webos_tv_oled65c4pua_dusqljr` for toasts.

The client tries `ws://<tv>:3000` (xinetd admits only 127.0.0.1 there), then `wss://<tv>:3001` without certificate checks; it
pings every 5 s and uses 2 s connect, 10 s receive and 20 s request timeouts. Home Assistant's coordinator calls `connect()` every
10 s while disconnected; in Active Standby LG accepts the TLS connection and then closes it with 1008 "Try Again Later (EWS)".

The session is `hello` → `ssap://system/getSystemInfo` → `register` (unsigned manifest with 37 permissions, `pairingType: PROMPT`,
the saved `client-key`) → pointer socket → system, software and network information → eight subscriptions. Three rules bind any
replacement: an error answer to `register` raises `WebOsTvPairError`, which Home Assistant turns into a re-authentication that
needs the TV's prompt; since 1.0.0 any subscription error other than `404 no such service or method` fails the whole connect; and
an error reply must keep LG's shape, `{"type":"error","id":…,"error":"<code> <text>","payload":{}}`, because a reply without
`payload` fails the connect even where aiowebostv expects a refusal (`getCurrentSWInformation`).

| SSAP URI (`ssap://…`) | Luna call behind it | Permission |
|---|---|---|
| `system/getSystemInfo` | apiadapter → `factorymanager/getFactoryOpt` (modelName, serialNumber) + `config/getConfigs` | none |
| `com.webos.service.networkinput/getPointerInputSocket` | direct; the socket path becomes a `wss://<tv>:3001/resources/<hash>/<name>` URL that expires after 30 s | CONTROL_MOUSE_AND_KEYBOARD |
| `com.webos.service.update/getCurrentSWInformation` | direct | READ_UPDATE_INFO (SECURE: refused, as aiowebostv expects) |
| `com.webos.service.connectionmanager/getinfo` | direct | READ_NETWORK_STATE |
| `com.webos.service.tvpower/power/getPowerState` (sub) | direct | CONTROL_POWER |
| `com.webos.applicationManager/getForegroundAppInfo` (sub) | direct | READ_RUNNING_APPS |
| `com.webos.media/getForegroundAppInfo` (sub) | direct | CONTROL_DISPLAY |
| `com.webos.applicationManager/listLaunchPoints` (sub), `listApps` | direct; icon paths become `/resources/` URLs | READ_INSTALLED_APPS |
| `audio/getVolume` (sub), `setVolume`, `volumeUp`, `volumeDown` | `com.webos.service.audio/master/…` | CONTROL_AUDIO |
| `audio/getStatus` (sub), `setMute` | apiadapter → `audio/master/getVolume` (mute from `volumeStatus.muteStatus`), `audio/master/muteVolume` | CONTROL_AUDIO |
| `com.webos.service.apiadapter/audio/getSoundOutput` (sub), `audio/changeSoundOutput` | apiadapter → `settingsservice/getSystemSettings`, `setSystemSettings` (`sound`/`soundOutput`) | CONTROL_AUDIO |
| `tv/getExternalInputList` (sub) | apiadapter → `eim/getAllInputStatus` | READ_INPUT_DEVICE_LIST |
| `tv/switchInput`, `tv/openChannel` | apiadapter → `eim/getInputStatus`, then a launch through apiadapter.pub | LAUNCH |
| `tv/getChannelList` (sub), `getCurrentChannel`, `getChannelProgramInfo` | apiadapter → `iepg/notifyChannelListChanged`, `utp.broadcast/getCurrentChannel`, `iepg/getChannelInfo` + `getEventList` | READ_TV_CHANNEL_LIST, READ_CURRENT_CHANNEL |
| `system.launcher/launch`, `open`, `close`, `getAppState` | apiadapter → `applicationManager/launch`, `open`, `closeByAppId`, `running` | LAUNCH |
| `system.notifications/createToast`, `closeToast`, `closeAlert`; `createAlert` | apiadapter (createAlert: apiadapter.pub) → `com.webos.notification/…`, toast icons through `com.palm.filecache` | WRITE_NOTIFICATION_TOAST |
| `system/turnOff`, `tv/channelUp`, `tv/channelDown` | `networkinput/controls/power`, `channelUp`, `channelDown` | CONTROL_POWER, CONTROL_INPUT_TV |
| `media.controls/play`, `pause`, `stop`, `rewind`, `fastForward` | `networkinput/controls/…` | CONTROL_INPUT_MEDIA_PLAYBACK |
| `com.webos.service.tvpower/power/turnOnScreen`, `turnOffScreen` | direct | CONTROL_TV_SCREEN |
| `com.webos.service.ime/insertText`, `deleteCharacters`, `sendEnterKey` | direct | CONTROL_INPUT_TEXT |
| `media.viewer/close`, `webapp/closeWebApp`, `settings/getSystemSettings` | apiadapter → `applicationManager/closeByAppId`, `settingsservice/getSystemSettings` | LAUNCH, LAUNCH_WEBAPP, READ_SETTINGS |
| `api/getServiceList` | the gateway's own list | none |

Not declared on this firmware, so LG answers `404 no such service or method`: `system/turnOn`, `tv/set3DOn`, `tv/set3DOff`,
`com.webos.service.appstatus/getAppStatus`, `com.webos.service.config/getConfigs`. The map comes from the gateway's
`interfaces/*.interface` (virtual name → provider, at most two hops) and apiadapter's `adapters/`; a permission list in an
interface means any one of them suffices. The pointer socket carries `type:button\nname:UP\n\n` style records, each written to
`/tmp/netinput.pointer.sock` with a 4-byte big-endian length prefix; nothing comes back.

LG's own phone apps (the ThinQ remote and its predecessors) use the same websocket with manifests signed against
`signing-certificates/svl-signing-cert.pem`, which unlock SECURE permissions, and pair by PIN, PROMPT, COMBINED or LGSWITCH-PIN,
including BLE-started pairing (`bleRequest`, `r2rMode`) and app2app channels [inference from the gateway's code; no phone-app
traffic was captured]. ThinQ's device side runs through homeconnect and LG's cloud, and phone casting through DIAL (`dial`,
ports 36866 and 18181), not through the gateway.

## 3. Pairing

Keys live in db8 kind `com.webos.secondscreen.gateway.clientprofile:1`, owned by the gateway: a 32-hex-digit random key per
client, its manifest, the granted permissions, a hash seed and the last access time, up to 100 clients. Other callers cannot read
it: `luna-send com.palm.db/find` on the kind answers `db: permission denied`.

For an unsigned manifest LG grants the requested permissions whose type in the gateway's `permission-types/*.permissions` is not
SECURE. That rule reproduces Home Assistant's stored grant exactly: 37 requested, 25 granted, the same 25 LG stored.

`register` with a known key and no manifest answers `registered`; with an unknown key and no manifest, `401 client-key does not
exist`. With a manifest and a known key LG keeps the key, refreshes the profile and answers `registered` without asking. With a
manifest and no known key LG asks on screen; for `PROMPT` that is `com.webos.notification/createAlert`, modal, titled "Connection
Request: <app>", whose Yes and No buttons call `palm://com.webos.service.secondscreen.gateway/pairing/acceptPairing` or
`rejectPairing`. It shows one at a time, waits 120 s, waits for the set to be on and turns the screen on. Under LumaTV the alert
is drawn by `rustui/ui/src/system/alert.rs`, which runs the buttons' calls. Only the owner of the gateway's bus name receives
those calls, so a Rust gateway cannot run LG's prompt.

LG therefore stays the pairing authority, and the Rust gateway only serves a key LG has accepted:

- A session whose key the Rust gateway has not seen, or whose manifest is signed, is relayed whole to LG's gateway over
  `ws://127.0.0.1:3000`. xinetd admits loopback on 3000 and starts LG's `nsl` path; LG's code gives loopback no extra trust (the
  client address is only displayed, so LG's device list shows relayed clients as 127.0.0.1).
- When LG answers `registered`, the Rust gateway stores the key's SHA-256, never the key, with the grant computed as above, in
  `/var/lib/lumatv/ssgw/clients.json` (0600). The client's later connections are served by Rust; a relayed session stays relayed
  until it ends.
- The relay passes the client's `Host` header through, so LG's resource URLs name the TV's address; the Rust gateway corrects their
  scheme to `https`/`wss` and relays `/resources/` paths it did not issue.
- Rust never answers `register` with an error of its own; errors come only from LG through the relay.

Home Assistant keeps its key: its first session after the switch runs through LG, later ones through Rust. Every key Rust accepts
is one LG accepts, so a fallback never shows a prompt. Removing a client through the gateway's `unpair` or `unpairAll` changes
only LG's database: the Rust gateway re-checks a stored key through LG once it is 30 days old, and the plugin page has Forget
clients. Until then a removed client still reaches the Rust gateway.

## 4. Launch and substitution

Today xinetd (2.3.15.4, `-dontfork -stayalive -inetd_compat`) owns ports 3000 (`only_from 127.0.0.1`) and 3001 and forks
`/usr/bin/nsl <port> com.webos.service.secondscreen.gateway` per connection with the socket on stdin. `nsl` registers
`com.webos.service.nsl-<pid>` (its role allows only that name, from `/usr/bin/nsl`), calls the gateway's `/heartbeat/ping`, which
makes ls-hubd start the host if needed, then `/heartbeat/notifyNamedSocket {"path":"/tmp/nsl_socket_<pid>","port":3001}`. The
gateway connects to that path, receives the socket through SCM_RIGHTS and serves TLS on it. The private key comes from
`luna://com.webos.service.sm/crypto/getSSGSSL`, which also answers `luna-send` (a 1,679-byte PEM key; not recorded), and the
certificate chain from the gateway's `certs/cert/`.

Rejected:

- Binding over `/usr/bin/nsl`: it also serves DIAL (36866, 18181) and HbbTV (8000, 8001), and LG's `nsl` run from any other path
  fails its role check.
- Owning the gateway's bus names from Rust: a 64-bit program cannot load LG's 32-bit `libluna-service2`, so it would need a 32-bit
  bridge with its own role registering LG's names, which then clash with LG's dynamic service when falling back.

Chosen: override only `/etc/xinetd.d/ssg_https`, mirroring `tools/sysbus.sh`.

- `tools/ssgw.sh on|off|status|early` keeps state in `/var/lib/lumatv/ssgw/` with an `enabled` marker. `on` bind-mounts LumaTV's
  copy of `ssg_https`, identical except `server = /var/lib/lumatv/ssgw/bin/ssgw-accept`, over `/etc/xinetd.d/ssg_https` (a
  read-only overlay; nothing is written to firmware) and sends xinetd SIGHUP to re-read its configuration. It binds only when the
  file's sha256 is `01011c104d1e6874a1bd6f691bb07fd539155e802a8b4a91fb4b2acb5b85547c`, `/usr/bin/nsl`'s is
  `d2936420cdc07d4a84aa27fcc1d1cfaf1d0d5a35574b2f15e3ca6e8174eedfa6` and nothing is mounted there yet. `off` deletes `enabled`
  first, unmounts and signals xinetd. A reboot restores LG's file; `early`, run from `tools/autostart.sh` beside `sysbus.sh early`,
  binds again while `enabled` exists and always exits 0. Port 3000 is left alone: the relay and the fallback use it.
- `ssgw-accept` is a static program of about 150 lines that xinetd runs as root per connection. It connects to the gateway's
  `/tmp/lumatv-ssgw/accept.sock` and waits up to 200 ms for a ready byte; only then does it send the client socket (SCM_RIGHTS) and
  wait for a taken byte. If the socket is missing, refuses, or the ready byte does not come, it execs `/usr/bin/nsl` with xinetd's
  arguments and the client socket still on stdin: LG's path exactly, with LG's role check intact. If the ready byte came but the
  taken byte did not, it closes the connection rather than risk two servers on one socket; the client reconnects (Home Assistant
  within 10 s).
- `ssgw`, the gateway, is a static aarch64 program like the helper and a standalone engine with no LumaTV code. It listens only on
  the accept socket, never on 3001. Like LG's, it closes new websockets with 1008 "Try Again Later (EWS)" in Active Standby and
  refuses all of them while the `network` setting `allowMobileDeviceAccess` is off; both come from luna subscriptions. It makes
  its luna calls through `luna-send`, as the helper does.

The switch is the plugin: enabling it runs `ssgw.sh on` and starts the gateway; disabling it stops the gateway and runs
`ssgw.sh off`. With the plugin off nothing of this runs.

## 5. Fallback, crashes and notification

The helper supervises `ssgw` as plugin `ssgw`, off until enabled (`plugins::local::OptIn`), on the pattern of
`helper/app/src/beszel.rs`: a thread started by `plugins::guard::spawn`, the child confined through `plugins::runner::confine` if
`luna-send` works for the plugin user (untested; otherwise root), stopped with `end` after a 2 s grace, strays removed by `sweep`,
status published as the plugin's facts and logged once per change with `crate::log_warn!`.

- Detection: the child exiting without being asked to, or three missed health pings over the accept socket's control channel, 10 s
  apart. A hung child is killed with SIGKILL.
- Fallback needs no supervisor: while the gateway is down or hung, `ssgw-accept` sends each new connection to LG's `nsl`. Home
  Assistant's open session drops with the crash and comes back through LG within about 10 s, with the same key.
- Switch-back: new connections go to Rust as soon as a restarted gateway answers its first ping. Sessions already on LG stay there
  until they end, at the latest when the set next goes to standby.
- Retry: restart after 2 s, doubling to 60 s (Beszel's `FIRST_PAUSE` and `LAST_PAUSE`). After three crashes within 10 minutes the
  supervisor stops trying until the next boot or the page's Try again, and connections stay on LG.
- Record: each crash writes `/var/lib/lumatv/ssgw/crash/<time>-<n>/` with `reason.txt` (exit status or signal, or "stopped
  answering"), the gateway's last 300 log lines from the supervisor's buffer, uptime, session count and the last 20 URIs served
  (no payloads, no keys); the newest 8 are kept, and `pending` marks an unread one, as `helper/app/src/crash.rs` does for the
  compositor. Ordinary gateway log lines go to the helper's RAM log and from there to the remote collector when `base.logging` is
  on.
- Notification: each crash sends one message through the saved Home Assistant link (`ha_store::HaLink` and `voice_ws::Ha`, as
  `ha_weather.rs` calls services) to the notify service chosen on the plugin page; Nitin's phone is `notify.mobile_app_nitins_iphone`.
  The message names the reason, that LG's gateway is serving, and when the next try is. Giving up after three crashes sends one
  more. The plugin page shows the notice and its details until it is read.
- LG's host is a cheap fallback only with the sysbus repair loaded. The page shows `lumatv sysbus status`; with LG's module the
  fallback costs about 22 % of a core.

## 6. Baseline and estimate

| Process (2026-11-13) | RSS | PSS | CPU |
|---|---|---|---|
| Node host, LG's module, all seven guests, 2 h 41 min old, Active Standby | 41,548 kB (peak 49,484) | 31,519 kB | 23.7 % over 60 s |
| Node host, repaired module, gateway only, 53 s old | 27,368 kB (peak 30,972) | – | exited within the next minute |
| Node host, repaired module (HANDOFF) | – | – | 0.067 % with HA connected, 0.45 % in Active Standby |
| `beszel-native` / Home Assistant plugin (Rust, for scale) | 1,148 / 948 kB | – | – |
| One `luna-send` subscription | 2,048 kB | 415 kB | – |

The node host also had 13 threads and 78 descriptors. MemAvailable was 900,684 kB of 2,030,392 kB.

Estimate for `ssgw` [inference]: 2–3 MB resident with TLS. It holds two `luna-send` subscriptions of its own (power state, mobile
access setting) and, with Home Assistant connected, six more: Home Assistant's eight less two that share one, `audio/getStatus`
with `audio/getVolume` and the power state with its own. That is about 3–4 MB PSS in standby and 5–6 MB PSS with Home Assistant
connected. A 32-bit bus bridge on the pattern of `compositor/sm/lumatv-sm.c` would carry every call over one connection and bring
that to about 3 MB. CPU is near zero between events; each command costs one `luna-send` start, and a refused standby connection
costs one TLS handshake and an `ssgw-accept` start.

## 7. Test plan

Host, in `cargo test`, each under a second:

- A fake luna bus that answers from recorded replies (`docs/research/arch/x4/all.jsonl` holds the luna traffic of one Home
  Assistant connect, setup and all eight subscriptions) and records calls, so each SSAP URI is checked against the luna call and
  payload LG's code makes.
- Golden SSAP exchanges: `hello`, `register` in each case of section 3, request, subscribe, unsubscribe, every error code
  (400, 401, 403, 404, 409, 413, 1008), recorded once from LG's gateway with a test client (one prompt accepted on the TV).
- The permission rule against LG's stored grant for Home Assistant (25 of 37).
- The relay: an unknown key goes to a fake LG endpoint; its `registered` stores the fingerprint and serves the next session
  natively; LG's errors pass through unchanged; resource URLs are rewritten.
- `ssgw-accept`: ready and taken bytes, a missing socket and a silent socket exec the fallback (a stub in place of `nsl`), a lost
  taken byte closes.
- aiowebostv 1.0.0 against `ssgw` on the host with the fake bus: connect, every subscription, every command Home Assistant sends,
  buttons through the pointer socket, toasts with an icon, standby refusal, reconnect after a server restart.
- `tests/test_ssgw.py` for `tools/ssgw.sh`, as `tests/test_sysbus.py` does for sysbus: pinned hashes, refusal on another
  firmware, `early` always exiting 0, `off` removing `enabled` first.

On the TV, with Home Assistant, each item passing on Rust and again with the gateway stopped (LG):

1. Home Assistant reconnects with its saved key: no prompt, no re-authentication; the second session is served by Rust.
2. State: on, Active Standby and screen off shown correctly; the screen switch works.
3. Volume set, step and mute, with state following within a second.
4. Every entry in the source list: app launches (Netflix, YouTube, LumaTV, Marquee) and HDMI inputs.
5. Play, pause, stop, previous and next in an app and on Live TV.
6. Turn off; turn on through the Wake-on-LAN automation.
7. The HomeKit remote automations: every `webostv.button` key.
8. `webostv.command` with a native URI and with one Rust does not implement (relayed).
9. `webostv.select_sound_output` and the sound output shown.
10. `notify` toast with an icon, drawn by LumaTV and held by Do Not Disturb.
11. App artwork in Home Assistant (the `/resources/` route).
12. A new client's pairing prompt (relayed to LG) on the TV.

Soak: seven days with Home Assistant connected and the set used normally. Pass: no crash, `ssgw` resident memory, descriptors and
`luna-send` children flat after the first day, and the media player's unavailable time no higher than in the week before.

Crash injection, each checked for the fallback, the phone message, the crash record and the plugin page: SIGKILL; SIGSTOP (a
hang, found within 30 s); the accept socket removed; the helper killed; a `luna-send` that never answers; xinetd SIGHUP with
sessions open; a reboot (the minutes before `early` are served by LG); a changed `ssg_https` hash (no bind); three crashes in
10 minutes (gives up, one extra message).

## 8. Size and risks

Size [inference]: `ssgw` about 3,500 lines (TLS and websocket 400, SSAP protocol 500, clients and relay 600, Home Assistant's
URIs 900, luna client 400, resources and pointer socket 350, control channel 150) and 1,500 lines of tests; `ssgw-accept` 150;
the helper plugin 700 and 300 of tests; `tools/ssgw.sh` 200 and 150 of tests. The binary is about 3 MB with rustls and ring.

Risks:

- A LAN-facing TLS service holding privileged bus access: a permission bug hands LAN clients root-level luna calls. `ssgw` serves
  only its fixed URI table with LG's permission data and relays everything else to LG; it never passes arbitrary URIs to luna.
- A wrong `register` answer sends Home Assistant into re-authentication, which needs someone at the TV. Rust never originates one.
- Firmware updates can change the interface files, apiadapter's behaviour, `ssg_https` or `nsl`. The pinned hashes stop the bind
  on a new firmware, leaving LG's gateway.
- Toasts sent through `luna-send` carry another source id. `rustui/ui/src/dnd.rs` `holds_toast` holds Home Assistant's toasts by
  the ids `com.webos.service.secondscreen.gateway` and `com.webos.service.apiadapter`, and treats other `com.webos.` ids as LG's
  own, so it needs the new id (untested).
- The memory gain needs the six services off, which stops LG's Home Hub, store payments, Sports Alert, Gallery+ previews, Quick
  cards and Magic Number until switched back.
- A client removed through LG's `unpair` keeps reaching the Rust gateway for up to 30 days, or until Forget clients.
- A second implementation of a protocol LG owns, to keep in step with every firmware.

It is a poor trade for CPU, which the sysbus repair already fixed, and for memory unless the six services are off. It is worth
building only if step 1 shows the node host stays away with them off.

## Build plan

1. Measure first, without Rust: cancel the six services' activities (a reversible boot-hook step; Nitin confirms Home Hub, Sports
   Alert and Magic Number first), reboot, and with Home Assistant's `webostv` entry disabled for one evening of normal use (its
   TV automations and the HomeKit remote are off meanwhile), record how often the node host starts and how long it stays. Go on
   only if it stays away apart from casting and LG's Settings. The step is `lumatv lgextras off` (declutter's opt group
   `lgextras`); `docs/research/jshost/watch.py` records the host's starts with the launch that caused each, from LG's
   `/var/log/dbg-log`, which on 2026-11-13 showed Home Assistant's reconnects through `nsl` starting the host about every 71 s in
   Active Standby.
2. `ssgw` core: TLS with LG's key and chain, websocket, `hello`, `register` with the client store and LG relay, request and
   subscription framing, error codes; host tests with the fake bus and golden exchanges.
3. Home Assistant's URIs in `ssgw`, taking apiadapter's JavaScript as the reference; resources and the pointer socket; aiowebostv
   1.0.0 against it on the host.
4. `ssgw-accept` and `tools/ssgw.sh` with `tests/test_ssgw.py`; `early` in `tools/autostart.sh`.
5. The helper plugin: OptIn, supervision, health, retry, crash records, notification through the Home Assistant link, the page
   (status, crash details, Try again, Forget clients, notify target, sysbus status); the toast source id in `holds_toast`.
6. On-TV acceptance (section 7), receipts recorded; then the crash-injection list.
7. Seven-day soak; then the plugin's measured memory and CPU go into HANDOFF.
8. Optional: a 32-bit bus bridge in place of per-call `luna-send`, if the soak shows the `luna-send` children matter.

## Untested

- xinetd 2.3.15 re-reading a bind-mounted `ssg_https` on SIGHUP on this set, and its behaviour with sessions open.
- `ssgw-accept` handing sockets over and exec'ing `nsl` with the role check passing.
- `luna-send` for the plugin's confined user.
- Cancelling the twelve activities, and whether anything else starts those services.
- The toast source id under `luna-send`, and Do Not Disturb with it.
- LG's gateway and apiadapter behaviour for every URI above beyond the recorded Home Assistant connect.

## Sources

- Gateway, apiadapter and host code copied from the set: `/usr/palm/services/com.webos.service.secondscreen.gateway/`,
  `/usr/palm/services/com.webos.service.apiadapter/`, `/usr/palm/nodejs/`, `/usr/bin/run-unified-service-server`,
  `/etc/xinetd.d/`, `/etc/palm/activities/`, `/usr/share/luna-service2/`.
- aiowebostv 0.10.0 (PyPI) and 1.0.0 (GitHub tag); Home Assistant 2026.11.0 `homeassistant/components/webostv/`.
- `docs/research/lg-apiadapter-interfaces.md`, `docs/research/arch/x4/all.jsonl`, HANDOFF "LG unified service CPU repair,
  2026-11-10" and "LG unified service repair kept across installs, 2026-11-13", `tools/sysbus.sh`, `helper/app/src/beszel.rs`,
  `helper/app/src/crash.rs`.
