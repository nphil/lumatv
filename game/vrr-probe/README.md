# LumaGame 1080p cadence experiment

**Host preparation only. No TV runner or install.** The question is whether a TV-native video window follows individual picture intervals rather than delivering the same pictures on a fixed refresh grid. The timer, the visible picture transitions and physical panel refresh are three different measurements.

## Prepare the pictures

From this directory, on a Linux host with Python 3.10+, FFmpeg (libx264, drawtext), ffprobe and Liberation Mono:

```sh
python3 -B -m unittest -v test_probe
python3 -B stimulus.py "$CODY_SCRATCH/vrr-frames"
```

The output directory must not exist. `--font PATH` selects another local font. The three CSVs index the **same** 720-picture Annex B H.264 file, one access unit per ID, no B-frames. The headers describe 1080p60 progressive 8-bit SDR; ordinary file playback is **not this test**. The intended output runner supplies the per-picture rate separately. `target_ns` is the planned start relative to the phase, not an actual send time. `period_ns` is exact, including fractions. For adjacent IDs, row *i* describes the dwell from *i* to *i+1*; the last dwell has no visible closing edge unless a subsequent marker is shown.

| Phase | Requested dwell | Duration |
|---|---|---:|
| `fixed120` | 1/120 s | 6.00 s |
| `shuffled` | 360 × 10 ms and 360 × 16 ms, deterministic order | 9.36 s |
| `same-mean` | 13 ms (1000/13 Hz) | 9.36 s |

The alternating patch uses gray levels 24/72 rather than black/white. The frame number moves horizontally every 60 frames, on the same scanline as the patch. No phase needs to loop or change panel brightness.

## Why the TV run is blocked

In the related LG O22 source (`core/vdec`), `vo_release` → `vdisp_close_instance` → `vsync_disconnect` disables the last owner's timer. `hal/vsync.c:228–241` gates the timer and interrupt but does not restore the two half-period registers or the scan-type bit. This is distinct from the earlier ForceVRR experiment; **no reboot requirement has been established for this path**.

On a **fresh, unshared** channel with a valid rate, `vds/vsync.c:183–199,267–298` selects the new owner's rate. `enable_vsync_dev` (`:144–160`) calls `vsync_hal_set_field` *before* `vsync_hal_enable`. `hal/vsync.c:243–285` writes both half-periods and changes the scan-type bit **if it differs**. Thus its value is established, not necessarily rewritten. This is not restoration of the old owner's values; shared and already-enabled paths do not provide that guarantee. A saved [retail playback trace](../../docs/research/framerate-result.md#round-11-when-the-return-to-120-hz-happened) records stale source timing at the next connection; it is not a raw timer-register dump.

**The retail ARM host does not expose `/proc/lxvdec/reg/vsync`.** On kernel `5.4.268-329.ptl4tv.2` at awake idle, that path returned `ENOENT`; `/proc/lxvo/vdisp/vsync` returned only `#ENDOFREAD_0000`. Receipt: `vrr-idle-baseline-20261121T191449/capture.txt`, SHA-256 `8412e638689cbd68009be47576671e58d3798db1f90ab493c209320fa63535bf`.

In the O22 build, `hal/proc.o` belongs to the decoder processor (`core/vdec/Makefile:53–101`), not the ARM host. The host forwards five named display reports, not the raw register dump (`ipc/local/proc.c:345–357`; `ipc/remote/driver.c:120–148`). Opening the host's `vsync` report takes a diagnostic-pipe mutex and sends a request to the decoder (`ipc/local/proc.c:243–274`). Disabled channels have no rows (`vds/vsync.c:668–680`): the end marker does not encode zero field counts. A playback capture through the same report cannot recover the idle tuple.

`compare_timer.py` understands saved dumps in the O22 **ARM address layout** only; it does not acquire them. It cannot compare the retail receipt above. For two complete dumps independently obtained in that format:

```sh
python3 -B compare_timer.py before.txt after.txt
```

The comparator prints both field counts, field-selection and enable bits for each channel; unequal or malformed receipts fail. Equality does not establish the rest of the display state. No authenticated idle baseline for the selected retail channel, restoration receipt or safe crash-recovery path is available here. In particular a zero/zero idle pair cannot be recreated by any 24–120 Hz picture. A guardian holding only a PID cannot restore it by closing that process. No raw register writes, DE commands, ForceVRR, reboot, or guess at a free channel are part of the test.

**Parked, not awaiting a TV slot.** Functional handback to stock playback was accepted as a weaker alternative to exact register restoration. Neither would establish that the panel follows individual frames. The [4K feasibility gate](../../docs/research/decoded-vrr-4k.md) supersedes this 1080p experiment; there is no decoder runner to install.

## Archived capture design (not a run sheet)

1. **0–2 min, baseline.** Record firmware identity, the assigned decoder/window/channel, owner list, pre-run raw fields, output rate and normal UI state. Abort on another owner, missing readback or a baseline the runner cannot restore.
2. **2–4 min, fixed 120.** Film the corner patch and adjacent number together. Use actual 240 fps or higher, tripod, fixed focus/exposure and short exposure (about 1/1000 s if available). No interpolation; keep the original file with capture timestamps. Start before ID 0 and stop after 719; allow about 12 seconds per clip.
3. **4–7 min, shuffled then same-mean.** Repeat once per phase without moving the camera. Save the phase label next to each original. The trial needs monotonic queue/send and release timestamps, frame IDs, drop/repeat results and independent timer readbacks, not just requested periods.
4. **7–10 min, teardown.** Compare the saved raw fields and ownership before/after; confirm the window is gone and ordinary playback resumes. A difference, unreadable state, hang or time limit ends the test. No automatic retry or reboot.

At 240 fps one camera sample spans 4.17 ms. Use many transitions aligned by frame ID and compare against both the requested sequence and a fixed 120 Hz grid, allowing a single consistent pipeline delay. Report missed/duplicated IDs, measurement uncertainty and offsets; a two-bin histogram is insufficient. A changing patch shows **visible content transitions**, not invisible panel refreshes that repeat a picture. If the two timing models cannot be separated, report *inconclusive*, not panel VRR. No 4K, HDR, network streaming or arrival-triggered VRR claim follows from this experiment.
