# Overlay cards over any app (Overlays, 2026-10-28)

Built on the offline host and committed; nothing has run on the TV. The switch is `overlays.enabled` in the owner's `theme.json`, off by default, until the TV check below passes.

```
lumatv theme set '{"overlays":{"enabled":true}}'      # on: live within 2 s (Home reads theme.json every 2 s)
lumatv theme set '{"overlays":{"enabled":false}}'     # off: the cards go, plugin notifications are LG toasts again
```

## Resume here

- **What it is.** One column of glass cards in the top right, over Home and over every app: a doorbell (the camera pop-up of `ha-cameras-result.md` is its card), a timer or an alarm that finished, a Home Assistant alert, a score. Mocks `docs/mocks/overlays/` (`node render.mjs`: stack, alert, score, timer, alarm, doorbell, zones; dark and light, 100 % and 200 % text; its checks find no overflow, no overlap, no card below 810 px, every word at 4.5:1 and every glyph at 3:1 on the worst frame under the glass).
- **Pieces.** `payload/compositor/lumatv/Overlays.js` (the queue: classes, priority, limits, dismissals; pure, `tests/overlays.test.js`), `panels/Overlays.qml` (the surface, a Resident of `LumaOverlay`), `panels/OverlayCard.qml` (one card), `LumaOverlay.qml` (wiring, `takes`, do-not-disturb), `panels/Cameras.qml` (the pop-up's small changes below), `kit/PluginHub.qml` (what is posted goes to the cards, not to a toast), `compositor/shell/Shell.qml` and `imports/LumaShell/Link.qml` (`takes` in the report, the `syskey` event), the compositor's `shell.rs` and `input.rs` (the two keys), `payload/theme/theme.json` (`overlays`, every number).
- **The one finding.** Under LumaTV's compositor the camera pop-up asked for the keyboard focus (`wants`), and the focus moves to the system layer's window: every key that window does not take is dropped, so a pop-up cost the app its arrows (its comment said they went on to the app; that holds only under LG's key filter). The cards ask the compositor for OK and Back by name instead (`takes`), and the pop-up does the same: the app keeps the focus and every other key.
- **Not built:** see the end (the LumaShow integration's `notify` messages, the helper publishing `importance`).

## What shows, and in what order

| Class | Priority | Up for | A record that makes it |
|---|---|---|---|
| doorbell | 0 | 15 s | `cameras.popup` (the camera's card); a notification of a channel `doorbell` (the Home Assistant plugin's ring) |
| alarm | 0 | 60 s | a notification of a channel `alarm`; a card at the `ringing` priority |
| timer | 0 | 30 s | a notification of a channel `timer`; a ringing card whose icon is a timer |
| alert | 1 | 10 s | any other notification (Home Assistant's `ha` channel: a low battery, a leak) |
| score | 2 | 8 s | a notification of a channel `scores`, `sports`, or `kind: score`, with a `score` (two sides) or as words |

The numbers are `theme.overlays` (`classes.<name>.{priority,dwellMs,waitMs,cooldownMs}`); `Overlays.js` `DEFAULTS` mirrors them and a test holds the two equal. The records are the plugin broker's (`plugin.<id>.notifications`, cards at the `ringing` priority): the ones LumaShow's devices draw, so one plugin speaks to a Show and the TV alike. The fields a plugin may add (`kind`, `level`, `score`) are the proposal `overlays-lumashow-proposal.md`; the TV reads them where they are and otherwise derives the class from the channel's name and icon.

The rules (`Overlays.js`, one test each in `tests/overlays.test.js`):

- At most two cards (Lucent's toast rule is queue, never stack; the brief asks for one or two, so two is the limit and only of one class), and only of the most important class that has something to show: while a doorbell, an alarm or a timer is up, an alert or a score waits and comes when it has gone, if it is still in date (`waitMs`); a score never shows beside a score. The camera pop-up takes a slot and the top; a card goes under it if it fits.
- No card ends below 810 px (75 % of 1080): a second card that would not fit waits; the first always shows. At large text the pop-up's picture gives up height (never below 120 px) until its card ends above 810. Cards are 560 wide, 64 px from the right edge (Home's own margin, where its status bar and dock end; 96 until 2026-10-29, `overlay-layout-result.md`), from y 134: the pop-up's place, below the status bar's row.
- A card is up for its class's time, counted while it is shown; the record's own end (`expiresAt`) and the plugin ending it (`notify.end`) take it away at once. A card the person dismissed, or that had its time, is not shown again until its record changes; a score that moved while it is up has its time afresh (never more than three times over), and not again within 30 s of the last.
- A doorbell record and the camera's doorbell pop-up within 20 s are one card (the camera's); a record that came first and is up when the pop-up arrives is taken into it.
- A `low` importance record is the drawer's alone (needs the helper to publish `importance`); a `secret` one shows the channel's name and no words.
- What a plugin posted before the helper came up is not news (PluginHub's priming); a restart replays nothing.

**Do not disturb.** Nothing shows, and what is up steps aside (it keeps the time it had), while a PIN prompt, an alert or the input popup of the notifications, the keyboard (LumaTV's or LG's), the screensaver, a panel of the overlay (quick panel, picker, drawer, a banner with buttons), a prompt or layer of Home, or the camera's viewer has the screen, and while the screen is off (a ring in standby is a card when the TV is switched on, if it is still in date). The camera pop-up follows the same rule now (it only waited before; one that is up steps aside and comes back with the time it has left).

**Keys.** OK acts (the record's first button, `POST /v1/plugins/<id>/actions/<action>`; a card with none opens the notification drawer), Back dismisses, both the top card's and only once it has been up 600 ms (`armMs`: a press made as it arrived was meant for the app). The card says what OK does under its words (`OK Clear`, `OK Open`). The arrows, the digits and every other key are the app's. Under LumaTV's compositor the shell reports `takes: ["ok","back"]` while a card is up and armed, and `shell.rs` hands those two keys to it as `syskey` events (press, repeat, release) instead of to the app, whose keyboard focus never moves; a release goes where its press went (a press the app already has keeps its repeats and release; a press the layer took has its release even if the cards are gone). Over Home the keys come through the bus (`Bus.cameraKey`), as the pop-up's did. Under LG's compositor the key filter asks `wants` and `key()` answers per key.

**Cost.** Nothing runs while no card is up and none waits: no timer, no animation, the layer is not drawn (`live`). The two cards and the probe that measures a card's height are built at warm-up (8 s), and the first card is drawn once with letters and digits while the shell warms its layer, as the toast slot is. A card is one pane and its words, no blur (the glass samples the overlay's flat colour, at `readingAlpha` over an app); it fades and slides in 180 ms and out in 150 ms, and nothing moves after it has landed. One timer, set for the next moment something changes.

## Changes to the camera pop-up (HaCameras agreed)

`panels/Cameras.qml`: `popupBottom` and `popupShown(kind)`; steps aside while the host is busy; its hint is in the second ink (the third is 3:1 on the worst frame) and its two keys wrap whole; the line under the name may take two lines; the picture fits under 810 px at large text. `kit/CameraFrame.qml` had no line in `kit/qmldir` and no `import "."` (`tests/kit.test.js` failed on both): added. `LumaOverlay.qml`: `wants` still answers LG's filter; `wantsFocus` (panels, the viewer) and `takes` are what LumaTV's compositor is told.

## Tests (host, all green 2026-10-28)

- `bun test tests/` 210 pass (`tests/overlays.test.js`: 22, the queue).
- `cd compositor && cargo test` all pass (`tests/overlay_keys.rs`: 9, the two keys: taken and paired, the arrows stay the app's, a press the app has keeps its release, a layer with the focus is not offered them, only OK and Back can be asked for); `cargo clippy --all-targets` clean for these files; `cargo fmt --check` clean for them (it still names `scene.rs`, `window.rs` and the `ALLOWED_METHODS` line of `shell.rs`, not mine).
- `cd tests && python3 -m unittest test_overlays` (16: a notification is a card within a moment and no toast; the keys are the card's only after `armMs` and only OK and Back; Back dismisses and a record is not shown again, OK runs the record's button; two cards at most, the most important first, none below 810; stacked at the right gap; held while the host is busy and back with the time it had; the pop-up on top with a card under it; an alert waits under the pop-up; a doorbell record and the pop-up are one card either way round; a record the plugin ends takes its card; a score; the door makes a card by hand; nothing with the switch off; over Home), `test_shell_cards` (3: the client asks for OK and Back once armed and never for the focus, a `syskey` is the card's key, nothing asked with the switch off), `test_camera_popup` (6, unchanged), `test_shell_overlay` (9), `python3 -m unittest tests.test_overlays_contrast` (arithmetic: the full ink 7:1, the second 4.5:1, each tone's glyph 3:1, both materials, the worst frame under each glass), `python3 tests/qmlhost/compile_all.py` 0 failed, `test_cutover` and `test_gate` green.
- Scenes (`python3 tests/qmlhost/scenes.py --only overlays-stack,overlays-alert,overlays-score,overlays-timer,overlays-alarm,overlays-alerts[,-200] --out DIR`): dark and light at 100 % and 200 % text over the film still of the mocks, the layout audit at both sizes, every card measured (above 810, inside the safe area, below 134).

## TV check list (not run; needs Main's slot)

An app with subtitles in front (Netflix playing a title, the frame probe running), `overlays.enabled` set, the helper running. The door is `echo '<json>' > /tmp/lumatv-overlay` (read once a second while an app is in front; the same text twice is not shown twice, so a new `id` or text each time); keys through `compositor/tools/tv-uinput-key.py` or the remote. `ctl shell` reads `takes`, `wants`, `systemLive`, `ui.cards` and the journal line `shell: the system layer takes ok and back for its cards`. Stop at the first failure and send back what is named.

1. Off, then on: with the switch off, `{"card":{...}}` shows nothing and a plugin notification is an LG toast; on, `ctl shell` says `ui.cards.on` true and `takes` `[]`.
2. A card over the film: `{"card":{"class":"timer","title":"Pasta","body":"Time's up","source":"Kitchen Show","ttl":60}}`. Within a second the card is top right, from y 134, ends above 810 (a capture: the film's subtitles untouched, the volume bar's place free); the film keeps playing without a stutter (the frame probe: no late frame beyond the baseline while the card fades in and while it is up). Record the first frame's time (the journal's `shell: system layer live; its last commit N ms ago`) and run the frame gate over the film and over an input: `python3 docs/research/perf/gate.py --surfaces overlay-netflix,overlay-hdmi` (it sets `overlays.enabled` for the run, makes a card through the door each cycle, dismisses it with Back and puts the switch off again; `kb.py` `!overlay`; the first-frame check is off for it because the door is read once a second).
3. Keys: for 0.6 s after it arrives OK goes to the app (Netflix shows its controls; `takes` still `[]`); then `takes` is `["ok","back"]`; Left and Right seek in the film (the app, not the card); OK dismisses the card and opens the drawer (a card with no button), the film is not paused by that OK; a new card and Back: dismissed, Netflix did not leave the title; a held Back still exits the app after 1 s as before.
4. Order: an alert (`{"card":{"class":"alert","title":"Low battery","body":"Back door sensor","source":"Home Assistant"}}`), then a doorbell (`"class":"doorbell"`): the doorbell shows, the alert waits and comes when the doorbell is dismissed. Two timers: both show, the second 16 px under the first, and a third waits.
5. Do not disturb: with a card up, raise an LG alert (`luna-send -n 1 luna://com.webos.notification/createAlert '{"sourceId":"com.webos.app.home","title":"An alert","message":"Closed with OK.","showTimeout":false,"buttons":[{"label":"OK"}]}'`), then the keyboard (a search field): the card steps aside each time and returns when they go, with the time it had left; a PIN prompt likewise.
6. A score: `{"card":{"class":"score","title":"Lakers 102, Celtics 99","body":"Q4 2:31","score":{"league":"NBA","status":"Q4 2:31","live":true,"away":{"name":"Lakers","score":102},"home":{"name":"Celtics","score":99}}}}`: the two lines, the leader in the full ink. Another with new numbers within 8 s refreshes the card; one within 30 s after it goes is not shown.
7. The camera pop-up (`cameras.enabled`, `ha-cameras-result.md` check 4): `POST /v1/cameras/event` with a doorbell: its card from y 134 and a door card (timer) under it if it ends above 810; a door alert waits under it. The arrows reach the app while it is up (HaCameras check 5, which the focus bug would have failed); OK expands to the full view, Back dismisses; a door `doorbell` card within 20 s of the pop-up is not a second card.
8. Real records, with the Home Assistant plugin linked: `persistent_notification.create` in Home Assistant is a card and not an LG toast; a doorbell ring is one card (the camera's when it shares the doorbell's camera), OK on the plugin's `Clear` clears it in Home Assistant.
9. Over Home: the same card at the same place; the D-pad moves Home's focus, OK and Back are the card's (through the bus); with Settings open nothing shows until it closes.
10. Cost: the gate above, and `ps` and the compositor's CPU with no card (unchanged from before), with a card up, and 5 minutes of Netflix with a card every 30 s through the door: no dropped frames beyond the baseline, `Rss` of the shell client flat.
11. Off again: `lumatv theme set '{"overlays":{"enabled":false}}'`: the cards go within 2 s, `takes` `[]`, toasts are back.

## Not built

- The LumaShow integration's `notify` messages (`lumashow.notify`, its persistent-notification option) reach the TV's bridge and are dropped (`cameras.rs` `LinkSink::notices`); the `Notice` has no `level` or `ttl`. Making them a first-party plugin's notifications (which the cards and the drawer both draw) is the change in `overlays-lumashow-proposal.md`, last section; today a Home Assistant alert is a card when the Home Assistant plugin is linked (Nitin's steps in `ha-plugin-result.md`).
- The helper does not publish a channel's `importance` with its records, so a `low` channel is a card until it does (about ten lines in `plugins/session.rs` `note_json`, and the fixture).
- No producer of timers, alarms or scores exists on the TV: a plugin (or the integration) posts them as records of those kinds. The mock cards for them are shown with the door.
- LG's own toasts draw in the same corner (from y 24, above the cards): two or three at once can cover the first card for their 5 s.
- A Settings row for the switch; the pointer (the Magic Remote cannot click a card: the cards take no pointer); sound.

## Changelog line

Overlay cards over any app: a doorbell (the camera pop-up), finished timers and alarms, Home Assistant alerts and live scores in one queue in the top right, two at a time at most, held while a PIN, alert or keyboard is up, never below 75 % of the height so subtitles stay clear; OK acts, Back dismisses, the arrows stay the app's (the compositor takes OK and Back by name; the camera pop-up no longer takes the app's keyboard focus). Off by default (`overlays.enabled`) until its TV check passes.

## TV round 2026-10-28 (round 14, `compositor/slot-2026-10-28-r14/img/card-over-netflix.png`, `alert-doorbell.png`)

`a1555df`, LumaTV's compositor, `overlays.enabled` on, Netflix playing. Steps 1 to 4 and 6: a timer card top right from y 134, subtitles untouched, `takes` `["ok","back"]`; Back dismisses it
and Netflix stays; an alert then a doorbell: the doorbell shows first; a score card with the leader in full ink. Not run: 5, 7 to 10. Result: PASS for what ran; switched off again, go-live with Main's word.
