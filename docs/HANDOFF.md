# LumaGlass — engineering handoff

Everything a person or model needs to pick this project up cold: how the mod
works, what was measured on the set, which approaches are dead and why, and
how to measure again. Facts here were observed on one set unless marked
[INFERENCE]. Credentials and the set's address are deliberately absent; they
live with the owner.

## Bounded remote logging

`base.logging` owns `enabled` (false), `host` (empty), and `port` (514). The About > Logging page edits those schema-backed values. Addresses are bare IPs or DNS names, not URLs or host:port pairs. Invalid updates leave the stored configuration unchanged. `GET /v1/logging` uses the standard topic envelope; `active` means the UDP worker is configured, never that a collector received a packet.

`GET /v1/logging` reports remote-forwarding status, not application capture consent. `base.logging.enabled` gates UDP output only; bounded local collection is independent. No supported external-app capture or opt-in API exists. Optional application capture requires its own explicit consent; Stats or overlay visibility is unrelated.

Ordinary producers send nonblocking Unix datagrams to `/tmp/lumatv-log.sock`: one `<PRI>tag: message` record, at most 4096 bytes. The helper owns `/tmp/lumatv-logs/current.log` and `previous.log`, each at most 512 KiB including newlines. The aggregate retained ordinary log payload is at most 1 MiB. The collector refuses non-RAM filesystems, oversized existing files, symlink files, and a second owner; there is no flash fallback. Producers drop records rather than block when unavailable or full. Socket buffers, record buffers, native readers and bounded Details tails are separately fixed overhead, not part of the retained-payload limit. Legacy user log files are not deleted or maintained as aliases.
The two logging workers reserve 128 KiB stacks each. Every helper-owned Unix and UDP logging socket requests 8 KiB send and receive buffers (Linux doubles these values for bookkeeping); no application-side record queue grows with collector downtime.
The collector accepts only a private directory owned by its effective uid, and descriptor-verified RAM regular files with mode0600, one hard link, and a collector ownership header. Directory-relative opens/rotation are pinned to the verified directory descriptor. Each retained file starts with LUMATV-LOG-V1 and a newline, counted inside512KiB and ignored by exact-tag readers; unrelated existing files are refused without adoption or truncation. The private128-byte owner.lock also records the bound socket identity. Startup removes only a matching, inactive stale collector socket; an unrelated or active socket is preserved. The socket parent must be trusted private/sticky RAM storage, and shutdown leaves any inode/type/owner replacement untouched.



The passive native source tails only LG’s existing RAM-backed `/var/log/messages`, starting at the end and handling truncation and replacement. It does not replay compressed rotations or override LG’s filtering. Critical kernel records (severity 0–4) use an independent, nonblocking `/dev/kmsg` descriptor positioned at the end; `/proc/kmsg` and the global kernel log cursor are never consumed. Native source availability, errors, gaps and dropped submissions appear under `value.sources`; unavailable sources are not claimed as covered. Kernel records can also appear in LG’s stdlog, so duplicates are possible. Neither LG daemon nor its whitelist/configuration is replaced or reloaded.
The independent critical-kernel source is outside LG’s accepted stdlog filter; its only selection is severity 0–4. It leaves LG’s readers and policy untouched rather than claiming LG approved those additional records.


The 1 MiB limit does not cover LG’s pre-existing native logs, the separately retained 768 KiB compositor trace, or the existing eight bounded crash diagnostic archives. Crash Details continues reading failure snapshots, not the current log window. These intentional diagnostics are not ordinary logger fallback storage.

The gated installer must stage the helper, matching `tools/lumatv`, and matching producer components together. The canonical apply, boot and `helper start` paths remove only obsolete `helper.json` `log.maxBytes` atomically before helper execution, preserving `log.level`, every other configuration byte, permissions and ownership. All product `helper.json` writers take an exclusive flock on the stable containing data-directory inode before opening/replacing the file; currently the canonical migration is the only production writer, while the helper itself only reads this process configuration and keeps runtime settings elsewhere. Invalid JSON or a detected external content/metadata change aborts migration without replacing the file. This serializes cooperating product writers; it does not claim compare-and-swap protection against an arbitrary external root editor in the final check/replace window. A helper-only binary replacement bypasses this mandatory migration. The product never ships a home collector address as a default. An owner-specific initial enabled/address/port change is a separate gated action. No native service configuration, firewall, port, DNS or collector changes are required.

The Netflix-button gate decodes `app:<id>` and `nothing` choices and checks mapped/foreground results, not journal phrasing. Other configured action types are reported as unsupported without invoking the button.


## Current maintenance state (2026-10-31 EDT)

The proposals reuse existing Lucent CSS, Manrope and audio assets. Seven 1920×1080 scale-1 renders were inspected in Cody’s visible shared Chromium: `shots/review-toast-dark.png`, `shots/review-toast-light.png`, `shots/review-audio-atmos-dark.png`, `shots/review-audio-dts-dark.png`, `shots/review-audio-dtsx-bright.png`, `shots/review-audio-dolby-light.png` and `shots/review-audio-stereo-dark.png`. Toast proposal is approximately 320×65 px with 24 px text and one bare check, versus the old 520–760×96 px and 27 px title with a 64 px tile. Audio uses original PNG glyphs left of the number and track in one floating group at the existing bottom position. Light/dark and 200% text showed no overlap or overflow; fonts loaded, browser errors were absent and conservative ink-contrast bounds exceeded 7. Evidence: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/review-render-proof.json`. These are illustrations, not TV acceptance or behavior changes.


Both keyboard tasks are complete. The compact-toast and audio proposals were approved and implemented in source, with host-native evidence in `/data/home/tmp/lumatv-ui-proof.68u1vel0`; they are not installed or accepted on the TV. The toast defaults to bottom centre with padding and uses existing Settings position controls. Transient LG feed 28737 reaches the toast only, not NoteLog subscription, history or unread count; genuine TV, plugin and AI entries remain. The newest toast replaces the current one and resets its deadline; older toasts do not resurface. Original audio PNG glyphs appear left of the number and bar, with no invented glyph for PCM, unknown formats or mute.

R4 is frozen and locally reviewed/exercised at /data/home/tmp/lumatv-f5-renderer-packet-r4. Entry SHA3e5fd3953c3c67aae80d849b0ab6187f7c3df9300d1336104b0d86614a41cd5b; scope-receipt-r4.json SHAb006de5f91f6f88dfa5cfc06af322ffee5141ffdbfdd4296c6f0e10b16537c37. execute-command.txt SHAc4596739ce95df9eca054f4df94ac85c0f9152536b09a7aa7b461d78be977735 hashes then compiles the same entry bytes. Entry/receipt are regular single-link0400 files. Direct Fix it offline authorized preparation only, not installation or transferred permission. No TV action or request is pending here.

Coordination03:20 EST November1: Marquee lease is RELEASED after reported stable0.10.0 runtimefce9888a deployment,54 matched files and native smoke. Final observed state was Home, Stats hidden, video_planes=0. No fresh refresh/divider read followed the earlier bounded capture;120/divider1 and8.3ms output remain historical evidence only. Zero video planes are not empty-media proof or interruption consent. Preserve subtitle sidecar ABI6 with epoch-tagged readiness publication in later alignment.

The UI-feature artifact /data/home/tmp/lumatv-foreground-pacing-20261031/lumatv-compositor, SHA256904c8e93a985aa25feb07afac8fdde2f0796b67317ac431ef86277e4edbcb6c0, is SUPERSEDED/NOT READY and ineligible for installation. It is not an authorized substitution for pinned legacy f5/R4. Toast/audio artifacts remain separate and uninstalled. Native callbacks and LG video-family ownership remain unchanged: divider1 follows100/119.88/120, not independent120-over100. No panel, Home-global120 or optical-seamlessness claim follows.

The existing final f5 replacement already corrects the old904 review findings. Independent narrow review of foreground-pacing-source-20261031-final.tgz confirms webos_shell.rs:114–130 recomputes app/property/window-type changes and :80–86,100–110 recomputes changed nongroup card state, including minimized→Maximized/Default. Archived tests/refresh.rs:229–260 explicitly covers these transitions. Existing legacy-tests.log records9 refresh,11 callback and2 control/capture tests passing; cross-build-legacy.log records a successful release build. These are historical records, not new runs.

Fresh SHA256SUMS-legacy verification passes for existing lumatv-compositor-legacy f5c864ae1e869da3e2f60af14faca77ef51664254c8043652dff84a2fbf3463a, the same hash pinned at frozen R4 scope-receipt-r4.json:18–20. No second implementation, rebuild, artifact substitution or packet edit is needed for these findings. Archived source/build records are an association, not independent reproducible source-to-binary proof. R4 remains separately unapproved and not hardware-accepted. Native Qt host gating supports no production shell change; it does not establish TV GPU-slot causality or video-wallpaper/all-config behavior.

Native120Pacing produced no analysis: its latest wake failed with HTTP400 before an answer, so no slow analysis is running. It remains the sole authorized deep-task; no replacement worker or role change was made. The Marquee caller reports no pending HomeLabber request; that does not transfer unused R3 permission or authorize R4/retained-HDMI interruption. Any future TV access requires fresh joint coordination and exact authorization. Current RESUME/HANDOFF status supersedes archived coordination prose; preserved source/binary archives and frozen packets remain unchanged.

The current source menu-pacing map and graphics evidence index are recorded in `docs/research/framerate-result.md`, Current foreground pacing policy. Ordinary overlays inherit the underlying eligible card, so foreground Marquee gets nominal120 while Home/system overlays retain startup base60; selected Qt animations are frame-driven but not universally120. The existing local refresh protocol smoke passed all9 cases. This does not establish current TV mode, unique120 pictures, global Home performance or direct-scanout device acceptance.

Local material/theme extension is separate from frozen R4 and all old artifacts: preserve Clear/Frosted, derive Liquid from the actual native Marquee topbar and Glazed from LumaShow native card/sheet rendering, add independent Strength beside Depth, and add20 light/20 dark palettes. The native Themes gallery belongs under Home & look. Nitin selected immediate focus preview, OK confirmation and Back restoration of the last confirmed theme. No Echo Show screenshot/device access is needed: Nitin replaced that request with a source audit. Shared Lucent adoption remains conditional on Nitin approving actual LumaTV renders; no shared-language edit is authorized yet.

The gallery uses the effective native text scale for its column count, not theme JSON alone. Theme thumbnails composite each palette’s translucent roles over its own canvas and sheet. Catalog text roles, including Ink3 headings, target7:1 on those resolved surfaces; arbitrary video still needs the existing reading fallback. Protocol14 adds outward edge-lens and tint-gradient fields, so the client and renderer must be built as a matching pair; no new UI binary substitutes for a frozen protocol/payload packet. Native Mesa captures are local rendering evidence, not TV timing or hardware acceptance.

Native startup uses /usr/bin/surface-manager -platform eglfs_webos with captured inherited LD settings. R4 requires both native-alias and installed-supervisor executable identity, the guarded private loader, all-LD/duplicate checks and a fresh stable ARM ELF32 shared-object header/hash proof for libsystrim. Pinned ELF64 loader was exercised locally in existing qemu: explicit private library path overrides LD_LIBRARY_PATH and an ELF32 preload is ignored. Actual TV preload/proc evidence remains unobserved; header/class compatibility is not vendor provenance, full supervisor trust or complete loader isolation.

The exact saved Home media profile has four retained HDMI2 rows and six occupied input/audio resources; it is not empty or proven idle. R4 rejects app/foreground/unknown/malformed state and full-record drift before disable; postrestart IDs may change within the same permitted kind. Future scope must explicitly allow possible background input/audio interruption and a fresh exclusive lease; snapshot checks are not atomic stream exclusion. Actual inspect and capture-based checks passed with sockets forbidden. Independent startup/media reviews found no remaining narrow blocker after full-header/inode corrections; guarded settlement/rollback transitions were exercised with explicit local stand-ins, not hardware. Renderer/R8/payloads are unchanged, and f5 remains NOT installed.

Picture-mode selection sends settings.pictureMode through LG setSystemSettings without a dimension selector, matching the recorded r19w immediate-change path. Dimensioned writes remain appropriate for per-mode controls, not mode selection. The generic compositor/native Luna bridge forwards that payload unchanged; it does not require or inject a dimension. The shared Sio writer discards stale context and refreshes remembered values/descriptions after acknowledgement or the bounded two-second guard. Dependent controls require fresh native mode, input and property availability.

Quick Settings and Picture use LG's visible/active choices; the legacy fallback is restricted to confirmed SDR with an absent descriptor. Popup selections retain their displayed native value through descriptor reordering. The shared borrowed caption formatter retains existing names and LG's proven dolbyHdrCinemaBright → Cinema Home; conditional Dolby captions are not guessed. Source is verified on the actual Rust/Mesa host, not installed or included in frozen R4.

The current saver source is rustui/ui/src/system/saver, with system/layer.rs and shell.rs owning presentation/input dispatch. LG option.screenSaverTimer remains the only start deadline. Native Screen Saver state is approved by LG's idle/media policy; a tvpower vote question alone is not approval. Home-owned negative voting stays Home-only; an app saver must wait for LG-confirmed Screen Saver so native playback/game vetoes remain authoritative.

Native UI checks reuse tests/rust_settings_host.py and tests/rust_home_host.py with the real lumatv-ui binary and Mesa. Helper screen-state replay proves UI behavior, not elapsed hardware timeout or individual app vetoes. The current ARM compile commands are compositor/tools/build-tv.sh lines26 and33; the full script also rebuilds deploy staging, while rustui/tools/build-tv.sh targets the older lumatv-rustui client. New UI/platform changes remain separate from the immutable R4 packet.

Current UI corrections pass 1094 affected UI tests (two ignored; 162 unchanged Home tests filtered), all 406 compositor library tests, and both production ARM builds. Native-host evidence covers SDR/Dolby arrows, explicit mode selection, confirmed readback on reopening and app-native saver key/pointer wake. Real mapped-app regressions cover wake isolation and pre-existing held-input ownership. Proof is retained at /data/home/tmp/lumatv-ui-proof.68u1vel0; private native-ui-fixes source/ARM archives supplement the resume handoff, not an installer or publication. Actual-TV movie mode switching, elapsed idle timeout and playback/game veto acceptance remain outstanding.

Parent local trace report: /tmp/marquee-readonly-20261101-{sched.gz,ring.json,capture.log}, monotonic18658.566782..18664.569939,374216 events,0 overrun; 718 draws/717 flips and two16.7ms intervals amid8.3ms output. late/lost/missed counters were0, so those counters do not establish zero stutter. Main-thread runnable/blocked timing is correlation, not sole cause; overlapping Mali slot events invalidate naive summed GPU attribution. No app/callback/governor/panel change follows from this sample.

The preserved source has no previously successful standalone renderer-only host CLI. Proven boundary is renderer_cutover(ctx) with the R8 transport: /tmp/lumatv-historical-stage.TyUJ9M4D/renderer-fragment.py:89-116; wake-maintenance-evidence/events.jsonl:23-54, attempt-923dafccaf6845f8927a9f5b434d39d2. Three commands exited0; two health records and renderer-cutover=healthy identify installed compositor6fe8bae..., not f5c864. Historical staging was deleted at event55. wake-maintenance.py:1328-1348 offers no renderer-only mode, and execute():971 onward continues into tools/wake/trial/coldboot; its enclosing entry must not be replayed for renderer-only work.

Historical commands below are provenance, not current execution instructions; their temporary stage was removed:
```sh
LUMATV_D=/tmp/lumatv-renderer-XXCdjMIc/renderer sh /tmp/lumatv-renderer-XXCdjMIc/renderer/cutover.sh disable
LUMATV_D=/tmp/lumatv-renderer-XXCdjMIc/renderer sh /tmp/lumatv-renderer-XXCdjMIc/renderer/cutover.sh install
/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv compositor on
```

R2 request `179715b6-e14a-4347-af5f-08b3ccfbcb0b` was APPROVED then FAILED before mutation, change `fb6f79b1-4d44-46d0-8e1b-637f70652148`, exit1 after1.7s. Local attempt-1606d622abce49a5b01f15b43c2b209c/events.jsonl contains only four read-only file/hash checks: brand matched; tools/lumatv76310050 differed from expected d22a3ced. No upload/disable/restart occurred. The original permission is consumed, not denied.

At 01:27 EDT November1 the parent explicitly released its TV lease after reporting native Marquee37d8d0f4 deployment, 54 matching payload hashes, successful rk18700 dialogue-enhanced Opus2-channel playback and seek-preview cancellation, followed by video[]. IPK SHA c9e95defb396b62062fd7405551606c135d4da318a694fb28ddfe420162b7812. Temporary UI120/tick8.333333 is parent-reported; automatic pacing and zero-stutter acceptance are not claimed. Lease release is not permission for platform execution. No TV calls or new approvals occurred here; frozen packets remain unchanged.

Foreground Marquee f5c864 remains NOT installed or TV-accepted. Parent’s ONE corrected-request permission remains UNSENT/UNUSED, not standing or transferable. No prompt or TV action during human use. This token retains no-new-requests; consumed R2 cannot be replayed. Nitin subsequently selected Fix it offline here, permitting a separate narrowly corrected R4 candidate only; it does not authorize R4 execution or transfer the parent’s request permission. R2/R3, receipts, payloads and the historical renderer/R8 functions remain frozen.

R2 remains frozen for audit, not retry: `/data/home/tmp/lumatv-f5-renderer-packet/renderer-entry-r2.py`, SHA256 `7fbb2ed738bb1a16e772240193bd49caf6440751569edecf8f5a21515d25f5c7`. Separate R3 is `/data/home/tmp/lumatv-f5-renderer-packet-r3/renderer-entry-r3.py`, SHA256 `e3f1642c10f513f53e0e05406af2cb3b8dad41b82a2c2578b18e87de2f42e666`; its frozen scope-receipt-r3.json SHA256 is `974eae34150180a0ddd01e2372f4b9d856d5b297b1246a67b12ad760ad3b2164`. R3 changes only the exact tools/lumatv pin d22a3ced→76310050. Payloads, guards, dependencies, callbacks and rollback behavior are unchanged; no additional accepted hashes.

Final reviewer `review-recognized-dispatch-763100` found no blocker in the exact e3f1642c entry’s single-pin diff, independent payload copies, reference and provenance. Actual local --inspect returned ok:true. This is a local correction review, not a full hardware/security review or execution approval. The frozen receipt retains its earlier review-requested wording; this handoff records the delivered final verdict without changing that receipt.

Dispatcher7631005064da542f26f644ae1b636597e729884c4288d996c027b60185f366cf matches the installed observation, preserved source and accepted wrapper-package.ipk SHA8b1aba0a629fab7413285249bf2768712f1e9e886471cd05d32d2641e5f55f1c, corroborated by `/data/home/tmp/lumatv-wake-install.ms6eh5iv/wrapper-package-verification.json`. Relevant compositor status/on initialization and transitive calls are unchanged; differences concern wake/boot paths. Pin the accepted installed artifact, not a version label or temporary trial payload: d22 was an earlier trial that failed and rolled back. The other five reference dependencies match the accepted package, but their live checks were not reached; full tree/startup guards remain execution-time requirements. Unobserved extra files remain refusals, not new accepted states.

Staging both archives plus extraction needs 222553876 bytes (212.24 MiB). Initial free `/tmp` must be at least 384 MiB; post-stage `/tmp` and `/var` require 128 MiB. Known temporary 60/120 OSD state is accepted before cutover with unchanged commit callbacks and canonical startup; candidate and rollback must settle Home at 60, with candidate auto/default60. Unrelated tree/flags/process/dispatch drift or unreadable state vetoes rollback rather than overwriting an uncertain state.

Existing display recovery restarts SM/WAM/IME/booster, returns Home, may close stale input/Netflix preload, and clears all temporary librdx reports, including reports arriving after the last check. Pre-existing regular reports are copied off-device and checked before disable; they are not restored to the TV. These effects belong in the owning Face ID plan. No standby/reboot, native-app/storage, panel/fr_rate, native video-path or wake-policy writes are included.

Marquee owns a separately reported Home hero art/text swap hitch; `FixHomeBannerHitch` owns its local investigation. The hidden-shell host proof neither diagnosed nor fixed it. Separate native-app authorization is not compositor/platform approval. Source and evidence checkpoints predate these coordination-only notes; compositor artifact `f5c864` remains the final built, reviewed artifact and is not installed.


Final Bazarr connection showed `https://bazarr.nateshome.net`, the saved key masked, Connected, Bazarr 1.6.2, Sonarr 4.0.20.3014 and Radarr 6.4.4.10685. No API key was read, logged or changed. Evidence includes `keyboard-bazarr-final-connection.png`, `keyboard-address-restored.png`, `keyboard-bazarr-key-two-bullets.png`, `keyboard-bazarr-key-one-bullet.png`, `keyboard-bazarr-key-reopened-empty.png`, `keyboard-lg-search-two-g.png`, `keyboard-lg-search-one-g.png` and sanitized key JSON files.

For Marquee, virtual Back hides the keyboard but leaves the edit active; app Back cancels it. Require IME null before reopening with OK, which otherwise commits the draft. API-key drafts reopen empty: a masked two-character draft, one Backspace, virtual Back and app Back reopened empty; empty Done does not write. Nonempty Done persists: never test it on changed real credentials. Normal keyboard navigation uses existing `ctl remote CODE tap`; `ctl key` targets the app and bypasses IME. Actual RIGHT moved focus 0→2 and g→h. LG Search ignored app Back/Home for closing its overlay; existing `closeByAppId` for `com.webos.app.voice` closed it. `own_hide` already emits input_panel_state 0 and intentionally leaves the field active; SDL dismissal is not an established defect. Parse control JSON stdout separately from the known wrong-ELF warning on stderr; never dump raw shell, IME surrounding text or API keys.


Wake root task is complete: activation `c413b8707ca44e47ac788a3296573221` passed cold Home invocation `b370dc321fda4bff9227a16b2d043d82`; the two-minute healthy window cleared naturally, the existing wake commit succeeded with enabled true and pending null, and postcommit cold invocation `6ecd5f05acbb47898f067be9aafbe2e2` also passed Home without a manual postboot Home launch. Six warm cases passed: `warm-home-wol`, `warm-livetv-wol`, `warm-hdmi2-wol`, `warm-home-remotekey`, `warm-livetv-remotekey`, and `warm-plex-wol` (Marquee, native legacy ID `com.beb.plxnative`). Each used real native standby for 25 seconds, fresh HA AVR-off evidence and isolated WOL or software powerOn with reason remoteKey; native bootd/LumaResumeApp, actual foreground and DISPLAY, closed CoverUI, persistent/live/settled compositor with zero failures, external_arc and fresh HA AVR-on/input were observed. Software remoteKey is not physical-button proof; the LiveTV screenshot was black and does not prove visible channel content. Evidence and screenshots: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/`. Source publication remains on hold.

Keyboard acceptance passed on the actual TV without source changes or a build. Existing `lumatv keyboard luma` enabled the persistent `/var/lib/lumatv/.keyboard-luma` flag; Rust keyboard capability was already active. Marquee (`com.beb.plxnative`, its legacy ID) Bazarr address and API-key fields passed typing, Backspace, hide, cancel, reopen and Done; LG Search (`com.webos.app.voice`, `launchMode:vkb`) independently passed typing and Backspace. This is not an FPS or latency benchmark. Final `keyboard-final-state.json` and `keyboard-final-home.png` show TV Active, native Home, IME null and keyboard luma; own counts activations 7, shown 7, texts 6, keys 6, nav 110, failed 0. The compositor was persistent, live, mounted and settled with zero failures; wake enabled/pending null, external_arc and fresh Denon on TV input. TV and AVR were left on. Evidence: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/`.

Separate CEC tests: sole HA `remote.turn_on` did not wake; sole Apple TV `remote.send_command(home)` woke Home standby to HDMI2 with actual native `hdmiCec` and AVR AppleTV/MPLAY on, without TV powerOn or a Home override. Native receiver MPLAY is the configured Apple TV, corroborated by fresh HA Apple TV state and prior r10/r11 logs; retain the original false raw/display-name comparison and `receiver-source-corroboration.json`. The direct CoverUI fallback chain passed from LiveTV: firstApp `{}`, secondApp `{}`, fallback absent ghost ID and finalFallback Home resulted in closed status and actual Home/DISPLAY; inventory confirmed ghost absent and Marquee (`com.beb.plxnative`) present. This does not prove a missing-saved-app wake. Physical Netflix Hotkey/OTT wake remains unaccepted: the historical Netflix log was copied from a prior boot and its stimulus is unknown, with no proven safe software equivalent. Native input/Hotkey unit contracts were reviewed, not accepted as hardware-button tests. Never write native intent files.


At 17:29 EDT, opening live tuner settings reportedly crashed Rust Home and left the TV on the old compositor. The trigger and resulting screen are owner-reported; the fault is not diagnosed. Rust Home was subsequently restored by the renderer installation below. Full stock-screen migration and the tuner-settings crash remain separate deferred backlog items.

Audio, bounded RAM/syslog logging and the custom keyboard were installed at 14:36 EDT. Wallpaper controls passed physical-TV acceptance at 16:24. The approved wake-maintenance batch subsequently installed compositor `6fe8baeae18aa1aa039a6fc6103cc1559214d44bfd55db378c31d27db7db7052`; it was live, persistent and settled before the tools-copy failure. UI `d4313791`, supervisor `3635ed0c`, helper `c70c9e6f` and capture `00999f22` are unchanged. The later accepted wake state is recorded above. Source publication remains on hold.

Specular 0.10→0.15 and Bloom 0.65→0.70 independently changed the frozen wallpaper picture. Each restoration returned the wallpaper-only crop `(60,150,1250,1000)` pixel-exactly to its baseline. The complete control values, including Motion On, were restored and the TV returned to unobstructed Home with eARC selected. Speed changed/restored through the TV controls earlier; speed-ratio rendering is host-tested, not a physical-TV timing measurement. The compositor is live, persistent and settled with zero failures.

The renderer previously discarded lighting controls at zero speed. It now retains those controls with motion disabled. A separate glyph-free-scene damage omission is also corrected; it is not established as the cause on Home. The actual compositor-output regression failed before and passed after; production Home wallpaper and Silk-report integration tests passed. Renderer unit tests remain unbuildable in the synchronized tree because its test-only upstream files are omitted.

Final wallpaper evidence: `/tmp/lumatv-wallpaper-fix-tkil2ncd/evidence-8dd46fb2/results.jsonl` and `/tmp/lumatv-historical-stage.TyUJ9M4D/ui-evidence-20261031-162401-ab4c9a31/`. The frozen batch `install-and-check.py` has SHA-256 `a7ce0ec42114aae845ac24e8438d10ea9cf174576ec539effb4a2e262773dbdc`; bundle `b9417dad810837e7811b912321f4aec839a2f9027af41bedb940c2a5dd15ebf1` differs from the previous bundle only in `bin/lumatv-compositor`. The successful verification run skipped installation. Grant `4dd9e715-6d4e-4ef7-a14b-95db671b8ce2` has one use remaining until 03:20 EST on November 1. Uploaded archive and individual captures were removed from TV RAM. Final free space was 881084 KiB in `/tmp` and 617968 KiB in `/var`; reports and images stay on Cody.

Three TV-only standby/wake cycles passed: Denon followed off/on without direct AVR power commands. Selecting Apple TV on the TV also selected Apple TV on the AVR after wake. HA timestamps are observations, not hardware latency measurements; three cycles do not establish perfect reliability. Evidence: deployment `power-hdmi-evidence.json`.

Physical Denon recovery passed with SIMPLINK disabled: the helper powered the AVR on and returned to eARC after a helper restart. The loopback unavailable-receiver case attempted wake, fell back to TV speakers while retaining saved eARC, stayed on speakers while the fixture remained off, and recovered eARC when it reported on. Audio, eARC, SIMPLINK, receiver address/port, volume and input were restored; formerly default receiver address/port are now explicit equivalent values. Evidence: `/data/home/tmp/lumatv-restore-20261030/audio-acceptance-recorded-dt4esdx5/events.jsonl`.

Logging forwards to `192.168.1.69:514` over UDP with a 1 MiB RAM limit. Actual kernel and app markers reached Unraid; accepted stdlog is covered, compressed rotations are not replayed. The last detailed readback reported zero source drops/gaps and 852 KiB RAM logs. Evidence: deployment `syslog-receipt.txt` and `final-installed-readback.jsonl`.

The actual Home Search keyboard passed entry, on-screen Backspace and Back. System > Logging displayed the destination, forwarding status and RAM cap; its custom address keyboard opened and cancelled without changing the destination. Evidence: deployment `ui-evidence-20261031-155857-e4f7cb8f/`. This proves shell keyboard interaction, not third-party app IME coverage or measured 120 FPS pacing.

Historically, wake destination was unresolved. Native `bootd.log` proves Home was correctly saved before standby, but stock `wakeOnLan` excludes full Home booting and selects the Last Input App, Live TV. The runtime native-policy trial failed its readiness/log/foreground check; recovery restored the stock policy path and original full-Home reason list. No full TV reboot or new wake/source acceptance sequence occurred. The failed native trial is not proof that the policy itself was rejected.

The native bootd binary and actual rules are captured at deployment `native-wake-sao1j7vh/`; binary SHA-256 is `e701cafc51ae7faba89452f1cdc1efb314c9fccf3e70eb404bdca3d17e5fbc27`. Stock Last App Booting handles only `cpuAbnormal` and reads `last_foreground_app_id.json:appId`, not the normal saved foreground `bootd_last_foreground_app_id.json:lastAppId`. The candidate subsequently installed clones the native Cover App action, retaining hotkey/OTT ordering and other explicit-destination exclusions; previous-app resume covers WOL, remote/local power, `micomPower` and `lastPowerOn`.

The installed Rust CoverUI replacement launches real apps with four fixed candidate slots, including a final Home fallback. Native wake has 37 local behavioral tests and real-CLI trial/mismatch recovery smoke. The earlier native trial proved policy loading, not destination selection: its log truncated Home to `com.webos.app.hom`. Source now terminates selector and shell-fallback output with a newline for LG’s final-byte-stripping reader. Both paths passed actual-TV CLI smoke through r8 without installation; temporary source was removed. Later persistent activation and destination acceptance are recorded above.

Retail `configd` accepts top-level `volatile:true`, but its loaded service merges volatile values before Factory values; Factory wins whole-key conflicts. Nonvolatile API writes update Factory. The trial removes only the two owned stock-valued Factory overrides after checking stock main-database fallback; recovery restores Factory first. Main defaults remain unchanged. Captured bootd constructor/getter confirm an absent path retains `/etc/palm/action-condition.json`.

Historical: TV BusyBox 1.35 rejects GNU `chmod --reference`; preserve permissions with `stat -c %a` and explicit octal `chmod`. This copy incompatibility was corrected in the maintenance packet. Its resumed execution, request `c92e8ad3-7370-4950-a86a-0337bebfb1af`, failed at `native-trial` and completed verified safe rollback. The original autostart and CLI hashes are restored (`5aa2f6fb9006a36e6590bfd6750c223ff65543b09fcea538c19369801661fc5c`, `c98f340da1f36fcc0ca94e17d708d32ae2c15ecee089e12ffc0c0580069f96d9`); installed `tools/wake.py` is absent. Inactive owned policy/snapshot files remain at `/mnt/lg/cmn_data/lumatv/wake`. The formerly absent config key now explicitly names `/etc/palm/action-condition.json`; the original reason list is restored.

Historical: Work is paused at the owner’s 18:12 EDT request. Final evidence is deployment `wake-maintenance-evidence/events.jsonl`, attempt `2ebcae5af8a74f948a60f1b0cbdbde1a`: `failure-safe-rollback-complete` follows healthy, persistent, settled compositor status with zero failures, a physical DISPLAY capture and `finish-saver-confirmed`. The capture is `attempt-2ebcae5af8a74f948a60f1b0cbdbde1a/display.png`. Home was visually observed before the saver was activated. TV remained Active; Denon was last observed on with eARC selected before the native trial. No AVR power/source commands or TV standby/reboot were issued by this run. Nothing is scheduled for this chat. Both maintenance approvals executed once; neither is pending.

Resumed diagnosis establishes that native `bootd` opens its log with `O_RDWR|O_CREAT|O_TRUNC` (`0x242`, captured ARM binary address `0x6b28a`). The old inode/size witness therefore rejects ordinary restart-time truncation. The saved failed trial did not preserve its individual predicates, so this is a confirmed verifier defect, not proof of which predicate failed that run.

The corrected witness reads at most the final 1 MiB and requires the latest `PERFORMANCE`-stamped `BOOTD_START` within the trial's monotonic-clock window before accepting both owned-policy load records. The captured logger uses `CLOCK_MONOTONIC`; this set reports no hardware timer. A different clock origin fails closed. The actual captured-log replay accepts a fresh truncated log and rejects the same log as stale. Trial failure records retain each check's successful observations and exceptions before rollback overwrites native logs.

The native-only diagnostic temporarily migrates only the exact reviewed inactive selector, runs one volatile trial, restores stock config and prior selector/markers, and verifies unchanged installed tools and healthy renderer. It performs no renderer installation, persistent activation, reboot, standby, input/app launch or saver change. The obsolete BusyBox-resume command remains invalid.

Earlier diagnostic `06d095e48d7f48719b781f07b54deb33` failed configuration readback before restart. The actual Factory database contained the stock path and five original reasons, shadowing the trial. Read-only state evidence is `/data/home/tmp/lumatv-wake-readonly-2odth4qv/`. Its leftover RAM directory contained only a 20404-byte Python bytecode file; that file and the empty directories are removed. New diagnostic imports disable bytecode writes.

Corrected diagnostic `234ed66a71784b0e8c2133fbb39bebd8` passed on the TV: both owned-policy load witnesses were true and foreground remained Home. Stock settings, original selector/markers and original tools were restored; renderer remained healthy, persistent and settled with zero failures. Evidence: `/data/home/tmp/lumatv-wake-layer-trial.m8iisQ/wake-maintenance-evidence/`. The earlier frozen packet is unchanged.

The owner resumed the wake fix and explicitly prohibited further HomeLabber access/approval requests. HomeLabber is not used. Previously successful testing, deployment and recovery paths are required; no further wrappers or replacement frameworks. Both instructions are saved in global memory. The original 86-task inventory remains intact; one completed learning-record item, six render-first UI items and the immediate keyboard replacement bring the total to 94 (39 complete, 0 open, 55 blocked). No source publication is authorized.

Historical: Activation `6928bb03053940598f3ed1c93054cde3` failed the snapshot-start policy witness. Stock configuration was restored before a six-second bootd restart timeout; later reads proved active/running, success and no pending job. The existing `wake off` then succeeded, with stock config and no pending/trial markers confirmed. The existing backup restore returned the original autostart/CLI hashes and removed installed `tools/wake.py`; renderer remains persistent, settled and healthy with zero failures. The newer inactive selector, failed report and migration backup remain preserved because exact marker restoration rejects the historical timeout error. Do not replay activation or bypass that check.

Current source serializes prepare/reload, existing landing/apply and finalize with invocation/activation binding and scoped restoration after owned input teardown. It accepts LG’s padded early-start timestamps and normalizes only corroborated native Input App inputcommon wrappers to the matching input. All 61 wake tests, both shell syntax checks and captured-log delivery/selection smoke passed. Independent native review cleared the wrapper/action-pairing contract after missing-request boundaries were fixed; captured-log replay is not TV acceptance.

The TV has reviewed source `cfc2c1aac8c33654051184c758406980767ee5e0a3cb828ccef244b08656f233`, package SHA256 `8b1aba0a629fab7413285249bf2768712f1e9e886471cd05d32d2641e5f55f1c`. All 151 QML files compiled; three installed tools/helper/408-file compositor tree matched and apply took 16 seconds. Fresh Home activation `c413b8707ca44e47ac788a3296573221` passed runtime trial AND actual cold startup, invocation `b370dc321fda4bff9227a16b2d043d82`: both loaders, native readiness, destination and original scenario accepted, actual Home DISPLAY captured with no manual postboot Home launch. The existing two-minute supervisor healthy window then cleared, all persistent/live/verified/settled checks passed with zero failures and boot-ok marker absent. Existing wake commit succeeded; status enabled, pending null. A fresh post-setup cold test is running with prior invocation reports explicitly ignored; standby/intent/receiver/fallback acceptance still remains.

Historical inactive directories are preserved at `/mnt/lg/cmn_data/lumatv/wake-archive-46a404c6723042deb67505e53c1ec217`, `/mnt/lg/cmn_data/lumatv/wake-archive-4c6ad6ee0f2d49dfa2b1885c33afe751` and `/mnt/lg/cmn_data/lumatv/wake-archive-437a8319ad91488d9ac0ee48926b1d67`. The last preserves the disarmed failed activation7ad report and B51 selector; all files were checked before/after atomic rename under both locks with stock effective/Factory settings and no bootd job. Current evidence and exact packages are private under `/data/home/tmp/lumatv-wake-install.ms6eh5iv/`; full pre-update `installed-package-and-state.tgz` remains recovery evidence, not permission to overwrite mutable live state.

Verified constraints, evidence links and unresolved startup questions are recorded in [wake-learnings.md](research/wake-learnings.md). Append dated findings there; distinguish observation from inference and update this current-state section separately. Later new-policy acceptance and its limits are recorded above. A stock-policy HDMI-CEC wake preserved HDMI2/Apple TV despite saved Home and stale firstAppId Home; its native log and DISPLAY capture are in the learning record. Reuse the existing scripts and documented 25-second standby baseline. The owner waived the panel-refresh wait at 19:30 EDT for this work only. Preserve unowned `rustui/ui/src/system/saver/model.rs`; no commit, push, tag or release under the source-publication hold.

The compact toast and audio glyph changes are source-implemented and host-native tested; TV installation and acceptance remain pending (current maintenance state above).

The owner reported LG’s old keyboard in Marquee’s Bazarr address/API-key entry. The existing Rust keyboard passed both fields and LG Search; see [keyboard-result.md](research/keyboard-result.md). The original broader keyboard task is complete; actual visual remote input was accepted, without an FPS benchmark.

TV verification reuses `compositor/tools/tv-gate.sh`, the October 29 r19/r19w Rust controls/capture helpers and the Round 8 receiver baseline. Full gates also change sessions, refresh and final power state; select relevant operations. Read control JSON with stdout/stderr separated because the TV loader emits a preload warning. Home Search is focus 103, gear 102; key code 217 is only a test fixture. Wallpaper rows clamp rather than wrap: navigate toward the actual target index.

The supervisor deliberately keeps its `running` marker for the first 120 healthy seconds. A fresh successful installation can therefore report live, persistent, zero failures and `unsettled:true`; wait for settling before final acceptance rather than reinstalling. Preserve durable Cody-side results when MCP times out before its server command finishes; never repeat an unknown action. Cody’s Paramiko 5.1.0 lives in the persistent Python user site under `/data/home/.local/lib/python3.11/site-packages`; container replacement does not guarantee system-site packages survive.

## Earlier snapshot (LumaTV, 2026-10-30 EDT)

- The TV runs forge main (round 27 in progress); LumaTV's compositor is live persistently,
  LG's as the automatic fallback. Rust kinds are off by default.
- Camera pop-ups use SUB0 (the second video window) over apps and the CPU texture over
  Home. `popups.on` is OFF after an unexplained reboot during the camera full view over
  YouTube (round 26); turn it on only once that is explained.
- 120 Hz frame work: Frames120b, `docs/research/frames-120-result.md`.
- lumashow-ha 0.3.0 and LumaShow v1-20261030a are released; HA's LumaShow 0.3.0 runs with
  48 entities shared to LumaTV.
- All screens move to the 64-bit Rust shell; no new QML. Home, quick panel, picker, drawer,
  Cameras and part of Settings are opt-in. Remaining Settings and system surfaces are in
  progress. QML retires per kind only after two device parity passes. Recovery points are
  `rust-home-result.md`, `rust-panels-result.md`, `rust-settings-result.md`,
  `rust-cameras-result.md` and `system-layer-result.md` under `docs/research/`.
- The saver gains a dim analog clock, date, weather, notices and plugin items. The helper
  rules and camera-over-saver fix are implemented; surfaces and device checks are separate
  work. HA couch-occupancy wake is parked. Scrypted replaces HA cameras; bridge installation
  and pairing remain human actions (`scrypted-cameras-design.md`).
- Privacy is addressed on the TV, not by router blocks. The early hook stubs programs
  before masking units; the boot timeline must establish whether it beats LG's starts
  (`privacy-menu-result.md`). No data deletion or system-image changes are authorised.
- OSD0_1 held writes but did not latch its header or show an extra layer under LG's AFBC
  path. The linear case remains open; evidence is in `compositor/osd01-probe-2026-10-29/`.
  `display-chip-direct.md` ranks hardware work; `tools/regdecode.py` decodes captures.
  Host policy tests do not establish working cursor, AFBC or extra video windows on the TV.
- Interrupted work has snapshots and ownership notes at
  `/data/home/tmp/lumatv-recovery-2026-10-29/RESUME.md`. Read that and the live worker roster
  before resuming: replacements may already own the original worktrees. Preserve unfinished
  files. Rate limits and an exhausted API balance are separate failure modes.
- At most 12 helpers and one deep-task worker; no spawning by helpers. Only the verified
  human gate permits protected actions. No raw root infrastructure commands, secret reads,
  network changes, data deletion or protected installs outside that gate. A changed installer
  script is not covered by an earlier standing approval.
- Hourly check-ins and a 06:05 EDT recovery check are scheduled. Conservative defaults resolve
  routine engineering choices. Marquee remains parked until the Rust screens are complete.

## Earlier snapshot (LumaTV, 2026-10-26 19:05 PDT)

- Source of truth: `docs/PLAN.md` (phases, gates, decisions), `docs/SURFACES.md`
  (inventory), each `docs/research/*-result.md` (its worker's recovery point). An
  implemented file is not proof that its feature passed on the TV.
- TV installs: every LumaTV install on the set goes through Nitin's Face ID
  (HomeLabber `run_command` on beastnas running the installer in the Cody container,
  approved on his phone). A yes in chat does not count (PLAN, Decided). A request that
  expires pauses HomeLabber approvals until Nitin resumes them.
- Helpers: at most 12 at once; briefs forbid spawning. One TV owner at a time,
  deploying from a clean worktree; the compile gate (`tests/qmlhost/compile_all.py`)
  runs before every package or hand deploy. Helpers commit from their own worktrees.
- Installs: HomeLabber standing grant on `sh /mnt/nvme/appdata/cody/home/tmp/lumatv-install/tv-install.sh`
  (installs whatever `/data/home/tmp/lumatv-next` points at; one Face ID per 12 h).
- On the set at 11:00 PDT (10-28): package a1555df (round 14) on **LG's compositor**:
  YouTube (Cobalt) and Plex (plxnative) SIGSEGV under LumaTV's compositor (LG's SDL patch
  calls the webOS seat proxy, which is NULL; evidence `docs/research/compositor/slot-2026-10-28-r14/appcompat/`),
  so persistent go-live is paused until AppCompat's fix passes with app launches in the gate.
  Always Ready on (Nitin). On: `inputcommon`, `voice`, `store`, `remote.quickAccess`,
  `remote.quickPanelEdit`, keys restyle. Passed under LumaTV's compositor and waiting for
  go-live: keyboard luma, app switcher, transitions, overlays, OLED care orbit, capture 1-5.
  Video wallpaper fails (black margin, darker grade); 120 Hz holds idle but not the slides.
  venc's next TV probe needs Nitin's OK (the M2M node wedged the kernel once).
- Done 2026-10-26: compositor gate 9 (every safety check passes, frame timing fails:
  `compositor-gate9-result.md`); compositor step 1, the remote's keys and the keyboard
  under LumaTV's compositor, passed on the set (`compositor-step1-result.md`); quick
  panel and picker round 16 (`shell-host-result.md`): over Netflix and HDMI every panel
  opens in 146-159 ms, over Home p90 2.42/2.54 ms.
- Open, in order (also the todo list):
  1. Installed 3036de9 through Face ID (2026-10-26 23:28 EDT, during Netflix, which kept
     playing): c496611's still mute mark, the Zigbee2MQTT OTA guard, `revert` removing
     `vt_devmode`. Not yet re-measured: the one late frame per panel opening over Home.
  2. Light sync: the Zigbee2MQTT OTA guard and back-off are built (4dd5a5b, offline);
     `vt_devmode` is on for good (Nitin, 2026-10-26). There is no Hue light by the TV:
     Nitin picks which lights follow the picture; WLED strip not set up (PLAN, Ambient
     light, Status).
  3. TV checks: Live TV restyle, Settings look-only walk, frame-rate blink with Nitin
     watching.
  4. Parked by Nitin: compositor frame timing and moving Home and the panels to
     LumaTV's compositor; Music Assistant.
  5. Nitin's steps: GFN sign-in, HA token and plugin enable, Bluetooth proxy link, LG
     store check, leak sensor OTA retry, Denon OSD off, Apple TV CEC off.
- Netflix tvq-pb-101 came from an input app left holding the audio decoder port;
  `lumatv inputs status|release`, and gate.py now closes the HDMI app it opens.
- Backups of uncommitted work: `/data/home/lumatv-backups/wip-*`.

Written so any model can continue cold. The plan is `docs/PLAN.md` (Status,
Phases, Decided); this file holds the proven mechanics.

- State: the build is under way (PLAN.md track C and the surfaces). Proven on
  the set and in the repository:
  - The rename to LumaTV (`docs/research/rename-result.md`), the Home stub
    (`docs/research/homestub-result.md`), the performance work of phase 1
    (`docs/research/perf/summary-2026-10-24-phase1.txt`).
  - Surfaces phase 1 (`docs/research/surfaces-p1-result.md`): fail-open key
    filter, OK on release, the focus rule, wallpaper tint, one token source,
    Manrope, the component kit.
  - Surfaces phase 2 (`docs/research/surfaces-p2-result.md`, which has its own
    Resume here): the startup warm-up, the per-surface frame gate and the
    startup A/B in `docs/research/perf`, the offline QML host
    (`tests/qmlhost`), Home's weather and news from the native helper, the
    helper's start and stop in the tool, the preload trim, the boot and
    power-off logo windows, `lumatv surfaces` (the registry of every switch and
    its way back) and the notification view (`lumatv notifications on|off`: LG's
    tree copied on the set and patched there with LumaTV's own toast laid over
    it, nothing of LG's shipped; `tests/qmlhost/notify.py` runs LG's real view
    offline). LumaTV's Settings (`Settings.qml`, `settings/`: Home & look and AI
    access pages) is a layer of Home opened from the gear's popover, and the AI
    prompt (`AiPrompt.qml`) draws the helper's `ai.prompt` over Home; both are
    tested offline against a fake helper (`tests/qmlhost/fakehelper.py`, which
    models `/v1/ai`) and have not run on the set. The set has run the warm-up and
    the Home gate (slot 1); that note's Log has the numbers.
  - The native helper (`helper/`, `docs/research/native-helper-result.md`):
    proven on the set (smoke test 35 of 35; AI-access smoke 42 of 42, which skips
    the two checks that touch the TV and the receiver). The plugin broker
    (LumaShow's plugin API level 1) is built and host-tested, not yet run on the
    set. Its API is on `127.0.0.1:18731`; files under `/var/lib/lumatv`:
    `helper.lock`, `helper.json` (optional), `cache/weather.json`
    and `news.json`, `settings/<owner>.json`, `bin/lumatv-helper` (the staged copy
    the tool runs). Stop it with `lumatv helper stop` or `lumatv-helper stop
    --brand <app dir>/brand.json`; if TLS misbehaves set `"fetch":
    {"backend":"curl"}` in `helper.json`.
  - 64-bit drawing (`gfx64/`, `docs/research/gfx64-result.md`): heap shim,
    launcher, pinned blob fetch; smoke run on the set 60.63 fps over idle Home.
    The Rockchip blob bundle is fetched by `gfx64/fetch-blob.sh` (about 37 MB)
    and never committed; an ipk that ships it must run that at build time.
  - LumaTV's own compositor (`compositor/`, `docs/research/compositor-c2-result.md`):
    C2 passed on the set; C3 and C4 built offline, first window and bridge dry
    run on the set. The tool owns the enable path now (`tools/compositor.sh`,
    `lumatv compositor on|off|status`, `lumatv compositor-boot` in the boot
    worker): the bundle itself is put in `/var/lib/lumatv/compositor` by
    `compositor/tools/tv-cutover.sh install` until it ships in the ipk. LG's
    compositor is the fallback on any failure; `lumatv revert` unmounts and
    forgets the flag. Fragility: after LG's compositor was away, `webapp-mgr`
    can be left `failed` and the web Home stub cannot launch (black panel):
    `systemctl reset-failed webapp-mgr; systemctl start webapp-mgr`, then
    launch `com.webos.app.home` (`tools/tv-recover-home.sh` does it, and
    holds `/tmp/lumatv-recover.lock` while it runs; the tool never restarts the
    compositor's unit while that exists).
  - Hardware census (`docs/research/hardware-census.md`), the top five by benefit
    over risk, none run yet and none needing a kernel module: 1 the video tap and
    picture histogram (`libvtcapture`, for ambient light), 2 the TV's own audio
    mix (`hw:1,2`, `hw:1,3`), 3 the hardware JPEG block, 4 hardware video decode
    for a video-plane wallpaper, 5 Bluetooth LE through LG's `bluetooth2` service
    (PLAN.md, Hardware census). LumaShow lessons:
    `docs/research/lumashow-lessons.md` (nice -19 on surface-manager is LG's own,
    nothing to adopt).
  - Declutter (`tools/declutter.sh`, `docs/research/declutter-result.md`): the
    `lean` profile is on and persisted on the set (uploaders, ads, push,
    settings, apps, voice, connectivity, share, misc; `ai`, `livetv` and `cast`
    off). Rebooted clean at 04:08 and checked at 05:15: Netflix, YouTube,
    PlxNative, Apple TV (about 20 s), Crunchyroll and Prime Video launch;
    GeForce NOW, a web app, opens the browser as before; the receiver is
    connected on HDMI 2 with sound on `external_arc`; Home Assistant sees the TV.
- Nitin's decisions of 2026-10-25 (also in PLAN.md Decided): the approved
  mockups in `docs/mocks` are the design, Settings layout included; LumaTV's
  compositor goes live as soon as it passes the C4 gate, LG's staying as the
  automatic fallback; LG's telemetry, ads, promotion and app-push services are
  removed entirely and webOS is trimmed to essentials through two reversible
  profiles, `ads` and `lean` (`docs/research/declutter-result.md`); LG's power-on and
  power-off logos are replaced by LumaTV's, three options each in
  `docs/mocks/logo`, option 1 the default; Prime Video stays unless an LG
  surface clearly pushes it. `lumatv preload on` keeps Netflix and Prime Video
  warm and drops YouTube, LG Channels and the browser from the list; the A/B on
  the set (slot 2) showed no gain, since it closes nothing that runs, so it is
  off until a cold boot shows one.
- Remote tests: done 2026-10-24 for Netflix 4K Dolby Vision playback (X4
  pass 3, contiguous-buffer hold) and the SSAP key device; results in
  PLAN.md. Still open: X4 during PlxNative playback
  (`docs/research/arch/x4/x4-monitor.sh`, `x4-analyse.py`); the histogram
  probe (its ABI was recovered offline, commit dd3c57a). Mute
  the receiver first (`media_player.living_room_avr`) and restore it after.
  Netflix deep link for the 4K Dolby Vision test film: launch `netflix` with
  `contentTarget` `m=https://www.netflix.com/watch/80141336&source_type=4`,
  then ENTER. Protected video captures black; the tests record compositor
  behaviour. During Dolby Vision playback Picture Mode reads `dolbyHdrCinema`;
  it returns to `filmMaker` on Home.
- Tests that need the owner: PIN prompt (a lock is a security setting he
  sets), pointer cost (Magic Remote moved by hand), CEC keys from other
  remotes, receiver volume drift.
- Set rules: confirm it is idle first (`getPowerState`,
  `getForegroundAppInfo`); Picture Mode stays `filmMaker`; never restart
  SAM; write only under `/var/lib/lumatv`, `/tmp`, configd, bind mounts
  and `/var/systemd/system/env`; revert every experiment; leave it in
  Active Standby unless the owner is about to watch; no DRM bypass. Slots on
  the set are asked of the main agent and answered "go <name>".
- Tests: `python3 -m unittest discover -s tests -p 'test_*.py'` (101, about
  55 s; the ones that start the offline QML host need PyQt5 and an X display,
  `tests/xdisplay.py` starts Xvfb itself) and `TZ=America/New_York bun test
  tests/` (36). The frame gate on the set: `python3 docs/research/perf/gate.py`
  (every number and rule is in its docstring); startup and preload A/B:
  `startup.py`, `preload_ab.py` beside it.
- Working style: one step at a time, commit and push after each
  (`git push -q forge main`), results written to files with their own
  "Resume here" note, so a model switch mid-task loses nothing.

## The rule that must not be broken

https://rootmy.tv/warning — **never write to the kernel, rootfs or tvservice
partitions.** A set bricked that way is not recoverable in software.

Everything this project does stays inside that boundary, deliberately:

- Only bind mounts and files under `/var` (writable ext4) and
  `/media/developer`. Never a raw write to a block device.
- The single exception is `snapshot-boot-manager --remove`, which clears the
  hibernation image on `mmcblk0p54`. That is LG's own tool on its own
  scratch partition, the kernel rebuilds the image on the next boot, and the
  worst case is one slow cold boot. It is not a firmware partition.
- `/usr`, `/etc`, `/lib` and `/mnt` are read-only overlays with lowerdir only
  and cannot be written even as root, which is why the mod shadows the
  compositor module and bind-mounts over Home's assets instead of editing
  them in place.
- No firmware flashing, no `dd` to `mmcblk0*`, no epk handling.

If a future change needs a partition write, stop and reconsider the design.

## The set

- LG OLED65C4PUA, model code `o22n2`, webOS 10.2.1, firmware 33.22.52, MediaTek
  (`chip=O22A3`), 2 GB RAM, armv7 userspace (`GLIBC_2.28`).
- Rooted with Homebrew Channel. SSH root login via Homebrew Channel's sshd; no
  passwordless sudo, the login is root. `luna-send` from a non-PTY SSH exec
  hangs — allocate a PTY. `journalctl` hangs — read `/var/log/messages`
  (monotonic timestamps in brackets), which rotates within minutes.
- A plain `reboot` returns powered on and is safe to drive over SSH. The luna
  `machineReboot` drops to standby and does not power back on. Wake-on-LAN
  works with `wolwowlOnOff` on (built-in LAN port), so a set in standby or
  fully off can be woken with a magic packet.
- `/sys/power/mode` reads `resume` on a normal boot and `making` on a boot
  that rebuilds the hibernation image.
- Screen capture: `luna://com.webos.service.capture/executeOneShot` with
  `{"path":"/tmp/x.png","method":"DISPLAY","format":"PNG","width":960,"height":540}`,
  then sftp the file. A blank panel returns ~835 bytes at 320x180.

### Developer-app filesystem and maintenance

The established deployment entry is `/data/home/tmp/lumatv-install/tv-install.sh`: app install, `tv-first-light.sh deploy`, then `cutover install`. The earlier installer uses separate SSH connections per command and a dedicated SFTP connection per transfer; native control commands use a PTY. Installed Rust binary hashes are recorded in `docs/research/compositor/slot-2026-10-29-r19w/log.txt`. Reuse this transport before adding another deployment mechanism. Full historical `apply` and cutover scripts also change unrelated app state and restart the display, so selective updates must retain the current artifact scope and private configuration.

On the reference set, `/media/developer/apps/usr/palm/applications/org.nphil.lumatv` and its developer-app ancestors through `/media/developer` are root-owned mode0777 directories. Existing public app files can also be mode0777. The approved maintenance packet must pin the inspected ancestry identities and retain no-follow checks; this layout is not a reason to relax private `/var/lib/lumatv` or RAM-workspace permissions, or to change directory modes globally. Device and inode numbers must come from current inspection, not a previous run.

The established full-package and Rust-bundle installation remains the deployment path. The experimental combined-maintenance runtime validator is not an installation prerequisite. Preserve user-facing configuration through the existing migrations, retain rollback copies and storage checks, and verify installed hashes and running services; a wrapper exit code alone does not prove installation.

Normal work is allowed while the TV is in use; a screensaver is not a maintenance prerequisite. OLED idle protection and state-dependent screensaver tests are separate. Display interruptions require owner coordination. Installation while watching permits an agreed compositor interruption, not deliberate TV or AVR standby; physical power-cycle acceptance stays separate.

Large Python maintenance programs travel over bounded SSH stdin with a fixed command under1KiB and data chunks at most16KiB. Dropbear documents a32KiB default receive-payload limit; the former quoted maintenance request was49,986bytes and reproduced a connection reset against that limit. The TV-specific reset cause remains unconfirmed. Native Luna commands retain the existing PTY transport. The current install-while-watching scope permits a compositor interruption but no deliberate final TV or AVR standby; physical power-cycle acceptance stays separate.

`surface-manager-daemon.service` starts `/usr/bin/surface-manager.sh`. Maintenance owns the verified unit and its complete existing cgroup, with process-start and executable continuity checks; child names alone do not establish ownership. An existing bind-mounted `/usr/bin/surface-manager` can retain an unlinked executable inode. Any zero-link read exception is restricted to that canonical path with matching mount, inode and content evidence; other regular files retain the single-link requirement.

## What the mod is

Two halves, one owner each.

- Compositor: a full shadow copy of the `WebOSCompositor` QML module (318
  files) at `/var/lib/lumaglass/qml`, with `views/fullscreen/
  StarfishFullscreenContainer.qml` swapped for stock-plus-one-`Loader` and a
  `lumaglass/` directory added beside it (LumaHome.qml, Glass, StatusBar,
  Widget, Dock, Tile, the widgets, pre-rendered icons). `surface-manager-
  daemon.service` has `EnvironmentFile=-/var/systemd/system/env/surface-
  manager.env`; the mod writes `QML2_IMPORT_PATH=/var/lib/lumaglass/qml:/usr/
  lib/qt5/qml` there. `/usr` is a lowerdir-only overlay and can never be
  written. The Loader instantiates LumaHome above the Home surface while
  Home is the container's app; the stock Flutter Home stays resident and
  unseen underneath, or the web stub that `homestub on` puts in its place (see
  "The Home stub").
- Home: `mount --bind /var/lib/lumaglass/assets` over
  `/usr/palm/applications/com.webos.app.home/data/flutter_assets/assets`
  (decluttered `home.xml`, strings, blank hero banners), then relaunch Home.
  A bind does not survive a reboot.

Input: the compositor's `KeyFilter` sends remote keys straight to the focused
Wayland window, so a QML item never sees them through QtQuick focus. The
stock `StarfishKeyFilter.qml` builds its handler chain from the configd list
`com.webos.surfacemanager.keyFilters` (`[{file, handler}]`, loaded through
`StarfishKeyFilterLoader.qml` + `Qt.include`, re-read live on change). The
tool appends `{file: /var/lib/lumaglass/keyfilter/lumaglass.js, handler:
handleLumaHome}` at apply and boot-bind and removes it at revert. That script
runs inside the compositor's QML engine, instantiates `LumaKeyBridge.qml` from
the module and dispatches through the `LumaBus.js` pragma-library instance
LumaHome registered with; it accepts a key only while Home is the active
surface and LumaHome consumed it, everything else falls through to LG's
chain. Pointer input is ordinary QtQuick mouse events (LumaHome sits above
the surface item and swallows what no card or tile takes).

Rendering: one `GaussianBlur` + `HueSaturation` of the wallpaper at half
resolution, baked once into a `ShaderEffectSource`; every card, tile and the
status bar is a single `ShaderEffect` quad that samples that texture at its
own screen rect and draws tint, inset edge, sheen, rim and drop shadow from a
rounded-rect signed distance. No layers, no per-frame effects; focus is a
uniform plus a transform. Text is `NativeRendering` on integer positions and
never inside a scaled item (the glass under it scales, the content does not).
Icons are PNGs rendered by `generators/gen-icons.py` at display size (no
SVG plugin on the set).

Frame budget, measured with `focusMs: 5000` and the per-second `[FPSLOG]`
counter (middle seconds are exact): the set is fill-rate bound. Dock alone
60; each widget card's glass quad costs ~2 fps until the whole layout sat at
50, and a two-read live wallpaper took 10 more. What brought it back to 60:
shadow-only early return outside the card/tile rect, margins trimmed to the
shadow reach, mediump colour math, scrim folded into the wallpaper pass, the
covered stock Home surface at opacity 0, and the wallpaper warp moved into
the vertex shader on a GridMesh. Every new full-screen or card-sized quad
must be re-measured this way. Measured: idle 0 frames; the analog second hand's
160 ms tick renders ~10 frames/s; a 150 ms focus change renders 9 frames,
i.e. 60 fps.

Theme: `/var/lib/lumatv/theme/` (`theme.json`, `layout.json`, assets) polled
every 2 s and applied live. The files there hold only what the owner changed:
LumaHome merges them over the shipped `payload/theme` files, which `stage_luma`
stages beside the QML as `theme.default.json` and `layout.default.json`
(objects merge key by key, arrays and scalars replace), and `lumatv apply` runs
`tools/theme.py minimize` on them so what was never changed follows an update.
`theme.json` is the one token source (colour roles of both materials in
Lucent's names, type roles, radii, spacing, focus, states, motion, font);
`tools/tokens.py` resolves it for the web CSS (`theme.css` in the web apps'
overlay), for the settings-sheet palette Quick Settings is patched with, and
for `theme/Tokens.js`, which QML in other processes imports by file URL. The
compositor's QML reads it through `kit/Lu.qml`. `layout.grid.rows` is a count
or an array of pixel heights; the shipped layout pins the mock's 222/238/392.

Key codes: the remote's Back arrives as evdev KEY_PREVIOUS (412), not
KEY_BACK (158); injecting 158 does nothing on Home. Qt 5.12's QML
XMLHttpRequest can PUT to file://, which is how the Home popovers save
theme.json (the compositor is not allowed to call Homebrew's exec service).
It goes through the network access manager's queue, though: PUTs of the Home
stub's heartbeat twice landed 5 s late while the timer ticked on time. Where
timing matters LG's own `TextFile` (`import TextFile 1.0`; `name` as a
`file:///` URL, `exists`, `read()`, `write(text)`, all synchronous) is used
instead, as in the stub's heartbeat, mode, escape and confirm files.

Luna from the layer: `Service { appId: LS.appId }` (the compositor's own
identity; a made-up appId gets no reply, silently). Account data:
`accountmanager/getLoginUserData {serviceName:"LGE", subscribe:true}`
(`profileNick`, `iconNick`, `profileBg`, `id`). Launch-point order:
`homelaunchpoints/listLaunchPoints` (subscribe) carries `unmovable`;
`moveLaunchPoint {launchPointId, position}` writes it.

Mini apps: LumaHome's `openMini(spec, fromRect)` grows `MiniFrame.qml` (one
Glass quad) from a card to the centred frame; `kind: "web"` launches
`org.nphil.lumaglass.frame` (appinfo `defaultWindowType: "overlay"`,
`transparent: true`, `visible: false`) with `{url, title, source, ago, mode,
light, rect}`; the app positions its pane at `rect` and leaves the rest clear.
WAM honours `defaultWindowType` from appinfo; overlay surfaces are accepted by
StarfishOverlayView and the compositor's own `handleSystemKeys` closes them on
Back. LumaHome subscribes to `applicationManager/running` to see the app
appear (spinner off) and disappear (frame closes). The app reaches the root
service through `PalmServiceBridge` (`window.webOS` is not present in a WAM
app without the webOS.js library); fonts must live inside the app directory
(file:// access to other directories is blocked). `kind: "qml"` content goes
in `MiniFrame.contentSlot` for native mini apps (weather, HA, calendar).
`appInstallService/dev/install {id, ipkUrl, subscribe}` installs a local ipk
on the rooted set; the frame ipk is built by `tools/build-ipk.sh` into
`payload/frame/` and installed by `install_frame_app` at apply.

Not built yet: widget rearrange/resize (hold on a card, move/snap on the
grid with animated repositioning, span/rows changes written back to
layout.json). The dock's move mode is the pattern to extend: pick up on
hold, ListModel.move so existing delegates animate, persist on drop.

State lives in `/var/lib/lumaglass/`; the CLI is `tools/lumaglass`, the boot
worker is `tools/autostart.sh`. `status` is the source of truth for what is
live.

## Boot: the proven mechanism

The set does not cold-boot from standby. `/proc/cmdline` carries
`snapshot resume=/dev/mmcblk0p54`, boot status reports
`snapshot-resume-done: true`, and the compositor restored from that image
carries the pid and start time it had when the image was made. A "first
start" of the compositor is therefore a memory restore, and systemd
timestamps read on a resumed boot are values baked into the image, not this
boot's.

On a genuine cold boot (`mode=making`), measured:

| event | monotonic |
| :--- | ---: |
| `mount-dynamic-partition.service` | 1.07–1.32 s |
| `ls-hubd` ExecMainStart | 1.32 s |
| `var.mount` (p58 on /var) active | 1.98 s |
| `surface-manager-daemon` ExecMainStart | 2.63 s |
| `sam.service` | 5.28 s |
| `mount-readwrite.service` active | 10.8–13.1 s |
| `kdump.service` ExecMainStart | 12.1–14.3 s |
| image capture (making boot) | ~15 s |
| Home first paint | ~12–18 s |
| Homebrew Channel `init.d` run-parts | ~33 s |

The compositor's first start can never load the modded QML, and the reason is
the filesystem, not ordering. Until `mount-readwrite.service` relocates it,
`/var` does not show the writable ext4 subtree, so `/var/systemd/system/env`
is not a path that exists yet; the daemon's optional `EnvironmentFile` read at
2.63 s is a silent no-op. Proof:

- The env file's atime, set to 2020 before a cold boot, was untouched after it.
- Marker variables planted in `ls-hubd.env`, `configd.env`, `memchute.env`,
  `tvpowerd.env`, `bootd.env` and `sam.env` were absent from every one of
  those processes' `/proc/<pid>/environ`. LG's own env files are inert that
  early too.
- `/var` is a subtree mount (`mountinfo` root `/var` on p58).
  `/var/systemd/system/env/surface-manager.env` and
  `/mnt/lg/cmn_data/var/systemd/system/env/surface-manager.env` are the same
  inode; `/mnt/lg/cmn_data/systemd` does not exist. A copy staged at p58's
  root was not read either, so the early `/var` is the firmware overlay
  (which holds only `lib`), not any view of p58.

Consequences: one compositor restart per boot is structural, ~13 s is the
floor for anything a root mod can do, and every restart after ~13 s picks the
env file up reliably.

### The early hook

Homebrew Channel's run-parts fires at ~33 s, after Home has painted, which
produced the stock-Home-then-blink-then-mod sequence. The mod now triggers
from `kdump.service`: it starts at 12–14 s, reads
`/var/systemd/system/env/kdump.env`, and its script's first act is a bare
`grep`. `persist on` writes `PATH=/var/lib/lumaglass/earlybin:...` into that
env file and a shim named `grep` into `earlybin` that starts the worker once
per boot and then `exec /bin/grep "$@"`. On a set without `crashkernel` on
the kernel command line the script is a no-op that exits 0, and the unit is
`Restart=no`. The shim uses absolute paths throughout (a bare `grep` inside
it would re-enter itself) and launches the worker with a plain PATH. The late
hook stands down when `/tmp/.lumaglass-worker` names a live pid or the boot
completed.

The worker never lets the wake preparation decide whether LumaTV starts. `wake boot-check prepare` only
chooses the app to land on; when it fails (it rolls itself back) the compositor is activated with no
landing. Its `systemctl daemon-reload` is cut off at 6 s and ran past that in the power-loss cold boot of
2026-11-03 20:38, which stopped the worker before activation and left the set on LG's compositor with
LumaTV's mounted. A standby activation that fails because the set was switched on meanwhile gets one more
pass, as an active boot. A failed preparation leaves the wake policy off (`wake status`: `off`, with
`failure.json` kept) until it is staged again.

### Readiness

`systemctl is-active`, the Home process, `NL_HOME_SHOWN` and
`getForegroundAppInfo` all go true ~8 s before anything is composited. The
only honest gate is the panel: `executeOneShot` at 320x180 returns 835 bytes
while blank and >79000 once Home is up. Measured after a restart: content at
+12 s, and asking for Home at +4 s does not delay it. The old sequence slept
14 s before launching Home and 8 s after; both sleeps are gone. The gate is
bounded at 30 s, and on the boot path a timeout is not an error (a boot into
standby never presents).

### Result

Wake from cold, frame-captured every 2 s (`evidence/boot/`): live TV at 12 s
(the image's restored state), black from ~17 s (our restart pre-empts Home's
first paint), modded home fully formed at 33–36 s. Previously ~55–60 s with a
stock-Home phase in between. Whether Home paints before the restart lands is
a race that varies by a second or two per boot.

## Standby: the proven mechanism

After power-off the set enters active standby (RAM alive) within ~3.4 s. A
power-on from there is instant with the mod still on screen. What made it go
cold was `faultmanager`: at power-off it scans
`/tmp/var/log/reports/librdx`, and any report there makes it answer
`responseSuspendRequest` with `ack:false, reason:"rebootToSuspend",
rebootReason:"rebootByFault"`; tvpowerd then reboots ~6 s after entering
standby. Captured on the bus with
`ls-monitor -f com.webos.service.tvpower` written to `/var` (not `/tmp`,
which does not survive the reboot).

Two reports were arming it.

1. The compositor restart makes `com.webos.app.inputcommon` segfault on
   teardown (`QFontDatabase::removeAllApplicationFonts`), landing ~3 s after
   the restart. The sweep meant to remove it never worked: it read names back
   from `ls`, which word-splits on the spaces every name contains and prints
   the 0x02 bytes rdxd uses in place of `/` as `?`; `rm -f` on those names
   was a no-op counted as success. The sweep now uses pathname expansion
   (raw bytes) and runs again once Home is back on the panel.
2. `webos-sddp` (Control4 discovery) segfaults in `SDDPSetDevice` (NULL
   dereference, backtrace `main → start_sddp → SDDPStart → SDDPSetDevice`)
   whenever it reads the interface list and finds a point-to-point interface
   carrying the multicast flag: the Tailscale `tun` (`/32`, no broadcast) on
   the reference set. Proven both ways: with `tailscaled` stopped it starts
   and stays active; with the tunnel present it dies within 3 s of
   `systemctl start`. A stock set has no such interface. At boot it starts at
   ~42 s, before Tailscale's hook (which sleeps 30 s) creates the tunnel, so
   it survives until the first wake re-reads interfaces - which is why it
   first looked like a wake crash. Not this mod's and not LG's for normal
   customers. Its script is gated on the set's own `enableSDDP` setting, so
   the **Instant on** toggle (`standby warm`) turns that off and Quick
   Start+ on, remembering the previous value; `standby cold` restores it and
   turns Quick Start+ off. Removing Tailscale from the set would make the
   SDDP part unnecessary.

   Nothing else on the set is affected by the tunnel: `avahi-daemon`
   (AirPlay/HomeKit), `chromecast-provisioning`, `miracast`, `appcasting`,
   `connman` all active; no faultmanager detections; no crashd or rdxd
   history; the three failed units (`broadcast-configd`, `f2io-on`, `gibbs`)
   are tuner one-shots that fail at 12-18 s, before the tunnel exists.

   Tailscale was removed from the set on 2026-09-14 (node logged out, daemon,
   init.d hook and `/media/developer/tailscale` deleted). Remote terminal
   access goes through the Unraid node's subnet routes instead; nothing on
   the set depends on a tunnel any more, and `enableSDDP` is off only because
   the Instant on toggle sets it.

Measured after both: three power-off / wake cycles of 150 s each with no
reboot, uptime carrying across, the same compositor still running, and Home
from a warm wake painting in 8 s (`evidence/home-after-warm-wake.png`).

Related facts from `tvpowerd`: `com.webos.service.tvpower.instantBootRefreshPeriod`
is 24 (hours) via configd, after which a warm set is refreshed cold once
("Wait for 1 Minute at active standby" then "Go to Cold"). There is a
`warmOwner` / `registerActiveStandbyRequest` protocol
(`{"clientName","timestamp","ack"}` responses) and a learned power-off
history at `/mnt/lg/cmn_data/tvpower/webos_offhistory`; neither needed
touching.

## The Home stub: the proven mechanism

LG's Home (`com.webos.app.home`, a Flutter app) is retired behind a web stub:
`payload/homestub` (same id, `type: web`, `appCategory: ["home"]`,
`visible: false`, a blank page) staged at `/var/lib/lumatv/homestub` and
bind-mounted over `/usr/palm/applications/com.webos.app.home`, so LumaHome is
the only Home. `lumatv homestub on|off|confirm|status`; results and the way
back in `docs/research/homestub-result.md`.

- **Order.** The Home assets bind goes in first and stays under the stub (the
  mount over the parent hides it; unmounting shows it again). Then SAM:
  `blockSystemApp` followed at once by `releaseBlockSystemApp` makes it read
  the directory again in under 0.1 s, with no SAM restart, and
  `blockedSystemAppList/USA.json` is byte-identical afterwards (md5 `26e1a4ce...`
  after dozens of cycles). `getAppInfo` then reports `type: web`. The release is
  retried: an app left blocked cannot launch. Only then is LG's instance
  stopped (`pkill` on `flutter-client -i com.webos.app.home`); with Home in
  front, SAM's `last_input_handler` opens the stub about a second later. Going
  back: unmount, block and release (`type: flutter`), then WebAppMgr's `killApp`
  on the stub page (`closeByAppId` only backgrounds it); SAM opens LG's Home
  about 3 s later.
- **The manifest must not set `handlesRelaunch`.** WAM then logs "handlesRelanch
  : true; Do not call raise()" and the stub stays behind the app the Home key
  was pressed in. Home launches skip memchute's memory check ("launch without
  memory check").
- **`/usr` is a shared mount**, so the bind and the unmount propagate into every
  app jail's mirror of the directory.
- **WebAppMgr is the stub's clock.** A compositor restart kills WAM, systemd
  restarts it, and the stub can only launch once it is back: 3 s on two boots
  and on every restart on an idle set, 56 s (stub at 65 s) on the first boot
  after an install. The fullscreen view is hidden and the container 0x0 while no
  surface exists, so nothing of LumaHome can cover that gap: black until Home
  is up. LumaHome is therefore built when the compositor starts (the container
  loads it for the base container at once), which also gives the guard its beat
  and a load error immediately.
- **WebAppMgr dies with the compositor.** After `systemctl restart
  surface-manager-daemon` (boot: restart at 35-36 s) the old WAM closes about
  2.5-3 s later and systemd starts a new one 0.1-0.3 s after that
  (`webapp-mgr.service`, `Restart=on-failure`; serving at 39 s). A web Home
  launched in between goes with the old one. SAM's own `last_input_handler`
  launches Home at about +3.4 s, into that gap, and loses it too; the tool's
  launch (`relaunch_home`) therefore waits until `webapp-mgr.service`'s
  `ExecMainStartTimestampMonotonic` differs from the value taken before the
  restart and `com.palm.webappmanager/listRunningApps` answers. Before that
  gate a boot left the panel black (the boot-5 failure in the result file),
  because the old rule took "Home running, boot path" for a boot into
  standby; it now needs a power state other than Active.
- **Qt 5.12's `Binding` does not restore a property** when its `when` ends over
  a surface that stays. The container sets the Home surface's opacity to 0 or 1
  explicitly; before that, LG's Home returning under a LumaHome that stepped
  aside stayed at opacity 0, a black screen.
- **Boot.** `autostart.sh --worker` runs `homestub-boot` first: stub in place at
  uptime 15 s in three boots, before the compositor restart (36 s), so Flutter
  Home never ran. It skips the stub when the previous boot never reached
  `boot-ok`, and a boot-time fault or two in a row turns the switch off.
  bootd's own first app is launched at 13.9 s: on a `reboot` that is the last
  input (reason `micomPower` is not a full-home boot), but a power-on that
  opens Home would start Flutter Home about 1 s before the 15 s mount and be
  killed 1.5 s later. `homeAutoLaunch` was left as the owner set it: holding
  it off would make every wake land on the last input.
- **The guard** (`lumatv homestub-watch`, one detached shell, 0.05 % of a
  core: builtin reads, a fifo for the one-second wait, one `grep` every 10 s)
  watches `/tmp/lumatv-beat`, which LumaHome rewrites once a second for as long
  as it lives. A beat unchanged for 5 s (and the file 3 s old) is a fault; the
  guard also acts at once when the compositor restarts without the module (its
  environment lacks the shadow path) and when the container writes `error` in
  place of a beat because LumaHome failed to load. A fault drops the stub
  (mode `lg`), opens Home if the stub was in front, and counts a strike; two in
  a row, or an unfinished boot, switch the stub off. Strikes clear after a
  minute of beats.
- **LumaHome and the tool talk through `/tmp`:** `lumatv-beat` (LumaHome writes),
  `lumatv-homemode` (tool writes: `stub|lg|legacy <token> [trial=<s>]`;
  `lg` steps the layer aside, the token changes with every write), `lumatv-escape`
  (the Home key writes) and `lumatv-confirm` (OK on the trial toast writes).
  The compositor may not run commands, so every action goes through a file the
  guard reads. LumaHome and the key bridge use LG's `TextFile` for all of them,
  synchronously; `XMLHttpRequest` PUTs for the beat were seen to land 5 s late.
- **Home held 650 ms** is watched by the key filter's first handler (the press
  still reaches LG's chain untouched, so a short press behaves as before). At
  650 ms it writes the request, cancels LG's own 2.5 s hold (Screen Zoom) with
  `longPressTimer.setBlockCancel(false); cancel()`, and swallows the repeats
  and the release. With no stub the layer hides itself, since LG's Home is
  already underneath.
- **Trial.** `on` writes `/tmp/.lumatv-homestub-trial` (a deadline in uptime
  seconds); the guard drops the stub when it passes. Only `confirm`, or OK on
  LumaHome's toast, writes `"homestub":true` to `config.json`; a reboot in the
  middle of a trial ends it, since the file is in tmpfs.
- **Do not run `apply` over an SSH command that can time out**: the shell dying
  mid-restart leaves the compositor on whatever it had loaded (a stale module
  that `apply` then calls current). Detach it (`nohup`) and read its output.
- **Boot direct (built 2026-11-03 behind a flag, not run on the TV).** The stub again, under the Rust shell: SAM's Home is the stub from the
  boot worker's first act while LumaTV's compositor is live, so Flutter Home never starts, and a Home hold of 5 s opens LG's.
  `docs/research/boot-direct.md`.

## Tried and dead

- **A LumaHome boot cover for the stub's late launch.** Built and logged on the
  set: the cover state flipped on, and the screen stayed black. The base
  container is 0x0 and invisible, and its parent view invisible too, while no
  surface exists, so a layer inside it cannot draw. Removed. Covering that gap
  needs the layer above the view (track C's compositor).
- **Holding `homeAutoLaunch` off while the stub is in place.** Nothing brought
  Flutter Home back in any test, so there was nothing for it to fix, and it
  changes where every wake lands (bootd opens the last input instead of Home).
- **Baking the mod into the hibernation image.** LG's own remake path is
  `snapshot-boot-manager --remove` then `reboot`; the next boot comes up
  `mode=making` and the kernel writes a fresh image to p54 (250 MB
  partition). Safe — worst case one cold boot, as after a firmware update —
  and exercised several times. It cannot carry the mod: the image is captured
  at ~15 s, before the 13 s floor plus a restart can finish, so the resumed
  compositor is always stock and the asset bind is absent. `snap_list` has no
  Home/sam entry, and `-m` from a live desktop would not fit against ~1.2 GB
  in use.
- **Staging the env file on p58's root** so the early `/var` would see it:
  not read (see above).
- **A unit drop-in or a `/run/systemd/system` unit**: `/etc` and `/usr` are
  read-only overlays; `/run` is writable but volatile and nothing early can
  populate it.
- **Making the restart cheaper**: the 12–16 s from restart to pixels is the
  compositor's cold start plus Home's cold start (`surface-manager.sh` kills
  every client on restart), not padding.
- **Shadowing `controllers/StarfishKeyFilter.qml`** to hook `preProcess`:
  never loaded. A marker in the shadow copy stayed silent across restarts and
  the log's own "Created a StarfishKeyFilterLoader object for /usr/lib/qt5/
  qml/KeyFilters/..." line shows the stock file running every time. Types
  reached through the module's qmldir resolve to `/usr/lib/qt5/qml` whatever
  `QML2_IMPORT_PATH` says; only relative-file references from an already-
  shadowed view (`Qt.createComponent("fullscreen/...")`, a Loader's relative
  `source`) follow the shadow. The configd key-filter list is the hook.
- **Stealing QtQuick activeFocus for the Home layer**: keys never arrive;
  the KeyFilter delivers them to the Wayland window, not through the item
  tree.
- **A multi-row app grid from `home.xml`**: appList is a fixed single row and
  a duplicated item is the same list.
- **Rewriting each app's appinfo/icon for tile plates** (the `tileicons.js`
  hack): worked, but touched app files and needed `sam` restarts; replaced by
  in-memory plate derivation in `Dock.qml`.
- **A 4K UI plane**: KMS accepts 3840x2160 but the OSD stream stays 1080p
  (torn lines), +290 MB RAM, capture breaks; the UHD OSD mode is chosen from
  boot `hwopt`, unreachable without a partition write. The OSD scaler index
  (`/sys/module/lg1k/parameters/gfx_o22_scaler_index`) changes nothing
  visible (0 breaks output).

## The other mod on this set

Starfish (`nphil/starfish`, formerly `lg-webos-mqtt`; still installed on the
set under `/var/lib/tvweb`) bind-mounts its own adblock `/etc/hosts`. That
bind stacks on top of Homebrew Channel's, which is how Homebrew's OS-update
block (four LG hosts pointed at loopback) was silently defeated: the TV found
a 1.7 GB firmware update and put its NSU alert up at boot+34 s for 30 s on
every boot. Starfish 0.34.3 carries those hosts whenever
`/var/luna/preferences/webosbrew_block_updates` exists.

tvweb used to offer QML screen saver replacements. They are gone as of tvweb
0.35.0, and the reason is worth keeping because it applies to any webOS 10
set.

The community mechanism (`webosbrew/custom-screensaver` and forks such as
`aabytt/custom-screensaver-aerial`) is a **single-file** bind:

    mount --bind our.qml /usr/palm/applications/com.webos.app.screensaver/qml/main.qml

Nothing else is touched, so the app SAM scanned is unchanged: no type change,
no SAM restart, and a boot hook re-applying it is enough. It works because on
webOS 5 to 23 the stock screen saver is a QML app - exactly the compatibility
range those projects advertise.

webOS 10 rewrote it in Flutter. The app directory holds `appinfo.json` with
`"type": "flutter"`, `data/flutter_assets/` and `lib/libapp.so`, and there is
no `qml/` directory at all, so aerial's own `apply.sh` exits "Target file does
not exist" here. tvweb's screen savers descended from that lineage, which is
why they never ran: every launch failed "Unable to start engine without AOT
data" and exited 85 ms later, and after a few of those tvpower refused all
further requests (`InvalidStatechangeRequest`) until a power cycle - the set
showed no screen saver at all. That, not the compositor widgets, was the cause
of "the screen saver never kicks in".

Rejected alternatives, all tested on the set:

- Binding our own directory over the whole app with a QML `appinfo.json` does
  run, but only after SAM re-reads the app type, which it caches at its boot
  scan (5s) while any writable filesystem arrives at ~13s. `systemctl restart
  sam` forces it and works, at the cost of a dropped Home and an `eim` crash
  whose report makes faultmanager reboot the set at the next power-off. It
  would also need repeating every boot.
- A copy under `/media/developer/apps` or `/media/cryptofs/apps` with the same
  app id is ignored: SAM dedupes by id and the system copy wins. Confirmed on
  two cold boots.

With stock restored, LG's idle detector `com.webos.service.nop` calls
`power2/turnOnScreenSaver` exactly 180 s after the last input and the stock app
stays up. Idle time is 3 minutes; `screenSaverTime` is not a settingsservice
key on this firmware. A Flutter build of our own would be the only native route
and was not attempted.

LumaTV's own saver does not replace that app. While Home is in front it is on
tvpower's screen saver list and answers the idle question with no, so the app
is never started (`docs/research/saver-own-result.md`; on the set the list, the
no on both lists, the idle timer's question about 176 s after the last key and
every 180 s after that, the key that ends it and a power-key standby (the list
keeps the client and the question after the wake is answered no) work; the
Magic Remote's pointer and the video saver are not yet tried).

## Compositor restart side effects still worth knowing

A bare `systemctl restart surface-manager-daemon` on stock firmware, no mod,
makes stock apps segfault on teardown. `faultmanager`'s policy is in
`/etc/faultmanager/faultconf.json`; `getFaultStatus` lists what it has seen.
The mod removes only the reports its own restart produced.

## Before a package or a hand deploy

`python3 tests/qmlhost/compile_all.py` compiles every QML file of the surface with the set's import layout and exits 1 on any syntax,
type, property or import error (a file that does not compile is a layer that never builds: Settings once left the panel on a bare
wallpaper). `tools/build-ipk.sh` runs it and stops on a failure or a missing PyQt5. A hand deploy (sftp of `payload/compositor/lumatv`
into the shadow module) runs it first, then loads each changed surface in the offline host (`tests/qmlhost/qmlhost.py <file>`), which
compiles and runs it and prints any warning; `LS` must come from `import WebOSCompositorBase 1.0` (`tests/test_ls_import.py`).

## Measuring again

- Cold boot on demand: `snapshot-boot-manager --remove; reboot` (expect the
  "making" splash once). Normal reboot: `reboot`.
- Whether the first compositor is stock: its `/proc/<pid>/environ`
  `QML2_IMPORT_PATH`, and `awk '{print int($22/100)}' /proc/<pid>/stat` for
  its start second.
- Panel timeline during boot: a background loop calling `executeOneShot`
  every 2 s from the early shim, recording `writtenBytes`; frames in
  `evidence/boot/` came from that.
- Standby: `ls-monitor -f com.webos.service.tvpower > /var/lib/lumaglass/x.log`
  before `luna://com.webos.service.tvpower/power2/powerOff
  '{"reason":"remoteKey"}'`; wake with a WoL packet; compare `/proc/uptime`
  before and after. SSH survives active standby.

## Research

`research/home-launcher-replacements.md` surveys other rooted-webOS home
replacements and the input-hook tooling around them; it informed the choice
to mod the compositor and Home in place rather than launch over them.

Prior art worth reading before designing anything new:

- `webosbrew/custom-screensaver` and its fork
  `aabytt/custom-screensaver-aerial` — the single-file bind over
  `qml/main.qml`, and the cleanest example of modding a stock app without
  touching what SAM scanned. Its compatibility stopping at webOS 23 is the
  tell that LG changed the app's runtime.
- `webosbrew/webos-homebrew-channel` — the root elevation, the `init.d`
  runner this mod hooks, the failsafe flag, and the `/etc/hosts` update
  block.
- `openlgtv` — firmware archaeology rather than runtime modding:
  `epk2extract` unpacks LG firmware images (useful for reading a stock app or
  binary from a newer release without owning that set), plus buildroot trees
  and `nsu_emu`, an update-server emulator. Nothing needed for this project,
  but it is where to look when a question is "what changed in firmware X".
