# Round 9: the overnight TV run (2026-10-26, TvRound9)

## Round 10/11

### Resume here

Round 11 resumed at 03:47 EDT. Clean package `3b7cceb` (includes `5d3267c`) is on
the set: 120 QML files compiled, 0 failed; package SHA-256
`da345b3bc410320d02768ae5ac4f13e984ed7b56d1e3ec94565d27996e9af59d`. Install
and apply completed at 03:52. Evidence: `round9/r11-2026-10-26/`; historical
diagnosis: `round9/r10-2026-10-26/log.txt`. The old scratch worktree is preserved
at `/tmp/lumatv-slot-r9`; the clean deployment worktree is `/tmp/lumatv-slot`.

Receiver read-back at 03:47: on, 65.5, unmuted, DOLBY DIGITAL, TV Audio. Round 10
restored the TV-speaker profile to 24 and selected external ARC again. Mute before
playback; close the playback app and bring Home forward before restoring unmute.
HDMI switching can change the receiver input and cancel mute.

Next: finish frame-rate probes, Live TV and Search checks, quick/picker gate,
GFN checks, and peer checks. Main confirmed the existing direct TV deployment
authorization; no infrastructure host changes are involved. No SAM restart,
rootfs writes, DRM or PIN probes, or light streams. Layout hash before deployment:
`e194fda89ba7219fb08c389b1c92573db64de9178272cd948c3a41ada336a6a2`. Finish with
Home, Active Standby, LG’s compositor, free decoder ports, and the receiver restored.

HDMI switching cancels mute: volume 0 was used during the HDMI regression. HA’s
`volume_set(0.655)` restored only 65; one TV Volume+ key on Home restored exactly
65.5. The receiver was unmuted on Home, then muted again for the next probes.

| Slice | Result | Evidence |
|---|---|---|
| TV-speaker profile | PASS, Round 10 | 01:39: volume 0 → 24 on `tv_speaker`; output restored to `external_arc`; receiver remained 65.5. |
| Netflix failure diagnosis | CONFIRMED, Round 10 | Old restart with HDMI 2 foreground left `adec.p1 Open=1`, `HDMI_PORT2`, while Home was foreground. Netflix reported `tvq-pb-101`; the decoder was not released with the input app. |
| Input release recovery | PASS, Round 10 | 01:58: `inputs release` launched and closed HDMI 2 through SAM; both decoder ports became `Open=0` in 3 s. |
| Fixed restart | PASS, packaged Round 11 | 03:53: HDMI 2 foreground, `adec.p1 Open=1 Start=1` → restart in 5 s → Home, both ports `Open=0`, no running input app. |
| Netflix after fixed restart | PASS, packaged Round 11 | 03:54–03:55: media service reported `playState=playing` for 20 s, Dolby Vision cinema mode, receiver muted throughout. Netflix closed, Home restored, both ports free. |
| Receiver restoration | PASS, Round 10 and Round 11 read-back | 02:11 restore and 03:47 HA read-back: on, 65.5, unmuted, DOLBY DIGITAL, TV Audio. The two 06:11 log timestamps are UTC and corrected immediately afterward. |
| Dolby signal and text | PASS telemetry and offline render; audible TV check deferred | Actual receiver data: `signal=Dolby Atmos - DD+`, `soundMode=DOLBY ATMOS`, saved as `netflix-receiver.json`. Dark/light offline renders of that reading (only mute disabled in the host) both show the full `Dolby Atmos` text, no warnings. Actual TV format text stays hidden while muted; unmuted TV check remains for the morning. |
| Persistent mute mark | FAIL subtitle clearance; motion works | `netflix-playing.png` and `netflix-mute-hop.png`: the mark moves horizontally after 10 s but covers the Netflix subtitle rows. Reported to Main for the volume owner. |
| Clean package | PASS | 120 QML, 0 failed; installer matched the helper hash after 8 s; apply took 18 s. LG compositor, Home stub, helper and lean profile intact; helper reports 14 apps kept from preloading. |

## Round 9 handoff (historical)

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
