# LumaTV shell architecture: how much of LG's stack to replace

Evidence-based study (2026-10-23) of replacing LG's stack layer by layer for
performance, stability and full use of the C4's hardware. Reference set:
LG C4 OLED65C4PUA, webOS 10.2.1, kernel 5.4.268, chip O22A3. All TV readings
in this document are read-only (`ps`, `/proc`, `/sys`, strings of binaries,
DRM read-only ioctls); nothing here changed the screen, power state or picture
mode. Probe sources and raw logs: `docs/research/arch/`.

## Resume here

- Done: file created; TV reachable (`192.168.1.242`, root, Homebrew default
  password, see `/data/home/lgtv/RESUME-NOTES.md`); process table
  (`arch/ps-01.txt`); DRM inventory (`arch/drmplanes.c` built with
  `zig cc -target aarch64-linux-musl -static`, output `arch/drmplanes-01.txt`,
  debugfs state `arch/drm-debugfs-01.txt`); 30 s per-process cost
  (`arch/proccost-30s.txt`, raw `arch/procsnap-a/b.txt`).
- Next: Wayland globals bound by Netflix / WAM / Flutter Home (strings of
  their libraries, not the tiny launcher binaries: `netflix/lib/*`,
  `/usr/lib/libcbe.so`, `/usr/lib/libflutter_engine.so`, `libwayland-webos-client`),
  luna-surfacemanager method usage (strings of SAM/WAM + OSE source),
  LumaShow compositor sizing, OSE source sizing, options and ranking.
- TV state left: untouched. Power state `Active`, picture mode `filmMaker`
  when read at 19:10. No tweak applied by this study. Nitin is using the
  set as of 19:20 (Main): only light read-only SSH until Main says it is
  free; no `ls-monitor`, no binary-wide greps.
- Continue with: `python3 arch/tv.py '<cmd>'` (paramiko helper), or the
  `tv()` function in the eval kernel.

## Findings log (chronological, raw numbers)

### DRM/KMS inventory (read-only, 19:13)

`arch/drmplanes` on `/dev/dri/card0`, driver `lg1k 1.0.0 (20210215) "LG
Graphics Output Driver"`, atomic and universal planes accepted, PRIME
import+export (cap 3), `ADDFB2_MODIFIERS` yes, `ASYNC_PAGE_FLIP` no,
cursor 128x128, no syncobj, one CRTC (38), one encoder (39), one connector
(31, type TV, connected, modes 1920x1080@60 / 3840x2160@60 / 1280x720@60,
all pixel clock 148.5 MHz: these are OSD canvas sizes, not panel timings).

Planes, all on the single CRTC, none with `zpos`, `alpha`, `pixel blend
mode` or `COLOR_ENCODING` properties exposed (debugfs shows the defaults
only):

| Plane | Type | Formats | Modifiers | State now |
|---|---|---|---|---|
| 32 | primary | AR24 XR24 AB24 XB24 | linear; AFBC `0x0800000000000062` (32x8 split sparse) for AB24/XB24 | fb 40, 1920x1080 AB24 AFBC, full CRTC |
| 34 | overlay | AR24 XR24 AB24 XB24 | same | **unused: no CRTC, no fb** |
| 36 | cursor | AR24 | none | **unused** (`hwcursor: false` in `/tmp/xdg/surface-manager.eglfs.json`) |

`surface-manager` (pid 7581) is the only DRM client and the master. It owns
three framebuffers, all AFBC AB24 and all `imported=yes` (dma-buf from the
Mali driver, not dumb buffers): 40 and 43 are 1920x1080 (double-buffered
`fb0`), 44 is 768x2160 (`fb1`, the second eglfs output), allocated but not
attached to any plane at the time of reading. `plane_mask=1`: the CRTC
scans out the primary plane only. The video path is not in DRM at all
(no YUV formats, no video planes): video runs through LG's `vsc`/`libvt`
mixer underneath the OSD, as expected.

