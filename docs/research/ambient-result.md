# Ambient light, phase 4: capture probe and helper-side modules

## Resume here

Done (2026-10-25, workstation only, the set has not been touched yet by this task):

- The four `vtCapture_*Histogram` signatures, the data layout, the 16/14-area preset geometry, the return codes and
  the process allow-list were recovered offline from `libvtcapture.so.1.0.0`, from LG's own caller
  (`hue_sync_service`) and from LG's published kernel header; see "ABI" below. Header: `ambient/capture/vthist.h`.
- 32-bit probe `ambient/capture/probe.c` (`./build.sh`, `./test.sh`): builds for the set
  (`arm-linux-gnueabi.2.28`, the same target as the other 32-bit pieces), host logic tests pass against a fake
  library (finalize on SIGTERM, on LG policy action, deny path, sweep, EAGAIN accounting).

Next, in order:

1. A read-only TV slot: confirm the pulled libraries match the workstation copies, whether
   `/var/luna/preferences/vt_devmode` exists, who holds `/dev/video60`, and what audio tools the set has.
2. A probe slot: 60 s at Home, 60 s on an HDMI input if one is active, 60 s of muted Netflix; then the two audio
   captures. The probe needs `vt_devmode` (or an LG-allowed process name, see "Permission gate") to get past
   `initHistogram`.
3. Helper-side Rust modules in `ambient/` (not started).

## ABI

Everything below is `libvtcapture.so.1.0.0` on the set's webOS 10.2.1 (32-bit ARM, Thumb-2, built from LG's
`lib32-screen-capture-webos 1.0.0-56`); the workstation copy is `research/zones/libvtcapture.so.1.0.0`
(byte-identical to the census copy). The kernel side is documented in LG's header
`v4l2-ext-vt.h` (`research/lgsrc/.../v4l2-ext-renderer-header/linux/`), which matches the binary field for field.

```c
typedef bool  (*vth_policy_cb)(void);                         // LG's resource manager taking the capture back
void *vtCapture_initHistogram(vth_policy_cb cb);              // handle (a VtHistogram*) or NULL
int   vtCapture_setPresetHistogram(void *h, int preset);      // 0..3; must precede the first get
int   vtCapture_getHistogram(void *h, struct vth_zone *out);  // out: N entries, 16 always enough
int   vtCapture_finalizeHistogram(void *h);

struct vth_zone { uint32_t area_id; uint16_t r[256], g[256], b[256]; };   // 0x604 bytes
```

- Data: per area, one 256-bin histogram per channel (8-bit code values, `u16` counts), `area_id` 0..N-1. The mean of
  a channel is `sum(i * bin[i]) / sum(bin)`. Whether the counts are raw pixel counts or scaled is for the probe to
  show; Hue Sync copies the three arrays verbatim into the same layout its own software fallback fills.
- Areas are rectangles in the picture (0,0 top left, fractions of width and height), not a grid, and overlap.
  Hardware presets (from LG's header, copied into the Rust zone table):
  - 0 video, no bars: 16 areas. 1 top/bottom bars skipped (picture rows 0.1265..0.8735). 2 left/right bars skipped
    (columns 0.13..0.87). 3 games: 14 areas, more weight on the edges.
  - The black-bar handling HyperHDR does in software is therefore a preset choice here: 1 for letterboxed films, 2 for
    pillarboxed, 0 otherwise; the helper still needs a detector to pick (see the mapping design).
- Return codes (from the ioctl errno): get 0 ok, 2 not started, 3 null out, 7 EOPNOTSUPP, 11 EAGAIN, 13 ENODATA,
  17 EACCES, 1 other. Init: NULL only; the reason is a PmLog line in context `capture` (`/var/log/messages`), which
  the probe prints after a failure.
- Control ids on `/dev/video60`: START 0x982a14, PRESET 0x982a15 (value 0..3), DATA 0x982a16 (size N*0x604),
  STOP 0x982a17, through `VIDIOC_S/G_EXT_CTRLS`. The header's own text says the driver captures SVP and DRM video
  inside the secure world and returns histogram data from it, which is what makes the API work under Netflix.
- Hue Sync's own sequence: `init(cb)`, `setPreset(h, mode)` with mode 0 unless it is 1..3, then a poll thread that
  calls `get` and hands the 16 areas to its Hue stream; `finalize` on stop.
- `cb` is called from the library's thread; Hue's callback logs and returns false. The library does not use the
  return value.

## Permission gate in initHistogram

`vtCapture_initHistogram` refuses the call (returns NULL, "your process has no permissions" in PmLog) unless one of
these holds, read from `/proc/<pid>/cmdline` and the file system:

- the command line contains `/com.lighting.huesync.service/hue_sync_service`, or
- the file `/var/luna/preferences/vt_devmode` exists ("dev mode on" is logged and the check passes).

The second is LG's own development switch, the first is what LG's Hue Sync service satisfies. Naming a process after
Hue Sync would pass the check by impersonation and is not used. Creating the flag file writes outside the set's
approved write list and relaxes the gate for every user of the library, so it needs Main's decision; the probe
reports whether it already exists.

Other facts from the same code: `initHistogram` also fails with code 11 when the resource manager's `acquire` is
refused, which is the "second `vtCapture_init` fails with 11" seen before (one VT consumer at a time).

## Open

Zone values, update rate, latency, CPU, audio: need the TV slots above.
