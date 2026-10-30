# Native privacy controls

## Resume here (2026-10-29 04:30)

**Done, on forge/main** (worktree `/tmp/privacy-wt`, detached at forge/main):

- `helper/privacy` (6e8b0aa, 79b43fd): `/v1/privacy` status, the toolkit-neutral menu, owner-only prepare/apply with a single-use confirmation, LG consents, ads-tier hosts, built-in Amazoff (per-TV Prime-only CA, per-route player filter). Adapter `helper/app/src/privacy.rs`.
- Early boot step (b62e7b5, d4cfb30): `tools/declutter.sh early` binds the stubs first, then masks the units (one reload), then stops and kills what already runs, and writes `early.json` with the boot's clock at each stage and the boot id; `tools/autostart.sh --worker` starts it first, in parallel with the Home stub and everything else; `status.early` carries `hook`, `pending`, `thisBoot`, `last`. `helper/privacy/src/boot.rs` turns that into `boot{blocked,reason,verified,lastPass,items}` on the ads, settings, uploaders and push rows; a record from an earlier boot proves nothing. Tests: `tests/test_declutter.py` (`Early`), `helper/privacy/tests/controller.rs` (`each_item_says_whether_it_is_blocked_from_boot_and_why`).

**Task (Main, 02:51 and 03:19):** LG's ACR, ad and telemetry pieces effectively removed at the source on the TV, not after about 14 s. No router or DNS block list (dropped by Nitin); names only as a TV-side verification aid. Two parts: the earliest possible hook, and stubs as close to removed as possible. No TV access, no spawning, never write `/usr` or the rootfs, everything behind the existing `persist` gate.

**Result:** the section "Blocking from early boot" below. The hook cannot move earlier than the kdump script at about +13.4 s in any way that is allowed and stable, and it does not need to: on the stock boot every ad and recognition program starts after it, and the network comes up 4 s after it, 10 s before the first LG server call. Nothing in the repo's evidence names a data directory or a timer of these services, so none is hidden or masked.

**Next, for whoever continues** (nothing here is built, each needs a TV slot):

1. Read the scripts of `bam`, `rec-info-adapter`, `rec-listmgr` and `nn-proxy` (`/etc/systemd/system/scripts/*.sh`, read only) to see whether one calls a bare command first; only then is a hook before kdump possible, and it gains 1.6 to 2.7 s.
2. Run the boot timeline check below on a `reboot` and on a `snapshot-boot-manager --remove` boot; it settles whether kdump's script runs again on a resumed boot.
3. With the groups off for one slot, list what LG's ad and recognition programs write (`find / -xdev -newer MARK -type f` after they ran); only named directories may be hidden under an empty tmpfs (reversible, nothing deleted).
4. Optional experiment, outside the `persist` gate: bind an empty file over a service definition in `/usr/share/dbus-1/{services,system-services}` and `ls-control scan-services`, to see whether `ls-hubd` forgets the service.

**Decisions taken:** the router list is dropped; nothing is deleted from `/var`; the early step stays behind `persist` and the skip file; no write to `/usr`, `/etc`, the rootfs, `/mnt`, the kernel command line or any block device; no LD_PRELOAD or library-path hook through the env files.


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

The old snapshot also has `"disable-ssl-cert":"1"` in Prime's `default_config.json`. The switch retires an earlier AmazOff install itself, so the owner has nothing to do first (`helper/privacy/src/legacy.rs`, `prime::retire_old`): Prime Video is closed (`applicationManager closeByAppId amazon`), every program whose command line, program or folder names `amazoff` is stopped (the monitor with the nginx it restarts, SIGTERM then SIGKILL, read again for anything started in between), a start-up script in `/var/lib/webosbrew/init.d` that names it is moved to `privacy/retired/` in LumaTV's state (kept, not deleted), and the one `disable-ssl-cert` key is removed from Prime's `default_config.json` (every other key stays). Activation still refuses if that key is there afterwards, rather than rely on disabled TLS checks. Any old global asset-host redirect is left to its existing owner. Off closes Prime Video before the redirect and certificate are removed. A TV where Prime Video has not run yet (no jail hosts file) or is not installed says so in the row (`reason`), and the row cannot be pressed. Not exercised on the TV: where the old app's start-up script lives is taken from the Homebrew Channel's start-up folder; a different place would leave the script, and the old app would be stopped but start again at the next boot.

Enabling reserves port 443 before changing trust or hosts. Off removes the jail redirect before stopping TLS, removes only the recorded matching LumaTV certificate, destroys the saved leaf key and removes the old shared CA from Prime's folder. It never restores goproxy. Externally changed certificates or damaged ownership records produce an error rather than deleting an unverified file. No system trust-store, `/usr`, rootfs or DRM-path writes are part of Amazoff.

Saved switches are resumed by the helper's privacy worker. Normal shutdown removes Prime's redirect before stopping its listener; a failed startup removes a stale redirect rather than leaving Prime pointing at no server. Host-blocker and Prime startup failures do not prevent each other's independent saved choice from being attempted.

## Blocking from early boot

### What the stock boot does

Read from the stock boot of 2026-10-25, before declutter was on: the syslog of that boot (`/tmp/lt/dc/live/boot7.log`, which starts at +12.5 s because `/var` is not writable earlier), `ls-hubd`'s launch lines (`lnch.txt`) and the unit census (`units.json`, activation times and ordering), with `docs/HANDOFF.md` and `docs/research/compositor-gate9-result.md` (round 17). Times are the boot's clock.

| Boot time | Event |
| --- | --- |
| +1.3 s | `crashd` and `remotelogger` units start |
| +10.7 s | `mount-readwrite.service` active: `/var` is the writable partition from here |
| +10.8 s | `faultmanager` unit |
| +11.8 s | `com.webos.service.pushclient` unit |
| +13.0 s | `sdx` is running (launched on demand; kept on purpose, the store needs it) |
| +13.4 s | `kdump.service` runs its script: the early hook |
| +14.7 s | `snapshot-boot-resume.service`; `INSTANTBOOT_RESUME` events at +15.8 s |
| +16.9 s | `pbs` launched, requested by the HDMI 2 app |
| +17.3 s | DHCP starts; the wired link is up at +17.5 s |
| +17.8 s | `service-logger` unit |
| +18.7 s | `contentminer` and `rdxd` units |
| +23.8 s | first LG server call: `sdx` to `us.nextlgsdp.com` |
| +25.7 s | push client to `aic.tv.wiselg.com` |
| +36.6 s | `uploadd`, launched by `faultmanager` |
| +41.5, 43.0, 43.5 s | `user-context-manager` unit; `sportsalarm` and `sdp-server-notice`, launched by the activity manager |
| +45.5 s | `adoverlay-service`, `admanager`, `acr2`, launched by the activity manager |
| +46.0 s | `admanager` to `us.info.lgsmartad.com` |
| +46.9, 49.3 s | `iot-client`; `nudge` unit |
| +73 s | the first pass of declutter's init.d hook (round 17), before this change the first thing that stopped any of it |

On the stock boot `acr2`, `admanager`, `adoverlay-service` and `sportsalarm` were launched from +43 to +46 s. Round 17's boot had only declutter's init.d pass (+73 s), so the same launches very probably ran there for about 30 s [INFERENCE: the launch times of the stock boot]; the round 17 note that this window was unprovable stands as unproven for that boot. The early step precedes each of them by about 30 s.

### Why the hook at +13.4 s is early enough

- Every program of the ads, uploaders and push groups except the push client, `faultmanager`, `crashd`, `remotelogger` and `sdx` starts after the hook: `pbs` by 3.5 s, `service-logger` and `contentminer` by 4 to 5 s, the rest by 30 s or more.
- The network comes up 4 s after the hook. Nothing contacted an LG server before +23.8 s, so the programs that start before the hook are stopped at +14 s, before they could send anything. `sdx` stays: it carries the store's authentication and is not an ad or recognition program.
- The pass is ordered by how soon LG can start each thing: the stub over each on-demand program first (one mount each), then the masks (one systemd reload), then the stop of what runs and the kill of the programs. `early.json` records the boot's clock at each stage (`startSec`, `stubbedSec`, `maskedSec`, `uptimeSec`), so the margin to `pbs` at +16.9 s is read, not assumed.

### The earliest hook

- Before +10.7 s nothing the mod can write is readable. `/var` shows only the firmware overlay, LG's env files planted for `ls-hubd`, `configd`, `memchute`, `tvpowerd`, `bootd` and `sam` were inert, and p58's root was not read (`docs/HANDOFF.md`, "Boot: the proven mechanism" and "Tried and dead"). `/usr`, `/etc`, `/lib` and `/mnt` are read-only overlays on the BSP squashfs (`mountinfo`), `/run` is a tmpfs that only running code fills, and a unit drop-in needs one of them. The kernel command line, the initramfs, the rootfs and the hibernation image are the protected partitions; nothing here touches them.
- After +10.7 s the proven mechanism is a unit's `EnvironmentFile` under `/var/systemd/system/env/`: a `PATH` that puts a shim in front of a bare command that the unit's script calls. `kdump.service` (+13.4 s) is what runs. The census names earlier script-based units with such a file, ordered after `mount-readwrite`: `rec-info-adapter` (+10.78 s) and `rec-listmgr` (+10.79 s), which exit at once, `bam` (+11.79 s) and `nn-proxy` (+11.85 s). Their scripts were never read, so whether one calls a bare command first is unknown. The best case moves the hook 1.6 to 2.7 s earlier: ahead of the push client, ahead of nothing else that matters, and onto a script of LG's with no reason to stay as it is across firmware. Not built; reading the scripts is the first TV check.
- Library hooks (`LD_PRELOAD`, `LD_LIBRARY_PATH`) through the same files are not used.

Before the hook run `faultmanager`, `crashd`, `remotelogger`, the push client and `sdx`. The units at +1.3 s start before `/var` is readable, and `faultmanager` (+10.76 s) 20 ms before the earliest script-based unit with an env file (+10.78 s), so no hook can pre-empt them; they are stopped about 14 s in, with the network 4 s away.

### Snapshot boot

`/proc/cmdline` carries `snapshot resume=`, and the stock log shows `snapshot-boot-resume.service` at +14.7 s and `INSTANTBOOT_RESUME` at +15.8 s. A resumed boot restores the processes of the image (`docs/HANDOFF.md`); whether kdump's script runs again on such a boot is not settled by the repo's evidence. The worker's own log line and `early.json` settle it, because the record carries the boot id (`/proc/sys/kernel/random/boot_id`) and `status` says whether it is this boot's (`thisBoot`). A pass of an earlier boot is never shown as this boot's: the row then reads "has not run since this boot started" and is not blocked.

### What the early step leaves and removes

Removed from the boot: the masked units never start or are stopped (`contentminer`, `rdxd`, `service-logger`, `crashd`, `remotelogger`, `faultmanager`, `uploadd`, the push client, `nudge`, `user-context-manager`, `iot-client`); the stubbed programs (`pbs`, `acr2`, `admanager`, `adoverlay-service`, `sportsalarm`, `uploadd`, `sdp-server-notice`, the IoT client's executable) are never launched as the real program, and a copy that already runs is killed.

What stays, and why: each call to a stubbed service still makes `ls-hubd` start `run-service` and the do-nothing script, about 70 ms and a log line (round 17: `pbs` 10, `sportsalarm` 12 launches in 8 minutes; none for `acr2`, `admanager`, `adoverlay-service`). The entries in `ls-hubd`'s service map stay; removing them needs a rescan that is unverified (next step 4). No privacy unit has a timer, socket or path unit (`TriggeredBy` is empty for every one in `units.json`; the set's only timers are `certupdmgr` and `systemd-tmpfiles-clean`), so there is nothing further to mask. No data directory of these services is named by any capture, so none is hidden; the check that finds them is next step 3.

### Guards, record and status

The step runs only with declutter's `persist` on, no skip file, no unfinished boot (`.boot-pending`, consumed by `boot`) and no unsettled earlier pass (`.early-pending`, cleared by a watch once `sam`, `webapp-mgr` and the power state answer, within three minutes). Any of these skips that boot once and records why; the `init.d` pass still applies everything later. `on`, `off` and `persist` drop the record, which describes only the groups that were on when it was written.

`declutter.sh status` carries `early`: `hook` (shim, persist flag, persist link, no skip file), `pending`, `thisBoot` and `last` (the record). The privacy rows for ads, settings, uploaders and push carry `boot`: `blocked`, `reason`, `verified`, `lastPass` and per-item `blocked` and `startedBefore`; the menu root carries `boot` with `earlyStep`, `pending`, `thisBoot` and `lastPass`. `blocked` is yes only when the group is on, the hook is installed, a pass has run since this boot started and did not skip, and it found none of the group's items already running. `verified` is null until such a pass, true when it found nothing running, false when LG had started something first (the items are listed). The settings group persists across a reboot and is blocked whenever it is on. Hosts is reported as not blocked from boot: the helper applies it when it starts, after the early step.

### Verification names (TV side only)

The hosts the stock boot log names, by program. Not a block list and not for the router: after the early step none of the stubbed or masked programs runs, so no line of the boot's syslog may show them as the source of one of these names, and `/etc/hosts` mapping them (the ads tier) is only a second line. `us.nextlgsdp.com` and `us.ibs.nextlgsdp.com` (`sdx`, the store) must keep resolving.

| Program | Names in the stock boot log |
| --- | --- |
| `admanager` | `us.info.lgsmartad.com` |
| `pbs` | `aic-ngfts.tv.wiselg.com`, `aic.cdplauncher.lgtvcommon.com`, `aic.lgchhomeapp.lgtvcommon.com`, `app-lgwebos.pluto.tv`, `aic-img-ngfts.lge.com`, `images.pluto.tv` |
| push client | `aic.tv.wiselg.com` |
| `iot-client`, rule engine | `connect-client.lgthinq.com`, `aic.api.lgtviot.com`, `aic-connect-client.lgthinq.com`, `aic-common.lgthinq.com`, `aic-mclip.lgthinq.com`, an `iot.us-west-2.amazonaws.com` endpoint |
| `sdx` (kept) | `us.nextlgsdp.com`, `ngfts.tv.wiselg.com`, `us.ibs.nextlgsdp.com` |
| `accountmanager` (kept) | `us.lgeapi.com`, `us.emp.lgsmartplatform.com`, `aic-gfts.tv.wiselg.com` |

### TV check: boot timeline

Preconditions: ads, uploaders and push on in Privacy; `lumatv status` shows `persist` and `early` true; `declutter.sh status` shows `early.hook` true. Record whether the boot is a `reboot` or a boot after `snapshot-boot-manager --remove` (`/sys/power/mode`: `resume` or `making`). Read first, no key pressed: `/etc/systemd/system/scripts/{bam,rec-info-adapter,rec-listmgr,nn-proxy,kdump}.sh` (first command of each, next step 1). Then reboot and read:

1. `declutter.sh status`: `early.thisBoot` true, `early.last.ran` true, `startSec` about 13 to 14, `stubbedSec` below 16.9 (the margin to `pbs`), `uptimeSec` below 17.5 (the network), `running` listing at most the push client, `faultmanager`, `crashd`, `remotelogger`; `pending` false once the set has settled.
2. `/var/lib/lumatv/declutter/log`: an `early:` line with the three clock values, `boot: settled` and `early: settled`; `killed a running copy` only for a program named in `running`.
3. `/var/lib/lumatv/log`: the worker's `declutter early:` line with its `[up N s]` stamp, before `homestub-boot`.
4. `systemctl show contentminer.service -p LoadState,ActiveEnterTimestampMonotonic` reads `masked` and 0; the same for `rdxd`, `nudge`, `uploadd`, `user-context-manager`, `iot-client`.
5. `/var/log/messages` of this boot: no `pbs`, `acr2`, `admanager`, `adoverlay-service`, `sportsalarm`, `contentminer`, `uploadd`, `nudge` or `iot-client` log line other than the stub's exit; none of the names in the table above attributed to them. Launch lines of `pbs`, `acr2`, `admanager`, `adoverlay-service`, `sportsalarm` end within about 70 ms.
6. The mount table shows the stub over all eight programs; Home appears and the boot completes as before.
7. Reboot again: the pass runs again and `bootId` changes. Then `touch /var/lib/lumatv/declutter/skip`, reboot and check `early.last.ran` false with the skip reason, `early.hook` false and the privacy rows not blocked; remove the file.

A name in `running` in step 1 means LG started it before the hook could act; the row says so, and `startSec` shows the margin.

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

## The update block and the IoT client at boot (cold boot of 2026-10-29)

On that boot (`docs/research/compositor/wake-2026-10-29/`, `/sys/power/mode` `resume`) the early pass ran at 13.7 s, the network came up at 22.7 s, the update service learned of 33.31.69 (1687 MB) at 29.7 s and posted its alert at 31.8 s, and the Homebrew Channel's `startup.sh` (the `/etc/hosts` bind of `snu.lge.com`, `su.lge.com`, `su-ssl.lge.com`, `su-dev.lge.com`) came at about 27 s. `iot-client` was started by `ls-hubd` at 36.8 s as a Luna service for `com.webos.service.homeconnect`; its unit is masked, which a Luna launch does not read.

**Update block.** `declutter.sh updateblock early`, the first thing autostart's worker starts (in parallel, before the group step), binds a copy of the table in force plus the four names over `/etc/hosts` when the channel's flag `/var/luna/preferences/webosbrew_block_updates` exists, LumaTV's persist is on and there is no skip file. The flag is the whole of the channel's configuration; no other update-service setting is read or written. The copy is `/tmp/lumatv-update-hosts` (tmpfs; `/tmp/hosts` is the channel's own file); the channel's bind at 27 s copies it and appends the names again, so the mapping never lapses. It does not depend on a declutter group or on declutter's persist link. Boot order relied on: the worker runs from the kdump shim at about 13.4 s on a resumed boot as well (`startSec` 13.73 on this one), `/var` is writable from 10.7 s, and the network is 9 s later; a table that already maps the four names (the channel's) is left alone and an empty or unreadable table is never bound.

The updater itself, its alert and any staged image are not touched: nothing is stubbed, killed or dismissed. With the names on loopback before the network exists, the check cannot reach LG, so no image is learned and no alert is raised. For two minutes a watch lays the names again if another tool's table replaced the mapping (the check at network arrival is the one that mattered; this covers a later rebind), ends if the channel's switch is turned off, and logs once at about 40 s the image size the update service reports (`getStatus`, read only). `declutter.sh updateblock status` and `declutter.sh status` (`updateBlock`: `flag`, `mapped`, `ours`, `hook`, `thisBoot`, `last`) say what the pass did; `declutter.sh updateblock off` takes the layer off when it is the table showing, and leaves the channel's when that is on top (then it goes at the next boot). With persist off the worker does not start and nothing is laid; the channel's own block stays as it was.

**IoT client.** `/usr/palm/services/com.webos.service.iotclient/iot-client` is an item of the `push` group (`luna`, `stub`), so it is stubbed in the early pass like `pbs` and `acr2`, appears among the push row's `boot.items` with its `blocked` and `startedBefore`, and `running` in the record names it when `ls-hubd` had it first. [INFERENCE: the executable path is the one in `ls-hubd`'s `LSHUB_SRV_LNCH` line (`run-service /usr/palm/services/com.webos.service.iotclient/iot-client`); that it is a regular file at that path on the set, and not a link whose target `/proc/PID/exe` shows, is not recorded. A missing file is logged by the pass ("no such file, not stubbed").]

### TV check for the next round

On a cold `reboot` and on a `/sys/power/mode` `resume` boot, with persist on, the channel's block on and the push group on: `declutter.sh status` has `updateBlock.thisBoot` true, `last.state` `bound`, `last.startSec` about 13 to 14 (before the network's 22.7 s); `/var/lib/lumatv/declutter/log` has the `updateblock:` line with its uptime and the later "knows an image of 0 bytes"; `/var/lib/lumatv/log` has `update block early:`; no "update now?" notification in `/var/log/messages` of the boot; `com.webos.service.update/getStatus` `nsuImageSize` 0. `early.last.stubbed` lists the IoT client's path, `/var/log/messages` has no `iot-client` line other than the stub's exit, no `LSHUB_SRV_LNCH` of it that outlives about 70 ms, and no connection to `aic.api.lgtviot.com`. Then `touch /var/lib/lumatv/declutter/skip` and reboot: `updateBlock.last.why` is the skip reason and nothing is laid (the channel's block still comes at 27 s); remove the file.

## LG Remote Service

LG's remote support (`com.webos.app.remoteservice`: with a code read out to LG, its staff see and control the TV) was reachable from LG's hidden support menu (`com.webos.app.tvhotkey`, `mute-hidden-action`), which three gear taps used to open and which no longer opens. It is a remote-access feature, so it is human-only (Principle 0):

- Blocked by default. The compositor drops any luna launch of the app (`launch`, `open`, `launchDefaultApp` naming it, from the shell, a key rule or its own code) unless `<data>/privacy/remoteservice.allowed` exists (`system_ui.rs`, `remote_service_refused`).
- The file is written only by the helper's owner-only, confirmed change (`POST /v1/privacy/prepare` then `apply`, item `remoteservice`, shown as Settings > Privacy > LG may collect > Allow LG Remote Service, a status row, Blocked until allowed; OK on Blocked asks once in the danger style, OK on Allowed blocks again with no question). A non-owner is refused with 403 before anything else, the token is single use and expires in 60 s.
- No assistant route: no AI tool, MCP tool or helper capability is added or extended for it, and LumaTV's Settings has no row that opens the app. Anything that would give an assistant a way to write that file or call the switch breaks this rule.
- LG's own screens can still launch the app through LG's application manager, which does not pass the compositor; the switch governs LumaTV's side only. Turning it off removes the file again.
