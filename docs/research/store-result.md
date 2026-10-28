# LG Content Store: what it is, what a theme can reach, what it costs, what the TV round checks

Owner: StoreTheme. Facts are from the set (LG OLED65C4PUA, webOS 10.2.1, firmware 33.22.52) or from its firmware snapshot unless marked
[INFERENCE]. "Host" is this workstation (headless Chromium 156), never the set. Sections 1 to 8 are from before the set was used; section 9 is the first TV
round (2026-10-28): the store opened under the theme and its open time measured, and the look failed on focused tiles, so the theme is off.

## Resume here

- Built, host-proven, **off by default**: `lumatv store on [parts]|off|status` (`tools/store.sh`, `tools/store_theme.py`, `payload/store/`).
  Parts: `look` (the skin), `motion` (no transition longer than 180 ms), `lazy` (lazy pictures, off unless named, see 6). `on` alone is
  look and motion. `off`, `revert` and a reboot give LG's store back exactly. Registry id `store`; `apply` mounts it again at boot while on.
- Next on the set: the colour of the words on LG's focused plate and the header's background (section 9, step 2); the skin's selectors are Sandstone's by
  inference from one capture, the page is LG's cloud page (not in the firmware snapshot), and WAM's inspector does not listen on this firmware, so its markup cannot be read from the set.
- Host proof: `python3 -m unittest tests.test_store_theme tests.test_store_stage tests.test_store_dump`, previews and audit in
  `docs/mocks/store/` (`node render.mjs`, `node bench.mjs`).

## 1. What the store is

`com.webos.app.discovery`, "Apps", version 10.0.7, from `uiinv/listApps.json` (the snapshot holds only its appinfo: its files are not in
`uiinv/apps`, which covers `/usr/palm/applications`):

| appinfo | value | meaning here |
|---|---|---|
| `type`, `main` | web, `index.html` | a WAM web app |
| `folderPath` | `/mnt/otncabi/usr/palm/applications/com.webos.app.discovery` | the firmware's `otncabi` partition: a read-only squashfs (`/dev/dm-5`, 11.0 MB used, `census1/dc-census/mountinfo.txt`) holding the browser (10.2.0), the store, membership (2.2.5) and adapp; not `/usr/palm/applications` |
| `defaultWindowType`, `transparent` | card, false | a full-screen opaque card (the Settings sub-apps are transparent overlays) |
| `resolution` | 1920x1080, 1280x720 | laid out at 1080p CSS pixels (LG: UHD is not supported for web apps) |
| `handlesRelaunch` | false | launching it again while it runs just raises it full screen (LG's appinfo reference) |
| `closeOnBackground` | false | stays resident after Home until LG's memory manager closes it |
| `supportQuickStart` | true | declared by 32 of 149 apps, among them LG Channels, which LG's preload manager does preload |
| `spinnerOnLaunch`, `checkUpdateOnLaunch`, `requiredEULA` generalTerms, `hasAccountService` | true, true, yes, true | LG's spinner until the first frame, an update check, a terms check and an account service at launch |
| `vendorExtension.allowCrossDomain`, `enableCBSPolicy` | true, true | cross-origin requests allowed; what CBS policy does is not established |
| `inspectable` | false | WAM's inspector may not list the page: its source lists system apps only when `/var/luna/preferences/debug_system_apps` exists, read at WAM's start [INFERENCE: the source is the open-source WAM, not LG's build]; creating that file would need a WAM restart, which nothing here does |

How it loads (`docs/research/surfaces/AppsHomeMediaAI.json`, confidence high, read from `index.html` and `js/Loader.js` on the set):
the app is a **thin loader that posts the device's info to LG's cloud page**, `https://<env>.app.lgwebostv.com/discovery_2024/discovery.html`
with `page=store`, and the view then shows that page: "server-rendered store, categories, featured apps. Offline: `network_error.html`.
Content is cloud-driven." Its sibling `com.webos.app.premium` is built the same way. So the store's DOM, CSS and scripts are LG's
server's, change without a firmware update, and are not in this repository. What is known of it is one capture of the home page
(`lgtv/caps/store_crunchyroll.jpg`, 1280x720): "Apps" title with four icon buttons (search, account, more, close), a tab strip
(Featured, Entertainment, Sports, Music, Game, News & Info, ...) with an underline, a banner carousel with arrows, shelves of 16:9
tiles with app icon, name, category, price and a download icon. It reads as Enact Sandstone ([INFERENCE]: the focus is Sandstone's light
rounded pill, the type and the header are Sandstone's).

Launch parameters seen: `{query: "category/THEME/20411"}` (the New key), `{query: "category/GAME_APPS/APPUPDATES", appReturn: {...}}`
(Settings, software update; LG's Settings bundle, `lgtv/research/lgsettings/main.js`), the Apps tile and the Apps remote key.

What it needs from LG at open (measured on the set, 2026-10-25, `declutter-result.md`): with `sdx` stubbed the store shows "Service is
temporarily unavailable"; with `sdx` kept it loads its catalogue. Hosts the store depends on (community reports gathered in `lgtv/tmp-research/endpoints.md`, not measured here):
`<cc>.nextlgsdp.com` and `<cc>.lgtvsdp.com` (catalogue, installs, the clock), `ngfts.lge.com` (thumbnails), `wiselg.com` (reported: blocking it
gives error E5.48.XV). Installs go through `appInstallService` (`APP_INSTALL_REQ`, caller `com.webos.app.discovery`).

## 2. Where a theme can be injected

WAM runs `webOSUserScripts/userScript.js` from an app's own directory in every document its view loads, before the page's scripts, **in
frames and pages on other origins too** (WAM `WebPageBase::setCustomUserScript`, path from `WebAppManagerConfig` `USER_SCRIPT_PATH`, default
`webOSUserScripts/userScript.js`; documented by webosbrew, whose youtube-webos mod runs in a remote page this way). `webapp-mgr.sh` does not
set `USER_SCRIPT_PATH`. That is the one injection point; the rest follow from it:

- **The directory is read-only and on `otncabi`**, so the existing web theme's mount (`webglass_set`: an overlay at
  `/usr/palm/applications/<app>`) does not reach it. `tools/store.sh` mounts an overlay (lower layer: LG's directory as SAM names it; upper
  layer: `webOSUserScripts/userScript.js` and nothing else) over `/mnt/otncabi/usr/palm/applications/com.webos.app.discovery`.
- **The page is on a cloud origin**, so the settings theme's `<link href="webOSUserScripts/lumatv.css">` (relative to the page) and its
  font beside the stylesheet (a page on https cannot load file://) cannot work. The store's script carries everything: one constructable
  stylesheet (adopted by the document, which a page's content security policy for inline `<style>` does not govern; host-proven with a
  CSP that forbids inline styles) and the one font as data (Manrope cut to weights 400-700 and Latin, Greek and Cyrillic: 39 KB,
  `generators/gen-store-font.py`). The script is 76 KB; host run time of its evaluation median 1.6 ms, p90 2.8 ms (the set's CPU is slower).
- **The script runs before `<html>` exists**; it needs no root, and the light material is chosen at build time, not by an attribute.
- **Frames**: the top document, frames of the app's own files and of `lgwebostv.com` are themed; an anonymous frame (a trailer, an ad)
  is not. Whether the cloud page replaces the loader's document or sits in a frame of it is not established; the script covers both.
- Not used: editing LG's `index.html` or `Loader.js` in the upper layer (that changes what the app loads), any interception of the cloud
  page, a Blink flag (WAM's switches are set by `webapp-mgr.sh`, which this package does not touch).
- **Unproven on the set**: that WAM runs the script from an overlay over `/mnt/otncabi`. WAM does read what the tool mounts over an
  app's directory (the Home stub's page is served from one: `homestub-result.md`), so the premise holds for `/usr/palm/applications`; the
  settings theme, built the same way, is listed "not run on the set yet". Step 1 of the TV round is exactly that.

## 3. Why it is slow to open and to scroll

Measured on this set or in LG's own files:

- **No open time of the store exists.** The memory slot launched it among others and saw it in the foreground "within 18 s"
  (`memory-result.md`, a poll limit, not a measurement).
- **Cold web cards are page-in heavy on this set.** The system browser's first launch after a reboot: about 1,400 major faults in the
  compositor around the launch, first paints 400-1,700 ms, a 116.8 ms gap when Home came back (`perf-tweaks.md`, baseline and "Root
  causes"); the browser card ran at 6.7-12.3 fps while loading. The store is another WAM card on the same 2 GB set.
- **WAM's own settings** (`webapp-mgr.sh`, the census): HTTP disk cache 50 MB for all web apps, wiped at every WAM start once it holds more
  than 3,500 files; a backgrounded page is purged after 10-15 s; decoded images are budgeted at 96 MB and the GPU at 250 MB; two raster
  threads and the GPU in the browser process; `LocalResourceCodeCache` is disabled (local scripts are compiled at every start); a
  suspend delay of 250 ms. `ENABLE_LAUNCH_OPTIMIZATION=1` is set; what it does is not established here.
- **LG's Sandstone**, counted in the Settings bundle's compiled CSS (1.1 MB, the same component set): 27 transition declarations, 100-400 ms
  (14 at 200 ms), almost all `transform` and `opacity`; **382 `box-shadow` declarations and 76 `will-change` declarations** (49 naming `box-shadow`);
  no `backdrop-filter`, no blur. A focus move in it is a pseudo-element fill plus a 200 ms transform with a shadow. LG's scroller also
  animates in script, which no stylesheet reaches.

[INFERENCE], to be settled by `store_dump.py` and the open-time runs below:

- The open is a chain of serial phases: SAM launch and the memory check, the loader's calls for device info (`sdx` among them), DNS and TLS
  to `*.lgwebostv.com`, the page and its bundle, the catalogue's JSON (`*.nextlgsdp.com`), then the pictures (`ngfts.lge.com`). The network
  and the server own most of it; a skin cannot shorten them.
- The pictures: if the CDN sends 1920x1080 banners and 640x360 tiles, a home page of 6 banners, 30 tiles and 30 icons decodes to about
  81 MiB (7.9 MiB per banner, 0.9 per tile, 0.25 per icon) against the 96 MB budget, so the shelves would re-decode as they scroll. The
  tool reports the real `decodedMB` and the pictures shown at under two thirds of their size.

## 4. What is possible and what is not

| Wanted | Possible? | Evidence |
|---|---|---|
| A Lucent skin on the store | yes, as far as the selectors meet LG's markup | WAM user script in every document and origin (section 2); host: 20 renders pass the audit |
| Without touching LG's files, reversibly | yes | overlay mount; `off` unmounts, 26 tests on the tool and the verb, with a fake overlay |
| Shorter transitions | yes (transitions and `Element.animate`); not the script-driven scroll | host: 600 ms transition ends in 218 ms; `transitionend` still fires |
| Cheaper focus | yes | the sheet replaces the shadow, `will-change` and fill of a focus move with a ring and a wash; host numbers in section 6 |
| Keep the store alive after leaving it | it already is (`closeOnBackground` false) until LG's memory manager closes it; making it permanent through `keepAliveApps` is not available | the list is configd's `com.webos.applicationManager` (the store is not in it); read at SAM's start [INFERENCE]; no shipped tool may restart SAM (`tests/test_tool_rules.py`) |
| Preload it hidden | possible in principle, not shipped | LG preloads LG Channels, a cloud-driven WAM web app that declares the same `supportQuickStart` and `allowCrossDomain`; the store has no entry in `supportAppList` (census `preload.json`); LG Channels' preloaded renderer holds 12 MB PSS and 21 MB in swap (`memory-result.md`), but it also sets `useAppHibernation` and the store does not [INFERENCE: that is part of why it is small]; whether the manager accepts a new id is unverified. Experiment E1 below |
| Make the cloud page faster (network, server, bundle) | no | not on the set |
| Smaller pictures | no | the sizes come from LG's CDN; rewriting what the page requests would change what it loads |
| Change what is sold, installed, priced, signed in | never | the script and sheet make no request, click, or text change; a test fails any rule that hides, moves or adds anything |
| Stay right when LG changes the page | no | the page is LG's and changes server-side; a selector that stops matching leaves that part LG's |

## 5. The theme

`payload/store/store.css` (the skin), `store-light.css` (the light material only: ink on every word, since LG draws white), `userScript.js`
(the code), `Manrope-store.woff2`; `tools/store_theme.py` builds the one script from them and the live `theme.json` (the material:
the theme's own, else what Home decided from the wallpaper, else dark); `tools/store.sh` stages and mounts it; `tools/lumatv` holds the
switch (`.store-on` holds the parts, `apply` mounts it again at boot, `revert` takes it away, `surfaces` lists it).

Lucent (TV profile, Rosé Pine): the canvas is the palette's quiet base with one static glow from the upper left; no full-screen glass
slab; Manrope on the page's text components (never on icon glyphs); tiles take the tile radius; popups and confirmations get one
opaque sheet; the focus rule is the highlight at once and the lift (1.02 rows, 1.03 + 4 px cards, 1.04 buttons) after 150 ms of rest,
cancelled by the next move because the element that lost focus has no transition to finish; the ring is 3 px and its alpha is derived
per material so that it is at least 3.3:1 against the canvas and against the tile's own fill. Sizes are LG's.

Audit of the stand-in pages with the real built script (`docs/mocks/store/render.mjs`, `audit.json`): 5 pages (home, category, detail,
search, install popup) x dark and light x 100 % and 200 % text = 20 renders, **0 overlapping text, 0 text under 4.5:1** (lowest 6.86:1
dark, 5.68:1 light), ring 3.39-3.79:1. The pages are stand-ins in Sandstone's compiled class names with an LG-like stock look
(`stock.css`), drawn from the capture and from LG's Settings CSS; the pictures are generated gradients. They prove the mechanism and the
sheet's own contributions, not that every selector meets LG's page. Previews: `docs/mocks/store/shots/<page>-<dark|light>-<100|200>.jpg`
and `<page>-stock.jpg` for LG's look.

## 6. The levers

| Lever (part) | What it does | Host evidence | On the set |
|---|---|---|---|
| Transition cap (`motion`) | a transition longer than 180 ms is played faster (not cut), a script's `Element.animate` is asked for 180 ms, repeating animations are left alone | a 600 ms transform ends in 218 ms (stock 614); 800 ms and 500 ms animations become 180; a spinner's rate stays 1 | [INFERENCE]: LG's Sandstone is mostly 200 ms, so the cap saves about 20 ms per transition where it applies and more on 300-400 ms ones |
| One ring and one wash for focus (`look`) | removes `box-shadow` and `will-change` on rows, buttons and tiles, the focus fill's pseudo-element, the picture's own scale | main-thread time per focus move on the stand-in, median of 3, 179 and 1814 elements: 10.5 and 14.8 ms stock, 8.4 and 8.3 ms themed; style recalculation per move 1.4 to 0.8 ms | [INFERENCE] |
| The cost of `look` | about 45 substring selectors are tested on every element | a full style recalculation of 1814 elements: 14.3 ms stock, 39.6 ms themed (23.7-65, a busy workstation) | [INFERENCE]: paid at first layout and when a class changes high in the tree; if the set shows it at load, run `store on motion` alone |
| No `backdrop-filter`, no text shadow | a guard | LG's Settings build has none | no gain expected |
| Lazy pictures (`lazy`) | images made by script wait until near the screen | the mechanism works; `new Image()` is untouched | **off, and risky**: a page that loads through a hidden `<img>` to learn it is ready would wait for ever. A switch of its own, judged on the set |
| Script cost | 76 KB parsed at each document | 1.6 ms median on the host | [INFERENCE]: tens of ms on the set, at each of the loader and the cloud page |

Expected gain, honestly: **none expected on a cold open** (the open is the network's and the server's); **none on a warm one** (a raise);
a modest one in how a focus move and a long transition feel, and it comes from `motion` and the cheaper focus; `look` is about looks and
costs a little at load. The set's numbers decide.

## 7. The Apps grid (mock 12)

Built and live, not a plan. `panels/AppsGrid.qml` (the rail of All apps, a section per kind, On the dock, Hidden; five columns), data in
`kit/Apps.qml`, a resident layer of Home (`Layers.qml` step 7), opened by `LumaHome.openApps` from the Dock's Apps tile. **Round 8
(2026-10-25) PASS on the set: OK on the Apps tile opens the library, 23 apps, focus moves, Back returns** (`round8-result.md` (e),
`round8/tv-2026-10-25/45-apps-grid.jpg`). Gaps seen then (rail rows wrapped into the next row, a stale clock) were fixed offline in
34bfc16, and the start-up error in `AppsGrid.qml:171` guarded in 2a627b2; neither re-seen on the set, and `gate.py --surfaces apps` has
not run. Only LG's Apps tile is redirected: the library lists the store as "LG Content Store" (`kit/Apps.qml` `storeId`) and LG's Apps
remote key still opens LG's store directly, so installs go through LG's store, and it is the surface this theme is for.

## 8. The next TV round

Order matters; each step says what PASS is. Default state at the start: `lumatv store status` reports `"on":false,"mounted":false`.

0. **LG's store works, theme off** (Nitin's pending "LG store check", declutter `lean` on): open it, see the catalogue, install and remove
   a small free app (`APP_INSTALL_REQ` and `APP_REMOVE_REQ` with `caller: com.webos.app.discovery` in `/var/log/messages`). Then read the
   markup, read-only: `cat /mnt/otncabi/usr/palm/applications/com.webos.app.discovery/index.html js/Loader.js` (the cloud page's address
   and what is posted), `curl -s http://127.0.0.1:9998/json/list` (or `wget -qO-`; WAM's inspector, the store page listed or not), and with the store
   open on each page type `python3 /media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/store_dump.py --out /tmp/store-<page>.json` (class names and the Sandstone components,
   effects counted, pictures and their sizes, stylesheets, fonts, timings; none of the page's words). **PASS**: the dump names
   `enact_sandstone_*` parts; the selectors of `store.css` that match nothing are listed and fixed before any theme run.
   If the inspector does not list the store (it is `inspectable: false`), say so and take the page by the loader's own request from the set.
1. **The script runs.** `lumatv store on`; open the store. **PASS**: `store_dump.py` reports `marker: "look,motion"` and `styles.adopted: 1`
   on the loader's and the cloud page's documents; `mount | grep discovery` shows one overlay; `ls .../webOSUserScripts` holds only
   `userScript.js`. If the marker is null: WAM does not run it from an overlay on `/mnt/otncabi`, and the theme stops here.
2. **Every page type, both materials, dark first**: home (each tab), a category (`{query:"category/GAME_APPS"}` [INFERENCE: the form of a plain category link] and the New key's
   `category/THEME/20411`), an app's detail page, search with the keyboard, the update list (`category/GAME_APPS/APPUPDATES`), the
   account page, a confirmation popup, and the offline page (`network_error.html`, network off). **PASS**: canvas, ink, Manrope; the focus
   ring visible and moving; nothing clipped, nothing unreadable; icons are icons (no boxes); the spinner visible; Back and Exit as before.
   A screenshot of each (`com.webos.service.capture`) goes to `docs/research/store/`. Light: set `theme.json` `material` to `light`,
   `lumatv store on`, repeat the home page and the detail page.
3. **Install works under the theme**: from the detail page, install the same small free app, then remove it from LumaTV's library
   (`APP_INSTALL_REQ`/`APP_REMOVE_REQ` as in step 0). **PASS**: identical outcome, the button and its states as LG's.
4. **Open time, before and after**, five runs each, `luna-send .../launch {"id":"com.webos.app.discovery"}`, time to the foreground
   (`getForegroundAppInfo` polled every 50 ms) and to the first shelf's pictures (captures as fast as the service allows). **Cold**: `closeByAppId`, then
   `killApp`, wait 5 s, launch. **Warm**: open, Home, launch again after 5 s, and after 60 s. States: `store off`; `store on motion`;
   `store on look`; `store on` (both). `MemAvailable` and the WAM renderer's RSS before and after the five runs. **PASS**: p50 of each
   state no worse than `off` by more than 10 %; report each number. A `look` that costs at load shows in the cold open.
5. **Scroll and focus, before and after**: `python3 docs/research/perf/gate.py --surfaces store-scroll` (opt-in surface: cold start, five
   D-pad moves down and five up on the home page) with `store off`, then with `store on`; for the page as it is, then `motion` alone.
   **PASS**: the first frame after each key within 50 ms; frames over 20 ms in the window not more than with `off`. The burst-end
   threshold is not asked of LG's page.
6. **The way back**: `lumatv store off`; `grep discovery /proc/mounts` is empty; the store opens as LG's (a screenshot of the home page
   against step 0's); `lumatv surfaces` shows `store` off. Then `on`, a reboot, and `status`: `apply` has mounted it again; `lumatv revert`
   clears it. Other apps (the settings theme's, Netflix, the browser) open as before.
7. **Lazy pictures** (only if step 4 shows the pictures are the cost): `store on look,motion,lazy`; **PASS** only if every picture still
   loads on the home page and on a category; any missing picture ends the test and `lazy` stays off.
8. **Experiment E1, preload** (only if step 4's cold open is the problem and memory allows): save
   `luna-send -n 1 luna://com.webos.service.config/getConfigs '{"configNames":["com.webos.service.preloadmanager.supportAppList"]}'`,
   add `{"id":"com.webos.app.discovery","isEnabled":true,"preloadMode":"partial","preloadType":"oneshot","requiredMemory":60,"staticPriority":10}`
   (shaped like the browser's entry) with `setConfigs`, send `setPreloadPolicy {"application":{"id":"com.webos.app.discovery","isEnabled":true}}`
   with the screen on, and read the manager's log for "preloaded". Measure the warm open and `MemAvailable`. Put the saved list back
   with `setConfigs` afterwards. Not shipped; the memory the trim bought (PLAN: 307 MB) is what it spends.

## 9. The TV round of 2026-10-28 (steps 0 to 6, dark material, nothing bought or installed)

Evidence in `docs/research/store/` (JPEGs). `48ee380` on the set, LumaTV's compositor, receiver muted. Nothing was installed or removed (steps 0 and 3 left that out); a detail page was only opened and its Install button never pressed. The reboot of step 6 was not made (the runbook leaves the reboot to Nitin).

- **0. LG's store works, theme off.** The foreground comes 0.55 to 0.89 s after the launch; cold, the page's first shelf shows at a median 2.36 s (2.14 to 2.96 s, five runs); warm (open, Home, launch after 5 s) 80 to 120 ms. The catalogue, a detail page (Fandango at Home, Just Dance Now), the Entertainment tab and the account panel (reached by a mis-aimed key, closed with Back, nothing pressed) opened. `index.html` and `js/Loader.js` were read: the loader sends the view to `<host>.app.lgwebostv.com/discovery_2024/discovery.html` after `sdx/getHttpHeaderForServiceRequest`. WAM's inspector does not listen on this firmware (no port 9998), so `store_dump.py` has nothing to talk to: the class-name dump of step 0 and the `marker` and `styles.adopted` reads of step 1 could not be taken, and the selectors were judged by eye.
- **1. The script runs.** `store on look,motion`: `ok`, 76,432 bytes, one overlay (`lumatv-store`) over the store's directory, `userScript.js` the only file in the upper layer. The store came back with the canvas below its header in the palette's navy, so WAM runs the script from an overlay on `/mnt/otncabi`.
- **2. The look fails.** A focused tile or button keeps LG's light-grey plate while its words turned light, so "LG Radio+", "Just Dance Now", the Install button and the "more" buttons are unreadable when focused (`on-scroll.jpg`, `on-detail-tabs-account.jpg`): the ring and wash the sheet meant for focus are not there, so its selectors miss LG's focus markup while the text rule hits. The header (title and tabs) also stays black over the navy canvas, two blacks side by side. Rows, tabs and the account panel read. Light material not run.
- **3.** Not run (no install).
- **4. Open time**, cold, five runs each, launch call to the first shelf's pictures: theme off median 2,360 ms (2,140 to 2,960), on 2,580 ms (2,430 to 2,730), +9.3 %, inside the 10 % bar; foreground 550 to 890 ms off, 590 to 650 ms on; warm 80 to 120 ms either way (three runs). `motion` alone and `look` alone not run.
- **5. Scroll and focus** (`gate.py --surfaces store-scroll`, cold start, five Downs and five Ups at the remote's pace): theme off first frame 361.7 ms and 15 late frames; on (look and motion) 330.3 ms and 21 late frames; none dropped, none caused by the kernel. LG's page fails the 50 ms first-frame limit either way, and the theme neither helps nor hurts in a way three windows can show.
- **6. The way back.** `store off`: no `discovery` line in `/proc/mounts`, the staged directory gone, `lumatv surfaces` shows `store` off, the store opens as LG's (the off runs above). `on`, a reboot and `status` (`apply` mounting it again) did not run.

The theme stays off. To fix before another run: the words' colour on LG's focused plate (follow the plate, or draw the ring and wash on the right elements) and the header's background; naming LG's focus classes needs the page's DOM, which this firmware gives no way to read from the set.

## Changelog line

- `lumatv store on|off|status`: LG's Content Store in the Lucent look and with transitions capped at 180 ms, off by default, as WAM's user
  script mounted over LG's read-only app directory; `off` and `revert` give LG's store back exactly. Host-checked only.
