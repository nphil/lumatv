# Gaming

First-party plugin, **off by default**. Enabling Gaming permits temporary picture controls, controller-reader scheduling and supported controllers' persistent settings for `org.nphil.lumagame`. The app continues without the plugin; NDL's game-mode load already applies low delay, so the display controls are expected to be redundant.

## Display lease

All routes are loopback-only and refused while the plugin is off or the screen is off. Writes use the loopback API's `X-Luma-Client` header.

- `POST /v1/gaming/lease`: `{"pid":123,"controls":["memc_low_delay","vpq_low_delay","motion_pro","de_low_delay"]}`. Omitting `controls` requests all four. The response's `lease` contains `id`, `pid`, `leaseSecs:6`, `renewEverySecs:2`, `applied` and `skipped` with reasons. Unqualified controls are skipped without opening a device.
- `POST /v1/gaming/renew`: `{"lease":"<id>"}`, every two seconds.
- `DELETE /v1/gaming/lease`: `{"lease":"<id>"}`, after the stream stops.
- `GET /v1/gaming/status`: current lease, qualification records, pending rollback and the last restoration's readbacks.
- `POST /v1/gaming/restore`: retries a retained rollback journal when no lease is active.

The holder must be a non-root process in either gaming app's process tree. A kernel process handle (pidfd) detects its death without waiting for expiry. Expiry, standby, plugin-off and orderly helper shutdown restore saved values; an active lease checks standby at most 250 ms apart. Only one display lease is held. A random lease id is required for renewal and release.

Each qualified control is read before changing it. Originals are atomically saved to `/run/lumatv-gaming/rollback.json` before the first set. Set and restoration both require matching readback. Any set failure rolls back the entire transaction, including a failed ioctl that may already have changed the device. A rejected restore is resolved when readback already matches the saved value; that entry is removed from memory and disk, including before orderly shutdown. Other failed restorations retain their original values and block new leases until replay succeeds. A starting helper replays a same-boot journal even while Gaming is off; an older boot's journal is discarded. Qualification records are stored in the helper data directory at `gaming/qualifications.json` and apply only to the boot and device that passed.

## Qualification

No control ships qualified. `POST /v1/gaming/qualify` accepts `{"control":"memc_low_delay"}` only from the TV Settings client (`X-Luma-Client: lumatv-ui`), with no active lease or pending rollback. It discovers the device and performs **read → set → read → restore → read**, journalling the original before the set. Success and failure are both recorded, with originals, requested and returned values, device identity, time and errors.

A successful round trip records `sequencePassed:true`, not automatically `qualified:true`. The request additionally needs `lifecycleReceipt`, naming the TV evidence for restoration after stop, expiry, UI death, standby, plugin-off and helper restart. `de_low_delay` also requires `liveReadbackReceipt`: `de_kwrap_setlowdelaymode` caches its setting, so getter agreement alone does not prove hardware state. Receipt references are attestations supplied by the Settings client, not measurements produced by the helper. Missing evidence leaves the control unqualified. Physical latency improvement requires a separate external measurement and is never inferred from these records.

VPQ discovery matches `/sys/class/video4linux/videoN` to `/sys/devices/platform/v4l2_vpq`, checks the registered major/minor and opens the character device without following links. The fixed V4L2 controls are MEMC low delay `0x00983A01` → 1, Motion Pro `0x00983A03` → 0 and VPQ low delay `0x00983D0E` → 1 (`V4L2_CID_EXT_VPQ_LOW_DELAY_MODE` in LG's GPL kdriver headers). VPQ low delay remains unqualified. DE discovers the registered `/dev/lg/de0` character device and uses ioctl `0x4004613c` (DE IPC `0x49`) → 1. No getter ioctl is assumed: DE readback uses `/proc/lg/de/win0/status`'s `lowdelaymode`, pending live-hardware qualification. Requests cannot supply paths, ioctls or arbitrary values.

On the TV (2026-11-15, helper `43e74d46`) the MEMC low delay and Motion Pro round trips passed. The VPQ EINVAL result used `0x00983D09`, which is `V4L2_CID_EXT_VPQ_BYPASS_BLOCK`, not low delay; it does not establish low-delay support on O22. DE discovery failed because the old helper searched for a nonexistent `resource_mgr` node. Corrected VPQ low delay and DE discovery still need a TV receipt. Details: [Gaming display features, section 1](../lumagame-gaming-features.md#tv-results-2026-11-15-helper-43e74d46).

## Controller readers

`POST /v1/gaming/pad-boost {pid,tid}` and `POST /v1/gaming/pad-unboost {pid,tid}` retain the existing policy: at most four `gfn-pad-event*` threads, non-root gaming app ancestry, FIFO priority 10 with reset-on-fork, online CPUs 1–3 when CPU 1 is available, and a 90-second renewal deadline. `GET /v1/gaming/pad-boost` lists them. Scheduling and CPU affinity are saved and restored; the plugin's watcher sweeps only while a lease or boost is active. The legacy `/v1/gfn/pad-*` routes use the same table and plugin gate until app cutover.

## Stats overlay

`GET /v1/gaming/display` serves the panel's side of LumaGame's stats overlay: the video window's status (`/proc/lg/de/win0/status`: size, source rate, HDR type, low delay, mute), the refresh rate from the `DE2VSYNC` interrupt (`osd_irq` where it is missing) counted per 100 ms window (average, slowest window and fewest refreshes over the last second), and with `?full=1` the DE firmware's status words SYS_VERI_10/11 (low delay, VRR, FRC, frame delay, video delay, input and output rates). Sampling starts with the first ask and stops 2 s after the last; LumaGame asks only while its overlay is up over a running stream, and the setting ships off. The counters are read every 100 ms and the window status every 250 ms. `DE_IOR_REG_RD` fails a nonzero register with EIO and prints its value to the kernel log as an ERR line, where the value is read under the sampler's thread id; so the status words are read only for `?full=1`: at the first ask, when the window status changes and every 30 s.

`GET /v1/gaming/overlay` and `POST /v1/gaming/overlay {"mode":"off"|"compact"|"full","changed"?}` mirror LumaGame's setting as the `gaming.overlay` topic and keep it in the helper's data directory. LumaGame keeps its own copy; a change older than the one held is refused (`taken: false`) and the held one returned. Gaming owns the Game stats quick tile and declares Off / Compact / Full through the shared plugin stepper contract. Quick Settings reads `plugin.gaming.tiles` and sends the selected stable value as `target` to `/v1/plugins/gaming/actions/stats-mode`; changing the setting from either entry point republishes the same tile. The tile is withdrawn with the plugin and unavailable until its current choice is known.

## GameSir settings

`GET /v1/gaming/gamesir` discovers a Cyclone 2 configuration interface and reads firmware, active profile and settings.
Its `state` is `ready`, `mode` (a controller without the configuration interface), or `missing`. Plugin-off and standby refuse
the request before discovery. `POST /v1/gaming/gamesir` accepts only `{setting,value}`: `report-rate` (250/500/1000),
`rumble-left` / `rumble-right` (0–4), `dpad-diagonal-lock` / `xinput-abxy-swap` (boolean), and `deadzone-left` /
`deadzone-right` (inner stick dead zone, 0–20%). Paths, driver IDs and firmware operations are not accepted.

Writes read the active profile first, preserve unrelated bytes and require matching read-back. Success returns the confirmed
profile with `verified:true` and `check.state:"match"`; a mismatch returns HTTP 409 with byte offset, wanted and found values.
One HID request runs at a time, outside the display lease lock. Each output report rechecks the plugin and screen gate.
Controller settings remain stored after Gaming is disabled; expanded consent names that persistence. Hardware access and
read-back still need TV acceptance.

## Leadjoy settings

`GET /v1/gaming/leadjoy` accepts `section`, `profile`, `sub`, `side`, `button`, `action` and `step` selectors. Defaults are
`general,1,0,0,1,0,1`; sections are `general`, `profiles`, `buttons`, `macros`, `sticks`, `curves`, `directions`, `triggers`,
`motion`, `gestures`, `rumble`. `button` is the macro number in `macros` and the gesture event plus one in `gestures`.
The response supplies the native settings schema, confirmed values and selected view. Profile count comes from the device.
`POST` accepts `{view:{section,profile,sub,side,button,action,step},setting,value}`; settings and ranges are fixed by that
section's schema, not arbitrary commands or paths. Selectors are read-only; `profile-activate`, the macro step actions and
`delete-macro` are separate explicit actions. A view whose macro, step or gesture no longer exists is refused with 409.
A write the controller acknowledged but did not read back returns 502 `unverified`. A macro edit is a delete followed by a
write; a failed write after the delete returns 502 `rewrite_failed`.

Discovery matches USB `4131:3519/8008`, the exact FF80 vendor descriptor, product `Xeno` and valid c190 lighting. Unknown
products are read-only. Each write rereads the complete configuration object, preserves unrelated fields and demands an ACK
plus matching read-back; writes are never retried. Plugin/screen gates are checked before discovery and every report transfer.
Logo lighting uses one RGB triplet and brightness. Hidden c191/c192 effects, firmware operations, ACK-only commands
(calibration, macro recording, reset/save/reload, sub-configuration triggers) and settings without complete vendor contracts
are not exposed. Read-back confirms application, not flash persistence or physical behaviour.

Needs TV: corrected DE discovery; DE status provenance versus the kernel cache; visible effect of every set; lifecycle restoration paths; same-boot helper-crash replay; pidfd support in the TV kernel; app-jail access to loopback; controller scheduling effect; physical latency comparison. No TV acceptance or speed improvement is claimed. The detailed qualification contract is [Gaming display features, section 1](../lumagame-gaming-features.md#1-allm--low-latency-partly-measured).
