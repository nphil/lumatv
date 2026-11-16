# Leadjoy Xeno Plus configuration protocol

Offline analysis of Leadjoy PC 1.4.0, Android 1.1.9 (39), and controller package V54.16.14. No controller commands or firmware updates were performed. “Vendor-supported” below means present in the official application's code, not accepted on the physical controller.

## Sources and identity

- [Official support](https://jp.leadjoy.ai/pages/xeno-plus-support), [PC 1.4.0 archive](https://cdn.shopify.com/s/files/1/0977/7981/0586/files/leadjoy-pc-app-05-09-1.4.0.zip?v=1778468407), [product page](https://www.leadjoy.ai/products/xeno-plus).
- PC `device_manager_lib.dll` SHA-256: `6515a9bcf26c0bd83e8a5d33c9ea6e5d9e148965fd04dbf2c23f47863e648678`. Native addresses below refer to this PE image, base `0x180000000`. The Tauri/Vue frontend includes command wrappers, capability gates, controls, and embedded configuration documentation in `index-DSWWSRXE.js`.
- Android `com.leadjoy.app`, base APK SHA-256: `aa0ffb5997ba0b647773b917699f2162a05b135ba7607070d0a13dd693ea2530`. Flutter/Dart 3.10.8; protocol package `gamepad_protocol`; MessagePack and `flutter_blue_plus` are present. Android native OTA libraries are separate from configuration.
- [Firmware API](https://app.leadjoy.top/api/v1/versions/available?channel=leadjoy_device_xeno_plus&deviceId=00000000-0000-0000-0000-000000000001), package download `/api/v1/files/94/download`. The API field named `sha256` contains an MD5, not a SHA-256. Independently calculated SHA-256 for V54.16.14: `dfeab865b147f204c91349fb99ac8d0a3c5ccb2c85fc134ac0c7b118413e2e44`.
- Original downloads, extracted frontend, firmware, API responses and analysis receipts: `/data/home/tmp/lumagame-rea-20261115/leadjoy/`. Targeted disassembly: `/tmp/cs/3dc58534e64e/leadjoy-re/`. REA image evidence: `ev_21baca56f7406ec8ab65d158908a73f89b9250b8186bc127e886395d9332b677`.

## USB transport and framing

The PC discovery code accepts VID `4131`, PID `3519` or `8008` (hexadecimal; DLL `0x180082524`). These IDs span modes/models and are not sufficient identification. Android's USB filter separately lists `4131:8808`; equivalence to the PC configuration interface is unverified. The PC enumerates interfaces dynamically and supports `LEADJOY_HID_INTERFACE`; no fixed interface number is established.

The V54 master contains this vendor HID report descriptor at `app.bin` file offset `0x4f764`:

```text
06 80 ff 09 01 a1 01 09 02 a1 00 09 03 15 00 26 ff 00
75 08 95 40 81 02 09 05 91 02 c0 c0
```

It declares usage page `FF80`, application usage `01`, 64-byte input/output reports, and no numbered report IDs. Match the descriptor, not the gamepad interface. The PC HID writer (`0x1800238d3`) prepends a zero report-ID byte to the 64-byte payload for a 65-byte OS write. That zero is not part of the protocol packet. An adapter must normalize any zero report-ID prefix on input too. Endpoint numbers reported by third parties are not a substitute for descriptor discovery.

| Packet | Bytes before padding | Limits |
|---|---|---|
| Short | `A5`, `N+4`, `CC`, `N` payload bytes, checksum | `N <= 60` |
| Long | `A4`, `N+5`, `CC`, fragment counter, `N` payload bytes, checksum | `N <= 59` |

The checksum is the wrapping byte sum of every preceding byte through the payload. The length includes the checksum, excludes zero padding, and never exceeds 64. The first long fragment sets counter bit 7; bits 0–6 count remaining fragments including the current fragment, descending to 1. Safe maximum: 127 fragments, 7,493 payload bytes. The PC wrapper's looser 255-fragment bound conflicts with its seven-bit parser; the crate rejects that overflow.

Evidence: short builder `0x1800a8022`, long builder `0x1800a7f71`, sender `0x1800a80de`, parser `0x1800a8d4d`, reassembler `0x1800a86d5`.

Payloads are MessagePack maps with string keys. JSON notation in this document describes those maps, not text transmitted to the controller. Example `{"c":12}` becomes `81 a1 63 0c`; its report starts `a5 08 cc 81 a1 63 0c 0a`, followed by zeros.

Responses contain `c` (command number), `e` (status; zero means success), and usually `r` (result). Successful setter acknowledgements can omit `r`; absence must not be interpreted as an invalid ACK. A successful ACK alone is not confirmation that a setting changed: issue its getter and compare the returned value. There is no request transaction ID. Own the interface exclusively, serialize exchanges, ignore unrelated command responses, apply a deadline, and do not automatically retry a write after timeout.

## Identity and read-back shapes

The `r` column is the value inside the response envelope, not the full response. Preserve unrecognized fields when editing a returned configuration.

| Read request | `r` shape | Meaning |
|---|---|---|
| `{"c":12}` | string | Firmware version, not a version object |
| `{"c":13}` | device identifier | Preserve the returned representation |
| `{"c":14}` | integer | Battery level |
| `{"c":17}` | `{"m":"Xeno","s":number}` in V54 | Product/family and competitive flag; `Xeno` alone does not distinguish Plus from every related model |
| `{"c":30}` | integer `0` or `1` | Dual output |
| `{"c":42}` | `{"m":number,"l":number}` | Controller mode and connection type |
| `{"c":44}` | integer | Polling rate in Hz |
| `{"c":46}` | integer | Step size; zero means adaptive |
| `{"c":48}` | `{"std":number,"ljod":number,"rjod":number,"ddl":number}` | Stick/D-pad settings |
| `{"c":50}` | integer | Supported profile count; V54 master returns **1**, although the PC schema permits slots 1–4 |
| `{"c":51,"b":slot}` | `{"b":slot,"d2":{"a":0_or_1,"n":name,"cfg":[subconfigs]}}` | New profile-summary format; `d1` is the older alternate format |
| `{"c":60,"g":{"b":slot,"c":sub,"k":[keys]}}` | `{"b":slot,"c":sub,"d1":[button mappings]}` | Requested button mappings |
| `{"c":70,"g":{"b":slot,"c":sub,"d":side}}` | `{"b":slot,"c":sub,"d":side,"d1":settings}` | One stick |
| `{"c":90,"g":{"b":slot,"c":sub}}` | `{"b":slot,"c":sub,"d1":settings}` | Both triggers |
| `{"c":100,"g":{"b":slot,"c":sub}}` | `{"b":slot,"c":sub,"d1":settings}` | Motion settings |
| `{"c":120,"g":{"b":slot,"c":sub}}` | `{"b":slot,"c":sub,"d1":settings}` | Both vibration sides |
| `{"c":190,"a":0}` | `{"brt":number,"rgb":[r,g,b]}` | Global/logo colour |
| `{"c":191,"g":{"d":side}}` | `{"d":side,"d1":{"brt":number,"m":number,"s":number,"rgb":[...]}}` | Unverified-on-Plus effect palette; API only |

Sub-config selectors are Main `0`, Sub1 `1`, Sub2 `2`. Stick/palette sides are left `0`, right `1`. The real device's reported profile count takes precedence over the application's maximum. Product strings such as `xenoPlus` also occur in PC model-selection routing; they must not be misrepresented as observed c17 replies. Master c17 handler: `0x1e3f410`; c50 handler: `0x1e4007a`; c51 handler: `0x1e40116`.

## Official application settings contract

For profile domains, read first, modify the relevant complete `d1`, then write the same selectors and the complete object. Setters use `w`, not `g`. Preserve fields hidden by the current UI, including trigger-rumble ranges, unused up actions, and curve points. A device read failure must not cause a write of guessed defaults.

| Feature | Write | Read-back / UI constraints |
|---|---|---|
| Polling | `{"c":45,"s":rate}` | c44; choices 125, 250, 500, 1000 Hz |
| Step precision | `{"c":47,"s":size}` | c46; 0/adaptive, 32, 64, 128, 256 |
| Dual output | `{"c":31,"s":0_or_1}` | c30 |
| Connection mode | `{"c":43,"s":{"m":mode}}` | c42; retain connection type separately; changing mode can disconnect the interface |
| Stick/D-pad options | `{"c":49,"s":{"std":swap,"ljod":left_direction,"rjod":right_direction,"ddl":diagonal_lock}}` | c48; booleans are integer 0/1 |
| Profile name | `{"c":53,"s":{"b":slot,"n":name}}` | c51; compare `d1.n` or `d2.n`, matching the returned summary version |
| Active profile | `{"c":52,"s":slot}` | c51 for the selected slot; compare summary `.a == 1` |
| Sub-config trigger | `{"c":54,"s":{"b":slot,"c":sub,"ko":keep_original,"at":action_type,"k":[keys]}}` | c51 `d2.cfg`, selected by `c`; do not manufacture `kao` on V54 |
| Buttons / turbo | `{"c":61,"w":{"b":slot,"c":sub,"d1":[mappings]}}` | c60 with the corresponding key list |
| Stick | `{"c":71,"w":{"b":slot,"c":sub,"d":side,"d1":settings}}` | c70 with the same selectors |
| Triggers | `{"c":91,"w":{"b":slot,"c":sub,"d1":settings}}` | c90 |
| Motion | `{"c":101,"w":{"b":slot,"c":sub,"d1":settings}}` | c100 |
| Rumble | `{"c":121,"w":{"b":slot,"c":sub,"d1":settings}}` | c120 |
| Logo colour/brightness | `{"c":190,"a":1,"s":{"brt":brightness,"rgb":[r,g,b]}}` | c190/a0; brightness 0–100, RGB channels 0–255 |

### Buttons and turbo

A mapping is `{"k":physical_key,"a":action,"dn":down_action,"up":up_action}`. `a=0` means Down only; `a=1` enables separate Down/Up actions. Each action carries all eight fields:

```text
t   target: 0 original, 1 macro, 2 gamepad, 3 keyboard, 4 mouse
v   four key bytes, unused positions zero-filled
n   macro name, empty string for non-macro actions
r   0 ordinary, 2 turbo, 1 macro
sp  turbo rate: 0, 5, 10, 20, 30
mr  macro execution: 0 None, 1 TapOnce, 2 HoldOnce, 3 HoldRepeat, 4 TapRepeat
ms  macro step count, u16
ri  macro repeat interval, u16
```

Key IDs: none 0; A 1, B 2, C 3, X 4, Y 5, Z 6; LB 7, RB 8, LT 9, RT 10; Select 11, Menu/Start 12, Home 13, L3 14, R3 15, Capture 16; Up 17, Down 18, Left 19, Right 20; Menu 21, Mode 22, FN 23; M1–M8 24–31; Power 32. Stick-direction IDs occupy 80–95; whole-stick IDs are 99/100. Keyboard values are HID keyboard usage bytes; mouse buttons use 1–5.

The Xeno Plus UI's remappable physical keys are M3, M4, M1, M2, A, B, X, Y, L3, R3, directions, LB/RB, LT/RT, Select, Capture, Menu/Start and C. Home/FN are not in its remappable list. M1 is labelled R5, M2 L5, M3 R4, M4 L4. Serializer `0x1800e35bb`; mapping serializer `0x1800e3b5a`; action conversion `0x18009917e`.

### Trigger and vibration fields

Trigger `d1` is flat, not nested left/right:

```text
mol / mor  left/right mode: Adaptive 0, RapidFire 1, Linear 2
dzl / dzr  [inner deadzone, outer deadzone], two unsigned bytes
vtl / vtr  vibration threshold, unsigned byte
```

Deadzone/threshold controls are 0–100. Preserve the other side. Mode values are confirmed by native formatter `0x1800a4ee3` and parser `0x1801048b6`; field serializers are `0x1800e84a3` and `0x1800e874f`.

Vibration `d1` is also flat: `enl/enr` enable (0/1), `grl/grr` grip `[start,max]`, `trl/trr` trigger `[start,max]`. All ranges are two bytes, UI percentages 0–100. The normal Xeno Plus vibration page exposes grip ranges; preserve trigger ranges even when hidden. Raw serializers: `0x1800ec9ab`, `0x1800ecd5e`.

### Stick fields

The 21-field `d1` uses `co` (two signed 16-bit center offsets), `ci` (shape: rounded rectangle 0, circle 1), `rjc` (release cut), `mm` (mouse mapping enabled), `mse` (mouse sensitivity), `mo` (mouse operation: Vector 0, Fixed 1), `mx/ry` (X/Y inversion), `asf` (signed anti-shake factor), `ct` (curve format), `cv` (flat alternating x/y curve numbers), `rt` (smooth shape), `rc` (curvature), `cpl/cpu/cpr/cpd` (direction trigger radii), and `tkl/tku/tkr/tkd` (direction actions, each `a/dn/up`).

Anti-shake is −10…10; release cut `rjc` is boolean and mouse sensitivity `mse` is 0–100. **`ct=1` selects Smooth; `ct=0` selects a point-defined curve.** Native parser `0x18010796c`, reader `0x18009bb61`, writer `0x18009b876` and serializer `0x1800e94b4` agree on these values. `rt` is Linear 0 / Convex 1 / Concave 2 / SCurve 3, independently of point count. Preserve the reported curve count and endpoint records. Inner/outer deadzone and compensation are curve endpoint coordinates, not additional wire fields.

Curve units require exact model and firmware identification. PC stick gate `PS` is `{XenoPlus:46,XenoSe:13}`; motion gate `zS` is `{XenoPlus:46}`. These gates do not include XenoPro. New-format encoding is `round(display * 65535 / 200)`; decoding multiplies by `200 / 65535` and snaps to 0.1. The editor's display range remains 0–100, so display 100 encodes as 32768, not 65535. Older curves use integer display units. Preserve out-of-range raw coordinates during unrelated edits rather than clamping them. Frontend `NS/FS/JS/HS`, characters 89352–91000; generic editor bounds at 840400–842260.

Center offsets have a separate exact `{XenoPlus:46}` gate: signed raw −32767…32767 in expert mode versus raw −100…100 in the older control (display −0.1…0.1, step 0.001). The native recognized-name list includes `Xeno`, `XenoSE`, `XenoPro` and `Xeno_Plus_COMP`, but does not establish a unique c17 `Xeno` → Plus mapping. Without that identity bridge, center and curve editing must remain unavailable; other edits must preserve raw `co` and `cv`.

The V54 firmware additionally shows different spellings for some stick fields: `mom` versus PC `mo`, and `xr/yr` versus PC `mx/ry`. A configuration page must select the spelling present in the fresh device result, preserve all other fields, and verify the result; inserting a missing PC-era key is not proof the newer firmware used it.

### Motion fields

Motion `d1` is a single flat object (serializer `0x1800e599f`, frontend conversion `0x180099b93`):

| Key | Official control / wire value |
|---|---|
| `sj` | Disabled 0, left-stick simulation 1, right-stick simulation 2, button-like 3, mouse 4 |
| `mlj` | Mouse-linked joystick: disabled 0, left 1, right 2 |
| `tm` | Disabled 0, Toggle 1, Hold 2, AlwaysOn 3, AlwaysOnPauseOnHold 4 |
| `tk` | Trigger-key byte array; frontend uses two keys |
| `kao` | Trigger-key logic, And 0 / Or 1 |
| `cv` | Alternating x/y curve coordinates as unsigned integers: the PC holds them as `u32` and writes each with the compact MessagePack uint encoding (`0x1800231cf`) |
| `rt`, `rc` | Smooth curve Linear 0 / Convex 1 / Concave 2 / SCurve 3, curvature 0–100 |
| `se`, `hvr` | Sensitivity and horizontal/vertical ratio, 0–100 |
| `had` | Horizontal axis Yaw 0 / Roll 1 / Mixed 2 |
| `hsr`, `hrr`, `vr` | Horizontal swing, horizontal roll, vertical reversal; 0/1 |
| `st` | Sensor: accelerometer 0, gyroscope 1 |
| `fs` | Legacy byte retained from the complete read; no separate current frontend control established |
| `ec` | Axis thresholds: array of `{et,acc,ag,a}`; `et`: Yaw 0 / Roll 1 / Pitch 2; acceleration/angle thresholds 0–100; `a`: Continuous 0 / Single 2 |
| `el` | Direction mappings: array of `{e,dn,up}` with full button actions; event `e`: YawLeft 0, YawRight 1, RollLeft 2, RollRight 3, PitchDown 4, PitchUp 5 |

The normal controller button-like motion page forces Single behavior. The controller and accessory use related event engines; identical human labels must not be assumed to have identical numeric axis values. Preserve an event's unedited up action rather than dropping it.

### Profiles, directional limiting and macros

Controller modes are XInput 0, Switch 1, PS4 2, PS5 3, mobile-touch 4 (parser `0x180102a2b`). Stick output directions are **default 0, four-way 1, eight-way 2**, not rotations (`0x18009f066`). The diagonal-lock and stick/D-pad-swap fields remain booleans.

The old c51 `d1` summary contains `{a,n,k1,k2}`. The newer `d2` contains `{a,n,cfg}`. `ko` is keep-original 0/1; `at` is Responsive 0 / Toggle 1 (`0x180102c20`). The V54 master emits only `c,ko,at,k` per sub-config (`0x1e402be`), ignores c54 `kao`, and the PC gates And/Or at `{XenoPlus:999}`. An acknowledged `kao` change is therefore not supported or verified. Comparable edits require a complete fresh matching entry, preserve unknown fields, and cannot add unreported fields other than the outer profile selector `b`.

V54 c54 accepts at most 32 `k` entries and stores physical IDs 1–32 as a bitmask (`0x1e40796`–`0x1e4080c`); zero padding, duplicates and input order disappear. The PC's two-key selection limit is not the wire capacity. Compare sub-config keys as a set, not an array; other protocol arrays retain their order-sensitive meaning. The getter's physical-key order is not numerical order.

Macro summary c170 returns an array of `{n:name,k:[key1,key2],s:step_count}`. Read c171 with top-level `n`; its result includes `n,s,t,d` and may include `k`. A classic macro write is:

```json
{"c":172,"w":{"n":"Jump","s":1,"t":0,"d":[0,1,40,80,1]}}
```

Each classic step contributes five unsigned numbers: `[device_type,key,interval,duration,1]`. Device types are gamepad 0, keyboard 1, mouse 2. Keys are wire key IDs, not the PC backend's internal enum ordinal. The last field is set to 1 by the official converter (`0x1800085db`); do not invent a sixth field. `s` is the u16 step count, not the flattened array length. V54 stores interval and duration as u16 milliseconds; the official UI limits each to 0–36000 ms and all macros together to 500 steps. The PC also parses advanced `t=1` steps; preserve their five-value raw records on import rather than converting them to classic timing steps.

V54 c172 (`0x1e4464e`) stores the macro when it reaches `d`, using the name parsed so far: `n` **must precede `d`**. It discards `s`, `t` and `k`; c170/c171 emit `k:[0,0]`, and c171 emits `t:0`. Writes append without replacing an existing name, and name lookups select the first match. Replacement therefore requires c173, verification of absence, then c172 and a fresh c170 containing exactly one matching name. c173 (`0x1e44998`) removes only the first match.

Names accept at most 33 UTF-8 bytes in c171/c172/c173, but only 32 in c61; c60 truncates names to 32 bytes. Creation and binding use a 32-byte limit with no NUL. The official app trims, truncates on character boundaries, rejects empty/duplicate names and `\/:*?"<>|`. V54 c61 (`0x1e4129e`) ignores `ms`, resolves the name to its CRC16 and stores `ri` as u16 for HoldRepeat/TapRepeat only. c60 reports the current step count. The app sends c170 `s` (0 if unknown), defaults to TapOnce and `ri:0`, and labels `ri` milliseconds with a 0–36000 range. An unknown name produces an unresolved binding; deleting or renaming its macro makes c60 return an original action (`t:0`).

Delete with `{"c":173,"n":name}`, then read c170 and verify that the name is absent. Recording uses c174 with `s=1` to start and `s=0` to stop; c175 carries unsolicited recorded-step reports. Recording requires exclusive ownership and an explicit stop on cancellation/disconnection. A recording ACK does not prove a macro was stored: write its name/steps with c172 and read c171 back. Serializer `0x1800e4b86`, macro envelope `0x1800dcf72`; V54 c170/c171 handlers `0x1e4436c`/`0x1e4445a`.

### Remaining command families and operations

The DLL's complete recovered command table (`0x1800bd5a2`) also includes the following. Firmware update commands and model-specific bootloader PIDs are deliberately outside the configuration crate.

| Commands | Shape / behavior |
|---|---|
| 10 reset, 11 shutdown, 15 save, 56 reload | No argument fields; mutating operations, not getters |
| 16 edit mode | `s:{s:state}`; PC supports an optional state |
| 40 button reporting | `s:0_or_1` |
| 41 calibration | `s:{t:kind,a:phase}`; kind 0 joystick/trigger, 1 motion; phase 0 start, 1 end; result/status is `{t,s}` |
| 55 reset configuration | `s:{b:slot,t:kind,st:sub_type}` |
| 130/131 FPS selection | Getter `g:{b,c,s:[slots]}`, slots 0–2; setter `w:{b,c,d1:[entries]}`; entry `s` slot, `n` name, `es` enable, `k` two key bytes, `kao` logic |
| 132–135 FPS files | 132 list; 133 delete with `n`; 134 detail with `n`; detail result `{n,t,d1}`; library capability does not establish a current Xeno Plus UI page |
| 220–224 motion accessories | 220 enumerate; 221 read `g:{b,c,mkn:[indices]}` (indices 1–4); 222 writes complete `w:{b,c,d1}`; 224 pairing is a separate explicit operation |

No entries were recovered for 223 or the 122–129 gaps; a switch-table hole is not an undocumented valid command. Commands 210–212 are firmware-only observations, not exposed application settings.

Save/reload/reset and calibration require operation-specific confirmation, not a fabricated expected setting. After save, re-read the edited domains; that checks their current values, **not survival of a power cycle**. After reload/reset, refresh each displayed domain and keep the operation status distinct from value comparison. After calibration, observe c41 completion/status and re-read stick/trigger settings; the physical centering/gyro result still requires a real-pad check.

### Motion accessory settings

c220 reports four `{mkn,c:[R,G,B],s}` entries; statuses are Disconnected 0, Pairing 1, Reconnecting 2, Connected 3. Missing entries are not evidence of disconnection. V54 c221 returns eleven fields per entry: `mkn,cs,pu,wm,cv,se,hvr,rt,rc,ec,el`. Preserve all eleven and unknown additions; the PC DLL's four-field reconstruction is lossy. Evidence: master `0x1e2f192`, `0x1e457a8`; native formatter `0x18009d725`.

Wear position `pu` is Custom 0, Forehead 1, Top 2, Knee 3, FootBack 4. Axis thresholds and directional actions use the motion wire enums above, not the frontend's different ordinals. Knee and FootBack expose only PitchUp/YawLeft/YawRight; Knee allows Single only, FootBack permits Continuous only for Pitch. Custom/Top/Forehead allow all axes and Continuous. Forehead reverses the displayed yaw direction. Changing position must not silently discard hidden mappings or rewrite unknown action values. Sources: DLL `0x18011d363`, `0x180008acd`, `0x1800947b6`; frontend `Ty/Py/Yr` around characters 94900–97500.

c224 is `{"c":224,"s":{"mkn":index,"a":action}}`, where Disconnect/Cancel is 0 and Pair/Re-pair is 1. Its ACK is not pairing completion; c220 supplies actual status. c221 uses `g` and c222 uses `w`, as existing profile queries/setters do (envelopes `0x1800d9eb5` / `0x1800dc2dc`); only c224 uses `s`.

### Android feature coverage

Android `libapp.so` SHA-256 `c2de0e9cbf926fefe88ce640fa329d9b591cb6302f671873224cf12b01759887` contains application-specific pages for FPS editing/repositories, advanced macro building blocks, calibration, profile import/copy, motion accessories, and camera/manual stick-curve sampling with history. These are real Android UI coverage gaps, not merely generic PC SDK commands.

Recovered Android gates distinguish page presence from model support. `isSupportFPSRepository` (`0x7b09d0`) requires `isLab && bladePlus`; `_JoystickConfigState.initState` (`0x89946c`) independently applies that same gate to the SubFpsPage route (`0x8d3a20`). This route is not enabled for `xenoPlus`. `isLab` comes from `SettingState.field_3b` (JSON writer `0x73077c`). Retained method dumps: `$CODY_SCRATCH/leadjoy-android-gates/`, particularly `fps-eligibility/asm`, `fps-flag-source/asm` and `joystick-route-callers/asm`.

Exact Xeno Plus firmware gates for decimal stick curves, stick curvature and stick-to-mouse support remain separate from the generic c17 `Xeno` identity, which does not resolve the exact product. Advanced-macro and stick-curve-assistance applicability remain unresolved. Android-page presence, model support, wire semantics and hardware acceptance are separate claims; full Android-app parity is not established.

The Android phone-touch mapper is a different feature: its Java `MacroStep` uses seven values `[type,startX,startY,endX,endY,duration,interval]`. It must not be passed to the controller's five-value macro format. Controller OTA/first-aid pages also exist, but firmware writing remains outside ordinary configuration and requires a separate recovery and risk plan.


## Rust API and integration boundary

`game/crates/leadjoy-config` is transport-independent. `encode`/`decode` operate on MessagePack values; `frame::Decoder` validates checksums, fragment ordering/counts and lengths. `Request::reports()` and `dry_run(&Change)` never open a device. Integers use the compact MessagePack encoding, as the PC does; the PC's encoder never emits float32, so curve points stay integers.

V54 parses maps incrementally, not as unordered objects. Both encoders emit selectors before their dependent fields, recursively through arrays: command `c` first; profile `b/c` before payloads; stick `d` before `d1`; button `k/a` before `dn/up` (`0x1e415f6`); motion axis `et` before thresholds and event `e` before actions (`0x1e42f68`); accessory `mkn` before its fields (`0x1e45a6a`); macro `n` before `d`. Motion `ec` precedes `el`, and stick `ct` precedes `rt`. Alphabetically sorted JSON maps violate these dependencies. These constraints are static V54 findings, not a real-controller acceptance result.

- `Request::query`, `profile_summary`, `profile_settings`, `buttons`, `joystick`, `fps`, `motion_device_config`, `macro_detail`, `fps_detail`, `lighting`: getter builders.
- `Change::dual_output`, `polling_rate`, `step_size`, `controller_mode`, `joystick_dpad`, `profile_name`, `activate_profile`, `sub_config`, `classic_macro`, `delete_macro`, `lighting`: paired setter/getter plans.
- `Change::profile_settings(setter, getter, complete_result)`: button, stick, trigger, motion, vibration, FPS and motion-device settings. Pass the **full `r`**, including `b/c/d` selectors and `d1`; preserve fresh unedited values. A mismatched selector is rejected before I/O.
- Builders reject what the official converters would: `joystick_dpad` needs `std/ddl` 0–1 and `ljod/rjod` 0–2; `classic_macro` needs a 1–32-byte name without NUL, at least one step and u16 timing; `delete_macro` needs a name present in a fresh c170 summary. `sub_config` compares only the fields c51 reports, so a V54 `kao` change is ACK-only.
- `settings::ButtonAction`, `MacroStep`, `MacroDevice`, `ControllerMode`, `TriggerMode` expose proven numeric encodings. All ordinary official settings can also be represented through known-command `Request::argument/new` without inventing a raw-command UI.
- `Controller<T>::read` only accepts getters. `apply(change, RunMode::DryRun)` sends nothing; `Apply` writes once, checks ACK, reads again and compares. A mismatch is `Error::ReadBack`; an ACK followed by a failed read exchange is `Error::Unverified`. A c43 mode switch can re-enumerate the interface, so after `Unverified` reconnect and read c42. `execute(request, mode)` is explicitly for operations and returns an ACK/status `CommandReceipt`, **not** a read-back success. `receive(command)` receives a matching unsolicited report without sending.
- `Request::unverified_xeno_plus_joystick_lighting` and `Change::unverified_xeno_plus_led_frame` are explicit API-only investigation paths. Ordinary `led_frame(Model::XenoPlus, ...)` remains rejected; ten-colour limits apply to both paths.

The caller supplies one exclusively owned `Transport`, the correct 64-byte vendor interface, permissions and deadlines. No discovery, privileged USB access, firmware updater, background polling, automatic write retry, or controller connection is hidden in the crate.


## Lighting and per-LED verdict

The original RGB-list lead applies to **c191/c192**, not the normal c190 logo setting. PC c190 serializes exactly one RGB triplet (`0x1800ea646`). The V54 master handler (`0x1e44be2`) likewise reads exactly three colour bytes and a brightness byte. This controls a common colour; it does not upload an image.

The PC enables its joystick-lighting page for Xeno SE/Saber, not Plus/Pro. Nevertheless, V54 Xeno Plus firmware registers c191 and c192 (`0x1e37208`, `0x1e37210`), with handlers `0x1e44d72` and `0x1e44f64`. They accept side 0/1 and **up to ten RGB triplets per side**. Each 36-byte record contains brightness, mode, 16-bit speed, colour count, 30 colour bytes and padding; the records start at RAM `0x16eee` and `0x16f12`. Setter colour count is bounded at 30 bytes (`0x1e450f6`); getter at ten triplets (`0x1e44f12`).

```json
{"c":191,"g":{"d":0}}
{"c":192,"s":{"d":0,"d1":{"brt":30,"m":2,"s":1000,"rgb":[255,0,0,0,255,0]}}}
```

The PC writes flat RGB bytes; its decoder also accepts nested triplets. Public effects are off `0`, steady `2`, gradient `3`, breathe `4`, trailing `5`; offered speeds are 500/1000/1500. Empty palettes mean rainbow in the frontend; one/two colours serve effect palettes. Mode `1` has no separately exposed control and is not evidence of a framebuffer mode.

**Verdict:** multiple-colour storage is proven; independent control of the Xeno Plus logo pixels is **not proven**. The owner reports individually addressable logo LEDs. Neither a one-colour command nor a shared-firmware palette proves or disproves that hardware report. The ten slots may address stick-ring effects, unused shared code, or another lighting surface. They are not established pixel indices or a complete logo matrix. Keep the experimental c191/c192 API explicitly named “unverified on Xeno Plus” and out of the UI.

The hidden PC `FirmwareDebugPage`, unlocked through `check_dev_mode` / `enable_dev_mode`, provides `debug_raw_command` and packet inspection. It is a generic MessagePack console, not an image-upload command. Additional master commands 210–212 exist: 210 returns `se`/`tap` settings; 211/212 parse configuration objects. None is established as an LED/image command. Factory/test strings also do not establish an undocumented logo uploader. Unresolved indirect calls prevent claiming that no hidden path exists.

### Future supervised c191/c192 test on the real pad

1. With the owner present, close the vendor app, identify the controller/version, read c190 and c191 for **both** sides, and save the complete replies plus raw packets before any write. Stop if either palette read fails or cannot be losslessly restored.
2. Change one side only to brightness 30, steady mode 2, speed 1000, and ten distinct RGB triplets: red, green, blue, yellow, cyan, magenta, orange, violet, white, grey. Read c191 back immediately. Do not try unknown modes or overflow the ten-colour bound.
3. Ask the owner which LEDs changed: logo, left/right stick rings, neither, or another surface; record a photo/video and colour order. Repeat the other side only after the first side is restored. Palette acceptance without a visible colour-to-LED mapping is not per-pixel success.
4. Restore each saved palette and the saved c190 setting, then read back and compare every field. Restore even if no LEDs visibly changed. Record firmware version, replies and evidence receipt paths.

**Persistence:** c192 updates RAM and its configuration CRC (`0x1e2d898`); an immediate flash write is not visible in that handler. These records are part of the persistent configuration bank. c15 SaveSettings calls `0x1e36d16` → `0x1e2ee20` → `0x1e2dc96`, using the `mnt/sdfile/app/USERIF` storage path and erase/write operations. Other paths also save the bank; automatic-save timing is not established. Treat temporary settings as potentially persistent, never rely on power cycling to undo them, and do not issue c15 while the test pattern is active. If a test pattern was saved, restore first, save the restored bank, and verify read-back again.

## Offline firmware findings

The outer package starts with `Leadjoy Upgrade Pack` (20 bytes), `01 00 00 02`, a little-endian u16 version-block length, padded version text and entry count. Entries contain name length/name, compressed/raw u32 lengths, five LZMA property bytes and an LZMA1 stream. All five downloaded pad/dongle packages parsed to their exact end. M/S/App version components refer to distinct images; they are not interchangeable.

| Image in V54.16.14 | Identity evidence | SHA-256 |
|---|---|---|
| `TJPlus_M_V54_2026-10-16.ufw` | JieLi BR23/AC635N build target, PI32v2; embedded `board/br23/board_ac635n_demo.c` | `e8943bbceb5aba189b8a24dca057d9d59b6396396a362c973d469f4073ebc61c` |
| `TJplus_S_V16_2026-05-30.bin` | WCH CH585, RISC-V; `CH585_BLE_LIB_V1.4.8` | `2bdfc98e0a40bf0aba8bf0d62f7d81b3c6a4426d41d5f7cc76e1707be79fa6b5` |
| `TJPlus_App_V14_2026-10-16.ufw` | JieLi BD19/AC632N build target, Q32S; `board/bd19/board_ac632n_demo.c`, P11 image | `8e947220e190e3134a572c5e0c9ee162c3f483da22e60c1a65dd1cb176087df3` |

These are build-target identities, not photographed package markings. The master application loads at `0x1e00120`, not `0x1e00000`; bootstrap copies file range `0x58510..0x64488` to RAM zero. Native firmware addresses above use that corrected mapping. JieLi processor definitions decode the command handlers, but some instructions remain unknown; decompiler-generated whole-program conclusions would be unsafe.

**Unresolved hardware boundary:** the exact LED driver, bus/pins, physical matrix geometry/ordering, and logo-pattern asset address/drawing routine are not recovered. Generic LiteEMF RGB code includes a weak WS2811-style SPI driver and WS2812FX; that is not proof that this PCB uses either. No named bitmap or logo-upload routine has been established in the images. Anonymous byte arrays cannot responsibly be labelled the logo without a verified driver and mapping. A readable PCB/LED-board teardown, pin tracing, or a recoverable driver capture is needed to cross that boundary; offline command parsing alone cannot supply it. A [public teardown](https://www.youtube.com/watch?v=YJvMt-Of7lY) is a possible board-evidence source, not a verified driver identification.

## Building our own Android app

A separate app can implement the recovered configuration maps and transport without copying the vendor UI or requiring vendor accounts for local configuration. USB is the clearest initial route: Android USB-host permission, descriptor-based interface selection, 64-byte packet framing, MessagePack, a single serialized request queue, explicit apply, and immediate read-back. USB-host endpoint transfers do not include the hidraw zero report-ID prefix. Never detach the gamepad interface merely because it shares the same VID/PID.

Bluetooth configuration still requires the app's exact service/characteristic UUIDs, write/notification semantics, negotiated payload sizing and any authentication to be established. Android gamepad pairing alone does not expose that configuration channel. The presence of `flutter_blue_plus` and an OTA-auth library is not a recovered BLE contract. USB IDs and packet framing must not be assumed to identify the BLE service.

The Rust crate can be shared through JNI or a small native bridge, with Android owning permission, USB I/O and lifecycle. Preserve the last complete read snapshot and distinguish “encoded”, “device acknowledged” and “read-back matched”. Close the session on disconnect; keep calibration/recording exclusive. Profile/macro import/export can stay local and should retain product/version metadata. Firmware download/installation is a separate, privileged workflow and is not part of this configuration library.

## Custom firmware route — documentation only

This route is **not implemented or attempted**. A replacement would require the matching JieLi BR23/BD19 SDK/compiler or a validated PI32v2/Q32S toolchain, the WCH CH585 SDK/RISC-V toolchain, exact flash/boot metadata, radio and calibration data, the inter-MCU protocol, and a physical LED/PCB map. The readable vendor update package is not proof of an unlocked bootloader or a supported custom-image installer.

Before any future flashing proposal, establish an independently tested wired recovery path, immutable backups of boot/configuration/calibration/identity areas, flash size/erase boundaries and per-image target validation. A normal configuration backup is not a full recovery image. Updating only one of the three processors can break their protocol even if each image boots. Wrong target, wrong address, damaged boot metadata, interrupted erase, overwritten factory calibration or incompatible radio code can permanently disable the controller. USB/Bluetooth OTA recovery may disappear with the broken firmware; external programming or inaccessible test pads may be required. Restoring the stock package is not a guaranteed unbrick procedure.

The lower-risk route remains stock firmware configuration first, followed by the supervised palette mapping test. Custom firmware is justified only after hardware mapping and recovery are proven, not merely because the logo's drawing path remains unknown.
