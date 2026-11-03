# Wake testing and deployment learnings

Append dated entries here. Keep observations, evidence and unresolved questions distinct. Link current state from `../HANDOFF.md`; do not treat historical success as acceptance of a later build.

## 2026-10-31, 19:08 EDT

### Established paths

Testing uses `compositor/tools/tv-gate.sh`, the October 29 r19/r19w Rust controls/capture helpers and the Round 8 receiver baseline. Deployment uses the existing full-package or Rust-bundle installation. The combined-maintenance validator is not an installation prerequisite. Select relevant gate operations: full gates can change sessions, refresh and final power state. The documented standby baseline is 25 seconds.

The owner requires previously successful testing, deployment and recovery paths, not replacement wrappers, installers, transports or test frameworks. Necessary fixes belong in those existing paths. No further HomeLabber access/approval requests. Source publication remains on hold.

### Configuration precedence

Retail configd accepts top-level `volatile:true`, but merges volatile values before Factory values; Factory wins whole-key conflicts, including arrays. Nonvolatile writes update Factory, not the main database. The actual Factory database held stock values for both wake keys, shadowing the temporary policy. Captured ARM code confirms the merge order and that null nonvolatile writes remove the Factory key. The main database must provide verified stock fallback before removing an owned override.

Recovery restores Factory before volatile values. The native bootd constructor/getter retains `/etc/palm/action-condition.json` when the path key is absent. Missing Factory files and foreign values are not equivalent to a safe empty database.

### Native log witness

Bootd opens its log with `O_RDWR|O_CREAT|O_TRUNC` (`0x242`, captured ARM address `0x6b28a`); inode/size growth is not a valid restart witness. The runtime trial instead checks the latest `BOOTD_START` against the trial's monotonic-clock window and requires both owned-policy load records. Reads are capped at the final 1 MiB. Preserve observations before rollback truncates the log again.

Corrected native diagnostic `234ed66a71784b0e8c2133fbb39bebd8` passed both policy-load witnesses with foreground Home unchanged, then restored stock settings and original tools/markers. This proves the runtime trial, not reboot or wake-destination acceptance.

### Snapshot startup and timeout recovery

The established boot notes (`../HANDOFF.md`, “Boot: the proven mechanism”) document hibernation-image restore, including old process and kernel state. Equal boot IDs neither disprove a snapshot resume nor prove a fresh boot. The early worker runs around 12–14 seconds; the earlier panel sequence showed restored Live TV near 12 seconds and Home around 33–36 seconds. A 15-second early policy check is not justified by that sequence. A longer wait alone cannot make an already initialized bootd reread its policy.

Activation `6928bb03053940598f3ed1c93054cde3` failed the boot check: both owned-policy witnesses were false, firstAppId was empty, and foreground was Live TV. The worker restored stock configuration, then the six-second systemctl subprocess deadline expired. Later Home foreground does not prove the missing native-policy acceptance.

A subprocess timeout does not prove the systemd operation stopped or failed. A subsequent non-PTY `systemctl --no-pager show bootd.service` read reported active/running, Result=success, ExecMainStatus=0 and no pending job. The normal boot-status command reported active and boot-done. Do not repeat an unresolved restart or activation. Keep systemctl reads noninteractive; Luna commands use the established PTY transport.

The existing installed `lumatv wake off` command subsequently returned `ok:true`, `phase:off`. Exact installed-tool/selector backup restoration remains separate work. Do not replay the activation packet to accomplish recovery.

### Evidence and remaining acceptance

Read-only captures: `/data/home/tmp/lumatv-wake-readonly-2odth4qv/`, including `post-activation-status.json`, `bootd-state-after-timeout.txt`, `boot-status-after-timeout.json`, `post-reboot-compositor.txt`, `post-reboot-bootd.log` and `existing-wake-off-result.json`.

Attempt events/backups: `/data/home/tmp/lumatv-wake-layer-trial.m8iisQ/wake-maintenance-evidence/`. Durable resume: `/data/home/tmp/lumatv-wake-handoff-20261031.B91cpuWy/RESUME.md`.

Cold/snapshot policy loading, Home/LiveTV/HDMI resume, software-triggered remote reason, Denon following and actual-TV unavailable-app fallback remain unaccepted. The wake task remains open. No new test wrapper is justified by these failures.

## 2026-10-31 — rollback follow-up

Readback after the existing `wake off` confirmed stock configuration and no pending/trial markers. The packet’s existing `restore_tools` operation restored the original autostart/CLI hashes and removed installed `tools/wake.py`. Renderer status remained persistent, settled, live and zero-failure. Evidence: `status-after-existing-off.json` and appended attempt events `existing-off-recovery-result` / `existing-tools-restoration-complete`.

The newer inactive selector, failed report and RAM migration backup remain intact: the exact-marker restore rejects the historical nonempty rollback error. Do not rewrite that evidence to satisfy the check or replay activation to recover. The worker no longer has a pending trial to run.

## 2026-10-31 — selector and explicit-destination contracts

Captured bootd `AbsCondition::runCommand` calls `fread` at `0x21184`, then unconditionally writes NUL over `buffer[bytes_read-1]` at `0x21190`. It expects newline-terminated stdout. The previous selector and its shell-error fallback emitted no newline. The accepted runtime trial’s native event at 5335.179586900 therefore contained `com.webos.app.hom`, not Home. Both policy-load witnesses can pass while the selected destination is invalid.

The existing selector and fallback now both emit a newline. Two consumer-facing regressions in `tests/test_wake.py` failed before the fix (HDMI2 lost its digit; Home lost its final letter), then the full 37-test wake suite passed. Actual-TV CLI smoke via the established r8 upload/PTY commands confirmed `0a` as the final byte for both success and missing-selector fallback. Evidence: `selector-native-output-smoke.json` in the read-only capture directory. Temporary source was removed; no installed policy or setting changed.

Bootd respawn suppresses first launch when a foreground already exists (`doBoot` at `0x5477c`; `doNormalBoot` foreground check at `0x53910`). It still reloads action conditions (`0x54790`). A bootd restart can therefore load the policy while deliberately preserving restored Live TV. `firstAppId` may remain empty after successful initialization; retain it as evidence, not a universal foreground proxy. Reloading and destination acceptance are separate checks.

Normal power reasons do not rule out explicit launches. The earlier native log `native-wake-sao1j7vh/var/log/bootd.log`, lines 1188–1240, records `remoteKey` with Hotkey Netflix, Hotkey-ON and a Cover App payload whose firstApp is Netflix. The same reason earlier produced empty Hotkey. Never force saved Home solely because the reason is `remoteKey`; preserve native Hotkey/OTT/CEC precedence. An existing landing-path correction requires the actual native selected-action discriminator, not a guessed reason whitelist.

## 2026-10-31 — passive wake evidence

The existing `tv-gate.sh woke` operation launches Home before its capture. It proves the compositor can show Home after wake, not that wake chose or preserved Home. Its `tv_gate.py wake` helper uses software `powerOn` with `remoteKey` for Active Standby; that is not a Wake-on-LAN case. Reuse the existing controls, journal and DISPLAY capture helpers without a destination-changing launch when testing resume.

The panel reported refresh due (run 615, last 4377 h, usage 4383.5 h, interval 4 h). The existing refresh guard was used. At 19:30 EDT the owner explicitly waived its wait for the current work. This is not a standing change to the default guard. The superseded background wait only reads and records; it sends no wake command.

The 19:31:15 EDT native transition reported `hdmiCec`. Before standby LG saved Home; this wake retained Last App Home but selected Last Input HDMI2, with Hotkey and OTT empty/OFF. Native full-home exclusion selected Input App and successfully launched `com.webos.app.hdmi2`. Actual foreground and DISPLAY capture showed Apple TV; the compositor remained persistent, settled, live and zero-failure. `getBootStatus` nevertheless reported firstAppId Home and firstAppLaunched true. A nonempty firstAppId can be stale too.

Existing HA Apple TV turn-on/Home commands initially left the TV in Active Standby; the subsequent TV turn-on was followed by the native HDMI-CEC transition. This sequence does not isolate which command caused wake. Service success alone is not physical wake proof. Evidence: `before-intentional-hdmi-bootd.log`, `after-apple-tv-home-bootd.log`, `after-ordinary-wake-bootd.log` and `intentional-hdmi-cec-after-wake.png` in the read-only capture directory. Stock policy and restored tools were in use; this is not acceptance of the changed wake policy.

## 2026-10-31 — recovery and input restoration

Private recovery archive `/data/home/tmp/lumatv-wake-install.ms6eh5iv/installed-package-and-state.tgz` contains the installed app, `/var/lib/lumatv` and `/mnt/lg/cmn_data/lumatv`, including inactive wake evidence. Remote and downloaded SHA256 matched `d1dbb09ac9e8fdb3bc979fa27147335094386504927e02081196a53ef982599f` (205568552 bytes, 5143 entries, mode 0600). The RAM copy was removed after verification. The TV remained Active with `com.beb.plxnative` foreground. This is recovery evidence, not permission to overwrite live mutable state indiscriminately.

The historical `tv-install.sh` has no backup mode and invokes an old full-package/Rust cutover. That cutover refuses the currently mounted compositor and must not be run wholesale. Reuse the historical r8 transfer, developer-app install, hash checks and existing apply with the current package input. Apply also restages helper/config/runtime outside the app directory, so an app-only backup is insufficient. Verify the three changed tools as well as the helper and compositor tree. Preserve failed staging bytes and metadata; neither `wake off` nor exact-marker restore replaces obsolete inactive staging. A separately verified, locked archival-disarm is required before fresh staging; do not weaken ownership or erase historical errors.

Native Input App has a base ID-only launch payload; bootd adds startup metadata. Existing actual-TV evidence proves `applicationManager/launch` with only `id: com.webos.app.hdmi2` reaches HDMI2: `ambient/probe-2026-10-25-1627/slot.sh:49`, `hdmi2-launch.txt` and `hdmi2-foreground.txt`. This permits reuse of the existing input landing path, not invented empty parameters or generalization to Hotkey/OTT payloads. The captured CEC Client Request is truncated at 1055 bytes and cannot supply its full startup metadata.

The failed activation's pre-rollback native launch interval was not captured; the surviving log starts after rollback. Its Live TV origin cannot be reconstructed. Historical stock wake-on-LAN delivery plus simulated Luma evaluation is a source regression, not observed new-policy acceptance. An inherited foreground baseline may use a fresh native initial Respawn record; the tool's own earlier reload marker must not become circular delivery evidence.

The native readiness predicate currently checks active `powerStatus` and `signals.boot-done`; `core-boot-done` was true in captured ready status but no implication between the signals has been established. A manual Home launch concurrent with owned input teardown cannot be perfectly distinguished from Home created by that teardown; neither is evidence for overriding an unrelated foreground or a newer native cycle.

## 2026-10-31 — installed trial and first cold-start failure

The existing full packager compiled 151 QML files with zero failures after installing its required PyQt5 checker dependency. No check was bypassed. Package SHA256 `28f4774a2a59e793a1db1337c7c111398ca370242750a9d841cc85cef95f3f21` was transferred, installed, hash-verified and applied through the historical operations. All three changed tools, helper and 408-file compositor tree matched. Apply took 22 seconds, Marquee (`com.beb.plxnative`) stayed foreground and the persistent renderer was settled with zero failures before the cold test. The exact package is preserved privately at `/data/home/tmp/lumatv-wake-install.ms6eh5iv/current-package.ipk`.

The five-file obsolete inactive wake directory was checked under both existing locks and atomically renamed intact to `/mnt/lg/cmn_data/lumatv/wake-archive-46a404c6723042deb67505e53c1ec217`. Every byte/hash, mode, ownership and link-count check passed; old failed report, failure and snapshot remain archived. The fresh canonical activation was created by the existing `wake on`, not by restoring or editing those old markers. Runtime trial passed both native load witnesses and preserved Home. Activation `3d6a9d68128d43b7a76cfead6fba35c8` is pending for Home, not committed.

One actual plain `reboot` returned to reported Home but produced no boot report after 143 seconds. Persistent compositor status was live/mounted, zero failures, unsettled true. Missing report is not proof that the hook never ran: prepare can reject before its first report write. Installed tool hashes still matched, persistence remained true, maintenance veto was absent and the HBC link resolved to the current app script. The marker held dead PID 3638, no boot-pending marker remained and the RAM collector directory was absent; those facts alone do not establish a stale-marker failure. No shim or marker guard was changed.

The first available native log, `cold-02-bootd.log` in the private installation directory, begins `PERFORMANCE(  9.865216160) : Boot) BOOTD_START`. Native startup initially read stock policy because configd reported actionConditionConfPath missing at 10.003 seconds. This confirms why the existing worker must wait and reload the owned policy. The `native_cycle` parser did not accept spaces after the opening parenthesis, unlike the existing load-witness parser. A captured padded timestamp is rejected even within the permitted age window; fix that existing parser before another actual cold test. Do not infer that it is the only startup failure.

All 55 cold-log samples and `before-startup-worker-trace-bootd.log` were preserved before one intentionally mutating bounded shell-trace replay of the installed worker. That replay is diagnosis, not cold acceptance: at uptime 490 seconds it reached prepare, which returned native wake cycle is stale; apply succeeded in 17 seconds, boot-ok succeeded and finalize rejected the absent report. Its now-old cycle cannot establish that the padding bug caused this later rejection. `installed-worker-trace.txt` is private evidence. No acceptance or commit may be based on this manual replay.

## 2026-10-31 — padded-time cold result and native inputcommon delivery

Ownership compares staged selector bytes with the executing wake backend. Updating only APP/wake.py makes stage, boot-check and even the new off refuse ownership. Use the old matching backend’s off first, then the existing locked, hash-pinned inactive-directory archival before the new package/stage/trial/persist. Off removes pending/trial but retains snapshot/policy/selector and any report. No guard was weakened. The retired three-file stage is intact at `/mnt/lg/cmn_data/lumatv/wake-archive-4c6ad6ee0f2d49dfa2b1885c33afe751`; its exact manifest and result are private.

Padded-timestamp package `79e96011bce6898f22ce9ab1f60aa04801bc9e74b22df32d932a1e7d2bba2b18` compiled all 151 QML files and matched all three installed tools, helper and 408-file compositor tree. Apply took 17 seconds. New activation7ad passed runtime trial. Its actual cold invocation1a71 reached prepare, loaded both policy consumers and selected saved Home, but refused foreground-before HDMI2 with native delivery provenance unavailable. `firstAppId` was empty. Later worker completion restored persistent/live/settled renderer status with zero failures; the observer’s earlier live-false sample was transient, not the final state. Activation remains unaccepted/uncommitted.

Initial stock-cycle evidence is `padding-cold-05-bootd.log`: Last Input HDMI2 at lines117–118; applicationManager launch at299, request300, successful response305, AbsAction success306 and immediately next Setfirstaction InputApp/`${Last Input App}` at307; Respawn HDMI2 at354. The launch ID is `com.webos.app.inputcommon`, with a complete direct `params.changeRunningAppId` HDMI2 inside otherwise truncated metadata. Captured ARM convertInputcommon at0x2f41c checks first-launch/snapshot, copies the resolved ID into changeRunningAppId at0x2f4c4, then writes outer inputcommon at0x2f500. This is a known native input wrapper, not an unknown app choice.

Normalize only this corroborated plain Input App launch: matching prior same-cycle Last Input, valid direct input target, exact request/response/action success and immediate following native first-action placeholder. Preserve the ID-only payload already proven on this TV; never fabricate missing params or return raw inputcommon for an invalid wrapper. Reject observed malformed, duplicate, conflicting, escaped or reordered target fields; newest successful native choice wins. This mapping does not apply to Hotkey/OTT and does not weaken foreground, ownership or newer-cycle checks. For truncated logs the unseen suffix is not validated; the constrained native producer and captured conversion supply that provenance.

Source a12 normalizes that actual captured cycle to HDMI2 and its captured fresh Luma evaluations to saved Home; 61 tests and both shell checks passed. This is captured-log smoke, not another actual-TV pass. A new matching activation and real cold check are still required.

## 2026-10-31 — accepted wake and next task

Wake root task is complete: activation `c413b8707ca44e47ac788a3296573221` passed cold Home invocation `b370dc321fda4bff9227a16b2d043d82`; the two-minute healthy window cleared naturally, the existing wake commit succeeded with enabled true and pending null, and postcommit cold invocation `6ecd5f05acbb47898f067be9aafbe2e2` also passed Home without a manual postboot Home launch. Six warm cases passed: `warm-home-wol`, `warm-livetv-wol`, `warm-hdmi2-wol`, `warm-home-remotekey`, `warm-livetv-remotekey`, and `warm-plex-wol` (Marquee, native legacy ID `com.beb.plxnative`). Each used real native standby for 25 seconds, fresh HA AVR-off evidence and isolated WOL or software powerOn with reason remoteKey; native bootd/LumaResumeApp, actual foreground and DISPLAY, closed CoverUI, persistent/live/settled compositor with zero failures, external_arc and fresh HA AVR-on/input were observed. Software remoteKey is not physical-button proof; the LiveTV screenshot was black and does not prove visible channel content. Evidence and screenshots: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/`. Source publication remains on hold.

Separate CEC tests: sole HA `remote.turn_on` did not wake; sole Apple TV `remote.send_command(home)` woke Home standby to HDMI2 with actual native `hdmiCec` and AVR AppleTV/MPLAY on, without TV powerOn or a Home override. Native receiver MPLAY is the configured Apple TV, corroborated by fresh HA Apple TV state and prior r10/r11 logs; retain the original false raw/display-name comparison and `receiver-source-corroboration.json`. The direct CoverUI fallback chain passed from LiveTV: firstApp `{}`, secondApp `{}`, fallback absent ghost ID and finalFallback Home resulted in closed status and actual Home/DISPLAY; inventory confirmed ghost absent and Marquee (`com.beb.plxnative`) present. This does not prove a missing-saved-app wake. Physical Netflix Hotkey/OTT wake remains unaccepted: the historical Netflix log was copied from a prior boot and its stimulus is unknown, with no proven safe software equivalent. Native input/Hotkey unit contracts were reviewed, not accepted as hardware-button tests. Never write native intent files.

Keyboard acceptance passed on the actual TV without source changes or a build. Existing `lumatv keyboard luma` enabled the persistent `/var/lib/lumatv/.keyboard-luma` flag; Rust keyboard capability was already active. Marquee (`com.beb.plxnative`, its legacy ID) Bazarr address and API-key fields passed typing, Backspace, hide, cancel, reopen and Done; LG Search (`com.webos.app.voice`, `launchMode:vkb`) independently passed typing and Backspace. This is not an FPS or latency benchmark. Final `keyboard-final-state.json` and `keyboard-final-home.png` show TV Active, native Home, IME null and keyboard luma; own counts activations 7, shown 7, texts 6, keys 6, nav 110, failed 0. The compositor was persistent, live, mounted and settled with zero failures; wake enabled/pending null, external_arc and fresh Denon on TV input. TV and AVR were left on. Evidence: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/`.

For future evidence, `tv_gate.wol` is isolated WOL; `tv_gate.wake` may use software powerOn. Do not launch Home before destination capture. Snapshot boot IDs can repeat: use a unique invocation token. The compositor running marker clears after two healthy minutes, not at boot. Parse compositor control JSON stdout separately from the known wrong-ELF loader warning on stderr. Never record raw shell or IME surrounding text or API keys.


## 2026-10-31 — Rust keyboard acceptance

Final Bazarr connection showed `https://bazarr.nateshome.net`, the saved key masked, Connected, Bazarr 1.6.2, Sonarr 4.0.20.3014 and Radarr 6.4.4.10685. No API key was read, logged or changed. Evidence includes `keyboard-bazarr-final-connection.png`, `keyboard-address-restored.png`, `keyboard-bazarr-key-two-bullets.png`, `keyboard-bazarr-key-one-bullet.png`, `keyboard-bazarr-key-reopened-empty.png`, `keyboard-lg-search-two-g.png`, `keyboard-lg-search-one-g.png` and sanitized key JSON files.

For Marquee, virtual Back hides the keyboard but leaves the edit active; app Back cancels it. Require IME null before reopening with OK, which otherwise commits the draft. API-key drafts reopen empty: a masked two-character draft, one Backspace, virtual Back and app Back reopened empty; empty Done does not write. Nonempty Done persists: never test it on changed real credentials. Normal keyboard navigation uses existing `ctl remote CODE tap`; `ctl key` targets the app and bypasses IME. Actual RIGHT moved focus 0→2 and g→h. LG Search ignored app Back/Home for closing its overlay; existing `closeByAppId` for `com.webos.app.voice` closed it. `own_hide` already emits input_panel_state 0 and intentionally leaves the field active; SDL dismissal is not an established defect. Parse control JSON stdout separately from the known wrong-ELF warning on stderr; never dump raw shell, IME surrounding text or API keys.


## 2026-10-31 — task reconciliation

Both keyboard tasks and the render-only task are complete. Review-only proposals: `docs/mocks/review-toast.html` and `docs/mocks/review-volume.html`. Nitin’s review is next; neither UI change is installed or implemented. Single-toast queueing, notification exclusion and runtime audio-codec selection remain pending review and implementation. The original broader keyboard task is complete after its native-app acceptance blocker was cleared. Actual visual remote input was accepted; FPS and latency were not benchmarked. Inventory: 39/94 done, 0 open and 55 blocked.


## 2026-10-31 — render-only proposals

The proposals reuse existing Lucent CSS, Manrope and audio assets. Seven 1920×1080 scale-1 renders were inspected in Cody’s visible shared Chromium: `shots/review-toast-dark.png`, `shots/review-toast-light.png`, `shots/review-audio-atmos-dark.png`, `shots/review-audio-dts-dark.png`, `shots/review-audio-dtsx-bright.png`, `shots/review-audio-dolby-light.png` and `shots/review-audio-stereo-dark.png`. Toast proposal is approximately 320×65 px with 24 px text and one bare check, versus the old 520–760×96 px and 27 px title with a 64 px tile. Audio uses original PNG glyphs left of the number and track in one floating group at the existing bottom position. Light/dark and 200% text showed no overlap or overflow; fonts loaded, browser errors were absent and conservative ink-contrast bounds exceeded 7. Evidence: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/review-render-proof.json`. These are illustrations, not TV acceptance or behavior changes.


## 2026-11-01 — native-only boot activation

The retired apply path restarted the compositor after binding the supervisor; native-only boot omitted activation. Binding does not replace an already-running stock process. The boot worker now activates native with service-scoped readiness. Confirmed Active Standby bypasses foreground-dependent wake prepare/finalize and skips landing; exact successful power readback is required again before restart or recovery. Active and unknown states retain wake-intent preparation. Eight startup regressions pass; corrected reboot followed by suspend wake produced native readiness and actual Settings render. LG snapshot restores can reuse boot IDs, and a local mutex mirror can outlive the remote lock after reboot: verify the remote owner independently.

LG picture replies can omit mode, range or input dimensions. Omission is not a context change. Keep confirmed context identity separate from the exact native dimension forwarded with writes; require a current explicit input for reset. Real context changes still clear old controls. The focused regression failed before correction and all eleven Sio tests passed afterward.

For the established native-bundle deploy inside Cody, set PATH=/data/home/.local/bin:/data/home/.cargo/bin:$PATH and LUMATV_FEATURES=ui. Docker exec does not inherit the interactive build PATH. Keep the accepted rollback archive off the TV and remove the transfer stage after verified activation.

Native picture dimensions are key-specific. The observed backlight description names input and pictureMode; its value reply supplies those exact dimensions. A pictureMode reply instead supplies dynamicRange and input. Forwarding that unrelated dimension with a backlight77-to78 write returned the native DB error and left77 unchanged; the backlight-native dimension was accepted and read back77. Keep the key-owned native context rather than borrowing the last category reply.

Native Settings acceptance uses the established gate’s `ctl remote CODE tap` route. `ctl key` sends to the focused app and does not exercise native shell routing. Host preference smoke must seed `theme/theme.json` as well as any `--theme` override; the override alone is not saved state. Scheduled snapshots execute before rendering overdue inputs, so allow a settled draw and check the saved value.

Glass-strength changes preserve Settings control instances and patch slider fill/text in place. Rebuilding sibling switches on each preference write restarts their transitions. Persist each change immediately, restyle the current panes, and defer other-surface rebuilds until350ms quiet.

## 2026-11-01, Settings card geometry and wallpaper ownership

Settings card bases and labels are not children of the focus plate. Applying card scale/lift to the plate alone misaligns its border. Keep its spatial transform neutral and animate highlight opacity; compare flattened outline/base bounds during and after focus at100/200%, including horizontal scrolling, not only pointer rectangles.

The Wallpaper editor is the existing native Home popover, with live preview and automatic save on close. Settings waits for the established Home foreground acknowledgement before hiding itself and opening the editor. Only intentional dismissal returns to Home & look; forced app/profile cleanup clears that return while retaining save-on-close.

Non-Rose wallpaper catalog entries use explicitly authored dark/light three-stop palettes. Rose retains original pixels. Existing saved/custom triplets are not silently migrated; selecting a family applies its current catalog palette. Native Home host captures need a completed frame after initial feeds;8500ms wait/5500ms capture produced the recorded representative previews. Host checks must pass a private data directory rather than the local /var/lib/lumatv default.

Most Settings, toast and volume surfaces already use Pane decoration. Solid picker/help/confirmation/keyboard paths and backdrop-free Liquid panes remain exceptions; renaming Glass type does not certify full material/depth coverage.

Native `ctl shell hide settings` is not a supported close operation. Use remote Back and verify `ui.settings.open=false` before sending Home navigation; otherwise those arrows/OK still operate the current Picture controls. Final readback for the installed5062 check was SDR/default/filmMaker/backlight68.

The existing tv-lock renew writes remote acquired time and local ACQUIRED/VERIFIED separately; the observed renewed mirror differed by1s. Verify own token, lane, TV, session and exact expiry, with remote-acquired ≤ local-acquired ≤ verified, rather than imposing timestamp equality. No lock protocol or tool was changed.
