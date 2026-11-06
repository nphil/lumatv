# USB Ethernet adapter: design (UsbEthDesign, 2026-11-06)

Design only; nothing was built and nothing on the TV was changed. Evidence tags: `TV:` read on this set on 2026-11-06, read-only; `LOG:` the set's syslog on
BeastNAS (`/mnt/user/system/syslog-192.168.1.242.log`, 2026-10-31 to 11-06); `REF:` [ohmybahgosh/LG-USB-2-ETH0](https://github.com/ohmybahgosh/LG-USB-2-ETH0)
2.4.16 at `1b1b48e` (MIT, POSIX sh) and its docs; `PUB:` public documentation; `[INFERENCE]` not run.

## Verdict

- **Two cables, no renaming.** The adapter joins as a second wired port that connman runs like any other (`eth1`, its own MAC and lease). While the screen is on
  and the adapter has an address, the `usb-ethernet` plugin steers the TV's traffic onto it. The built-in port keeps `eth0`, `74:c1:7e:bd:ae:30` and
  192.168.1.242 throughout, so Wake-on-LAN, Always Ready, the tuner, the BLE proxy and LG's own network status stay stock. When the adapter goes (pulled, or its
  power cut in Always Ready), traffic is back on the built-in port with nothing to repair.
- **The drivers are already in the kernel.** `r8152` v1.10.11 (RTL8152 and RTL8153), `ax88179_178a` and `cdc_ncm` are built in; no module is loaded. The adapter
  to buy is a USB-A gigabit **RTL8153**.
- **Ceiling about 300 Mbit/s**, three times the built-in port's 95: LG specifies all three sockets as USB 2.0, and an RTL8153 on `r8152` measures 290 to 330
  Mbit/s there (REF). More only if the plug-in check reads 5 Gbit/s.
- **Nothing on the boot path.** Every change is runtime state the helper applies after it starts; a reboot is stock.

## Drivers

TV: kernel `5.4.268-329.ptl4tv.2` (aarch64, built 2025-04-17), no `/proc/config.gz`, no `.ko` files. `modules.builtin` lists `r8152`, `ax88179_178a`, `asix`,
`cdc_ether`, `cdc_ncm`, `rndis_host`, `cdc_subset`, `rtl8150` and `usbnet`; `/sys/module/r8152/version` is v1.10.11, which drives RTL8152 and RTL8153 but not
RTL8156. REF's firmware audit finds the same drivers built into firmware 33.31.69 (kernel `ptl4tv.5`), so they survive a firmware update.

LG's `r8152` predates the configuration selector Linux 5.11 added, so an RTL8153 comes up in its standard NCM configuration and `cdc_ncm` takes it (REF: 200 to
255 Mbit/s, with drops). Writing 1 to the adapter's `bConfigurationValue` hands it to `r8152` (REF `realtek_mode`: 290 to 330 Mbit/s); if `r8152` has not bound
within 5 s, the old value goes back. The adapter returns on `cdc_ncm` after every standby (REF 2.4.2), so the switch repeats on every appearance.

Prebuilt out-of-tree modules exist for this exact kernel (REF's `lg-webos-usb-drivers` at `fc8753f`, `experimental/5.4.268-329.ptl4tv.2/`), built from LG's C2
source (webOS25 WG ReNew, kernel 324) with struct offsets checked, never run on a TV. SHA-256:

- `r8152_oot.ko` (Realtek r8152 v2.22.1 with DonZoomik's patches; adds RTL8156): `450d1655b507c61a50cc94640926c727bd528bfb4bbd0e2b277f7e50c0ad81a9`
- `aqc111.ko`: `0336bd70566e72a418c201532c300d736c3a2b080044dae7a5cebe6652b2b6bb`
- `dm9601.ko`: `6dc9b71cbeebb5a45f09c817777b6f4bb8db8ab4e3678635ca1b0c0ff177b3ee`
- `lan78xx.ko`: `14a2ef530ec8cd2e53f65ac8bce344709c773d401a4a60b6ab9fc68c63e6d824`

DonZoomik's tested `r8152_oot.ko` (`fe1691622747c106af44f7834675ec75ca3327a797c6bbec394e2998db66e8cb`) is for `ptl4tv.5` and does not load here: LG kernels have
`MODVERSIONS` off and no module signing, so `vermagic` must match exactly. Under `r8152_oot`, scatter-gather must be off (`rtl_adv/sg_en`) or LG's xHCI hangs
(REF). The design uses none of them: RTL8156's 2.5 Gbit/s gains nothing behind USB 2.0, and a module stops matching at the next firmware update.

Building one (REF `docs/driver-build.md`, `scripts/driver-build/build.sh`): LG's kernel package from [opensource.lge.com](https://opensource.lge.com/), the TV's
own config taken from its firmware (`scripts/firmware-audit/audit.sh SAVE=DIR`), a build with `5.4.268-329.ptl4tv.2` as the version, then checks of `vermagic`,
imported symbols and struct offsets.

Licences: the kernel drivers and `r8152_oot` are GPL-2.0 (shipping a module means offering its source). REF is MIT; no REF code is copied, its findings are
credited here. iperf3, the speed-test server, is BSD-licensed.

## USB sockets and speed

TV: three controllers reach the sockets, each with VBUS and over-current lines in the device tree (`ctl-gpios`, `ocd-gpios`):

- `xhci-hcd.1` (`usb@ca080000`, buses 3 and 4) and `xhci-hcd.2` (`usb@ca100000`, buses 5 and 6): xHCI, each with a 480 Mbit/s and a 5 Gbit/s root port;
- EHCI and OHCI at `usb@ca180000` and `usb@ca184000` (buses 7 and 8): USB 2.0 only, holding the game controller's dongle (054c:09cc on 8-1).

The fourth controller, `xhci-hcd.0` (buses 1 and 2), is internal: Wi-Fi and Bluetooth (043e:310e). Which physical socket is which is not mapped; the adapter's
bus number tells. PUB: LG lists "USB Input: 3ea (v 2.0)" for the OLED65C4PUA ([lg.com](https://www.lg.com/ca_en/tv-soundbars/oled/oled65c4pua)), and
[displayspecifications.com](https://www.displayspecifications.com/en/model/1b82389c) 3 × USB 2.0. [INFERENCE] The chip's 5 Gbit/s ports are not wired to the
sockets; the adapter's `speed` file (480 or 5000) settles it.

Measured by REF on USB 2.0:

| Adapter and driver | TV | Mbit/s |
|---|---|---|
| Built-in port | LG sets | ~95 |
| RTL8153, `cdc_ncm` | C4 | 200–255, with drops |
| RTL8153, `r8152` | LG sets | 290–330 |
| RTL8156, `r8152_oot` | C2 | 272 down, 287 up |
| AX88179, built-in `ax88179_178a` | G4 (2024) | 292 down, 51 up |
| AX88179, built-in `ax88179_178a` | UN7300 | 228 down, 325 up |

USB 2.0 is shared by both directions, so download and upload together stay near 300 Mbit/s; REF saw a C2's busiest core a third busy at that rate. The
AX88179's 51 Mbit/s upload on a G4, a 2024 OLED on the C4's kernel family, rules it out. At 5 Gbit/s an RTL8153 could approach gigabit line rate (~940 Mbit/s);
no LG set has been measured there. TV: the built-in port is `stmmac` on a 100 Mbit/s PHY ("LG1K Fast Ethernet Transceiver").

## Rename or connman

Stock webOS: connman runs every wired port and keys its services by MAC (TV: `ethernet_74c17ebdae30_cable` for the built-in, and `ethernet_3c18a0426393_cable`,
last leased 192.168.1.77 on 2026-01-16: an adapter used on this set before). `main.conf` (read-only) prefers ethernet to Wi-Fi but cannot rank one wired port
above another; [INFERENCE] between two connected wired services connman keeps the first default. LG's status names the wired port `eth0` (TV:
`connectionmanager/getStatus`); REF found that with the adapter as `eth1` and no link on `eth0`, Settings says "Not Connected" and Paramount+ refuses to play.
The kernel has no policy routing (TV: `ip rule` gives "Operation not supported"), so traffic cannot be split by source address.

**Rename**, REF's way (built-in to `eth2`, adapter to `eth0`), fits one cable. Here it would cost Wake-on-LAN, the tuner, the BLE proxy and Home Assistant in
standby, since Always Ready cuts the adapter's power (below). With a second cable the parked built-in has to stay down, but connman raises downed ports again
(REF retries each rename on "Device or resource busy"), and [INFERENCE] a raised built-in either repeats the MAC on the LAN or takes a second address. Every
screen-off and screen-on becomes a swap of `eth0`, and a failed swap leaves no `eth0` until a reboot.

**connman plus steering** (recommended). Nothing is renamed and connman leases the adapter as `eth1`. While the screen is on and both ports have addresses, the
plugin:

1. adds 192.168.1.0/25 and 192.168.1.128/25 through the adapter, with its address as source. More specific than the built-in's /24, they carry all LAN traffic
   (BeastNAS and Plex at .69, the tuner's streams) without touching a route the kernel or connman made;
2. sets `arp_ignore=1` and `arp_announce=2` on both ports, so each answers ARP only for its own address and the router keeps .242 on the built-in's MAC;
3. makes the adapter connman's default service (`connmanctl move-before`, connman's `MoveBefore`), so connman moves the default route itself and internet traffic
   follows. Streaming services stay well under 95 Mbit/s; GeForce NOW at its top bitrates is the case that needs it.

Steps 1 and 2 leave connman and LG's view exactly stock. Step 3 changes connman's default and is the open question: whether LG's status stays "Connected" and
connman keeps the order (check T3). If it fails, the plugin ships with steps 1 and 2 only. Replies to connections that came in on the built-in leave through
the adapter, which this kernel allows (TV: `rp_filter` 2, loose). [INFERENCE] Broadcast and multicast arrive on both ports, so a discovery request can get two
answers carrying the same identity.

## Address, identity and Wake-on-LAN

The TV's identity stays on the built-in port, so neither a MAC clone nor a new reservation is needed: 192.168.1.242 and `74:c1:7e:bd:ae:30` never move
(HomeLabber `router_clients`: "LGwebOSTV", wired, .242 on that MAC). That keeps:

- the HDHomeRun device id, made from `/sys/class/net/eth0/address` (`config.rs` `HdhrConfig.mac_file`, `tuner.rs:184`): Plex keeps its tuner;
- the BLE proxy's ESPHome identity and onhome's `lumatv-<last6>` name (`ble.rs:375` `interface_mac`, `ble.rs:636`, `onhome/mod.rs:387,425`): Home Assistant keeps
  its devices;
- Home Assistant's `automation.living_room_tv_wake_on_lan`, which sends the magic packet to `74:c1:7e:bd:ae:30`.

The adapter takes any address; a UniFi fixed IP and name for its MAC only makes it easier to recognise. Step 3 needs one helper change: `sys::lan_ipv4` binds
listeners to the default route's interface, so it would follow the adapter to its own address and back at every screen-off, rebinding the BLE proxy
(`ble.rs:600`) and, through `interface_mac`, showing Home Assistant another MAC. It takes `eth0`'s private address first.

A MAC clone belongs to the rename approach: set before connman's DHCP, it would give the adapter .242 and every identity above. This design does not use it.

Wake-on-LAN works today from Always Ready: LOG has 14 power-ons with `PowerOnReason` `wakeOnLan`, beside 43 `remoteKey` and 2 `hdmiCec`. Nothing in userspace
listens for magic packets (TV: no UDP 9 or 7 listener, no packet socket), so [INFERENCE] LG catches them below userspace on the built-in port. Standby starts in
the stock arrangement (next section), so that path is untouched.

## Standby, LG's reboots and boot order

Always Ready cuts power to the external USB sockets, measured on the USB 2.0 one (LOG, 2026-11-06): the dongle drops a second after the change to Always Ready
(16:59:36, gone 16:59:37) and re-enumerates one to two seconds before each return to Active (15:48, 15:59, 16:30, 16:36, 16:53). TV: in Always Ready, 8-1 is
absent while the internal Wi-Fi (1-1) stays. [INFERENCE] The xHCI sockets behave the same (check U4). Through standby, then, the built-in port carries the tuner,
the BLE proxy, Home Assistant and Wake-on-LAN, as it does today.

The plugin does not wait for the power cut. It follows the helper's `screen` topic (tvpowerd's power state) and hands everything back when the panel goes off,
about 7 s before Always Ready begins (16:59:29 to 16:59:36 on Nov 6). Standby therefore starts in the stock arrangement, whatever LG does to the network on
entering it and even if a socket keeps its power. At screen-on the adapter returns on `cdc_ncm`; the plugin switches it to `r8152`, connman leases it, and
steering resumes, [INFERENCE] 5 to 8 s after the panel lights (check T5). Until then, traffic uses the built-in port.

LG reboots the set from Always Ready on its own: LOG has `rebootByARRegular` cold reboots on Nov 2 at 08:54 and 18:45 and Nov 6 at 06:42, each back in Always
Ready (the boot worker logged `standby:true`; Always Ready again by 06:42:55). The adapter then stays unpowered until screen-on, which the steps above cover.

Boot order: nothing is needed early. connmand starts at 12.89 s (TV) and leases ports as they appear; the boot worker (the `kdump` hook, 12 to 14 s) starts the
helper, which applies steering once the plugin is on and the screen is on. Until then, the built-in port carries traffic as now. Nothing is added to the
`init.d` hook (`50-lumatv`) or the boot worker.

## Safety and undo

- **Fallback** is the built-in port: always cabled, always up, never renamed, re-addressed or taken down. No Wi-Fi fallback is needed; REF turns Wi-Fi on after
  60 s without carrier for one-cable setups, which here would only add a third address and MAC.
- **Pulling the adapter mid-use**: the kernel removes its routes with the interface and connman falls back to the built-in's service. Connections opened through
  the adapter break and their apps reconnect (a stream re-buffers). Nothing is left to repair.
- **One-step undo**: switching the plugin off in Settings, or "Use the built-in port" on its page. Either deletes the two routes it added, puts the built-in
  first in connman again and sets the ARP settings back to 0. Pulling the adapter undoes the routing by itself.
- **Reboot**: every change is runtime state (two routes, connman's order, ARP settings, the adapter's USB configuration), so a reboot is stock whether or not
  the plugin is on; with it on, the helper reapplies after it starts. [INFERENCE] `MoveBefore` also stamps the two services' `Modified` time in
  `/var/lib/connman`, and connman orders services from scratch at boot (check T6).
- **The 10 s Home escape** and the fault fallback stop the helper (`lumatv-sm` kills everything under LumaTV's directories, then runs `recover-home.sh --stock`).
  Without the helper, steering lasts until the adapter loses power or is pulled; the stock UI has a network either way, and neither script changes. On its
  next start the helper reapplies if the plugin is on and restores if it is off.

## Architecture

**Engine `lumanet`** (`helper/net`, a workspace member like `helper/tuner`): no LumaTV dependency; `libc` plus `serde_json` for iperf3's parameters. A small
CLI (`status`, `steer`, `restore`, `speedtest`) makes it usable on its own.

- `watch`: one rtnetlink socket on the link, IPv4 address and IPv4 route groups, read as events.
- `usb`: facts for a network interface from sysfs, under a root path that tests replace with a fake tree: vendor and product IDs and names, bound driver, USB
  speed, link speed and duplex, MAC, carrier.
- `realtek`: the `bConfigurationValue` switch with its 5 s revert.
- `steer`: a pure function from what is observed (adapter and built-in addressed, screen on, paused) to what is wanted (the two routes, the ARP settings,
  connman's first service), then an idempotent apply and a `restore`, through rtnetlink, `/proc/sys` and `connmanctl`. Tested without a TV.
- `speedtest`: an iperf3 TCP client (download is iperf3's reverse mode), its socket bound to one port (`SO_BINDTODEVICE`) so each port is measured alone.

**Plugin `usb-ethernet`** (`helper/app/src/usbeth.rs`, shaped like `tuner.rs`): first-party, listed in `FIRST_PARTY_IDS`, off until Nitin switches it on, with
an `OptIn` consent in plain words ("Moves the TV's network traffic to a USB Ethernet adapter while one is plugged in and the screen is on. Changes routing only
while switched on."). On, a thread runs the engine with the `screen` topic as an input; off, it restores and exits. The privileged parts (a sysfs write, routes,
sysctls, `connmanctl`, binding a socket to a port) run in the helper, which runs as root (TV: uid 0), and only while the plugin is on. A sandboxed plugin could
not do this: plugin API level 1 has no network access.

Its Settings page is a first-party page like the tuner's (facts and actions through `Local`, styled only by the theme): adapter (product name), chip (from
vendor and product, such as RTL8153 for 0bda:8153), driver, link speed, USB speed (480 is USB 2.0), the adapter's address and the TV's (.242, built-in port),
state (carrying the TV's traffic / screen off, built-in port / no adapter), a speed test (down and up, adapter and built-in, against `server`, default
192.168.1.69:5201) and "Use the built-in port".

Boot hook point: none (above). Core change: `sys::lan_ipv4` prefers `eth0`, needed for step 3 only.

## Steps and checks

Nitin, by hand: plug an RTL8153 USB-A adapter into a free socket (not the one holding the controller dongle) and run a second cable to it from the switch; the
built-in port stays cabled.

Read-only checks with it plugged in, before any code:

- U1: the adapter's bus and `speed` (480 or 5000).
- U2: bound driver (`cdc_ncm` expected), interface name (`eth1` expected), MAC.
- U3: connman's services and default, `ip route`, LG `getStatus`, Settings › Network.
- U4: screen off for a minute: is the adapter's USB device still there?

Build: `lumanet` and the plugin with their tests and `cargo clippy`, the `lan_ipv4` change, and the iperf3 server on BeastNAS once approved. Install from `main`.

On the TV with the plugin on, each with a receipt:

- T1: the Realtek switch: `r8152` bound, link 1000 Mbit/s full duplex.
- T2: speed test, adapter against built-in, down and up; 250 Mbit/s or more counts as working on USB 2.0.
- T3: steering holds for 30 minutes: LAN routes and the default route through the adapter; Settings shows Connected; Netflix and Paramount+ play; a Plex 4K
  remux plays without buffering.
- T4: identity: Plex sees the tuner at .242 (`discover.json`), Home Assistant's BLE proxy stays connected under the same MAC, UniFi shows .242 only on
  `74:c1:7e:bd:ae:30`.
- T5: three screen-off and screen-on cycles: traffic back on the built-in before Always Ready; Wake-on-LAN from Always Ready through the Home Assistant
  automation; steering back after screen-on, timed.
- T6: one reboot (by hand, or LG's next regular one): stock until the helper starts, steering after.
- T7: the adapter pulled mid-stream: playback carries on through the built-in after a re-buffer.
- T8: the 10 s Home escape with the plugin on: the stock UI has a network.
- T9: plugin off: `ip route`, the ARP settings and connman's order match a snapshot taken before T1.

## Decisions for Nitin

1. A second cable at the TV (a free switch port, or a small gigabit switch behind it). With one cable the TV has no network in standby.
2. The adapter: a USB-A RTL8153. The one this TV saw in January (MAC `3c:18:a0:42:63:93`, made by Luxshare) will do if it is still around and is an RTL8153.
3. An iperf3 server container on BeastNAS for the speed test: LAN only, port 5201, no hostname.
4. Optional: a UniFi fixed IP and name for the adapter.

If U4 finds the adapter keeps power in Always Ready, one cable becomes possible with the rename approach and a MAC clone; Wake-on-LAN through an adapter would
still be untested.
