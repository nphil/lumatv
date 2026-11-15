# Bluetooth controllers on the C4: sniff mode and DualSense output

Read-only session 2026-11-15 14:01-14:20 on the OLED65C4 (O22, webOS 10.2.1, firmware 33.22.52), TV in Active
Standby: no Bluetooth calls that change link state. LG binaries, configs and Ghidra output under
`/data/home/tmp/lumagame-rea-20261115/` (kept out of git). **Measured** = read from the TV or LG's code;
**Inferred** = not observed.

## The stack (measured)

- `com.webos.service.bluetooth2` is `/usr/sbin/webos-bluetooth-service`: Bluedroid SIL (`libbtstdbluedroidsil.so`)
  over MediaTek's Fluoride (`/usr/lib/libbluetooth.so`). `bthidmanager` owns HID devices.
- Link control over Luna is two methods: `device/internal/stopSniff {address}` and
  `device/internal/startSniff {address, minInterval, maxInterval, attempt, timeout}`. HID output is
  `hid/internal/sendData`, `getReport`, `setReport`. All five sit in API groups `devices` and `bluetooth.manage` on
  this firmware.
- Nothing persistent exists: no link-policy, QoS or no-sniff method; `bt_stack.conf` and `bt_did.conf` hold only
  logging settings; device quirks (`INTEROP_HID_HOST_LIMIT_SNIFF_INTERVAL` and the MediaTek list) are compiled in,
  with no interop file on the TV.

## How sniff is set

- **bthidmanager, once per connection (measured, decompiled):** when a HID device connects
  (`mw_bthidmngr_udev_createDevice`) and is a game controller (its name is in bthidmanager's gamepad table, or its
  Class of Device gives controller type 8, which reads as the gamepad minor class (inferred)), it calls `stopSniff`,
  then `startSniff {"minInterval":12,"maxInterval":12,"attempt":1,"timeout":0}`: a 7.5 ms sniff interval.
- **The stack's sniff table (measured, read from `libbluetooth.so`):** 800/400, 400/200, 18/18, 150/50, 18/10,
  36/30 and 18/14 slots (0.625 ms each), park 800/400. It matches AOSP's table except the two entries AOSP gives HID
  (idle 180/150, active 54/30), which this build shortens to 18/18 and 18/10 (11.25 ms). That HID uses those two
  entries here, as in AOSP, is inferred.
- **What happens in practice (measured on an LG G5 by punktfunk, client-webos `docs/NOTES.md`):** without help, pad
  input arrives in ~77.5 ms bursts; one `stopSniff` holds only for seconds; re-asserting it every 250 ms gives
  ~400 reports/s, and each call answers in 1-3 ms.

The bursts contradict both the 7.5 ms connect-time setting and the short stack table, so something later stretches
the interval, such as sniff subrating or a renegotiation by the pad (inferred). The C4's burst interval was not
measured.

## DualSense output (measured on the G5 by punktfunk)

`hid/internal/sendData {"address","reportData":[...]}` with `reportData` as a plain int array and no `reportId` key
(an extra key fails the schema). Bluetooth report `0x31` is 78 bytes and needs the CRC32 that `hid-playstation`
appends, seeded with `0xA2`; without it the pad ignores a call the service reports as successful. Haptics audio is
report `0x32` (142 bytes) every 10.667 ms and only works with sniff held off. `getReport` hangs and `setReport`
returns error 4.

## LumaGame integration

1. A helper route in the gaming plugin, held by the pad-boost lease: for each Bluetooth pad (bus `0005`), send
   `stopSniff {"address"}` every 250 ms, and at once (at most every 200 ms) when the reader sees a gap over 50 ms
   between reports. About four calls a second per pad, 1-3 ms each.
2. At lease end, `startSniff {"address","minInterval":12,"maxInterval":12,"attempt":1,"timeout":0}`, LG's own
   gamepad values.
3. DualSense lightbar, player LEDs and trigger effects: `sendData` report `0x31` with the CRC, sent on change only.
   Rumble stays on evdev force feedback. Speaker and haptics audio (`0x32`/`0x36`) need step 1 running.

USB pads, including the GameSir receiver, need none of this.

Untested on this TV: the effect of `stopSniff`/`startSniff`, `sendData`, whether a dev-mode app holds the `devices`
group (which would let LumaGame call these without the helper), and the burst interval without the keeper.
