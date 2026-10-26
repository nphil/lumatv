# Controller input for the GeForce NOW app: the receiver, the TV's input path, the design

Read-only TV session 2026-10-25 20:00-20:25 (set woken from Active Standby with Nitin's approval). Evidence under
`docs/research/gfn-input/`. Tags as in `opennow-result.md`: `TV:` measured on this set, `[INFERENCE]` not measured.

## Resume here

Done: the receiver's USB facts, who reads it on webOS, the kernel knobs, the cause of the pad "doing nothing" on
2026-10-25 (an old boot hook, below), and the input design for the OpenNOW port.
Open: nothing to measure before Phase 4 of `opennow-result.md`. The two approvals below are for Phase 4, not now.
Set state at the end of the session: left on (Nitin was at the set with the pad and PadGuard was testing on it); not
put back in Active Standby.

## The pad did nothing: cause and fix

`TV:` every time the receiver enumerated, `hid-sony` bound it, created the gamepad, touchpad and motion devices
(`dmesg-receiver.txt`), and 0.07-2.4 s later the HID device lost its driver with no USB disconnect, no reset and no
kernel message (`messages-7900-7915.log`, `messages-8328-8349.log`: the children's `remove` uevents arrive, PDM
toasts "HID device is disconnected", the USB device stays). The actor is a webosbrew boot hook from the 2026-09-14
session, `/var/lib/webosbrew/init.d/50-padblock` (text in `tv-facts.txt`): a loop that every 3 s writes every
`0003:054C:09CC.*` HID device to its `driver/unbind`, installed so Netflix would not read this pad's idle axes as a
stick held up-left while it charged by cable. It matched the 2.4 GHz receiver because the receiver presents the same
DualShock 4 id.

Fix (applied by Main during the session, 20:11-20:13): the hook renamed to `/var/lib/webosbrew/50-padblock.off-20261025`
(outside `init.d`, so it does not run at boot) and the running loop killed; the receiver was replugged and
`hid-sony` kept it (`ds4-bound-state.txt`). The Netflix problem it solved is better handled by the GFN app's
exclusive grab while streaming (below) and, for charging by cable, by a hook keyed on the pad's *cable* identity
only, which this receiver does not share. LumaTV's declutter profile does not touch any service in this path
(`payload/declutter/items.txt`; the applied state masks no input, HID, PDM or compositor unit).

## The receiver

`TV:` `tv-facts.txt`, `dmesg-receiver.txt`, `ds4-rdesc-debugfs.txt`.

| Item | Finding |
|---|---|
| USB id, strings | `054c:09cc` bcdDevice 1.21, "Guangzhou Chicken Run Network Technology Co., Ltd." (GameSir) "Wireless Controller": a 2.4 GHz receiver presenting as a DualShock 4 v2. Serial none; HID uniq (the DS4 "MAC" it reports) `1c:a0:b8:4c:7a:99` |
| Bus, speed, port | Full speed (12 Mbit/s) on the OHCI companion controller `usb8` (`8-1`, PDM "USB port 2"); IRQ 128 `ohci_hcd:usb8`, served on CPU 0. The TV's third port is xHCI (`usb3`); a full-speed device gains nothing there |
| Interface | One HID interface (03/00/00), `usbhid`; endpoints: IN interrupt `bInterval 1` = **1 ms poll**, 64 bytes; OUT interrupt `bInterval 5` = 5 ms, 64 bytes (rumble/lightbar) |
| Report descriptor | 467 bytes, the stock DS4 v2 descriptor: input report 1 (64 bytes: X Y Z Rz, hat, 14 buttons, 6-bit counter, Rx Ry triggers, 54 vendor bytes: timestamp, battery, gyro, accel, touchpad), output report 5 (31 bytes), the feature reports 2/4/8/0x10-0x15, 0x80-0x8x, 0xA0-0xA3 |
| Actual report rate | **500 reports/s**: 1000 reports in a 2 s read of `/sys/kernel/debug/hid/<dev>/events`, OHCI interrupts +1007 in the same 2 s (`ds4-report-rate.txt`). The device answers every other 1 ms poll; the radio link is a 2 ms cycle [INFERENCE from the rate]. Byte 7 (the DS4 report counter) stays 0, unlike a real DS4 |
| Kernel driver | `hid-sony` (built in, stock 5.4.268 symbols, no LG patch found: `kallsyms-hid.txt`); whitelisted as a gamepad by LG's `lginput2` (vid 1356/pid 2508 in its `supportGamePadTbl` 1.006) |
| Nodes | `input46` → `/dev/input/event15` + `/dev/input/js6` (gamepad), `input47` → `event12` (touchpad), `input48` → `event14` (motion sensors); `/dev/hidraw0`; LEDs `0003:054C:09CC.<n>:{red,green,blue,global}`; `power_supply/sony_controller_battery_<mac>` |
| Permissions | `event*` and `hidraw*` are 0660 root:compositor (gid 505, `permissions.rules`); `js*` 0664 root:input |
| Force feedback | `capabilities/ff = 1 7030000 0 0`: FF_RUMBLE, FF_PERIODIC (square, triangle, sine), FF_GAIN, through `ff-memless` and the DS4 output report 5 on the 5 ms OUT endpoint |
| Second device | Until 20:09 a second USB device sat on the xHCI port (`3-1`, "port 3"): `4131:8008` "LiteEMF" "Hid Vendor", serial `123456789012`, one vendor HID interface (usage page 0xFF80, 64-byte IN and OUT at 1 ms), `hid-generic`, `hiddev96`+`hidraw0`, no input device. Both devices enumerated together at every USB power-up and disconnected within 76 ms of each other at 20:09; only the receiver came back. Its identity is Nitin's to confirm (a second dongle, or the pad's own cable in vendor mode) |
| Power | `power/control` on (no autosuspend) for the receiver; LG masks the TV's USB ports entirely in Active Standby (both devices disconnect at "power off", return at "Screen On") |

## Who reads it on webOS

`TV:` processes and open files (`tv-facts.txt`), binaries by `strings` and disassembly (`libim.so.1.0.0`, `libStarfishInput.so`).

- **`surface-manager`** (LG's Qt compositor, root) opens every `/dev/input/event*` through LG's `libim.so` input manager (`QT_QPA_EGLFS_DISABLE_INPUT=1`: Qt's own evdev is off). It is the only holder of `event15`. It closes the motion-sensor device ("Close device: … Motion Sensors") and keeps the gamepad and touchpad. `libim` reads with `epoll_wait` + `read`; its one `ioctl` call site is not `EVIOCGRAB`, and neither library contains any evdev ioctl constant: **nothing grabs the pad**, so a second reader sees every event, and a grab by the app silences the compositor for that device.
- `libStarfishInput` maps gamepad buttons to webOS keys (`Key_webOS_Gamepad_Back/Menu/Select/Settings/DPAD_*/ChannelUp/Down/Volume*`, `Key_webOS_XBox_Gamepad_App`) and hands them to the focused Wayland client as key events, and to LG's key chain (Home, Back). That is the path a web app (WAM) or any client gets without doing anything, and the path that must be cut while a stream is in front.
- **`lginput2`** opens every new `hidraw` node (its `uhid` monitor thread, to detect Magic Remotes) and keeps the fd; it does not write to a gamepad. It owns the gamepad whitelist and the "can only be used with game apps" toast.
- **`physical-device-manager`** (PDM) watches udev, counts the device, toasts connect/disconnect, and knows `/sys/bus/usb/drivers/xpad` for Xbox pads. `bthidmanager` ignores it ("NOT Bluetooth HID Device").
- Nothing in LumaTV reads it; the helper, the compositor and the card app do not open input devices.

Latency of LG's path for a web app: compositor evdev read → Qt key event → Wayland key → WAM → JS, frame-paced by the
page (one or two 16.7 ms frames) [INFERENCE: not measured here]. Chromium's Gamepad API polls at 60 Hz on top.

## Kernel knobs

`TV:` `tv-facts.txt`, `cpu-state.txt`.

- `usbhid` has `jspoll`, `kbpoll`, `mousepoll` (all 0) and `quirks` (`(null)`). None helps: `jspoll` applies to top-level usage Joystick, this device's is Gamepad; the IN endpoint is already at the 1 ms minimum; the module is built in, so `quirks` is only settable on the kernel command line (the padblock hook's comment is right).
- `xpad` is present (`auto_poweroff=Y`), `hid-playstation`, `hid-nintendo`, `hid-joycon-webos`, `hid-logitech`, `hid-microsoft`, `ff-memless`, `uinput`, `uhid`, `joydev`, `evdev` all built in; `/dev/uinput` is root-only (0600).
- CPU: 4 cores, `performance` governor pinned at 1.4 GHz, **no cpuidle driver** (idle is WFI only, so wake-up cost is tens of µs); LG hot-unplugs CPU2/CPU3 continuously ("CPU3: shutdown … Booted secondary processor" every few seconds in dmesg). The OHCI interrupt lands on CPU 0. `sched_rt_runtime_us` 950000; no LG daemon runs real-time except `irq/93-HDMI20_A` (FIFO 50).
- HID debugfs is on: `/sys/kernel/debug/hid/<dev>/events` is a free report tap (root only).

## Design: controller input in the OpenNOW port

The engine (the 64-bit Rust streamer that OpenNOW's transport crate already lives in) reads the pad itself; no SDL,
no joystick API, no LG service in the path.

1. **Device discovery.** Scan `/dev/input/event*` and read `/sys/class/input/eventN/device/{name,uniq,capabilities/*}` (both mounted read-only in SAM's jail, `jail_native_devmode.conf`); take devices with `EV_ABS` + `BTN_GAMEPAD`/`BTN_SOUTH`. Hotplug: inotify on `/dev/input` (a devtmpfs bind; works) plus a 2 s rescan while no pad is present. Four slots, by `uniq` then by node.
2. **Reader thread.** One blocking `read()` loop per pad on its own thread (not polled, not on the render or network thread): `libc::read` of `input_event` records, state updated on each event, snapshot sent at `SYN_REPORT`. Coalesce inside one report only; never wait for a frame. The receiver delivers at most 500 reports/s and OpenNOW's 54-byte state packet (`docs/native-gamepad-input.md`: `0x020d`, 38-byte XInput-style event, per-controller u16 sequence on the partially reliable channel) is small, so send every changed report; rate-limit only to the report rate. Map DS4 → XInput layout: `BTN_SOUTH/EAST/WEST/NORTH` → A/B/X/Y, `BTN_TL/TR` → shoulders, `ABS_Z/ABS_RZ` (0-255) → triggers, `ABS_X/Y/RX/RY` (0-255) → sticks (centre 128, dead zone in the engine), `ABS_HAT0X/Y` → d-pad, `BTN_SELECT/START/MODE/THUMBL/THUMBR`. Capture timestamp from the evdev event time (`CLOCK_MONOTONIC` via `EVIOCSCLOCKID`).
3. **Exclusive grab while streaming.** `EVIOCGRAB(1)` on the gamepad node when the stream goes in front, `EVIOCGRAB(0)` when it leaves or the process exits (the kernel releases a grab on close, so a crash cannot wedge the pad). With the grab held, LG's compositor sees nothing from this pad: no Home/Back/Settings mapping, no toast from a press. The touchpad (`event12`) and motion (`event14`) nodes are left alone (the compositor only holds the touchpad as a mouse; grab it too if its cursor shows up over the stream). The Magic Remote stays LG's and remains the escape hatch; the app's own exit chord (hold Select+Start 1 s) is read from the grabbed stream.
4. **Rumble.** GFN's haptic feedback report (OpenNOW's `NvstFeedbackReport` path) → one `FF_RUMBLE` effect uploaded once with `EVIOCSFF`, retriggered by writing `EV_FF` events with the new strong/weak magnitudes; `hid-sony` turns that into output report 5 on the 5 ms OUT endpoint. No hidraw needed; the LEDs are root-only sysfs and stay as they are.
5. **Priority.** The app cannot raise its own threads (jail: `RLIMIT_RTPRIO` 0, no `CAP_SYS_NICE`); a `lumatv-helper` request "boost thread <tid> to SCHED_FIFO 10" (root, applied to the reader thread only) is the one privilege worth adding, and the affinity hint `CPU 1` keeps the reader off CPU 0's OHCI interrupt. Without it the reader still wakes in ~0.1 ms on this governor; the boost matters only under load.
6. **Permissions.** None to add. SAM runs a native dev app as uid 5227 gid 5000 with supplementary groups `input` (19) and `compositor` (505) (`jail_native_devmode.conf` line 23; `cardapp-result.md`), and `/dev/input` is bound into the jail, so `event15` (0660 root:compositor) opens read-write: read, `EVIOCGRAB`, `EVIOCSFF` and the `EV_FF` write all work as that user. The read-only bind does not block opening a character device for writing. `hidraw` nodes in the jail are static copies made at jail creation (`copynod`), so they are not relied on.
7. **Disconnect.** On `ENODEV` from `read()` send the neutral state and an empty bitmap registration (OpenNOW's ordered release), drop the grab, go back to discovery.

Estimated added latency on the TV, pad button to packet on the wire (`[INFERENCE]`, from the measured rates and the
scheduler facts; to be measured in Phase 4 with a press timestamp against the packet send time):

| Stage | Typical | Worst |
|---|---|---|
| Pad → receiver radio (2 ms cycle) | 1 ms | 2 ms |
| Receiver → host (1 ms USB poll, full speed) | 0.5 ms | 1 ms |
| OHCI IRQ → usbhid → hid-sony → evdev wake | 0.05 ms | 0.2 ms |
| Reader thread wake, map, serialize, send | 0.1 ms | 1 ms (CPU hotplug, no boost) |
| **Total on the set** | **~1.7 ms** | **~4 ms** |

LG's own path for a web app adds roughly 16-33 ms on top of the same first three stages (compositor key event,
Wayland, WAM, page frame, Gamepad API poll) [INFERENCE]. The same receiver over LG's Bluetooth would add the BT HID
stack and `uhid` (the Magic Remote's path, `0005:…` devices under `/devices/virtual/misc/uhid`) and is not used.

## Approvals needed (Phase 4, none now)

| Change | Risk | Revert |
|---|---|---|
| Install the OpenNOW card app (reads `/dev/input/event*`, grabs the pad while in front) | While the app is in front the pad does nothing for LG's shell; the remote still works; the grab ends when the app leaves or dies | `appInstallService` remove |
| `lumatv-helper` thread-boost request (SCHED_FIFO on one tid of the app) | A runaway reader could take a core; capped by `sched_rt_runtime_us` 95 % | Helper restart, or not implementing the boost |
| Keep `50-padblock` off (done 2026-10-25); a replacement keyed on the pad's cable id if Nitin still charges by cable through the TV | None for the receiver | Move the file back into `init.d` |

No kernel parameter, module unbind, udev rule or compositor change is needed for the receiver.
