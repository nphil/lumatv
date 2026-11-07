# Volume and mute display: LumaTV's own, the only one

## Resume here
Current source note (2026-10-31): the approved native volume layout renders the original RGBA audio PNG glyphs left of the number and bar; PCM, unknown formats and mute have no fake fallback. The existing helper path, freshness handling, module path and format mapper were already present. Host-native rendering proves the assets decode and render, but does not diagnose or verify the missing glyph on the actual TV. These UI changes have not been installed or accepted on the TV. See `rust-shell-core-result.md` for native input and evidence details.

The following resume state is historical.


State at 2026-10-25 22:35 (the cut for TV round 8): the display is a slim bar at the bottom centre (Nitin, 22:20), in the repo and offline-tested. The set
still shows the right-edge pane of `ddc293c` until the round's `lumatv apply`. The first deploy took two restarts: the first, with files copied unrendered,
left Home and the overlay unloaded for about two minutes (see Deploying by hand). `lumatv revert` is the way back and brings LG's popup back with it.
In: the bar, the key moving the level at once, the format mark (display and mapper). Not in: the helper does not publish `soundMode` and `signal` yet, so
the mark stays hidden on the set (see The format mark). Open: frame cost over video on the set; the TV's own volume with the receiver off.

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
- **Whose volume**: with the sound over ARC or optical and the receiver on and answering, the receiver's own level (0 to 98 in steps of 0.5,
  rounded to a whole number whatever the Denon's display is set to: no decimals, no unit), not webOS's counter, which moves by one per key (31, 32, 31 on Volume+,
  Volume-) and says nothing of the Denon. With the TV's speakers, headphones or Bluetooth as the output, or with the receiver off, unreachable or
  switched off in Settings, the TV's own volume (`LS.audioService.volume` of 100). Nothing on the bar says which of the two it is. A receiver that
  may answer and has not yet (a key after minutes of quiet) is waited for 250 ms; the TV's counter is shown only if it does not. A key in that wait
  counts from the level of the display's aged copy and shows once the answer comes, so the first press of a hold after a quiet spell is a step too.
- **The key answers first, and the receiver follows the number**: the bar appears on the key's own frame (no fade in, 150 ms out) with the level as a target. A tap
  moves it one receiver step (0.5, as read on the set: 42.5 to 43 to 42.5); a held key moves it by time, not by repeat: 6 steps a second, 4 more each second held,
  at most 16, never past the receiver's own limit (`volumeMax`, from its `MVMAX`). LG's audio service passes each of the 25 repeats a second on over CEC and the
  Denon takes them slower and drops the rest, so a count per repeat ran ahead of the receiver and jumped back at release (Nitin, 2026-10-28). The target is posted
  to the helper at most every 100 ms (`POST /v1/receiver/volume`); `receiver_volume.rs` sends it as an absolute `MVxx`/`MVxx5` over the web port, one command in
  100 ms at most and only the newest level; a command that fails goes again.
- **Late CEC steps** (measured on the set, 2026-11-05: four 1 s holds and six taps, keys, posts, commands and the topic on one clock): the key reaches the helper
  8 to 11 ms after it is pressed and the receiver reports the level sent about 40 ms later, but the Denon takes the key's own CEC step 120 to 300 ms after
  it, usually after the level sent. Each tap overshot by one step until the helper's read-back 400 ms later, and a hold's last steps landed up to 300 ms after
  the release, after the receiver had reported the bar's last level. The bar took the reading at that report, so three of the four holds showed the late step
  and then the correction: 36 then 36.5 after a hold down, 40 then 39.5 after a hold up. The bar now keeps the target until the receiver has reported it after
  the last post with nothing else for 500 ms (`quietMs`), or 1.5 s (`settleMs`) after the last key whatever the receiver says; a report from before the last
  post, or of another level, never shows meanwhile, and a change made at the receiver shows after it. The helper hears every reading: another level within
  1.5 s of the last one asked (`volumeHoldMs`), after the receiver reported that one, is a late step, sent back once no level has been asked for 400 ms
  (`volumeSettleMs`; sooner, a hold's next level only overtakes it), at most 3 times; a level never reported is read back after 400 ms. Nothing heavy runs on a
  press: no blur is rebuilt, hidden text is not laid out (the format mark and the glyph are `visible` only when used), the fill animates for 100 ms.
  Offline, from the audio service's report to the next presented frame: see `volume_sheet.py --latency`.
- **The receiver's volume keys are the display's alone** (2026-11-06): the read-back put the late step back but never stopped it. All 12 taps of a check
  reached the bar's level 47 to 116 ms after the key, then LG's CEC step took the Denon one step past it 110 to 270 ms after the key, and the helper put it
  back 450 to 680 ms after (`/data/home/tmp/lumatv-regress-20261106/vc-on/`). The display now says in its report when the receiver takes the keys
  (`volumeKeys: "receiver"`: the sound over ARC or optical, the receiver on and answering with a known level, neither the TV nor the receiver muted), and
  while it does the compositor gives Volume+ and Volume- and their repeats to the display alone, numbered, in the form of the audio service's report, and
  never calls the service: no CEC step goes out, and the level posted is the only one. The display answers with the newest number it counted
  (`volumeAck`); a key it has not answered within 250 ms (the display hangs, or the helper or the receiver plugin went away under it) goes to the service
  after all, and the service's report of it names the key (`keyReturned`) so the display shows it without counting it again. Mute stays the service's,
  and so does a volume key while either side is muted: the service's key clears the mute on the TV and the receiver, and that one key can still
  overshoot until the read-back.
- **Mute**: the number gives way to a slashed speaker and the bar is dimmed; no word. From the receiver's `MU` push or, with no receiver, the TV's mute
  (and at once from the key when the TV's `cause` says mute). Then a 64 px bubble with a slashed speaker stays while the sound is
  muted, over an app in the top corners (x 96 from either edge, y 134 below the status bar, or 94 px further in and down; never the lower third, where Netflix draws
  its subtitles, nor the middle), over Home and its saver along the status bar's row (y 52, x 560 to 1360: Home's corners are the clock and weather widgets), visiting the four places in turn every 10 s so no pixel holds a still mark, as LG's bubble did. TV round 11 showed the
  first form, at the lower middle, on the subtitles (`round9/r11-2026-10-26/netflix-mute-hop.png`); `11-volume-osd-mark-netflix` renders the four places over that capture. After a restart the first copy of the receiver topic can carry a stamp older than the display accepts (the helper moves a stamp quietly), so
  the display asks the helper for the newest copy when the helper comes up and when a copy arrives aged, and once more 2.5 s later; without it a mute that
  was on before the restart showed no mark until the next change. The receiver's mute is kept while the
  helper keeps reading (the display asks for the newer stamp each time its copy ages out) and is dropped 1.5 s after the readings stop.
- **Shape**: a pill 440 px wide, or as wide as the number, a 140 px bar and the format mark need, 56 to 64 px tall (the label type
  and 12 px above and below), 64 px above the bottom edge, centred; at 200 % text it grows in both. The number in the label size, semibold, in a slot as
  wide as "100" so the bar does not move with the digits; the bar 8 px thick, filled from the left in the ink; the format's mark at the right
  end, in the ink at full strength (a fainter ink would not hold the contrast floor), 28 px tall (the label size, 26, times 1.08, so it follows the text size)
  and no wider than 196 px (7 times that), which draws Dolby Audio, nine times as wide as it is tall, at 22 px; no word, and no mark and no gap for a format that has none. No caption, no icon tile (it read as a button), nothing to focus.
- **Over video**: the glass is painted at `readingAlpha` (theme.json, 0.8; over Home 1) so the picture shows through the fill; the 1 px edge and the
  shadow keep their weight (`Glass.fillAlpha`). Against the worst frame, white under the dark glass and black under the light, blended in the
  plane's own values, the ink is 9.2:1 (dark) and 11.4:1 (light), the bar 5.7:1 and 9.0:1 against its track; 7:1 (Lucent's distance contrast)
  would hold down to 0.73 and 0.63. `tests/test_osd_contrast.py` holds the number and measures the same on the rendered picture.
- **Fixed on the way**: the `LS` singleton is in `WebOSCompositorBase` on the set (WebOSServices has none), so a file that imports only
  WebOSServices fails with "LS is not defined" (the log shows it at `Layers.qml:119`, the Settings launch); the offline host's stand-in puts `LS`
  in WebOSServices and hides it. `kit/TvVolume.qml` imports both. Files that still use `LS` with the one import: `Layers.qml`, `LumaOverlay.qml`,
  `panels/QuickPanel.qml` (its volume nudge), `panels/Screensaver.qml`, `kit/Sio.qml`, `kit/Power.qml`, `kit/PluginHub.qml`, `kit/NoteLog.qml`,
  `kit/Apps.qml`.

## The format mark

The Denon says what it plays in words. Read-only queries answered by the S760H on 2026-10-25 (nothing was playing): `MS?` gives `MSDOLBY AUDIO-DSUR` and a
status batch (`MV`, `MVMAX 81`, `PS…`, `MU`, `SI`); `SSINFAISSIG ?` gives `SSINFAISSIG 13` and `SYSDA PCM ZERO` (the input signal in words); `PW?` adds
`SYSMI Dolby Audio - Dolby Surround` (the mode in words); `SSINFAISFSV ?` gives `NON`; `OPINFASP ?` gives a channel map of digits. The HTTP
`AppCommand` `GetSurroundModeStatus` answers an empty list. `kit/AudioFormat.js` maps `SYSDA` (the format) and `MS` (the mode word) to "Dolby Digital+ ·
Surround", "Dolby Atmos", "DTS:X", "PCM 2.0", "Multi Ch In" and the like, and to nothing when they say nothing ("PCM ZERO", an unknown text); no raw code is
shown; `logo` maps the same two texts to the mark the bar draws, or to nothing. Seen on the set: "PCM ZERO" with "DOLBY AUDIO-DSUR" (idle) and, over Netflix on 2026-10-26, `signal` "Dolby Atmos - DD+" with `soundMode` "DOLBY ATMOS" (the
stream's name is `<format> - <carrier>`; the bar draws the Dolby Atmos mark). The other patterns follow the receiver's published naming and have to be confirmed
while something plays (`tests/test_audio_format.py`). The bar shows the mark only with the receiver on and fresh, not muted.

Why marks and not words: the words ("Dolby Digital+ · Surround", "DTS-HD · Stereo") made the bar busy (Nitin, 2026-10-26); the recognisable marks say the same in
less. Four marks cover what the receiver names: Dolby Atmos; Dolby Audio, for Dolby Digital, Digital+ and TrueHD; DTS:X; DTS, for DTS and DTS-HD (Master
Audio too). PCM, plain stereo, Multi Ch In and what is not known show nothing at the end of the bar. No IMAX Enhanced mark: no text the receiver has been read
saying names it, so `AudioFormat.js` has nothing to detect it by; it joins when one is read. The words stay as the mark's accessible name (`LuLogo.label`).
The files are `icons/audio/<key>.png` (`atmos`, `dolby`, `dtsx`, `dts`), white on transparent like the Lucide icons, drawn by `kit/LuLogo.qml` in the bar's ink so
they follow the dark and light glass and the contrast; `generators/gen-audio-logos.py` makes them from the Wikimedia Commons sources kept in
`generators/audio-logos/` (a private build: the marks are their owners' trademarks), 112 px tall, 4 times the 28 px drawn, for a 4K panel and 200 % text.

Not done: the helper (`helper/app/src/receiver_telnet.rs`) has to carry two more lines into the `receiver` topic: `soundMode` (the last `MS<text>` heard, asked
with `MS?` on connect) and `signal` (the `SYSDA` text, asked with `SSINFAISSIG ?` on connect and on a mode or input change). Until it does, `rc.soundMode`
and `rc.signal` are absent and the text is hidden.

## TV check for this slice (round 8)

1. After `lumatv apply`: the compositor log has no `LumaOverlay failed`, no `[Resident] overlay volume did not load`, no ReferenceError from `VolumeOsd`,
   `TvVolume` or `AudioFormat`.
2. Home, then Plex's menu, then HDMI 2: Volume+ once, Volume- once (note the receiver's level first through Home Assistant or `/v1/receiver`). Pass: a slim
   bar at the bottom centre appears at once (capture within 0.3 s of the key), the number moves by 0.5 and back, LG's popup never shows, the log has
   `onVolumeUpdated, volume view is not available` once per press and `[VolumeOsd] cue: the bar appears now` within a few ms of it, the receiver ends where
   it began. Over Plex and HDMI 2 the picture shows through the bar and the number reads.
3. Hold Volume+ for about three seconds (receiver muted or low first). Pass: the number climbs at a steady, gently rising pace (about 3 a second at the start, 8 at
   the most) and the Denon's own display keeps up with it; at release the number holds and never steps back, the Denon ends on it (a late CEC step is put
   back within about half a second) and the bar takes its reading (`/v1/receiver`, `volumeAbs`), the same number. Holding past the receiver's limit stops
   the number at the limit.
4. Mute at Home (then unmute at Home, with apps closed first). Pass: the number becomes a slashed speaker with a dim bar, no word; after the hold a 64 px
   bubble stays in a top corner and has moved to another place within ten seconds; unmuting removes it; LG's bubble never shows.
5. Format: with the helper that publishes `soundMode` and `signal`, play something with a known format on the Denon and check the mark against the Denon's front display.
6. Latency: the key injector prints its epoch time (`kb.py`); the compositor log line `[VolumeOsd] cue` carries its own. The difference is key to cue (the TV's
   report adds 44 to 63 ms); the bar is drawn on the frame after the cue. A capture 16 ms after the key cannot be had from the capture service (one grab takes
   about 0.9 s), so the frame time comes from `lumatv fps on` (`[FPSJANK]` lines must not appear around a press).

## Tests and renders

- `tests/test_volume_patch.py`: the patch against LG's own file; LG's view hosted offline (`tests/qmlhost/volumeview.py`) opens its popup on a
  change, stock, and never does, patched, for every sound output and for the country path.
- `tests/test_volume_osd.py`: sources, the wait, the TV's own volume, the mute and its mark, readings that age out, over video and over Home, no
  key, no focus, no second display.
- `tests/test_osd_contrast.py`: the arithmetic and the pixels, both materials, on a white and a black frame; the level and the Dolby Audio, Dolby Atmos and
  DTS:X marks each at 7:1.
- `tests/test_audio_format.py`: the words and the marks, and the mark files against `icons/audio/index.json`.
- `python3 tests/qmlhost/scenes.py --only 11-volume-osd,11-volume-osd-db,11-volume-osd-muted,11-volume-osd-mark,11-volume-osd-tv,11-volume-osd-tv-arc,11-volume-osd-offline,11-volume-osd-home`
  renders dark and light; mock 11 (`docs/mocks/11-volume-osd.html`, shots in `docs/mocks/shots`: `-atmos` over a dark film frame, `-dtsx` over a bright one,
  `-format` for Dolby Audio, `-stereo` with no mark, each with `-light`) is the design. `python3 tests/qmlhost/volume_sheet.py` renders the host's own
  contact sheet over Home and over film, bright and game frames (`volume-osd/renders/`).

## On the set (2026-10-25, 21:36 to 21:45)

- Home, Plex (its menu, nothing playing) and HDMI 2 (the Apple TV's profile screen through the Denon), Volume+ then Volume- on each
  (`volume-osd/after-home.png`, `after-plex.png`, `after-hdmi2.png`): LumaTV's pane only, at the right edge, vertical, the picture showing
  through it over Plex and HDMI 2; LG's popup never showed, and the log has `onVolumeUpdated, volume view is not available` once per press. The
  receiver read 42.5, not muted, before and after each pair (Home Assistant: 0.425).
- Mute at Home (`after-home-muted.png`, `after-home-mark.png`): the pane says "Muted" and "was 42.5", then the mark stays at the right edge and has
  moved to another place 14 s later; the unmute (also at Home) took the mark away and the receiver read unmuted. LG's bubble never showed. The
  mark on the set is the first form, a speaker over the word "Muted" in a 128 px chip; the layout audit flags that stack (the words sit 26 px below
  the middle of the box), so the repo's mark is the icon-only bubble, which shows after the next restart.
- Contrast on the captures (ink against its own background, 7:1 is the distance floor): the level number 9.6:1 over Home, 12.4:1 over a bright Plex
  picture, 17.4:1 over HDMI 2; the line under it 10.5:1, 12.7:1, 17.4:1.
- On HDMI 2 the Denon draws its own volume bar into the picture (the level, a bar, "ECO : OFF" at the bottom). It is in the receiver's video, not LG's,
  and only the receiver's own setup menu can switch it off.
- The log shows `LS is not defined` at `kit/NoteLog.qml:57` and `kit/Apps.qml:32` at start (the defect above); nothing from the display's files.
- Not checked on the set: frame cost of the pane over video (`lumatv fps on`, `[FPSJANK]`), the first press after a boot, and the TV's own volume with
  the receiver off (the offline scenes `11-volume-osd-tv*` cover it).

## Deploying by hand

The package build fills the brand placeholders (`{{qmlModule}}`, `{{dataDir}}`, `{{appId}}`, `{{shortId}}`, `{{displayName}}`) in every `.qml`, `.js`,
`.json`, `.css`, `.html`, `.sed` and `.md` file (`render` in `tools/build-ipk.sh`). A file copied from a worktree to the set unrendered leaves Home and the
overlay unloaded (the log shows a `%7B%7BqmlModule%7D%7D` path); run the same substitution on the copies, then `lumatv apply`.

## Not done

- A sound-output change (Settings, Sound out) does not itself open the pane; LG's popup did not either.
- Audio guidance: LG spoke the volume through the same view (`speakVolume`); with the view not available it is silent. The set has it off.

## TV round 2026-10-28 (round 14, `compositor/slot-2026-10-28-r14/img/vol-home-up.png`, `vol-netflix-up.png`)

`a1555df` (1ae1234 in it), LumaTV's compositor, receiver on and muted for the round. Volume+ and Volume- over Home: the bar at the bottom centre, `61`. Over Netflix playing (Dolby Vision
test title): the bar sits about 120 px higher, clear of the subtitles (they stay at the lower edge), with the Dolby Atmos mark. PASS. Not run: hold, mute, latency.
