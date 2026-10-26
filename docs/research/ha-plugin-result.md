# Home Assistant plugin on the TV

LumaShow's `plugins/home-assistant` runs on the TV unchanged, through the helper's plugin broker (API level 1). Its cards are chips in Home's At-home lane, its tiles are rows of the quick panel, its notifications are in the drawer, and its settings are a page of Settings (Home & look › Home Assistant). With no plugin card the lane says in two or three words where Home Assistant stands; the placeholder chips ("3 lights on", "Locked", "Playing · Sonos") are gone.

## Resume here

Done on the offline host and committed. Nothing has run on the TV.

- **The lane** (`2634637`, `7baeef5`): `widgets/HomeAssistantWidget.qml` draws the plugin's cards (the ring first), or one chip with the plugin's state (`PluginHub.laneNone`: Not switched on, Not linked, Token refused, Offline, Connecting, Nothing to show, Cannot run, Starting, Not running). OK on that chip opens Settings at the page.
- **The helper** (`5e5838d`): `plugins/runner.rs` starts the plugins the owner approved as users of their own; `plugins/ha.rs` is the lease (the plugin asks the helper for Home Assistant, never holds the address or token); `plugins/install.rs` and `cli.rs` are the owner's commands; `ha_store.rs` keeps the address and token (`<data>/ha/link.json`, 0600). New routes: `GET /v1/plugins/{id}/choices/{provider}` (the pickers of a plugin's page) and `POST /v1/plugins/lease/ha/refresh`. New topics: `ha.link` (state, address, whether a token is kept) and `signal.occupancy` (what the plugin says of the room; nothing on the TV reads it yet).
- **The plugin** is built from LumaShow's own crate: `helper/plugins/home-assistant` (bin `lumatv-plugin-home-assistant`, exe `home-assistant`, LumaShow's `main.rs` over its library). `helper/build.sh release` builds it with the helper and writes `helper/dist/plugins/home-assistant/{plugin.json,bin/home-assistant}` (LumaShow's manifest, copied unchanged: the approval code is the same on the Show and on the TV); `tools/build-ipk.sh` packs it under `payload/plugins/home-assistant`.
- **Settings**: `settings/PageHomeAssistant.qml` (tabs from the plugin's schema, drawn by `settings/PluginRows.js`: a switch, a number, a picked room, lights and scenes picked in an order) and a Home & look row that opens it. The address and "Saved" or "Not saved" for the token are shown, "Forget link" takes both away (OK, then "Sure?" and OK), and neither is ever typed or read. Rows say a name and a value only (Nitin, 2026-10-25); what a row would otherwise explain, including how to link, is its `help`, drawn by the shared info mark.
- **Tests**: `helper/app/tests/ha_plugin.rs` (the real plugin against the real broker and `ha_link::fake::FakeHa`: link, room to tiles and a card, a tap to a service call, doorbell ring and notice and Clear, pickers, token never in anything the TV draws, refused token tried once, forget, no link, a manifest naming a door stays off; and a test that fails when `tests/qmlhost/fixtures/ha-plugin.json` is not what the plugin and helper make now: `LUMATV_WRITE_FIXTURES=1` rewrites it); unit tests in `ha`, `runner`, `install`, `session`; one `#[ignore]` test (root only) that installs the real program with `plugin-enable`, runs it as its own user through the runner and ends it with `plugin-disable`. QML: `tests/test_ha_plugin_surfaces.py` (lane), `tests/test_ha_plugin_page.py` (quick panel, drawer, page), scenes `36-home-ha*`, `10-quick-panel-ha`, `13-notifications-ha`, `30-settings-home-assistant*` (`tests/qmlhost/scenes_haplugin.py`).

Open, in this order:

1. Nitin links it (steps below).
2. The TV check list (below).
3. LumaShow proposals (below): a lock card needs a change there.

## Nitin's steps

Home Assistant's tokens are not scoped, so the TV gets its own user.

1. In Home Assistant: Settings › People › Users › Add user, name `lumatv-home`, administrator off, "can only log in from the local network" on. Log in as that user, open its profile › Security › Long-lived access tokens › Create token, name `LumaTV home`, and copy it (it is shown once). [INFERENCE: a user who is not an administrator can read the areas and entities and call light and scene services; if the room list stays empty or the lights do not answer, the token has to come from an administrator.]
2. On the TV, as root over the Homebrew Channel's ssh:

   ```sh
   H=/var/lib/lumatv/bin/lumatv-helper
   B=/media/developer/apps/usr/palm/applications/org.nphil.lumatv/brand.json
   $H plugin-consent --brand $B --id home-assistant
   ```

   It prints what the plugin may do in words and a code (`bd302afc…` for LumaShow's manifest as it is today; it changes if the manifest does). Read the list. It is: use the TV's connection to Home Assistant, read rooms, notifications, lights, scenes, events and sensors, switch lights and scenes, show cards and quick switches, send notifications, say whether someone is in the room. Then:

   ```sh
   $H plugin-enable --brand $B --id home-assistant --approve <the code>
   $H ha-link --brand $B --url http://192.168.1.146:8123
   ```

   Paste the token, press Return, then Ctrl-D. The command prints the address, waits up to 10 s and prints what Home Assistant said (`connected`, `auth_failed`, `offline`); it never prints the token. `ha-unlink` and "Forget link" in Settings remove it; `plugin-disable --id home-assistant` switches the plugin off and ends its program.
3. Settings › Home & look › Home Assistant › Room: pick the room. The lane, the quick panel and (Doorbell tab) the doorbell follow.

## What it does, and what it cannot do

- Lights and scenes can be switched from the lane's chips and the quick panel. A doorbell ring arrives as a card and a notification; Home Assistant's own notifications arrive in the drawer.
- Locks, alarms, sirens, cameras, covers and valves are never called from the TV: the helper refuses those domains to every plugin whatever its manifest says, and a manifest that names one is not switched on. (The Show's plugin API refuses locks, alarms and sirens; the TV also refuses cameras, covers and valves.) Nothing on the TV unlocks or disarms.
- The address and token are written only by `ha-link` in a shell on the TV: no key press, no route of the helper and no plugin can set either. The Settings page can only take access away.
- What the owner approved is what runs: the approval code is the hash of the manifest's consent text, and the program's own hash is recorded when it is installed; a program or manifest that changed is not started and the `plugins` topic says why.

## How it works

- The helper (root) installs the package into `<data>/plugins/home-assistant` (`plugin-enable`) and runs `bin/home-assistant` as uid 20000 (the first free of the plugins' range), group 2000, in a session of its own, with a cleared environment, `no_new_privs`, no core dumps, priority 10 and ceilings (128 MiB address space for class S, 64 files, 64 processes, 1 MiB per file). If the helper dies, the program gets SIGTERM; a program that stops is started again after 2 s, doubling to a minute. The TV's kernel has no seccomp and no namespaces, so this is not the Show's box: the ceilings catch a runaway and only reviewed plugins belong in `enabled.json`.
- The program connects to the broker at `/run/lumashow-plugins/broker.sock`, is told where the lease is, and asks the helper to watch entities (`ha.watch`), call a service (`ha.call`) and list registries (`ha.list`). The helper holds the one WebSocket to Home Assistant, refuses what the manifest did not ask for, and refuses the never-called domains. A refused token is tried once, then only after `ha-link` or "Forget link".
- The plugin's cards, tiles and notifications are topics `plugin.home-assistant.cards|tiles|notifications|facts`; `PluginHub.qml` turns them into the lane's chips, the quick panel's rows and the drawer's entries. A tap is `POST /v1/plugins/home-assistant/actions/<action>`. The plugin's settings are `/v1/settings/home-assistant`, written in their kinds (a list of ids is `{"type":"ordered"}`).
- Logs: the helper's `/var/lib/lumatv/log-helper` (lines starting `plugins:`), the program's own `/var/lib/lumatv/plugins/home-assistant/log` (rotated at 256 KiB).

## TV check list (not verified: nothing here has run on the TV)

1. `$H plugin-status --brand $B` before anything: `enabled` is empty.
2. After `plugin-enable`, within a few seconds: `log-helper` has `plugins: home-assistant: started its program`, `ps` shows `home-assistant` with uid 20000 (not root) and `NoNewPrivs: 1` in `/proc/<pid>/status`, and `/run/lumashow-plugins/broker.sock` exists. If the uid or group switch or the limits fail on the TV's kernel, the program's entry in `GET /v1/plugins` carries the reason.
3. `curl -s http://127.0.0.1:18731/v1/plugins` lists `home-assistant` as `connected`.
4. `ha-link` ends with `connected` and Home Assistant's version. A wrong token ends with `auth_failed`, the lane says "Token refused", and Home Assistant's log shows one failed login, not a stream.
5. The page shows address, "Saved", Room tab with the rooms; picking one makes the At-home card show "<Room> N of M on".
6. A tap on a light chip or quick-panel tile toggles the real lamp and the tile follows; a scene tile activates the scene. A light switched from Home Assistant changes the tile.
7. Pick the doorbell (Doorbell tab) and ring it: a card at the top of the lane, a banner, an entry in the drawer; OK on it clears it. A Home Assistant notification (a low battery) appears in the drawer.
8. Stop Home Assistant: the lane says "Offline" and the program stays up; start it: it reconnects by itself.
9. "Forget link" with two OKs: the lane says "Not linked", `/var/lib/lumatv/ha/` has no `link.json`.
10. `plugin-disable --id home-assistant` ends the program in under 3 s and the lane says "Not switched on". Reboot the TV with it enabled: the helper starts it again.
11. Nothing printed anywhere holds the token: search `log-helper`, `plugins/home-assistant/log` and `GET /v1/state` for its first characters.
12. Idle cost: resident size and CPU of `home-assistant` and the helper before and after (record them here).

## For LumaShow (not edited; this is the exact proposal)

1. The mock's lane shows "Front door Locked", but the plugin does not read locks. Reading state only: add `ha.read:lock` to `plugins/home-assistant/plugin.json` and to the plugin's reads, and a read-only card for the configured room's locks (state, no action). The helper already allows `ha.read` of `lock`; it refuses `ha.call` of it. The approval code changes when the manifest does.
2. The page's help texts speak of "the Show" (for example the presence field). Neutral wording ("the device") would fit both.
3. The plugin's icons are `base.icons/home-assistant` and `base.icons/doorbell`; `PluginHub.iconMap` maps them to house-wifi and bell, the closest this TV has.

## Open

- `rustui/app/src/home.rs` (the Rust UI spike, which does not ship) still has the four placeholder chips.
- The plugin's own `help` texts reach the rows' info marks as they are; some speak of "the Show" (see above).
- The quick panel draws a scene tile as a switch (the plugin sends it with a state): a tap activates it.
- There is no on-TV entry of the address or token, on purpose.
- Back from the page returns to Home's status bar, as every Settings page opened from Home does.
- `signal.occupancy` is published and read by nothing.
