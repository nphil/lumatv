# Volume and mute display: LumaTV's own, the only one

## Resume here

State at 2026-10-25: built and tested offline (`tests/test_volume_osd.py`, `tests/test_osd_contrast.py`, `tests/test_volume_patch.py`); not
yet on the set. It takes one compositor restart (`lumatv apply` rebuilds the shadow module and restarts the compositor once); `lumatv revert` is
the way back and brings LG's popup back with it. Checks to run on the set after the restart are at the end.

## What LG draws and how it is stopped

One view: `WebOSCompositor/views/StarfishVolumeView.qml` (in `views/StarfishViewsRoot.qml` above the fullscreen, overlay, notification, voice
agent and nudge views). It hosts `views/volume/StarfishVolume.qml` (the level bar and its number or, on an ARC output, a speaker with a highlighted
+ or -; on this set `soundOutput` is `external_arc` and `volumeSyncable` false, so the speaker form) and `views/volume/StarfishVolumeMute.qml` (a
96 px bubble that stays while the TV is muted and moves 94 px every 10 s). Both open from
`Connections { target: LS.audioService; onVolumeUpdated: { if (available) updateVisibility() ... } }`, and `available` is the view's own switch:
the power-off state, the hotel-mode setting and the configd key `com.webos.surfacemanager.volumeOSD`, which `Settings.subscribe` keeps live.
The mute bubble starts from `volume.onClosed`, which asks `available` too; one path (the country setting changing to Ukraine) calls
`updateVisibility()` without asking. Multi View's window decoration has a volume bar of its own (`StarfishServerDecoration.qml`, `volumeUI`),
drawn only in Multi View, which the set does not use. Nothing else of LG's draws a volume or a mute.

The compositor loads this view from the shadow module: its compiled-QML cache on the set names
`/var/lib/lumatv/qml/WebOSCompositor/views/StarfishVolumeView.qml` and `views/volume/StarfishVolume.qml` as the sources (the key filter's
controller stays LG's, as HANDOFF says). So `build_shadow` patches the shadow's copy: `tools/volume_patch.py` puts `false &&` in front of
`available` and makes `updateVisibility()` return when it is not available. The set answers each change with LG's own
"volume view is not available" line in `/var/log/messages`, once per change. Three anchors, each found exactly once, else the tool refuses and
LG's file stays (the shadow then shows LG's popup beside LumaTV's display until the patch follows the firmware). The set's stock file is
byte-identical to the census copy (md5 `c46615d0b503802680850252548ba5a3`, webOS 10.2.1-5202).

Why not the configd key `volumeOSD = false`: it is a persistent setting of the TV, so it would outlive the shadow module that carries LumaTV's
display; a TV that fell back to LG's module (a failed apply, the stock compositor) would have no volume display at all. The patched view exists
only in the shadow module, so LG's popup is back whenever that module is not the one loaded, and it goes off in the same restart that brings
LumaTV's display. Nothing is written to configd.

Before, on the set at 20:3x (`volume-osd/before-up.png`, `before-down.png`): one Volume+ and one Volume- drew LG's speaker with a highlighted
+ / - at the right edge and, as Home's own pill, LumaTV's old display at the lower left, both at once.

## What LumaTV draws

`panels/VolumeOsd.qml`, held by `LumaOverlay.qml` at z 3, the layer that is always up above Home and above every app, HDMI input and Live TV (the
container's overlay Loader is no longer hidden while Home is in front; the overlay's banners and drawer still wait for an app). Home's layers hold no
second one (a test fails if they do). The display takes no key and no focus.

- **When**: the audio service reports a change (`kit/TvVolume.qml`, LG's own `volumeUpdated`: a volume key, a mute, Home Assistant, an app), or the
  receiver topic reports a new level or mute (the Denon's own remote too). On the set the report comes 44 to 63 ms after the key, with `cause`
  `volumeUp` or `volumeDown`, whatever is in front, so no key plumbing is needed over an app. It goes 2.5 s after the last of them.
- **Whose volume**: with the sound over ARC or optical and the receiver on and answering, the receiver's own level in its units (0 to 98, or
  decibels split into the number and "dB"), not webOS's counter, which moves by one per key (31, 32, 31 on Volume+, Volume-) and says nothing of
  the Denon. With the TV's speakers, headphones or Bluetooth as the output, or with the receiver off, unreachable or switched off in Settings, the
  TV's own volume (`LS.audioService.volume` of 100), captioned with the output (TV, ARC, Headphones, Bluetooth). A receiver that may answer and has
  not yet (a key after minutes of quiet) is waited for 250 ms; the TV's counter is shown only if it does not.
- **Mute**: the word and what it was ("was 46"), the level dimmed, from the receiver's `MU` push or, with no receiver, the TV's mute. Then a mark
  stays at the right edge while the sound is muted (LG's bubble did), moving 94 px between three places every 10 s so no pixel holds a still mark.
  The receiver's mute is kept while the helper keeps reading (the display asks for the newer stamp each time its copy ages out) and is dropped
  1.5 s after the readings stop.
- **Shape**: a pane 176 px wide (wider when the words need it, at 200 % text too), 64 px from the right edge, vertically centred, 28 px radius;
  level in the title size, the line under it in caption size, a 12 px track 216 px long filled from the bottom in the ink, a 64 px speaker tile in
  the accent while the sound is on. Lucent's catalogue row for the volume OSD: compact overlay, lateral safe region, no focus.
- **Over video**: the glass is painted at `readingAlpha` (theme.json, 0.8; over Home 1) so the picture shows through the fill; the 1 px edge and the
  shadow keep their weight (`Glass.fillAlpha`). Against the worst frame, white under the dark glass and black under the light, blended in the
  plane's own values, the ink is 9.2:1 (dark) and 11.4:1 (light), the bar 5.7:1 and 9.0:1 against its track; 7:1 (Lucent's distance contrast)
  would hold down to 0.73 and 0.63. `tests/test_osd_contrast.py` holds the number and measures the same on the rendered picture.
- **Fixed on the way**: the `LS` singleton is in `WebOSCompositorBase` on the set (WebOSServices has none), so a file that imports only
  WebOSServices fails with "LS is not defined" (the log shows it at `Layers.qml:119`, the Settings launch); the offline host's stand-in puts `LS`
  in WebOSServices and hides it. `kit/TvVolume.qml` imports both. Files that still use `LS` with the one import: `Layers.qml`, `LumaOverlay.qml`,
  `panels/QuickPanel.qml` (its volume nudge), `panels/Screensaver.qml`, `kit/Sio.qml`, `kit/Power.qml`, `kit/PluginHub.qml`, `kit/NoteLog.qml`,
  `kit/Apps.qml`.

## Tests and renders

- `tests/test_volume_patch.py`: the patch against LG's own file; LG's view hosted offline (`tests/qmlhost/volumeview.py`) opens its popup on a
  change, stock, and never does, patched, for every sound output and for the country path.
- `tests/test_volume_osd.py`: sources, the wait, the TV's own volume, the mute and its mark, readings that age out, over video and over Home, no
  key, no focus, no second display.
- `tests/test_osd_contrast.py`: the arithmetic and the pixels, both materials, on a white and a black frame.
- `python3 tests/qmlhost/scenes.py --only 11-volume-osd,11-volume-osd-db,11-volume-osd-muted,11-volume-osd-mark,11-volume-osd-tv,11-volume-osd-tv-arc,11-volume-osd-offline,11-volume-osd-home`
  renders dark and light; mock 11 (`docs/mocks/11-volume-osd.html`, shots in `docs/mocks/shots`) is the design.

## On the set after the restart (not yet run)

1. `lumatv apply`; `/var/log/messages` shows no `LumaOverlay failed` and no `[Resident] overlay volume did not load`.
2. Home, Plex and HDMI 2, Volume+ then Volume- on each: LumaTV's pane only, at the right edge; `volume view is not available` twice in the log; the
   receiver ends where it began (`curl http://127.0.0.1:18731/v1/receiver`).
3. Mute: the pane says Muted, the mark stays after it, moves in ten seconds, goes with the unmute; LG's bubble never shows.
4. The glass over a bright and a dark picture on HDMI 2 (`luna-send ... capture/executeOneShot`): the numbers above should hold on the plane.
5. Frame cost of the pane over video (`lumatv fps on`, `[FPSJANK]` lines) and the first press after boot.
6. Receiver off (or `base.receiver` switched off in Settings): the TV's own volume shows.

## Not done

- A sound-output change (Settings, Sound out) does not itself open the pane; LG's popup did not either.
- Audio guidance: LG spoke the volume through the same view (`speakVolume`); with the view not available it is silent. The set has it off.
