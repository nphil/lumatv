# Plugin migration audit

Core keeps the trusted pieces: HA token store and weather source, Denon receiver, HDHomeRun tuner, screensaver and BLE key. Packaged plugins (home-assistant, scrypted, photos, ble-proxy) are the migration targets, wired through the existing broker (plugins/ in the helper, FIRST_PARTY_IDS in local.rs). Permissions limit what each can reach: ha.call domains, camera.provider (level 2), lease:decoder (level 2), net.listen brokered, net.lan for LAN devices. Denon and the tuner need new permissions before they can move.

Report (full table, for the main agent to write to docs/research/plugin-migration-audit.md):

| Integration | Files today | Plugin status | Stays in trusted helper | UI surfaces | Settings and secrets (names only) | Size | Risks |
|---|---|---|---|---|---|---|---|
| HA smart home (room tiles, lamps) | onhome/history.rs, onhome/tile.rs, rustui onhome/, home/ | Packaged home-assistant 0.2.0 (class L, lease:ha, lease:decoder); widgets room, doorbell | HA connection, token, ha.call checks | Home card tiles, doorbell slot | ha/link.json (secret); show-ha-token (secret) | M | Moving the card changes live lamp control. Lamp control currently off. |
| HA token store | ha_store.rs, voice_store.rs | Core | Yes | None | ha/link.json, voice/ha.json (secrets) | S keep | Rewriting either breaks HA until re-linked. |
| HA weather source | ha_weather.rs, home/weather_card.rs, base.weather source=homeassistant | Core; first-party weather runs separately | Token read | Weather card | base.weather source and entity | M | Two weather sources can drift. |
| Sensor history | onhome/history.rs | Core | Recorder queries with token | Sensor sheet | Sensors shared with TV | M | Sensor sheet untested on TV. |
| Scrypted cameras (status-bar button, viewer, feeder) | scrypted/ (events, policy, popup_stream, routes, bridge), live/mod.rs, gfn/feeder/, rustui cameras/ | Packaged scrypted (class M, camera.provider, net.lan 192.168.1.69:10444); first-party id scrypted-cameras reserved, not spawned | Login, token, RTSP via GFN feeder, event ring, policy | Cameras page, status-bar button, popup | scrypted-login.json (secret); scrypted-cameras.json; wall-cameras.json | L | Two ids clash; video needs lease:decoder (level 2). |
| Ring doorbell | No own module; Scrypted-bridged camera (live/mod.rs; commit ac106ac6) | Part of Scrypted | Same as Scrypted | Doorbell tile, Cameras | Same as Scrypted | In Scrypted | Do not split out. |
| Plex photos and wallpaper | Plex link not located; profiles.rs wallpaper key | Packaged photos (class L, net.lan.http:plex, ui.ambient, ui.fullscreen, storage 16 MB) | Plex address and token if stored | Ambient frame, fullscreen | Plex key and address (not read) | M | Plex link location unknown. |
| Denon receiver | receiver.rs, receiver_telnet.rs, receiver_volume.rs, receiver_settings.rs, receiver_sync.rs, ai/receiver_ctl.rs, lumatv_denon crate | Core | Raw telnet and volume pacing | Volume keys, receiver status | base.receiver (enabled, default true); receiver.volumeGapMs, volumeSettleMs, volumeRetries; telnet_port | L | No raw TCP permission at level 2. Needs new permission. |
| HDHomeRun tuner (Live TV) | hdhr/ (HdhrConfig in config.rs), helper/tuner/ | Core behind base.livetv (off by default); commit a559ccb3 | Inbound HTTP (discover, lineup, auto, xmltv) | Live TV settings | base.livetv; mac_file, dvb_dir, python, port 5004 | L | Needs inbound HTTP; settings crash undiagnosed. Commits 11343557 and 429039de not verified. |
| Screensaver content | saver.rs, rustui system/saver/, /v1/saver | Core | Screen state, OLED protection | Screensaver, complications | screensaver.style, screensaver.own | M | Accepted on TV 2026-11-02; keep core, move complication sources only. |
| BLE proxy (HA Bluetooth) | ble* modules | Packaged ble-proxy (class L, lease:bluetooth, lease:esphome, net.listen:6053, net.mdns) | ESPHome key and device identity | Tiles, notifications | Device key (secret, not read) | L | Voice and BLE share one device; move last. |
| Built-in weather (first-party) | wx*.rs, home/weather_card.rs, first-party weather | First-party in helper | No | Weather card | Weather settings | M | Already first-party; changing it changes a working card. |
| Clock, news, ambient | First-party in helper (local.rs) | First-party | No | Clock, news, ambient | Feed settings | S | Already first-party; low value now. |
| Marquee | Native LG app com.beb.plxnative (HANDOFF, PLAN); not in helper or rustui code | Not a plugin; separate native app | Out of scope | Marquee app | Own (not read) | Out of scope | Parked per PLAN. |
| Plugin broker | helper/app/src/plugins/ | Core | Yes | None | <data>/plugins/enabled.json (roster; owner-written only) | n/a | Out of scope. |

Verified plugin API facts: camera.provider is level 2 (needs cameras:1); lease:esphome is level 1 (esphome.attach); lease:decoder is level 2; net.listen:6053 is brokered via net.listening (Wi-Fi address and mac). ha.call allowed: light, scene, switch, cover, climate, fan, media_player, lock, alarm_control_panel, siren. Refused: script, automation, homeassistant, mqtt. Classes S/M/L (RAM 8/24/64 MiB).

Proposed order (lowest risk and highest value first; TV works at each step):
1. Plex photos (photos, additive). Locate the Plex link first.
2. Scrypted cameras (scrypted). Pick one id first (scrypted vs scrypted-cameras). Keep Ring inside it. Test the live view on the TV before switching.
3. Home Assistant (home-assistant). Keep token store and weather source in core. Move only room and doorbell widgets. Lamp control stays off until tested.
4. Screensaver complications: keep core; move only complication sources.
5. BLE proxy (ble-proxy) last; shares the voice device and ESPHome key.
6. Denon and HDHomeRun: stay in core until new permissions exist.
7. Marquee: out of scope.

Unverified: status-bar camera button location; Plex link code; commits 11343557 and 429039de; repo uncommitted state (not checked; do not commit). No secret values were read.

Live-state risks on Nitin's TV: HA tokens (ha/link.json, voice/ha.json); Scrypted login and camera layout (scrypted-login.json, wall-cameras.json); base.livetv settings; paneldim.json must be restored on every exit; direct boot (LG Home removal) means no core piece may depend on LG Home.