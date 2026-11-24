# Multi Link

Multi Link is off by default at Settings › Network › Wired › Multi Link. It keeps the saved Wi-Fi connection beside Ethernet while the screen is on and offers source addresses to apps that support parallel downloads. A USB gigabit adapter is the preferred first upgrade: one sufficiently fast wire needs neither aggregation nor another radio. USB Ethernet, built-in Ethernet and Wi-Fi share the same link model.

The helper owns preparation; the reusable `multilink` crate owns selection, passive traffic measurements and transactional rollback. The existing Settings schema renderer supplies every row and theme token. The page has no overlay, animation or new long-dwell element; normal Settings idle/panel-care rules apply.

## Network policy

A healthy, unambiguous wired IPv4 default route is required. Each additional address must be private, on the same subnet and on a different eligible physical interface; Wi-Fi also needs a reported signal of at least −75 dBm. Unknown state withdraws extra-link eligibility. A client must still prove that its particular server returns data to each bound address; link state is not an end-to-end connectivity certificate.

For multiple eligible links, the helper sets each link's `arp_ignore=1`, `arp_announce=2` and `rp_filter=2`. These isolate the addresses and admit the asymmetric return path. Global `all`/`default` values, connected routes, IPv6, DNS, public host routes and `p2p0` are not changed. A stricter global ARP rule prevents activation rather than being weakened.

While Wi-Fi is a candidate, the Plex server's own address gets a /32 route through the wired gateway on `wlan0` with the Wi-Fi address as source (`ip route replace <server>/32 via <gateway> dev wlan0 src <wlan0 address>`; the gateway is the router, so a server on another subnet, such as 192.168.5.10 behind the router, needs no other route). The address is not configured: it is the one an app has an established TCP connection to on port 32400, re-read every 2 s; the last such address is kept in `<helper data>/multi-link/plex-host` across idle spells and reboots, and the address plex.tv lists for the signed-in server is used only before any has been seen. When the address changes, or Wi-Fi stops being a candidate, the old route is restored from the journal (normally: removed, so the server is reached over the wire again).

Every other private IPv4 /32 route through `wlan0` and the same gateway as the wired default is temporarily moved to the wired interface, without a preferred source address. This removes destination-only Wi-Fi workarounds that would otherwise carry controls and acknowledgements over the radio. Other routes stay LG's; unmodified apps do not gain aggregation. The helper neither installs a new default route nor competes with ConnMan's technology preference.

The private journal is `<helper data>/multi-link/rollback.json`, mode 0600. Each original is written and synced before its mutation; application and restoration require readback. Disable, standby, normal exit and panic unwinding restore owned changes. Startup recovers an unfinished journal before enabling anything. Reboot recovery discards old kernel-only values but restores borrowed Wi-Fi power/profile state. A later external change is not overwritten; failed restoration remains in the journal and is reported.

Wi-Fi credentials remain entirely with LG. A connected saved profile is reused; otherwise the only saved profile may be joined. Multiple saved profiles require a selection in LG's network settings, not an arbitrary choice. Screen sharing takes priority: `wifiDirect` must be disconnected before radio changes; a pending radio restoration waits rather than interrupting a share.

## Read-only app contract

`GET /v1/state/network.multi-link` uses the existing loopback helper API and its normal envelope. The topic is also available through `/v1/events`. The status has:

- `schema: 1`, helper `boot`, `generation`, `enabled`, `active`, `reason`;
- `sampledUptimeMs` in Linux `CLOCK_BOOTTIME` milliseconds and `validForMs: 1000` while active;
- `controlInterface`, ordered `candidates` (`interface`, `kind`, `address`, `prefix`, `carrierChanges`, `nominalMbps`, `signalDbm`, byte counters);
- `rates` per interface (`rxMbps`, `txMbps`, `peakRxMbps`), `ipv4Only: true`, `sourceProbeRequired: true`.

The wrapper returned by `/v1/state/{topic}` contains the topic under `value`. Samples are passive interface traffic, including other apps; `nominalMbps` is a link negotiation rate, not measured throughput. A changed source address or carrier generation resets that link's traffic peak. Candidates prefer fast wired links, then observed Wi-Fi throughput; a downloader measures its own useful payload rate before allocating work.

Clients reject missing, disabled, inactive, stale, future-dated or unfamiliar-schema status, and clear old sockets when an address disappears. The normal wired downloader remains the fallback. No app may treat the reported sum of interface counters as usable film capacity. HTTPS verification, redirects, server authentication and object identity remain the client's responsibility. The Marquee integration contract is supplied separately; no Marquee code is part of this change.

## TV evidence and limits

The 2026-11-21 receipt is `docs/evidence/multi-link-2026-11-21.json`. Four-second downloads from the local test host, with sockets bound as uid 5365, measured 94.0 Mbps wired, 147.7 Mbps Wi-Fi and 275.7 Mbps combined. Wi-Fi-only traffic received 81.2 MB on Wi-Fi and transmitted just 5.9 KB there; acknowledgements used Ethernet. With the wire paced at 70 Mbps, combined payload throughput was 245.6 Mbps and concurrent wired HTTP requests had 4.6 ms median, 8.2 ms p95 and 8.8 ms maximum latency. Saturating both links raised p95 to 17.2 ms, so preserving wired headroom is part of the downloader contract.

Read-only requests to Plex on the other VLAN returned on the bound interfaces: 657 IP bytes on Ethernet for the Ethernet address and 657 on Wi-Fi for the Wi-Fi address. These establish the gateway's return-path behaviour; they do not qualify the new helper's route changes or the jailed Marquee downloader.

The TV registers `ax88179_178a` and `cdc_ether` as built-in USB drivers and `r8152` and `cdc_ncm` as modules. Several root hubs report 5000 Mbps, including Bus 010. The adapter's chipset, external-port speed and actual throughput remain unknown until it is connected.

No plugin was installed, no app was restarted and no picture, audio, boot, wake or panel-protection setting was changed. The Wi-Fi association, ARP settings, reverse-path filters and Plex host route remained as found. The plugin UI, real Wi-Fi loss/weakening, DHCP renew, standby/wake, roaming, USB hotplug and whole-app integration remain unaccepted on hardware. Deterministic tests cover selection, address/carrier changes, passive samples and journal recovery; those are not physical lifecycle measurements.

## Side effects and acceptance

A second connected radio uses additional power while awake; its amount is not measured. Standby restores the original radio state rather than keeping Wi-Fi awake for this plugin. Wake-on-LAN, Wake-on-Wi-Fi, LG's network preferences, DNS and the pixel-refresher schedule are not written. LG's network screen can show both connections, but applications without Multi Link continue to use ordinary routing. AirPlay discovery and P2P routing are not altered; coexistence still needs a real sharing test.

After playback ends and a separate TV window is granted:

1. Save addresses, routes, per-interface/global ARP and reverse-path settings, Wi-Fi power/profile, wake settings and the panel-care schedule. Enable the plugin; confirm both links and genuine passive measurements on its themed page, plus the read-only status from the actual Marquee jail.
2. Measure source-bound ranges and wired control latency for no more than 8 seconds each: wire, Wi-Fi, both, and both with wired pacing. Check interface bytes and returned object hashes, including the Plex VLAN.
3. Interrupt only the test client's Wi-Fi leg, then briefly disconnect Wi-Fi through LG with no film playing. Repeat with a weak signal. The client must stop assigning Wi-Fi within 500 ms and dispatch unfinished ranges on Ethernet within 1 second; bytes must remain identical and ordered.
4. Renew Wi-Fi DHCP, including a changed address, and roam to another AP on the saved SSID. Old addresses must disappear, ranges must restart on the wire, and new Wi-Fi work must await a successful server probe. Keep Ethernet up throughout.
5. Disable, standby/wake and stop/restart the helper separately. Compare the saved kernel/radio state after each restoration, test wired wake, and confirm no panel-care schedule change. Test AirPlay/P2P before and during enable/disable; no share may be interrupted.
6. Connect the USB adapter; record VID:PID, bound driver, link rate and the external port's actual bus speed. Test USB alone and hot-unplug during a disposable download. Prefer it alone when it meets peak demand; retain multiple links only when they add useful headroom.
