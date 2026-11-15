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

## Working rules

- Read /data/home/lgtv/CLAUDE.md. Commit straight to main, push to origin and forge, explicit paths only (other sessions commit concurrently).
- TV access: lease via /data/home/Marquee/tools/tv-lock.sh. Tell the LumaTV OS chat beforehand about anything that wakes the screen, writes, installs or restarts the helper (read-only sessions need no notice), and again after. Mute the receiver before waking the screen and re-mute after every wake or app launch (the Denon keeper and ARC unmute it about 10 s later; after a reboot the screen comes up on Home, so mute as soon as the receiver reads ON); leave the set in Active Standby with the receiver off and unmuted. Never touch the ssgw plugin or /etc/xinetd.d/ssg_https, never install compositor/ui, never change OLED light, brightness or picture mode.
- Detect a TV reboot by the uptime resetting, not by `/proc/sys/kernel/random/boot_id` (this TV keeps the same boot_id across reboots).
- After a helper install, confirm `lumatv ssgw status` (enabled, entry ours, gateway true) and no new crash record in /var/lib/lumatv/ssgw/crash/.
- Never write VRR registers or ForceVRR without an owner-approved plan. Owner standing approval (2026-11-15, until ~17:00 PST): run TV tests without asking, except anything with brick risk (firmware loads, flashing); for those write a risk-minimising plan and ask him first.
- GFN sign-in from a phone: use a private browser tab (a stale NVIDIA session gives "Login Failed"). Never submit the device code from this host; that consumes it.
- Host builds that pull `ring`: `CC="$PWD/tools/cc-host"` (see game/crates/lumagame-meta). The helper's root-runner test needs a world-traversable TMPDIR, which this host's private scratch lacks.
