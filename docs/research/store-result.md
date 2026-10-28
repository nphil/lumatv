# LG Content Store: what it is, what a theme can reach, what it costs, what the TV round checks

Owner: StoreTheme. Facts are from the set (LG OLED65C4PUA, webOS 10.2.1, firmware 33.22.52) or from its firmware snapshot unless marked
[INFERENCE]. "Host" is this workstation (headless Chromium 156), never the set. Sections 1 to 3 are from before the set was used; section 9 is the first TV
round (2026-10-28): the store opened under the theme and its open time measured, and the look failed on focused tiles, so the theme was off. Section 10
is the theme rebuilt after it; sections 4 to 6 and 8 describe the rebuilt one.

## Resume here

- Rebuilt after the first TV round (section 10), host-proven, **off by default**: `lumatv store on [parts]|off|status|probe [parts]`
  (`tools/store.sh`, `tools/store_theme.py`, `tools/store_probe.py`, `payload/store/`). Parts: `look` (Manrope behind LG's family names; glass
  plate, ring and ink on the plate LG draws under a focused item, found by what it computes and not by class name), `motion` (no transition
  longer than 180 ms), `canvas` and `lazy` (opt-in), `probe` (diagnostic, only through `probe`). `on` alone is look and motion. `off`, `revert`
  and a reboot give LG's store back exactly. Registry id `store`; `apply` mounts it again at boot while on; a probe is never kept.
- Next on the set: `lumatv store probe`, open the store and move the focus over a tile, a tab and a button on each page, then
  `python3 /media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/store_probe.py report /var/lib/lumatv/store-probe`: the page's real class
  names, its `--sand-*` variables and where its focus plate is (section 10.4). WAM's inspector does not listen on this firmware, so the page posts
  that to the set itself. Then `store on` and section 8.
- Host proof: `python3 -m unittest tests.test_store_theme tests.test_store_stage tests.test_store_probe`; previews and audit in
  `docs/mocks/store/` (`node render.mjs`, `node bench.mjs --v1 <first version's script>`).

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
  CSP that forbids inline styles) and the one font as base64 (Manrope cut to Latin and to weights 300-700 as one variable face: 21 KB,
  `generators/gen-store-font.py`, declared behind LG's own family names). The default script is 52 KB (the first version's was 76 KB); host run time of its
  evaluation median 3.7 ms (section 6; the set's CPU is slower).
- **The script runs before `<html>` exists**; it needs no root, and its stylesheet is one constructable sheet, so a page policy that forbids inline styles does not see it.
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

[INFERENCE], to be settled by the probe (section 10.4) and the open-time runs of section 8:

- The open is a chain of serial phases: SAM launch and the memory check, the loader's calls for device info (`sdx` among them), DNS and TLS
  to `*.lgwebostv.com`, the page and its bundle, the catalogue's JSON (`*.nextlgsdp.com`), then the pictures (`ngfts.lge.com`). The network
  and the server own most of it; a skin cannot shorten them.
- The pictures: if the CDN sends 1920x1080 banners and 640x360 tiles, a home page of 6 banners, 30 tiles and 30 icons decodes to about
  81 MiB (7.9 MiB per banner, 0.9 per tile, 0.25 per icon) against the 96 MB budget, so the shelves would re-decode as they scroll. The
  tool reports the real `decodedMB` and the pictures shown at under two thirds of their size.

## 4. What is possible and what is not

| Wanted | Possible? | Evidence |
|---|---|---|
| A Lucent skin on the store | yes, as far as the theme can find LG's focus plate | WAM user script in every document and origin (section 2); host: 30 renders in six ways of building the plate pass the pixel audit (section 5) |
| Without touching LG's files, reversibly | yes | overlay mount; `off` unmounts, 41 tests on the tool and the verb, with a fake overlay |
| Shorter transitions | yes (transitions and `Element.animate`); not the script-driven scroll | host: 600 ms transition ends in 218 ms; `transitionend` still fires |
| Cheaper focus | not claimed | the markup is unknown, so the theme recolours the plate and leaves LG's own effects (shadows, `will-change`, scale); a focus move costs the page one look at the item (section 6) |
| Keep the store alive after leaving it | it already is (`closeOnBackground` false) until LG's memory manager closes it; making it permanent through `keepAliveApps` is not available | the list is configd's `com.webos.applicationManager` (the store is not in it); read at SAM's start [INFERENCE]; no shipped tool may restart SAM (`tests/test_tool_rules.py`) |
| Preload it hidden | possible in principle, not shipped | LG preloads LG Channels, a cloud-driven WAM web app that declares the same `supportQuickStart` and `allowCrossDomain`; the store has no entry in `supportAppList` (census `preload.json`); LG Channels' preloaded renderer holds 12 MB PSS and 21 MB in swap (`memory-result.md`), but it also sets `useAppHibernation` and the store does not [INFERENCE: that is part of why it is small]; whether the manager accepts a new id is unverified. Experiment E1 below |
| Make the cloud page faster (network, server, bundle) | no | not on the set |
| Smaller pictures | no | the sizes come from LG's CDN; rewriting what the page requests would change what it loads |
| Change what is sold, installed, priced, signed in | never | the script and sheet make no request, click, or text change; a test fails any rule that hides, moves or adds anything |
| Stay right when LG changes the page | partly | the page is LG's and changes server-side; the theme matches nothing by class, so a markup it does not understand stays LG's, with LG's own words, and is never made unreadable |

## 5. The theme

Files: `payload/store/store.css` (nine attribute rules, two more for `canvas`), `userScript.js` (the code), `Manrope-store.woff2` (the face);
`tools/store_theme.py` builds the one script from them and the live `theme.json` (the dark material only: LG's pictures, banners and words are made for
black, so the light material of the first version is gone); `tools/store.sh` stages and mounts it; `tools/store_probe.py` is the probe's receiver and
report; `tools/lumatv` holds the switch (`.store-on` holds the parts, `apply` mounts it at boot, `revert` takes it away, `surfaces` lists it).

What `look` does, by what the page computes and never by a class name:

- **Face.** Manrope is declared behind the names Sandstone, MuseoSans, LG Smart UI and LG Display, one `@font-face` per weight LG's build declares
  (a single face for a range of weights loses to the page's own at each exact weight: measured), so every text component takes it with no selector.
  One Latin variable face, 300 to 700, 21 KB, one blob URL decoded once; a character it lacks falls to LG's face of the name.
- **Plate.** On focus the script looks in the focused element and five levels below it for a light (luminance 0.35 and up), opaque background on the
  element, its `::before` or its `::after`, that has words on it and covers 12 % of the item to 70 % of the screen; a colour transition on its way in
  is judged by where it ends. It looks at the focus event, at the mutation of a class LG adds after it and at the start of a transition, all before the
  frame is drawn (1 to 5 ms on the stand-ins), and at 60, 200 and 460 ms as the last resort.
- **Glass, ring, ink.** The plate becomes the palette's focus glass (opaque `#2C263A`) with a 2 px ring (the ink at 40 %: 3.3:1 against black and against
  the glass; where the page around the plate is lighter than black, a popup's panel, the script raises the alpha until the ring is 3.3:1 on both sides) and a row's radius; its words take the ink in the tier LG gave them (full, 80 %, 66 %: all over 4.5:1 on the glass), and words LG had made
  translucent are made solid so the tier is what is seen. Words on a surface of their own (a badge) and the white tile behind an icon are left alone.
  An item with no plate (a poster, an icon button) gets the ring alone, outside its edge. Nothing scales or lifts.
- **Way back.** When the focus leaves, every mark comes off at once; a plate LG hides by fading its opacity keeps its glass for the fade, so LG's light
  plate is never seen fading out under the dark one.
- **`canvas`** (opt-in): the palette's canvas in place of LG's black, with every large near-black band painted like it (the header's: TV round 11 saw two
  blacks side by side). Off by default, because LG's pictures are made for black.

Audit of the stand-in pages with the real built script (`docs/mocks/store/render.mjs`, `audit.json`). The pages (`gen_pages.py`) are LG's page as the
TV showed it (`docs/research/store/off-*.jpg`: black canvas and header, 416 px tiles whose caption row's focus plate is rgb(230, 230, 230), the Install plate,
the grey-blue "more" buttons), in six ways LG might build the plate, none readable from the set: `pseudo` (a `::before` faded in by opacity), `child` (a child
element whose colour transitions in), `self` (the item's own background), `tile` (the plate is the caption below a focused tile), `opaque` (as `child`, class
names that mean nothing) and `late` (as `child`, switched by a class the page's own focus handler adds after the event). The five pages (home, category,
detail, search, install popup) in `pseudo` and home and detail in the five others, at 100 % and 200 % text, are 30 renders; each focuses one item and is
measured from its pixels: **every word on the plate the theme drew is over 4.5:1 (lowest 4.57:1, a lower bound: thin strokes never reach their colour),
every other word is no worse than LG drew it, no text overlaps, the ring is over 3:1 against the page and against the plate, the plate is found within
5 ms of the focus, the face is Manrope, the icon tile is not taken for a plate, and 60 ms into LG's fade-out (its transitions slowed to a tenth) the
plate's place is not light**, where LG's own plate is. Previews: `docs/mocks/store/shots/<page>-pseudo-<100|200>.jpg`, `<page>-pseudo-stock.jpg`
for LG's look and `focus-<dialect>.jpg` (LG's focused tile beside the themed one) for each of the six.

## 6. The levers

Host numbers from `docs/mocks/store/bench.mjs`: a 2,174-element stand-in page that a script renders 300 ms after the document, as the store's cloud page
renders, with each script as WAM would run it; median of 15 runs at full speed and of 11 with the CPU slowed 4x (the set's is slower still). Each cell is
full speed / slowed. These say what a script costs relative to the page, not what the set does: the TV round decides.

| Script | Bytes | Style recalculation at load, ms | Main-thread busy to load + 1 s, ms | First paint, ms | One full style recalculation, ms | One focus move, ms |
|---|---|---|---|---|---|---|
| none (LG's page) | 0 | 13.9 / 56.7 | 65.4 / 269.4 | 32 / 84 | 0.1 / 0.4 | 0.27 / 1.15 |
| first version (`look,motion`) | 76,432 | 21.3 / 87.1 | 83.0 / 344.9 | 40 / 108 | 11.2 / 49.2 | 0.29 / 1.30 |
| `look,motion` (now) | 51,839 | 13.2 / 56.0 | 73.3 / 300.9 | 40 / 104 | 0.1 / 0.7 | 1.10 / 4.75 |
| `motion` alone | 6,073 | 13.1 / 54.3 | 63.8 / 270.5 | 28 / 84 | 0.1 / 0.5 | 0.27 / 1.13 |

- **The first version cost** 17.6 / 75.5 ms of load (+27 % on this page) and a full style recalculation of 11.2 / 49.2 ms against 0.1 / 0.4: forty-five
  `[class*=...]` selectors are tested against every element.
- **Now** 7.9 / 31.5 ms (+12 %), about 0.45 of the first version's. All that is left is the face: the same script without it is within noise of LG's page (265.8 ms against 269.4
  slowed). The face is 21 KB (the first version's 39 KB) and is made at once and not when the page is idle: a face that arrives after the page's words have
  been laid out costs a second layout (about 50 ms for 2,200 elements in the markup, measured); made with the `FontFace` API instead it takes 43 ms to parse
  sixteen times (slowed), which is worse. [INFERENCE] On the set, scaled by that ratio from the round's +220 ms (+9.3 %): about +95 ms, +4 %, under the 10 % bar and
  inside the run-to-run spread (2.14 to 2.96 s).
- **A focus move** costs 0.8 / 3.6 ms more: one look at the item (its few big descendants' styles, its words' boxes) before the frame is drawn. A look of 3.6 ms with the CPU
  slowed 4x is a fifth of a 60 fps frame [INFERENCE: the set's factor is not known]; the gate's `store-scroll` (section 8, step 5) shows whether it matters.
- `canvas` adds 7 / 10 ms of load for the observer that finds LG's black bands; opt-in.

| Lever (part) | What it does | Host evidence | On the set |
|---|---|---|---|
| Transition cap (`motion`) | a transition longer than 180 ms is played faster (not cut), a script's `Element.animate` is asked for 180 ms, repeating animations are left alone | a 600 ms transform ends in 213 ms (stock 625); 800 ms and 500 ms animations become 180; a spinner's rate stays 1; costs 6 KB and nothing measurable at load | [INFERENCE]: LG's Sandstone is mostly 200 ms, so the cap saves about 20 ms per transition where it applies and more on 300 to 400 ms ones |
| Lazy pictures (`lazy`) | images made by script wait until near the screen | the mechanism works; `new Image()` is untouched | **off, and risky**: a page that loads through a hidden `<img>` to learn it is ready would wait for ever. A switch of its own, judged on the set |
| No `backdrop-filter`, no text shadow, no filter | a guard: the sheet has none, a test fails one | LG's Settings build has none | no gain expected |

Expected gain, honestly: **none expected on a cold open** (the open is the network's and the server's; the theme costs a little); **none on a warm one**; a
modest one in how a long transition feels, from `motion`. `look` is about looks and costs a little at load and at each focus move. The set's numbers decide.

## 7. The Apps grid (mock 12)

Built and live, not a plan. `panels/AppsGrid.qml` (the rail of All apps, a section per kind, On the dock, Hidden; five columns), data in
`kit/Apps.qml`, a resident layer of Home (`Layers.qml` step 7), opened by `LumaHome.openApps` from the Dock's Apps tile. **Round 8
(2026-10-25) PASS on the set: OK on the Apps tile opens the library, 23 apps, focus moves, Back returns** (`round8-result.md` (e),
`round8/tv-2026-10-25/45-apps-grid.jpg`). Gaps seen then (rail rows wrapped into the next row, a stale clock) were fixed offline in
34bfc16, and the start-up error in `AppsGrid.qml:171` guarded in 2a627b2; neither re-seen on the set, and `gate.py --surfaces apps` has
not run. Only LG's Apps tile is redirected: the library lists the store as "LG Content Store" (`kit/Apps.qml` `storeId`) and LG's Apps
remote key still opens LG's store directly, so installs go through LG's store, and it is the surface this theme is for.

## 8. The next TV round

Order matters; each step says what PASS is. Default state at the start: `lumatv store status` reports `"on":false,"mounted":false,"probe":{"running":false,...}`.
`T` below is `python3 /media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/store_probe.py`.

0. **The probe** (it replaces the page dump the closed inspector port made impossible). `lumatv store probe`; the store is ended by the command, so open it
   again, and on each page type (home and each tab, a category, a detail page, search, an update list, a confirmation popup) wait 3 s, then move the focus over a
   tile, a tab and a button. `T report /var/lib/lumatv/store-probe > /tmp/probe.txt`, copied to `docs/research/store/probe-<page>.txt` (class names and styles,
   none of the page's words). **PASS**: the report has `env`, `dom` and `focus` posts and says what the page's components are called, which light plates it
   has at rest and under each focus, its black bands, its `--sand-*` variables and its font families. If nothing arrives, `receiver.log` in that directory says
   whether the page reached the receiver; a page policy that forbids the post [INFERENCE: not seen] shows in no post at all, and then the markup is read from
   the capture by eye and the theme's plate-finder is what the next step tests.
1. **The finder on LG's own page.** `lumatv store probe look`, the same walk, and a capture of each focused item (`com.webos.service.capture`,
   `docs/research/store/probe-look-*.jpg`). Each `focus` record's "the theme found" line says which plate it found, in which of its three forms, how many words
   it took and after how many ms. **PASS**: on every focused tile, tab and button LG's light plate is the dark glass with the ring and its words read
   (compare `on-scroll.jpg` and `on-detail-tabs-account.jpg`); no light plate is seen fading out when the focus moves on; the Install and "more" buttons read;
   poster and icon buttons have the ring; nothing is scaled or moved. A focused item the finder did not find stays LG's, a light plate with dark words: that is
   a finding for the probe's report, not a fault of the way back.
2. **The theme on.** `lumatv store off`, then `lumatv store on`; the pages of step 0 again (the account panel is another app and is not themed). **PASS** as step 1,
   and: Manrope on the page's text, icons are icons, the spinner visible, Back and Exit as before. Once, `store on look,motion,canvas`: the header and the page
   are the same canvas.
3. **Install works under the theme**: from the detail page, install the same small free app, then remove it from LumaTV's library (`APP_INSTALL_REQ` and
   `APP_REMOVE_REQ` with `caller: com.webos.app.discovery` in `/var/log/messages`). **PASS**: the outcome identical to the theme off, the button and its states as LG's.
4. **Open time, before and after**, five runs each, `luna-send .../launch {"id":"com.webos.app.discovery"}`, time to the foreground (`getForegroundAppInfo`
   polled every 50 ms) and to the first shelf's pictures (captures as fast as the service allows). **Cold**: `closeByAppId`, then `killApp`, wait 5 s, launch.
   **Warm**: open, Home, launch again after 5 s and after 60 s. States: `store off`; `store on motion`; `store on look`; `store on`. `MemAvailable` and the WAM
   renderer's RSS before and after the five runs. **PASS**: the p50 of each state no worse than `off` by more than 10 %; report each number (the first version:
   +9.3 %, cold, 2,360 to 2,580 ms). A cost of `look` shows in the cold open.
5. **Scroll and focus, before and after**: `python3 docs/research/perf/gate.py --surfaces store-scroll` (opt-in surface: cold start, five D-pad moves down and five
   up on the home page) with `store off`, then with `store on`. **PASS**: the first frame after each key within 50 ms (LG's page does not meet it either way:
   362 ms off, 330 ms on in round 11); frames over 20 ms in the window not more than with `off`.
6. **The way back**: `lumatv store off`; `grep discovery /proc/mounts` is empty; the store opens as LG's (a capture of the home page against step 0's);
   `lumatv surfaces` shows `store` off and `lumatv store status` no receiver. Then `on`, a reboot and `status`: `apply` has mounted it again; `lumatv revert`
   clears it. After a reboot with only a probe run, LG's store is what opens. Other apps (the settings theme's, Netflix, the browser) open as before.
7. **Lazy pictures** (only if step 4 shows the pictures are the cost): `store on look,motion,lazy`; **PASS** only if every picture still loads on the home page and
   on a category; any missing picture ends the test and `lazy` stays off.
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

## 10. After the first TV round: the rebuilt theme

### 10.1 What the TV showed, and why

- Focused tiles and buttons kept LG's light plate and took light words (section 9, step 2). The first version matched forty-five substring selectors of
  Sandstone's class names, taken from LG's Settings build and one capture of the store; LG's focus markup is none of them, while the blanket rule that
  turned words light matched. The failure was the worst one available: light on light.
- The header stayed black over the navy canvas: its band is a container LG paints itself.
- The cold open was +9.3 % and the scroll gate's first frame did not improve: the selectors cost a full style recalculation at every load (11 ms on the host's
  2,200 elements, section 6) and the script carried a 39 KB face inside 76 KB.
- WAM's inspector does not listen on this firmware, so the page's markup cannot be read from the set, and a theme that needs it cannot be right on the first try.

### 10.2 What changed

- **No selector names the page.** The face is put behind LG's family names (one rule per weight), and the plate is found by what the page computes (section 5). A
  markup the finder does not understand stays LG's, with LG's words: the words are only ever recoloured together with a plate that is recoloured with them.
- **Ink follows the plate**, tier by tier; words on a surface of their own and the white tile behind an icon are left alone; an item with no plate gets a ring.
- **Dark only.** LG's pictures, banners and words are made for black; the light material of the first version is gone (`store-light.css`, `light_skin`).
- **The canvas is opt-in** (`canvas`): the default no longer repaints LG's black, so there is no second black beside the header's.
- **Cheaper.** A 21 KB Latin variable face (the first version's 39 KB), nine rules, no recalculation (section 6).
- **Fewer fades.** The plate comes without LG's transition and goes without a light plate fading out (section 5).
- **A probe instead of the page dump** (10.4). `tools/store_dump.py` is removed: it needed the inspector.

### 10.3 What the host proves, and what it cannot

`tests.test_store_theme` runs the real built script in Chromium against pages in six ways of building the plate and one more for each thing that must not be
taken for one (an icon's white tile, a badge, a panel the size of the screen, an item with no plate); `render.mjs` audits 30 stand-in renders from their pixels
(section 5). It cannot prove LG's real markup is one of the six, or that the cloud page's policy lets the probe's post out. The finder fails towards LG's look
and not towards an unreadable one, and the probe says what the real markup is.

### 10.4 The probe

`lumatv store probe [parts]` starts `tools/store_probe.py serve` (stdlib sockets, loopback only, 30 minutes, bounded in posts and bytes) and mounts the script with
its `probe` part and that receiver's port, then ends the store so its next launch loads it; with no parts the page is LG's as it is, with `look` the theme and the
probe together. The page posts, to `http://127.0.0.1:<port>/p` (an image beacon to `/b` when the post is refused), the following, never its words (a text node
is posted as its length, a URL as its host and path, any attribute but class, id and role as the length of its value): `env` (families used, font faces
declared, stylesheets, `--sand-*` variables, meta policy), `dom` (up to 3,500 nodes: tag, classes, rectangle, the computed styles that matter, the `::before`
and `::after` of each; at load and after a navigation), `focus` (400 ms after a focus: the item, its ancestors and its subtree with the plates the theme found),
`classes` (the classes that change within 700 ms of a key press, and the kind of key: how the page marks the item the remote moved to when it is not by the browser's
focus) and `csp` and `error`. `python3 tools/store_probe.py report /var/lib/lumatv/store-probe` prints what a person needs to fix the finder. Never persisted: a reboot,
`lumatv store on` and `lumatv store off` end the receiver, and the last session's files stay (`store-probe.prev` once a new one starts).

### 10.5 What is still unknown

- The page's real markup and which of the six ways its plate is built: step 0 of section 8.
- Whether the cloud page's content security policy lets the probe's post out (not seen on the host, where the page's own policy is a stand-in).
- The cost of the face and of a focus move on the set: the cold-open and scroll-gate steps of section 8.
- Whether `store on` as a whole is the right default for LG's store once the look reads: if the cold open is not within 5 %, `store on motion` is the cheap part.

## Changelog line

- `lumatv store on|off|status|probe`: LG's Content Store in the Lucent look, rebuilt after the first TV round: Manrope behind Sandstone's names, a glass plate with a ring
  and ink on the plate LG draws under a focused item (found by what the page computes, not by class), transitions capped at 180 ms; off by default, as WAM's user
  script mounted over LG's read-only app directory, `off` and `revert` give LG's store back exactly. `probe` posts the page's structure to a receiver on the set for the
  next round. Host-checked only.

## 11. The TV round of 2026-10-28, `a1555df` (round 14, `compositor/slot-2026-10-28-r14/img/probe-focus-2.png`, `on-focus2.png`)

`lumatv store probe look` then the store opened (foreground 0.65-0.84 s after the launch, three cold runs with `store on`): the probe received `focus` and `classes` posts (33 files in `/var/lib/lumatv/store-probe`);
focus moves are marked by `.focus` on `div.blank.item-tit-wrap` (tiles) and `div.blank.btn.item-downBtn`; the finder found the plate on every focused tile (`ps:div:43470`, 3 words, 4-12 ms)
and found none for the round download and header buttons (`null`; they keep LG's light plate and dark glyph, which read). Tiles show the dark glass with the ring and light words; no light-on-light.
`store on` (look, motion) over the same pages looks the same. Not run: the page walk of steps 3, 5, 6 (install, scroll gate, reboot).
Result: PASS for steps 0 to 2; left on.
