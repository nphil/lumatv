# App switcher: a second press of Home

Built and host-tested; off until its TV check passes. Switches (Settings > Remote & pointer): `remote.appSwitcher` (App switcher on a second Home press) and `remote.recents` (Recent apps on Back, which opens the same switcher). The shell tells the compositor it can show it (`can: ["switcher"]`) only while `remote.appSwitcher` is on; with it off every Home press is what it was. Mocks: `docs/mocks/switcher/` (dark and light, 100 % and 200 % text, the shrink from an app as four frames).

## What it does

| Home press | What happens |
|---|---|
| Any single press | Goes Home when the key is let go, exactly as before: the call to the app manager is made in the call that sees the release; nothing waits to see whether a second press comes. |
| A second press within 400 ms (release to release) of a press that went Home from an app | The switcher opens with that app's picture shrinking from the whole screen into its card (200 ms). The second press does not ask for Home again. |
| A press on Home with nothing open on it and at least one app running | Goes Home as before; the compositor also tells the shell, which opens the switcher when Home has nothing open (a layer, Settings, a prompt, edit mode close first, as they always did). |
| A press while it is open | Closes it. |
| Hold of 650 ms | The escape to LG's compositor, unchanged (`input.rs`, `lumatv-sm.c`); the tests hold it right after a tap and with the switcher open. |

Back from the top of Home (`remote.recents`) opens the same switcher: one panel, one list, two doors. The old Home-layer Recents panel is gone.

In the switcher: Left and Right move between the cards (a window of four moves a card at a time past four), Down goes to the card's Quit, Up to Quit all in the header, OK opens the card, quits, or asks; Back or Home closes; a pointer hovers to focus and clicks to act. Quit all asks first (opaque backdrop at `scrimHide`, Cancel in focus, the danger style on Quit all only). An app being quit shows "Quitting…" and dims until its window is gone, "Couldn't quit" with "Try again" after 4 s. No apps open shows its own words. After 45 s without a key it closes (a still picture of an app is not left on an OLED).

## How

**Pictures are the windows' own buffers, drawn again scaled by the compositor.** Nothing is copied to the CPU. The shell cannot sample another client's buffer, so it places the cards (`POST /v1/switcher` `layout`: rectangle and corner radius per live app) and the compositor draws each app's last buffer there, under the shell's system layer and over everything else, with one scrim (the palette's `scrimHide`, .973: words sit on it). Pictures move on the compositor's clock (`Track`: p = (now - t0) / ms, Lucent's settle and exit curves, retargeted from where they are), so a 200 ms move lasts 200 ms at 60 and at 120 Hz. The shell's chrome (names, ring, Quit, hints) only fades; the focus ring never moves while it shows. Closed, the plan carries nothing of it, the renderer holds nothing, and there is no timer.

- Files: `compositor/crates/lumatv-compositor/src/switcher.rs` (trigger `HomeTaps`, apps, motion, ops, topics), `src/scene/backdrop.rs` (the scrim and pictures in the plan, damage, `coverage`), `src/render/gl/previews.rs` (GL: a program that rounds the corners, and a chain of two copies at half and a quarter size made by exact 2x draws so a 1920x1080 window shrunk to 400x225 does not shimmer), `render/software.rs` (host), small hooks in `input.rs`, `shell.rs`, `scene.rs`, `server/state.rs`, `output.rs`, `kms/mod.rs`, `control.rs`. Shell: `payload/compositor/lumatv/panels/Switcher.qml` and `SwitcherLayout.js` (held by `LumaOverlay.qml`), `compositor/shell/Shell.qml`, `imports/LumaShell/Link.qml`, `LumaBus.js`, `LumaHome.qml` (Back, `atRest`), `settings/PageRemote.qml`.
- Topics to the shell: `apps` (running apps, newest first: id, title, `picture` live | video | none, size, `state` "" | quitting | stuck) and `switcher` (`open`, `from`, `n`); `homekey` carries `switcher: true` when it is on offer. Requests from the shell: `open`, `layout`, `close`, `activate`, `quit`, `quitall`. `ctl switcher [open|close|quitall|activate ID|quit ID]` does the same from the TV's shell and reports the cards.
- The list is the compositor's cards (mapped, in front at some time, not in a surface group), newest first, minus Home's stub and LumaTV's own apps. An app whose picture is on the video plane (a hole in its window, an exported video window, or an input: HDMI, Live TV, DP, USB-C) is never sampled; the switcher shows its icon on a tile of its own colours (the icon decoded at 16 px and drawn large: no blur shader) and says "No preview". Nothing of the video plane, a DRM path or a protected picture is read.
- Quit: the window gets `close` and the app manager gets `luna://com.webos.applicationManager/closeByAppId {id, reason: "windowPolicy"}`, the call LG's own recents makes (`StarfishRecents.__killItem` closes the window with that reason). OK on a card: `applicationManager/launch {id}` and the card's picture grows to the whole screen; it is dropped when that app is in front (or after a second).
- Under LG's compositor (the shadow module, after the Home hold escape) the same surface takes LG's `listRecents` and snapshots and calls the app manager itself; there is no second press there.
- Transitions: `Scene::take_visual` hands over the picture of an app that was on its way home at the second press; while the switcher is active `scene.transitions.suspended` stops the transitions (agreed with that slice).

Motion: pictures arrive 200 ms (from 94 % and transparent, or the app's whole screen), leave 160 ms, the dim 160 ms, a card out of the window or being quit fades 120 ms, the chosen picture grows 200 ms; reduced motion is opacity only at 120 ms. The PLAN gate's 180 ms burst after a key counts the first frame (about 40 ms after the key) plus 200 ms: this switcher's motion is the 200 ms the task allows, not the 110 ms panel slides.

## Checks run

- Compositor: `cargo test` (all green, with and without `--features ui`; `tests/switcher.rs` 24 tests: the single press is Home in the release's own call, the second press, the 400 ms edge, switch off and shell away, the hold at 649/650 ms right after a tap and with it open, the offer on Home, video apps, pictures only while open (plan, pixels, closed again), scaling and rounded corners on the software and on Mesa's GL renderer, shrink from an app in 200 ms, reduced motion, the 600 ms layout limit, OK launches and grows, Quit through `closeByAppId` with the card going when its window does, an app that does not quit, Quit all, another app coming to the front, a shell that connects again being told the apps again; 12 unit tests in `switcher.rs`), clippy both ways, rustfmt of the new files.
- Shell: `python3 tests/qmlhost/compile_all.py` 0 failed; `tests/test_switcher.py` (the surface through the host with the overlay: places sent, D-pad, each choice's request, Quit all's question, the window of four, closing and stuck cards, reduced motion, LG's compositor path), `tests/test_shell_switcher.py` (through `Shell.qml` and the channel stand-in: `can`, the topics, the keys, the Home offer); `tests/test_cutover.py` and `tests/test_gate.py` green; renders `docs/research/restyle-fixes/switcher/` (`python3 tests/qmlhost/scenes.py --only switcher-open,... --out ...`, dark and light, 200 % text through the overlap audit).
- Mocks: `docs/mocks/switcher/render.mjs` (overlap, overflow, safe area, no type under 21 px at 100 %) and `contrast.py` (weakest text 5.3:1 light, 7.5:1 dark, measured on the pictures).

## TV check (not run: no TV access)

`T=lumatv-compositor`. Open Netflix, Plex and YouTube (go Home between), then:

1. Off: `lumatv theme set '{"remote":{"appSwitcher":false}}'`. Over Plex, tap Home twice within 0.3 s: two `launchDefaultApp` lines in `ctl journal`, no switcher. Same on Home.
2. On: `lumatv theme set '{"remote":{"appSwitcher":true}}'`; `$T ctl shell`: `can` has `switcher` within 2 s.
3. A single Home tap over Plex: Home appears as before (journal `input: Home: launchDefaultApp`), nothing else, no later line. Time from the key to Home against step 1: the same.
4. Over Plex, Home twice within 0.4 s: one `launchDefaultApp`, the journal says `Home twice within 400 ms: the app switcher, with … shrinking into its card` and `switcher: the shell placed the cards`; Plex's picture shrinks into the first card in 200 ms (`ctl capture /tmp/a.png` at 80 ms steps, or the frame gate), the other cards arrive with it, dim and names come in; focus on Plex, `ctl switcher` lists the three apps `live` (or `video` for an input). On Home with the switcher closed, one Home tap opens it from Home (no shrink); a second tap closes it.
5. Pictures: each card shows its app's last picture, not black; a card of Live TV or an HDMI input shows its icon on a tile with "No preview". If an app's card is black or missing, `$T ctl windows` shows whether its window is still mapped while covered (the list is the mapped cards) and what buffer it holds.
6. Keys and pointer: Left/Right, Down to Quit and Up to Quit all, OK opens the card and the app is in front within a second (`foreground` in `ctl keys`), Back closes, the Magic Remote hovers and clicks; Back from the top of Home (with `remote.recents` on) opens the same switcher.
7. Quit: on a card, Down then OK: `ctl luna`/the journal shows `closeByAppId {id, reason: windowPolicy}`, `luna-send -n 1 luna://com.webos.applicationManager/running '{}'` no longer lists the app within a few seconds, the card dims, says Quitting and goes, the others slide to fill. Quit all asks first; Cancel changes nothing; Quit all closes every app, none of LumaTV's own.
8. The hold: Home held 650 ms with the switcher open and closed still falls back to LG's compositor (`escaped` marker), as `tv-gate.sh homekey` expects.
9. Frames: `tv-gate.sh frames` with the switcher opened and closed over Home and over an app (`Q:switcher-show` is in the trace): no refresh skipped, `renderUsMax` of the opening frames under 8 ms; closed, `ctl wake` shows the tick back at 10 a second within 3 s and `ctl switcher` says `Closed`.
10. Burn-in care: with `oledcare` on, the shell's chrome walks up to 3 px while the pictures stand; the ring is then up to 3 px off the picture's edge, which is accepted.

Pass: 3 to 9 hold; then `remote.appSwitcher` ships on.

## Known unknowns

- Whether a covered app keeps its window mapped with its last buffer on the set (LG minimizes the card a new card covers, `settle_cards`); the list is the mapped cards, as `listRecents` here always was. If a kind of app unmaps on minimize it is not listed.
- A window with subsurfaces is drawn from its root surface only.
- The first draw from a driver buffer of another process on Mali is warmed at its commit (`prepare`); a buffer still warming is skipped for that frame.

## TV round 2026-10-28 (round 14, `compositor/slot-2026-10-28-r14/img/switcher-open.png`, `sw-quitall-ask.png`)

`a1555df`, LumaTV's compositor, Netflix and Apple TV open (Plex and YouTube could not be: they crash under this compositor, see compositor-gate9-result.md round 14).
Off: Home twice goes Home and no switcher. On (`remote.appSwitcher`): `can` has `switcher` within 3 s; a single Home goes Home; Home twice within 0.4 s over Netflix opens the switcher
(`opened #1 over 2 app(s), netflix shrinking into its card`, `the shell placed the cards`), both cards placed; Apple TV's card shows its picture, Netflix's is black (video plane: INFO).
Right then OK opens Apple TV (in front within 2.5 s); Back closes; Down then OK on Quit sent `closeByAppId netflix` and the app left `running`, the other cards stayed; Up Up OK asks "Quit all"
with Cancel focused, Cancel changes nothing. Not run: pictures of Live TV/HDMI, the held-Home escape (the gate's `homekey` passes), the frame gate with the switcher.
Result: PASS for what ran; set back to off (the setting does nothing under LG's compositor), go-live with Main's word.
