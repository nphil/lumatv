# Local profiles

Nitin and Isabel each have a profile on the TV: their own Home layout, dock, quick panel, look, wallpaper, accent and text size, and their
own name and picture in the status bar. The LG account is not a profile and is no longer read for one. The logic and the data are the helper's
(`helper/app/src/profiles.rs`, host tests in the module, `/v1/profiles`, topic `profiles`); the screens are the Rust shell's
(`docs/research/rust-shell-plan.md`, 4.4). Nothing new is QML.

## Per profile and shared

The one table is `payload/compositor/lumatv/profile-scope.json`: dotted paths of `theme.json`, longest prefix wins, a key in neither list is
shared (what every key was before profiles, so a key added by anyone is the household's until its path is added).

| Per profile | Shared (household) |
|---|---|
| look: `wallpaper`, `material`, `dark`, `light`, `accent`, `tint`, `depth`, `saturation`, `wallpaperMotion` (not `source`, `glass`) | picture, sound, network, inputs, receiver, ambient light (LG settings and the helper's settings) |
| text size: `type` | privacy, AI access, updates, panel care (`oledcare`), screensaver, power-on and power-off logo |
| cards and rows: the layout file (`layout.json`), `clock`, `news`, `weather.units`, `weather.days` | the weather place (`weather.zip`), remote-key behaviour, `overlays.enabled` and sizes, `cameras.*` |
| dock apps and order: `apps` (`dockOrder`, `dockHide`, hidden, names), `tiles.icons`, `tiles.plates` | camera options (the helper's `scrypted-cameras`, device-wide) |
| quick panel tiles, `remote.quickAccess`, `remote.appKeys`, `search`, `motion.reduced`, `layout.overlays`, `overlays.classes` | LG's own launch point list: what is installed, never written for a profile |

Reasons: the screen is one for the room, so everything that changes what the TV does (picture, sound, network, the receiver, privacy) is
shared; everything that is only how one person likes their Home is theirs. The camera pop-up policy runs in the helper for the whole TV.

## Storage and switching

`theme/theme.json`, `layout.json` and `tint.json` stay the **working copy** that Home, `lumatv theme set` and `lumatv layout` already read and
write: the household's keys plus the active profile's. Each profile keeps its own part in `profiles/<id>/` (`profile.json`, `theme.json`,
`layout.json`, `tint.json`, a photo) and `profiles/index.json` names the owner and the active profile.

- **Switch.** The helper writes the new person's part into the working copy and publishes `profiles`; Home reads the files at once
  (`LumaHome.profilesChanged`; the 2 s look is the fallback) and closes what was open. The intent is written first and cleared last, so a
  helper that dies half-way finishes the switch at its next start. A host switch of a 19 KB theme takes 8 ms in a debug build.
- **Edits.** The helper carries what Home wrote back to the profile it was written in (every 1.5 s and before every switch). Theme.json
  carries `_profile`, so an edit saved by a screen that had not yet seen a switch lands in the person who made it, and the working copy is
  made the new person's again.
- **Migration** is the first start with no index: the working copy becomes Nitin (his own keys cut out, the rest staying), Isabel is made from
  his arrangement (cards, dock, quick panel, key choices, popup places; not his look). Deleting `profiles/` is the way back.
- **LG's patched surfaces** (generated at `lumatv apply`: LG's own Quick Settings, keyboard, notifications, web apps) take the look of whoever is
  active at that moment; LumaTV's own panels follow the active profile live.
- **Dock.** `apps.dockOrder` and `apps.dockHide` are the person's; LG's list is not moved or hidden any more.
- **Speed.** The helper's part is single-digit milliseconds. Home applies a switch in one reload; the veil that hides it, and the picker's
  150 ms after wake, are the Rust shell's to meet (budget: a switch fully applied under 300 ms). No network is on the path: the gallery and
  photos are files.

## Who may do what

The owner (Nitin) is the administrator: only the owner's profile may create or remove a profile or set the options, and shared settings are
his. The helper enforces what it owns: the profile rules themselves and every helper module that calls `Profiles::require_owner()` (403
`owner_only`; Privacy and "Open apps without updating" do). LG's own settings that QML writes through `settingsservice`, and LG's Settings
app, are not gated tonight: the Rust Settings locks those rows from `canAdmin` in the topic ("Only Nitin can change this", with Switch to
Nitin). A profile's name, colour and picture are its own or the owner's.

An optional four-digit PIN per profile: needed to switch to that profile unless the owner is active; five wrong in a row make it wait 30 s,
each further five twice as long, five minutes at most. It is a courtesy lock between two people, not security (four digits have ten thousand
guesses and the files are the TV's): only a salted hash is stored, no digit is logged, AI access (MCP) has no profile tool.

## LG account

Home read `accountmanager/getLoginUserData` in one place, the status bar's profile item (`StatusBar.qml`); that call and its "Manage LG account"
row are gone, and the shell's bridge allowlist no longer names the service. LG's own surfaces keep working because they never went through
LumaTV: the store (`com.webos.app.discovery`), the LG Account app and its overlay, and the services with permission files that name the service
(on the set: its own role, client permission and manifest, `com.webos.service.audio`, and the API permissions of `sportsalert` and `compat`).
Sign-in stays reachable in LG's Settings through "More in LG settings".

## Avatars

A picture of the profile is one of: a bundled one (24 in `payload/avatars/`, two sets of twelve from DiceBear's CC0 1.0 styles, People
(Lorelei) and Critters; `tools/gen_avatars.py` rebuilds them, `NOTICE.md` names the artists), its own photo, or the colour and the letter.
Avatars are squares; the screen gives them their shape. A photo is a PNG or JPEG of at most 48 KB sent as base64 to
`POST /v1/profiles/{id}/avatar`, which the screen crops and shrinks first. Import from Home Assistant (a person's picture) and from Immich
goes through Home Assistant's media source, with the helper fetching on the saved link, so the TV never holds an Immich key and no port is
opened; it needs a picture fetch in the shared `ha-link` crate (LumaShow's, not forked here) and is not built.

## Open questions (Nitin)

1. **Who may change the shared settings?** Recommended: only the owner's profile (Nitin), as built. Isabel changes everything about her own Home;
   privacy, AI access, updates and network stay Nitin's.
2. **Show "Who's watching?" when the TV turns on?** Recommended: off (as built, `pickerOnWake`), the TV starting in the profile used last.
3. **Should Isabel's profile have a PIN?** Recommended: none for either, set only where one wants it; the owner can always clear hers.

## Screens (the Rust shell's; decisions)

- **Avatar:** a rounded square (corner about a fifth of the side, Lucent's tile radius), never a circle: the picture, or the colour and the letter.
- **Status bar:** the chip is the avatar at 48 and the name; OK opens the quick panel's strip.
- **Quick panel:** the profiles are its first row, one OK switches, the current one marked with a tick and full ink, the others in the second ink; a
  profile with a PIN asks it in place.
- **"Who's watching?":** full screen over the blurred wallpaper, tiles of 220 with the name under, the current one ticked, focus as everywhere
  (landing light, tile scale 1.12), Back keeps the current person. Shown on wake when `pickerOnWake` (off by default) and Home is in front.
- **PIN pad:** four dots; the remote's digits fill them and the fourth submits; a 3 x 4 pad for the D-pad and the pointer; a wrong PIN shakes and
  clears, `429 pin_locked` shows the wait.
- **Settings > Profiles:** the list; Add (owner), edit name, avatar (gallery, photo, colour and letter), colour, PIN, text size; Remove with the danger
  button and a confirm; the two options.
- **First run:** a short sheet over the new Home (look: dark, light or auto, and the accent), then `POST /v1/profiles/{id}` `{"firstRun": false}`.
- **Switching:** an opaque veil (90 ms in, held while the new Home is applied, 130 ms out; reduced motion a 120 ms fade), so a late frame is never
  seen; the picker is resident and shown under 150 ms after wake; nothing on the path touches the network.

Until those exist tonight's QML shows the active profile's name, letter and colour in the status bar and switches from its popover. Profiles are
created and edited with the API, for example
`curl -s -X POST -H 'X-Luma-Client: cli' -H 'Content-Type: application/json' -d '{"id":"isabel"}' 127.0.0.1:18731/v1/profiles/switch`.

## TV checks

1. After install: `GET /v1/profiles` lists Nitin (owner, active) and Isabel; `theme/theme.json` has `_profile: "nitin"` and is otherwise what it was;
   Home looks as it did. Nothing in the log from `profiles`.
2. Switch to Isabel from the status bar popover: Home shows her layout and look within a moment, the dock follows her order, nothing stays open.
   Switch back: Nitin's Home is exactly as before (wallpaper, accent, cards, dock order, quick panel).
3. Move a dock tile as Isabel, change the accent as Nitin: switch back and forth; each keeps its own. `lumatv layout get` and `lumatv theme get`
   show the active person's.
4. Kill the helper during a switch (`kill -9`), start it: one profile is active and the working copy is that person's whole.
5. Wallpaper that differs between the two: the accent and light/dark follow the new wallpaper after a sample; note any visible pop.
6. A large text size on one profile: open the quick panel, Settings and the dock; nothing overlaps (frame gate for first draws of the new sizes).
7. `getLoginUserData` is not called (`lumatv log`), the LG Content Store opens and installs as before, the LG Account page opens from LG's Settings.
8. Standby and wake: the active profile is the one before standby; `pickerOnWake` (set with `POST /v1/profiles/options`) has no screen until the Rust shell.
