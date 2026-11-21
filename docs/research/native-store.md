# A native Content Store in LumaTV: feasibility and plan

Owner: StoreResearch, 2026-11-21. Study and one approved install test; nothing built. Facts are from the set (OLED65C4PUA,
webOS 10.2.1, firmware 33.22.52) or from LG's public store bundle, unless marked [INFERENCE]. Nothing was bought or signed
in and no app was launched. Theme work on LG's own store page is in `store-result.md`.

## Verdict

Feasible, and with less LG-private surface than expected. Browse, app pages, search and install/update/remove can all run
through **OS services the helper already reaches as root**: `com.webos.service.sdx/send` makes the authenticated catalogue
calls (sdx attaches the device token, so LumaTV never holds one), and `appInstallService` / `pkgInstall` do the download,
signature check, licence and install, exactly as for LG's store. LumaTV only draws the pages. Paid apps, LG account sign-in
and age checks stay in LG's store, reached by a launch.

## 1. How LG's store gets its catalogue

- `com.webos.app.discovery` 10.0.7 (`/mnt/otncabi/usr/palm/applications/com.webos.app.discovery`) is a 3 KB loader:
  `js/Loader.js` reads `sdx/getHttpHeaderForServiceRequest`, strips the auth headers, and posts the rest to
  `https://<cc>.app.lgwebostv.com/discovery_2024/discovery.html`. The store itself is LG's cloud page: Vue, one 1.77 MB bundle
  `app.<hash>.js.br` (Last-Modified 2026-08-12), a service worker. It changes without a firmware update.
- The page subscribes to the same sdx headers, takes `HOST` (`US.nextlgsdp.com`), and on webOS 25+ (`isNextSdp`) calls
  `POST /auth/10.0.0/authentication` for a bearer token, then plain HTTPS to the endpoints below. Header keys sdx returns
  (values never recorded): `Authorization`, `X-Authentication`, `X-Device-ID`, `X-Device-Model` (`HE_DTV_W24G_AFABATAA`),
  `X-Device-Country` (`US`), `X-Device-Language`, `X-Device-FW-Version`, `X-Device-Eula`, `X-Device-FCK`, profile ids and more.
- sdx maps service names to bases (`/mnt/lg/cmn_data/sdp/sdx/server_addr_version.conf`, version `10.0.0`):
  `sdp_apps` → `apps/10.0.0/`, `sdp_home` → `home/10.0.0/`, `sdp_search` → `search/10.0.0/`, `sdp_apps_resource`,
  `sdp_auth`, all on `<cc>.nextlgsdp.com`. `sdx/send {serviceName, url, methodType: REQ_SSL_GET_METHOD|REQ_SSL_POST_METHOD,
  bodyData}` sends a signed request and returns `{serverResponse: {code, message, response: "<json text>"}}`.

Endpoints, from the bundle's table (`sdp: {key, url, oapUrl, method}`) and checked through `sdx/send` from root on the set:

| Use | Call (via `sdx/send`) | Seen on the set |
|---|---|---|
| Home shelves, banners, categories | `sdp_home` GET `apps/featuredPage` | 200, 45 KB: `bannerList` (8), `headerRecommendation`, `menu.appsCategory` (7: Entertainment 003, Sports 003/006, Music 003/007, Game 002, News & Info 006, Life 004, Education 005), `recommendation.recommendationCategory` (Most Popular, Newly Updated, Premium, a seasonal shelf) |
| Category list, paged | `sdp_apps` GET `app/list?categoryId=&itemCnt=&startIdx=&rankType=`; sub-category as `app/list/003/006?...` | `rankType` 1 Hot, 2 New, 5 All, empty = all three; `itemCnt=200` accepted; `totalAppCount` per rank |
| App page | `sdp_apps` GET `app/app?app_id=<id>` | name, description, `version`, `updateDate`, `sellerName`, `categoryName`, `iconURL`, `appPreviewList` (screenshots), `evaluationAverage`/`Count`, `price`, `bFree`, `needLogin`, `adultYN`, `ageType`, `nsuYN`, `installable`, `appFileList` (`packFileSize`, `unpackFileSize`), `capability`, `dcms` (privacy labels) |
| Related apps | `sdp_apps` GET `app/recommendedapp?app_id=` | not called |
| Updates available | `sdp_apps` POST `app/update` `{appInfo:[{appId, binId, version}]}` → `appUpdateList` | not called (the updater does it daily) |
| Rate an app | `sdp_apps` POST `app/eval_app` | never call |
| Search | `sdp_search` POST `search/retrieval`, body `query=<q>&version=v2&service=universal_search&start_index=1&max_results=N&domain=app` | 200; "weather" → 70 apps, `doc[]`: `app_id`, `title`, `thumbnail`, `price`, `star_score`, `genre` |

Images (`iconURL`, previews, banners) are on `ngfts.tv.wiselg.com` and load without auth (icon fetched from the
workstation: 200, `image/gif`). LG's own store has no search of its own: its search button launches `com.webos.app.voice`
with `activateType: search`; the request shape above is the one `lennylxx/lg-ipk-download` uses and was confirmed here.

Whole US catalogue, measured: 60 `app/list` calls (200 per page, `rankType=5`) in 18 s, **3,269 unique apps** (Entertainment
2,075, Game 618, Life 298, News 200, Education 78), **0 with a non-zero price**, 1 adult-flagged; an index of id, name,
category and icon URL is 0.66 MB of JSON. A full sync is cheap enough to run once a day.

## 2. Install, update, remove

- The bundle installs with `luna://com.webos.appInstallService/install {id, subscribe:false, silence:false,
  target:{deviceId:"INTERNAL_STORAGE"}}`, or, when `com.webos.service.pkgInstall/status` answers, the same shape on
  `pkgInstall/install` plus `wait` (pkginstalld handles dependencies and forwards apps to appInstallService). Before it,
  `queryInstallCapacity {id, size, uncompressedSize, freeupSpace}`; progress from subscribing to `.../status`
  (`status.apps[].details.{simpleStatus: none|download|install, state, progress, currentInstallCount, totalInstallCount}`);
  `cancel {id}`; `remove {id}`.
- An id alone is enough: `appinstalld` asks `sdp_apps app/install?id=` for a signed package URL, downloads through
  `downloadmanager`, installs rights through `com.webos.service.sm/license/apps/installRights`, verifies with
  `pkgverifier/verifyIpk` and `sm/verifyAppSigning`, and reports `app/installdone` (strings of `/usr/sbin/appinstalld`). LG's
  server decides entitlement, so an id LumaTV may not have is refused by LG, not by LumaTV.
- Permissions: `install`, `remove`, `status`, `cancel`, `queryInstallCapacity` are in the private group
  `appinstalld.internal` (`/usr/share/luna-service2/api-permissions.d/com.webos.appinstalld.api.json`); `sdx/send` needs
  `sdx.control` (private). Root `luna-send`, which the helper uses (`helper/app/src/luna.rs`), reaches both: `sdx/send` worked
  in this study and `appInstallService/remove` already runs from the helper and the UI (`autoinstall-result.md`,
  `rustui/ui/src/home/model.rs`). An `install` from root was run in Phase 0 (§6).
- No caller allow-list. Decompiled `AppInstallService::cb_install` (`/usr/sbin/appinstalld`, sha256 `f9591ba1…`, at
  `0x17bec0`) reads the caller with `LSUtils::getCallerId` (application id, else service name) and compares it only with
  `com.webos.service.pkgInstall`, which alone may pass the extra `$install {appFile, allowDowngrade, restoredRequest}`;
  any other caller gets the plain store install, and its id is recorded as `client` in status. `cb_remove`, `cb_cancel`
  and `cb_dev_install` also read the caller but compare it with no app or service name. The gate is the luna ACL group,
  which root meets.
- Updates: `appUpdateService/statusAll` lists pending updates; LumaTV's "Open apps without updating" empties that list
  (`openapps.rs`), so the store module asks `app/update` itself for its Updates page.

## 3. What stays LG-only

- **Paid apps and purchases.** No purchase endpoint is in the store bundle; strings say "App can be installed after
  purchase". The US catalogue has no paid app today. A non-zero `price` or a `billing` error → hand over to LG's store.
- **LG account sign-in** (`needLogin: Y`) and **age verification** (`adultYN: Y`): LG's store launches
  `com.webos.app.membership` (`query: login` / `adultAuth`). LumaTV launches LG's store on that app's page instead:
  `com.webos.app.discovery {query: "category/GAME_APPS/<appId>"}` opens the detail page (`APPUPDATES` and `APPMANAGE` in
  place of the id open LG's Updates and My apps); the parser is in the bundle's launch handler.
- **Ratings** and LG's promotion/voucher pages (`app/eval_app`, `apps/theme`, `apps/voucher`): left out.
- **NSU apps** (`nsuYN: Y`, need a firmware update first): LG's store only; firmware updates are blocked on this set.

## 4. Robustness against LG's changes

Three layers, by how stable they are:

1. **OS services** (`sdx/send`, `appInstallService`, `pkgInstall`, `applicationManager`): firmware-bound, change only with an
   OS update, which this set blocks. `appInstallService` matches the documented webOS OSE API.
2. **SDP REST API** (`apps/10.0.0/...`): versioned by path; the same paths serve every webOS 25+ store page, the daily
   updater and third-party tools. LG ships new paths beside old ones (`9.0.0` for webOS 24, `10.0.0` for 25, both in the
   bundle). Most stable of LG's cloud parts; it can still change shape [INFERENCE].
3. **The cloud page** (`discovery_2024`): changes at will. LumaTV does not depend on it; it is only the fallback.

Plan for breakage:

- **Pins**, as for the ssgw pins: the API version is read from `server_addr_version.conf` (not hard-coded) and must be in a
  known list; each response is parsed into a typed struct of only the fields LumaTV uses (unknown fields ignored, a missing
  required field = broken). One redacted capture per endpoint is the host test fixture.
- **Checked in the daily run**: the catalogue sync itself is the conformance check (featured page, every `app/list` page);
  its result is the `store` topic (`ok | broken <endpoint> <LG code>`). No separate probe traffic.
- **Fallback**: a failed load, a parse error or a `broken` state shows an "Open LG's store" button on the page, and the
  Apps screen's store row opens `com.webos.app.discovery` directly (today's behaviour) until a sync passes again. Install
  errors from appinstalld (`IPK_VERIFY_ERR`, `download failed`, capacity) are shown in LumaTV's words with the same button.
- **Never written to**: nothing of LG's is patched, mounted or blocked; LG's store keeps working beside LumaTV's.

## 5. Architecture (Rust only, inside what exists)

Decisions (Nitin, 2026-11-21): the remote has no Apps key, so the store lives in the **Apps screen's rail**: a "Get apps"
row replaces "LG Content Store" as the primary entry, and "LG Content Store" stays one row below it. **No LG promotion
banners**: the store shows categories, Popular and New only (`recommendationCategory` types 1 and 2 and `app/list`).

- **Helper**: one module, `store.rs`, a plugin off until enabled (its settings page: the switch and "Use LG's store
  instead"). It holds no thread of its own and nothing resident beyond the cached files:
  - `sdx/send` through the existing `luna::once` (4 s, 1 MiB; measured calls 0.3–2.4 s, pages ≤ 200 items);
  - one daily run: featured page plus every `app/list` page (≈ 60 calls, 18 s) to `/var/lib/lumatv/store/catalogue.json`
    (id, name, category, icon URL, flags; 0.66 MB for 3,269 apps), and the `app/update` POST for installed apps;
  - app pages fetched on open (`app/app`), not cached beyond the session;
  - search: substring and word-start match over the cached names in memory (3,269 strings); no index, no server search;
  - images: icon and screenshot files fetched with the existing `fetcher` and kept in a size-capped folder. LG's CDN
    labels every file `image/gif`, but the bytes are PNG or JPEG (sampled 30), which `png` and `jpeg-decoder`, already in
    `rustui/ui`, read. Screenshots are up to 2.6 MB, so only the app page loads them, one at a time;
  - installs: one job at a time; `luna::start` subscription to `appInstallService/install`, closed by the helper on
    `installed`, `removed` or an error (the service keeps the subscription open after `installed`: measured 60 s);
  - routes on the existing helper API and one hub topic `store` (health plus the running job's state and progress).
- **UI** (`rustui/ui/src/home/apps.rs` and the existing kit): "Get apps" is a rail section like the others. Its grid is
  the same app grid, filled from the catalogue: a row of the seven categories, then Popular and New. Selecting a cell
  opens an app page built from existing kit rows and buttons (icon, name, seller, size, version, rating, description,
  screenshots, Install/Open/Update/Remove with progress). The Apps screen's own search covers both installed apps and the
  catalogue. Theme tokens only; no web view, no new framework, no auto-advancing element; the screen dims with the shell.
- **LG's store as hand-off**: the "LG Content Store" row and the app page's "Open in LG's store"
  (`{query: "category/GAME_APPS/<appId>"}`) for paid, sign-in, adult and NSU apps. Once LG's store leaves the screen,
  `app_priority` closes it through its existing SAM close (`close_app`: thawed first, WAM told), as one more reason
  beside memory pressure, so LG's store does not stay resident in `WebAppMgr`.

## 6. Phase 0: install and remove from root (done, 2026-11-21, Nitin present)

App: `com.masterclass.lg` (MasterClass 2.0.0), the smallest free, not-adult, no-sign-in, not-installed app among the
278 Education and News apps checked: 89,284 B packed, 157,470 B unpacked. Called exactly as the helper would, root
`luna-send` to `com.webos.appInstallService`, under the TV lock, nothing launched or shown.

- `queryInstallCapacity {id, size, uncompressedSize}` → `{requiredSize: 349154}` in 0.13 s.
- `install {id, subscribe:true, target:{deviceId:"INTERNAL_STORAGE"}}`, events by seconds from the request:
  0.18 s accepted, statusValues 262–264 (no `state`; the `app/install` URL and rights step [INFERENCE]); 0.84 s
  `ipk download current` 0 % → 1.05 s 17 % → 1.07 s `ipk download complete` 100; `ipk verifying` 1.07–1.09 s;
  `ipk parsing` 1.09–1.14 s; `app closing` 1.14–1.16 s; `installing : installing RO` 1.16–2.20 s; `installing : start /
  installing / service installing / activity installing / post install done` 2.20–2.43 s; **`installed` (statusValue 30)
  at 2.43 s**. Every event carries `details.{state, progress, simpleStatus, client: "com.webos.lunasend-<pid>"}`.
- The launch point appeared at once (`listLaunchPoints`); the log records `NL_APP_INSTALLED {"caller":
  "com.webos.lunasend-…"}`. The first run, whose event stream was not recorded, also installed (17:58:51Z).
- `remove {id, subscribe:true}`: `app closing` → `remove start` → `removing` (activities, jail, RO, service files, ipk,
  data) → **`removed` (statusValue 31) at 0.40 s**; launch point and `/media/cryptofs/apps/usr/palm/applications/<id>`
  gone. Both installs were removed; the set is as before.
- `appinstalld`, `pkginstalld` (started although only `appInstallService` was called) and `LunaDownloadMgr` exited by
  themselves within about 3 minutes of the last call.

Still untested: a large app (download time, `pause`/`cancel` mid-download), an update of an installed app, a
`needLogin: Y` app (most News apps carry it, the set's LG account state decides the outcome), and the error shape of a
paid app.

## 7. Background cost: what runs, what the store needs

Measured on the set with LG's store closed and LumaTV on screen (RSS; CPU as share of one core, 30 s window and average
since start):

| Process | RSS | CPU 30 s / avg | Kind | Native store needs it |
|---|---|---|---|---|
| `sdx` | 10.9 MB | 0.45 % / 0.85 % over 21 h | resident, boot | yes: auth and every catalogue call; also LG Home's server link, and stubbing it broke LG's store (declutter note) |
| `update` (firmware checker) | 6.1 MB | 0 / 0.02 % | resident | no |
| `accountmanager` + account `mojodb` | 7.5 MB | 0 / 0.01 % | resident, system-wide | no (sign-in apps go to LG's store) |
| `com.webos.app.discovery` in `WebAppMgr` | 0 now | – | only while LG's store is open | no |
| `billing` (JS host guest), `sdp-server-notice` | 0 now | – | dynamic; billing's start activity is cancelled by `lgextras` | no |
| `appinstalld`, `pkginstalld`, `LunaDownloadMgr` | 0 now; 5.9 + 5.1 + 7.0 MB while working | 0.24 % / 0.08 % / 0.15 % avg while up | dynamic, exit ≈ 3 min after use | only during an install, update or remove |

LG's own background store jobs, from `/etc/palm/activities`: `com.webos.appUpdateService.check` (every 1 d), `.power`
(every power-on), `.auth` (on sdx device authentication), `.usb`, and `com.webos.service.pkgUpdate` (on
`appUpdateService/statusAll`). Each starts `appinstalld` and asks LG for updates [INFERENCE: from the activity files;
not timed]. With automatic updates already off (`autoinstall` group), they only fill the "update to open" list.

Plan: nothing store-related stays resident beyond `sdx`, which the TV needs anyway.

- The native store starts the install services only for a job; they exit on their own (measured).
- A declutter opt item for the store plugin cancels the four `appUpdateService` activities and the `pkgUpdate` one, the
  same way `lgextras` cancels its activities, re-applied by declutter's boot hook. The helper's daily catalogue run makes
  the one `app/update` call instead, through `sdx`, with no `appinstalld` start unless an update is chosen.
- LG's store is closed after a hand-off (§5).

Before → after, at rest: identical (≈ 24.5 MB of `sdx`, `update` and account processes, none removable for the store).
Per day: LG's check plus one per power-on, each starting ≈ 18 MB of install services for about 3 minutes → one batch of
≈ 61 `sdx` calls (≈ 20 s) and no install services [INFERENCE until the declutter item is measured].

## 8. Plan

| Phase | Work | Effort |
|---|---|---|
| 0. Proof on the set | install/remove from root (§6) | done |
| 1. Helper | `store.rs`: sdx client, typed responses and fixtures, daily run, name search, image files, install job, routes, topic, host tests | 2–3 days |
| 2. UI | "Get apps" rail section, catalogue grid with categories/Popular/New, app page, progress, fallback button | 3–4 days |
| 3. Background | declutter opt item for LG's update activities, `app/update` in the daily run, closing LG's store after a hand-off | 1 day |
| 4. TV acceptance | open time against LG's store (LG's cold first shelf: median 2.36 s), install/update/remove of a large app, fallback with sdx answering errors, process check after a week | 1 day |

About 7–9 days.

## 9. Risks

- LG could add a caller check to `appinstalld` (today `cb_install` has none) in a firmware update [INFERENCE]; firmware
  updates are blocked here, and the fallback covers it.
- LG could reshape `10.0.0` responses or retire the path; typed parsing and the daily run catch it within a day.
- Calls are made with the TV's own identity at a rate no higher than LG's store and updater; a misbehaving loop could get
  the device rate-limited [INFERENCE], so the sync is once a day and capped.
- Region: the catalogue follows `X-Device-Country`; LumaTV does not fake another region.
