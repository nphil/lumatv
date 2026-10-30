# On Home: Home Assistant's lights, sensors and devices on Home (result, 2026-10-30)

The Smart home card of Home in the Rust shell, fed by Home Assistant's LumaShow integration. Built and tested on the offline host; nothing has run on the TV.

## Resume here

State before this work (read on 2026-10-30): the Home Assistant plugin was not installed on the TV, no link file existed, nothing listened on 6054, and Home Assistant's *LumaTV* entry of the LumaShow integration was loaded but not connected, with no entity shared. The camera path through `ha-bridge` had been removed from the helper with the Scrypted cameras, so the TV had no end for the integration to dial.

Done:

- **The TV is one more LumaShow device to Home Assistant, and holds no token.** `helper/app/src/onhome/` listens on `onhome.port` (6054) once `onhome.enabled` is set in `helper.json` and a pairing key exists (`ble-link`, already done on the set: 6053 listens). The integration dials the TV; Home Assistant pushes what the owner shared with it (`lumashow-ha` `docs/protocol.md` sections 4 to 6) and accepts the TV's calls on the entities it marked controllable. The listener is `ha-bridge`'s session (`Session`, `wire`) in a socket loop of its own (`machine.rs`), because `ha-bridge`'s mirror keeps six attributes and drops a thermostat's temperatures, a cover's position and a fan's speed. Only `states` and `actions` are offered, so no camera and no notification comes down this link. `mirror.rs` keeps the entities whole and drops any domain Home has no tile for.
- **Words, not attributes** (`tile.rs`): the helper turns each shared entity into a tile (name, status line, glyph, tone, what OK does, the numbers a sheet moves). Lights (on/off, brightness, colour temperature), switches and fans (speed), scenes and scripts (run), sensors (value with unit; doors, windows, motion, moisture and the like as words), thermostats (room and set point, mode), covers (open, close, stop, position), locks and media players (status). An unavailable entity says so; with Home Assistant away every tile says "Not reachable" in its own name.
- **Routes and topic:** `onhome` topic; `GET /v1/onhome`, `GET /v1/onhome/shared` (the picker: search, room, kind, paged), `POST /v1/onhome/picks` (owner's profile only), `POST /v1/onhome/act`.
- **Home:** `home/onhome.rs`. A `Smart home` card (Settings › Home widgets adds it, or the layout editor): tiles two to a row on a card 640 px wide or more, one on a narrower one. OK enters the card; the D-pad walks the tiles; OK switches a light, switch or fan or runs a scene or script; OK on a thermostat, a cover or a thing that is only shown, a held OK, or the Menu key opens the tile's sheet. In the sheet Left and Right change the row in front (a slider is sent once the key rests, and what was set is shown until Home Assistant's own value arrives); a refusal comes back in words on the card and the sheet. With no tile the card says why (not switched on, not paired, waiting for Home Assistant, nothing shared yet) and OK opens the page.
- **Settings:** Home & look › Home Assistant › **On Home**. *On Home*: where the link stands, the tiles by kind (Left and Right move one earlier or later among its kind), "Show everything shared", and the way to the Home widgets page. *Add*: what Home Assistant shares, searched by name, narrowed by room or kind, grouped by kind, one switch each. Until the owner chooses, Home shows everything shared; the first change makes the list the owner's own (`<data>/onhome/picks.json`).
- **Icons:** eleven Lucide glyphs added to the module's set (thermometer, droplets, door-open, door-closed, fan, blinds, footprints, battery, lock-open, wind; `zap` was there).
- **Tests:** `helper/app/tests/onhome.rs` (a fake integration over the real wire: what is shared becomes tiles, a control reaches the integration and its answer comes back, nothing is asked for a lock or a garage door, a thing that is not on the list is not controlled, the topic carries no key or address), unit tests in `tile.rs` and `mirror.rs`; `rustui` unit tests in `home/onhome.rs`, `home/model.rs`, `home/surface.rs` (the card and the sheet read clean to the contrast and layout lint at 100 % and 200 %, dark and light) and `settings/pages/onhome.rs`; `tests/test_rust_onhome.py` on the host harness writes `docs/mocks/host-rust/onhome/*.png`.

## Safety (Principle 0)

- The TV never asks for a lock, an alarm panel, a siren, a camera, a valve, a garage door, a gate or a door cover: `tile::plan` is a table of (domain, verb) to one service and the few plain values it takes, and nothing else builds a call. A switch, script or scene whose name says it unlocks, disarms or silences a camera or an alarm is refused whatever its domain. A lock is shown, never operated.
- Home Assistant decides first: a service the integration did not list for the entity (`ctl`) is not called, and one that needs the PIN (`ctl_pin`) is not even kept. Control paused in Home Assistant shows tiles and calls nothing.
- Which entities the TV sees is the owner's choice in Home Assistant; the TV cannot widen it. The TV has no Home Assistant address or token, and the listener lets in only the holder of the TV's pairing key.

## Nitin's steps

1. In Home Assistant: Settings › Devices & services › **LumaShow** › the **LumaTV** entry › **Configure**. *Pick by type* (Lights, Switches, Scenes, Climate, Covers, Sensors) or by area or entity, then *Control*: let the TV act on light, switch, fan, scene, script, climate and cover; leave locks, alarms and cameras out. Leave *Send camera events* off. Save.
2. Main switches the listener on: `"onhome": {"enabled": true}` in `/var/lib/lumatv/helper.json`, then the helper restarts (an agent does not open ports: this is the same owner step as `cameras.enabled` was).
3. Within a minute Home Assistant shows LumaTV connected (`binary_sensor.lumatv_connected` on, *Entities shared* above 0).
4. On the TV: Settings › Home widgets › Smart home › Half (or Home's layout editor › add). Choose what Home shows in Settings › Home & look › Home Assistant › On Home.

## TV check list (not verified)

1. `ss -ltn` shows 6054 on the TV's LAN address; `avahi-browse -rt _lumashow._tcp` (or Home Assistant's discovery) sees `lumatv-<six>`; the log has `onhome: listening at`.
2. Home Assistant's LumaTV entry connects by itself; `GET /v1/onhome` says `connected` with the shared count; the log says the integration connected.
3. A light, a switch and a scene shared and controllable: OK on the tile does it, the tile follows, a change made in Home Assistant moves the tile.
4. A dimmable light and a thermostat: a held OK opens the sheet, Left and Right change brightness and the set point, the lamp and the thermostat follow.
5. A lock and a garage door shared: shown, OK opens a sheet of words, nothing is sent (`log-helper` has no `actions.call` for them).
6. Stop Home Assistant: the tiles say Not reachable; start it: they come back by themselves.
7. Unpair (`ble-unlink`): the listener ends and Home says Not paired.
8. Idle cost: resident size and CPU of the helper before and after.

## Not built, and why

- No pointer clicks on a tile (the card is driven by the remote; the pointer selects the card as for every card).
- No weather, camera, notification or media control: the protocol carries no weather module, cameras come through Scrypted, and the integration's `media_player` verbs are not wired (a player is shown, not controlled).
- A thermostat's unit is read from the shared temperature sensors, else from its own numbers (the integration sends none for a thermostat): a Celsius thermostat in a house of Fahrenheit sensors would be read as Fahrenheit.
- The old Home Assistant plugin path (`ha-plugin-result.md`) is untouched and still works for lights and scenes in the At-home lane once linked; it needs a token on the TV, which this does not.
