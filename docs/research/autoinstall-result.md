# LG's silent app installs: DAZN on 2026-10-25, the route, the block

Owner: NoAutoInstall. Own paths: this note, `payload/declutter/{items,groups}.txt`,
`payload/declutter/dc.py`, `tools/declutter.sh` (the `autoinstall` group, the
`sync` kind, `DECLUTTER_ROOT`), `tests/test_declutter.py`. Facts are from the set
(LG OLED65C4PUA, webOS 10.2.1) read on 2026-10-25 20:20-21:30 EDT and from the
census captures of 02:21-02:58 in `/tmp/lt/dc/` on the workstation, unless marked
[INFERENCE]. The logs of 02:59 themselves were rotated away (the set keeps about
25 minutes of `/var/log/messages`).

## Resume here

- DAZN was not a new install: it was a **factory stub** (`type: stub`, version
  0.0.44, `internalInstallationOnly`, files of 2024-05-16, appinfo of
  2024-10-12 = the set's first setup; the US7 preload list) that LG's **app
  updater** turned into the full app at 02:59:25, two and a half minutes after
  the set went into Active Standby. The same route is open for the four factory
  stubs still on the set (Twitch, Kanto, Highbrow, Tennis TV).
- Nothing on the set stops that route today: LG's visible switch "Allow
  Automatic Updates" (`support.automaticUpdate`) was off when it happened, the
  Home Hub's own "Automatic Update" (`option.appUpdateMode`) is `auto`, the
  homebrew "LG App Update Blocker" is installed but inactive and aims at the
  wrong hosts, and the `lean` profile does not touch the updater.
- The block is in the repository as declutter group `autoinstall` (profile
  `ads`), **not applied on the set**: the step list is at the end. Host tests:
  `python3 -m unittest tests.test_declutter`.
- Apple TV's 00:46 install time is Nitin's own update (Main, 2026-10-25).
- Home: a held OK (or the Menu key) on a dock tile already offers Uninstall
  (`LumaHome.qml` `openTileMenu` → `appMenu` → `askUninstall`), with the same
  question as the library and `appInstallService/remove` behind "Uninstall";
  LG's system apps and the HDMI bookmarks get no Uninstall. Key-path test
  `test_a_held_ok_on_a_dock_tile_offers_uninstall_...` in
  `tests/test_shell_surfaces.py`; scene `06-confirm-uninstall-dock`.

## What happened at 02:59

| when (EDT) | fact | evidence |
|---|---|---|
| 2024-10-12 20:16 | `dazn` arrives as a stub with the set's first setup, with `tv.twitch.tv.starshot.lg`, `karaokesmart` (Kanto), `com.joinhighbrow.highbrow`, `com.deltatre.tennistv.lgapp` (US7 list) | `/var/palm/customization/launchpoints/US7/applist.json`; opkg `Installed-Time` 1728778602-605; `appinfo.json` mtimes |
| until 02:29 | the stub folder is there, SAM does not list it (`listApps`, `listLaunchPoints`): LG's Home and LumaTV never showed it | census `dc/w/dc-census2/stub_apps.txt`, `docs/SURFACES.md` ("Store stubs ... only the first two are in listApps") |
| 02:49:32 | the daily check of the updater is due (`com.webos.appUpdateService.check`, `interval 1d, local`; next start 2026-10-26 02:49:32) | `activitymanager/list` |
| 02:56:47-50 | remote key: Active Standby | census `dc/live/messages_now.txt` |
| 02:59:24 | the installer writes its progress icon for `dazn` (9385 bytes = the stub's 130x130 icon: an update of a package it has, not a fresh install) | `/var/palm/data/com.webos.appInstallService/icon/dazn` |
| 02:59:25 | the full DAZN web app (3.0 MB, `redirect.html` to tv.dazn.com, `installTime` 1792911565) replaces the stub; pkginstalld's wait list is rewritten empty; from here SAM lists it and it is on Home | `installTime`; `/var/palm/data/com.webos.service.pkgInstall/waitHistory` mtime |
| 20:02:19-23 | at the wake, activitymanager launches `com.webos.appUpdateService` (`/usr/bin/run-service /usr/sbin/appinstalld`), `AUTO_APPUPDATE_CHECK_START`, `APPUPDATE_DETECT {"app_id":"dazn","status":"N",...,"type":"A"}`: the same updater, nothing new | `/var/log/messages` (artifact 11069) |
| 20:20:51 | removed by the main agent through `appInstallService/remove`; the updater runs its check again right after | same |

Not the customization sync: SAM's wake-time sync takes LG's country list
(`applications_dosci`, USA, 20 ids) and installs what is missing and not on the
deleted list; `dazn` was never on that list (the backup of `USA/applist.json`,
untouched since 2026-10-02, has no `dazn`; the list the server sent at 20:02:19
has none). Not a launch: nothing launches apps in Active Standby, and
`appinstalld` took the stub's own icon, which it does for an update of a package
it already has.

## The updater

`appinstalld` registers two names from one service file
(`Name=com.webos.appInstallService;com.webos.appUpdateService`, `Type=dynamic`).
Under the second name it is the `AppUpdater`: activities `.check` (daily),
`.power` (every power change) and `.usb`; `check` asks SAM for `listApps` and
`listPackages`, posts `appInfo [{appId, binId, version}]` to LG's `sdp_apps`
`app/update` through `sdx` (it knows `stub` packages by name), gets
`appUpdateList` (`fileURL`, `prohibitYN`, `nsu`) and installs each entry with
`auto` and `silence`, then reports `app/installdone`; the toast texts it carries
include "Application has been installed on the end of Home." (strings of
`/usr/sbin/appinstalld`). It subscribes to `support.automaticUpdate`
(`SettingsObserverImpl::onAutoUpdateSettingsChanged` →
`AppUpdater::onAutoUpdateModeChanged`), the switch LG shows as Support ›
Software Update › Allow Automatic Updates. That switch is `off` on the set and
has been (`com.webos.settings.default` and `system` both `off` in the x4
capture), and the stub was updated all the same. `option.appUpdateMode`
(`auto` | `manual`, hidden in Settings, read by the Home Hub app as "Automatic
Update") is `auto`; `appinstalld` does not name it in its strings [INFERENCE:
LG's server side may read it through the update request].

How the stub could be updated although SAM did not list it is not proven: the
stub had an opkg entry and a folder, and LG's server has known the set's
packages since the first setup (`app/installdone`), so either side could have
named it [INFERENCE].

## Other routes LG has, and their state

- `applicationManager/launch {"autoInstallation": true}` is LG's "open it,
  installing it first" flag, used by nudges (`deeplink_appId`), the Magic
  Remote's app keys (Disney+ is not installed, so its key would install it),
  Live Menu, voice, notifications and ten LG apps (`grep` over the set). Needs
  a launch, so a person or a nudge; `nudge` is masked in `lean`.
- The customization sync, at every boot and wake: `homelaunchpoints` asks `sdx`
  (`sdp_init/services`), SAM writes the list and installs what is missing. With
  `contentminer` masked it still runs (20:02:19: `received_applist_from_server`,
  `start_sync`, `FAIL_WRITING_NEWAPPLIST` on the immutable file, `sync_done`).
  What keeps it from installing is SAM's deleted list, not the immutable file:
  an id LG adds to the country list that is not on the deleted list is
  installed at the next wake. `declutter-result.md` says the sync goes through
  `contentminer`; on this firmware it is `homelaunchpoints` + `sdx`.
- `pkgInstall/install {"id"}` (pkginstalld, system packages and resources),
  callers: SAM, ai-inference-manager, chromecast-provisioning, eplmanager,
  google-home-controller, opapp-manager, panelcontroller, systemprofile, the
  Home Hub app.
- LG's own block list for dynamic services, `/var/preferences/servicemanager/blocked-services.json`
  (read by `/usr/bin/run-service`, today `com.webos.service.signlanguageavatar`),
  cannot stop the updater alone: `run-service` checks the first name of the
  service file, which is the install service the store needs.
- The homebrew "LG App Update Blocker" (`org.webosbrew.appupdateblocker`, in
  the launcher) writes `*.lgtvsdp.com` into `/etc/hosts` and clears the forced
  "update to open" marker. Neither is in place (`/etc/hosts` has no `lgtvsdp`
  line, no `init.d/appupdateblocker`), and the set's servers are
  `US.nextlgsdp.com` and `aic-gfts.tv.wiselg.com`, which its list does not
  name. It is what made the auto-installs look blocked.

## Can it recur today

- DAZN itself: through the updater only if LG's `app/update` answer still
  names it for this set and the updater takes an id it has no package for
  [INFERENCE, the logs of a wake with such an answer would show it]; through
  the sync only if LG adds `dazn` to the USA list (then as a stub).
- Twitch, Kanto, Highbrow, Tennis TV: yes, the same route, the day their
  vendors ship a version newer than the stub's.
- Any id LG adds to the country list: yes, at the next wake, unless it is on
  the deleted list.

## The block: declutter group `autoinstall` (profile `ads`)

| item | what | why |
|---|---|---|
| `setting option.appUpdateMode "manual"` | LG's own automatic-app-update switch, held off; `off` puts `auto` back | the Home Hub's "Automatic Update"; the one LG-side knob for the updater that the set exposes |
| `setting support.automaticUpdate "off"` | LG's "Allow Automatic Updates", recorded and held | the updater subscribes to it; already off, a reset or a menu visit cannot flip it unnoticed |
| `app dazn`, the four stubs | removed through `appInstallService/remove`, their folders and opkg entries kept in `declutter/backup`, their ids on SAM's deleted list and out of the US7 list | no stub, nothing for an update to turn into an app; DAZN on the deleted list in case LG lists it |
| `sync lists` | every id of LG's country lists that SAM does not list as installed goes on the deleted list (`dc.py deleted-missing`) | the wake-time sync has nothing left to add; installed apps are never marked |

Kept: the install service, the store's own installs and updates (the store
calls the install service itself; `sdx` stays), the "update to open" prompt
LG shows for an app with a pending update. Cost: apps update only by hand.
`off --group autoinstall` restores the two settings, the stub folders and the
list files; the lists are then edited again for every other group that is
still on (the `apps` group's two ids), so switching one off does not undo the
other (`tests/test_declutter.py`).

Not done, and why: masking the updater is not possible without masking the
install service (one process, one service file); an LS2 permission cannot be
taken away by a file under `/var`; blocking LG's hosts stops the store too.
Whether `appUpdateMode: manual` is what the updater obeys is unproven until a
wake after the group is on shows the check without an `AUTO_APPUPDATE_START`;
the stub removal and the deleted list do not depend on it.

Removing a dormant stub through the install service is untested on the set
[INFERENCE: opkg knows the package, so the utility should take it]; if a folder
stays, the tool logs `apps still installed after 20 s` and `status` counts the
item as not in target.

## TV step list (next slot, Nitin's approval: it removes four stub folders and sets two LG switches)

1. `python3 /tmp/lt/tv.py put` `tools/declutter.sh`, `payload/declutter/items.txt`,
   `payload/declutter/groups.txt`, `payload/declutter/dc.py` into
   `/var/lib/lumatv/dc-dev/` (same paths).
2. `/var/lib/lumatv/dc-dev/tools/declutter.sh on --group autoinstall` →
   `{"ok":true,"on":"autoinstall"}`; `declutter.sh status` → `autoinstall`
   `on: true`, `inTarget: 8/8`.
3. Read back: `luna-send -n 1 luna://com.webos.settingsservice/getSystemSettings '{"category":"option","keys":["appUpdateMode"]}'`
   → `manual`; `ls /media/cryptofs/apps/usr/palm/applications/` has no
   `tv.twitch.tv.starshot.lg`, `karaokesmart`, `com.joinhighbrow.highbrow`,
   `com.deltatre.tennistv.lgapp`; `deletedSystemAppList.json` names them, `dazn`
   and every not-installed id of the USA list; `lsattr` shows `i` on both
   `applist.json`; `/var/lib/lumatv/declutter/backup/app-*.tgz` has the four.
4. The store still works (Nitin, on the set): LG Content Store opens and lists
   its catalogue; an app page's Install/Update button works (install a small
   free app, then remove it from LumaTV's library: both go through
   `appInstallService`, `APP_INSTALL_REQ` and `APP_REMOVE_REQ` with
   `caller: com.webos.app.discovery` / LumaTV in `/var/log/messages`).
5. The auto path: standby and wake once (remote key). In `/var/log/messages`:
   `AUTO_APPUPDATE_CHECK_START`/`END` with no `AUTO_APPUPDATE_START` and no
   `APP_INSTALL_REQ`; `SAM CUSTOMIZATION_INFO ... sync_done` with no
   `APP_INSTALL_REQ`; `grep -i "AutoUpdateMode\|appUpdateMode" /var/log/messages`
   for the updater's reaction to the switch (not seen yet).
6. One reboot: `declutter.sh status` unchanged (settings and removals persist;
   `boot` re-applies the group).
7. Anything wrong: `declutter.sh off --group autoinstall` (settings, folders
   and lists back), or `touch /var/lib/lumatv/declutter/skip` and reboot.

## Seen on the way (not this slice's)

- `/media/cryptofs/apps/usr/palm/packages/com.lgtvod.us.app` (05:30:38) is
  still there while the app folder is gone, and SAM's `listPackages` lists it
  (version 1.0.19): a leftover of the `apps` group's off/on bisect; harmless,
  `off` of the `apps` group puts the app back over it.
- `declutter-result.md` credits the sync to `contentminer`; the 20:02 wake shows
  `homelaunchpoints` doing the `RequestServerInfo` through `sdx`.
