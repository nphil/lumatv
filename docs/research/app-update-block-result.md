# Open apps without updating

Settings › System › Support › Update › **Open apps without updating**. Owner: AppUpdateBlock. Facts are from the set
(LG OLED65C4PUA, webOS 10.2.1, read only on 2026-10-29) and from the blocker's published source unless marked [INFERENCE].
Code: `helper/app/src/openapps.rs`, `tools/lumatv` (`openapps`), `PageSupport.qml`; tests in the same slice.

## What LG does

- LG's app updater (`/usr/sbin/appinstalld`, name `com.webos.appUpdateService`; `pkginstalld` holds the same strings, `AppUpdater`
  and `updateInfo`, for system packages) asks LG's server which installed apps have a newer version and keeps the answer as a list of
  `{id, status, nsu, prohibit, type}`, in memory and in
  `/mnt/lg/cmn_data/var/palm/data/com.webos.appInstallService/updateInfo`. The server is `serverInfo`:
  `http://US.nextlgsdp.com`. When the answer cannot be fetched the updater reads the file (`AUTO_APPUPDATE_CHECK_FAIL`, then
  `APPUPDATE_DETECT` for the cached entry; on the set every 8 minutes or so).
- SAM subscribes to the list (`appUpdateService/statusAll`). `TVExtension::OnUpdateInfoChanged` clears its table
  (`AppInfoManager::reset_all_update_info`) and rebuilds it from every notice; `AppinstalldSubscriber::OnUpdateServiceStatusChanged`
  [INFERENCE: by its name] asks again when the service comes up. An app in the table gets a question at launch, built by `get_payload_for_checking_update`:
  "A new version of the app is available. Launch current version of app or update to latest version." (Update / Launch / Cancel) or
  "You must update this app to launch it. The update may cause stored data to be deleted. Do you want to update this app?" (Yes / No).
  Which entries get which question is decided by the update type stored with the entry (three values are valid, the first gives
  the first question, the other two the second); which of `status` (`N`, `F`) and `type` (`A`, `S`) it is was not read out
  [INFERENCE: the second question is the one the blocker's README calls the forced update].
- The set today: `statusAll` names one entry, `{"nsu":"Y","status":"F","prohibit":"N","id":"com.webos.app.lifeonscreen","type":"S"}`
  (file written 2026-10-25 00:46); `option.appUpdateMode` is `manual`, `support.automaticUpdate` `off` (declutter group
  `autoinstall`); `appinstalld` was not running until the reads above started it.

## The blocker (`org.webosbrew.appupdateblocker` 1.0.0)

Upstream: github.com/dr0dr1dr2dr3/lgappupdateblocker, BSD-3-Clause, copyright 2025 dr0dr1dr2dr3 (the set's `index.js` and `service.js` are
webpack bundles of `frontend/` and `service/`; the Homebrew repo's page for the app links the 1.0.0 release). Its service
(`org.webosbrew.appupdateblocker.service`, elevated to root through the Homebrew Channel's `elevate-service`) has:

| method | does |
|---|---|
| `readUpdateInfo` / `clearUpdateInfo` | reads the file above / writes `[]` into it ("hard restart your TV to apply") |
| `addUpdateDomains` / `removeUpdateDomains` | appends `0.0.0.0 <name>` for each of the 173 `*.lgtvsdp.com` names of `lg_update_domains.txt` to `/etc/hosts` / deletes every line containing `lgtvsdp.com` |
| `checkHostsStatus` | counts `lgtvsdp.com` in `/etc/hosts` |
| `installPersistentScript` / `removePersistentScript` / `checkPersistentScript` / `autoblock` | writes `/var/lib/webosbrew/init.d/appupdateblocker`, which re-adds the domains at boot (the hosts file is rebuilt at boot) |
| `listSSHKeys`, `addSSHKey`, `clearSSHKeys` | `authorized_keys` management; unrelated to updates, not taken |

Nothing of it is active on the set: no `lgtvsdp` line in `/etc/hosts`, no `init.d/appupdateblocker`. The file is the real
mechanism; the hosts block aims at names this firmware does not use (the updater asks `US.nextlgsdp.com`), so it would change
nothing here.

## What LumaTV does instead

The helper keeps the list empty while the setting is on. Nothing of the network is blocked, so the store still updates an app by hand;
nothing of LG's services is called but `appUpdateService/statusAll`, once, after a change of the file.

- Flag file `/var/lib/lumatv/openapps/on` (the data dir); `POST /v1/openapps {"enabled": bool}` sets it and ticks once;
  `GET /v1/openapps` and the topic `openapps` carry `{enabled, entries, ids, state, saved}`.
- A tick every 10 s: list empty → `holding`. List not empty and `appinstalld` running → `waiting`, nothing touched (it may be
  installing an app the owner asked for in the store, and may write its memory back to the file when it ends). List not empty and
  `appinstalld` not running → the list is saved to `/var/lib/lumatv/openapps/updateInfo.saved`, the file is replaced by `[]`
  (written beside it, renamed), and `statusAll` is asked once: the updater starts from the empty file and SAM, seeing the service come
  up, rebuilds its table empty. A file that is not a JSON array is left alone (`unreadable`).
- Off: the flag goes; the saved list goes back into the file if the file is empty and the updater is not running, and is dropped if
  LG wrote a newer list meanwhile. `lumatv revert` does the same (`openapps_restore`).
- Boot: the helper starts at boot and reads the flag; no `init.d` script, no `/etc/hosts` change. While the helper is stopped
  (`lumatv revert`) nothing holds the list.
- Costs: apps are updated only by hand. A list that LG writes after a successful check stays in force until the next tick finds
  the updater idle (10 s at best), so an app launched in that window can still ask. Entries for system packages (the `S` entry
  above) are emptied too, as the blocker does. LG's "Allow Automatic Updates" and `option.appUpdateMode` are not touched.
- Credit: the idea of emptying `updateInfo` is the blocker's; no code of it is used.

## Not proven (the TV check below reads these)

1. That `appinstalld` goes idle and exits on its own (it was not running before the first read), so that `waiting` ends.
2. That it does not write its list back into the file when it exits (the blocker advises a hard restart; if it does, the next tick
   empties the file again and nothing else changes).
3. That an entry as shown above, with a third-party id, gives the "must update" question and that an empty list takes it away
   (step 4 below also shows which `status`/`type` give which question).

## TV check list (a test round owns the TV; the steps that write say so)

1. Read: `lumatv openapps status` → `off`, one entry (`com.webos.app.lifeonscreen`), `saved` false. Settings › System › Support ›
   Update shows the row, off, "Off; LG lists 1 app as waiting for an update".
2. Turn it on in Settings (or `wget -qO- --header 'X-Luma-Client: t' --header 'Content-Type: application/json' --post-data
   '{"enabled":true}' http://127.0.0.1:18731/v1/openapps`). Within a few seconds: state `holding`; `cat` of the file is `[]`;
   `updateInfo.saved` holds the old list; `luna-send -n 1 -f luna://com.webos.appUpdateService/statusAll '{}'` has `apps: []`;
   `/var/log/messages` has no error of `appinstalld`; the helper log says "LG's list emptied; the updater answers with no app".
3. Store: with it on, open the LG Content Store and update one small app by hand: the update installs; while it runs the row says
   "waiting" at most, never empties the list under the install (check `ps | grep appinstalld`).
4. Writes the file, so only when the test round says so. With it off, put an entry for an installed third-party app into the
   file (the shape of the entry in step 1, the app's id, `"type":"A"`, `"status":"F"`; then `"type":"S"` if the first does not ask),
   start the updater (`luna-send -n 1 -f luna://com.webos.appUpdateService/statusAll '{}'` while `appinstalld` is not running) and
   launch the app from Home: LG asks to update. Turn the setting on, wait for `holding`, launch the app again: it opens with no
   question. Turn it off: the entry is back in the file and the app asks again.
5. Reboot with it on: `lumatv openapps status` → `enabled` true after the helper is up. Put the entry of step 4 back into the file:
   within 15 s the state passes `clearing` or `waiting` and returns to `holding`; the app opens.
6. Off restores LG's behaviour: flag gone, saved list back in the file, `lumatv openapps status` → `off` with the entries, no file
   left beside LG's (`updateInfo.lumatv`).
7. Anything wrong: turn it off in Settings, or `lumatv revert` (the list goes back).

## Host tests

`cargo test -p lumatv-helper --lib openapps` (list parsing, the tick against a fake updater, restore, the endpoint, the real updater
read through a fake `luna-send` and a fake `/proc`), `python3 -m unittest tests.test_lumatv_openapps` (the shell status, restore and
the whole tool), `python3 -m unittest tests.test_settings_coverage` (the row, its states, the posts, the dimmed state).
