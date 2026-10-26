# Round 9: the overnight TV run (2026-10-26, TvRound9)

## Resume here

Started 00:00 EDT. On the set: the ipk of `d57f2ce` (clean worktree `/tmp/lumatv-slot`, compile gate 117 QML, 0 failed;
install 4 s, `apply` 23 s, one compositor restart, every `[Resident] * built` line, Home in front at 00:01). Evidence in
`docs/research/round9/tv-2026-10-26/` (`log.txt` the timeline, captures as PNG). Receiver: found on, 65.5, not muted,
DOLBY DIGITAL, TV Audio; set to 0.10 for the compositor gate (the Denon cancels its mute on a volume key), 0.655 to put back.

Done: the compositor's eighth gate (FAIL on x1, netflix, wd-crash; LG's compositor live, flag off; results table).

Next, in order: autoinstall standby/wake then one reboot; Dolby format text and the mute cue; the frame-rate table; the
voice-app QML copy and E3; the peers' commits (RoundFixes, VideoPlaneSaver, LiveTvRestyle, SettingsRest2) in batches with a
quick/picker re-gate. End state per the rules: Home in front, Active Standby, helper + Home stub + lean + preload trim intact,
LumaTV's compositor live only if the full gate passed, receiver 0.655 unmuted.

## Round 9 results

(filled in as each slice ends)

| Slice | Result | Evidence and numbers |
|---|---|---|
| Deploy `d57f2ce` | PASS, two startup warnings | dev/install 4 s, `apply` 23 s; `[Resident]` splash, banner, voice, search, drawer, saver, picker, edit, settings, quick, menu, confirm, remote, overlay banner/drawer/volume, apps, overlay quick panel, overlay input picker all built; no `is not defined`. New at start: `panels/AppsGrid.qml:171: TypeError: Value is undefined and could not be converted to an object` (`page.rail[parent.at].sub` before the rail exists) and two `[Dock] launchPointsList parse failed: SyntaxError` (SAM not answering yet); LG's own `InputPicker.qml:35` and `StarfishViewAdjustments.qml:50` as before |
| Compositor gate 8 (`slot-2026-10-26-0003`, 00:03-00:13, `--enable-after` armed) | FAIL, LG's stays | firmware, homeheld, live, volume (0.10 → 0.105 → 0.10), hdmi, standby/wake (PreparePowerOff 1, PreparePowerOn 1, no PIN), homekey, wd-hang, escape PASS. **x1 FAIL**: the client's own swaps 0 over 20 ms (3599 frames, 59.97 fps, worst 19.7 ms, 70 in 17.5-20 ms; the best run yet) but the compositor's log has `frame gap 20.4 ms on surface 185 (x1) after 1516 steady frames` and a `page-flip stamp 33.37 ms` right after `tick: frame held while a driver buffer warms` (`gate/x1-compositor.txt`). **netflix FAIL**: the deep link opens to Netflix's error `tvq-pb-101 (5.2.8)` ("issue with your streaming device"), no video window, Picture Mode stays filmMaker (`gate/netflix.png`); the same screen as slot 1606 (163003 bytes) while slot 1730 played Dolby Vision: intermittent under LumaTV's compositor, never seen under LG's. **wd-crash FAIL on the reason string only**: fallback in 0.5 s, failures 0 → 1, LG's back, SAM unchanged, but `last-fallback` reads `compositor closed the bridge socket` instead of `killed by signal 9` (the supervisor sees the socket close before it reaps the child). Trial: 2 loop passes over 20 ms (`loop: 23.2 ms in one pass (tick)`) |
