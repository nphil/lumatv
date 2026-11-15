# GameSir Cyclone 2: configuration protocol, modes and the TV

How GameSir's own software configures the Cyclone 2 and its 2.4 GHz receiver, decoded from GameSir Connect 1.14.5 and
encoded in `game/crates/gamesir-config`. Tags: `Connect:` read from Connect's main-process bundle (line numbers refer to
the prettified `connect-pretty/main.js`), `GLT:` hardware captures in gamesir-linux-tools, `TV:` measured on the set,
`[INFERENCE]` not measured. Nothing here has been run against a real Cyclone 2.

## Resume here

Done: protocol, modes, firmware path (documented only), the TV's driver situation, and the crate, which is tested on the host
against a fake controller only. Open: the real-device checks at the end.

## Sources

| App | Version | Download | sha256 |
|---|---|---|---|
| GameSir Connect (Windows) | 1.14.5 | `https://pc-connect-update.gamesir-cdn.com/1.14.5/GameSir%20Connect%20Setup%201.14.5.exe` | `6524cdee294a2e7705bc0491cd5644d61fd08c1dba553494f77e7ea588979277` |
| GameSir App (Android, XAPK) | 5.9.22 | APKPure mirror, URL in `GameSir_v5.9.22_apkpure.url` | `e1f8b03c6af913bd2e95064f5724541a0b20ba7b1a115a573a54ca2c062bd8f8` (base APK `be6776a7a042ff2a8d6af673dcedce16773c8cc149f7528557d370e8627e8d34`) |
| GameHub (Android) | 6.3.2 | `https://wwwdl.mx520.com/ux-landscape/app-apk/b8cc/9d/9f/GameHub_6.3.2_b8cc9d9f2bdfbc5eaf91799a5ea69c10.apk` | `e51456bd3fb1c041dbdf3c756878ce1ddb37dfdb933b266142b22354b7d50ed2` |

Installers, decompiled trees and copies of the references below are in `/data/home/tmp/lumagame-rea-20261115/gamesir/`,
outside git. Connect is the only one of the three that configures the pad over USB. The GameSir App's USB list
(`com.xiaoji.sdk.usb.device.GameSirUsbInputDevice`) has no Cyclone 2. GameHub knows it only by its Bluetooth name
(`sre`: "GameSir-Cyclone 2", key `gamesir_c2`), a Bluetooth path not studied here.

Corroboration from projects that drive real pads:
[gamesir-linux-tools `docs/RESEARCH.md`](https://github.com/broroeror/gamesir-linux-tools/blob/main/docs/RESEARCH.md)
(command captures, a 30/30 answer sweep on `3537:1053`, the firmware loader),
[cyclone2-linux](https://github.com/vdemonchy/cyclone2-linux) (modes, battery and RGB in X-input mode),
[steam-for-linux #13311](https://github.com/ValveSoftware/steam-for-linux/issues/13311) and
[r/linux_gaming, 2024-12-19](https://www.reddit.com/r/linux_gaming/comments/1hhllnt/gamesir_cyclone_2_controller_on_linux_steamos)
(mode switching on Linux).

## Modes and USB identities

| Mode (Home LED) | Receiver id | Cable id | Linux driver | Configuration |
|---|---|---|---|---|
| X-input (green) | `3537:102A`, ADC variant `3537:100B` | `3537:101D`, ADC `3537:1053` | `xpad` on the vendor-class gamepad interface | HID usage page 0xFFF0 beside it |
| DS4 (blue) | `054C:09CC` | | `hid-sony` / `hid-playstation` | none |
| Switch (red) | `057E:2009` | | `hid-nintendo` | none |
| HID (yellow), or receiver with the controller off | `3537:0575` | | `hid-generic` | declared, answers nothing |

`Connect:` the model table `am` (34391-34452) gives the four configuration ids (`ConfigModePid` 4125, 4138, 4179, 4107),
`UsagePage: 65520` (0xFFF0) and `GamepadUsagePage: -1`, meaning the gamepad side is not HID. `ConfigMethods: []`: Connect has no
command that changes the mode, and it connects only in X-input mode (GameSir's Connect page, cited by cyclone2-linux).
cyclone2-linux lists the other three identities and finds the vendor interface gone in DS4 and Switch mode. The ADC variant
has its own ids and firmware line (`GS_C2_ADC_DEVICE`, 3.x); Connect drives both through the same proxy class `ys`
("proxyC2", 13331).

The mode is set on the pad. Holding View+Menu for about 2 s cycles X-input, Switch and DS4 and survives power-off. The
power-on combinations (Home+X and similar) last one session, and on Linux hosts the pad falls back to DS4 within a second
(Reddit thread, Steam issue).

## Transport

- The configuration interface is a HID interface whose descriptor carries usage page 0xFFF0 with output report 0x0F and
  input reports 0x10 (replies) and 0x12 (gamepad state), each with 63 bytes of payload. Every report is 64 bytes with its
  id; there is no checksum or sequence number.
- `Connect:` `addDevice` (12967) opens the node with node-hid and writes the heartbeat `0F F2 00` at once; a 1 s timer
  (13055, `W` 7068) repeats it while Connect's window has focus.
- `Connect:` queue `Uo` (12957-13109) keeps one request in flight, resends it after 200 ms and drops it after the tenth send.
  `ys.receivedData` (13621) treats every input report except 0x12 as a reply, without checking its id. Any reply kind other
  than busy Ack, ReadRGBAck and ProfileChanged counts as the acknowledgement. `GLT:` replies arrive as `10 05 …`, `10 0C …`.
- `GLT:` a wired pad can expose two configuration nodes; one streams an empty 0x12 report and stays silent.
- `GLT:` a sleeping controller leaves the receiver at `3537:0575`, which streams all-zero 0x12 reports and answers nothing.

## Commands

All bytes as sent, zero-padded to 64. Builders are `Ro` methods (12798-12888).

| Bytes | `Ro` builder | Purpose | Crate |
|---|---|---|---|
| `0F F2 00` | `getHeartBeatCommand` | keep-alive; `0F F2 01` is Connect's button-test mode | yes |
| `0F 09` | `getReadFWVersionCommand` | firmware versions | yes |
| `0F 0B` | `getReadCurrentProfileCommand` | active profile | yes |
| `0F 07 n` | `getSwitchProfileCommand` | make profile n (1-4, shift 5) active | yes |
| `0F 04 n hi lo len` | `getReadProfileCommand` | read `len` ≤ 58 bytes of region n at offset `hi lo` | yes |
| `0F 03 n hi lo len data…` | `getWriteProfileCommand` | write `len` ≤ 58 bytes | yes |
| `0F 20 66 55 L R` | `getVibrationCommand` | run the motors (0-255) until the next one | yes |
| `0F 0D` | `getReadRGBCommand` | read the current colours | no |
| `0F 0D s f` | `getSetLightStatusCommand` | play (`s` 1) or pause (0) the animation on keyframe `f` | no |
| `0F 10 n hi lo len` | `getRefreshProfileCommandByOffset` | re-read after a ProfileChanged notice | no |
| `0F 16 k s` | `getSetMacroStatusCommand` | macro recording on L4 (`k` 1) or R4 (2) | no |
| `0F 04 n 03 9D 07` | `getReadDeviceInfoCommand` | 7 bytes at 0x039D, past the profile | no |
| `0F 03 n 03 9E 06 ascii…` | `getSetDeviceIdCommand` | rewrite the device id | never |
| `0F FD x` | `getSetCalibrationStateCommand` | stick, trigger and sensor calibration | never |
| `0F FC` | `getEnterUpgradeModeCommand` | enter the firmware loader | never |
| `0F 17 55 88` | `getQuickUpdateCommand` | enter the firmware loader ("manual update", `ys` 13555) | never |
| `0F 01`, `0F 02 x` | `getEnter/ExitProfileConfigCommand` | not used by `ys` | no |

## Replies

Byte 0 is 0x10 and byte 1 the kind, named by `wo` (13143).

| Kind | Name | Layout |
|---|---|---|
| 0x05 | ReadProfileAck | `[2]` region, `[3..5)` offset big-endian, `[5]` length, data from `[6]` (`ys.putDataToProfile`, 13710) |
| 0x06 | Ack | `[2]` 1 = busy, so the request is resent |
| 0x0A | ReadFirmwareVersionAck | controller ASCII `[4..9)`, receiver ASCII `[12..17)`; each digit is a dotted part, "123" is 1.2.3 |
| 0x0C | ReadCurrentProfileAck | `[2]` slot; 0 means 1 |
| 0x0E | ReadRGBAck | colours |
| 0x0F | ProfileChanged | `[2]` region (0x30: the active profile changed), `[3..5)` offset, `[5..7)` length; the pad changed settings itself |
| 0x11 | RefreshProfileAck | |
| 0xF0-0xF4, 0xFC | Download, DownloadAck, HeartBeat, ReadKeyStatus, ReadKeyStatusAck, RequestToUpgrade | |

Which reply acknowledges a write or a switch on the Cyclone 2 is unknown: Connect would take almost any, and GLT records
none for writes. The crate accepts an Ack or an echo of the command and otherwise decides by reading back (writes) or by
asking for the active profile (switches).

## Settings blocks

Regions 1-4 are the profiles and 5 the shift layer, 680 bytes each; region 0x20 is the lighting, 635 bytes. Reads go in
58-byte chunks: 12 for a profile, 11 for the lights. `Connect:` `ys.writeProfile` (13721) sends only the span from the first
to the last changed byte (`Oa.bytesDiff`), and nothing commits afterwards; `Ro` has no save command. `GLT:` a profile takes
writes only while it is the active one, and Connect switches to a profile before editing it. The crate switches, writes,
reads back, then switches back.

Profile layout (`$a.fromBytes` / `fromModel`, 12451):

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 32 | name | ASCII, NUL padded |
| 32, 33 | 1 each | left, right motor | 0, 25, 50, 75, 100 (Connect's levels 0-4); 34, 35 are the trigger motors |
| 36 | 5 | audio | not decoded |
| 41 | 2 | shift | not decoded |
| 43 | 1 | D-pad diagonal lock | 0 / 1 |
| 44 | 1 | X-input A/B and X/Y swap | 0 / 1 |
| 45 | 1 | Switch A/B and X/Y swap | 0 / 1 |
| 46 | 1 | report rate | 0 = 250 Hz, 1 = 500 Hz, 2 = 1000 Hz |
| 64 | 16 × 7 | buttons | `[turbo on, turbo rate 0-30, remap on, out1, out2, out3, toggle]` for Up Down Left Right LB RB LS RS A B X Y Home View Menu Share |
| 176, 335 | 159 each | L4, R4 | the same 7 bytes, then a macro |
| 494, 522 | 28 each | LT, RT | +2 deadzone (5), +7 remap (5), +12 hair trigger `0x80 \| 1` rapid or `0x80 \| 2` fixed, +13 start, +14 end, +15 curve (13) |
| 550, 582 | 32 each | LS, RS | +0 enabled, +1 raw trace, +2 deadzone (5), +7 curve (13), +20 remap (12) |
| 614, 647 | 33 each | motion aim, tilt | not decoded |

Face buttons are stored in PlayStation order and named that way in Connect: A is cross, B circle, X square, Y triangle.
Output codes: 1-20 the buttons (Up, Down, Left, Right, LB, RB, LS, RS, A, B, X, Y, Home, View, Menu, Share, L4, R4, LT, RT),
21-28 stick directions (LS up, down, left, right, then RS), 50-155 keyboard keys, 200-206 mouse buttons and wheel, 255
disabled. A deadzone is `[on, inner, outer, anti_inner, anti_outer]` in percent. A curve is `[on, type (0 linear, 1
concave/convex, 2 S, 3 custom), curvature 0-100, 0, 0, x1, y1, x2, y2, x3, y3, 255, 255]`, each point scaled to the
deadzone as `round((p − low) / (high − low) × 255)` with JavaScript rounding. Connect's default stick bytes are
`01 00 01 0A 64 0A 64 01 00 64 00 00 2B 2B 71 71 B8 B8 FF FF`.

Lighting (`qo`, 13200):

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | active animation slot, 0-3 |
| 1 + 124 × i | 124 | animation i (five stored, Connect selects four): `[keyframes, effect count, 20 − speed, brightness 0-100]`, then 8 keyframes of 5 RGB triplets at `+4 + 15 × frame + 3 × zone` |
| 621 | 1 | audio-reactive |
| 623 | 3 | profile LED |
| 626 | 1 | raise to wake |
| 627 | 1 | standby minutes: 0 (never), 1, 5, 10, 20 |

`GLT:` byte 0 of an animation doubles as a preset type: 0x05 flow, 0x08 rainbow, 0x02 pulse, 0x06 alarm, 0x01 with a palette
for solid colours. The zone order is unresolved: `qo` names LowerLeft, LowerRight, UpperLeft, Home, UpperRight; GLT maps the
positions to left grip, right grip, none, profile, home. The crate's solid colour paints all five.

## Gamepad state, report 0x12

`Connect:` `ie` (7173), `Qo.push` (13243), `ys.parseGamepadState` (13579).

| Byte | Content |
|---|---|
| 1-4 | LX, LY, RX, RY, centre 0x80 |
| 5 | low nibble hat (0 up, clockwise to 7); bits 4-7 X, A, B, Y |
| 6 | bits 0-7: LB, RB, LT, RT (digital), View, Menu, LS, RS |
| 7 | bit 0 Home, bit 1 Share |
| 8, 9 | LT, RT |
| 10-34 | not read by Connect |
| 35, 36 | charging state, battery level (cyclone2-linux reads the same bytes) |
| 37 | light slot index |
| 38-52 | live colours, Home, LowerLeft, LowerRight, UpperLeft, UpperRight |
| 53 | bits 0-1 macro recording, bit 4 on L4, bit 5 on R4 |
| 60 | bit 3 L4, bit 4 R4, bit 5 M |

## Firmware update (not implemented)

- `0F FC` and `0F 17 55 88` reboot the pad into its JieLi loader. `GLT:` the MCU is a JieLi BR23 that reappears as
  `4C4A:2342` "BR23UBOOT1.00" mass storage; Connect watches for vendor 19530 (0x4C4A, 35258).
- `Connect:` the worker `dfu/upgradWoker.js` loads `JL_upgradeUtility_x86.dll` through ffi and calls
  `JL_setSdkAndToolId(0x55aa55aa, 0x5a5a)`, `JL_loadFirmwareData`, `JL_getFirmwarePidVid` (the product string inside the image),
  `JL_openDevice`, `JL_getDevicePidVid` and `JL_upgradeDeviceByHandle`. On result -3 it retries with the `No_Key` image (36686).
- Bundled images: `GS_C2_DEVICE_V123`, `GS_C2_ADC_DEVICE_V346` and `V352`, `GS_C2_Dongle_V119` and `V121`, each as `_Key` and
  `_No_Key`. The model table pins controller 1.2.3 and ADC 3.5.2. Online updates come from
  `https://gamesir.release.szhjc-tech.com/connect/`, but `um` (34948) ships with `enableFirmwareUpdate: false`.
- Hazard (`GLT:`): the receiver is a separate BR23 and must never receive controller firmware. The two are told apart by the
  product string at 0x1010 of the flash header, `GS_C2_ADC_DEVICE` against `GS_C2_Dongle`.

## On the TV

- `TV:` kernel `5.4.268-329.ptl4tv.2`, with no `/proc/config.gz` and no `modules.builtin.modinfo`. Built in: `usbhid`,
  `hid-generic`, `hid-sony`, `hid-playstation`, `hid-nintendo`, `hid-microsoft`, `joydev`, `uinput` and `xpad`, whose
  `/sys/bus/usb/drivers/xpad/new_id` exists. `/proc/kallsyms` shows only xpad's functions, which match stock 5.4.
- Stock 5.4.268 xpad has no GameSir (0x3537) entry; mainline gained `XPAD_XBOX360_VENDOR(0x3537)` later. [INFERENCE] In
  X-input mode the receiver gets no gamepad driver on the set unless the trusted helper writes `3537 102a ff` (ADC variant
  `3537 100b ff`) to `new_id`. xpad then types the FF/5D/01 gamepad interface as an Xbox 360 pad (`xpad_probe`, 5.4.268
  lines 1802-1812). The third field limits the match to vendor-class interfaces: without it, xpad types any other
  two-endpoint interface of an unknown device as an original Xbox pad and could take the HID configuration interface from
  `usbhid`.
- `docs/research/gfn-input-result.md`: in DS4 mode the receiver is `054C:09CC` with `hid-sony` and no configuration
  interface.
- LG lists DualShock 4 and the wired Xbox 360 pad as supported game controllers; in X-input mode the receiver presents
  GameSir's ids, not Microsoft's.
- Receipt: `/data/home/tmp/lumagame-rea-20261115/gamesir/tv-probe-20261115.txt` (read-only; no GameSir device attached).

Recommendation: DS4 mode for play. Streaming engines get gyro and rumble through `hid-sony`, and native webOS games get a
controller LG supports. X-input mode only for changing settings with `gamesir-config`, then back. X-input mode also has the
real battery level, which DS4 mode reports as a fixed low value (cyclone2-linux).

## The crate

`game/crates/gamesir-config` (library and CLI, hidraw only):

- `list`, `info`, `dump`, `set [--dry-run]`, `switch`, `rumble`.
- A node is used only if its descriptor has usage page 0xFFF0 and report 0x0F, and the CLI asks each candidate for its
  firmware version before using it.
- Every setting is validated against the ranges Connect offers before anything is sent. Writes carry only the changed span
  and end with a full read-back compared byte by byte.
- None of the "never" commands above are encoded.

## Unknown, needs the real pad

- Whether xpad binds in X-input mode, and whether the `new_id` route works.
- Which reply acknowledges writes and switches; and whether the shift layer (region 5) belongs to the active profile.
- Whether settings written in X-input mode apply in DS4 mode.
- Which variant the pad is: its X-input receiver id is `3537:102A` (Cyclone 2) or `3537:100B` (ADC).
- Bytes 10-34 of report 0x12 (possibly motion), the lighting zone order, and what the pad does with DS4 lightbar output.
- Whether a profile write needs the switch, as GLT found, on this pad's firmware.
