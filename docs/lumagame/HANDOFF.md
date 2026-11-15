# LumaGame handoff

Current state and how to resume. Main design docs: ARCHITECTURE.md, DESIGN.md, PRODUCT.md (this folder); display research: docs/lumagame-gaming-features.md.

## State (2026-11-15)

- App `org.nphil.lumagame` installed on the TV from main `04a4c1ab` (build-id `11b01edb054731d5`, ipk sha256 `b07b60d72de82dee…`). The TV helper is `43e74d46`; the fix in `cddab248` (gaming rollback journal) is built but not installed.
- GeForce NOW works end to end on the TV: sign-in (account nphil, stored under the app's data dir), library with art, 4K120 stream, Back/overlay, end session. The shader cache brings the second launch's window to 397 ms (1975 ms cold).
- Old apps `org.nphil.lumatv.gfn` and `io.unom.punktfunk.client-webos` are still installed. Remove them only after the at-home checks below pass.
- Sunshine/Apollo engine: separate GPL repo github.com/nphil/lumagame-gamestream (v0.1.1), pinned in game/engines.lock, fetched by game/tools/fetch-gamestream.sh.

## Needs the owner at home

Controller presses in a game (grab fix 2b263a35, untested with presses), sound by ear, Punktfunk pairing and stream with the gaming PC, Sunshine/Apollo pairing and stream, physical latency measurement, and installing helper cddab248.

## Research in progress or queued

Order of work: VRR + low-latency hardware truth (running), REA of LG Game Dashboard / BT controllers / multichannel audio (running), Controller Lab screen and GameSir Cyclone 2 config protocol (running). Queued: NDL/VO on-glass presentation timestamp, VO 120 selection cap (144 Hz, AV1 at 120), HDR re-apply drop and Dolby Vision, 4:4:4 VDEC gate. Results land in docs/research/*-result.md and docs/lumagame-gaming-features.md.

## Native games (study of github.com/pustal/Web-Mario-64, 2026-11-15)

Web-Mario-64 is a native webOS app (`type: native`), not a web build: Ghostship (an SM64 PC port) with a webOS patch layer, SDL2 2.30.12 from webosbrew's SDL-webOS fork (armv7), GLES 3 on the TV's own driver loaded at run time, packed as an ipk with `ares-package`. Remote keys map to SDL keys (Back 482, colour keys 486-489, Exit 505). Ghostship runs game logic at 30 fps with frame interpolation; its "match refresh rate" option should drive 120 Hz (read from code, untested). Plan for LumaGame's native engine: keep ports as separate SAM apps launched by LumaGame; first build a minimal SDL2 + GLES 3 app with a frame-time log, measure 120 Hz on the TV, then test the return-to-LumaGame handoff. LumaTV's own GPU path is rustui/render (loads `libGLES_mali.so` by path) with the 64-bit runtime in gfx64/.

## Working rules

- Read /data/home/lgtv/CLAUDE.md. Commit straight to main, push to origin and forge, explicit paths only (other sessions commit concurrently).
- TV access: lease via /data/home/Marquee/tools/tv-lock.sh. Tell the LumaTV OS chat beforehand about anything that wakes the screen, writes, installs or restarts the helper (read-only sessions need no notice), and again after. Mute the receiver before waking the screen and re-mute after every wake or app launch (the Denon keeper and ARC unmute it about 10 s later; after a reboot the screen comes up on Home, so mute as soon as the receiver reads ON); leave the set in Active Standby with the receiver off and unmuted. Never touch the ssgw plugin or /etc/xinetd.d/ssg_https, never install compositor/ui, never change OLED light, brightness or picture mode.
- Ending a session: after MUOFF to the receiver, wait for it to report its audio status over CEC (or ask with CEC 0x71) before PWSTANDBY; otherwise the TV keeps a stale mute flag from the receiver (ARC mirrors it). Never unmute the TV directly (it goes out over CEC to the receiver).
- Detect a TV reboot by the uptime resetting, not by `/proc/sys/kernel/random/boot_id` (this TV keeps the same boot_id across reboots).
- After a helper install, confirm `lumatv ssgw status` (enabled, entry ours, gateway true) and no new crash record in /var/lib/lumatv/ssgw/crash/.
- Never write VRR registers or ForceVRR without an owner-approved plan. Owner standing approval (2026-11-15, until ~17:00 PST): run TV tests without asking, except anything with brick risk (firmware loads, flashing); for those write a risk-minimising plan and ask him first.
- GFN sign-in from a phone: use a private browser tab (a stale NVIDIA session gives "Login Failed"). Never submit the device code from this host; that consumes it.
- Host builds that pull `ring`: `CC="$PWD/tools/cc-host"` (see game/crates/lumagame-meta). The helper's root-runner test needs a world-traversable TMPDIR, which this host's private scratch lacks.
