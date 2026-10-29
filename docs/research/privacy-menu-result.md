# Native privacy controls

Privacy is implemented in `helper/privacy` and exposed by the helper, not a new QML page. `GET /v1/privacy` and `GET /v1/privacy/menu` return the same toolkit-neutral menu model for the Rust shell. The existing Support › Privacy page and its App Permissions and Delete My Personal Information pages remain unchanged. No HTML mock was present or added.

## Switches and confirmation

The model includes availability and reasons, desired state, observed declutter state, the 16 individual LG privacy-setting values and targets, consent documents, and Prime route results. `canAdmin` follows the active owner profile. Links retain the existing `permissions` and `deletedata` destination IDs; merely reading the model never opens either page or deletes anything.

Every write uses two owner-only requests:

1. `POST /v1/privacy/prepare` with `{ "id": "hosts", "enabled": true }` returns the exact change and a random confirmation token valid for 60 seconds.
2. `POST /v1/privacy/apply` repeats that ID and value with `confirmation`. The token is single-use and cannot confirm a different change. Changed LG agreements require a fresh preview.

The shell must present that preview and obtain a deliberate human confirmation before applying it. Preparing a change is not permission to apply it automatically. The HTTP interface remains within the helper's existing access boundary; no new AI tool, remotely exposed listener or infrastructure permission is added.

The four service/preset switches reuse `tools/declutter.sh` and its existing `ads`, `settings`, `uploaders` and `push` groups. The package carries that script and its existing payload; there is no second service killer or settings rewriter. Enabling a group also enables its existing boot persistence. Disabling restores the group's saved state through the same tool. Status reads settings and declutter state without calling the ACR or advertising services and accidentally restarting them.

LG consent changes use the firmware's `/var/palm/license/eulaInfoNetwork.json` mapping, not assumed agreement IDs. Both `eulaStatus` and `eulaInfoNetwork` are written together and read back. Shared advertising agreements can revoke dependent advertising flags; platform terms and hidden channel consent remain protected. A voice flag or voice agreement cannot change as a side effect of another switch. Missing mappings or documents make the corresponding action unavailable.

## Gap table against Glasshouse

| Glasshouse capability | LumaTV implementation / boundary |
| --- | --- |
| ACR, viewing-information and advertising service controls | Existing declutter `ads` group plus the existing privacy-setting preset; consent flags are separately visible. No parallel ACR service controller. |
| Home, screensaver, viewing-history and promotion settings | Existing 16-setting `settings` group, with each observed value and target in the model. Off restores saved values. |
| Uploaders and push promotions | Existing `uploaders` and `push` groups. Local logging and Content Store authentication remain outside these blocks. |
| Ads-tier LG/Alphonso/third-party hosts | Glasshouse-derived exact-name ads list, IPv4 and IPv6, with regional names. The existing country file is used when available; otherwise Glasshouse's fallback regions are used. |
| Full/platform hosts tier | Deliberately excluded: it can block Content Store, ThinQ and LG time services. No new platform-wide deny list. |
| Custom hosts editor | No new editor. Existing foreign entries, including another owner's custom list and Homebrew update blocks, are preserved on both on and off. |
| Hosts persistence and competing bind mounts | A writable managed table is bind-mounted over `/etc/hosts`; the rootfs file is not rewritten. A five-second maintenance check merges a replacement foreign table and reinstates only the explicitly enabled managed block. |
| LG consent flags and agreement documents | Dynamic firmware mapping, exact confirmation preview, coupled flag/document updates and read-back. Protected terms and indirect voice changes are refused. |
| Advertising ID reset, Limit Ad Tracking, ad-cookie reset | Existing Support › Privacy controls remain unchanged; no duplicate actions are introduced in this module. The ad-cookie privacy preset remains available through declutter. |
| Physical microphone / voice-service controls | Not added. Explicit mapped voice-consent requests require confirmation; microphone hardware and voice service shutdown remain outside this menu's service groups. |
| Firmware update blocking | Existing Homebrew/LumaTV update-blocking mechanism remains authoritative; its host entries are preserved, not replaced by a second update blocker. |
| App permissions and personal-data deletion | Existing page destinations are included as links. The new backend performs neither permission grants nor data deletion. |
| Prime Video ad filtering | Built-in Rust implementation described below; not a dependency on a separately installed AmazOff app. |

The host list and structural player transform derive from `rorygallagher2024/glasshouse` and `samoz83/AmazOff` (the latter's `c23c434` revision), with the original AmazOff work by azoffshowy. MIT attribution and license text are included in `payload/privacy/NOTICE` and shipped in the app. No upstream nginx binary, shared trust certificate, private key or frozen Amazon player bundle is shipped.

## Why the installed AmazOff broke

The read-only TV snapshot contains AmazOff 0.2.3 and a frozen `ATVUnfPlayerBundle-1.0.17803.0`, SHA-256 `d5696ab1e4c16069acf6d67cba3a0149ce51a99ffa6cf4ac73e9dfd48eaee612`. Its nginx configuration sends that same old file for `onebox1`, `wave1` and `wave2`, has no `wave3` route, and returns 404 for other paths on the intercepted asset host.

The current `wave1` player is `1.0.25364.0`, SHA-256 `420f6d7257eb1c28930a41721cca138f1fe1099118c2e7ca2e608a3a7ac1630e`; `wave3` is a different player, `1.0.25327.0`, SHA-256 `98ea9211eea18d9e7cf4b6f032d7062fd95b5dbf3126dd017f90acecad61c847`. The live players also request native Sye modules under `/packages/SyeCorePlayerNativeModule-1.0/…` on the same host. A single stale player, a missing route and 404s for required non-player assets are concrete compatibility faults. The newer fork's `wave3`-to-`wave1` alias is not retained either.

The snapshot also shows Starfish/tvweb's `/var/lib/tvweb/adblock_hosts` bind replacing the system hosts table while Prime's jail retained the old IPv4/IPv6 override. The TV has multiple owners of the global hosts mount; a global-only override is not reliable. LumaTV uses only Prime's jail for the Prime redirect and preserves foreign global hosts entries for the separate tracker blocker.

The installed Prime app is 9.1.14, Rust-Ignite-XP / NativeIgnite, native-code version `16.0.0.2026101207-avpk`. The exact request from the last failed playback was not captured, so the evidence does not establish which compatibility fault was encountered first. An empty AmazOff access log does not establish that HTTPS was unused: its logging directive is in the HTTP server, not the TLS server.

## Built-in Amazoff fix

The loopback-only TLS front handles `cloudfront.xp-assets.aiv-cdn.net:443`. It fetches that fixed host through normal certificate-verified HTTPS, resolving through the configured DNS servers directly so the local redirect cannot loop back into itself. No arbitrary upstream URL, client authentication material or public listener is accepted. Four concurrent workers, eight-second socket deadlines and a 32 MiB response limit bound the work.

Only the exact `/family/lg/ATVUnfPlayerBundle-1.0/<route>/js/ATVUnfPlayerBundle.js` path is transformed. Each route fetches its own current player. All structural anchors and expected match counts must pass before any modification is served. Other asset bytes, including native Sye modules, pass through with their HTTP status, media type and redirect/range metadata. No frozen player or cross-route substitution is used. Playback sessions, entitlement checks, media and DRM/licence servers are not rewritten.

Unknown or changed players are served unchanged, with an explicit `unmodified` reason rather than an ad-free claim. The model reports `off`, `unavailable`, `waiting`, `active`, `partial` or `unmodified` protection and source/patched SHA-256 per observed route. A running listener alone is not reported as proof of filtering.

TLS uses the shared `rustls 0.23` / `ring 0.17.14` provider, with default features disabled and `ring`, `std`, `tls12` enabled. The shared target-specific Zig compiler shim builds ring's C/assembly for aarch64 musl. No separate nginx process or ARMv7 binary is needed.

### Trust and old-installation prerequisite

The old AmazOff trust certificate is the shared goproxy CA. Its signing key is public, so retaining it lets another holder of that key impersonate hosts trusted by Prime. Its certificate DER SHA-256 is `4ca2f57b4a9958bf7743c9234aa12e80efaf7dcd8351ad624874666b700ef4f6`; cleanup matches that fingerprint, not just the legacy filename `fe1de658.0`.

LumaTV generates a different CA for each TV, critically name-constrained to `cloudfront.xp-assets.aiv-cdn.net`, with CA path length zero. The CA signing key exists only in memory and is dropped immediately after signing the single leaf certificate; it is never written. Only the leaf key is saved, in a mode-0600 ownership record. The CA is installed under an available OpenSSL subject-hash filename only in `/media/cryptofs/apps/usr/palm/applications/amazon/bin/certs/`. A single IPv4 redirect is maintained in `/var/palm/jail/amazon/etc/hosts`; old duplicate IPv4/IPv6 entries for that name are removed while unrelated aliases are preserved.

The old snapshot also has `"disable-ssl-cert":"1"` in Prime's `default_config.json`. Activation refuses that override instead of relying on disabled TLS checks. Before activation, the human operator must stop the old AmazOff listener, disable its autostart and remove the old TLS-disable override, restoring Prime's normal certificate verification. Any old global asset-host redirect also needs removal by its existing owner. LumaTV does not kill the old process, edit its startup files or modify Prime's configuration to perform that migration.

Enabling reserves port 443 before changing trust or hosts. Off removes the jail redirect before stopping TLS, removes only the recorded matching LumaTV certificate, destroys the saved leaf key and removes the old shared CA from Prime's folder. It never restores goproxy. Externally changed certificates or damaged ownership records produce an error rather than deleting an unverified file. No system trust-store, `/usr`, rootfs or DRM-path writes are part of Amazoff.

Saved switches are resumed by the helper's privacy worker. Normal shutdown removes Prime's redirect before stopping its listener; a failed startup removes a stale redirect rather than leaving Prime pointing at no server. Host-blocker and Prime startup failures do not prevent each other's independent saved choice from being attempted.

## Blocking from early boot

LG starts its ad, recognition and upload pieces before declutter's first boot pass, so the masks and stubs must be down earlier. Boot order on the set, from `docs/HANDOFF.md` (cold boot, measured) and `docs/research/compositor-gate9-result.md` (round 17, booted 21:53:58):

| Boot time | Event |
| --- | --- |
| +10.8 to 13.1 s | `mount-readwrite.service` active; `/var` is writable only from here |
| +12.1 to 14.3 s | `kdump.service` starts; its shim starts `autostart.sh --worker`, the earliest point the mod runs (about +13 s is the floor for a root mod) |
| +16.8 s | `pbs` (`com.webos.service.pbsw`) subscribes at `bootd` |
| +19 to +43 s | `contentminer.service` active, ended by itself |
| +22 s, +30 s | `webos-rbd.target`, `webos-bd.target` pull `user-context-manager`, `nudge`, `uploadd` |
| +33 s | Homebrew Channel's `init.d` runs `52-declutter boot` |
| +70 to +73 s | first declutter pass done: units killed, `ads` group on, stubs bound |

`acr2`, `admanager`, `adoverlay-service` and `sportsalarm` have no record in that window: the syslog keeps about 25 minutes and starts at +12.7 s, and no process accounting exists. `declutter.sh early` existed (masks only) but nothing called it: the kdump shim only starts the worker, and the stubs came with the first pass.

The worker now runs `declutter.sh early` right after `homestub-boot` (Home must be covered by about +14 s), under `timeout 20`. It sits in the worker, not the shim, so an app update needs no `persist on` to take effect. `early`:

- masks the units of every group that is on in `/run/systemd/system.control` and reloads systemd once;
- binds the do-nothing stub over each stubbed program of those groups (`pbs`, `acr2`, `admanager`, `adoverlay-service`, `sportsalarm`, `uploadd`, `sdp-server-notice`), then stops (`--no-block`) any of the units already active and kills any real copy of the programs, so nothing launches as the real program from then on;
- writes `early.json` (start and end uptime to the hundredth, masked, stubbed and already-running names) and starts a watch that waits for `sam`, `webapp-mgr` and the power state;
- leaves settings, apps and list edits to LG's own persistence and the boot pass.

Guard against a boot loop: `early` runs only with declutter's `persist` on, no skip file, no unfinished boot (`.boot-pending`, consumed by `boot`) and no unsettled earlier early pass (`.early-pending`, cleared by the watch once the set answers, within three minutes). Any of these skips that boot once and records why; the `init.d` pass still applies everything later. `on`, `off` and `persist` drop the record, which describes only the groups that were on when it was written.

`declutter.sh status` gains `early`: `hook` (shim, persist flag, persist link and no skip file), `pending`, and `last` (the record). The privacy rows for ads, settings, uploaders and push carry `boot`: `blocked`, `reason`, `verified`, `lastPass` and per-item `blocked` / `startedBefore`; the menu root carries `boot` with `earlyStep`, `pending` and `lastPass`. `blocked` is yes only when the group is on, the hook is installed, the last pass was not skipped and found none of the group's items already running. `verified` is null until a boot has run a pass, true when the pass found nothing running, false when LG had started something first (the items are listed). The settings group persists across a reboot and is blocked whenever it is on. Hosts is reported as not blocked from boot: the helper applies it when it starts, after the early step. A `kill`-only program would be reported as stoppable only after it starts; none is in these four groups.

Not covered: anything LG starts before about +13 s, and a TV with LumaTV's `persist` off, where only the `init.d` pass runs.

### TV check: boot timeline

Preconditions: ads, uploaders and push on in Privacy; `lumatv status` shows `persist` and `early` true; `declutter.sh status` shows `early.hook` true. Note whether the boot is a cold boot or a snapshot resume (`/proc/cmdline`, boot status `snapshot-resume-done`; `docs/HANDOFF.md`, "Boot: the proven mechanism"): on a resume the pass in the log may be the one saved in the image. Then reboot and read, pressing nothing:

1. `declutter.sh status`: `early.last.ran` true, `uptimeSec` below 16.8, `running` empty, `pending` false once the set has settled (within three minutes).
2. `/var/lib/lumatv/declutter/log`: an `early:` line with the uptime, no `killed a running copy` line for this boot, `boot: settled` and `early: settled`.
3. `/var/lib/lumatv/log`: the worker's `declutter early:` line with its `[up N s]` stamp, before `compositor-boot`.
4. `systemctl show contentminer.service -p LoadState,ActiveEnterTimestampMonotonic` reads `masked` and 0; the same for `nudge`, `uploadd` and `user-context-manager`.
5. `bootd.log` and the syslog: no `pbsw` or `contentminer` start of the real programs; launches of `pbs`, `acr2`, `admanager`, `adoverlay-service` and `sportsalarm` exit 0 within about 70 ms.
6. The mount table shows the stub on all seven programs; Home appears and the boot completes as before.
7. Reboot again: the pass runs again. Then `touch /var/lib/lumatv/declutter/skip`, reboot and check `early.last.ran` false with the skip reason and `early.hook` false; remove the file.

A name in `running` in step 1 means LG started it before the hook could act; the row says so, and the timeline shows the margin.

## Host evidence and TV checklist

The host privacy tests use fake LG/declutter operations, temporary files and a loopback TLS server/client. They cover exact single-use confirmations, nonintrusive reads, existing-group reuse, agreement changes and shared voice protection, foreign/update host preservation, restart maintenance, per-TV trust constraints, legacy CA removal, ownership-safe off, old TLS-disable refusal, HTTP framing and per-route filtering/passthrough. The research fixture run compares both current wave outputs byte-for-byte with the upstream transform; `onebox2` is explicitly rejected rather than treated as compatible.

Command: `CARGO_BUILD_JOBS=2 PRIVACY_REFERENCE_DIR=/tmp/privacy-scratch cargo test -p lumatv-privacy` from `helper/`. The scratch-dependent reference tests additionally use the previously captured public certificate and public player bundles; normal deterministic fixtures remain in the crate. HTTP adapter tests cover owner denial, required confirmation and malformed requests without allowing any platform access.

No TV change, app launch, installation or playback check was performed for this implementation. The following checks require a separately approved TV slot, after the old-installation prerequisite:

1. Inspect the packaged API/menu and both existing page destinations; verify non-owner writes are refused and cancel leaves every setting unchanged.
2. With explicit approval, exercise each existing declutter group on/off and compare settings, saved originals and persistence. Confirm ACR status reads do not restart a stopped service.
3. Inspect the hosts table before/on/off, with Homebrew update blocks and the existing Starfish owner present; verify Content Store and ThinQ still work and one managed block survives a foreign rebind.
4. Confirm a mapped LG advertising consent both ways, checking flags and documents; cancel voice/terms changes and never confirm personal-data deletion as part of this check.
5. Enable Amazoff after confirmation; inspect the constrained Prime-only CA, absence of the shared CA, one jail redirect and no system trust/config changes. Launch Prime, observe the actual route and protection result, exercise an ad-bearing title, seeking and a native Sye path, and check ordinary/DRM playback for regressions.
6. Confirm Amazoff off restores direct verified access, removes its certificate and key, does not reinstate goproxy, and leaves unrelated certificates/host entries intact. Check helper restart and standby/wake with a saved choice, then return the TV to standby.
