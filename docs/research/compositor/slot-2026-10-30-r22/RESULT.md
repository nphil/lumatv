# Round 22 (TV slot 2026-10-30)

## Resume here

- Installed: forge 5afc7e41 (worktree `/tmp/lumatv-r22`, staged as `/data/home/tmp/lumatv-next`; build script `/tmp/r22-build/build22.sh`). LumaTV compositor persistent and live, helper running, Smart home listener on (6054, HA's LumaShow entry connected).
- Newer commits (21a560e3 video windows in UI px, e81474d6 sub window) are not installed yet; rebuild and reinstall when Frames120 pushes or at 05:00, then run CameraPopup's A2/A3 and D1, SubWindow's M2, and the frames gate at 60 and 120 Hz.
- Not yet run: Marquee steps (waits for Main).
- Changed on the set for the night, to put back at the end: picture backlight (filmMaker/default) 0, original 80 (the 68 seen at the start was the OLED care dim in progress); `/var/lib/lumatv/helper.json` gained `cameras.testEvents` true (original kept as `helper.json.before-r22`).
- Install procedure that worked: `cutover.sh disable` on the set, wait for "recover: Home on the panel" plus 60 s, the tv-install grant (the MCP call times out client-side at 30 s but the script keeps running; read `/tmp/lumatv-install-out/log.txt`), then `cutover.sh install` and `cutover.sh enable` over ssh (install refuses while the compositor is mounted).

## Results on 5afc7e41

| # | Check | Result |
|---|---|---|
| 1 | Install, hashes, persistent | pass: helper sha, compositor and ui md5 equal the build; flag persistent, live |
| 1 | Home: avatar, icons, weather | pass (`home1.png`) |
| 2 | Port 6054 and HA entry | pass: listening; entry 01M627CQ3HVVVFM62HQEYVFE57 `loaded`, helper log "the LumaShow integration connected"; the grant's script printed `lumatv: not found`, helper restarted by hand |
| 3a | Weather card on Home | 43,264 px per frame, 1 rect, ~75 % of frames (60 per s); the doc's "about 31 k px" is 40 % lower than measured |
| 3b | Weather card, Home covered (Browser in front) | pass: 5 frames in 9.6 s, all `ui 0 draws 0 layers`, 0 card draws |
| 3c | Weather view | pass: OK on the card opens it (`wx-now.png`), Down walks Now to Radar |
| 3d | Radar first frame | 1.4 s from the Down key to the first frame (page shows "Loading the radar" first); map and frame fade in by 2.3 s |
| 3e | Radar loop | pass: frames alternate FORECAST and RADAR (`wx-radar-sheet.png`), a step about every 0.56 s |
| 3f | Location | Atlanta: the helper's base.weather zip 30311 (33.723, -84.4702), unchanged |
| 3g | App icons on the dock | pass: distinct hue and glyph per app (`home1.png`) |
| 3h | Camera pop-up corner | pass: card top edge at y=32, right edge at x=1888 (32 px from the corner); the picture box was black (round 21's videooutputd units bug, fixed in 21a560e3, not installed) |
| 3i | Toast | bottom centre (`toast1.png`); only a test change in the blocker test moved, no placement code of toast or volume changed |
| 3j | Volume bar | not run: a volume key would change the receiver |
| - | Frames120 ring probe | `frames120-settings-ring.json` (notes `r22-settings-open`, `r22-settings-steps`) |
