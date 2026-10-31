# Power key: slow standby, repeated presses, set gone from the network (2026-10-30, installed 93496e61)

## Resume here

Nothing in the compositor's code path can hold the Power key or the standby for seconds, so the cause of the 20:28 report is **not found**. Written
without the set (off the network). Built on forge main after `68f5b6ca`:

- Power is the first thing `State::key_event_at` handles (`input.rs`, `power_key`): given to the focused window at once, press, repeats and release;
  then the screen's care hears of it. The owner's button choices (`buttons::protected`), the key router and the Rust shell's layers (the saver's
  "any key", `took` entries) no longer see it, so no press can wait on an earlier press's release.
- The journal now times the standby from the key: `input: Power pressed[, N ms after the one before]`, `tvpower: the set is "X" (was "Y"), N ms after the
  Power key`, `tvpower: PreparePowerOff (...) -> ack, N ms after the Power key`. Host test `keys::power_goes_to_the_app_first_on_every_press_...`.

### TV check (next slot, set on, LumaTV compositor persistent, Home in front, pop-ups off)

Needs the power state log and a 60 fps phone video of the panel with the remote in frame.

1. Before: `uptime`, `ls /tmp/var/log/reports/librdx | wc -l` (any file there arms the reboot below), `lumatv standbyscreen status`,
   `ls /var/luna/preferences/webosbrew_failsafe /var/lib/lumatv/.boot-pending` (both should be absent), `lumatv compositor status` failures.
   Start `ls-monitor -f com.webos.service.tvpower > /var/lib/lumaglass/power.log` (on /var, tmpfs does not survive a reboot).
2. One press of Power on the remote, video running. **Press-to-dark** = video. Then `ctl journal`: the `Power pressed` line, each `tvpower: the set is` line
   and the two `-> ack` lines give press to `request power off`, to the ack, to `active standby` in ms. LG's own compositor measured 3.7 s from
   `request power off` to `active standby` (`shell-architecture.md`, pass 2); LumaTV's logo run 1.0 s from the `power/powerOff` call
   (`surfaces-p2-result.md`); `HANDOFF.md` "within ~3.4 s". A press-to-dark over about 4 s, or a gap before `request power off`, is below the compositor
   (lginput2, the remote's Bluetooth wake, tvpower); a gap between `request power off logo` / `request active standby` and our ack is ours.
3. Ten seconds after dark: `ping` and SSH. Active Standby keeps SSH (HANDOFF); a set that is gone is a real off or a reboot: read `uptime` when it is
   back and `ls /tmp/var/log/reports/librdx` (tmpfs: empty after a boot, so read it **before** the press) and the power log for `rebootToSuspend`.
4. Wake with Power, wait for Home, press Power four times at 0.5 s: every press is a `Power pressed` line, the set ends in one state, and the
   journal shows what each press did (a press during `request power off` is LG's: tvpower may cancel it).
5. Repeat 2 and 3 with the saver up (3 min idle) and with a panel open; compare.

## What was read

**The path.** `lginput2` makes every remote an evdev device and handles Power itself, below the compositor: LG's key chain has no handler for it
(`surfaces/KeyEntryPoints.json`: "Power (Key_webOS_PowerOnOff) has no handler in the QML chain at all: handled below the compositor"). LumaTV's compositor
reads the same devices and, before this change, routed 116 like any unknown key (`route`: `_ => Client`) after the owner's button choices, the router and the
shell's layers. Standby is tvpower's: `request power off` -> `request power off logo` (registered clients ack) -> `request active standby` ->
`PreparePowerOff` (28 services ack within 160 ms) -> `active standby`. The compositor takes part only in two acks (`luna_methods.rs`), sent in the same
loop pass as the push; the panel's brightness put back (`panel_standby`) is queued before the ack, both async through the bridge's writer thread.

**Nothing found that delays Power.** Not a sleep, sync luna call, fade or flush on the input path (`care_input`, `remap_button` and its one `stat`, the
router, the journal are all microseconds). The Rust Home's `took` entries and the saver's "any key" swallow could hold a repeat of a key whose release
was lost (Power's release does not arrive when the set goes down under it); Power no longer goes through them. The owner's Remote > Buttons could take
Power through `code:116` (`custom_code` filtered only Home and the pointer's reports); `protected` now has it. Today's engine commits (93496e61 take-in-ahead,
floor, page copy, PopupFix, FullViewBack, SubWindow2) are in the render path or the UI process: `take_in_ahead` applies waiting commits of 64+ operations
(about 7 ms at the largest, round 27), nothing waits on a socket or a timer. Panel dim, OLED care, the saver and the helper make async calls only.
The watchdog (`lumatv-sm.c`) treats every non-Active power state as healthy and a standby of any length as no stall.

## Standby timing on record

- LG's compositor, pass 2 (`shell-architecture.md`): request power off 56.9 s, request active standby 60.2, prepare power off 60.3, active standby 60.6;
  wake: prepare power on 90.7, active 91.0.
- `HANDOFF.md`: after power-off the set enters active standby within ~3.4 s; a power-on from there is instant.
- `surfaces-p2-result.md`: `power/powerOff` with the LumaTV logo: Active Standby 1.0 s after the call.
- Round 9 (`round9-result.md`): power key -> Active Standby "8 s later" (a coarse read after a sleep, not a measurement).
- The gate's `standby` check (every gate since 10-25) records rounds (PreparePowerOff 1, PreparePowerOn 1), fallbacks 0 and uptime, with fixed sleeps:
  **no press-to-dark time exists** for the key; this document's TV check is the first.

## The set going off the network

`HANDOFF.md` ("Standby: the proven mechanism"): in standby SSH survives, unless `faultmanager` finds a crash report in `/tmp/var/log/reports/librdx` at power-off:
it answers the suspend request `ack:false, rebootToSuspend` and tvpower reboots the set about 6 s after it entered standby. Any process crash since the
boot arms it (not only the compositor restart's inputcommon one); round 26's unexplained reboot at 14:18 shows the set is not crash-free today.
Also real off, no network: Always Ready off (`standbyscreen off`: gate 9, port 22 refused 6+ min, wake needs a cold boot; Nitin's decision of 10-28 is Always Ready on) and
Quick Start+ off (`standby cold`).

HA history (Main, 2026-10-30): TV off 18:22, helper gone 18:23, on 20:33 by a cold boot with Homebrew's startup not run (ports 22 and 6054 closed, LG Home).
Two flags give exactly that after a boot that is cut short: Homebrew's failsafe (`/var/luna/preferences/webosbrew_failsafe`, armed at its startup, cleared about
10 s after run-parts; set on power-off inside that window disables every root customization, SSH included: `autostart.sh`) and ours, `.boot-pending`
(`tools/lumatv`: `boot-bind` touches it, `boot-ok` removes it after the boot's apply, about 70 s: `apply ok at 72 s` on round 26's boot). `compositor.sh`
and `boot-bind` then take the failsafe path (LG's compositor, LG Home). Our shutdown path writes neither: `autostart.sh` returns to run-parts at once, the
compositor and helper only ack and idle in standby. A boot that is put to suspend or off within those windows (a reboot-to-suspend after a fault, or a short
boot into standby) leaves both set, so it fits an 18:22 standby that rebooted and went dark again, and not a slow teardown. Not proven: the boot between
18:22 and 20:33 is not on any record read here.

To settle it on the set: `/var/log/messages` of the 20:33 boot (its oldest line, `bootd`, `faultmanager`, `rdxd` entries for 18:2x), `tvpower`'s power-off reason
(`remoteKey`, `rebootByFault`, `OFF_BY_KEYTIMEOUT` as in `wake-2026-10-29/README.md`), the supervisor's log and `/var/lib/lumatv/crash` for 18:20-18:25, the two
flags' mtimes, and `/var/lib/lumatv/.boot-pending` before any later apply clears it.

Open decision for Nitin and Main, not made here: `.boot-pending` could be cleared when the compositor is asked by tvpower to prepare a power off (it is alive and
answering then), so that an orderly power-off during the first 70 s of a boot no longer makes the next boot a failsafe one. It loosens a safety net, so it waits for
the evidence above.
