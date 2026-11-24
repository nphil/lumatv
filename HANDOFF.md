
## 2026-11-23 helper + UI quiet install (13cd5621)

Installed with Netflix in front (live swap, compositor untouched): helper 83bc5e2d -> 293bb019 (restarted, plugins connected, audio-passthrough route answers, idle CPU unchanged), lumatv-ui 24ae7126 -> 68b26d09 (shell client restarted). Prewarm held ("Home not idle"). Not tested on the set: dock card lift, Restart confirm focus, frame gate, Keep apps ready row, app open times, tuner takeover toast and Live TV close.

## 2026-11-23 screen checks on the set (main 70283ab3)

Installed: helper 293bb019 -> 22a19fd1 (payload and bin; restarted, compositor untouched, 18 plugins connected). UI 68b26d09 and compositor 3ba09192 unchanged. The helper's tuner tests (132) and prewarm tests (13) passed at 022b13a4.

On the set: dock card lifted and Restart LumaTV confirm focus match the renders (`ui-focus-tv-*.png`). 120 Hz gate, 3 cycles each: home-dpad, home-widgets and the new `system-confirm` surface (task manager, System, Restart LumaTV confirm, Right/Left, Back out; never confirmed) dropped 0 frames, burst over 20 ms 0. Settings > System > General has Keep apps ready (On), order Marquee, Netflix, Prime Video, Apple TV, Live TV. Prewarm from cold launched Netflix, Prime Video and Live TV in the background with Home in front and no audio pipeline; Apple TV and Marquee are skipped by design. Tuner: two clients on one RF (2.1 and 2.2) shared at about 1.1 and 0.25 MB/s, one leaving did not interrupt the other; a client on another RF ended both with a clean EOF and `displaced` named it; with Live TV in front the toast "TestD is now using the tuner" showed and Live TV closed. The takeover landed on the LumaTV app's own page; the helper now launches `com.webos.app.home`, which shows LumaTV's Home (checked after the install). Two clients from one IP replace each other by design.

## 2026-11-23 helper install (main f1dff184)

Installed on the set: helper 22a19fd1 -> 6e05f76d (live swap, compositor untouched), carrying the TrueHD un-mix order and `POST /v1/audio-passthrough/flowing` (answers "no session is armed" with none armed) and the tuner fixes of 70283ab3. UI bdd2884c (from 9057b877) was installed at 21:20; compositor unchanged. audio-passthrough, voice-assistant and live-tv-tuner connected.

## 2026-11-23 weather install (main e85b765a)
Installed on the C4: helper 6e05f76d -> 696e735f, lumatv-ui bdd2884c -> 686031e6. Profile nitin: clock.weekStart monday, layout.statusBar.date centre. Capture home-tv-weather-calendar.png: month grid, centred date and week list shown. Untested: weather day open/close and the 120 Hz frame gate (gate refuses: no compositor bundle in /tmp/lumatv).

## 2026-11-23 helper install (main 6950ab31)

Installed on the C4: helper 696e735f -> 2998ff4c (payload and bin, live swap, pid 25817 -> 9920, compositor untouched), carrying the TrueHD passthrough's takeover of LG's mixer input for the player's own stream. The running helper's exe reads 2998ff4c and `GET /v1/audio-passthrough/route` answers unarmed. Not yet tried on the set: the takeover, retake and give-back during a Marquee TrueHD Load; that uMS lists Marquee's pipeline holding `PCMMC4` (taken from its `smp_cb` resource list); audiooutputd's connect of `AMIXER4` without `pipelineAttribute`.

## 2026-11-24 helper install (main 5ac499ca)

Installed on the C4 with nothing playing and the passthrough unarmed: helper 2998ff4c -> 9ec753c5 (payload and bin, live swap, pid 4733 -> 10438, compositor untouched); the running exe reads 9ec753c5. After a mixer takeover the ADEC connect now follows the un-mix, since the output codec follows the main input, which stayed AMIXER4 in the 22:59 run. Not yet tried on the set: Pass-Through showing within the 1 s grace after that connect.

## 2026-11-24 helper ARP guard
Core arp-guard thread sets arp_ignore=1/arp_announce=2 on all and on every interface sharing a subnet (every 5 s); fixes eth0/wlan0 ARP flux. Helper d9d93614 installed (907e9174 before), built on 5ac499ca. Multi Link's 0/0 was only its restore of originals.
