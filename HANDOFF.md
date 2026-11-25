
## 2026-11-24 helper luna subscriptions through one bus bridge (installed, helper a43633e2)


## 2026-11-24 heap audit on the host (installed 2026-11-24 from main 22bb2b81: lumatv-ui bd0cd310, compositor f238da43)

`docs/research/memory-map.md` "Heap audit (host)" has the owners, the three fixes (wallpaper original copy, render-target zero buffer, warm-up picture) and the exact on-set check. Host live heap at Home idle 23.25 to 15.34 MiB; On the set the compositor's resident anon heap went 70.9 to 33.0 MB (PSS 103.9 to 66.5); the UI saves nothing there (the owner's theme grades the wallpaper). The on-set numbers are in the memory map. After the compositor apply `apps-grid` dropped 1 frame in 755 at 120 Hz in two runs (0 before); no copy of the previous compositor b83b168b was kept (a `--now` staged apply drops it). `home-widgets` and the apps-grid open exceed the 80-layer gate on `main` already (host, run to run 3-15 frames); not touched here. `rustui/patches/` lags `render/src` (anim, renderer-trim, session, ui/*, feather, geometry, shaders-fade): `make-patches.sh` would pick up that drift; only the gl and renderer patches were regenerated.
## 2026-11-24 helper luna subscriptions through one bus bridge (code on main, not installed)

Installed from main 385b6d71 with Netflix in front (helper restarted in 8 s, compositor and UI untouched): helper d9fab219 -> a43633e2 (payload and bin copies), carrying `lumatv-lsbridge` 161f6a92. The helper's 13 `luna-send` subscription children are one 32-bit bridge (`helper/lsbridge/`, embedded by `helper/app/build.rs`, protocol at the head of `helper/app/src/lsbridge.rs`); `ble_scan` keeps its own `luna-send`. The helper stages the bridge under `/var/lib/lumatv/lsbridge/` and writes its role (a privileged copy of LG's `luna-send` role) and manifest into `/var/luna-service2-dev` itself, so `tv_install.py --only helper` stays one file. A bridge that does not start, register or stay up leaves each subscription on `luna-send`. `libluna-service2.so.3.21.2` has no `LSHandleSetDisconnectHandler`, so the bridge ends when its bus sockets hang up.

On the set (helper 151 s old; PSS/USS MB): 13 `luna-send` 5.21/5.01 -> bridge 0.39/0.37, 162 -> 150 processes; the first reply of all 13 subscriptions equals `luna-send`'s; Netflix's pipelines were identical before and after; killing the bridge brings a new one in 0.1 s and every subscription back within 30 s, with no fallback to `luna-send`. The helper itself read 23.5/23.3 against 33.0/32.8 (anon 25.7 -> 16.8), which the bridge does not explain: the build carries every commit since d9fab219. `MemAvailable` 501.8 -> 329.3 MB and `Shmem` 25.07 -> 24.77 are not comparable (Netflix resident in the second read). Details in `docs/research/memory-map.md` section 7.

Untested: a foreground or settings change delivered through the bridge after the swap, and the hub restarting (host test only). Rollback: `tv_install.py --only helper --helper /data/home/tmp/memtrim-helper-backup/lumatv-helper-d9fab219 --deploy <dir>` (the installer wants a deploy tree whose `cutover.sh` and `compositor.sh` are the repo's `compositor/tools/tv-cutover.sh` and `tools/compositor.sh`; a directory with just those two is enough for a helper-only install), then remove `/var/luna-service2-dev/{roles.d/lumatv-lsbridge.json,manifests.d/org.nphil.lumatv.lsbridge.json}` and run `ls-control scan-volatile-dirs`.

Not changed: the two `openssl s_client` children (both to `ha.nateshome.net`: the voice assistant's `voice_ws.rs` and LumaShow's `ha-link`; in-process TLS needs a rustls transport with the CA bundle there, 1.3 USS each) and `arecord` (the voice assistant's wake-word microphone). Heap audit: attributed on the set, nothing changed. The compositor's 46.9 MB resident heap holds 36.8 MB in use; the free 10 MB is locked on purpose, and giving it back trades frame timing, so it needs the 120 Hz gate. Candidates in the UI, unmeasured: `lumashow-ui/src/kept.rs` keeps each kept image layer's source pixels (`Arc<[u8]>`); `ImageFile::release()` in `rustui/render/src/shm.rs` drops the compositor's mapping but not the tmpfs pages, so `Shmem` falls only if the file is truncated or removed.

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
