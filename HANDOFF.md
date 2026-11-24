
## 2026-11-23 helper + UI quiet install (13cd5621)

Installed with Netflix in front (live swap, compositor untouched): helper 83bc5e2d -> 293bb019 (restarted, plugins connected, audio-passthrough route answers, idle CPU unchanged), lumatv-ui 24ae7126 -> 68b26d09 (shell client restarted). Prewarm held ("Home not idle"). Not tested on the set: dock card lift, Restart confirm focus, frame gate, Keep apps ready row, app open times, tuner takeover toast and Live TV close.

## 2026-11-23 screen checks on the set (main 70283ab3)

Installed: helper 293bb019 -> 22a19fd1 (payload and bin; restarted, compositor untouched, 18 plugins connected). UI 68b26d09 and compositor 3ba09192 unchanged. The helper's tuner tests (132) and prewarm tests (13) passed at 022b13a4.

On the set: dock card lifted and Restart LumaTV confirm focus match the renders (`ui-focus-tv-*.png`). 120 Hz gate, 3 cycles each: home-dpad, home-widgets and the new `system-confirm` surface (task manager, System, Restart LumaTV confirm, Right/Left, Back out; never confirmed) dropped 0 frames, burst over 20 ms 0. Settings > System > General has Keep apps ready (On), order Marquee, Netflix, Prime Video, Apple TV, Live TV. Prewarm from cold launched Netflix, Prime Video and Live TV in the background with Home in front and no audio pipeline; Apple TV and Marquee are skipped by design. Tuner: two clients on one RF (2.1 and 2.2) shared at about 1.1 and 0.25 MB/s, one leaving did not interrupt the other; a client on another RF ended both with a clean EOF and `displaced` named it; with Live TV in front the toast "TestD is now using the tuner" showed and Live TV closed. The takeover landed on the LumaTV app's own page; the helper now launches `com.webos.app.home`, which shows LumaTV's Home (checked after the install). Two clients from one IP replace each other by design.

## 2026-11-23 helper install (main f1dff184)

Installed on the set: helper 22a19fd1 -> 6e05f76d (live swap, compositor untouched), carrying the TrueHD un-mix order and `POST /v1/audio-passthrough/flowing` (answers "no session is armed" with none armed) and the tuner fixes of 70283ab3. UI bdd2884c (from 9057b877) was installed at 21:20; compositor unchanged. audio-passthrough, voice-assistant and live-tv-tuner connected.

## 2026-11-23 weather install (main e85b765a)
Installed on the C4: helper 6e05f76d -> 696e735f, lumatv-ui bdd2884c -> 686031e6. Profile nitin: clock.weekStart monday, layout.statusBar.date centre. Capture home-tv-weather-calendar.png: month grid, centred date and week list shown. Untested: weather day open/close and the 120 Hz frame gate (gate refuses: no compositor bundle in /tmp/lumatv).
