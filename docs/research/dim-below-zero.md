# Dimmer than OLED pixel brightness 0

Research only; nothing was written to the TV. Goal: a dark-room level below LG's `picture.backlight` 0, with no pixel switched off and no wear or compensation cost. TV: OLED65C4PUA, webOS 10.2.1, chip O22A3, `panelBacklight 4 (OLED)` (`/proc/lg/ovi/status`). Facts marked [INFERENCE] are not observed on this set.

## Where `backlight` goes

- `picture.backlight` is 0..100 step 1, dimension `[input, pictureMode]` (`getSystemSettingDesc`). pqcontroller maps it through `gCoeffUiDimmingCurve` in `libpqdb.so` (identity: 0,20,40,60,80,100 to the same), so the UI scale is linear.
- The dimming engine (`pqcontroller`, `_VPQ_ENGINE_*` around 0x7fd00-0x81400) turns the UI percent into `_dimPercentPWM`, scales it by `_gOledAiLuminance`, clamps it to a PWM min and max (log line "By PWM Min&Max ragne", `pwmMax`), and keeps the result as `dimmingPwm`. `getInfo {"key":"backlightVal"}` reads it back: 60 at UI 15 (eco, Energy saving auto, 1 lux). That readback is an exact, cheap probe for the whole chain.
- No SoC PWM carries the OLED light. `/etc/panelcontroller/db/panelcontroller.db.pwm.json` has no OLED rows (panelcontroller logs "No PWM DB for OLED"), `be_pwm_hw_o22.c:298` marks the PWM block `lcd = 0` on OLED, `/sys/class/backlight` and `/sys/class/leds` carry no panel node, and `/proc/lg/be/pwm_duty` shows only `mask:0x2 duty:252` across days. LG's PQ docs say OLED light goes to the TCON (`TPC_RECOVERY_PARAM`: "PQ controls oled light via TCON"). pqcontroller and panelcontroller reach the TCON over I2C (`OLEDMODULE_Write*`, `oled_i2c_isTconAccessible`, `isOledTconAccessible`).
- Floor: no light floor or min table is held in a data file. `/etc/panelcontroller`, `/etc/configd/layers/base/com.webos.service.pqcontroller.json` (`pqCustomVersion LG`) and `pqsettingsEnv.json` were read; none holds one. The UI value to TCON value step and any minimum sit in `pqcontroller`/`libpqdb`/TCON firmware. Not decoded; changing it would mean patching system binaries, which is out of bounds.
- Nits: no table on the set. A measured neighbour (AVS Forum, LG C5 83", Filmmaker, JETI 1211 + K10a): OLED light 10 = 98 nits, 24 = 151, 54 = 248, 100 = 409, about 3.5 nits per step. That extrapolates to about 60 nits at 0 [INFERENCE for the C4]. At 0 the panel is therefore still near 60 nits peak white, which is why a lower level is wanted. The set's own value is measured in the test plan.
- LumaTV's panel dim (`paneldim.rs`, `docs/research/panel-dim-result.md`) never goes below `FLOOR` 10, so it is not a lever either; its value-to-light step was never measured.

## Options, best first

### 1. Dedicated night preset on LG's own keys (backlight 0 plus contrast)

Safest and cheapest. Every value is inside LG's range, per picture mode and input.

- `backlight` 0 (above).
- `contrast`: Range 0..100, `[input, pictureMode]`; currently 85 in eco. A digital gain on the signal, so it lowers peak white and the shadow-to-white ratio at once. Cost: fewer code values below white [INFERENCE: the PQ blocks run above 10 bit and dither, so a cut to about half is under one bit; verify on a ramp]. It leaves the near-black end alone, which is where non-uniformity lives.
- `gamma` low/medium/high1/high2 (`[input, pictureMode]`, row visible but `active:false` at the moment): high2 darkens mid-tones, not peak white. Useful for a dark-room look, no peak effect.
- `adjustingLuminance` -50..+50, 22 points, `[input, pictureMode]`, `visible:false`: a per-point luminance trim of the white-balance grid; step sizes too small to matter and the owner's tuning lives there. Not used.
- `energySaving` auto/off/min/med/max is global (`dimension []`), so a preset cannot carry its own value. `VPQ_ENGINE_GetEnergySavingDim` (0x7f845) reads a four-byte table at 0xb9ca0: 65, 35, 0, 100 for levels 1-4 (min, med, max, screen off by the string order). Whether it is a cap or a reduction is not decoded; min/med/max stay a measurement item. Auto is the sensor-driven dim that auto brightness already replaces (`docs/AGENTS.md`), and it moved `backlight` 68 to 75 by itself in one earlier run, so it must be `off` while a night preset is measured.
- `eyeComfortMode` on/off, global: a warm filter that removes blue, so it lowers light slightly and shifts tint. Optional extra, not a dimmer.
- `peakBrightness` is already `off`; nothing below it. No `aiBrightness` key exists in the settings DB ("no result in DB").
- `oledBrightness` is not a settings key; it is `backlight`.

Build: a preset row (mode plus keys) in the existing picture-preset path; no new TV-side code. Risk: low. Auto brightness stays compatible (its floor is half the mode's level, so at level 0 it has nothing to take).

### 2. pqcontroller `oledAiLuminance` (best candidate for a true level below 0)

LG's AI-brightness hook. `setProperties {"oledAiLuminance": N}` reaches `controller_setOledAiLuminance`, then `DIL_VPQ_ENGINE_setOledAiLuminance` (0x7fcd1), which accepts 1..100 and stores it at +0x2c. The dim calculation (0x80646-0x80682, log "_gOledAiLuminance[%d]") does `_dimPercentPWM = AI * _dimPercentPWM / 100`, so 50 halves the dim percent after the UI curve, inside the same chain as `backlight`. If the later steps keep resolution, this is a panel-side cut with no bit-depth cost and a floor 100 times lower than the UI step.

Unknowns: the branch that uses it may run only with LG's AI brightness on; LG's AI service may overwrite it; the PWM clamp may floor it; the final TCON quantisation is not decoded. All of these show in one get-only probe: `backlightVal` before and after, and `eyeQLux`.

Build: a small helper call set/clear plus a restore on mode exit and standby. Risk: medium-low (LG-internal knob, runtime-only [INFERENCE], no raw register). Gate behind the TV test below.

### 3. LumaTV GPU dim (`u_dim`)

Exists (`oledcare`, `paneldim` fallback, `boxdim`). `render/gl.rs` line 248 scales colour by `u_dim` per item; `scene.rs` `Plan::sole_fullscreen` requires gain 1.0, so a dimmed frame is always composed. `docs/research/oledcare-result.md`: "An app's card, a video, the moving wallpaper's plane ... are never moved or dimmed"; `oledcare-apps-result.md`: a hole's video "on the panel at the same brightness". Confirmed: it scales the shell, apps drawn into buffers and Settings, and never the hardware video plane (HDMI inputs, Netflix and Plex video, anything shown through a hole). Costs: 8-bit buffers lose a bit at half gain (banding in the UI's gradients), and a full-screen frame is no longer scanned out directly, so the GPU composes every frame. Use only as a UI-side complement to option 1 or 2, never as the whole answer.

### 4. pqcontroller `reduceLuminance`

`reduceLuminance {tpcMode, leadTime, luminance, temporalFilter}` ends in `_pq_reduceBacklight4Gallery` (0x8e581), which only ramps the UI backlight byte down toward the target. The target 0 equals UI 0, so it cannot go lower; it is also the TPC protection hook. No gain; do not use.

### 5. Output-path gain in the display block (OSD gain, CSC, post gain)

`V4L2_CID_EXT_VBE_OSD_GAIN` (OLED only) scales the OSD plane, not video. Gain in the video path (output CSC, post gain) is only reachable through raw registers (`be_reg_dpe_o22a0.h`; `DE_IOW_REG_WR` writes any physical address) in the backend aperture that also holds OLED compensation (`APB_OLED_BASE 0xCC403000`), demura and the panel link (`docs/research/display-chip-direct.md`). A digital cut there lands at the final 10-bit link, probably with no dither, so near-black banding is worst where it matters [INFERENCE]. Raw writes are not the chosen route. Do not pursue.

### 6. TCON light write (`writeOled`/`readOled`, `LED_TCON_write`, `DILE_VBE_Write4KTcon`)

The only way to a lower panel floor than LG's. The TCON holds compensation (JB LUT, offset), power sequence and light in one I2C space, and panelcontroller gates access during compensation (`isOledTconAccessible`, an expiry timer). Out of bounds: it risks compensation and the panel. Do not pursue.

LSR, GSR, TPC, IRR, CPC and the orbit are protection or compensation features (`v4l2-ext-vbe.h`, `v4l2-ext-pq.h`). None is a dimmer and none may be changed.

## Panel health and the floor

Less light is less wear: OLED ageing follows drive current and is steeper than linear [INFERENCE: industry rule], so a lower night level only helps. The risks at low drive are on image quality, not life:

- near-black non-uniformity (vignette, dirty-screen bands) shows as drive falls;
- tint shift in near-black and dark greys (W-OLED chromaticity moves at low drive);
- banding where a digital gain drops code values (options 1, 3 and 5);
- flicker if the panel's own light control chops at very low levels [INFERENCE; check by eye and with a slow-motion phone clip].

Compensation cycles (the pixel refresher, JB, standby-time compensation) and Screen Shift, Logo Luminance and TPC/GSR stay as LG sets them; none of the options touches them. Safe floor: the lowest level at which a 5% grey patch is still clearly distinct from black, the 0-10% ramp has no visible banding, and a 5% grey field shows no tint or dirty-screen bands. A peak white of about 20 to 30 nits is the expected result [INFERENCE]; the test decides.

## Recommendation

1. Ship option 1 as a night preset: `backlight` 0, `contrast` lowered to the measured step, nothing else.
2. Test option 2 on the TV in the same session. If `oledAiLuminance` holds against LG's own AI and `backlightVal` falls with it, use it for the final levels and keep contrast near its normal value.
3. Keep the GPU dim for the shell only. Leave options 4 to 6 alone.

## TV test plan (owner present, dark room, SDR, one change at a time)

1. Snapshot the mode's settings; record `backlightVal` and `eyeQLux`. Test patterns: 5% grey and 2.5% grey windows on black, a 0-10% ramp, a full 10% grey field, a full white field.
2. Ladder A, preset: `backlight` 15, 10, 5, 0 (read `backlightVal` at each: shows the mapping and where it bottoms out); then at 0, `contrast` 85, 70, 55, 40. At each step: `backlightVal`, `eyeQLux` on the full white field (relative only; the sensor reads whole lux and the screen's own light gave 4 to 5), a phone lux reading at a fixed spot and exposure (relative), and the owner's verdict on the five checks in the floor paragraph.
3. Ladder B: with `backlight` 0 and `contrast` back at normal, `oledAiLuminance` 100, 75, 50, 25, 10; each held 30 s to see whether LG overwrites it; `backlightVal` and `eyeQLux` each time; same owner checks. Reset to the original value after every step.
4. Pass: the floor paragraph holds at the chosen level; `backlightVal` is stable for 5 minutes; nothing else (Screen Shift, refresher, standby) changes. Record the receipt path. Restore the snapshot at the end.
5. Not tested by this plan: HDR and Dolby Vision (separate contexts; the sub-zero level is SDR only), the standby compensation cycle across a night, the C4's real nits at 0.
