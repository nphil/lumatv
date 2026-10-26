# Pad guard: the 2.4 GHz controller receiver usable for games, invisible to Netflix when the pad is off (PadGuard)

## Resume here

Built, host-tested (24 tests) and installed on the set on 2026-10-25; verified with Nitin switching the pad on and off. Pad on: the
USB interface is bound to usbhid/hid-sony and the input nodes exist within 1 s. Pad off: the nodes are gone about 1.5 s after the
receiver reappears on USB (about 3.5 s after it dropped off USB at switch-off), and Netflix, run for about 60 s with the pad off, held no pad node.
Nothing is left to do unless Nitin reports a pad that does not attach or a stick that sticks; look at `/tmp/padguard.log` on the TV.

| What | Where |
|---|---|
| Daemon (single instance, flock on `/tmp/padguard.lock`) | `payload/padguard/padguard.py` -> `/var/lib/webosbrew/padguard.py` |
| Boot hook, replaces `50-padblock` (kept as `/var/lib/webosbrew/50-padblock.off-20261025`) | `payload/padguard/50-padguard` -> `/var/lib/webosbrew/init.d/50-padguard` |
| Tests (fake sysfs/devfs, fake prober) | `tests/test_padguard.py` (`python3 -m unittest tests.test_padguard`) |
| Log (transitions only, truncated at 64 KB) | `/tmp/padguard.log` on the TV |

Uninstall: delete `init.d/50-padguard`, kill the daemon, `echo 8-1:1.0 > /sys/bus/usb/drivers/usbhid/bind` if the pad is detached.

## What the receiver does (measured on the set)

The receiver (054c:09cc, `usb 8-1`, full speed, generic-ohci) never goes quiet:

| State | On the wire | evdev |
|---|---|---|
| Pad on | report ID `0x01`, 64 bytes, about 1000/s, sticks near `0x80` | normal |
| Pad off (receiver still enumerated) | report ID `0x00`, 64 bytes, about 500/s, all zero except a 2-byte counter at bytes 10-11 | nodes exist, hid-sony ignores the reports, every axis stays at its initial 0 (stick held up-left) |

Report presence is therefore not liveness; the report ID is. Switching the pad off also makes the receiver drop off USB and come back
about 2 s later (new device number; the first enumeration attempt fails with -62, the second works), and the kernel binds usbhid
straight away, so nodes appear for a moment by themselves. Switching it on does not re-enumerate the receiver.

## How the guard works

- **Attached (usbhid + hid-sony bound).** Every 0.5 s the pad's hidraw node is read for up to 0.2 s and live (`0x01`) reports are counted.
  A separate hidraw reader does not take events from the evdev nodes. Fewer than 10 in two samples in a row: usbhid is unbound from the
  interface (`/sys/bus/usb/drivers/usbhid/unbind`), so all nodes vanish. A pad found already bound (boot, receiver re-enumerated) is judged
  at once; after the guard's own bind there is a 1 s grace while hid-sony probes.
- **Detached.** Nothing polls the endpoint once usbhid is unbound (urbnum stops), so the guard claims the interface through usbfs
  (`/dev/bus/usb/BBB/DDD`, ioctl `CLAIMINTERFACE`) and every 0.5 s submits an interrupt-IN URB on the endpoint for 30 ms. A pad that is off
  yields ID-0 reports and no verdict. Once a live report appears the window extends to 1 s; 10 live reports release the interface and bind
  usbhid again, which creates the nodes. A URB left in flight all the time was tried first and cost about 9 % of a core (a wakeup per
  report from a pad that is off); sampling costs about 0.8 %, attached or detached.
- **Why not the alternatives.** Unbinding only hid-sony stops usbhid's polling too (measured), so nothing is left to read. The sysfs `urbnum`
  counter cannot tell a live pad from an off one (both stream). Probing by rebinding would show nodes with a dead stick each cycle.
  Leaving the pad bound would leave Netflix the stuck-stick nodes.
- **Window of exposure.** Between the receiver reappearing and the guard's second sample (about 1.1 s measured on the set) the nodes
  exist with a stick at 0. The kernel binds before any userspace can intervene; shortening it would need a usbhid quirk, and the
  parameter is read-only on this kernel.

## Verification on the set (2026-10-25)

| Step | Result |
|---|---|
| usbfs probe with the pad on | 999 reports in 2 s, first after 1.4 ms |
| Pad on, hidraw read 3 s | 3001 reports, all ID `0x01`, sticks `0x80 0x7e`, counter and motion bytes moving |
| Pad off, usbfs read 4 s, no input node present | 2000 reports, all ID `0x00` |
| Pad switched on | receiver already enumerated; nodes present 1 s after (`reports flowing, attached`) |
| Pad switched off (tuned guard) | `gone` 20:31:10, back 20:31:12, nodes 20:31:12.4, detached 20:31:13.5 |
| Netflix, pad off, 60 s, receiver muted | no 054C device in `/proc/bus/input/devices`; Netflix held two input fds, none of the pad's; closed, Home in front, receiver unmuted |
| Manual usbhid unbind while the guard ran | claimed and re-bound within 1.5 s |

Not exercised: pressing buttons on the evdev node (hidraw report counts stand in), a charge-only wired pad (no reports at all: handled the
same, never attached), a second pad on another port (covered by a host test only).

## Notes for GfnInput's stream work

While the pad is on, `/dev/hidrawN`, `eventN` and `jsN` for the pad exist and are stable; across an off/on cycle the numbering changes
(device number, hid instance and event numbers all advance), so open by name/uniq (`1c:a0:b8:4c:7a:99`), not by number.
