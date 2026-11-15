# LumaGame handoff

Current state and how to resume. Main design docs: ARCHITECTURE.md, DESIGN.md, PRODUCT.md (this folder); display research: docs/lumagame-gaming-features.md.

## State (2026-11-15 14:10 PST)

- App `org.nphil.lumagame` installed on the TV from main `04a4c1ab` (build-id `11b01edb054731d5`, ipk sha256 `b07b60d72de82dee…`). TV helper `43e74d46`. Built but NOT installed yet: helper fixes `cddab248` (gaming rollback journal) and `88a1a431` (VPQ low-delay id 0x00983D0E), plus Controller Lab / GameSir / Leadjoy pages (see "In flight").
- GeForce NOW works end to end on the TV: sign-in (account nphil, stored under the app's data dir), library with art, 4K120 stream, Back/overlay, end session. Shader cache: second launch's window at 397 ms (1975 ms cold).
- Old apps `org.nphil.lumatv.gfn` and `io.unom.punktfunk.client-webos` are still installed. Remove them only after the at-home checks pass.
- Sunshine/Apollo engine: separate GPL repo github.com/nphil/lumagame-gamestream (v0.1.1), pinned in game/engines.lock, fetched by game/tools/fetch-gamestream.sh.
- Controllers: GameSir Cyclone 2 protocol + crate `game/crates/gamesir-config` (`05374333`, docs/research/gamesir-cyclone2-protocol.md); Leadjoy Xeno Plus protocol + crate `game/crates/leadjoy-config` (`88fbfd7f`, docs/research/leadjoy-xeno-plus-protocol.md). Keep both pads in DS4 mode for play; GameSir settings need X-input mode (View+Menu 2 s), the config HID is absent in DS4 mode. Nothing has been written to a real pad.

## In flight

- ControllerLab-5 (worker, may have finished or died): Controller Lab screen + GameSir and Leadjoy settings pages through a helper route. Resume note: /data/home/tmp/lumagame-wip/ControllerLab.md. If its files are still uncommitted in the worktree (rustui/app/**, helper/**, game/crates/lumagame-core, lumagame-engine-gfn, game/engines/punktfunk, docs/lumagame/controller-lab.md, docs/plugins/gaming.md, tests/qmlhost/fixtures/ha-plugin.json), finish from that note: tests + clippy, commit with explicit paths, push both remotes, rebuild ipk + helper. `rustui/ui/src/system/volume.rs` is someone else's: never commit it.

## Next work, in order

1. Finish ControllerLab (above). Then install the new LumaGame ipk and helper on the TV (announce to LumaTV first; post-checks after the helper install).
2. With Nitin at home (he sets the time): controller presses in a game, sound by ear, Punktfunk pairing/stream and Sunshine/Apollo pairing/stream with his gaming PC, GameSir + Leadjoy settings read/write on the real pads (Leadjoy 10-colour palette test with save/restore, see the protocol doc), physical latency with Controller Lab's press-to-flash test. Then uninstall the old GFN and Punktfunk apps.
3. Steady-rate VRR mode for local streams (docs/lumagame-gaming-features.md "True VRR"): per session the helper sets `VRR 1` and the RAM decoder params (vsync_disable_phaseshift 1, vsync_shift_delta 3000, vsync_shift_limit2 800 via /proc/lxipc/vdec/user_command); only a reboot restores; fix the 120 fps start-up dip; look for a reboot-free exit (`vdisp_set_vsync_shift` untested).
4. Native-games proof: a minimal SDL2 + GLES 3 app launched by LumaGame with a frame-time log at 120 Hz and a clean return (see "Native games" below).
5. Frame-on-screen timestamps (needed for real latency numbers and pacing). Not in signed firmware: LG's pipeline has an internal "event 72" (position + clock at notification) that NDL does not forward and direct-ES mode does not generate (docs/research/rea-firmware-result.md Q1). Routes, in order: (a) hook libNDL/starfish internals inside our own feeder process (NDL runs in-process; no firmware change); (b) per-picture kernel lines (display_picture / vdisp monitor, seen in the 4:2:0 control run of the 4:4:4 test) or VO /proc entries with timestamps; (c) DE2VSYNC/osd_irq counts correlated with feeder send times.
6. Smaller items: BT pad stopSniff route (docs/research/rea-bt-controllers-result.md), overlay stats from /proc/lg/de/win0/status, 5.1 as 6-channel Opus (docs/research/rea-ndl-audio-result.md), feeder GLib warning loop after final unload in serve mode, 120 Hz UI frame-gate of LumaGame screens, screensaver over LumaGame.
7. True VRR via a patched DE firmware (Nitin wants this). Streams: VDEC (signed) presents on its own timer, so DE-only changes cannot follow stream frames unless decoded frames can reach the OSD/GPU plane (unknown; check whether hardware decode can output to a texture). Native games/emulators on the OSD plane: a patched DE could start a refresh per OSD flip (candidate: one PSYNC per screen update, offsets in the True VRR section). Order: (a) offline design of the patch + the decode-to-texture question; (b) recovery check with an unmodified copy of the active image through the /var/firmware/pqefw%d.bin override (prove SSH stays up and removing the file + reboot restores); (c) patched image with Nitin at the TV. Steps (b) and (c) need his explicit approval.
8. Needs Nitin's explicit approval (brick risk): any DE firmware modification (plan in docs/research/rea-firmware-result.md "DE firmware modification route"). Never load or patch firmware without him at the TV.

## Research results (settled)

- Low latency: NDL's game-mode load already applies every latency control; extra helper controls are redundant (docs/lumagame-gaming-features.md).
- VRR: true per-frame VRR is not reachable for decoded video without changing signed VDEC firmware; the DE follows the declared or 300-frame-averaged rate. A 72 fps stream already gets 144 Hz scan-out. Cloud G-SYNC stays off.
- 4:4:4 refused by VDEC firmware (signed); 120 fps cap compiled into signed vdec.bin. DE firmware is unsigned on O22 (plan only).
- LG Game Dashboard is HDMI-only with no app entry point; LumaGame draws its own overlay.
- Results: docs/research/rea-*-result.md and docs/lumagame-gaming-features.md.

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
- Coordination: the LumaTV OS chat (session 01a19410-481a-719d-9a02-554dd995690b) owns compositor, ui, ssgw, declutter and the receiver keeper; message it with send_to_chat before and after TV writes, installs, wakes, reboots or helper restarts. Workers cannot message other chats; the parent relays their start/finish notices.
- Workers: one deep-task at a time. Hand each worker a resume note under /data/home/tmp/lumagame-wip/<Name>.md and ask it to keep that note current so a restart can continue. Give every worker the files it owns; others commit concurrently.
- Check every worker result before marking it done: commit on origin/main, receipts present, claims consistent with the doc. Several reports this session had wrong paths or overstated results.
- Reporting to Nitin: he is not a programmer and has ADHD. Answer first in plain words, short bullets, numbers welcome, one clear next step at the end. Explain jargon or leave it out.
- Disk: build output and LG binaries never in git; scratch in $CODY_SCRATCH; the shared scratch /tmp/cs/3dc58534e64e was wiped once by a worker, so keep anything needed later under /data/home/tmp/lumagame-*.
- Host builds that pull `ring`: `CC="$PWD/tools/cc-host"` (see game/crates/lumagame-meta). The helper's root-runner test needs a world-traversable TMPDIR, which this host's private scratch lacks.
