# Controller input for the GeForce NOW app: the receiver, the TV's input path, the design

Read-only TV session 2026-10-25 20:00-20:25 (set woken from Active Standby with Nitin's approval). Evidence under
`docs/research/gfn-input/`. Tags as in `opennow-result.md`: `TV:` measured on this set, `[INFERENCE]` not measured.

## Resume here

Done: the receiver's USB facts, who reads it on webOS, the kernel knobs, the cause of the pad "doing nothing" on
2026-10-25 (an old boot hook, below), the input design for the OpenNOW port, and (host-proved, not yet on the set) the two pieces
the design left open: the helper's reader-thread boost and the Phase 4 latency tool ("Built" below).
Open: Phase 4 on the set ("Phase 4 on the TV"): Nitin plugs the receiver, starts a game and presses buttons for 30 s; the numbers
replace the `[INFERENCE]` ones in the latency table. The approvals below are for that run.
Set state at the end of the 2026-10-25 session: left on (Nitin was at the set with the pad and PadGuard was testing on it); not
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
5. **Priority.** The app cannot raise its own threads (jail: `RLIMIT_RTPRIO` 0, no `CAP_SYS_NICE`); `lumatv-helper` raises the one that matters, root to thread and nothing else: `SCHED_FIFO` 10 and an affinity of CPU 1 (plus 2 and 3 while online) for each pad's reader thread, only while the stream is in front. Without it the reader still wakes in ~0.1 ms on this governor; the boost matters under load. How it is scoped and what it does: "Built", below.
6. **Permissions.** None to add. SAM runs a native dev app as uid 5227 gid 5000 with supplementary groups `input` (19) and `compositor` (505) (`jail_native_devmode.conf` line 23; `cardapp-result.md`), and `/dev/input` is bound into the jail, so `event15` (0660 root:compositor) opens read-write: read, `EVIOCGRAB`, `EVIOCSFF` and the `EV_FF` write all work as that user. The read-only bind does not block opening a character device for writing. `hidraw` nodes in the jail are static copies made at jail creation (`copynod`), so they are not relied on.
7. **Disconnect.** On `ENODEV` from `read()` send the neutral state and an empty bitmap registration (OpenNOW's ordered release), drop the grab, go back to discovery.

Estimated added latency on the TV, pad button to packet on the wire (`[INFERENCE]`, from the measured rates and the
scheduler facts; to be measured in Phase 4 with a press timestamp against the packet send time):

| Stage | Typical | Worst |
|---|---|---|
| Pad → receiver radio (2 ms cycle) | 1 ms | 2 ms |
| Receiver → host (1 ms USB poll, full speed) | 0.5 ms | 1 ms |
| OHCI IRQ → usbhid → hid-sony → evdev wake | 0.05 ms | 0.2 ms |
| Reader thread wake, map, serialize, queue | 0.1 ms | 1 ms (CPU hotplug, no boost) |
| Engine takes the packet from the queue (the core polls it every 250 µs: `NATIVE_INPUT_POLL_INTERVAL`, upstream) | 0.13 ms | 0.25 ms |
| **Total on the set** | **~1.8 ms** | **~4.3 ms** |

LG's own path for a web app adds roughly 16-33 ms on top of the same first three stages (compositor key event,
Wayland, WAM, page frame, Gamepad API poll) [INFERENCE]. The same receiver over LG's Bluetooth would add the BT HID
stack and `uhid` (the Magic Remote's path, `0005:…` devices under `/devices/virtual/misc/uhid`) and is not used.

## Approvals needed (Phase 4, none now)

| Change | Risk | Revert |
|---|---|---|
| Install the OpenNOW card app (reads `/dev/input/event*`, grabs the pad while in front) | While the app is in front the pad does nothing for LG's shell; the remote still works; the grab ends when the app leaves or dies | `appInstallService` remove |
| `lumatv-helper` thread-boost request: `SCHED_FIFO` 10 and CPUs 1-3 for the app's `gfn-pad-event<N>` threads only (built; refuses everything else) | A runaway reader could take a core; capped by `sched_rt_runtime_us` 95 %, and the boost is a 90 s lease the app renews, so a hung app loses it | `POST /v1/gfn/pad-unboost`, helper restart, or `LUMATV_GFN_BOOST=0` for the app |
| Keep `50-padblock` off (done 2026-10-25); a replacement keyed on the pad's cable id if Nitin still charges by cable through the TV | None for the receiver | Move the file back into `init.d` |

No kernel parameter, module unbind, udev rule or compositor change is needed for the receiver.

## Built (2026-10-27, host-proved: cargo tests, clippy; nothing run on the set)

**The boost** (`helper/app/src/gfn_boost.rs`, `gfn/engine/crates/lumatv-gfn-pad/src/boost.rs`). When the stream comes in front (the
grab is taken) the pad crate asks the helper, over loopback, to boost each reader thread: `POST /v1/gfn/pad-boost {pid, tid}`, again
every 30 s, and `POST /v1/gfn/pad-unboost` when the grab is let go or the engine stops. The helper does it as root with
`sched_setscheduler` (`SCHED_FIFO` 10 with reset-on-fork) and `sched_setaffinity` (CPU 1, plus 2 and 3 while `/sys/devices/system/cpu/online`
lists them; nothing is changed on a set with no CPU 1). It refuses, with a reason in the answer and one line in `streamer.log`
(`gfn-pad: no boost for thread N: ...`):

| Refused | Code |
|---|---|
| a thread that is not in that process | `not_found` |
| a thread not named `gfn-pad-event<N>` (the reader's own name; the discovery, stats, boost and engine threads are never boosted) | `refused` |
| a process of uid 0 | `refused` |
| a process that is not the app's: neither it nor one of its 6 nearest ancestors has `org.nphil.lumatv.gfn` as a path component of its executable or arguments (the engine runs through the package's loader, so the engine's path is an argument) | `refused` |
| a fifth thread | `refused` |

It ends by itself: `unboost` puts back the thread's own policy, priority and affinity (read before the boost, not assumed); a
thread that ends or whose number is reused (checked by start time) is dropped untouched; 90 s after the last request the lease
expires and the thread is restored, so an app that hangs or is killed leaves nothing elevated. The helper checks the claim, not the
caller: any loopback process could ask for the same boost of the same thread, which is the grant itself and nothing wider. If the
jail turns out to have its own PID namespace the numbers the app sends will not match the helper's `/proc` and every request is
`not_found` (a Phase 4 check, below). Host tests: a made-up `/proc` and a recording scheduler (foreign process, other names,
root, tree depth, the limit, lease expiry, reused number, failed syscall, CPU hot-plug), and the pad crate against a fake helper
(request, renewal, release at grab-off and at stop, refusal and absent helper leave the pad working).

**The latency tool.** The engine always follows each button press (a report in which a button went down) through five instants, all
on `CLOCK_MONOTONIC` (the pad's evdev descriptor is switched to it with `EVIOCSCLOCKID`, so the kernel's own time stamp compares
without conversion): the kernel's stamp of the event (hid-sony delivering it), the reader's `read()` returning, the packet in the
engine's queue, the engine taking it from the queue to give the transport, and (optionally) the return of the send. Per press this
costs three clock reads. Once a second the finished presses are appended to `latency.tsv` and `latency.json` in the app's data directory
(`/var/lib/lumatv-gfn`, `LUMATV_GFN_LATENCY_DIR`), and the figures go to the app's stats line over the pad side channel:
with Show stats on, the line reads `120 fps · 78 Mbit/s · pad 0.4 ms (p99 1.3)`: the median and p99 from the kernel stamp to the
handoff (to the send's return when the preload is on), over the last 512 presses.

- `bin64/lumatv-gfn-latency run --dir /var/lib/lumatv-gfn --seconds 30 [--hid /sys/kernel/debug/hid/<dev>/events]` takes the presses
  stamped in the next 30 s and prints p50/p90/p99/max per stage and the handoff's histogram; `report [--last N]` does the same for
  what is in the file. It works while the stream is in front because it reads the engine's record, not the pad (which is grabbed).
- The send's return is stamped by `bin64/lumatv-gfn-sendtap.so`, an `LD_PRELOAD` library wrapping `send`, `sendto` and `sendmsg`
  (upstream's transport sends with `UdpSocket::send_to`, so the libc symbol is the only place to stamp it without patching upstream).
  The engine's wrapper loads it only when `/var/lib/lumatv-gfn/sendtap` exists. It reports the first send that returned after the
  handoff and within 20 ms of it; another packet that happens to leave in that gap makes the figure read early.
- `--hid` reads the HID layer's own tap (root only), which prints each input report as the USB driver hands it up, before hid-sony
  makes evdev events of it. The first report with a newly pressed button (DualShock 4 layout, which this receiver presents) gives
  the "USB arrival"; the row `USB arrival to evdev stamp` is the kernel's own delivery time. It can read slightly negative: the
  reading process wakes after the stamp is taken.

Host tests use a pseudo-terminal as the evdev node (the reader reads 24-byte `input_event` records from it exactly as from the real
node; its ioctls fail, so the stamp clock is configured), a FIFO for the HID events file, a fake preload probe, and the real
preload library loaded into a child process.

## Phase 4 on the TV

Needs: the GFN ipk built with the three new binaries (`gfn/build-ipk.sh`) and installed, the helper build with `/v1/gfn/pad-boost`, SSH
as root, Nitin at the set. The latency run needs no other change to the set.

1. Main: check the helper has the route: `curl -s http://127.0.0.1:18731/v1 | grep pad-boost`.
2. Nitin plugs the receiver into the port PDM calls "USB port 2" (the OHCI one, `8-1`: `ls /sys/bus/usb/devices/8-1`) and checks the
   pad's lights.
3. Main, optional (the send stamps): `touch /var/lib/lumatv-gfn/sendtap`, before the game starts (the engine's wrapper reads it at
   each stream start).
4. Nitin turns Show stats on in the app's overlay, starts a game and plays normally for a minute: the stats line shows `pad ...`
   after the first presses.
5. Main, once the game is running, checks the boost took: `curl -s http://127.0.0.1:18731/v1/gfn/pad-boost` lists the reader's
   `pid`/`tid`; `awk '{print $39, $40, $41}' /proc/<pid>/task/<tid>/stat` prints the CPU, the real-time priority and the policy, which
   should be a CPU other than 0 (it may move between 1 and 3), `10` and `1` (FIFO); `grep 'no boost' /var/lib/lumatv-gfn/streamer.log`
   finds nothing. A `not_found` there means the jail has its own PID namespace (the app's numbers are not the helper's).
6. Nitin presses buttons, varied and fast (A, B, X, Y, shoulders, d-pad) for 30 s, without a pause longer than a second, while Main runs:
   `/media/developer/apps/usr/palm/applications/org.nphil.lumatv.gfn/bin64/lumatv-gfn-latency run --dir /var/lib/lumatv-gfn --seconds 30 --hid /sys/kernel/debug/hid/0003:054C:09CC.<n>/events`
   (`ls /sys/kernel/debug/hid | grep 054C:09CC`; leave `--hid` off if the file is not there). About 100-200 presses make a usable histogram.
7. For the comparison without the boost, Main, straight before a second 20 s run: `curl -s -X POST -H 'X-Luma-Client: main' -H 'Content-Type: application/json'
   -d '{"pid":<pid>,"tid":<tid>}' http://127.0.0.1:18731/v1/gfn/pad-unboost`; the app renews in at most 30 s, so the run must start
   at once. Optionally repeat both runs with a second load (a 4K stream is load enough) and with CPU 2/3 hot-unplugged moments in dmesg.
8. Main puts the outputs, `latency.tsv` and the `ps`-style lines of step 5 in `docs/research/gfn-input/phase4-*.txt`, removes
   `/var/lib/lumatv-gfn/sendtap`, and leaves the set as Nitin had it.

Expected, from the measured rates and the scheduler facts (`[INFERENCE]` until the run), milliseconds after the kernel's evdev stamp:

| Stage | p50 | p99 | max |
|---|---|---|---|
| USB arrival to evdev stamp (`--hid`) | 0.00-0.10 | 0.2 | 0.5 |
| reader woke | 0.05-0.15 | 0.3 | 1 |
| queued | +0.01 | 0.3 | 1 |
| handoff (the headline) | 0.2-0.3 | 0.6 | 1.5 (2 without the boost under load) |
| send return | 0.3-0.5 | 0.9 | 2 |

A handoff p99 over 1 ms with the boost on and the reader on a CPU other than 0 means something else is in the way (the 250 µs drain is
uniform and cannot exceed 0.25 ms by itself); a send return far after the handoff means the transport thread is the delay, not the
pad. Add 1.5 ms (radio cycle and USB poll, which the stamp does not include) for the pad-to-packet figure in the table above.

## Remaining ideas (not done)

- Pin the OHCI interrupt (IRQ 128, `ohci_hcd:usb8`) to CPU 1 with `/proc/irq/128/smp_affinity`: root only, and it changes LG's own IRQ
  layout (the HDMI interrupt runs FIFO 50 beside it); listed, not done. The reader's affinity off CPU 0 already avoids queueing behind it.
- Wake the engine's queue drain on a push instead of the core's 250 µs poll (average 0.13 ms): needs a change to the pinned OpenNOW
  core or a wake-up it already listens for.
- Stamp the send inside the transport instead of by preload: also an upstream change.
- Give the engine's drain and transport threads the same narrow FIFO boost, if the run shows they are the delay (a wider grant, so
  only on evidence).
- Skip the input layer by reading the pad's `hidraw` node (tens of microseconds); the jail's `hidraw` nodes are static copies, so
  it is not usable from the app.
