# On Home: Home Assistant's lights, sensors and devices on Home (result, 2026-10-30)

The Smart home card in the Rust shell receives Home Assistant's LumaShow integration over an encrypted connection.

## Current pairing (2026-11-02)

The existing LumaTV entry `01M627CQ3HVVVFM62HQEYVFE57` is connected. Its 48 shared entities and all control/sensitive options are unchanged; the TV exposes 46 supported tiles, with control disabled. No device action was sent.

The missing TV key was recreated with the existing `ble-link` command and supplied privately to the entry's automatic `reauth_confirm` flow over verified HTTPS. The key remains only in the protected TV store and HA configuration; no key belongs in this note. The token-backed plugin is separate from this card's encrypted pairing.

Reauthentication is integration-initiated. The HA REST flow endpoint ignores a supplied `context.source`; top-level `entry_id` starts **reconfigure**, not reauthentication. Discover the existing repair through `config_entries/flow/progress`, then complete that flow.

Native Settings reported 46 things and displayed live light states. The Smart home widget is enabled at Half width in the former At home slot; At home is hidden, not deleted. Clock, weather and apps remain. The existing canonical bundle installer refreshed the stale UI verification record without changing any installed binary; native startup is persistent/live, firmware verified and failures zero. A cold reboot was not performed.

 The bounded screen window expired after the widget selection was captured. Cleanup restored filmMaker/default OLED pixel backlight 84, confirmed Active Standby and removed the temporary remote key helper. The ordinary lease was released with local mirror absent and own remote token gone. Proof is under `/data/home/tmp/lumatv-ha-cameras-20261102/`; `smart-home-enabled.png` shows the saved widget choice. Earlier implementation notes below are historical.

## Resume here

**Locks (2026-10-30, Nitin's rules).** Done on the host, pushed with the commit that carries this note; nothing has run on the TV.

- **Lock:** one OK on an open lock's tile (or Lock on its sheet) calls `lock.lock` through `/v1/onhome/act`, from anywhere, with the status on the tile.
- **Unlock is a person at the TV's remote, and nothing else.** OK on a locked lock's tile opens the sheet on the question "Unlock <name>?" with Cancel in front and Unlock (danger colour) second; only the key `ok` on Unlock asks. The shell then reads `onhome/shell.json` (a token the helper writes at every start, 0600 in a 0700 directory, never in a topic or an answer) and posts `{id, token}` to `POST /v1/onhome/unlock`. That route answers only a loopback peer that shows the token. `POST /v1/onhome/act` refuses `unlock` and `open` for every lock, state and list of services (`needs_shell`), the planner has no row for them, the mirror strips them from a lock's ordinary services, and the listener sends them only for a confirmed touch from `OnHome::unlock`. The plugins' lease (`NEVER_CALLED`), the AI access module, luna and voice have no way to call a lock.
- **Journal:** every try is a line of `<data dir>/onhome/journal.jsonl` (and the helper's log): when, which lock, done, failed or refused, and whether the shell asked. Refused tries from the API are written too.
- **Home Assistant has the last word,** and today it asks for its PIN: a lock's `unlock` is a sensitive service (`lumashow-ha` `policy.py`, `hub.py` `_precheck_pin`), allowed only with the entity's sensitive switch on, a PIN set, `origin: touch`, `confirmed: true` and the PIN. The TV sends the touch and the confirmation and holds no PIN, so Home Assistant answers `pin_required` and the TV says "Home Assistant wants its PIN for this. Allow unlocking from the TV with a confirmation in its LumaTV entry". Unlock by confirmation alone needs a change in `lumashow-ha` (below).
- **AI and automation paths:** `docs/research/onhome-ai-unlock-gate.md` (what HomeLabber must expose; until it exists they simply refuse).

Next, in order: (1) `lumashow-ha`: the confirm-only unlock option and a release (Main's call); (2) Nitin: allow the lock entities for control and sensitive in the LumaTV entry, add the unlock notification automation (the YAML is in the gate note); (3) TV check list items 9 to 12 below; (4) HomeLabber's gated unlock tool, which is the only AI path.

State before this work (read on 2026-10-30): the Home Assistant plugin was not installed on the TV, no link file existed, nothing listened on 6054, and Home Assistant's *LumaTV* entry of the LumaShow integration was loaded but not connected, with no entity shared. The camera path through `ha-bridge` had been removed from the helper with the Scrypted cameras, so the TV had no end for the integration to dial.

Done:

- **The TV is one more LumaShow device to Home Assistant, and holds no token.** `helper/app/src/onhome/` listens on `onhome.port` (6054) while the Home Assistant plugin is switched on in Settings and a pairing key exists (`ble-link`, already done on the set: 6053 listens). The integration dials the TV; Home Assistant pushes what the owner shared with it (`lumashow-ha` `docs/protocol.md` sections 4 to 6) and accepts the TV's calls on the entities it marked controllable. The listener is `ha-bridge`'s session (`Session`, `wire`) in a socket loop of its own (`machine.rs`), because `ha-bridge`'s mirror keeps six attributes and drops a thermostat's temperatures, a cover's position and a fan's speed. Only `states` and `actions` are offered, so no camera and no notification comes down this link. `mirror.rs` keeps the entities whole and drops any domain Home has no tile for.
- **Words, not attributes** (`tile.rs`): the helper turns each shared entity into a tile (name, status line, glyph, tone, what OK does, the numbers a sheet moves). Lights (on/off, brightness, colour temperature), switches and fans (speed), scenes and scripts (run), sensors (value with unit; doors, windows, motion, moisture and the like as words), thermostats (room and set point, mode), covers (open, close, stop, position), locks and media players (status). An unavailable entity says so; with Home Assistant away every tile says "Not reachable" in its own name.
- **Routes and topic:** `onhome` topic; `GET /v1/onhome`, `GET /v1/onhome/shared` (the picker: search, room, kind, paged), `POST /v1/onhome/picks` (owner's profile only), `POST /v1/onhome/act`.
- **Home:** `home/onhome.rs`. A `Smart home` card (Settings › Home widgets adds it, or the layout editor): tiles two to a row on a card 640 px wide or more, one on a narrower one. OK enters the card; the D-pad walks the tiles; OK switches a light, switch or fan or runs a scene or script; OK on a thermostat, a cover or a thing that is only shown, a held OK, or the Menu key opens the tile's sheet. In the sheet Left and Right change the row in front (a slider is sent once the key rests, and what was set is shown until Home Assistant's own value arrives); a refusal comes back in words on the card and the sheet. With no tile the card says why (not switched on, not paired, waiting for Home Assistant, nothing shared yet) and OK opens the page.
- **Settings:** Home & look › Home Assistant › **On Home**. *On Home*: where the link stands, the tiles by kind (Left and Right move one earlier or later among its kind), "Show everything shared", and the way to the Home widgets page. *Add*: what Home Assistant shares, searched by name, narrowed by room or kind, grouped by kind, one switch each. Until the owner chooses, Home shows everything shared; the first change makes the list the owner's own (`<data>/onhome/picks.json`).
- **Icons:** eleven Lucide glyphs added to the module's set (thermometer, droplets, door-open, door-closed, fan, blinds, footprints, battery, lock-open, wind; `zap` was there).
- **Tests:** `helper/app/tests/onhome.rs` (a fake integration over the real wire: what is shared becomes tiles, a control reaches the integration and its answer comes back, nothing is asked for a lock or a garage door, a thing that is not on the list is not controlled, the topic carries no key or address), unit tests in `tile.rs` and `mirror.rs`; `rustui` unit tests in `home/onhome.rs`, `home/model.rs`, `home/surface.rs` (the card and the sheet read clean to the contrast and layout lint at 100 % and 200 %, dark and light) and `settings/pages/onhome.rs`; `tests/test_rust_onhome.py` on the host harness writes `docs/mocks/host-rust/onhome/*.png`.

## Safety (Principle 0)

- The TV never asks for an alarm panel, a siren, a camera, a valve, a garage door, a gate or a door cover, and a lock is only ever locked by a route: `tile::plan` is a table of (domain, verb) to one service and the few plain values it takes, and nothing else builds a call. A switch, script or scene whose name says it unlocks, disarms or silences a camera or an alarm is refused whatever its domain. A lock is locked from anywhere and unlocked only by a person at the TV (see the top).
- Home Assistant decides first: a service the integration did not list for the entity (`ctl`) is not called, and one that needs the PIN (`ctl_pin`) is not even kept. Control paused in Home Assistant shows tiles and calls nothing.
- Which entities the TV sees is the owner's choice in Home Assistant; the TV cannot widen it. The TV has no Home Assistant address or token, and the listener lets in only the holder of the TV's pairing key.

## Nitin's steps

1. In Home Assistant: Settings › Devices & services › **LumaShow** › the **LumaTV** entry › **Configure**. *Pick by type* (Lights, Switches, Scenes, Climate, Covers, Sensors) or by area or entity, then *Control*: let the TV act on light, switch, fan, scene, script, climate and cover; leave locks, alarms and cameras out. Leave *Send camera events* off. Save.
2. Switch the listener on: Settings › Home & look › Home Assistant › Use Home Assistant (it asks first; an agent does not open ports). It listens at once, with no helper restart.
3. Within a minute Home Assistant shows LumaTV connected (`binary_sensor.lumatv_connected` on, *Entities shared* above 0).
4. On the TV: Settings › Home widgets › Smart home › Half (or Home's layout editor › add). Choose what Home shows in Settings › Home & look › Home Assistant › On Home.

## TV check list (not verified)

1. `ss -ltn` shows 6054 on the TV's LAN address; `avahi-browse -rt _lumashow._tcp` (or Home Assistant's discovery) sees `lumatv-<six>`; the log has `onhome: listening at`.
2. Home Assistant's LumaTV entry connects by itself; `GET /v1/onhome` says `connected` with the shared count; the log says the integration connected.
3. A light, a switch and a scene shared and controllable: OK on the tile does it, the tile follows, a change made in Home Assistant moves the tile.
4. A dimmable light and a thermostat: a held OK opens the sheet, Left and Right change brightness and the set point, the lamp and the thermostat follow.
5. A garage door shared: shown, OK opens a sheet of words, nothing is sent (`log-helper` has no `actions.call` for it).
6. Stop Home Assistant: the tiles say Not reachable; start it: they come back by themselves.
7. Unpair (`ble-unlink`): the listener ends and Home says Not paired.
8. Idle cost: resident size and CPU of the helper before and after.
9. A lock shared and allowed for control: an open lock locks with one OK and the tile follows; a locked one opens the question, Cancel does nothing, Unlock asks Home Assistant (expect the PIN words until `lumashow-ha` has the option).
10. `/var/lib/lumatv/onhome/shell.json` exists, mode 0600, and the shell (`lumatv-ui`) can read it: Unlock says "The TV cannot unlock from here" when it cannot.
11. From the TV's loopback without the token, `curl -XPOST -H 'X-Luma-Client: x' -d '{"id":"lock.front_door","verb":"unlock"}' http://127.0.0.1:18731/v1/onhome/act` answers 409 `needs_shell` and Home Assistant's log shows no call; `POST /v1/onhome/unlock` with a wrong token answers 403. Both are lines of the journal.
12. After an unlock from the remote: the journal line, Home Assistant's logbook entry (`lumashow_action`, `lock.unlock`, `touch`) and the phone notification.

## Not built, and why

- No pointer clicks on a tile (the card is driven by the remote; the pointer selects the card as for every card).
- No weather, camera, notification or media control: the protocol carries no weather module, cameras come through Scrypted, and the integration's `media_player` verbs are not wired (a player is shown, not controlled).
- A thermostat's unit is read from the shared temperature sensors, else from its own numbers (the integration sends none for a thermostat): a Celsius thermostat in a house of Fahrenheit sensors would be read as Fahrenheit.
- The old Home Assistant plugin path (`ha-plugin-result.md`) is untouched and still works for lights and scenes in the At-home lane once linked; it needs a token on the TV, which this does not.
