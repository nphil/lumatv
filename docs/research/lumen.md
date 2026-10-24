# Lumen (ammar027/lumen-webos, lumen-gallery-webos): what LumaTV lifts

Both MIT. Numbers are the author's, from one webOS 22 set (65UQ8040PSB,
Chromium 87, Mali-T820 class), not measured here.

## Taken

1. Focus scale without clipping. A scaled focus tile with `overflow:hidden`
   and rounded clipping rebuilds a mask every frame (`tools/apply_perf2.py`).
   LumaTV's focused tile grows without a clip; rounded corners come from the
   SDF shader, which costs nothing extra.
2. Release when hidden. After 5 s out of sight the wallpaper image and video
   are dropped and reloaded on show (`tools/apply_lowmem.py`). A decoded
   1080p image is about 8 MB before its blurred copies.
3. Staged start. Only the first row is built at launch; the rest 3.5 s later
   or at the first key press, and wallpaper motion starts 6 s after launch
   and only after 1.5 s without a key (`tools/apply_bootdefer.py`).
4. Resting-pointer guard. A resting Magic Remote pointer fires hover on
   whatever slides under it; a hover counts only when the pointer actually
   moved (`tools/apply_pointerfix.py`, issue #4 on webOS 25).
5. Dim modes for the wallpaper: Off, At night, Always, as an opacity-only
   black layer (`tools/apply_wpdim.py`).
6. Focus strength setting: Subtle 1.08, Normal, Strong 1.26
   (`tools/apply_focus.py`).
7. Art-mode pipeline from lumen-gallery: decode the next picture before the
   swap, 1.4 s opacity crossfade, keep the old picture until the new one is
   ready, skip a broken one after 300 ms, weighted shuffle across
   collections, a quick bar (Previous, Like, Hide, Next), and one declarative
   option list that drives both the settings panel and the remote keys.
8. Power facts (`docs/power-on.md`): every reboot lands in Active Standby;
   `turnOnScreen` does not wake a set that is off; `homeAutoLaunch` on holds
   LG Home for about 7 s, off puts the replacement in front at about 5 s.

## Not taken

- Parallax tilt: the author excluded it as laggy on the set.
- Live blur during movement; LumaTV keeps the one cached blur.
- Art fetched at view time through `images.weserv.nl` with no cache. LumaTV
  keeps pictures locally, with a per-image licence check (Wikimedia images
  are partly CC BY-SA).
- The gallery's static frame, matte and clock layers: they have no pixel
  shift or dim cycle, which is a burn-in risk on OLED. LumaTV's art mode
  orbits and dims everything.
- Root shell calls from a web UI (`org.webosbrew.hbchannel.service/exec`) and
  the 1 s foreground polling watcher; LumaTV uses its helper and events.
- Keep-alive by editing LG config lists and rebooting, and `setDefaultApp`
  for Home: config and reboot steps outside the plan.
- The standby clock faces' SF Rounded font (not redistributable).
