# Bluetooth LE sensors and presence for Home Assistant

Census rank 5 (`hardware-census.md:23,111-112,149`; `PLAN.md` table row 5; `HANDOFF.md:92`). Built in the helper; the TV
slot has not run. Evidence from the set is in `ble/` (read-only reads and strings, 2026-10-25).

## The radio

There is no `hci0`: the controller (`/dev/stpbt`, MediaTek MT7668 combo, Bluedroid stack, firmware `12:11:01:22`) belongs to
`webos-bluetooth-service` (`hardware-census.md:111`). Its luna service `com.webos.service.bluetooth2` is the only way to hear
advertisements. Raw HCI through the same service (`adapter/internal/sendHciCommand`) exists and is not used: it would bypass the
stack that also carries the Magic Remote.

LE scan API (`ble/startscan-evidence.txt`, webOS OSE reference
<https://www.webosose.org/docs/reference/ls2-api/com-webos-service-bluetooth2/>, section `le/startScan`):

| | |
|---|---|
| Call | `luna-send -n 1000000 luna://com.webos.service.bluetooth2/le/startScan '{"subscribe":true}'` |
| Parameters | `subscribe` (required), `address`, `name`, `adapterAddress`, `serviceUuid`, `serviceData`, `manufacturerData` filters, `always` (not documented) |
| Reply to the call | `{"subscribed":true,"returnValue":true,"adapterAddress":"..."}` |
| Later replies | `device` (the one heard: new, or a changed property such as RSSI) and `devices` (all known); a device is `{address, name, rssi, typeOfDevice:"ble", scanRecord:[bytes], manufacturerData, ...}`, the object `device/getStatus` shows on the set |
| Permission | group `bluetooth.manage`; every `com.webos.lunasend-*` client has the group `all`, and `bluetooth.query` reads (`adapter/getStatus`, `le/getStatus`, `device/getStatus`) answered to `luna-send` on the set |
| End | the scan lasts as long as the subscriber; the service logs `StartScan listener dropped` |

The set today: adapter `84:a3:29:b9:4d:43` powered, discoverable, LE advertising already on (`le/getStatus`), the Magic Remote
(`LGE MR24`, BLE) and one classic device paired (`ble/tv-reads.txt`).

Not used: `le/internal/startScan` (adds `advertisingType`), `startMultiScan` (filters) and `setScanParameters` (interval and
window): they change the service's scan for every client, including LG's own.

## How advertisements reach Home Assistant

Decision: the TV answers as an **ESPHome Bluetooth proxy**, the protocol of LumaShow's `ble-proxy` plugin
(`/data/home/LumaShow/lumashow/plugins/ble-proxy`), built on the same `esphome-api` crate (Noise `NNpsk0` framing, messages) and
the same `mdns` responder. Home Assistant's own ESPHome and Bluetooth integrations then do the rest (every BLE sensor integration,
iBeacon, Private BLE Device), with nothing installed in Home Assistant. Alternatives rejected: MQTT (the helper would need a
decoder per sensor family and a broker), a custom integration (Nitin installs code in Home Assistant, and every sensor family is
reimplemented).

Differences from the plugin: no HCI host (LG's service replaces it), **passive scanning only** (features `PASSIVE_SCAN |
RAW_ADVERTISEMENTS | STATE_AND_MODE` = 97; no connections, pairing or cache, so sensors that need a GATT connection are out of
scope), and the radio limits below. Modules (`helper/app/src/`): `ble_proxy` (protocol), `ble_scan` (luna and LG's JSON),
`ble_duty` (limits), `ble_store` and `ble_cli` (the owner's link), `ble` (sockets, mDNS, the loop).

## Radio limits

One antenna serves Wi-Fi and Bluetooth, so a scan costs Wi-Fi throughput and Bluetooth audio. Enforced in code, not settings
(`ble_store::check`, `ble_duty`):

- a window of 5 to 30 s (default 10), then a rest at least as long (default 50), at most 600 s: never more than 50 % of the time,
  a sixth by default;
- the rest counts from the end of a window even when Home Assistant reconnects in a loop;
- no window while the panel is dark (unless `--standby`), a Bluetooth audio or hands-free link is up (`device/getStatus`), or the
  adapter is off; held guards are looked at again after 5 s (screen) or 30 s (radio);
- a failed, refused or silent (5 s) scan backs off 1, 2, 4, 8 minutes; the fifth failure in a row ends the trying until the owner
  links again;
- the scan child is killed at every window end, on every failure, with no subscriber, on unlink and on stop, and by the kernel
  (`PR_SET_PDEATHSIG`) if the helper is killed;
- LG's replies are read one device at a time (`device`, not the whole `devices` list), at most 1024 queued.

## Principle 0

Off by default: no `ble/proxy.json`, no socket, no announcement, no scan (one look at the file every 10 s). Only the owner's
`lumatv-helper ble-link` (not a route, setting or assistant) creates it. The socket binds the TV's private address (never
`0.0.0.0`), serves private peers, keeps four clients, and exists only once LG's service has answered. The key is 256 bits from the
kernel, shown by `ble-link`, never logged. What is heard is never stored, logged or put in the topic `ble`
(`GET /v1/state/ble`: state, counts, last problem).

## Checked on the host

- 70 unit and rig tests (`cargo test -p lumatv-helper --lib ble`): LG's JSON in the documented shapes, duty and backoff in virtual
  time, the proxy against the real client end of the protocol, the runner over real loopback sockets with a fake radio, and
  `LunaBluetooth` against a fake `luna-send` script (replies pretty-printed over lines, a child that exits, the child killed at
  stop and with its starting thread).
- `helper/smoke-ble.py`: the real helper binary, a fake `luna-send`, and Home Assistant's client library (`aioesphomeapi` 46.22):
  device info (flags 97, Bluetooth address), scanner state idle then running, about 5 advertisements a second in the window and none
  in the rest, and a SIGKILLed helper takes its scan child with it.

Not checked, because it needs the set: everything in the next section.

## Unknowns the TV slot answers

1. Whether `luna-send` is allowed `le/startScan` (everything else says yes) and what its first reply is.
2. Whether `scanRecord` holds the advertisement and the scan response, and is not empty (otherwise `ble_scan::advertisement`
   rebuilds name and manufacturer data only).
3. Whether a device that repeats the same advertisement with the same RSSI produces a reply; if not, `--always` is the candidate.
4. Whether the scan is passive, and whether the MT7668 is silent on its first scan (as it is on a raw HCI host).
5. The cost: Wi-Fi round trip and CPU during a window, with a busy room (each reply carries the whole device list).
6. Whether the Magic Remote, Bluetooth audio or a standby TV is disturbed.

## TV step list (slot of about 10 min; Main's OK first; abort on any drop of the remote or audio)

1. Baseline, read-only: `luna-send -n 1 luna://com.webos.service.bluetooth2/adapter/getStatus '{}'` (as in `ble/tv-reads.txt`);
   `ping -c 20 <gateway>` and note the round trip; load of `top -b -n 1`.
2. One scan, 15 s, a known tag or a phone advertising (nRF Connect) in range, the remote and a Bluetooth speaker idle:
   `timeout 15 luna-send -n 1000000 luna://com.webos.service.bluetooth2/le/startScan '{"subscribe":true}' > /tmp/ble-scan-1.txt`
   with `ping` running beside it. Check: unknown 1 and 2 above, the remote still answers, no `luna-send` left over
   (`ps | grep startScan`), the log says `StartScan listener dropped`. Do not commit the file (neighbours' addresses): keep the shape
   and redact.
3. The same with `'{"subscribe":true,"always":true}'`, and the count of replies for a 1 Hz beacon against step 2.
4. Install the ipk with the helper, then (the owner) `lumatv-helper ble-link --brand <brand.json>` and read
   `/v1/state/ble`: `listening`, then `scanning` after Home Assistant subscribes. Watch `silentWindows` (unknown 4) and the
   round trip during a window against step 1.
5. With the remote in use and then Bluetooth audio playing: the proxy must go `held` for audio and the remote must not stutter.
6. Undo: `lumatv-helper ble-unlink --brand <brand.json>`; within 10 s the socket closes, `ps` shows no `luna-send`, the state is
   `off`.

## Nitin's steps in Home Assistant

1. Have `lumatv-helper ble-link` run on the TV (Nitin's own act: it opens a port on the home network); it prints the TV's address,
   the port (6053) and the **key**. Keep the key; it is asked once.
2. Home Assistant: Settings, Devices & services. "lumatv-ble" should appear under Discovered: Configure, paste the key, Submit. If
   it does not: Add integration, ESPHome, enter the TV's address and port 6053, then the key.
3. Settings, Devices & services, Bluetooth, Configure: "lumatv-ble" is listed as a scanner (passive). Sensors and trackers
   appear on their own as Home Assistant recognises them (Xiaomi BLE, Govee, iBeacon, Private BLE Device for phones). For room
   presence by signal strength, Bermuda (HACS) can use this scanner; it sees the TV only for 10 s a minute by default.
4. To take it out: delete the device in Home Assistant, then `lumatv-helper ble-unlink`.

Duty is set with `ble-link --scan-seconds 20 --rest-seconds 40` (the key stays); `--standby` allows scanning with the panel off.
